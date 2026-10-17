//! `GET /v1/families` and `GET /v1/families/{family}`: the family collection
//! and one family, under `CatalogRead`.
//!
//! The rows come through `aircraft_app`'s
//! [`FamilyReader`](aircraft_app::catalog::FamilyReader) port, because this
//! crate may not reach `aircraft_db` or `SQLx`. The shape follows
//! [`crate::routes::models`] -- one sort allowlist, flat query members reaching
//! the shared cursor path, and an RFC 9457 document for every refusal -- and
//! differs from it deliberately in one place, recorded below.
//!
//! **A malformed identifier is a `400` here, and `404` is reserved for a
//! well-formed one that names nothing.** `docs/architecture/http_v1_decisions.md`
//! § "Problem responses" publishes malformed input and not-found as distinct
//! classes with distinct types, and issue #39 requires the split. The model
//! detail route originally collapsed both into `404` on an
//! information-disclosure argument that does not survive contact with this
//! route: every slug the argument would protect is already enumerated by the
//! collection under the same scope, so the only thing the collapse hid was a
//! client's own malformed request. `crate::routes::models::model` was corrected
//! to match when this route landed; change both together.
//!
//! The path segment is the family's `slug`. Issue #39 writes the parameter as
//! `{family_id}`; `aircraft_core.families.id` is withheld from every catalog
//! projection on the reasoning `database/data_dictionary.md` records -- a
//! surrogate key is not a public identifier -- so the served parameter is
//! `{family}`, as `/v1/reference/{catalog}` and `/v1/models/{model}` spell
//! theirs.

use std::num::NonZeroU16;

use aircraft_app::{
  catalog::{FamilyDetail, FamilyFilter, FamilySummary},
  ingestion::PersistenceError,
};
use aircraft_domain::catalog::{CountryCode, Slug};
use axum::{
  extract::{OriginalUri, Path, State},
  response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
  ApiState,
  pagination::{FilterFingerprint, ListQuery, PageResponse},
  problem::{ApiProblem, ApiQuery, PerimeterResponses, ProblemDetails},
};

/// Which order a caller asked for.
///
/// A closed enum rather than a string, so `?sort=name_desc` is refused by the
/// query deserialization that already refuses `?limit=abc`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FamilySort {
  #[default]
  SlugAsc,
}

impl FamilySort {
  const fn code(self) -> &'static str {
    match self {
      Self::SlugAsc => "slug_asc",
    }
  }
}

/// The collection's query parameters.
///
/// `limit` and `cursor` are declared here rather than by flattening
/// [`ListQuery`]; `crate::pagination` records why that cannot work through
/// `axum`'s `Query`. They reach the shared cursor path through
/// [`ListQuery::new`].
///
/// Both filters are the public identifiers, not resolved keys:
/// [`FamilyFilter`] carries a [`Slug`] and a [`CountryCode`] and the adapter
/// compares them in SQL, so unlike the model collection's family filter there
/// is no id to resolve and no lookup to run first.
#[derive(Debug, Deserialize)]
pub struct FamilyListQuery {
  sort: Option<FamilySort>,
  manufacturer: Option<String>,
  country_of_origin: Option<String>,
  limit: Option<NonZeroU16>,
  cursor: Option<String>,
}

/// One family as a collection row.
#[derive(Debug, Serialize, ToSchema)]
pub struct FamilySummaryResponse {
  /// The stable public identifier, as `aircraft_ref.slug_text` spells it.
  pub slug: String,
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub common_name: Option<String>,
  /// The manufacturing organisation's own slug, absent when the family records
  /// no manufacturer.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub manufacturer: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub country_of_origin: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_flight_year: Option<i16>,
}

impl From<FamilySummary> for FamilySummaryResponse {
  /// Destructured rather than read field by field, so a member added to
  /// [`FamilySummary`] is a compile error here instead of a member silently
  /// missing from the wire. The two are deliberately separate representations;
  /// this keeps the mapping between them exhaustive.
  fn from(family: FamilySummary) -> Self {
    let FamilySummary {
      slug,
      name,
      common_name,
      manufacturer,
      country_of_origin,
      first_flight_year,
    } = family;

    Self {
      slug: slug.as_str().to_owned(),
      name,
      common_name,
      manufacturer: manufacturer.map(|value| value.as_str().to_owned()),
      country_of_origin: country_of_origin.map(|value| value.as_str().to_owned()),
      first_flight_year,
    }
  }
}

