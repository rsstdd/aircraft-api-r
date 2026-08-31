use axum::{Json, extract::State};
use serde::Serialize;
use utoipa::ToSchema;

use crate::ApiState;

#[derive(Debug, Serialize, ToSchema)]
pub struct VersionResponse {
  pub version: &'static str,
  /// Absent rather than null when the binary was built without a commit
  /// stamped in, so a deployment that cannot report one is distinguishable
  /// from one reporting an empty value.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub build_commit: Option<&'static str>,
}

#[utoipa::path(
  get,
  path = "/version",
  tag = "health",
  summary = "Report the running build",
  description = "Report the package version and, when it was stamped in at build time, the commit.",
  responses((status = 200, description = "The running build", body = VersionResponse))
)]
#[allow(clippy::unused_async)] // Axum handlers must return a future even when no work is awaited.
pub async fn version(State(state): State<ApiState>) -> Json<VersionResponse> {
  Json(VersionResponse { version: state.version, build_commit: state.build_commit })
}
