//! HTTP shutdown state shared by the composition root and router.
//!
//! Dependency readiness remains a separate application concern. This state
//! records the process lifecycle and counts work cancelled at the deadline.
//! Its phase is monotonic and latched so late subscribers cannot miss a
//! transition.

use std::sync::{
  Arc,
  atomic::{AtomicUsize, Ordering},
};

use axum::{
  extract::{Request, State},
  http::StatusCode,
  middleware::Next,
  response::{IntoResponse as _, Response},
};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
enum Phase {
  #[default]
  Serving,
  Draining,
  Cancelled,
}

#[derive(Debug, Default)]
struct Inner {
  in_flight: AtomicUsize,
  phase: watch::Sender<Phase>,
}

/// The shared HTTP shutdown phase and in-flight request count.
#[derive(Clone, Debug, Default)]
pub struct ShutdownState {
  inner: Arc<Inner>,
}

impl ShutdownState {
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Marks the process as draining without moving a cancelled process backward.
  pub fn begin_draining(&self) {
    self.advance_to(Phase::Draining);
  }

  /// Cancels requests that are still running after the drain window.
  pub fn cancel(&self) {
    self.advance_to(Phase::Cancelled);
  }

  fn advance_to(&self, phase: Phase) {
    self.inner.phase.send_if_modified(|current| {
      let advanced = phase > *current;
      if advanced {
        *current = phase;
      }
      advanced
    });
  }

  #[must_use]
  pub fn is_draining(&self) -> bool {
    *self.inner.phase.borrow() >= Phase::Draining
  }

  /// How many requests are inside a handler right now.
  #[must_use]
  pub fn in_flight(&self) -> usize {
    self.inner.in_flight.load(Ordering::Acquire)
  }

  /// Resolves once draining has begun, including for a late subscriber.
  pub async fn draining(&self) {
    self.reaches(Phase::Draining).await;
  }

  async fn cancelled(&self) {
    self.reaches(Phase::Cancelled).await;
  }

  async fn reaches(&self, phase: Phase) {
    let mut phases = self.inner.phase.subscribe();
    let _ = phases.wait_for(|current| *current >= phase).await;
  }

  fn enter(&self) -> InFlightGuard {
    self.inner.in_flight.fetch_add(1, Ordering::AcqRel);
    InFlightGuard(Arc::clone(&self.inner))
  }
}

/// Releases the count even when cancellation drops the handler future.
struct InFlightGuard(Arc<Inner>);

impl Drop for InFlightGuard {
  fn drop(&mut self) {
    self.0.in_flight.fetch_sub(1, Ordering::AcqRel);
  }
}

/// Counts a request for as long as its handler is running and abandons it if
/// the drain window expires first.
///
/// The guard is released when the handler returns its response, not when the
/// body finishes reaching the client. Every route here answers with a complete
/// in-memory body, so the two coincide; a streaming route would need the guard
/// carried into the body.
///
/// Forced cancellation returns only a status. The structured shutdown problem
/// belongs to `/ready`; reusing it here would falsely name `/ready` as the
/// instance of a cancelled request to any other route.
pub async fn track_in_flight(
  State(shutdown): State<ShutdownState>,
  request: Request,
  next: Next,
) -> Response {
  let _guard = shutdown.enter();

  tokio::select! {
    response = next.run(request) => response,
    () = shutdown.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
  }
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, so a panicking timeout is fine.
  #![allow(clippy::expect_used)]

  use std::time::Duration;

  use tokio::time::timeout;

  use super::ShutdownState;

  #[tokio::test]
  async fn draining_is_latched_before_waiting_begins() {
    let shutdown = ShutdownState::new();
    shutdown.begin_draining();

    timeout(Duration::from_secs(1), shutdown.draining())
      .await
      .expect("draining begun before subscription must still be observed");
  }

  #[tokio::test]
  async fn cancellation_is_latched_before_waiting_begins() {
    let shutdown = ShutdownState::new();
    shutdown.cancel();

    timeout(Duration::from_secs(1), shutdown.cancelled())
      .await
      .expect("cancellation issued before subscription must still be observed");
  }

  #[tokio::test]
  async fn a_later_drain_signal_cannot_revoke_a_cancellation() {
    let shutdown = ShutdownState::new();
    shutdown.cancel();
    shutdown.begin_draining();

    assert!(shutdown.is_draining(), "a cancelled process is still a draining one");
    timeout(Duration::from_secs(1), shutdown.cancelled())
      .await
      .expect("a later drain signal must not revoke cancellation");
  }
}
