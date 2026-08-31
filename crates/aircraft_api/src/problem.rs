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
  pub instance: &'static str,
}

impl ProblemDetails {
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
      instance: "/ready",
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
