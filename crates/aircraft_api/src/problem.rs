//! Problem documents, as defined by RFC 9457.
//!
//! Every field is a fixed string chosen at compile time. Nothing here is
//! derived from an error value, which is what keeps `detail` -- the field the
//! specification provides for an explanation, and therefore the one a database
//! diagnostic would leak through -- safe to publish. `aircraft_api` never sees
//! a connection string, but it does see `PersistenceError`, whose message
//! carries whatever `SQLx` reported.

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
  /// Optional, as RFC 9457 section 3.1 allows. A route-owned problem names its
  /// route; a perimeter problem names nothing, because the same rejection can
  /// answer any request and a fixed route here would be a false statement about
  /// which one was refused. Omitted from the wire rather than sent as `null`, so
  /// the two route problems that predate this field serialize exactly as before.
  ///
  /// `nullable = false` keeps the generated schema honest: `Option` would
  /// otherwise be published as `["string", "null"]`, describing a `null` this
  /// server never emits and telling a generated client to model one.
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  pub instance: Option<&'static str>,
}

impl ProblemDetails {
  /// The request body could not be read as a complete byte sequence.
  #[must_use]
  pub const fn malformed_input() -> Self {
    Self {
      kind: "/problems/malformed-input",
      title: "Bad Request",
      status: 400,
      detail: "The request body could not be read.",
      instance: None,
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
      instance: Some("/ready"),
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
      instance: Some("/ready"),
    }
  }

  /// The drain window expired while this request was still in flight.
  ///
  /// The same problem *type* as [`Self::shutting_down`], because it is the same
  /// failure class -- the service is shutting down -- and
  /// `docs/architecture/http_v1_decisions.md` gives each class one stable type.
  /// The `detail` and the absent `instance` are what distinguish being refused
  /// at the door from being cut off partway through, on a route this can answer
  /// for but cannot name.
  #[must_use]
  pub const fn shutdown_cancelled() -> Self {
    Self {
      kind: "/problems/shutting-down",
      title: "Service Unavailable",
      status: 503,
      detail: "The service stopped waiting for this request so it could shut down.",
      instance: None,
    }
  }

  /// The request body is larger than the perimeter accepts.
  ///
  /// One of the three perimeter problems below. All three name no `instance`:
  /// they are answers the boundary gives before routing is meaningful, so there
  /// is no single occurrence to point at. The correlation identifier in
  /// `X-Request-Id` is what ties one of these to a specific request, and it is
  /// present because `correlation::correlate` wraps the whole perimeter.
  ///
  /// The detail names no limit. Publishing the exact ceiling tells a caller
  /// probing for one precisely how much to send, and an operator who needs the
  /// number has it in configuration.
  #[must_use]
  pub const fn payload_too_large() -> Self {
    Self {
      kind: "/problems/payload-too-large",
      title: "Payload Too Large",
      status: 413,
      detail: "The request body is larger than this service accepts.",
      instance: None,
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
  pub const fn overloaded() -> Self {
    Self {
      kind: "/problems/overloaded",
      title: "Service Unavailable",
      status: 503,
      detail: "The service is at capacity and refused this request rather than queueing it.",
      instance: None,
    }
  }

  /// The handler did not answer within the perimeter deadline.
  ///
  /// `504` rather than `408`: a `408` says the *client* was too slow to send its
  /// request, which blames the wrong party for a handler that overran.
  #[must_use]
  pub const fn deadline_exceeded() -> Self {
    Self {
      kind: "/problems/deadline-exceeded",
      title: "Gateway Timeout",
      status: 504,
      detail: "The service did not produce a response within its deadline.",
      instance: None,
    }
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
