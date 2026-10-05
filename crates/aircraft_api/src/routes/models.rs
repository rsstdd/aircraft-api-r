//! `GET /v1/models` and `GET /v1/models/{model}`: the model collection and one
//! model, under `CatalogRead`.
//!
//! The rows come through `aircraft_app`'s [`ModelReader`](aircraft_app::catalog::ModelReader)
//! port, and the family filter is resolved through
//! [`family_id`](aircraft_app::catalog::FamilyReader::family_id), because this crate
//! may not reach `aircraft_db` or `SQLx`. `docs/architecture/http_v1_decisions.md`
//! fixes three behaviours this module implements rather than decides:
//! keyset pagination with a per-endpoint sort allowlist, a `404` for a missing
//! parent against a successful empty collection for a childless one, and an
//! RFC 9457 problem document for every `4xx` and `5xx`.
//!
//! The path segment is the model's `slug`, not its surrogate key. Issue #41
//! writes the parameter as `{model_id}`; `aircraft_core.models.id` is withheld
//! from every catalog projection on the reasoning
//! `database/data_dictionary.md` records -- a surrogate key is not a public
//! identifier -- so the served parameter is `{model}`, spelled as
//! `/v1/reference/{catalog}` spells its own slug segment.

use std::num::NonZeroU16;

use aircraft_app::{
  catalog::{ModelDetail, ModelFilter, ModelSummary},
  ingestion::PersistenceError,
  pagination::Page,
};
use aircraft_domain::catalog::Slug;
use axum::{
  extract::{OriginalUri, Path, State},
  response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
  ApiState,
  pagination::{CursorPosition, FilterFingerprint, ListQuery, PageResponse, encode},
  problem::{ApiProblem, ApiQuery, PerimeterResponses, ProblemDetails},
};

/// This endpoint's whole sort allowlist.
///
/// One order, because `aircraft_core.models.slug` is `NOT NULL UNIQUE`
/// (`004:76`) and so is already the total order the accepted decision requires
/// of every sort -- the unique identifier *is* the sort key, and no separate
/// tiebreaker exists to add. A second order is a change to this allowlist and
/// to the adapter's `ORDER BY` together.
///
/// Spelled as a constant because two things compare against it: the `sort`
/// query parameter and the `sort` a cursor carries. The parameter is typed, so
/// the comparison that can actually fail is the cursor's.
const SLUG_ASCENDING: &str = "slug_asc";

/// Which order a caller asked for.
///
/// A closed enum rather than a string, so `?sort=name_desc` is refused by the
/// same query deserialization that already refuses `?limit=abc` -- the `400`
/// the acceptance criteria require of an unsupported sort, with no comparison
/// of this module's own.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelSort {
  #[default]
  SlugAsc,
}

impl ModelSort {
  const fn code(self) -> &'static str {
    match self {
      Self::SlugAsc => SLUG_ASCENDING,
    }
  }
}

/// The collection's query parameters.
///
/// The `limit` and `cursor` members are declared here rather than by flattening
/// [`ListQuery`]: `crates/aircraft_api/src/pagination.rs` records why that
/// cannot work through `axum`'s `Query`. They reach the shared cursor path
/// through [`ListQuery::new`], so the decode and the page bound still have one
/// implementation.
#[derive(Debug, Deserialize)]
pub struct ModelListQuery {
  sort: Option<ModelSort>,
  /// The parent family's slug. Validated here, resolved to a [`FamilyId`]
  /// through the port, because [`ModelFilter::family`] takes a resolved parent.
  family: Option<String>,
  limit: Option<NonZeroU16>,
  cursor: Option<String>,
}

/// One model as a collection row.
#[derive(Debug, Serialize, ToSchema)]
pub struct ModelSummaryResponse {
  /// The stable public identifier, as `aircraft_ref.slug_text` spells it.
  pub slug: String,
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub display_name: Option<String>,
  /// The parent family's slug.
  pub family: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub series: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub generation: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_flight_year: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub certification_year: Option<i16>,
}

impl From<ModelSummary> for ModelSummaryResponse {
  /// Destructured rather than read field by field, so that a member added to
  /// [`ModelSummary`] is a compile error here instead of a member silently
  /// missing from the wire. The two types are deliberately separate
  /// representations; this makes the mapping between them exhaustive.
  fn from(model: ModelSummary) -> Self {
    let ModelSummary {
      slug,
      name,
      display_name,
      family,
      series,
      generation,
      first_flight_year,
      certification_year,
    } = model;

    Self {
      slug: slug.as_str().to_owned(),
      name,
      display_name,
      family: family.as_str().to_owned(),
      series,
      generation,
      first_flight_year,
      certification_year,
    }
  }
}

