pub mod dto;
pub mod routes;

use axum::{Router, routing::get};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Aircraft Management Engine API",
        description = "HTTP contracts implemented by the Aircraft Management Engine",
        version = "0.1.0"
    ),
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
    use anyhow::Result;
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
}
