//! Problem documents, as defined by RFC 9457.
//!
//! Every field is a fixed string chosen at compile time. Nothing here is
//! derived from an error value, which is what keeps `detail` -- the field the
//! specification provides for an explanation, and therefore the one a database
//! diagnostic would leak through -- safe to publish. `aircraft_api` never sees
//! a connection string, but it does see `PersistenceError`, whose message
//! carries whatever `SQLx` reported.

use std::{borrow::Cow, collections::BTreeMap};

use axum::{
  Json,
  http::{StatusCode, header},
  response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::{
  IntoResponses, ToSchema,
  openapi::{
    HeaderBuilder, Ref, RefOr, ResponseBuilder, ResponsesBuilder, content::ContentBuilder,
    schema::ObjectBuilder,
  },
};

/// The media type RFC 9457 assigns to these documents. A client that
/// distinguishes problems from ordinary payloads keys on this, not on the
/// status code.
const PROBLEM_MEDIA_TYPE: &str = "application/problem+json";

#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
  /// A URI reference identifying the problem type. Relative by design: it
  /// resolves against the request URL, so no environment has to agree on a
  /// hostname for the link to be correct.
  #[serde(rename = "type")]
  pub kind: &'static str,
  pub title: &'static str,
  pub status: u16,
  pub detail: &'static str,
  /// The specific occurrence this problem describes.
  ///
  /// Required, because `/ready` has published one since it shipped and
  /// `oasdiff` fails the build on `response-property-became-optional`. A route
  /// names itself with a borrowed constant, a perimeter refusal owns the path
  /// it refused, and `Cow` is what lets one required field be both.
  #[schema(value_type = String)]
  pub instance: Cow<'static, str>,
}

impl ProblemDetails {
  /// The request body could not be read as a complete byte sequence.
  #[must_use]
  pub fn malformed_input(instance: &str) -> Self {
    Self {
      kind: "/problems/malformed-input",
      title: "Bad Request",
      status: 400,
      detail: "The request body could not be read.",
      instance: Self::bounded_instance(instance),
    }
  }

  /// The readiness probe could not confirm the database.
  ///
  /// One cause is reported for every failure -- unreachable, saturated, or too
  /// slow -- because the distinction is useful to an operator reading logs and
  /// useful to nobody on the far side of the socket.
  #[must_use]
  pub const fn database_unavailable() -> Self {
    Self {
      kind: "/problems/database-unavailable",
      title: "Service Unavailable",
      status: 503,
      detail: "The service cannot reach its database.",
      instance: Cow::Borrowed("/ready"),
    }
  }

  /// The process is draining and will not admit new work.
  ///
  /// Distinct from [`Self::database_unavailable`] on purpose: the two share a
  /// status code and nothing else. An operator reading a 503 during a rollout
  /// must be able to tell an orderly shutdown from a database outage without
  /// correlating logs.
  #[must_use]
  pub const fn shutting_down() -> Self {
    Self {
      kind: "/problems/shutting-down",
      title: "Service Unavailable",
      status: 503,
      detail: "The service is shutting down and is not accepting new work.",
      instance: Cow::Borrowed("/ready"),
    }
  }

  /// The drain window expired while this request was still in flight.
  ///
  /// Shares [`Self::shutting_down`]'s type because it is the same failure class,
  /// which `docs/architecture/http_v1_decisions.md` gives one stable type. Only
  /// `detail` separates being refused at the door from being cut off partway.
  #[must_use]
  pub fn shutdown_cancelled(instance: &str) -> Self {
    Self {
      kind: "/problems/shutting-down",
      title: "Service Unavailable",
      status: 503,
      detail: "The service stopped waiting for this request so it could shut down.",
      instance: Self::bounded_instance(instance),
    }
  }

  /// The request body is larger than the perimeter accepts.
  ///
  /// The perimeter answers before routing, so this and the two below name the
  /// refused path rather than a matched route.
  ///
  /// The detail names no limit: publishing the ceiling tells a caller probing
  /// for it exactly how much to send, and an operator has it in configuration.
  #[must_use]
  pub fn payload_too_large(instance: &str) -> Self {
    Self {
      kind: "/problems/payload-too-large",
      title: "Payload Too Large",
      status: 413,
      detail: "The request body is larger than this service accepts.",
      instance: Self::bounded_instance(instance),
    }
  }

  /// The service is already serving as many requests as it admits.
  ///
  /// A third `503` alongside [`Self::database_unavailable`] and
  /// [`Self::shutting_down`], and distinct from both for the same reason they
  /// are distinct from each other: an operator seeing a `503` must be able to
  /// tell overload from a database outage and from an orderly rollout without
  /// correlating logs. Shed load is the only one of the three a client can fix
  /// by retrying.
  #[must_use]
  pub fn overloaded(instance: &str) -> Self {
    Self {
      kind: "/problems/overloaded",
      title: "Service Unavailable",
      status: 503,
      detail: "The service is at capacity and refused this request rather than queueing it.",
      instance: Self::bounded_instance(instance),
    }
  }

  /// The handler did not answer within the perimeter deadline.
  ///
  /// `504` rather than `408`: a `408` says the *client* was too slow to send its
  /// request, which blames the wrong party for a handler that overran.
  #[must_use]
  pub fn deadline_exceeded(instance: &str) -> Self {
    Self {
      kind: "/problems/deadline-exceeded",
      title: "Gateway Timeout",
      status: 504,
      detail: "The service did not produce a response within its deadline.",
      instance: Self::bounded_instance(instance),
    }
  }

