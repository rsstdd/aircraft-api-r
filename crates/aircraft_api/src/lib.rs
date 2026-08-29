#![deny(clippy::as_conversions, clippy::indexing_slicing)]

pub mod routes;

use axum::{Router, routing::get};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
      title = "Aircraft Management Engine API",
      description = "HTTP contracts implemented by the Aircraft Management Engine",
      version = "0.1.0",
      license(name = "Proprietary"),
      contact(
        name = "Aircraft Management Engine maintainers",
        url = "https://github.com/rsstdd/aircraft-api-r"
      )
    ),
    servers((url = "/", description = "Current deployment origin")),
    paths(routes::health::health),
    components(schemas(routes::health::HealthResponse)),
    tags((name = "health", description = "Service health checks"))
)]
struct ApiDoc;

pub fn router() -> Router {
  Router::new().route("/health", get(routes::health::health))
}

#[must_use]
pub fn openapi() -> utoipa::openapi::OpenApi {
  ApiDoc::openapi()
}

#[cfg(test)]
mod tests {
  use anyhow::{Context, Result};
  use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
  };
  use serde_json::json;
  use tower::ServiceExt;

  use super::*;

  #[tokio::test]
  async fn health_route_returns_ok() -> Result<()> {
    let response = router().oneshot(Request::get("/health").body(Body::empty())?).await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await?;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&body)?, json!({ "status": "ok" }));
    Ok(())
  }

  #[test]
  fn openapi_uses_a_proprietary_license_without_an_spdx_identifier() -> Result<()> {
    let license = openapi().info.license.context("OpenAPI license should be present")?;

    assert_eq!(license.name, "Proprietary");
    assert!(license.identifier.is_none());
    Ok(())
  }

  #[test]
  fn openapi_operations_explain_their_contract() -> Result<()> {
    let document = serde_json::to_value(openapi())?;

    assert_eq!(
      document.pointer("/info/contact/url"),
      Some(&json!("https://github.com/rsstdd/aircraft-api-r"))
    );
    assert_eq!(document.pointer("/servers/0/url"), Some(&json!("/")));
    assert_eq!(
      document.pointer("/paths/~1health/get/summary"),
      Some(&json!("Check service health"))
    );
    assert_eq!(
      document.pointer("/paths/~1health/get/description"),
      Some(&json!("Report whether the process can serve HTTP requests."))
    );
    Ok(())
  }
}
