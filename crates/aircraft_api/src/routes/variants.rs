//! `GET /v1/variants`: the variant collection, under `CatalogRead`.
//!
//! The rows come through `aircraft_app`'s
//! [`VariantReader`](aircraft_app::catalog::VariantReader) port; the `model` and
//! `family` filters are resolved through
//! [`model_id`](aircraft_app::catalog::ModelReader::model_id) and
//! [`family_id`](aircraft_app::catalog::FamilyReader::family_id), because
//! [`VariantFilter`] carries resolved keys and a request carries slugs.
//!
//! The shape follows [`crate::routes::families`] and [`crate::routes::models`]:
//! one sort allowlist, flat query members reaching the shared cursor path, a
//! malformed identifier as `400` and an absent one as `404`, and an RFC 9457
//! document for every refusal. Three things are this endpoint's own:
//!
//! - **A reference code no vocabulary carries is a `400` naming the filter**,
//!   not an empty page. The adapter reports it as
//!   [`VariantReadError::UnknownFilterCode`] and this module maps it; issue #42
//!   required the distinction and #43 requires the status.
//! - **`produced_in_year` is range-checked here**, against the bounds
//!   `aircraft_ref.year_value` declares, so a year the schema could not store is
//!   the caller's `400` rather than a query that cannot match.
//! - **No specification values appear in a summary.** [`VariantSummaryResponse`]
//!   projects `aircraft_core.variants` columns only; performance, weight, and
//!   cost measurements are curated data living in other schemas, and a pending
//!   assertion never becomes canonical. The response has no member that could
//!   carry one, which is what #43's fourth criterion asks for and what
//!   `a_summary_carries_no_specification_values` pins.

use std::num::NonZeroU16;

use aircraft_app::{
  catalog::{VariantFilter, VariantReadError, VariantSummary},
  ingestion::PersistenceError,
};
use aircraft_domain::catalog::{CountryCode, LookupCode, Slug};
use axum::{
  extract::{OriginalUri, State},
  response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
  ApiState,
  pagination::{FilterFingerprint, ListQuery, PageResponse},
  problem::{ApiProblem, ApiQuery, PerimeterResponses, ProblemDetails},
};

/// The inclusive bounds `aircraft_ref.year_value` declares.
///
/// Mirrored here because the filter is checked before any statement runs: a
/// year outside them is a request the schema could never match, and reporting it
/// as an invalid filter is more useful than an empty page.
///
/// `database/migrations/001_extensions_schemas_domains_triggers.sql:117-118`
/// declares `CHECK (VALUE BETWEEN 1900 AND 2100)`. That migration is hashed and
/// immutable, so it cannot name this constant back; `database/data_dictionary.md`
/// carries the other half of the mirror instead, and the three move together.
const EARLIEST_YEAR: i16 = 1900;
const LATEST_YEAR: i16 = 2100;

/// Which order a caller asked for.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariantSort {
  #[default]
  SlugAsc,
}

impl VariantSort {
  const fn code(self) -> &'static str {
    match self {
      Self::SlugAsc => "slug_asc",
    }
  }
}

/// The collection's query parameters.
///
/// Nine filters, the sort, and the page members. `limit` and `cursor` are
/// declared here rather than by flattening [`ListQuery`]; `crate::pagination`
/// records why that cannot work through `axum`'s `Query`.
#[derive(Debug, Deserialize)]
pub struct VariantListQuery {
  sort: Option<VariantSort>,
  model: Option<String>,
  family: Option<String>,
  variant_type: Option<String>,
  service_status: Option<String>,
  landing_gear_type: Option<String>,
  propulsion_category: Option<String>,
  country_of_origin: Option<String>,
  is_in_production: Option<bool>,
  produced_in_year: Option<i16>,
  limit: Option<NonZeroU16>,
  cursor: Option<String>,
}

