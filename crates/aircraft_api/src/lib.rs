#![deny(clippy::as_conversions, clippy::indexing_slicing)]

mod correlation;
mod limits;
pub mod problem;
pub mod routes;
pub mod shutdown;

use std::sync::Arc;

use aircraft_app::readiness::ReadinessProbe;
use axum::{Router, routing::get};
use utoipa::OpenApi;

pub use crate::limits::{InvalidOrigin, PerimeterLimits};
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
///
/// The perimeter limits arrive the same way and for the same reason as the
/// readiness port: they are `aircraft_config`'s values, and this crate may not
/// depend on that crate.
#[derive(Clone)]
pub struct ApiState {
  pub readiness: Arc<dyn ReadinessProbe>,
  pub version: &'static str,
  pub build_commit: Option<&'static str>,
  pub shutdown: ShutdownState,
  pub limits: PerimeterLimits,
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
      .field("limits", &self.limits)
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
/// Layer order is load-bearing. The last layer applied is the outermost, so the
/// stack below reads innermost first and runs outermost first:
///
/// ```text
/// correlate                   stamps and traces every response, refusals included
/// refuse_oversized_preflight  the only bound CORS would otherwise swallow
/// CorsLayer                   a preflight is answered before a permit is taken
/// shed_when_saturated         bounds concurrent handlers and body buffers
/// track_in_flight             makes body reads and handlers cancellable on shutdown
/// enforce_deadline            bounds body reception and handler execution together
/// refuse_oversized_body       innermost, reads at most the configured body limit
/// ```
///
/// Three orderings carry weight. Correlation is outermost, so every response --
/// the fallback `404` and each perimeter refusal included -- is stamped and
/// traced. CORS sits above the semaphore because a preflight is a header-only
/// answer, but below the body limit for `OPTIONS` alone: it answers a preflight
/// without calling inward, so an oversized one would otherwise get `200`, while
/// every other method is refused *inside* it and so keeps the
/// `Access-Control-Allow-Origin` that makes the `413` readable. The semaphore
/// wraps body buffering so concurrency also bounds aggregate memory, and the
/// timeout wraps reception and execution so a slow upload cannot hold either.
pub fn router(state: ApiState) -> Router {
  let shutdown = state.shutdown.clone();
  let limits = state.limits.clone();

  Router::new()
    .route("/health", get(routes::health::health))
    .route("/ready", get(routes::ready::ready))
    .route("/version", get(routes::version::version))
    .layer(axum::middleware::from_fn_with_state(
      limits.body_bytes(),
      PerimeterLimits::refuse_oversized_body,
    ))
    .layer(axum::middleware::from_fn_with_state(
      limits.timeout(),
      PerimeterLimits::enforce_deadline,
    ))
    .layer(axum::middleware::from_fn_with_state(shutdown, shutdown::track_in_flight))
    .layer(axum::middleware::from_fn_with_state(
      limits.permits(),
      PerimeterLimits::shed_when_saturated,
    ))
    .layer(limits.cors_layer())
    .layer(axum::middleware::from_fn_with_state(
      limits.body_bytes(),
      PerimeterLimits::refuse_oversized_preflight,
    ))
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

  use std::{
    convert::Infallible,
    sync::{
      Arc,
      atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
  };

  use aircraft_app::ingestion::PersistenceError;
  use anyhow::{Context, Result};
  use async_trait::async_trait;
  use axum::{
    body::{Body, HttpBody as _, to_bytes},
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

  /// A perimeter wide enough that no test trips a bound it was not written for.
  ///
  /// Deliberately generous rather than disabled: a helper that turned the
  /// perimeter off would let a layer-ordering mistake pass every other test in
  /// this file. The numbers are not a claim about `aircraft_config`'s defaults,
  /// which are pinned by their own test over there.
  /// The perimeter a test starts from; override only the bound it exercises.
  ///
  /// Collapses the four-line `state_with_limits(.., limits_with(..))` pair the
  /// perimeter tests used to repeat, so each test shows the one value it is
  /// about rather than three defaults it is not.
  struct Perimeter {
    body_bytes: usize,
    timeout: Duration,
    max_concurrent: usize,
    origins: Vec<String>,
  }

  impl Default for Perimeter {
    fn default() -> Self {
      Self {
        body_bytes: 1_048_576,
        timeout: Duration::from_secs(30),
        max_concurrent: 256,
        origins: Vec::new(),
      }
    }
  }

  impl Perimeter {
    fn state(self, readiness: Arc<dyn ReadinessProbe>) -> ApiState {
      ApiState {
        readiness,
        version: "9.9.9-test",
        build_commit: None,
        shutdown: ShutdownState::new(),
        limits: PerimeterLimits::new(
          self.body_bytes,
          self.timeout,
          self.max_concurrent,
          &self.origins,
        )
        .expect("the test perimeter must be usable"),
      }
    }
  }

  fn state(readiness: Arc<dyn ReadinessProbe>) -> ApiState {
    Perimeter::default().state(readiness)
  }

  async fn body_of(response: axum::response::Response) -> Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), 4096).await?;
    Ok(serde_json::from_slice(&bytes)?)
  }

