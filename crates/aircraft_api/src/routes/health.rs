use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::problem::ProblemDetails;

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
  params(
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "Service is running",
      body = HealthResponse,
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 400,
      description = "The request body could not be read",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 413,
      description = "The request body is larger than the perimeter accepts",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 503,
      description = "The service is at capacity, or is shutting down",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    ),
    (
      status = 504,
      description = "The handler did not answer within the perimeter deadline",
      body = ProblemDetails,
      content_type = "application/problem+json",
      headers(
        ("X-Request-Id" = String, description = "The correlation identifier for this \
         request, echoed from the client or generated here.")
      )
    )
  )
)]
#[allow(clippy::unused_async)] // Axum handlers must return a future even when no work is awaited.
pub async fn health() -> Json<HealthResponse> {
  Json(HealthResponse { status: "ok" })
}
