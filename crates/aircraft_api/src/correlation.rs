//! Request correlation and the per-request trace event.
//!
//! Every response leaves with an `X-Request-Id`, and every request leaves one
//! structured event behind carrying that identifier, the method, the matched
//! route, the status, and how long the handler took. Those five fields are the
//! whole record on purpose: an operator correlating a client report with a log
//! needs them, and nothing else here is safe to keep. Headers, bodies, query
//! strings, and raw paths are all attacker-controlled and never recorded.
//!
//! Subscriber installation is `aircraft_observability`'s job. This module only
//! emits.

// `unreachable_pub` rejects a bare `pub` here and `clippy::redundant_pub_crate`
// rejects `pub(crate)`; `-D warnings` denies both. `pub(super)` is not a third
// option: this module is a child of the crate root, so it resolves to the same
// visibility and trips the same lint, which reports it as `pub(crate)`.
// Silencing that beats widening the crate's public API for a style lint. Scoped
// to this file, not the workspace.
#![allow(clippy::redundant_pub_crate)]

use std::time::Instant;

use axum::{
  extract::{MatchedPath, Request},
  http::{HeaderMap, HeaderName, HeaderValue},
  middleware::Next,
  response::Response,
};
use tracing::Instrument as _;
use uuid::Uuid;

/// The header carrying the identifier in both directions.
///
/// Spelled lowercase because HTTP/2 requires lowercase field names on the wire.
/// `HeaderMap` compares case-insensitively, so an HTTP/1.1 client sending
/// `X-Request-Id` still matches.
pub(crate) const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The longest identifier this service will adopt from a client.
///
/// A correlation identifier is echoed into every log line for the request, so an
/// unbounded one is an unbounded log record. 128 characters holds a UUID, a
/// hex-encoded 64-byte trace context, and every ULID or Snowflake shape a caller
/// is likely to arrive with.
const MAX_REQUEST_ID_LENGTH: usize = 128;

/// The `route` recorded when no route matched.
///
/// The raw URI is deliberately not used as a fallback. It is attacker-supplied,
/// unbounded, and routinely carries credentials in a query string, so recording
/// it would defeat the redaction this module exists to guarantee. The bracketed
/// form mirrors the `<stdin>` locator `aircraft_ingest` reports for input that
/// has no filename.
const UNMATCHED_ROUTE: &str = "<unmatched>";

/// A correlation identifier that is valid to send back out.
///
/// It holds the `HeaderValue` rather than a `String` because that is the form
/// the response needs: the value that was validated is the value that ships, and
/// no fallible conversion sits between the two.
#[derive(Debug)]
pub(crate) struct RequestId(HeaderValue);

impl RequestId {
  /// Takes the caller's identifier, or mints one.
  ///
  /// Exactly one header value is required. A repeated `X-Request-Id` is
  /// ambiguous about which value the sender meant, and this service will not
  /// pick one for them; ambiguous is treated as invalid and replaced, which is
  /// also what stops a smuggled second value from deciding what a log line says.
  fn from_headers(headers: &HeaderMap) -> Self {
    let mut values = headers.get_all(REQUEST_ID).iter();

    match (values.next(), values.next()) {
      (Some(value), None) => Self::accepted(value),
      _ => None,
    }
    .unwrap_or_else(Self::generate)
  }

  /// Adopts a client-supplied identifier, or refuses it.
  ///
  /// `is_ascii_graphic` is the visible range `0x21..=0x7E`, so space, DEL, and
  /// every control character are refused: an adopted identifier cannot break a
  /// log line or inject an escape sequence into a terminal reading one.
  pub(crate) fn accepted(value: &HeaderValue) -> Option<Self> {
    let bytes = value.as_bytes();
    let usable =
      (1..=MAX_REQUEST_ID_LENGTH).contains(&bytes.len()) && bytes.iter().all(u8::is_ascii_graphic);

    usable.then(|| Self(value.clone()))
  }

  /// Mints an identifier for a request that arrived without a usable one.
  ///
  /// Version 4 rather than a counter: identifiers cross process and replica
  /// boundaries, and a counter would collide the moment a second replica exists.
  fn generate() -> Self {
    // Scoped to the only place it means anything. A hyphenated UUID is 36
    // visible ASCII characters, which `HeaderValue` always accepts, so the
    // fallback below is unreachable; it exists because an unreachable panic on a
    // request path is worse than an uninformative header. It is deliberately not
    // UUID-shaped, so one in a log is a signal rather than a puzzle.
    const UNRENDERABLE: HeaderValue = HeaderValue::from_static("generated-id-unavailable");

    let mut buffer = Uuid::encode_buffer();
    let rendered = Uuid::new_v4().hyphenated().encode_lower(&mut buffer);

    Self(HeaderValue::from_str(rendered).unwrap_or(UNRENDERABLE))
  }