/// One variant as a collection row.
///
/// Wider than the family and model rows because a variant is what a client
/// compares, which is the reasoning [`VariantSummary`] records. Every member is
/// an `aircraft_core.variants` column or its parent model's slug; none is a
/// measurement.
#[derive(Debug, Serialize, ToSchema)]
pub struct VariantSummaryResponse {
  /// The stable public identifier, as `aircraft_ref.slug_text` spells it.
  pub slug: String,
  pub name: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub popular_name: Option<String>,
  /// The parent model's slug.
  pub model: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub variant_type: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub service_status: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub country_of_origin: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub first_flight_year: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub certification_year: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub production_start_year: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub production_end_year: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub passenger_capacity: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub crew_count: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub engine_count: Option<i16>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub landing_gear_type: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub propulsion_category: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub is_in_production: Option<bool>,
}

impl From<VariantSummary> for VariantSummaryResponse {
  /// Destructured rather than read field by field, so a member added to
  /// [`VariantSummary`] is a compile error here instead of a member silently
  /// missing from the wire -- and, for this endpoint, so that a measurement
  /// added to the application type cannot reach a summary unnoticed.
  fn from(variant: VariantSummary) -> Self {
    let VariantSummary {
      slug,
      name,
      popular_name,
      model,
      variant_type,
      service_status,
      country_of_origin,
      first_flight_year,
      certification_year,
      production_start_year,
      production_end_year,
      passenger_capacity,
      crew_count,
      engine_count,
      landing_gear_type,
      propulsion_category,
      is_in_production,
    } = variant;

    Self {
      slug: slug.as_str().to_owned(),
      name,
      popular_name,
      model: model.as_str().to_owned(),
      variant_type: variant_type.map(|code| code.as_str().to_owned()),
      service_status: service_status.map(|code| code.as_str().to_owned()),
      country_of_origin: country_of_origin.map(|code| code.as_str().to_owned()),
      first_flight_year,
      certification_year,
      production_start_year,
      production_end_year,
      passenger_capacity,
      crew_count,
      engine_count,
      landing_gear_type: landing_gear_type.map(|code| code.as_str().to_owned()),
      propulsion_category: propulsion_category.map(|code| code.as_str().to_owned()),
      is_in_production,
    }
  }
}

/// The failure class a read reported, as the one answer it maps to.
///
/// Three outcomes, not two. An unknown filter code is the caller's `400`; the
/// other two are `ApiProblem::from_persistence`'s, shared with the sibling
/// catalog routes. Every class is logged and none is served.
fn read_failure(error: &VariantReadError, instance: &str) -> Response {
  match error {
    VariantReadError::UnknownFilterCode(filter) => {
      tracing::debug!(filter, "variant filter named an unknown reference code");
      ApiProblem::validation_failed(instance).into_response()
    }
    VariantReadError::Persistence(failure) => read_failure_at(failure, instance, "list_variants"),
  }
}

/// A persistence failure from a named step: either filter resolution or the
/// collection read.
fn read_failure_at(error: &PersistenceError, instance: &str, operation: &str) -> Response {
  tracing::warn!(code = error.code(), operation, "variant read failed");
  ApiProblem::from_persistence(error, instance).into_response()
}

/// The reference-code filters, parsed into the domain types the port takes.
///
/// A bundle rather than five bindings because the handler needs all of them or
/// none: any one failing to parse is the same single refusal, so there is
/// nothing for the caller to distinguish and nothing to report per field.
struct FilterCodes {
  variant_type: Option<LookupCode>,
  service_status: Option<LookupCode>,
  landing_gear_type: Option<LookupCode>,
  propulsion_category: Option<LookupCode>,
  country_of_origin: Option<CountryCode>,
}