/// One model, whole.
///
/// The summary's members are repeated rather than nested or flattened, which is
/// what `AGENTS.md` asks for in keeping HTTP DTOs explicit mappings of the
/// application types instead of structural reuse of them. `name_aliases` is
/// always present, possibly empty, because [`ModelDetail`] already collapsed a
/// NULL array to an empty one and a client should not have to decide again.
#[derive(Debug, Serialize, ToSchema)]
pub struct ModelDetailResponse {
  pub slug: String,
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub display_name: Option<String>,
  pub family: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub series: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub generation: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_flight_year: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub certification_year: Option<i16>,
  pub name_aliases: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
}

impl From<ModelDetail> for ModelDetailResponse {
  /// Destructured for the reason [`ModelSummaryResponse::from`] gives, on both
  /// levels: a member added to either [`ModelDetail`] or the [`ModelSummary`]
  /// it carries stops this compiling.
  fn from(model: ModelDetail) -> Self {
    let ModelDetail { summary, name_aliases, description } = model;
    let ModelSummary {
      slug,
      name,
      display_name,
      family,
      series,
      generation,
      first_flight_year,
      certification_year,
    } = summary;

    Self {
      slug: slug.as_str().to_owned(),
      name,
      display_name,
      family: family.as_str().to_owned(),
      series,
      generation,
      first_flight_year,
      certification_year,
      name_aliases,
      description,
    }
  }
}

/// The failure class a read reported, as the one answer it maps to.
///
/// The split is `routes::reference::catalog`'s and the reasoning is the same: a
/// `Database` failure is a dependency outage that `503` correctly invites a
/// retry for, while an `Invariant` failure means the database answered and
/// broke a rule this service set, which no retry clears. The class is logged
/// and never served, so the document carries no diagnostic.
fn read_failure(error: &PersistenceError, instance: &str, operation: &str) -> Response {
  tracing::warn!(code = error.code(), operation, "model read failed");
  match error {
    PersistenceError::Database { .. } => ApiProblem::database_unavailable(instance),
    PersistenceError::Invariant(_) => ApiProblem::internal(instance),
  }
  .into_response()
}

/// What this endpoint's cursor is bound to.
///
/// Stable by construction: one filter, rendered the same way whether or not it
/// was sent, so a cursor issued unfiltered is refused on a filtered scan and
/// the reverse. An unstable normalization would silently invalidate every
/// outstanding cursor, which is why the absent case is an empty value rather
/// than an omitted term.
fn fingerprint(family: Option<&str>) -> FilterFingerprint {
  FilterFingerprint::of(&format!("family={}", family.unwrap_or_default()))
}

