//! `aircraft-server` composition root.
//!
//! Configuration, telemetry, persistence, and routing are each owned by their
//! own crate, so this binary only resolves them in order and hands the bound
//! socket to [`aircraft_server::serve`]. Graceful shutdown and perimeter limits
//! are separate stories and are deliberately absent.

use std::sync::Arc;

use aircraft_api::ApiState;
use aircraft_config::{DatabaseSettings, Settings};
use aircraft_db::readiness::PoolReadiness;
use anyhow::{Context, Result};
use secrecy::ExposeSecret as _;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
  aircraft_observability::logging::init();

  // Every setting is resolved before anything is connected or bound, so a
  // misconfigured deployment fails naming the setting that is wrong rather than
  // surfacing later as a connection or bind error somewhere downstream.
  let settings = Settings::load().context("loading HTTP settings")?;
  let database = DatabaseSettings::load().context("loading database settings")?;

  // The pool is built before the listener. A server that cannot reach its
  // database has nothing to serve, and binding first would advertise an endpoint
  // that could never answer. `connect` returns only after a live connection has
  // executed a statement, so reaching the next line means the database answered.
  //
  // The pool is held for the lifetime of the process by the readiness probe in
  // the router state. The bounds are read back from the pool rather than from
  // the settings, so the log reports what was built and not what was asked for.
  let pool = aircraft_db::pool::connect(
    database.url.expose_secret(),
    database.max_connections,
    database.acquire_timeout_seconds,
    database.statement_timeout_seconds,
  )
  .await
  .context("connecting the database pool")?;
  tracing::info!(max_connections = pool.options().get_max_connections(), "database pool ready");

  // The build identity is resolved here because this is the crate that is
  // actually built and deployed; `aircraft_api` would otherwise report its own
  // package version, which is a different thing that happens to match today.
  // BUILD_COMMIT is optional: a source checkout builds without it, and the
  // route reports its absence rather than inventing a value.
  let state = ApiState {
    readiness: Arc::new(PoolReadiness::new(pool)),
    version: env!("CARGO_PKG_VERSION"),
    build_commit: option_env!("BUILD_COMMIT"),
  };

  let address = settings.bind_address();
  let listener = TcpListener::bind(&address).await.with_context(|| format!("binding {address}"))?;

  // The bound address is read back from the listener rather than reused from
  // configuration, so the log names what is actually being served: a host that
  // resolves through DNS reaches the socket under an address the settings never
  // spelled out.
  let bound = listener.local_addr().context("reading the bound address")?;
  tracing::info!(address = %bound, "aircraft-server listening");

  aircraft_server::serve(listener, state).await
}
