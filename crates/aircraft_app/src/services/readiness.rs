//! Readiness port.
//!
//! `/ready` reports whether the process can serve database-backed traffic, but
//! `aircraft_api` may depend on neither `aircraft_db` nor `SQLx`: both are
//! refused by `cargo run -p xtask -- boundaries`. The probe is therefore a port
//! declared here, implemented over a pool in `aircraft_db`, and injected by
//! `apps/server`.

use async_trait::async_trait;

use super::ingestion::PersistenceError;

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
  /// Reports whether the database is reachable and answering statements now.
  ///
  /// The implementation owns its own deadline rather than accepting one. A
  /// probe that outlives the interval of the caller polling it cannot shed
  /// traffic in time to matter, so the bound belongs with the adapter that
  /// knows what it is waiting on.
  async fn check(&self) -> Result<(), PersistenceError>;
}