  fn into_header_value(self) -> HeaderValue {
    self.0
  }
}

impl std::fmt::Display for RequestId {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    // Every byte was checked to be visible ASCII before the value was built, so
    // `to_str` cannot fail here. An empty field is a better answer than a panic
    // on a request path.
    formatter.write_str(self.0.to_str().unwrap_or_default())
  }
}

/// Correlates one request and records its outcome.
///
/// This layer is applied outside [`crate::shutdown::track_in_flight`], so it
/// also stamps and records the bare `503` that a shutdown cancellation produces
/// -- a response no handler ever sees. `Router::layer` covers the fallback as
/// well as the routes, so an unmatched path is stamped and recorded too.
///
/// The latency measures the handler, not the client: it ends when the response
/// is ready, not when its body has finished reaching the socket. Every route
/// here answers with a complete in-memory body, so the two coincide today; a
/// streaming route would need the measurement carried into the body, the same
/// caveat that applies to the in-flight guard.
pub(crate) async fn correlate(request: Request, next: Next) -> Response {
  let id = RequestId::from_headers(request.headers());
  let method = request.method().clone();
  // Cloned rather than borrowed because the request is about to move into the
  // handler, and cloning a `MatchedPath` clones an `Arc<str>`.
  let matched = request.extensions().get::<MatchedPath>().cloned();

  // The span carries the identifier alone. Every event a handler emits -- the
  // readiness probe's failure warning, for one -- is then attributable to its
  // request without any of them restating transport fields they do not own.
  let span = tracing::info_span!("http_request", request_id = %id);

  let started = Instant::now();
  let mut response = next.run(request).instrument(span).await;
  let latency = started.elapsed();
  let route = matched.as_ref().map_or(UNMATCHED_ROUTE, MatchedPath::as_str);

  // One event at one level, whatever the status. Severity belongs to the
  // subsystem that failed and is reported there; a second arm here would
  // duplicate every field to say what `status` already says.
  tracing::info!(
    request_id = %id,
    method = %method,
    route,
    status = response.status().as_u16(),
    latency_ms = latency.as_millis(),
    "request completed"
  );

  response.headers_mut().insert(REQUEST_ID, id.into_header_value());
  response
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, so a panicking accessor is fine.
  #![allow(clippy::expect_used)]

  use axum::http::HeaderValue;
  use uuid::Uuid;

  use super::RequestId;

  /// The adoption rule, at and around every boundary it has.
  ///
  /// The lengths are literals rather than `MAX_REQUEST_ID_LENGTH` arithmetic.
  /// The ceiling this pins is the one published to clients -- every route's
  /// `X-Request-Id` parameter description says "1-128 visible ASCII characters"
  /// -- so moving the constant has to break this test instead of moving it
  /// along. The space and tab cases separate "visible ASCII" from "printable"
  /// and from "whatever `HeaderValue` happens to permit": both are legal header
  /// bytes that this service refuses.
  #[test]
  fn only_short_visible_ascii_identifiers_are_adopted() {
    let longest = "a".repeat(128);
    let too_long = "a".repeat(129);
    let cases: &[(&[u8], bool)] = &[
      (b"9f8e", true),
      (b"a", true),
      (longest.as_bytes(), true),
      (too_long.as_bytes(), false),
      (b"", false),
      (b"has space", false),
      (b"has\ttab", false),
      ("caf\u{e9}".as_bytes(), false),
    ];

    for (raw, expected) in cases {
      let value = HeaderValue::from_bytes(raw).expect("the case must be a legal header value");

      assert_eq!(
        RequestId::accepted(&value).is_some(),
        *expected,
        "input {:?}",
        String::from_utf8_lossy(raw)
      );
    }
  }

  /// Both assertions are load-bearing. Without the first, a client handed an
  /// identifier this service would itself refuse could not send it back, and
  /// correlation would break on every hop. The second is what catches
  /// `generate`'s unreachable fallback becoming reachable: that value is 24
  /// visible ASCII characters, so it round-trips happily while correlating
  /// every request in the process to the same string.
  #[test]
  fn a_generated_identifier_is_a_uuid_this_service_would_itself_adopt() {
    let generated = RequestId::generate();
    let rendered = generated.to_string();
    let returned = generated.into_header_value();

    assert!(
      RequestId::accepted(&returned).is_some(),
      "a generated identifier must round-trip: {rendered}"
    );
    assert!(
      Uuid::try_parse(&rendered).is_ok(),
      "generate must mint a UUID rather than fall back: {rendered}"
    );
  }
}