/// `None` when any supplied code is not one its domain type accepts.
///
/// The four lookup codes share a parser and the country does not, which is why
/// this is one function over the query rather than a loop over pairs: the types
/// differ, and a trait to unify them would be an abstraction over two shapes
/// that already have names.
fn parse_codes(query: &VariantListQuery) -> Option<FilterCodes> {
  let lookup = |value: Option<&String>| -> Option<Option<LookupCode>> {
    value.map(String::as_str).map(LookupCode::try_from).transpose().ok()
  };

  Some(FilterCodes {
    variant_type: lookup(query.variant_type.as_ref())?,
    service_status: lookup(query.service_status.as_ref())?,
    landing_gear_type: lookup(query.landing_gear_type.as_ref())?,
    propulsion_category: lookup(query.propulsion_category.as_ref())?,
    country_of_origin: query
      .country_of_origin
      .as_deref()
      .map(CountryCode::try_from)
      .transpose()
      .ok()?,
  })
}

/// What this endpoint's cursor is bound to.
///
/// All nine filters in a fixed order, each rendered the same way whether or not
/// it was sent, so a cursor issued under one filter set is refused under any
/// other. The absent case is an empty value rather than an omitted term, because
/// an unstable normalization silently invalidates every outstanding cursor.
fn fingerprint(query: &VariantListQuery) -> FilterFingerprint {
  fn text(value: Option<&String>) -> &str {
    value.map(String::as_str).unwrap_or_default()
  }
  FilterFingerprint::of(&format!(
    "model={}&family={}&variant_type={}&service_status={}&landing_gear_type={}\
     &propulsion_category={}&country_of_origin={}&is_in_production={}&produced_in_year={}",
    text(query.model.as_ref()),
    text(query.family.as_ref()),
    text(query.variant_type.as_ref()),
    text(query.service_status.as_ref()),
    text(query.landing_gear_type.as_ref()),
    text(query.propulsion_category.as_ref()),
    text(query.country_of_origin.as_ref()),
    query.is_in_production.map(|value| value.to_string()).unwrap_or_default(),
    query.produced_in_year.map(|value| value.to_string()).unwrap_or_default(),
  ))
}

