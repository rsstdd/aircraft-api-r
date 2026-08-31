#![deny(clippy::as_conversions, clippy::indexing_slicing)]

pub mod problem;
pub mod routes;
pub mod shutdown;

use std::sync::Arc;

use aircraft_app::readiness::ReadinessProbe;
use axum::{Router, routing::get};
use utoipa::OpenApi;

use crate::shutdown::ShutdownState;

/// What the router needs from its composition root.
///
/// The readiness probe arrives as a port rather than a pool because
/// `cargo run -p xtask -- boundaries` refuses `aircraft_db` and `SQLx` here;
/// the build identity arrives as data because this crate has no business
/// knowing which binary embedded it.
///
/// The shutdown state is shared with the composition root rather than owned
/// here: the router reads it to refuse new work, and `aircraft_server::serve`
/// writes it when a signal arrives.
#[derive(Clone)]
pub struct ApiState {
  pub readiness: Arc<dyn ReadinessProbe>,
  pub version: &'static str,
  pub build_commit: Option<&'static str>,
  pub shutdown: ShutdownState,
}

/// Written by hand because a trait object cannot derive it, and requiring
/// `Debug` on the port would put a formatting concern in the application layer.
/// The probe is named rather than rendered; there is nothing in it to print.
impl std::fmt::Debug for ApiState {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ApiState")
      .field("version", &self.version)
      .field("build_commit", &self.build_commit)
      .field("draining", &self.shutdown.is_draining())
      .finish_non_exhaustive()
  }
}

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
    paths(routes::health::health, routes::ready::ready, routes::version::version),
    components(schemas(
      routes::health::HealthResponse,
      routes::ready::ReadyResponse,
      routes::version::VersionResponse,
      problem::ProblemDetails
    )),
    tags((name = "health", description = "Service health checks"))
)]
struct ApiDoc;

/// Builds the router.
///
/// `/health` takes no state at all, which is what keeps it answerable while the
/// database is down: a liveness probe that consults the database reports the
/// process dead whenever its dependency is, and gets the process killed for
/// someone else's outage.
pub fn router(state: ApiState) -> Router {
  let shutdown = state.shutdown.clone();

  Router::new()
    .route("/health", get(routes::health::health))
    .route("/ready", get(routes::ready::ready))
    .route("/version", get(routes::version::version))
    .layer(axum::middleware::from_fn_with_state(shutdown, shutdown::track_in_flight))
    .with_state(state)
}

