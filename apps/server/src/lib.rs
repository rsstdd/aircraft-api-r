//! HTTP runtime composition.
//!
//! This crate wires adapters together and owns no business rules; routes and
//! their contracts belong to `aircraft_api`, and persistence to `aircraft_db`.
//! What it does own is the shutdown lifecycle: when acceptance stops, how long
//! work already in flight has to finish, and what happens to work that does not.

use std::{
  future::{Future, IntoFuture as _},
  time::Duration,
};

use aircraft_api::ApiState;
use anyhow::{Context, Result};
use tokio::{net::TcpListener, time::timeout};

/// How long cancelled work is given to reach its client before the process
/// stops waiting for it.
///
/// Cancellation ends every tracked handler at once, but the connection tasks
/// that write those responses belong to `axum`, and this function has no
/// narrower way to bound them. One second is far longer than writing a small
/// in-memory response takes, and short enough that a client which has stopped
/// reading cannot extend a shutdown past its configured window by more than
/// that. An unbounded await here would hand that client the power to defeat the
/// window entirely, which is the one thing this function exists to prevent.
const CANCELLED_FLUSH: Duration = Duration::from_secs(1);

/// Serves the API router on an already-bound listener until `signal` resolves,
/// then drains for at most `grace` before cancelling what is left.
///
/// State is taken rather than built here for the same reason the listener is:
/// this crate composes, and the readiness port and build identity are both
/// decisions the composition root owns.
///
/// The listener is taken already bound rather than built from an address here,
/// so a caller that needs the assigned port -- binding port 0 in a test, or a
/// socket passed in by a supervisor -- can read it before the first request.
/// Binding is therefore the composition root's job, not this function's.
///
/// `signal` is a parameter for the same reason. A `serve` that reached for
/// `tokio::signal` itself could only be exercised by signalling the test
/// process; taking the future lets the drain window be driven directly.
///
/// An expired drain window is a completed shutdown, not a failure: the process
/// was asked to stop, and it stopped. The requests it cut short are reported as
/// a warning carrying their count, which is the only place that number exists.
///
/// # Errors
///
/// Returns an error if the server stops with an I/O failure. An expired drain
/// window is not one of those.
pub async fn serve(
  listener: TcpListener,
  state: ApiState,
  signal: impl Future<Output = ()> + Send + 'static,
  grace: Duration,
) -> Result<()> {
  let shutdown = state.shutdown.clone();

  let server = axum::serve(listener, aircraft_api::router(state))
    .with_graceful_shutdown({
      let shutdown = shutdown.clone();
      async move {
        signal.await;
        // Readiness fails before `axum` is told to stop accepting, so a load
        // balancer polling during the gap is refused by a process that can
        // still answer rather than by a closed socket. The same call is what
        // releases the await below, so the refusal and the deadline that bounds
        // it cannot come apart: they are one transition.
        shutdown.begin_draining();
      }
    })
    .into_future();
  tokio::pin!(server);

  tokio::select! {
    result = &mut server => return result.context("the HTTP server stopped"),
    () = shutdown.draining() => {}
  }
  tracing::info!(in_flight = shutdown.in_flight(), ?grace, "draining in-flight requests");

  // `axum` waits for in-flight requests indefinitely and offers no deadline of
  // its own, so the bound is this race. It starts at the signal rather than at
  // startup, which is why the drain is a second await instead of a `timeout`
  // wrapped around the whole server.
  let Ok(result) = timeout(grace, server.as_mut()).await else {
    // Counted before cancelling, because cancelling is what empties the
    // counter. `timeout` only borrowed the server future, so nothing has been
    // abandoned yet and this is still the work about to be cut short.
    tracing::warn!(
      cancelled = shutdown.in_flight(),
      ?grace,
      "shutdown grace expired before requests drained"
    );
    shutdown.cancel();

    if timeout(CANCELLED_FLUSH, server).await.is_err() {
      tracing::warn!(flush = ?CANCELLED_FLUSH, "cancelled responses did not reach their clients");
    }
    return Ok(());
  };

  tracing::info!("in-flight requests drained");
  result.context("the HTTP server stopped")
}
