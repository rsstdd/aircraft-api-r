use axum::{Json, extract::State};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{ApiState, problem::ProblemDetails};

#[derive(Debug, Serialize, ToSchema)]
pub struct VersionResponse {
  pub version: &'static str,
  /// Absent rather than null when the binary was built without a commit
  /// stamped in, so a deployment that cannot report one is distinguishable
  /// from one reporting an empty value.
  #[serde(skip_serializing_if = "Option::is_none")]
  // `utoipa` renders any `Option<T>` as a nullable union without reading
  // `skip_serializing_if`, so without this the published contract would offer a
  // `null` this endpoint never emits.
  #[schema(nullable = false)]
  pub build_commit: Option<&'static str>,
}

#[utoipa::path(
  get,
  path = "/version",
  tag = "health",
  summary = "Report the running build",
  description = "Report the package version and, when it was stamped in at build time, the commit.",
  params(
    ("X-Request-Id" = Option<String>, Header, description = "Correlation identifier. \
     Adopted when it is 1-128 visible ASCII characters sent exactly once; otherwise \
     the service generates one.", nullable = false)
  ),
  responses(
    (
      status = 200,
      description = "The running build",
      body = VersionResponse,
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
pub async fn version(State(state): State<ApiState>) -> Json<VersionResponse> {
  Json(VersionResponse { version: state.version, build_commit: state.build_commit })
}
