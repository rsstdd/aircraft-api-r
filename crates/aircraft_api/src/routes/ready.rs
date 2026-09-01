use axum::{
  Json,
  extract::State,
  response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
  ApiState,
  problem::{PerimeterResponses, ProblemDetails},
};

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadyResponse {
  pub status: &'static str,
}

#[utoipa::path(
  get,
  path = "/ready",
  tag = "health",
  summary = "Check readiness to serve database-backed traffic",
  description = "Report whether the database is reachable and answering statements.",
  params(
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "The database answered",
      body = ReadyResponse,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    PerimeterResponses,
    (
      status = 503,
      description = "The service is draining, is at capacity, or could not reach \
                     the database within the probe deadline",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
  )
)]
pub async fn ready(State(state): State<ApiState>) -> Response {
  // Checked before the probe, not after. A process that has begun draining is
  // unready whatever the database says, and spending the probe deadline on a
  // connection it is about to drop only delays the answer a load balancer is
  // waiting for.
  if state.shutdown.is_draining() {
    return ProblemDetails::shutting_down().into_response();
  }

  match state.readiness.check().await {
    Ok(()) => (Json(ReadyResponse { status: "ready" })).into_response(),
    Err(error) => {
      // The failure class is logged and not served. An operator correlating a
      // failing probe with its cause reads it here; the client gets a document
      // that cannot carry a diagnostic.
      tracing::warn!(code = error.code(), "readiness probe failed");
      ProblemDetails::database_unavailable().into_response()
    }
  }
}
