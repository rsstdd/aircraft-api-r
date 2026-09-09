//! `GET /v1/reference/{catalog}`: the seeded lookup catalogs, conditionally.
//!
//! `docs/architecture/http_v1_decisions.md` § "Conditional requests and
//! reference catalogs" is the accepted contract and names this module in turn.
//! The allowlist is `aircraft_domain`'s [`Catalog`]; the rows come through
//! `aircraft_app`'s `CatalogReader` port, because this crate may not reach
//! `aircraft_db` or `SQLx`.

use aircraft_app::{ingestion::PersistenceError, reference::CatalogEntry};
use aircraft_domain::reference::Catalog;
use axum::{
  extract::{OriginalUri, Path, State},
  http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
  },
  response::{IntoResponse, Response},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::{
  ApiState,
  problem::{ApiProblem, PerimeterResponses, ProblemDetails},
};

/// How many entity-tags one request may be compared against.
///
/// `If-None-Match` is caller-supplied and its grammar is a list, so without a
/// ceiling a header of ten thousand commas would buy ten thousand comparisons.
/// Root `AGENTS.md` requires the bound to be explicit at the layer that reads
/// the input. A client with more than a few validators for one resource is
/// already outside anything this route issues.
const MAX_VALIDATORS: usize = 64;

/// What a catalog response says about a caching client's obligations.
///
/// `private` because the route is authenticated under `CatalogRead`, so a shared
/// cache must not retain the body; `no-cache` because a stored response must be
/// revalidated, which is what the `ETag` makes cheap.
const CATALOG_CACHE_CONTROL: HeaderValue = HeaderValue::from_static("private, no-cache");

/// One lookup row on the wire.
///
/// `description` is omitted when the row has none, or when the catalog's table
/// has no such column at all -- the rule the measurement representation follows,
/// and deliberately the opposite of `PageResponse::next_cursor`, which is `null`
/// on a final page so a paging client can read the member rather than test for
/// it. Nothing polls a catalog for a missing description.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogEntryResponse {
  /// The stable machine code, as `aircraft_ref.lookup_code` spells it.
  pub code: String,
  pub label: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
}

impl From<CatalogEntry> for CatalogEntryResponse {
  fn from(entry: CatalogEntry) -> Self {
    Self { code: entry.code, label: entry.label, description: entry.description }
  }
}

/// The strong validator for a serialized catalog.
///
/// Over the response bytes rather than over the rows, so the tag cannot describe
/// anything but what was sent. Strong, per the accepted decision: the rows are
/// totally ordered and one serializer renders them, so equal catalogs give equal
/// bytes.
fn entity_tag(body: &[u8]) -> String {
  use std::fmt::Write as _;

  let mut tag = String::with_capacity(66);
  tag.push('"');
  for byte in &Sha256::digest(body) {
    // Writing to a `String` cannot fail. The result is discarded rather than
    // unwrapped so that no formatting error can panic on a request path, which
    // is what `FilterFingerprint::to_hex` does and for the same reason. The two
    // stay separate: a cursor fingerprint and an entity-tag are different
    // contracts that happen to share an algorithm.
    let _ = write!(tag, "{byte:02x}");
  }
  tag.push('"');
  tag
}

/// Whether the request's `If-None-Match` names this entity.
///
/// The header is the list RFC 9110 defines, not a single value, and a recipient
/// may also have split it across field lines -- hence `get_all`. Comparison is
/// weak, as RFC 9110 § 13.1.2 requires of `If-None-Match`, so a client echoing
/// `W/"..."` still matches the strong tag this route issued. A non-ASCII header
/// simply does not match; it is not an error worth a distinct response.
fn if_none_match_matches(headers: &HeaderMap, etag: &str) -> bool {
  headers
    .get_all(IF_NONE_MATCH)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .take(MAX_VALIDATORS)
    .map(str::trim)
    .any(|candidate| candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag)
}