/// One family, whole.
///
/// The summary's members are repeated rather than nested or flattened, which is
/// what `AGENTS.md` asks of HTTP DTOs: explicit mappings of the application
/// types, not structural reuse of them. `name_aliases` is always present,
/// possibly empty, because [`FamilyDetail`] already collapsed a NULL array to
/// an empty one.
#[derive(Debug, Serialize, ToSchema)]
pub struct FamilyDetailResponse {
  pub slug: String,
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub common_name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub manufacturer: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub country_of_origin: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_flight_year: Option<i16>,
  pub name_aliases: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
}

impl From<FamilyDetail> for FamilyDetailResponse {
  /// Destructured on both levels, for the reason
  /// [`FamilySummaryResponse::from`] gives.
  fn from(family: FamilyDetail) -> Self {
    let FamilyDetail { summary, name_aliases, description } = family;
    let FamilySummary {
      slug,
      name,
      common_name,
      manufacturer,
      country_of_origin,
      first_flight_year,
    } = summary;

    Self {
      slug: slug.as_str().to_owned(),
      name,
      common_name,
      manufacturer: manufacturer.map(|value| value.as_str().to_owned()),
      country_of_origin: country_of_origin.map(|value| value.as_str().to_owned()),
      first_flight_year,
      name_aliases,
      description,
    }
  }
}

/// Logs the failure class and answers with the status its class earns.
///
/// The mapping is `ApiProblem::from_persistence`'s, shared with every other
/// catalog route. What stays here is the log line: the class is recorded for an
/// operator correlating the failure, and never served, because the document
/// cannot carry a diagnostic.
fn read_failure(error: &PersistenceError, instance: &str, operation: &str) -> Response {
  tracing::warn!(code = error.code(), operation, "family read failed");
  ApiProblem::from_persistence(error, instance).into_response()
}

/// What this endpoint's cursor is bound to.
///
/// Stable by construction: both filters in a fixed order, each rendered the same
/// way whether or not it was sent, so a cursor issued under one filter set is
/// refused under another. The absent case is an empty value rather than an
/// omitted term, because an unstable normalization silently invalidates every
/// outstanding cursor.
fn fingerprint(manufacturer: Option<&str>, country: Option<&str>) -> FilterFingerprint {
  FilterFingerprint::of(&format!(
    "manufacturer={}&country_of_origin={}",
    manufacturer.unwrap_or_default(),
    country.unwrap_or_default()
  ))
}

