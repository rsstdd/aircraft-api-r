//! `aircraft-server` composition root.
//!
//! Configuration, telemetry, persistence, and routing are each owned by their
//! own crate, so this binary only resolves them in order and hands the bound
//! socket to [`aircraft_server::serve`] together with the signal that ends it.
//! Perimeter limits are a separate story and are deliberately absent.

use std::{sync::Arc, time::Duration};

use aircraft_api::{ApiState, shutdown::ShutdownState};
use aircraft_config::{DatabaseSettings, Settings};
use aircraft_db::readiness::PoolReadiness;
use anyhow::{Context, Result};
use secrecy::ExposeSecret as _;
use tokio::{
  net::TcpListener,
  signal::unix::{SignalKind, signal},
};

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
    shutdown: ShutdownState::new(),
  };

  let address = settings.bind_address();
  let listener = TcpListener::bind(&address).await.with_context(|| format!("binding {address}"))?;

  // The bound address is read back from the listener rather than reused from
  // configuration, so the log names what is actually being served: a host that
  // resolves through DNS reaches the socket under an address the settings never
  // spelled out.
  let bound = listener.local_addr().context("reading the bound address")?;
  tracing::info!(address = %bound, "aircraft-server listening");

  // Both handlers are installed before the first request can arrive, and not
  // earlier: a signal during startup finds the default disposition, which ends
  // a process that has nothing in flight to lose. Installing them sooner would
  // instead queue the signal behind a slow pool connection and look like a
  // server ignoring SIGTERM.
  //
  // SIGTERM is what an orchestrator sends; SIGINT is what a terminal sends.
  // There is no `cfg(unix)` fallback because this binary is a Linux service,
  // and a build that silently ignored SIGTERM would be worse than one that does
  // not compile.
  let mut terminate = signal(SignalKind::terminate()).context("installing the SIGTERM handler")?;
  let mut interrupt = signal(SignalKind::interrupt()).context("installing the SIGINT handler")?;
  let shutdown = async move {
    let received = tokio::select! {
      _ = terminate.recv() => "SIGTERM",
      _ = interrupt.recv() => "SIGINT",
    };
    tracing::info!(signal = received, "shutdown requested");
  };

  aircraft_server::serve(
    listener,
    state,
    shutdown,
    Duration::from_secs(settings.http.shutdown_grace_seconds),
  )
  .await
}
