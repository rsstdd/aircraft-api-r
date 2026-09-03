// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Trace-content gates for the correlation layer.
//!
//! These live in their own test binary rather than beside the router tests in
//! `src/lib.rs`, and the reason is mechanical. `tracing` resolves a callsite's
//! interest the first time that callsite is reached, from the dispatch of
//! whichever thread reached it, and caches the answer for the rest of the
//! process -- so a callsite first reached on a thread with no dispatch is
//! disabled permanently. The router tests attach no dispatch, and under
//! `cargo test` they would share this file's process. Cargo builds each
//! `tests/*.rs` as its own binary, so every future in this file carries a
//! subscriber and no sibling can silence one.
//!
//! `apps/server/tests/shutdown.rs` documents the same hazard and solves it the
//! other way, by handing every server future an explicit dispatch.

use std::{
  sync::{Arc, Mutex},
  time::Duration,
};

use aircraft_api::{
  ApiState, PerimeterLimits, authentication::require_authentication, problem::ApiProblem,
  router_with_routes, shutdown::ShutdownState,
};
use aircraft_app::{
  authentication::{
    AuthenticatedPrincipal, AuthenticationService, CredentialLookup, CredentialLookupRecord, Scope,
  },
  credential_issuance::CredentialVerifier,
  ingestion::PersistenceError,
  readiness::ReadinessProbe,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::{
  Extension, Router,
  body::{Body, to_bytes},
  http::{Request, StatusCode, header},
  response::{IntoResponse, Response},
  routing::get,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

struct AlwaysReady;

#[async_trait]
impl ReadinessProbe for AlwaysReady {
  async fn check(&self) -> Result<(), PersistenceError> {
    Ok(())
  }
}

/// Fails carrying `message`, so a test can prove what does and does not reach
/// the trace.
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

/// Collects formatted `tracing` output so a test can read an event back.
///
/// The same shape as the writer in `apps/server/tests/shutdown.rs`, duplicated
/// rather than shared: the only home for it would be `aircraft_testsupport`,
/// and taking a dev-dependency on a Docker-backed `PostgreSQL` harness to reach a
/// byte buffer costs far more than the twenty lines it saves.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
  fn contents(&self) -> String {
    String::from_utf8_lossy(&self.0.lock().expect("the log buffer lock is poisoned")).into_owned()
  }
}

impl std::io::Write for CapturedLogs {
  fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
    self.0.lock().expect("the log buffer lock is poisoned").extend_from_slice(buffer);
    Ok(buffer.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
  type Writer = Self;

  fn make_writer(&'writer self) -> Self::Writer {
    self.clone()
  }
}

struct Traced {
  response: Response,
  /// Every captured byte, for asserting what is *absent*. Parsing first would
  /// hide a secret that leaked into a field this file does not name.
  raw: String,
  events: Vec<Value>,
}

impl Traced {
  fn event(&self, message: &str) -> Result<&Value> {
    self
      .events
      .iter()
      .find(|event| event.get("message").and_then(Value::as_str) == Some(message))
      .with_context(|| format!("no {message:?} event was recorded in: {}", self.raw))
  }

  fn request_id(&self) -> Result<&str> {
    self
      .response
      .headers()
      .get("x-request-id")
      .and_then(|value| value.to_str().ok())
      .context("the response carried no correlation identifier")
  }
}

/// Drives one request through the real router with a JSON subscriber attached
/// to that future alone.
///
/// `flatten_event` puts the event's own fields at the top level and
/// `with_current_span` records the span a nested event was emitted inside,
/// which is what lets a handler's warning be tied back to its request.
async fn trace(state: ApiState, request: Request<Body>) -> Result<Traced> {
  trace_router(aircraft_api::router(state), request).await
}

async fn trace_router(app: Router, request: Request<Body>) -> Result<Traced> {
  let logs = CapturedLogs::default();
  let subscriber = tracing_subscriber::fmt()
    .json()
    .flatten_event(true)
    .with_current_span(true)
    .with_span_list(false)
    .with_writer(logs.clone())
    .finish();

  let response = app.oneshot(request).with_subscriber(tracing::Dispatch::new(subscriber)).await?;

  let raw = logs.contents();
  let events = raw
    .lines()
    .filter(|line| !line.trim().is_empty())
    .map(|line| {
      serde_json::from_str(line).with_context(|| format!("a captured line did not parse: {line}"))
    })
    .collect::<Result<Vec<Value>>>()?;

  Ok(Traced { response, raw, events })
}

const UNKNOWN_ERROR_PATH: &str = "/__test/internal";
const UNKNOWN_ERROR_SENTINEL: &str = "SELECT secret_digest FROM aircraft_auth.api_credentials; \
  uq_apc_secret_digest; postgres://curator:hunter2@db.internal:5432/aircraft; \
  /srv/private/aircraft.json";

struct UnknownHandlerError(anyhow::Error);

impl IntoResponse for UnknownHandlerError {
  fn into_response(self) -> Response {
    let _source = self.0;
    tracing::error!(
      class = "unclassified_application_error",
      code = "INTERNAL_ERROR",
      "request failed"
    );
    ApiProblem::internal(UNKNOWN_ERROR_PATH).into_response()
  }
}

async fn fail_with_unknown_error() -> Result<StatusCode, UnknownHandlerError> {
  Err(UnknownHandlerError(anyhow::anyhow!(UNKNOWN_ERROR_SENTINEL)))
}

#[tokio::test]
async fn an_unknown_error_returns_a_generic_correlated_500() -> Result<()> {
  const REQUEST_ID: &str = "internal-error-test";

  let routes = Router::new().route(UNKNOWN_ERROR_PATH, get(fail_with_unknown_error));
  let traced = trace_router(
    router_with_routes(state(Arc::new(AlwaysReady)), routes),
    Request::get(UNKNOWN_ERROR_PATH).header("x-request-id", REQUEST_ID).body(Body::empty())?,
  )
  .await?;

  assert_eq!(traced.response.status(), StatusCode::INTERNAL_SERVER_ERROR);
  assert_eq!(traced.request_id()?, REQUEST_ID);
  assert_eq!(
    traced.response.headers().get(header::CONTENT_TYPE),
    Some(&"application/problem+json".parse()?)
  );

  let diagnostic = traced.event("request failed")?;
  assert_eq!(diagnostic.get("class"), Some(&json!("unclassified_application_error")));
  assert_eq!(diagnostic.get("code"), Some(&json!("INTERNAL_ERROR")));
  assert_eq!(diagnostic.pointer("/span/request_id"), Some(&json!(REQUEST_ID)));
  for sentinel in
    ["SELECT secret_digest", "uq_apc_secret_digest", "hunter2", "db.internal", "/srv/private"]
  {
    assert!(!traced.raw.contains(sentinel), "the internal source reached the trace");
  }

  let body = to_bytes(traced.response.into_body(), 4096).await?;
  let document: Value = serde_json::from_slice(&body)?;
  assert_eq!(
    document,
    json!({
      "type": "/problems/internal-error",
      "title": "Internal Server Error",
      "status": 500,
      "detail": "The service encountered an unexpected error.",
      "instance": UNKNOWN_ERROR_PATH,
    })
  );
  for sentinel in
    ["SELECT secret_digest", "uq_apc_secret_digest", "hunter2", "db.internal", "/srv/private"]
  {
    assert!(!body.windows(sentinel.len()).any(|window| window == sentinel.as_bytes()));
  }
  Ok(())
}

fn state(readiness: Arc<dyn ReadinessProbe>) -> ApiState {
  ApiState {
    readiness,
    version: "9.9.9-test",
    build_commit: None,
    shutdown: ShutdownState::new(),
    limits: PerimeterLimits::new(1_048_576, Duration::from_secs(30), 256, &[])
      .expect("an empty origin list cannot fail"),
  }
}

/// Asserts every field the completion event promises, for one request.
///
/// Shared by both trace tests rather than written out twice. A field that is
/// only wrong on one status code is exactly the drift that a second,
/// hand-maintained copy of these assertions stops catching.
///
/// The identifier is compared against the response header rather than merely
/// asserted present, because a layer that recorded one identifier and returned
/// another would satisfy every other assertion here and correlate nothing.
fn assert_completed(traced: &Traced, method: &str, route: &str, status: u16) -> Result<()> {
  let completed = traced.event("request completed")?;

  assert_eq!(completed.get("method"), Some(&Value::from(method)));
  assert_eq!(completed.get("route"), Some(&Value::from(route)));
  assert_eq!(completed.get("status"), Some(&Value::from(status)));
  assert_eq!(completed.get("request_id").and_then(Value::as_str), Some(traced.request_id()?));
  assert!(completed.get("latency_ms").is_some(), "no latency was recorded: {completed}");
  Ok(())
}

/// The success half of the required trace coverage.
#[tokio::test]
async fn a_successful_request_is_traced_with_its_method_route_status_and_latency() -> Result<()> {
  let traced =
    trace(state(Arc::new(AlwaysReady)), Request::get("/health").body(Body::empty())?).await?;

  assert_completed(&traced, "GET", "/health", 200)
}

/// The failure half, and the only assertion that proves the span actually wraps
/// the handler rather than merely preceding it in source order.
///
/// `/ready`'s own warning is emitted deep inside the handler and names no
/// request. If it can be tied back to this request, the span was current while
/// the handler ran.
#[tokio::test]
async fn a_failing_request_is_traced_and_correlated_with_its_request_id() -> Result<()> {
  let traced = trace(
    state(Arc::new(FailsWith("connection refused"))),
    Request::get("/ready").body(Body::empty())?,
  )
  .await?;

  assert_eq!(traced.response.status(), 503);
  assert_completed(&traced, "GET", "/ready", 503)?;

  let probe_failure = traced.event("readiness probe failed")?;
  assert_eq!(
    probe_failure.pointer("/span/request_id").and_then(Value::as_str),
    Some(traced.request_id()?),
    "the handler's own event was not attributable to its request: {}",
    traced.raw
  );
  Ok(())
}

/// Acceptance criterion 4, on both sides of the route branch.
///
/// One sentinel is sent three ways -- as an `Authorization` header, in a query
/// string, and in the body -- because each would leak through a different
/// mistake: recording headers, recording the raw URI instead of the matched
/// route, and recording the body.
///
/// The unmatched case is the one the `<unmatched>` fallback exists for. A
/// request that matches no route carries no `MatchedPath`, and the raw URI is
/// the obvious wrong thing to reach for; without this case that whole branch
/// goes unredacted and untested. `<unmatched>` is spelled out rather than
/// imported because `aircraft_api` keeps the constant private, and because an
/// expectation table is read against the contract, not against the code.
///
/// Reading the completion event before asserting absence is the anti-vacuity
/// guard, and it applies per case: a layer that recorded nothing would contain
/// no sentinel either, and every absence assertion would pass for the wrong
/// reason.
#[tokio::test]
async fn no_authorization_header_query_string_or_body_reaches_the_trace() -> Result<()> {
  const SENTINEL: &str = "postgres://curator:hunter2@db.internal:5432/aircraft";
  const CASES: [(&str, &str); 2] = [("/health", "/health"), ("/no-such-route", "<unmatched>")];

  for (path, expected_route) in CASES {
    let traced = trace(
      state(Arc::new(AlwaysReady)),
      Request::get(format!("{path}?credential={SENTINEL}"))
        .header("authorization", format!("Bearer {SENTINEL}"))
        .body(Body::from(SENTINEL))?,
    )
    .await?;

    let completed = traced.event("request completed")?;
    assert_eq!(completed.get("route"), Some(&Value::from(expected_route)), "path {path}");
    assert!(
      !traced.raw.contains(SENTINEL),
      "path {path}: a secret reached the trace: {}",
      traced.raw
    );
    assert!(
      !traced.raw.contains("hunter2"),
      "path {path}: a credential reached the trace: {}",
      traced.raw
    );
    assert!(
      !traced.raw.contains("credential="),
      "path {path}: the query string reached the trace: {}",
      traced.raw
    );
  }
  Ok(())
}

/// The published vector from `aircraft_app::credential_issuance`'s tests, and
/// the digest that test computed outside the service.
const TOKEN: &str = "ak1_00010203-0405-4607-8809-0a0b0c0d0e0f_\
                     101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f";
const SECRET: &str = "101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f";
const TOKEN_SHA256: &str = "9fdc9e0584edf8647a3677f4e45dd77c303caf819eace32829d3a1ae5e21b4b5";
const PROTECTED: &str = "/__test/protected";
/// What a lookup adapter might echo: the stored digest inside a driver message.
const LOOKUP_FAILURE: &str =
  "row 9fdc9e0584edf8647a3677f4e45dd77c303caf819eace32829d3a1ae5e21b4b5 could not be read";

enum Lookup {
  Live,
  Missing,
  Failing,
}

#[async_trait]
impl CredentialLookup for Lookup {
  async fn resolve(
    &self,
    _key_id: Uuid,
  ) -> Result<Option<CredentialLookupRecord>, PersistenceError> {
    match self {
      Self::Live => Ok(Some(CredentialLookupRecord {
        verifier: CredentialVerifier::of_token(TOKEN),
        revoked: false,
        disabled: false,
        principal_id: 7,
        scopes: vec![Scope::CatalogRead],
        tier: "TEST_TIER".to_owned(),
      })),
      Self::Missing => Ok(None),
      Self::Failing => Err(PersistenceError::Database {
        code: "DATABASE_57P01".to_owned(),
        message: LOOKUP_FAILURE.to_owned(),
      }),
    }
  }
}

fn protected_router(lookup: Lookup) -> Router {
  let routes = Router::new()
    .route(
      PROTECTED,
      get(|Extension(principal): Extension<AuthenticatedPrincipal>| async move {
        principal.principal_id().to_string()
      }),
    )
    .route_layer(axum::middleware::from_fn_with_state(
      Arc::new(AuthenticationService::new(Arc::new(lookup))),
      require_authentication,
    ));
  router_with_routes(state(Arc::new(AlwaysReady)), routes)
}

/// Acceptance criterion 4 for the authentication layer, on every outcome.
///
/// The token is sent on every request and reaches the service on all three,
/// so it is in scope for every event a layer or the middleware emits. The
/// failing lookup also plants the stored digest in the message an adapter
/// could echo, which is what proves the `warn` carries the code and not the
/// message. Reading the completion event first, with the status the case
/// expects, is the anti-vacuity guard; the `warn` event is read the same way.
#[tokio::test]
async fn authentication_traces_carry_no_credential_material_on_any_outcome() -> Result<()> {
  const CASES: [(&str, Lookup, u16); 3] = [
    ("accepted", Lookup::Live, 200),
    ("rejected", Lookup::Missing, 401),
    ("unavailable", Lookup::Failing, 503),
  ];

  for (case, lookup, status) in CASES {
    let traced = trace_router(
      protected_router(lookup),
      Request::get(PROTECTED)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())?,
    )
    .await?;

    assert_eq!(traced.response.status(), status, "{case}");
    assert_completed(&traced, "GET", PROTECTED, status)?;
    if status == 503 {
      let failure = traced.event("credential lookup failed")?;
      assert_eq!(failure.get("code"), Some(&json!("DATABASE_57P01")), "{case}");
      assert_eq!(
        failure.pointer("/span/request_id").and_then(Value::as_str),
        Some(traced.request_id()?),
        "{case}: the lookup failure was not attributable to its request"
      );
    }
    // Fixed sentences: rendering the trace on failure would print what the
    // assertion exists to keep out of output.
    assert!(!traced.raw.contains(TOKEN), "{case}: the clear token reached the trace");
    assert!(!traced.raw.contains(SECRET), "{case}: the secret reached the trace");
    assert!(!traced.raw.contains(TOKEN_SHA256), "{case}: the verifier reached the trace");
    assert!(
      !traced.raw.contains("could not be read"),
      "{case}: the lookup message reached the trace"
    );
  }
  Ok(())
}