#[utoipa::path(
  get,
  path = "/v1/reference/{catalog}",
  tag = "reference",
  summary = "List a seeded reference catalog",
  description = "Serve one lookup catalog's active rows in its stable order. The \
                 catalog is named by an allowlisted slug; any other value is a 404. \
                 Responses carry a strong ETag, and a matching If-None-Match is \
                 answered 304 with no body.",
  params(
    (
      "catalog" = String, Path,
      description = "The catalog to read. Only the allowlisted slugs are served."
    ),
    ("If-None-Match" = Option<String>, Header, description = "Entity-tags from an \
     earlier response. A match is answered 304.", nullable = false),
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "The catalog's active rows, in its stable order",
      body = Vec<CatalogEntryResponse>,
      headers(
        ("ETag" = String, description = "Strong validator for this representation."),
        ("Cache-Control" = String, description = "private, no-cache."),
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 304,
      description = "The client's validator matches; no body is sent",
      headers(
        ("ETag" = String, description = "Strong validator for this representation."),
        ("Cache-Control" = String, description = "private, no-cache."),
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 404,
      description = "No catalog has that name",
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
                     catalog",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn catalog(
  State(state): State<ApiState>,
  OriginalUri(uri): OriginalUri,
  Path(requested): Path<String>,
  headers: HeaderMap,
) -> Response {
  let instance = uri.path();

  // First, and before anything is read: an unknown segment is refused here, so
  // no statement is ever selected for a name the allowlist does not hold. The
  // port takes a `Catalog` and not a string, so this is also the only way in.
  let Ok(catalog) = Catalog::try_from(requested.as_str()) else {
    return ApiProblem::not_found(instance).into_response();
  };

  let entries = match state.catalogs.entries(catalog).await {
    Ok(entries) => entries,
    Err(error) => {
      // The failure class is logged and not served, as `/ready` does: an
      // operator correlating the failure reads it here, and the client gets a
      // document that cannot carry a diagnostic.
      tracing::warn!(
        code = error.code(),
        catalog = catalog.slug(),
        "reference catalog read failed"
      );
      // The two variants are different answers, not one. A `Database` failure
      // is a dependency outage and `503` correctly invites a retry. An
      // `Invariant` failure means the database answered and the answer broke a
      // rule this service set -- today, a catalog past `MAX_CATALOG_ROWS` --
      // which is an internal fault no retry can clear, so it is the `500` the
      // problem table assigns. Collapsing them would tell an operator the
      // database is down while it is healthy.
      return match error {
        PersistenceError::Database { .. } => ApiProblem::database_unavailable(instance),
        PersistenceError::Invariant(_) => ApiProblem::internal(instance),
      }
      .into_response();
    }
  };

  let body: Vec<CatalogEntryResponse> =
    entries.into_iter().map(CatalogEntryResponse::from).collect();

  // Serialized once and reused for both the validator and the body, so the tag
  // cannot describe bytes other than the ones sent. Serializing owned `String`s
  // cannot fail; the arm exists because returning a typed refusal is cheaper
  // than arguing a panic, and an empty body would be a worse answer than a 500.
  let Ok(serialized) = serde_json::to_vec(&body) else {
    return ApiProblem::internal(instance).into_response();
  };

  let etag = entity_tag(&serialized);
  let Ok(validator) = HeaderValue::from_str(&etag) else {
    return ApiProblem::internal(instance).into_response();
  };

  let mut response_headers = HeaderMap::new();
  response_headers.insert(ETAG, validator);
  response_headers.insert(CACHE_CONTROL, CATALOG_CACHE_CONTROL);

  if if_none_match_matches(&headers, &etag) {
    return (StatusCode::NOT_MODIFIED, response_headers).into_response();
  }

  response_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
  (StatusCode::OK, response_headers, serialized).into_response()
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use super::*;

  /// Weak comparison, a list, and `*`, as RFC 9110 § 13.1.2 has it -- and the
  /// negatives, because a matcher that returned `true` for everything would
  /// satisfy the positives alone.
  #[test]
  fn a_validator_matches_weakly_anywhere_in_the_list() {
    let etag = "\"abc\"";
    let matching = ["\"abc\"", "W/\"abc\"", "*", "\"zzz\", \"abc\"", "  \"abc\"  ", "*, \"zzz\""];
    let others = ["\"abcd\"", "W/\"abcd\"", "abc", "\"\"", "W/W/\"abc\"", ""];

    for header in matching {
      let mut headers = HeaderMap::new();
      headers.insert(IF_NONE_MATCH, HeaderValue::from_static(""));
      headers.insert(IF_NONE_MATCH, HeaderValue::from_str(header).expect("ascii"));
      assert!(if_none_match_matches(&headers, etag), "{header:?} names {etag}");
    }
    for header in others {
      let mut headers = HeaderMap::new();
      headers.insert(IF_NONE_MATCH, HeaderValue::from_str(header).expect("ascii"));
      assert!(!if_none_match_matches(&headers, etag), "{header:?} does not name {etag}");
    }

    assert!(!if_none_match_matches(&HeaderMap::new(), etag), "an absent header names nothing");
  }

  /// Split across field lines rather than commas, which a recipient is allowed
  /// to see and `HeaderMap::get` alone would miss.
  #[test]
  fn a_validator_in_a_second_field_line_is_still_compared() {
    let mut headers = HeaderMap::new();
    headers.append(IF_NONE_MATCH, HeaderValue::from_static("\"other\""));
    headers.append(IF_NONE_MATCH, HeaderValue::from_static("\"abc\""));

    assert!(if_none_match_matches(&headers, "\"abc\""));
  }

  /// The scan is bounded, so a pathological header costs a fixed number of
  /// comparisons rather than one per comma.
  #[test]
  fn only_a_bounded_number_of_validators_is_compared() {
    let mut header = "\"miss\", ".repeat(MAX_VALIDATORS);
    header.push_str("\"abc\"");
    let mut headers = HeaderMap::new();
    headers.insert(IF_NONE_MATCH, HeaderValue::from_str(&header).expect("ascii"));

    assert!(
      !if_none_match_matches(&headers, "\"abc\""),
      "a validator past the ceiling is not reached"
    );
  }

  #[test]
  fn a_validator_is_a_quoted_digest_that_distinguishes_bodies() {
    let tag = entity_tag(b"[]");

    assert!(tag.starts_with('"') && tag.ends_with('"'), "an entity-tag is quoted: {tag}");
    assert_eq!(tag.len(), 66, "a quoted SHA-256 hex digest: {tag}");
    assert_eq!(tag, entity_tag(b"[]"), "equal bytes give equal validators");
    assert_ne!(tag, entity_tag(b"[ ]"), "different bytes give different validators");
  }
}
