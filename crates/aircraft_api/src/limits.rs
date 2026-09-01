//! The perimeter: what the boundary refuses before a handler runs.
//!
//! Four bounds, each answering with an RFC 9457 problem document.
//! `docs/architecture/http_v1_decisions.md` requires every API-originated `4xx`
//! or `5xx` to use `application/problem+json` with a stable type per failure
//! class, and names payload-too-large, dependency-unavailable, and timeout among
//! them. The three constructors those map to live in [`crate::problem`].
//!
//! That decision is also why two of the bounds are written here rather than
//! taken from `tower-http`. `RequestBodyLimitLayer` and `TimeoutLayer` both
//! answer with a bodyless response whose shape their caller cannot influence, so
//! adopting them would publish exactly the empty `413` and `504` the decision
//! forbids. The body middleware keeps the header fast path, then boundedly reads
//! an undeclared body before dispatch so enforcement does not depend on whether
//! a handler happens to consume it. The timeout uses `tokio::time::timeout`.
//!
//! The bounds themselves are `aircraft_config`'s. They arrive here as
//! [`PerimeterLimits`] rather than as settings because
//! `cargo run -p xtask -- boundaries` refuses `aircraft_config` in this crate,
//! and because HTTP-facing values and typed configuration are separate
//! representations by design.

use std::{sync::Arc, time::Duration};

use axum::{
  body::Body,
  extract::{Request, State},
  http::{HeaderValue, Method, header},
  middleware::Next,
  response::{IntoResponse as _, Response},
};
use futures::StreamExt as _;
use tokio::sync::Semaphore;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::problem::ProblemDetails;

/// An origin the perimeter cannot safely use in its explicit CORS allow-list.
#[derive(Debug, thiserror::Error)]
#[error("the configured CORS origin {0:?} is not usable in an explicit allow-list")]
pub struct InvalidOrigin(String);

/// What the perimeter refuses, and how much of it.
///
/// Cheap to clone because it lives in [`crate::ApiState`], which axum clones
/// once per request: three `Copy` bounds and one refcount bump.
#[derive(Clone, Debug)]
pub struct PerimeterLimits {
  body_bytes: usize,
  timeout: Duration,
  max_concurrent: usize,
  allowed_origins: Arc<[HeaderValue]>,
}

impl PerimeterLimits {
  /// Builds the perimeter from validated configuration.
  ///
  /// # Errors
  ///
  /// Returns [`InvalidOrigin`] naming the first origin that cannot become a safe
  /// explicit allowed-origin header value. A wildcard is rejected because
  /// `tower_http::cors::AllowOrigin::list` panics when it receives one.
  pub fn new(
    body_bytes: usize,
    timeout: Duration,
    max_concurrent: usize,
    allowed_origins: &[String],
  ) -> Result<Self, InvalidOrigin> {
    let allowed_origins = allowed_origins
      .iter()
      .map(|origin| {
        if origin == "*" {
          return Err(InvalidOrigin(origin.clone()));
        }
        HeaderValue::from_str(origin).map_err(|_| InvalidOrigin(origin.clone()))
      })
      .collect::<Result<Arc<[HeaderValue]>, InvalidOrigin>>()?;

    Ok(Self { body_bytes, timeout, max_concurrent, allowed_origins })
  }

  pub(crate) const fn body_bytes(&self) -> usize {
    self.body_bytes
  }

  pub(crate) const fn timeout(&self) -> Duration {
    self.timeout
  }

  /// The admission semaphore for one router.
  ///
  /// Built here rather than stored so the count and the thing enforcing it
  /// cannot drift apart, and so a `PerimeterLimits` clone carries a bound
  /// rather than a share of somebody else's live permits.
  pub(crate) fn permits(&self) -> Arc<Semaphore> {
    Arc::new(Semaphore::new(self.max_concurrent))
  }

