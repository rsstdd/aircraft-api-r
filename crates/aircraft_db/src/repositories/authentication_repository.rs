//! Credential verification lookup: one bound `SELECT` by `key_id` that returns
//! every fact `aircraft_app::authentication` reads.
//!
//! The statement probes `aircraft_auth.api_credentials` by its primary key,
//! joins the principal, and left-joins the grants into one sorted array, so a
//! credential, its revocation, its principal's disablement, tier, and scopes
//! are observed from one statement snapshot and cost one round trip. Revoked
//! and disabled rows are returned with their flags set rather than filtered:
//! the service compares the digest on the same path for every state, and a
//! `WHERE revoked_at IS NULL` here would hand it a cheaper rejection.
//!
//! No transaction: the read is a single statement, and there is nothing to
//! make atomic across two. The bounds are the pool's `acquire_timeout` and the
//! session `statement_timeout` that [`crate::pool::connect`] sets, plus the
//! request deadline of whichever perimeter composes the middleware.

use aircraft_app::{
  authentication::{CredentialLookup, CredentialLookupRecord, Scope},
  credential_issuance::CredentialVerifier,
  ingestion::PersistenceError,
};
use async_trait::async_trait;
use sqlx_core::{error::Error as SqlxError, query::query, row::Row};
use sqlx_postgres::{PgPool, PgRow};
use uuid::Uuid;

use super::credential_repository::redacted_database_error;

/// The verification projection.
///
/// Every column read here is one the restricted runtime role is granted,
/// column by column, in `database/roles/app_grants.sql`, which names this
/// constant in turn; a column added to this statement without a matching
/// grant fails `the_runtime_role_executes_only_the_verification_projection`
/// with `42501`, not silently. `decode(..., 'hex')` hands the digest over as
/// bytes so no text form of it is ever decoded into a `String` this side; the
/// `::text` casts are what let `SQLx` decode the `lookup_code` domain.
/// Grouping by both primary keys is what lets the credential and principal
/// columns be selected beside the aggregate. The aggregate's `ORDER BY` is
/// the determinism guarantee, not the current plan: the grants primary key
/// already yields code order today, so no test can tell the two apart, and
/// the clause is what keeps that true when the planner changes its mind.
const LOOKUP_CREDENTIAL: &str = "SELECT decode(c.secret_digest, 'hex') AS secret_digest,
            c.revoked_at IS NOT NULL AS revoked,
            p.id AS principal_id,
            p.disabled_at IS NOT NULL AS disabled,
            p.rate_limit_tier_code::text AS tier,
            COALESCE(
              array_agg(g.scope_code::text ORDER BY g.scope_code)
                FILTER (WHERE g.scope_code IS NOT NULL),
              ARRAY[]::text[]
            ) AS scopes
       FROM aircraft_auth.api_credentials c
       JOIN aircraft_auth.principals p ON p.id = c.principal_id
       LEFT JOIN aircraft_auth.principal_scope_grants g ON g.principal_id = p.id
      WHERE c.key_id = $1
      GROUP BY c.key_id, p.id";

/// Static on purpose: a stored value outside the schema's contract is reported
/// as a class, never rendered. `chk_apc_secret_digest` and `fk_psg_scope` make
/// both unreachable through the migrations; they are defense against a table
/// altered by hand.
const DIGEST_NOT_32_BYTES: &str = "stored credential digest is not 32 bytes";
const SCOPE_OUTSIDE_VOCABULARY: &str = "granted scope is outside the closed vocabulary";

#[derive(Clone, Debug)]
pub struct SqlxCredentialLookup {
  pool: PgPool,
}

impl SqlxCredentialLookup {
  /// Over the application pool the server already holds, for the reason
  /// [`crate::readiness::PoolReadiness`] shares it: a private connection would
  /// be a path no bound applies to.
  #[must_use]
  pub const fn from_pool(pool: PgPool) -> Self {
    Self { pool }
  }
}

#[async_trait]
impl CredentialLookup for SqlxCredentialLookup {
  async fn resolve(
    &self,
    key_id: Uuid,
  ) -> Result<Option<CredentialLookupRecord>, PersistenceError> {
    query(LOOKUP_CREDENTIAL)
      .bind(key_id)
      .fetch_optional(&self.pool)
      .await
      .map_err(lookup_database_error)?
      .as_ref()
      .map(record_from_row)
      .transpose()
  }
}

fn record_from_row(row: &PgRow) -> Result<CredentialLookupRecord, PersistenceError> {
  let digest: Vec<u8> = row.try_get("secret_digest").map_err(lookup_database_error)?;
  // The `Err` of `try_into` is the bytes themselves; the closure discards it
  // unrendered.
  let digest: [u8; 32] = digest
    .try_into()
    .map_err(|_wrong_length| PersistenceError::Invariant(DIGEST_NOT_32_BYTES.to_owned()))?;
  let codes: Vec<String> = row.try_get("scopes").map_err(lookup_database_error)?;
  let scopes = codes
    .iter()
    .map(|code| Scope::try_from(code.as_str()))
    .collect::<Result<Vec<Scope>, _>>()
    .map_err(|_unknown| PersistenceError::Invariant(SCOPE_OUTSIDE_VOCABULARY.to_owned()))?;

  Ok(CredentialLookupRecord {
    verifier: CredentialVerifier::from_digest(digest),
    revoked: row.try_get("revoked").map_err(lookup_database_error)?,
    disabled: row.try_get("disabled").map_err(lookup_database_error)?,
    principal_id: row.try_get("principal_id").map_err(lookup_database_error)?,
    scopes,
    tier: row.try_get("tier").map_err(lookup_database_error)?,
  })
}

fn lookup_database_error(error: SqlxError) -> PersistenceError {
  redacted_database_error("credential lookup", error)
}