#[utoipa::path(
  get,
  path = "/v1/models",
  tag = "catalog",
  summary = "List models",
  description = "One keyset-paginated page of models, optionally filtered to a \
                 parent family. A family slug that names no family is a 404; a \
                 family that exists with no models is a successful empty \
                 collection. The only sort is slug_asc.",
  params(
    (
      "family" = Option<String>, Query,
      description = "Parent family slug. A value that is not a slug is a 400; \
                     one that names no family is a 404.", nullable = false
    ),
    (
      "sort" = Option<ModelSort>, Query,
      description = "Sort order. slug_asc is the only supported value.",
      nullable = false
    ),
    (
      // `minimum = 1` and no `maximum`, because that is what the service does:
      // `limit=0` is refused, while a value above the ceiling is capped to it
      // rather than rejected (`PageLimit::from_requested`). Publishing
      // `maximum = 200` would call a request invalid that this service accepts.
      "limit" = Option<u16>, Query, minimum = 1,
      description = "Rows per page. Defaults to 50; values above 200 are capped to \
                     200. A limit of 0 is refused.",
      nullable = false
    ),
    (
      "cursor" = Option<String>, Query,
      description = "Opaque continuation token from a previous page's \
                     next_cursor. Clients must not construct or interpret one.",
      nullable = false
    ),
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "One page of models, in slug order, with a null next_cursor on \
                     the final page",
      body = PageResponse<ModelSummaryResponse>,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 400,
      description = "An unsupported sort, a family filter that is not a slug, or a \
                     cursor that is malformed, of another version, for another sort, \
                     or for different filters",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 404,
      description = "The family filter names no family",
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
                     models",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn list_models(
  State(state): State<ApiState>,
  OriginalUri(uri): OriginalUri,
  ApiQuery(query): ApiQuery<ModelListQuery>,
) -> Response {
  let instance = uri.path();
  let sort = query.sort.unwrap_or_default();

  // The filter is validated and resolved before any page is read, so a request
  // naming a family that cannot exist never reaches the collection statement.
  let family = match query.family.as_deref() {
    None => None,
    Some(requested) => {
      let Ok(slug) = Slug::try_from(requested) else {
        return ApiProblem::validation_failed(instance).into_response();
      };
      match state.families.family_id(&slug).await {
        Ok(Some(id)) => Some(id),
        // A missing parent, which the accepted decision answers 404 -- not an
        // empty page, which is reserved for a family that exists and has no
        // models, and not a 400, which would call a well-formed request
        // malformed.
        Ok(None) => return ApiProblem::not_found(instance).into_response(),
        Err(error) => return read_failure(&error, instance, "resolve_family"),
      }
    }
  };

  let filters = fingerprint(query.family.as_deref());
  let (limit, resume) =
    match ListQuery::new(query.limit, query.cursor).into_page_request(filters, instance) {
      Ok(request) => request,
      Err(problem) => return problem.into_response(),
    };

  // A cursor carries the sort it was issued for, and this endpoint takes the
  // column from its own allowlist rather than from the token, so the comparison
  // is the only thing the token's `sort` is used for.
  let after = match resume {
    None => None,
    Some(position) if position.sort != sort.code() => {
      return ApiProblem::validation_failed(instance).into_response();
    }
    // `last_value` is caller-supplied text that decoding does not interpret, so
    // a token carrying something that is not a slug is a 400 here and never a
    // statement parameter.
    Some(position) => match Slug::try_from(position.last_value.as_str()) {
      Ok(slug) => Some(slug),
      Err(_) => return ApiProblem::validation_failed(instance).into_response(),
    },
  };

  let page = match state.models.list_models(&ModelFilter { family }, limit, after.as_ref()).await {
    Ok(page) => page,
    Err(error) => return read_failure(&error, instance, "list_models"),
  };

  axum::Json(page_response(page, sort, filters)).into_response()
}

/// Renders a page, issuing the continuation its last row implies.
///
/// The cursor's `last_value` and `tiebreaker` are the same slug because this
/// endpoint's sort key *is* its unique identifier; they are both set rather
/// than one left empty so the token stays readable by the shared decoder and a
/// future second sort does not change the payload's shape.
fn page_response(
  page: Page<ModelSummary, Slug>,
  sort: ModelSort,
  filters: FilterFingerprint,
) -> PageResponse<ModelSummaryResponse> {
  let (resume, rows) = page.into_parts();
  let next_cursor = resume.map(|last| {
    encode(
      &CursorPosition {
        sort: sort.code().to_owned(),
        last_value: last.as_str().to_owned(),
        tiebreaker: last.as_str().to_owned(),
      },
      filters,
    )
  });

  PageResponse { items: rows.into_iter().map(ModelSummaryResponse::from).collect(), next_cursor }
}

#[utoipa::path(
  get,
  path = "/v1/models/{model}",
  tag = "catalog",
  summary = "Read one model",
  description = "One model by its slug. A slug no model carries is a 404.",
  params(
    (
      "model" = String, Path,
      description = "The model's slug. The surrogate key is not a public identifier \
                     and is never accepted here."
    ),
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "The model",
      body = ModelDetailResponse,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 404,
      description = "No model has that slug",
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
                     model",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn model(
  State(state): State<ApiState>,
  OriginalUri(uri): OriginalUri,
  Path(requested): Path<String>,
) -> Response {
  let instance = uri.path();

  // A path segment that is not a slug names nothing, so it is the same `404` a
  // well-formed slug with no row gets. Answering `400` here would tell a caller
  // probing identifiers which of their guesses were even shaped right.
  let Ok(slug) = Slug::try_from(requested.as_str()) else {
    return ApiProblem::not_found(instance).into_response();
  };

  match state.models.model(&slug).await {
    Ok(Some(detail)) => axum::Json(ModelDetailResponse::from(detail)).into_response(),
    Ok(None) => ApiProblem::not_found(instance).into_response(),
    Err(error) => read_failure(&error, instance, "model"),
  }
}