  /// The CORS policy for the routes this crate serves.
  ///
  /// `AllowOrigin::list` **panics** when handed a wildcard, so
  /// [`Self::new`] rejects one before it can enter `allowed_origins`.
  ///
  /// `X-Request-Id` is both allowed and exposed. Correlation is contract in both
  /// directions, and a cross-origin caller can neither send an identifier nor
  /// read the one it was given back unless it appears in both lists.
  pub(crate) fn cors_layer(&self) -> CorsLayer {
    CorsLayer::new()
      .allow_origin(AllowOrigin::list(self.allowed_origins.iter().cloned()))
      .allow_methods([Method::GET, Method::HEAD])
      .allow_headers([header::CONTENT_TYPE, crate::correlation::REQUEST_ID])
      .expose_headers([crate::correlation::REQUEST_ID])
  }

  /// Refuses a request whose body exceeds the limit.
  ///
  /// A declared oversized body is refused before a byte is read. Otherwise the
  /// body is consumed here, at most `max_bytes` are retained, and an accepted
  /// body is rebuilt for the handler. Reading at the boundary is what makes the
  /// limit apply to chunked requests even when the selected handler reads no
  /// body at all.
  ///
  /// An unparsable `content-length` is treated as absent rather than as a
  /// rejection. Hyper refuses a malformed framing header before this point, so a
  /// value that survives to here and still fails to parse as `usize` is one this
  /// bound has no opinion about; answering it with a size error would report a
  /// limit that was never compared against anything.
  pub(crate) async fn refuse_oversized_body(
    State(max_bytes): State<usize>,
    request: Request,
    next: Next,
  ) -> Response {
    let declared = request
      .headers()
      .get(header::CONTENT_LENGTH)
      .and_then(|value| value.to_str().ok())
      .and_then(|value| value.parse::<usize>().ok());

    if declared.is_some_and(|length| length > max_bytes) {
      return ProblemDetails::payload_too_large().into_response();
    }

    let (parts, body) = request.into_parts();
    let mut stream = body.into_data_stream();
    let mut buffered = Vec::new();

    while let Some(chunk) = stream.next().await {
      let Ok(chunk) = chunk else {
        return ProblemDetails::malformed_input().into_response();
      };
      if buffered.len().checked_add(chunk.len()).is_none_or(|length| length > max_bytes) {
        return ProblemDetails::payload_too_large().into_response();
      }
      buffered.extend_from_slice(&chunk);
    }

    next.run(Request::from_parts(parts, Body::from(buffered))).await
  }

  /// Sheds a request rather than queueing it when the service is saturated.
  ///
  /// `try_acquire_owned` is the whole design. `tower::limit::ConcurrencyLimit`
  /// would apply backpressure through `poll_ready` and hold the request until a
  /// permit frees, which is queueing under another name; taking the permit or
  /// refusing immediately is what "shed rather than queue" means.
  ///
  /// The permit is bound to a named local so it lives until this function
  /// returns. Binding it to a bare `_` would drop it at once and turn the bound
  /// into a no-op that every test asserting a successful response still passes.
  ///
  /// It is released when the response is *ready*, not when its body has finished
  /// reaching the socket. Every route here answers with a complete in-memory
  /// body, so the two coincide today; this is the same caveat
  /// [`crate::correlation::correlate`] records for its latency measurement, and
  /// a streaming route would have to carry the permit into the body.
  pub(crate) async fn shed_when_saturated(
    State(permits): State<Arc<Semaphore>>,
    request: Request,
    next: Next,
  ) -> Response {
    let Ok(_permit) = permits.try_acquire_owned() else {
      return ProblemDetails::overloaded().into_response();
    };

    next.run(request).await
  }

  /// Ends a handler that overruns the deadline.
  ///
  /// The inner future includes body reception and the handler. Dropping it
  /// cancels either phase, so a timed-out request stops holding the permit and
  /// in-flight slot it took. The semaphore remains outside this deadline: one
  /// wedged request must release its permit rather than make the service shed
  /// everything behind it for the life of the process.
  pub(crate) async fn enforce_deadline(
    State(deadline): State<Duration>,
    request: Request,
    next: Next,
  ) -> Response {
    tokio::time::timeout(deadline, next.run(request))
      .await
      .unwrap_or_else(|_elapsed| ProblemDetails::deadline_exceeded().into_response())
  }
}
