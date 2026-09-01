//! Subscriber installation.
//!
//! This crate owns how telemetry is emitted, never what is emitted. Per-request
//! spans and events belong to the HTTP boundary in `aircraft_api`.

use tracing_subscriber::{EnvFilter, filter::LevelFilter};

/// Installs the process-wide `tracing` subscriber on standard error.
///
/// The default is `INFO` rather than `EnvFilter`'s own `ERROR`, because the
/// events this service exists to produce -- the bound address, the pool bounds,
/// the shutdown transitions, and one line per request -- are all `INFO`. Left at
/// `ERROR` a healthy deployment logs nothing at all, and an operator has to know
/// to set `RUST_LOG` before the service becomes observable. `RUST_LOG` still
/// overrides this, per target and per level, exactly as before.
///
/// # Panics
///
/// Panics if a global subscriber is already installed. This is a composition
/// root's first call; a test that needs to read events attaches its own
/// dispatch to the future under test rather than calling this.
pub fn init() {
  tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter(
      EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy(),
    )
    .init();
}
