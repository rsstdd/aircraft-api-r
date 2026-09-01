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

use aircraft_api::{ApiState, PerimeterLimits, shutdown::ShutdownState};
use aircraft_app::{ingestion::PersistenceError, readiness::ReadinessProbe};
use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::{body::Body, http::Request, response::Response};
use serde_json::Value;
use tower::ServiceExt as _;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::fmt::MakeWriter;

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
  let logs = CapturedLogs::default();
  let subscriber = tracing_subscriber::fmt()
    .json()
    .flatten_event(true)
    .with_current_span(true)
    .with_span_list(false)
    .with_writer(logs.clone())
    .finish();

  let response = aircraft_api::router(state)
    .oneshot(request)
    .with_subscriber(tracing::Dispatch::new(subscriber))
    .await?;

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