  /// The refused request's path, bounded before it is reflected back.
  ///
  /// The path is caller-controlled and `serde_json` escapes it, so this is a
  /// size and control-character bound rather than an injection guard -- the
  /// same 255-character ceiling `aircraft_ingest::sanitize_locator` uses.
  fn bounded_instance(path: &str) -> Cow<'static, str> {
    const MAX_INSTANCE_CHARS: usize = 255;

    Cow::Owned(
      path.chars().filter(|character| !character.is_control()).take(MAX_INSTANCE_CHARS).collect(),
    )
  }
}

impl IntoResponse for ProblemDetails {
  fn into_response(self) -> Response {
    // The status is carried in the document as well as the response line, and
    // RFC 9457 section 3.1 requires them to agree. Constructing it from the
    // field rather than alongside it is what keeps them from drifting apart.
    let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, [(header::CONTENT_TYPE, PROBLEM_MEDIA_TYPE)], Json(self)).into_response()
  }
}

/// The refusals the perimeter can answer *any* route with.
///
/// Declared once and referenced from every `#[utoipa::path]` rather than
/// repeated per route: these four statuses come from middleware that wraps the
/// whole router, so a route-by-route copy would let one route's contract drift
/// from what the perimeter actually does.
pub(crate) struct PerimeterResponses;

impl IntoResponses for PerimeterResponses {
  fn responses() -> BTreeMap<String, RefOr<utoipa::openapi::Response>> {
    /// Kept in step with the constructors above; each entry is one of them.
    const REFUSALS: [(&str, &str); 4] = [
      ("400", "The request body could not be read"),
      ("413", "The request body is larger than the perimeter accepts"),
      ("503", "The service is at capacity, or is shutting down"),
      ("504", "The handler did not answer within the perimeter deadline"),
    ];

    REFUSALS
      .into_iter()
      .fold(ResponsesBuilder::new(), |responses, (status, description)| {
        responses.response(
          status,
          ResponseBuilder::new()
            .description(description)
            .content(
              PROBLEM_MEDIA_TYPE,
              ContentBuilder::new().schema(Some(Ref::from_schema_name("ProblemDetails"))).build(),
            )
            .header(
              // The canonical spelling, matching what every route's 200 response
              // already publishes. `HeaderName` would lowercase it and make one
              // document disagree with itself.
              "X-Request-Id",
              HeaderBuilder::new()
                .schema(ObjectBuilder::new().schema_type(utoipa::openapi::schema::Type::String))
                .description(Some(REQUEST_ID_DESCRIPTION))
                .build(),
            ),
        )
      })
      .build()
      .into()
  }
}

/// The one description every correlated response gives for `X-Request-Id`.
const REQUEST_ID_DESCRIPTION: &str =
  "The correlation identifier for this request, echoed from the client or generated here.";

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use serde_json::json;

  use super::ProblemDetails;

  /// The published wire form of every problem this service can emit.
  ///
  /// One table so the contract is read in one place rather than reassembled
  /// from the behavioural tests, which assert only the status, type, and
  /// instance that tell one refusal from another. The expected values are
  /// literals a reviewer checks against
  /// `docs/architecture/http_v1_decisions.md`, not values re-derived from the
  /// constructors -- a copy of the code under test would pass for any contract,
  /// including a wrong one.
  ///
  /// The `instance` a perimeter problem carries is supplied by its caller, so
  /// `"/refused"` here stands for whatever path was refused.
  #[test]
  fn each_problem_serializes_to_its_published_document() {
    let cases = [
      (
        ProblemDetails::malformed_input("/refused"),
        json!({
          "type": "/problems/malformed-input",
          "title": "Bad Request",
          "status": 400,
          "detail": "The request body could not be read.",
          "instance": "/refused",
        }),
      ),
      (
        ProblemDetails::payload_too_large("/refused"),
        json!({
          "type": "/problems/payload-too-large",
          "title": "Payload Too Large",
          "status": 413,
          "detail": "The request body is larger than this service accepts.",
          "instance": "/refused",
        }),
      ),
      (
        ProblemDetails::overloaded("/refused"),
        json!({
          "type": "/problems/overloaded",
          "title": "Service Unavailable",
          "status": 503,
          "detail": "The service is at capacity and refused this request rather than queueing it.",
          "instance": "/refused",
        }),
      ),
      (
        ProblemDetails::deadline_exceeded("/refused"),
        json!({
          "type": "/problems/deadline-exceeded",
          "title": "Gateway Timeout",
          "status": 504,
          "detail": "The service did not produce a response within its deadline.",
          "instance": "/refused",
        }),
      ),
      (
        ProblemDetails::shutdown_cancelled("/refused"),
        json!({
          "type": "/problems/shutting-down",
          "title": "Service Unavailable",
          "status": 503,
          "detail": "The service stopped waiting for this request so it could shut down.",
          "instance": "/refused",
        }),
      ),
      (
        ProblemDetails::database_unavailable(),
        json!({
          "type": "/problems/database-unavailable",
          "title": "Service Unavailable",
          "status": 503,
          "detail": "The service cannot reach its database.",
          "instance": "/ready",
        }),
      ),
      (
        ProblemDetails::shutting_down(),
        json!({
          "type": "/problems/shutting-down",
          "title": "Service Unavailable",
          "status": 503,
          "detail": "The service is shutting down and is not accepting new work.",
          "instance": "/ready",
        }),
      ),
    ];

    for (problem, expected) in cases {
      let serialized = serde_json::to_value(&problem).expect("a problem document must serialize");
      assert_eq!(serialized, expected, "the published document changed");
    }
  }
}
