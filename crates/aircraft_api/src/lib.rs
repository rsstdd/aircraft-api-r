#![deny(clippy::as_conversions, clippy::indexing_slicing)]

mod correlation;
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
///
/// Layer order is load-bearing. The last layer applied is the outermost, so
/// correlation wraps the in-flight tracker: the bare `503` a shutdown
/// cancellation produces is stamped and recorded like any other response, and
/// so is the fallback `404`, which no route ever sees.
pub fn router(state: ApiState) -> Router {
  let shutdown = state.shutdown.clone();

  Router::new()
    .route("/health", get(routes::health::health))
    .route("/ready", get(routes::ready::ready))
    .route("/version", get(routes::version::version))
    .layer(axum::middleware::from_fn_with_state(shutdown, shutdown::track_in_flight))
    .layer(axum::middleware::from_fn(correlation::correlate))
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
    http::{HeaderValue, Request, StatusCode, header},
  };
  use serde_json::json;
  use tokio::sync::{Notify, mpsc};
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

  /// Parks inside the handler and never returns, so a cancellation can be
  /// issued while a request is provably in flight rather than after a race.
  struct ParksUntilCancelled {
    entered: mpsc::Sender<()>,
    release: Arc<Notify>,
  }

  #[async_trait]
  impl ReadinessProbe for ParksUntilCancelled {
    async fn check(&self) -> Result<(), PersistenceError> {
      let _ = self.entered.send(()).await;
      self.release.notified().await;
      Ok(())
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

  fn request_id_of(response: &axum::response::Response) -> Result<HeaderValue> {
    response
      .headers()
      .get(correlation::REQUEST_ID)
      .cloned()
      .context("the response carried no correlation identifier")
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

  /// Acceptance criteria 1 and 3. A caller that already has an identifier keeps
  /// it, which is the entire reason for accepting one: the string in its logs
  /// and the string in ours are then the same string.
  #[tokio::test]
  async fn a_valid_request_id_is_echoed_back() -> Result<()> {
    const SUPPLIED: &str = "9f8e7d6c-client-supplied";

    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(
        Request::get("/health").header(correlation::REQUEST_ID, SUPPLIED).body(Body::empty())?,
      )
      .await?;

    assert_eq!(request_id_of(&response)?, SUPPLIED);
    Ok(())
  }

  /// Acceptance criterion 2, for a request that supplied nothing.
  ///
  /// The replacement is checked against the rule clients are held to rather
  /// than against a shape spelled out here, so a generator that started
  /// emitting something this service would refuse fails the test instead of
  /// agreeing with it.
  #[tokio::test]
  async fn a_missing_request_id_is_generated_and_returned() -> Result<()> {
    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(Request::get("/health").body(Body::empty())?)
      .await?;

    let returned = request_id_of(&response)?;
    assert!(
      correlation::RequestId::accepted(&returned).is_some(),
      "a generated identifier must satisfy the rule clients are held to: {returned:?}"
    );
    Ok(())
  }

  /// Acceptance criterion 2, for the half a "fill in when absent" layer misses.
  ///
  /// Both assertions are load-bearing: the first fails if an over-long
  /// identifier is adopted, and the second if it is dropped instead of
  /// replaced, which would leave the response with no identifier at all.
  #[tokio::test]
  async fn an_invalid_request_id_is_replaced_rather_than_echoed() -> Result<()> {
    let supplied = "x".repeat(129);

    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(
        Request::get("/health").header(correlation::REQUEST_ID, &supplied).body(Body::empty())?,
      )
      .await?;

    let returned = request_id_of(&response)?;
    assert_ne!(returned, supplied.as_str(), "an over-long identifier must not be adopted");
    assert!(
      correlation::RequestId::accepted(&returned).is_some(),
      "the replacement must itself be usable: {returned:?}"
    );
    Ok(())
  }

  /// Both values are individually valid, so this fails only if the header is
  /// read with `get` rather than `get_all`. A service that took the first value
  /// would let a smuggled second one decide what its own logs say.
  #[tokio::test]
  async fn a_repeated_request_id_header_is_ambiguous_and_replaced() -> Result<()> {
    const FIRST: &str = "first-value";
    const SECOND: &str = "second-value";

    let response = router(state(Arc::new(AlwaysReady)))
      .oneshot(
        Request::get("/health")
          .header(correlation::REQUEST_ID, FIRST)
          .header(correlation::REQUEST_ID, SECOND)
          .body(Body::empty())?,
      )
      .await?;

    let returned = request_id_of(&response)?;
    assert_ne!(returned, FIRST, "an ambiguous identifier must not be adopted");
    assert_ne!(returned, SECOND, "an ambiguous identifier must not be adopted");
    Ok(())
  }

  /// Acceptance criterion 3, for every response that reaches a route or the
  /// fallback. The cancelled `503`, which reaches neither, has its own gate
  /// below.
  ///
  /// The probe fails throughout, so `/ready` answers with the problem document
  /// rather than the handler's own body, and the last path matches no route at
  /// all. Those two are the cases a handler-level mechanism would miss: neither
  /// response is written by a handler.
  #[tokio::test]
  async fn routed_and_unrouted_responses_carry_a_request_id() -> Result<()> {
    const PATHS: [&str; 4] = ["/health", "/ready", "/version", "/no-such-route"];

    for path in PATHS {
      let response = router(state(Arc::new(FailsWith("connection refused"))))
        .oneshot(Request::get(path).body(Body::empty())?)
        .await?;

      let returned = request_id_of(&response).with_context(|| format!("path {path}"))?;
      assert!(
        correlation::RequestId::accepted(&returned).is_some(),
        "path {path} returned an unusable identifier: {returned:?}"
      );
    }
    Ok(())
  }

  /// The proof for the layer order, which two doc comments call load-bearing
  /// and nothing verified.
  ///
  /// A shutdown cancellation is answered by `track_in_flight` itself, so no
  /// handler ever runs for it. Correlation is applied after that layer and is
  /// therefore outside it: swap the two `.layer` lines in `router` and this
  /// response loses its identifier while every other test still passes.
  ///
  /// Parking the handler is what makes the timing deterministic rather than a
  /// race between a fast route and the cancellation. The empty body is the
  /// second half: it distinguishes this `503` from `/ready`'s problem document,
  /// so the test cannot pass on the wrong `503`.
  #[tokio::test]
  async fn a_cancelled_request_still_carries_a_request_id() -> Result<()> {
    let shutdown = ShutdownState::new();
    let (entered_tx, mut entered) = mpsc::channel(1);
    let release = Arc::new(Notify::new());
    let probe = ParksUntilCancelled { entered: entered_tx, release };
    let mut state = state(Arc::new(probe));
    state.shutdown = shutdown.clone();

    let request = Request::get("/ready").body(Body::empty())?;
    let serving = tokio::spawn(router(state).oneshot(request));

    entered.recv().await.context("the request never reached the handler")?;
    shutdown.cancel();
    let response = serving.await?.context("the router failed to answer")?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let returned = request_id_of(&response)?;
    assert!(
      correlation::RequestId::accepted(&returned).is_some(),
      "a cancelled request must still be correlated: {returned:?}"
    );
    assert!(
      to_bytes(response.into_body(), 4096).await?.is_empty(),
      "forced cancellation must answer with a bodyless 503"
    );
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

  /// Correlation is contract in both directions: a generated client that does
  /// not know the header exists can neither send an identifier nor read the one
  /// it was given back, and correlation stops at this service's edge.
  #[test]
  fn openapi_publishes_the_request_id_header_in_both_directions() -> Result<()> {
    const RESPONSES: [(&str, &str); 4] =
      [("~1health", "200"), ("~1ready", "200"), ("~1ready", "503"), ("~1version", "200")];
    const PATHS: [&str; 3] = ["~1health", "~1ready", "~1version"];

    let document = serde_json::to_value(openapi())?;

    for (path, status) in RESPONSES {
      let pointer = format!("/paths/{path}/get/responses/{status}/headers/X-Request-Id");
      assert!(document.pointer(&pointer).is_some(), "missing {pointer}: {document}");
    }

    for path in PATHS {
      let parameters = document
        .pointer(&format!("/paths/{path}/get/parameters"))
        .and_then(serde_json::Value::as_array)
        .with_context(|| format!("{path} publishes no parameters"))?;
      assert!(
        parameters.iter().any(|parameter| {
          parameter.pointer("/name") == Some(&json!("X-Request-Id"))
            && parameter.pointer("/in") == Some(&json!("header"))
        }),
        "{path} does not accept a correlation identifier: {parameters:?}"
      );
    }
    Ok(())
  }
}
