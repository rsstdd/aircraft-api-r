//! The readiness adapter behind `aircraft_api`'s `/ready` route.
//!
//! Implements [`ReadinessProbe`] over the pool built by
//! [`connect`](crate::pool::connect). The port is declared in `aircraft_app`
//! because `cargo run -p xtask -- boundaries` refuses `aircraft_db` and `SQLx`
//! inside `aircraft_api`.

use std::time::Duration;

use aircraft_app::{ingestion::PersistenceError, readiness::ReadinessProbe};
use async_trait::async_trait;
use sqlx_core::query::query;
use sqlx_postgres::PgPool;
use tokio::time::timeout;

use crate::repositories::ingestion_repository::database_error;

/// How long a readiness check may run before the database is reported
/// unavailable.
///
/// Fixed rather than configurable, and deliberately far below the pool's own
/// `acquire_timeout`, which is measured in seconds and governs request traffic.
/// Readiness gives up sooner because it answers a different question: a probe
/// that takes longer than the interval of whatever polls it never sheds traffic,
/// it only makes the poller time out instead. A setting would be a way to raise
/// it past that point by accident, so the bound is part of the route's contract.
const READINESS_DEADLINE: Duration = Duration::from_millis(250);

/// Reports readiness by round-tripping a statement through the application pool.
#[derive(Debug)]
pub struct PoolReadiness {
  pool: PgPool,
}

impl PoolReadiness {
  /// Wraps the pool the server already holds. Readiness deliberately shares it:
  /// a probe with a private connection reports on a path no request takes, and
  /// would answer ready while every real caller queued behind an exhausted pool.
  #[must_use]
  pub const fn new(pool: PgPool) -> Self {
    Self { pool }
  }
}

#[async_trait]
impl ReadinessProbe for PoolReadiness {
  /// # Errors
  ///
  /// Returns [`PersistenceError::Database`] carrying `DATABASE_UNAVAILABLE`
  /// when the deadline expires -- whether waiting for a connection or for the
  /// server to answer -- and the sanitized `SQLx` failure otherwise.
  async fn check(&self) -> Result<(), PersistenceError> {
    // The deadline spans acquisition and the round trip together. Acquiring a
    // connection proves only that the pool had one free; a server that has
    // stopped answering hands back a connection that still fails the statement.
    match timeout(READINESS_DEADLINE, query("SELECT 1").execute(&self.pool)).await {
      Ok(Ok(_answered)) => Ok(()),
      Ok(Err(error)) => Err(database_error(error)),
      Err(_elapsed) => Err(PersistenceError::Database {
        code: "DATABASE_UNAVAILABLE".to_owned(),
        message: "the database did not answer within the readiness deadline".to_owned(),
      }),
    }
  }
}