#[must_use]
pub fn openapi() -> utoipa::openapi::OpenApi {
  ApiDoc::openapi()
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, and the probe below fails by
  // panicking on a call that must never happen.
  #![allow(clippy::expect_used, clippy::panic)]

  use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
  };

  use aircraft_app::ingestion::PersistenceError;
  use anyhow::{Context, Result};
  use async_trait::async_trait;
  use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
  };
  use serde_json::json;
  use tower::ServiceExt;

  use super::*;

  struct AlwaysReady;

  #[async_trait]
  impl ReadinessProbe for AlwaysReady {
    async fn check(&self) -> Result<(), PersistenceError> {
      Ok(())
    }
  }

  /// Fails carrying `message`, so a test can prove what does and does not reach
  /// the client.
  struct FailsWith(&'static str);

  #[async_trait]
  impl ReadinessProbe for FailsWith {
    async fn check(&self) -> Result<(), PersistenceError> {
      Err(PersistenceError::Database {
        code: "DATABASE_UNAVAILABLE".to_owned(),
        message: self.0.to_owned(),
      })
    }
  }

  /// Panics if consulted. This is what makes criterion 1 falsifiable: a
  /// `/health` that grew a database check would fail here rather than pass by
  /// happening not to have one.
  struct NeverConsulted;

  #[async_trait]
  impl ReadinessProbe for NeverConsulted {
    async fn check(&self) -> Result<(), PersistenceError> {
      panic!("the readiness probe must not be consulted here");
    }
  }

  /// Reads the in-flight count from inside a running handler, which is the only
  /// place the counter is ever non-zero.
  struct RecordsInFlight {
    shutdown: ShutdownState,
    seen: Arc<AtomicUsize>,
  }

  #[async_trait]
  impl ReadinessProbe for RecordsInFlight {
    async fn check(&self) -> Result<(), PersistenceError> {
      self.seen.store(self.shutdown.in_flight(), Ordering::Release);
      Ok(())
    }
  }

  fn state(readiness: Arc<dyn ReadinessProbe>) -> ApiState {
    ApiState {
      readiness,
      version: "9.9.9-test",
      build_commit: None,
      shutdown: ShutdownState::new(),
    }
  }

  async fn body_of(response: axum::response::Response) -> Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), 4096).await?;
    Ok(serde_json::from_slice(&bytes)?)
  }

  #[tokio::test]
  async fn health_route_returns_ok() -> Result<()> {
    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(Request::get("/health").body(Body::empty())?)
      .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_of(response).await?, json!({ "status": "ok" }));
    Ok(())
  }

  #[tokio::test]
  async fn health_answers_without_consulting_the_readiness_probe() -> Result<()> {
    let response = router(state(Arc::new(NeverConsulted)))
      .oneshot(Request::get("/health").body(Body::empty())?)
      .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
  }

  #[tokio::test]
  async fn a_reachable_database_answers_ready() -> Result<()> {
    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(Request::get("/ready").body(Body::empty())?)
      .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_of(response).await?, json!({ "status": "ready" }));
    Ok(())
  }

  #[tokio::test]
  async fn an_unreachable_database_answers_a_problem_document() -> Result<()> {
    let response = router(state(Arc::new(FailsWith("connection refused"))))
      .oneshot(Request::get("/ready").body(Body::empty())?)
      .await?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
      response.headers().get(header::CONTENT_TYPE).context("a problem needs a media type")?,
      "application/problem+json",
      "RFC 9457 problem documents are not application/json"
    );
    assert_eq!(
      body_of(response).await?,
      json!({
        "type": "/problems/database-unavailable",
        "title": "Service Unavailable",
        "status": 503,
        "detail": "The service cannot reach its database.",
        "instance": "/ready"
      })
    );
    Ok(())
  }

  /// The redaction guard for the `detail` field. RFC 9457 gives a problem
  /// document somewhere to put an explanation, which is exactly where a
  /// database diagnostic would leak if `detail` were ever derived from the
  /// error instead of fixed.
  #[tokio::test]
  async fn a_readiness_failure_never_reaches_the_client() -> Result<()> {
    const SENTINEL: &str = "postgres://curator:hunter2@db.internal:5432/aircraft";

    let response = router(state(Arc::new(FailsWith(SENTINEL))))
      .oneshot(Request::get("/ready").body(Body::empty())?)
      .await?;

    let rendered = body_of(response).await?.to_string();
    assert!(!rendered.contains(SENTINEL), "the probe error reached the client: {rendered}");
    assert!(!rendered.contains("hunter2"), "a credential reached the client: {rendered}");
    Ok(())
  }

  #[tokio::test]
  async fn version_reports_the_package_version() -> Result<()> {
    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(Request::get("/version").body(Body::empty())?)
      .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_of(response).await?, json!({ "version": "9.9.9-test" }));
    Ok(())
  }

  #[tokio::test]
  async fn version_reports_a_build_commit_when_one_was_compiled_in() -> Result<()> {
    let mut state = state(Arc::new(AlwaysReady));
    state.build_commit = Some("2f9c1ab");

    let response = router(state).oneshot(Request::get("/version").body(Body::empty())?).await?;

    assert_eq!(
      body_of(response).await?,
      json!({ "version": "9.9.9-test", "build_commit": "2f9c1ab" })
    );
    Ok(())
  }

  /// Acceptance criterion 1. `NeverConsulted` is the anti-vacuity guard: it
  /// panics when called, so this passes only if the drain check short-circuits
  /// *before* the probe rather than by some unrelated 503. The problem type is
  /// asserted literally because reusing `database_unavailable` would tell an
  /// operator a healthy database is down.
  #[tokio::test]
  async fn readiness_reports_shutting_down_once_draining_begins() -> Result<()> {
    let state = state(Arc::new(NeverConsulted));
    state.shutdown.begin_draining();

    let response = router(state).oneshot(Request::get("/ready").body(Body::empty())?).await?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
      response.headers().get(header::CONTENT_TYPE).context("a problem needs a media type")?,
      "application/problem+json"
    );
    assert_eq!(
      body_of(response).await?,
      json!({
        "type": "/problems/shutting-down",
        "title": "Service Unavailable",
        "status": 503,
        "detail": "The service is shutting down and is not accepting new work.",
        "instance": "/ready"
      })
    );
    Ok(())
  }

  /// The count the expiry warning reports. Both assertions are load-bearing:
  /// the first fails if the layer never counts, and the second fails if the
  /// decrement is dropped -- a counter that only rises would report every
  /// request ever served as cancelled.
  #[tokio::test]
  async fn a_request_is_counted_while_its_handler_runs_and_not_after() -> Result<()> {
    let shutdown = ShutdownState::new();
    let seen = Arc::new(AtomicUsize::new(0));
    let probe = RecordsInFlight { shutdown: shutdown.clone(), seen: Arc::clone(&seen) };
    let mut state = state(Arc::new(probe));
    state.shutdown = shutdown.clone();

    let response = router(state).oneshot(Request::get("/ready").body(Body::empty())?).await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(seen.load(Ordering::Acquire), 1, "the handler was not counted while it ran");
    assert_eq!(shutdown.in_flight(), 0, "the count was not released when the handler returned");
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

  /// The 503 is part of the published contract, and its media type is the part
  /// a generated client gets wrong if the annotation drifts back to
  /// `application/json`.
  #[test]
  fn openapi_publishes_the_readiness_problem_document() -> Result<()> {
    let document = serde_json::to_value(openapi())?;

    assert!(
      document
        .pointer("/paths/~1ready/get/responses/503/content/application~1problem+json")
        .is_some(),
      "the 503 must be published as a problem document: {document}"
    );
    assert!(document.pointer("/paths/~1version/get").is_some(), "/version must be published");
    Ok(())
  }
}