#[utoipa::path(
  get,
  path = "/v1/families",
  tag = "catalog",
  summary = "List families",
  description = "One keyset-paginated page of families, optionally filtered by \
                 manufacturer slug and country of origin. A filter value that is \
                 not a valid identifier is a 400. A filter that is valid but \
                 matches nothing is a successful empty collection, because both \
                 filters are compared directly and no existence lookup runs. The \
                 only sort is slug_asc.",
  params(
    (
      "manufacturer" = Option<String>, Query,
      description = "Manufacturer organisation slug. A value that is not a slug is \
                     a 400.", nullable = false
    ),
    (
      "country_of_origin" = Option<String>, Query,
      description = "ISO 3166-1 alpha-3 country code, upper case. A value that is \
                     not one is a 400.", nullable = false
    ),
    (
      "sort" = Option<FamilySort>, Query,
      description = "Sort order. slug_asc is the only supported value.",
      nullable = false
    ),
    (
      "limit" = Option<u16>, Query, minimum = 1,
      description = "Rows per page. Defaults to 50; values above 200 are capped to \
                     200. A limit of 0 is refused.",
      nullable = false
    ),
    (
      "cursor" = Option<String>, Query,
      description = "Opaque continuation token from a previous page's next_cursor. \
                     Clients must not construct or interpret one.",
      nullable = false
    ),
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "One page of families, in slug order, with a null next_cursor on \
                     the final page",
      body = PageResponse<FamilySummaryResponse>,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 400,
      description = "An unsupported sort, a filter value that is not a valid \
                     identifier, or a cursor that is malformed, of another version, \
                     for another sort, or for different filters",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    PerimeterResponses,
    (
      status = 503,
      description = "The service is draining, is at capacity, or could not read the \
                     families",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn list_families(
  State(state): State<ApiState>,
  OriginalUri(uri): OriginalUri,
  ApiQuery(query): ApiQuery<FamilyListQuery>,
) -> Response {
  let instance = uri.path();
  let sort = query.sort.unwrap_or_default();

  // Both filters are validated before any page is read, so a value the domain
  // refuses never reaches the collection statement.
  let Ok(manufacturer) = query.manufacturer.as_deref().map(Slug::try_from).transpose() else {
    return ApiProblem::validation_failed(instance).into_response();
  };
  let Ok(country) = query.country_of_origin.as_deref().map(CountryCode::try_from).transpose()
  else {
    return ApiProblem::validation_failed(instance).into_response();
  };

  let filters = fingerprint(query.manufacturer.as_deref(), query.country_of_origin.as_deref());
  let (limit, resume) =
    match ListQuery::new(query.limit, query.cursor).into_page_request(filters, instance) {
      Ok(request) => request,
      Err(problem) => return problem.into_response(),
    };

  // A cursor carries the sort it was issued for, and this endpoint takes the
  // column from its own allowlist rather than from the token, so comparison is
  // the only use the token's `sort` has.
  let after = match resume {
    None => None,
    Some(position) if position.sort != sort.code() => {
      return ApiProblem::validation_failed(instance).into_response();
    }
    // `last_value` is caller-supplied text decoding does not interpret, so a
    // token carrying something that is not a slug is a 400 here and never a
    // statement parameter.
    Some(position) => match Slug::try_from(position.last_value.as_str()) {
      Ok(slug) => Some(slug),
      Err(_) => return ApiProblem::validation_failed(instance).into_response(),
    },
  };

  let filter = FamilyFilter { manufacturer, country_of_origin: country };
  match state.families.list_families(&filter, limit, after.as_ref()).await {
    Ok(page) => axum::Json(crate::pagination::render_page(
      page,
      sort.code(),
      filters,
      limit,
      FamilySummaryResponse::from,
    ))
    .into_response(),
    Err(error) => read_failure(&error, instance, "list_families"),
  }
}

#[utoipa::path(
  get,
  path = "/v1/families/{family}",
  tag = "catalog",
  summary = "Read one family",
  description = "One family by its slug. A segment that is not a slug is a 400; a \
                 slug no family carries is a 404.",
  params(
    (
      "family" = String, Path,
      description = "The family's slug. The surrogate key is not a public identifier \
                     and is never accepted here."
    ),
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "The family",
      body = FamilyDetailResponse,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 400,
      description = "The path segment is not a slug",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 404,
      description = "No family has that slug",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    PerimeterResponses,
    (
      status = 503,
      description = "The service is draining, is at capacity, or could not read the \
                     family",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn family(
  State(state): State<ApiState>,
  OriginalUri(uri): OriginalUri,
  Path(requested): Path<String>,
) -> Response {
  let instance = uri.path();

  // Malformed, not absent: the caller's own request is wrong, and the problem
  // table carries that as its own class. See this module's header for why the
  // information-disclosure argument for collapsing it into 404 does not hold
  // for a catalog whose collection enumerates every slug under the same scope.
  let Ok(slug) = Slug::try_from(requested.as_str()) else {
    return ApiProblem::validation_failed(instance).into_response();
  };

  match state.families.family(&slug).await {
    Ok(Some(detail)) => axum::Json(FamilyDetailResponse::from(detail)).into_response(),
    Ok(None) => ApiProblem::not_found(instance).into_response(),
    Err(error) => read_failure(&error, instance, "family"),
  }
}
