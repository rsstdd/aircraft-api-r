//! HTTP runtime composition.
//!
//! This crate wires adapters together and owns no business rules; routes and
//! their contracts belong to `aircraft_api`, and persistence to `aircraft_db`.

use anyhow::{Context, Result};
use tokio::net::TcpListener;

/// Serves the API router on an already-bound listener until the future is
/// dropped or the server fails.
///
/// The listener is taken already bound rather than built from an address here,
/// so a caller that needs the assigned port -- binding port 0 in a test, or a
/// socket passed in by a supervisor -- can read it before the first request.
/// Binding is therefore the composition root's job, not this function's.
///
/// # Errors
///
/// Returns an error if the server stops with an I/O failure.
pub async fn serve(listener: TcpListener) -> Result<()> {
  axum::serve(listener, aircraft_api::router()).await.context("the HTTP server stopped")
}