  /// Asserts a refusal by the parts that tell one refusal from another.
  ///
  /// The full wire form of every problem -- title and detail included -- is
  /// pinned once by `each_problem_serializes_to_its_published_document`, so a
  /// behavioural test only has to show that the *right* refusal answered: the
  /// status, the stable type, and the request it named. The media type is
  /// asserted here too, because `docs/architecture/http_v1_decisions.md`
  /// requires it and a client keys on it rather than on the status.
  async fn assert_refusal(
    response: axum::response::Response,
    status: u16,
    kind: &str,
    instance: &str,
  ) -> Result<()> {
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(
      response.headers().get(header::CONTENT_TYPE).context("a problem needs a media type")?,
      "application/problem+json",
      "RFC 9457 problem documents are not application/json"
    );

    let document = body_of(response).await?;
    assert_eq!(document.pointer("/type"), Some(&json!(kind)), "wrong problem type: {document}");
    assert_eq!(
      document.pointer("/instance"),
      Some(&json!(instance)),
      "a refusal must name the request it answered: {document}"
    );
    Ok(())
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
  /// race between a fast route and the cancellation. The document is the second
  /// half: it shares a problem type with `/ready`'s shutdown response but names
  /// no `instance`, so the test cannot pass on the wrong `503`.
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
    assert_refusal(response, 503, "/problems/shutting-down", "/ready").await
  }

  /// A perimeter that differs from [`permissive_limits`] in exactly one bound,
  /// so each test below names the bound it is about and inherits the rest.

  #[test]
  fn a_wildcard_origin_is_rejected_before_cors_layer_construction() {
    let origins = ["*".to_owned()];

    let error = PerimeterLimits::new(1_048_576, Duration::from_secs(30), 256, &origins)
      .expect_err("a wildcard must not reach AllowOrigin::list");

    assert_eq!(
      error.to_string(),
      "the configured CORS origin \"*\" is not usable in an explicit allow-list"
    );
  }

  /// Acceptance criterion 1.
  ///
  /// The route is `/ready` rather than `/health` because `/ready` is the one
  /// that consults the probe, so `NeverConsulted` is a real anti-vacuity guard
  /// here: if the limit did not fire, the handler would run and panic.
  ///
  /// `content-length` is set explicitly, and that is the point rather than test
  /// convenience: this pins the fast path that refuses a declared overflow
  /// before reading the body. The streamed case below pins the other path.
  #[tokio::test]
  async fn an_oversized_body_is_refused_before_a_handler_runs() -> Result<()> {
    const LIMIT: usize = 1024;

    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(
        Request::get("/ready")
          .header(header::CONTENT_LENGTH, LIMIT + 1)
          .body(Body::from(vec![b'a'; LIMIT + 1]))?,
      )
      .await?;

    assert_refusal(response, 413, "/problems/payload-too-large", "/ready").await
  }