#[utoipa::path(
  get,
  path = "/v1/variants",
  tag = "catalog",
  summary = "List variants",
  description = "One keyset-paginated page of variants. All nine filters combine \
                 conjunctively. A model or family slug naming nothing is a 404; a \
                 reference code no vocabulary carries, a year outside 1900-2100, an \
                 unsupported sort, and a malformed cursor are 400. The reported \
                 limit is the one applied, after defaulting to 50 and capping at \
                 200. Summaries carry identity and lineage only; no specification \
                 measurement appears, canonical or pending.",
  params(
    ("model" = Option<String>, Query, description = "Parent model slug.", nullable = false),
    ("family" = Option<String>, Query, description = "Grandparent family slug.",
     nullable = false),
    ("variant_type" = Option<String>, Query, description = "Variant type code.",
     nullable = false),
    ("service_status" = Option<String>, Query, description = "Service status code.",
     nullable = false),
    ("landing_gear_type" = Option<String>, Query, description = "Landing gear type code.",
     nullable = false),
    ("propulsion_category" = Option<String>, Query, description = "Propulsion category code.",
     nullable = false),
    ("country_of_origin" = Option<String>, Query,
     description = "ISO 3166-1 alpha-3 country code, upper case.", nullable = false),
    ("is_in_production" = Option<bool>, Query,
     description = "Whether the variant is recorded as in production.", nullable = false),
    (
      "produced_in_year" = Option<i16>, Query, minimum = 1900, maximum = 2100,
      description = "Variants whose production run covered this year. A run with no \
                     end year is still in production and is included.",
      nullable = false
    ),
    ("sort" = Option<VariantSort>, Query,
     description = "Sort order. slug_asc is the only supported value.", nullable = false),
    (
      "limit" = Option<u16>, Query, minimum = 1,
      description = "Rows per page. Defaults to 50; values above 200 are capped to \
                     200, and the applied value is reported as the response's limit. \
                     A limit of 0 is refused.",
      nullable = false
    ),
    ("cursor" = Option<String>, Query,
     description = "Opaque continuation token from a previous page's next_cursor. \
                    Clients must not construct or interpret one.", nullable = false),
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "One page of variants, in slug order, with a null next_cursor on \
                     the final page and the applied limit",
      body = PageResponse<VariantSummaryResponse>,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 400,
      description = "An unsupported sort, an identifier that is not a slug, a \
                     reference code no vocabulary carries, a year outside the \
                     schema's range, or a cursor that is malformed, of another \
                     version, for another sort, or for different filters",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 404,
      description = "The model or family filter names no such row",
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
                     variants",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn list_variants(
  State(state): State<ApiState>,
  OriginalUri(uri): OriginalUri,
  ApiQuery(query): ApiQuery<VariantListQuery>,
) -> Response {
  let instance = uri.path();
  let sort = query.sort.unwrap_or_default();

  // Shape first, for every identifier the request carries, so nothing the
  // domain refuses reaches a lookup or a statement. One pass rather than seven
  // near-identical `let Ok(..) else` arms: every one of them is the same
  // refusal, and the only thing that differed was which parser ran.
  let Some(codes) = parse_codes(&query) else {
    return ApiProblem::validation_failed(instance).into_response();
  };
  let Ok(model_slug) = query.model.as_deref().map(Slug::try_from).transpose() else {
    return ApiProblem::validation_failed(instance).into_response();
  };
  let Ok(family_slug) = query.family.as_deref().map(Slug::try_from).transpose() else {
    return ApiProblem::validation_failed(instance).into_response();
  };

  // A year the schema's `year_value` domain could not hold is an invalid filter,
  // not a query that happens to match nothing.
  if query.produced_in_year.is_some_and(|year| !(EARLIEST_YEAR..=LATEST_YEAR).contains(&year)) {
    return ApiProblem::validation_failed(instance).into_response();
  }

  // Then the two parents, resolved to the keys the filter takes. A slug naming
  // nothing is a missing parent, which the accepted decision answers 404.
  let model = match model_slug {
    None => None,
    Some(slug) => match state.models.model_id(&slug).await {
      Ok(Some(id)) => Some(id),
      Ok(None) => return ApiProblem::not_found(instance).into_response(),
      Err(error) => return read_failure_at(&error, instance, "resolve_model"),
    },
  };
  let family = match family_slug {
    None => None,
    Some(slug) => match state.families.family_id(&slug).await {
      Ok(Some(id)) => Some(id),
      Ok(None) => return ApiProblem::not_found(instance).into_response(),
      Err(error) => return read_failure_at(&error, instance, "resolve_family"),
    },
  };

  let filters = fingerprint(&query);
  let (limit, resume) =
    match ListQuery::new(query.limit, query.cursor.clone()).into_page_request(filters, instance) {
      Ok(request) => request,
      Err(problem) => return problem.into_response(),
    };

  let after = match resume {
    None => None,
    Some(position) if position.sort != sort.code() => {
      return ApiProblem::validation_failed(instance).into_response();
    }
    Some(position) => match Slug::try_from(position.last_value.as_str()) {
      Ok(slug) => Some(slug),
      Err(_) => return ApiProblem::validation_failed(instance).into_response(),
    },
  };

  let filter = VariantFilter {
    model,
    family,
    variant_type: codes.variant_type,
    service_status: codes.service_status,
    landing_gear_type: codes.landing_gear_type,
    propulsion_category: codes.propulsion_category,
    country_of_origin: codes.country_of_origin,
    is_in_production: query.is_in_production,
    produced_in_year: query.produced_in_year,
  };

  match state.variants.list_variants(&filter, limit, after.as_ref()).await {
    Ok(page) => axum::Json(crate::pagination::render_page(
      page,
      sort.code(),
      filters,
      limit,
      VariantSummaryResponse::from,
    ))
    .into_response(),
    Err(error) => read_failure(&error, instance),
  }
}
