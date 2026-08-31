//! `aircraft-server` composition root.
//!
//! Configuration, telemetry, and routing are each owned by their own crate, so
//! this binary only resolves them in order and hands the bound socket to
//! [`aircraft_server::serve`]. Graceful shutdown, the database pool, readiness,
//! and perimeter limits are separate stories and are deliberately absent.

use aircraft_config::Settings;
use anyhow::{Context, Result};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
  aircraft_observability::logging::init();

  let settings = Settings::load().context("loading HTTP settings")?;
  let address = settings.bind_address();
  let listener = TcpListener::bind(&address).await.with_context(|| format!("binding {address}"))?;

  // The bound address is read back from the listener rather than reused from
  // configuration, so the log names what is actually being served: a host that
  // resolves through DNS reaches the socket under an address the settings never
  // spelled out.
  let bound = listener.local_addr().context("reading the bound address")?;
  tracing::info!(address = %bound, "aircraft-server listening");

  aircraft_server::serve(listener).await
}