  /// The undeclared half of the body limit. A streamed body has no upper size
  /// hint and the request carries no `Content-Length`, matching chunked HTTP
  /// framing rather than merely deleting the header from an in-memory body.
  #[tokio::test]
  async fn an_unknown_length_oversized_body_is_refused_before_a_handler_runs() -> Result<()> {
    const LIMIT: usize = 1024;

    let body = Body::from_stream(futures::stream::iter([
      Ok::<_, Infallible>(vec![b'a'; LIMIT]),
      Ok(vec![b'a']),
    ]));
    assert!(body.size_hint().upper().is_none(), "the test body must have unknown length");

    let request = Request::get("/ready").body(body)?;
    assert!(
      request.headers().get(header::CONTENT_LENGTH).is_none(),
      "the request must not declare its length"
    );

    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(NeverConsulted));
    let response = router(state).oneshot(request).await?;

    assert_refusal(response, 413, "/problems/payload-too-large", "/ready").await
  }

  /// The accepted side of the unknown-length boundary. Without it, a layer
  /// that rejected every chunked body without reading it would satisfy the
  /// overflow test above.
  #[tokio::test]
  async fn an_unknown_length_body_within_the_limit_reaches_the_handler() -> Result<()> {
    const LIMIT: usize = 1024;

    let body = Body::from_stream(futures::stream::iter([
      Ok::<_, Infallible>(vec![b'a'; LIMIT / 2]),
      Ok(vec![b'a'; LIMIT / 2]),
    ]));
    assert!(body.size_hint().upper().is_none(), "the test body must have unknown length");

    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(AlwaysReady));
    let response = router(state).oneshot(Request::get("/ready").body(body)?).await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
  }

  /// A transport failure is malformed input, not evidence that the configured
  /// size ceiling was crossed. Its diagnostic stays server-owned and static.
  #[tokio::test]
  async fn an_unreadable_body_is_answered_as_malformed_input() -> Result<()> {
    let body = Body::from_stream(futures::stream::once(async {
      Err::<Vec<u8>, _>(std::io::Error::other("fixture body failure"))
    }));
    let state =
      Perimeter { body_bytes: 1024, ..Perimeter::default() }.state(Arc::new(NeverConsulted));
    let response = router(state).oneshot(Request::get("/ready").body(body)?).await?;

    assert_refusal(response, 400, "/problems/malformed-input", "/ready").await
  }

  /// The declared-length fast path, as distinct from the streaming bound.
  ///
  /// Both refusals answer `413`, so the two tests around this one cannot tell
  /// them apart: delete the `content-length` comparison and the streaming bound
  /// still rejects an oversized body -- after buffering it. Acceptance criterion
  /// 1 says *before* full buffering, and this is the only test that pins the
  /// difference.
  ///
  /// The body panics if it is ever polled, so a single read of it fails the
  /// test rather than merely making it slower.
  #[tokio::test]
  async fn a_declared_oversized_body_is_refused_without_reading_it() -> Result<()> {
    const LIMIT: usize = 1024;

    let body =
      Body::from_stream(futures::stream::repeat_with(|| -> Result<Vec<u8>, Infallible> {
        panic!("the body must not be read once content-length already exceeds the limit")
      }));

    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(Request::get("/ready").header(header::CONTENT_LENGTH, LIMIT + 1).body(body)?)
      .await?;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    Ok(())
  }

  /// The accepted side of the boundary above. Paired with it, this fixes the
  /// limit at the configured length rather than somewhere below it: a layer
  /// installed with the wrong bound fails here instead of passing both.
  #[tokio::test]
  async fn a_request_within_the_body_limit_still_succeeds() -> Result<()> {
    const LIMIT: usize = 1024;

    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(AlwaysReady));

    let response = router(state)
      .oneshot(
        Request::get("/ready")
          .header(header::CONTENT_LENGTH, LIMIT)
          .body(Body::from(vec![b'a'; LIMIT]))?,
      )
      .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
  }

  /// Acceptance criterion 2.
  ///
  /// `Duration::ZERO` against a parked handler rather than a short deadline
  /// against a fast one: the outcome is decided by the layer rather than by a
  /// race, and the suite gains no sleep. Axum pins its own timeout behaviour
  /// the same way.
  ///
  /// The status is asserted as `504` specifically. `TimeoutLayer::new` answers
  /// `408`, which says the *client* was too slow to send its request -- a claim
  /// about the wrong party -- so a refactor back to the deprecated constructor
  /// has to fail here.
  #[tokio::test]
  async fn a_handler_past_the_deadline_is_answered_with_gateway_timeout() -> Result<()> {
    let (entered, _) = mpsc::channel(1);
    let probe = ParksUntilCancelled { entered, release: Arc::new(Notify::new()) };
    let state =
      Perimeter { timeout: Duration::ZERO, ..Perimeter::default() }.state(Arc::new(probe));

    let response = router(state).oneshot(Request::get("/ready").body(Body::empty())?).await?;

    assert_refusal(response, 504, "/problems/deadline-exceeded", "/ready").await
  }

  /// The anti-vacuity half: a layer that timed out unconditionally, or a
  /// deadline accidentally wired to zero for every request, fails here while
  /// passing the test above.
  #[tokio::test]
  async fn a_fast_handler_is_unaffected_by_the_deadline() -> Result<()> {
    let state = Perimeter::default().state(Arc::new(AlwaysReady));

    let response = router(state).oneshot(Request::get("/ready").body(Body::empty())?).await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
  }

  /// Drives one router to saturation and returns what the next request gets.
  ///
  /// Shared by the two tests below rather than written twice: the parking setup
  /// is the expensive half, and a second hand-maintained copy is exactly the
  /// drift that stops one of them catching a regression.
  ///
  /// The second request is raced against a deadline so that "queued" fails as a
  /// diagnosable assertion rather than as a hung test binary.
  async fn shed_response() -> Result<axum::response::Response> {
    let (entered_tx, mut entered) = mpsc::channel(1);
    let release = Arc::new(Notify::new());
    let probe = ParksUntilCancelled { entered: entered_tx, release: Arc::clone(&release) };
    let state = Perimeter { max_concurrent: 1, ..Perimeter::default() }.state(Arc::new(probe));

    let app = router(state);
    let parked = tokio::spawn(app.clone().oneshot(Request::get("/ready").body(Body::empty())?));
    entered.recv().await.context("the first request never reached the handler")?;

    let response = tokio::time::timeout(
      Duration::from_secs(5),
      app.oneshot(Request::get("/ready").body(Body::empty())?),
    )
    .await
    .context("the second request queued behind the parked handler instead of being shed")??;

    release.notify_one();
    let _ = parked.await;
    Ok(response)
  }

  /// Acceptance criterion 3.
  ///
  /// The empty body is the second half of the assertion: it distinguishes this
  /// `503` from `/ready`'s problem document, so the test cannot pass on the
  /// wrong `503`. The probe succeeds throughout, so a readiness failure is not
  /// an available explanation either.
  #[tokio::test]
  async fn excess_concurrency_is_shed_rather_than_queued() -> Result<()> {
    let response = shed_response().await?;

    assert_refusal(response, 503, "/problems/overloaded", "/ready").await
  }

  /// The shed layer sits inside correlation, so a refused request is still
  /// traceable. Move `correlate` inside it and this fails while every other
  /// perimeter test passes.
  #[tokio::test]
  async fn a_shed_request_still_carries_a_request_id() -> Result<()> {
    let response = shed_response().await?;

    let returned = request_id_of(&response)?;
    assert!(
      correlation::RequestId::accepted(&returned).is_some(),
      "a shed request must still be correlated: {returned:?}"
    );
    Ok(())
  }

  /// Both halves of the allow-list in one behaviour.
  ///
  /// A denied origin is not a status code: `CorsLayer` answers normally and
  /// simply omits `Access-Control-Allow-Origin`, leaving the browser to refuse
  /// the read. Asserting the header's absence is therefore the only way to
  /// observe a denial, and asserting the `200` alongside it stops a future
  /// `403` from being mistaken for correct behaviour.
  #[tokio::test]
  async fn an_allowed_origin_is_echoed_and_an_unlisted_one_is_not() -> Result<()> {
    const ALLOWED: &str = "https://app.example.com";
    const UNLISTED: &str = "https://not.example.com";

    let origins = [ALLOWED.to_owned()];
    let state =
      Perimeter { origins: origins.to_vec(), ..Perimeter::default() }.state(Arc::new(AlwaysReady));
    let app = router(state);

    let allowed = app
      .clone()
      .oneshot(Request::get("/ready").header(header::ORIGIN, ALLOWED).body(Body::empty())?)
      .await?;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(
      allowed
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .context("a listed origin must be echoed")?,
      ALLOWED
    );

    let unlisted = app
      .oneshot(Request::get("/ready").header(header::ORIGIN, UNLISTED).body(Body::empty())?)
      .await?;
    assert_eq!(unlisted.status(), StatusCode::OK, "a denial is a missing header, not a status");
    assert!(
      unlisted.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
      "an unlisted origin must not be echoed: {:?}",
      unlisted.headers()
    );
    Ok(())
  }

  /// The preflight short-circuit.
  ///
  /// `NeverConsulted` proves no handler ran, and the response headers pin the
  /// method and request-header policy rather than merely proving some CORS layer
  /// answered.
  #[tokio::test]
  async fn a_preflight_request_is_answered_without_reaching_a_handler() -> Result<()> {
    const ALLOWED: &str = "https://app.example.com";

    let origins = [ALLOWED.to_owned()];
    let state = Perimeter { origins: origins.to_vec(), ..Perimeter::default() }
      .state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(
        Request::options("/ready")
          .header(header::ORIGIN, ALLOWED)
          .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
          .header(header::ACCESS_CONTROL_REQUEST_HEADERS, correlation::REQUEST_ID.as_str())
          .body(Body::empty())?,
      )
      .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .context("a preflight must name the allowed origin")?,
      ALLOWED
    );
    assert_eq!(
      response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .context("a preflight must name the allowed methods")?,
      "GET,HEAD"
    );
    assert_eq!(
      response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .context("a preflight must allow the requested correlation header")?,
      "content-type,x-request-id"
    );
    Ok(())
  }

  /// A perimeter refusal stays readable to the cross-origin caller it refuses.
  ///
  /// The `413` is raised inside `CorsLayer`, so it leaves with
  /// `Access-Control-Allow-Origin` and a browser can read the problem document.
  /// Refusing the same request outside CORS would answer the same status with no
  /// CORS header, which a browser surfaces as an opaque network error — the
  /// RFC 9457 body would be published and then be unreadable by the one caller
  /// it was written for. This is why
  /// [`PerimeterLimits::refuse_oversized_preflight`] is restricted to `OPTIONS`
  /// rather than refusing every oversized request at the outer edge.
  #[tokio::test]
  async fn a_cross_origin_refusal_still_carries_its_cors_header() -> Result<()> {
    const ALLOWED: &str = "https://app.example.com";
    const LIMIT: usize = 1024;

    let origins = [ALLOWED.to_owned()];
    let state = Perimeter { body_bytes: LIMIT, origins: origins.to_vec(), ..Perimeter::default() }
      .state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(
        Request::get("/ready")
          .header(header::ORIGIN, ALLOWED)
          .header(header::CONTENT_LENGTH, LIMIT + 1)
          .body(Body::empty())?,
      )
      .await?;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
      response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .context("a refused cross-origin request must still name the allowed origin")?,
      ALLOWED
    );
    Ok(())
  }

  /// A preflight is a header-only question, so it may not smuggle a body past
  /// the perimeter's byte ceiling.
  ///
  /// `CorsLayer` answers any `OPTIONS` on the method alone and drops the body
  /// without reading it (`tower-http-0.7.0/src/cors/mod.rs:715`), so the
  /// declared-length refusal has to sit outside it. The panicking body is the
  /// anti-vacuity guard: it proves the refusal is driven by `content-length`
  /// rather than by reading, which is the same property
  /// `a_declared_oversized_body_is_refused_without_reading_it` pins for a `GET`.
  #[tokio::test]
  async fn a_preflight_declaring_an_oversized_body_is_refused() -> Result<()> {
    const ALLOWED: &str = "https://app.example.com";
    const LIMIT: usize = 1024;

    let body =
      Body::from_stream(futures::stream::repeat_with(|| -> Result<Vec<u8>, Infallible> {
        panic!("a preflight body must never be read")
      }));

    let origins = [ALLOWED.to_owned()];
    let state = Perimeter { body_bytes: LIMIT, origins: origins.to_vec(), ..Perimeter::default() }
      .state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(
        Request::options("/ready")
          .header(header::ORIGIN, ALLOWED)
          .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
          .header(header::CONTENT_LENGTH, LIMIT + 1)
          .body(body)?,
      )
      .await?;

    assert_refusal(response, 413, "/problems/payload-too-large", "/ready").await
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

  /// Perimeter middleware can answer before every handler, so every operation
  /// must publish every perimeter failure a generated client can receive.
  #[test]
  fn openapi_publishes_every_perimeter_response_for_every_route() -> Result<()> {
    const PATHS: [&str; 3] = ["~1health", "~1ready", "~1version"];
    const STATUSES: [&str; 4] = ["400", "413", "503", "504"];

    let document = serde_json::to_value(openapi())?;

    for path in PATHS {
      for status in STATUSES {
        let pointer = format!("/paths/{path}/get/responses/{status}");
        assert!(document.pointer(&pointer).is_some(), "missing {pointer}: {document}");
      }
    }
    Ok(())
  }

  /// A perimeter problem names the request it actually refused.
  ///
  /// The path here is deliberately not a route: the perimeter answers before
  /// routing, so it must report what was asked for rather than a matched route.
  /// This is the anti-vacuity guard for every other problem assertion in this
  /// file -- they all request `/ready`, so a `bounded_instance` that returned a
  /// constant would satisfy them and fail only here.
  #[tokio::test]
  async fn a_perimeter_problem_names_the_refused_request_path() -> Result<()> {
    const LIMIT: usize = 1024;
    const UNROUTED: &str = "/not-a-route";

    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(
        Request::get(UNROUTED).header(header::CONTENT_LENGTH, LIMIT + 1).body(Body::empty())?,
      )
      .await?;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), 4096).await?;
    let document: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
      document.pointer("/instance"),
      Some(&json!(UNROUTED)),
      "the problem must name the path that was refused: {document}"
    );
    Ok(())
  }

  /// The reflected path is bounded before it reaches the response body.
  ///
  /// The path is caller-controlled, so an unbounded copy would let a caller
  /// choose the size of the document this service emits. `AGENTS.md` requires a
  /// ceiling on anything untrusted that reaches a response.
  #[tokio::test]
  async fn a_reflected_request_path_is_bounded() -> Result<()> {
    const LIMIT: usize = 1024;
    const MAX_INSTANCE_CHARS: usize = 255;

    let long_path = format!("/{}", "a".repeat(4096));
    let state =
      Perimeter { body_bytes: LIMIT, ..Perimeter::default() }.state(Arc::new(NeverConsulted));

    let response = router(state)
      .oneshot(
        Request::get(&long_path).header(header::CONTENT_LENGTH, LIMIT + 1).body(Body::empty())?,
      )
      .await?;

    let body = to_bytes(response.into_body(), 4096).await?;
    let document: serde_json::Value = serde_json::from_slice(&body)?;
    let instance = document
      .pointer("/instance")
      .and_then(serde_json::Value::as_str)
      .context("the problem names no instance")?;

    assert_eq!(
      instance.chars().count(),
      MAX_INSTANCE_CHARS,
      "an oversized path must be truncated, not echoed whole"
    );
    Ok(())
  }

  /// `instance` is a required part of the published problem contract.
  ///
  /// `/ready` has answered `503` with an `instance` since the readiness route
  /// shipped. Making the field optional so perimeter problems could omit it
  /// broke that: `oasdiff` reports `response-property-became-optional` for
  /// `GET /ready` `503`, an error under the `fail-on: WARN` gate in
  /// `.github/workflows/ci.yml`. Every problem now names the request it
  /// answered, so the field is required again.
  ///
  /// Both halves are load-bearing: the first fails if the field is dropped from
  /// `required`, the second if it is published as nullable -- and a nullable
  /// `instance` would describe a `null` this server never emits.
  #[test]
  fn the_problem_instance_is_published_as_required_and_never_nullable() -> Result<()> {
    let document = serde_json::to_value(openapi())?;
    let schema = document
      .pointer("/components/schemas/ProblemDetails")
      .context("ProblemDetails is not published")?;

    let required = schema
      .pointer("/required")
      .and_then(serde_json::Value::as_array)
      .context("ProblemDetails publishes no required list")?;
    assert!(
      required.contains(&json!("instance")),
      "instance must stay required; /ready's 503 has always carried it: {required:?}"
    );

    assert_eq!(
      schema.pointer("/properties/instance/type"),
      Some(&json!("string")),
      "instance must not be published as nullable: {schema}"
    );
    Ok(())
  }

  /// The whole of the accepted problem-response rule, read off the generated
  /// document rather than a hand-kept list.
  ///
  /// `docs/architecture/http_v1_decisions.md` requires every API-originated
  /// `4xx` or `5xx` to use `application/problem+json` and to carry a request ID.
  /// Iterating the document is what makes this hold for routes that do not exist
  /// yet: a new operation, or a new failure status on an existing one, is
  /// checked the moment it is published rather than when somebody remembers to
  /// extend a list.
  ///
  /// The count is the anti-vacuity guard. Every assertion below lives inside
  /// three nested loops, so a document that published no failure response at all
  /// -- or a pointer typo that matched nothing -- would pass every one of them
  /// by never running.
  #[test]
  fn every_published_failure_is_a_problem_document_carrying_a_request_id() -> Result<()> {
    let document = serde_json::to_value(openapi())?;
    let paths =
      document.pointer("/paths").and_then(serde_json::Value::as_object).context("no paths")?;
    let mut checked = 0_usize;

    for (path, item) in paths {
      let operations = item.as_object().with_context(|| format!("{path} is not an object"))?;
      for (method, operation) in operations {
        let responses = operation
          .pointer("/responses")
          .and_then(serde_json::Value::as_object)
          .with_context(|| format!("{method} {path} publishes no responses"))?;

        for (status, response) in responses {
          if !status.starts_with('4') && !status.starts_with('5') {
            continue;
          }
          let where_ = format!("{method} {path} -> {status}");

          assert!(
            response.pointer("/content/application~1problem+json").is_some(),
            "{where_} is not published as a problem document: {response}"
          );
          assert!(
            response.pointer("/content/application~1json").is_none(),
            "{where_} also publishes application/json, which is not the problem contract"
          );
          assert!(
            response.pointer("/headers/X-Request-Id").is_some(),
            "{where_} publishes no correlation identifier: {response}"
          );
          checked += 1;
        }
      }
    }

    assert!(checked >= 9, "only {checked} failure responses were examined; the loops are vacuous");
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
