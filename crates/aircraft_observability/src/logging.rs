//! Subscriber installation.
//!
//! This crate owns how telemetry is emitted, never what is emitted. Per-request
//! spans and events belong to the HTTP boundary in `aircraft_api`.

use tracing_subscriber::EnvFilter;

/// The environment variable that overrides [`DEFAULT_FILTER`] entirely.
const FILTER_VARIABLE: &str = "RUST_LOG";

/// This workspace's own targets at `INFO`, over an `ERROR` floor for everything
/// else.
///
/// The floor is a security boundary, not tidiness. A dependency logs whatever it
/// likes at whatever level it likes, and at least one logs a credential: `sqlx`
/// ends its connect-parameter match with
/// `warn!(%key, %value, "ignoring unrecognized connect parameter")`, and its
/// recognized set excludes `sslpassword`. A global `INFO` default therefore
/// publishes the client-key passphrase out of the database URL, which is
/// exactly what this service must never log.
///
/// An allow-list rather than a `sqlx=off` deny-list, because a deny-list only
/// covers the dependency someone already caught leaking.
///
/// `ERROR` rather than nothing at all, so a `hyper` or `tokio` failure still
/// reaches an operator. That is also the level `EnvFilter` itself defaults to,
/// so third-party output is no louder than it was before this crate set a
/// default. The known leak is at `WARN`, below this floor.
///
/// `apps/server/tests/health.rs` proves the leak stays closed under both this
/// value and the one `.env.example` documents.
const DEFAULT_FILTER: &str = "error,\
   aircraft_server=info,\
   aircraft_api=info,\
   aircraft_app=info,\
   aircraft_db=info,\
   aircraft_ingest=info,\
   aircraft_observability=info";

/// Installs the process-wide `tracing` subscriber on standard error.
///
/// When `RUST_LOG` is unset or blank, this workspace's own targets are enabled
/// at `INFO` over an `ERROR` floor for every other crate. `RUST_LOG` otherwise
/// replaces that outright rather than adding to it, which is `EnvFilter`'s own
/// semantics and the reason the default cannot be expressed as a fallback
/// level.
///
/// The floor is a disclosure boundary: a dependency chooses its own levels, and
/// at least one logs a credential below `ERROR`. Widening `RUST_LOG` to a
/// third-party target accepts that. See the `DEFAULT_FILTER` constant in this
/// module for which crate and why.
///
/// # Panics
///
/// Panics if a global subscriber is already installed. This is a composition
/// root's first call; a test that needs to read events attaches its own
/// dispatch to the future under test rather than calling this.
pub fn init() {
  tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(filter()).init();
}

/// Resolves `RUST_LOG` against [`DEFAULT_FILTER`].
///
/// A blank value is treated as absent rather than as "log nothing": an empty
/// environment variable is how a shell spells "unset" by accident, and reading
/// it as a deliberate silencing would disable logging in the case hardest to
/// diagnose.
///
/// Both branches parse through `parse_lossy`, so an unparsable directive is
/// dropped with a warning instead of aborting startup, whichever source it came
/// from. `EnvFilter::builder().with_default_directive(..)` is deliberately not
/// used: it accepts a single directive, and this default is a list.
fn filter() -> EnvFilter {
  let configured = std::env::var(FILTER_VARIABLE).unwrap_or_default();
  let directives = if configured.trim().is_empty() { DEFAULT_FILTER } else { configured.as_str() };

  EnvFilter::builder().parse_lossy(directives)
}
