//! Credential persistence: one bound `INSERT` into `aircraft_auth.api_credentials`
//! inside an explicit transaction.
//!
//! Issuance writes exactly the row migration 025 defines. The statement alone
//! is atomic, but the returned row is decoded afterward and that decoding can
//! fail, so the transaction stays open until a `CredentialRecord` exists and
//! commits only then. A failure anywhere before commit drops the transaction,
//! which rolls it back and leaves no row.
//!
//! Every failure passes through the credential mapper. It keeps the
//! SQLSTATE and bounded message the shared `database_error` produces, names the
//! operation, and applies `aircraft_app`'s digest scrub so the adapter is safe
//! on its own; the service applies the same scrub again to whatever any store
//! returns. The clear token has no field on `NewCredential`, so it cannot reach
//! SQL, and `PostgreSQL`'s primary message names a constraint rather than a
//! value; the scrub guards against a future message or mapper that renders
//! the row.

use aircraft_app::credential_issuance::{
  CredentialRecord, CredentialStore, NewCredential, redact_digest_runs,
};
use aircraft_app::ingestion::PersistenceError;
use async_trait::async_trait;
use sqlx_core::{error::Error as SqlxError, query::query, row::Row};
use sqlx_postgres::{PgPool, PgRow};

use super::ingestion_repository::{database_code, sanitize_database_message};

/// Timestamps come from the column defaults, so the record returned is the row
/// as stored rather than a client-side guess at it.
const INSERT_CREDENTIAL: &str = "INSERT INTO aircraft_auth.api_credentials \
       (key_id, principal_id, secret_digest, label)
     VALUES ($1, $2, $3, $4)
     RETURNING key_id, principal_id, label, created_at, updated_at";

#[derive(Clone, Debug)]
pub struct SqlxCredentialStore {
  pool: PgPool,
}

impl SqlxCredentialStore {
  #[must_use]
  pub const fn from_pool(pool: PgPool) -> Self {
    Self { pool }
  }
}

#[async_trait]
impl CredentialStore for SqlxCredentialStore {
  async fn persist(&self, credential: NewCredential) -> Result<CredentialRecord, PersistenceError> {
    let mut transaction = self.pool.begin().await.map_err(credential_database_error)?;
    let row = query(INSERT_CREDENTIAL)
      .bind(credential.key_id)
      .bind(credential.principal_id)
      .bind(credential.verifier.hex())
      .bind(&credential.label)
      .fetch_one(&mut *transaction)
      .await
      .map_err(credential_database_error)?;
    // Decode before commit: a `?` here drops the transaction, which rolls it back.
    let record = record_from_row(&row)?;
    transaction.commit().await.map_err(credential_database_error)?;
    Ok(record)
  }
}

fn record_from_row(row: &PgRow) -> Result<CredentialRecord, PersistenceError> {
  Ok(CredentialRecord {
    key_id: row.try_get("key_id").map_err(credential_database_error)?,
    principal_id: row.try_get("principal_id").map_err(credential_database_error)?,
    label: row.try_get("label").map_err(credential_database_error)?,
    created_at: row.try_get("created_at").map_err(credential_database_error)?,
    updated_at: row.try_get("updated_at").map_err(credential_database_error)?,
  })
}

fn credential_database_error(error: SqlxError) -> PersistenceError {
  redacted_database_error("credential insert", error)
}

/// The shared mapper's SQLSTATE code and bounded message, prefixed with the
/// operation and with any digest-length hexadecimal run replaced. `detail()`
/// and `constraint()` are never read: a unique or check violation's detail
/// renders the offending row, digest included.
///
/// Redaction runs on the driver's full text and the bound is applied after:
/// a digest cut at the bound would be shorter than the run redaction looks
/// for, and would survive.
///
/// Shared with `authentication_repository`, the other adapter whose rows
/// carry a digest.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn redacted_database_error(operation: &str, error: SqlxError) -> PersistenceError {
  let redacted = redact_digest_runs(&error.to_string());
  PersistenceError::Database {
    code: database_code(&error),
    message: format!("{operation}: {}", sanitize_database_message(&redacted)),
  }
}

#[cfg(test)]
mod tests {
  use sqlx_core::error::Error as SqlxError;

  use super::credential_database_error;

  const DIGEST: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

  /// The scrub has to be wired into the mapper, not merely exist: a driver
  /// message is the one input a test can shape freely.
  #[test]
  fn a_digest_in_a_driver_message_is_redacted_by_the_credential_mapper() {
    let error = credential_database_error(SqlxError::Protocol(format!("row {DIGEST} rejected")));

    assert_eq!(error.code(), "DATABASE_ERROR", "a driver failure keeps the generic code");
    assert_eq!(
      error.to_string(),
      "DATABASE_ERROR: credential insert: encountered unexpected or invalid data: row [REDACTED] \
       rejected"
    );
  }

  /// `sanitize_database_message` cuts at 1000 characters. A digest cut there
  /// is shorter than the redaction threshold, so redaction has to see the
  /// driver's full text first. The padding sweep crosses the bound with the
  /// digest at every offset; the first case, well inside the bound, is the
  /// anti-vacuity guard that the digest was in the text at all.
  #[test]
  fn a_digest_straddling_the_message_bound_is_still_redacted() {
    let message_with = |padding: usize| {
      credential_database_error(SqlxError::Protocol(format!("{}{DIGEST}", "x".repeat(padding))))
        .to_string()
    };

    assert!(message_with(900).contains("[REDACTED]"), "a digest inside the bound is replaced");
    for padding in 900..=1_000 {
      // Any surviving prefix of the digest four characters or longer starts
      // with this; the assertion must not render the message itself.
      assert!(!message_with(padding).contains("dead"), "padding {padding}: a fragment survived");
    }
  }
}
