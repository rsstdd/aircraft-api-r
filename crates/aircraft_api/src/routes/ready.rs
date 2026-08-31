use axum::{
  Json,
  extract::State,
  response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{ApiState, problem::ProblemDetails};

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
  responses(
    (status = 200, description = "The database answered", body = ReadyResponse),
    (
      status = 503,
      description = "The database could not be reached within the probe deadline",
      body = ProblemDetails,
      content_type = "application/problem+json"
    )
  )
)]
pub async fn ready(State(state): State<ApiState>) -> Response {
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
