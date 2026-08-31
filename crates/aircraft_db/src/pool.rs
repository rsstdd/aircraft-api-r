//! The bounded `PostgreSQL` pool the HTTP runtime serves from.
//!
//! Separate from [`SqlxIngestionStore::connect`](crate::SqlxIngestionStore),
//! whose bounds exist for a writer that takes advisory locks. This pool serves
//! reads, so it sets no lock timeout and empties `search_path` instead: the
//! server's SQL is required to be schema-qualified, and an empty path is what
//! turns that requirement into something `PostgreSQL` enforces rather than
//! something review has to notice.

use std::time::Duration;

use aircraft_app::ingestion::PersistenceError;
use sqlx_core::{error::Error as SqlxError, query::query};
use sqlx_postgres::{PgPool, PgPoolOptions};

use crate::repositories::ingestion_repository::database_error;

/// Opens the application pool and proves it can reach `PostgreSQL`.
///
/// The returned pool has already executed a statement on a live connection:
/// `connect` acquires one eagerly, and `after_connect` runs against it. A caller
/// that receives `Ok` therefore holds a pool that has round-tripped, not merely
/// a pool that was configured.
///
/// # Errors
///
/// Returns [`PersistenceError::Database`] if the connection is refused, the
/// credentials are rejected, or the session settings cannot be applied. No
/// variant carries the connection string.
pub async fn connect(
  url: &str,
  max_connections: u32,
  acquire_timeout_seconds: u64,
  statement_timeout_seconds: u64,
) -> Result<PgPool, PersistenceError> {
  let statement_timeout = format!("{}ms", statement_timeout_seconds.saturating_mul(1_000));
  PgPoolOptions::new()
    .max_connections(max_connections)
    .acquire_timeout(Duration::from_secs(acquire_timeout_seconds))
    .after_connect(move |connection, _metadata| {
      let statement_timeout = statement_timeout.clone();
      Box::pin(async move {
        query(
          "SELECT set_config('statement_timeout',$1,FALSE),
                  set_config('search_path','',FALSE)",
        )
        .bind(statement_timeout)
        .execute(connection)
        .await?;
        Ok(())
      })
    })
    .connect(url)
    .await
    .map_err(connection_error)
}

/// Maps a connection failure to the persistence vocabulary without letting the
/// connection string into the diagnostic.
///
/// Every other failure goes through [`database_error`], which sanitizes and
/// bounds the message. `Configuration` cannot: it renders its source, and for a
/// connection failure that source is the connection string, password included.
/// `aircraft_config` already validated the URL's shape, so the fact worth
/// reporting is which setting to look at -- not the value it holds.
fn connection_error(error: SqlxError) -> PersistenceError {
  match error {
    SqlxError::Configuration(_) => PersistenceError::Database {
      code: "DATABASE_CONFIGURATION".to_owned(),
      message: "database.url could not be used as a PostgreSQL connection string".to_owned(),
    },
    other => database_error(other),
  }
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, so panicking accessors are fine.
  #![allow(clippy::expect_used)]

  use super::{PersistenceError, SqlxError, connection_error};

  /// Both halves are load-bearing. Dropping the `Configuration` arm leaves the
  /// credential in the message, which the first assertion catches; replacing the
  /// message with an empty string would satisfy that assertion alone, which the
  /// second one catches.
  #[test]
  fn a_configuration_failure_names_its_setting_and_no_connection_value() {
    const PASSWORD: &str = "n0t-in-any-diagnostic";
    let source = format!("postgres://aircraft_api_app:{PASSWORD}@localhost:5432/aircraft");

    let error = connection_error(SqlxError::Configuration(source.into()));

    let message = error.to_string();
    assert!(!message.contains(PASSWORD), "a configuration failure echoed the credential");
    assert!(
      message.contains("database.url"),
      "the failure must name the setting to look at: {message}"
    );
    assert!(
      matches!(error, PersistenceError::Database { .. }),
      "a connection failure stays a database failure"
    );
  }
}
