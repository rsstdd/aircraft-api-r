//! Problem documents, as defined by RFC 9457.
//!
//! Every field is a fixed string chosen at compile time. Nothing here is
//! derived from an error value, which is what keeps `detail` -- the field the
//! specification provides for an explanation, and therefore the one a database
//! diagnostic would leak through -- safe to publish. `aircraft_api` never sees
//! a connection string, but it does see `PersistenceError`, whose message
//! carries whatever `SQLx` reported.

use std::borrow::Cow;

use axum::{
  Json,
  http::{StatusCode, header},
  response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

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
  /// Always present. A route-owned problem names its route as a borrowed
  /// constant; a perimeter problem names the path of the request it refused,
  /// which it owns. `Cow` is what lets one required field be both.
  ///
  /// This field was briefly optional, so that a perimeter refusal -- which can
  /// answer any request -- would not have to name a route it could not know.
  /// That was a mistake: `/ready` has answered `503` with an `instance` since
  /// the readiness route shipped, and a shed or drain-cancelled `503` on that
  /// same route dropped it. `oasdiff` classifies that as
  /// `response-property-became-optional`, an error under the `fail-on: WARN`
  /// gate in `.github/workflows/ci.yml`, and it is a real break for any client
  /// that reads the field. Naming the actual request path answers the original
  /// objection honestly: it is the occurrence, not a guess at a route.
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
  /// The same problem *type* as [`Self::shutting_down`], because it is the same
  /// failure class -- the service is shutting down -- and
  /// `docs/architecture/http_v1_decisions.md` gives each class one stable type.
  /// The `detail` is what distinguishes being refused at the door from being
  /// cut off partway through; both name the request they answered.
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
  /// One of the three perimeter problems below. All three name the path of the
  /// request they refused rather than a route, because the boundary answers
  /// before routing is meaningful and has no matched route to cite. The
  /// correlation identifier in `X-Request-Id` is what ties one of these to a
  /// specific request, and it is present because `correlation::correlate`
  /// wraps the whole perimeter.
  ///
  /// The detail names no limit. Publishing the exact ceiling tells a caller
  /// probing for one precisely how much to send, and an operator who needs the
  /// number has it in configuration.
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
  /// The path is caller-controlled. `serde_json` escapes it on the way out, so
  /// this is not an injection guard; it is the size and control-character bound
  /// `AGENTS.md` requires of anything untrusted that reaches a response, and it
  /// uses the same 255-character ceiling `aircraft_ingest::sanitize_locator`
  /// applies to the other untrusted string this workspace echoes back.
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
