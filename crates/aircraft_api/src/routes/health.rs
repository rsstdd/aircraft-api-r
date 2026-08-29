use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
  pub status: &'static str,
}

#[utoipa::path(
  get,
  path = "/health",
  tag = "health",
  summary = "Check service health",
  description = "Report whether the process can serve HTTP requests.",
  responses(
    (status = 200, description = "Service is running", body = HealthResponse)
    )
)]
#[allow(clippy::unused_async)] // Axum handlers must return a future even when no work is awaited.
pub async fn health() -> Json<HealthResponse> {
  Json(HealthResponse { status: "ok" })
}
