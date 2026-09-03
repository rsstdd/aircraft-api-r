// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Verification-lookup gates for `SqlxCredentialLookup`, against the canonical
//! install in a disposable `PostgreSQL` container.
//!
//! Everything here is a fact `PostgreSQL` decides: what one statement returns
//! for a key, that a principal with no grants still resolves, that revocation
//! and disablement are read from the timestamps, what the restricted runtime
//! role may and may not read, and that a saturated pool bounds the lookup.
//! Every assertion that touches a token or digest fails with a fixed sentence.

use std::{
  borrow::Cow,
  sync::Arc,
  time::{Duration, Instant},
};

use aircraft_app::{
  authentication::{CredentialLookup, Scope},
  credential_issuance::{CredentialIssuanceService, IssueCredential, IssuedCredential},
  ingestion::PersistenceError,
};
use aircraft_db::{SqlxCredentialLookup, SqlxCredentialStore, pool::connect};
use aircraft_testsupport::{TestResult, install_schema, run_psql, start_postgres};
use secrecy::ExposeSecret as _;
use sha2::{Digest, Sha256};
use sqlx_core::{
  error::{DatabaseError, Error as SqlxError},
  query::query,
  query_scalar::query_scalar,
};
use sqlx_postgres::PgPool;
use uuid::Uuid;

/// The shipped provisioning SQL, embedded so this gate cannot drift from the
/// files an administrator runs; `app_grants.sql` names this file in its header.
const CREATE_APP_ROLE_SQL: &str = include_str!("../../../database/roles/create_app_role.sql");
const APP_GRANTS_SQL: &str = include_str!("../../../database/roles/app_grants.sql");
const RUNTIME_ROLE: &str = "aircraft_api_app";
const RUNTIME_ROLE_PASSWORD: &str = "gate-only-runtime-password";

const TIER_CODE: &str = "TEST_TIER";
const ACQUIRE_TIMEOUT_SECONDS: u64 = 5;
const STATEMENT_TIMEOUT_SECONDS: u64 = 17;

/// Fails with a fixed sentence: rendering either argument would print the
/// very material the assertion exists to keep out of output.
fn assert_absent(haystack: &str, needle: &str, what: &str) {
  assert!(!haystack.contains(needle), "{what} was disclosed");
}

async fn principal(pool: &PgPool, name: &str) -> TestResult<i64> {
  query(
    "INSERT INTO aircraft_auth.rate_limit_tiers (code, label)
         VALUES ($1, 'Test tier') ON CONFLICT (code) DO NOTHING",
  )
  .bind(TIER_CODE)
  .execute(pool)
  .await?;
  let id = query_scalar(
    "INSERT INTO aircraft_auth.principals (name, rate_limit_tier_code)
         VALUES ($1, $2) RETURNING id",
  )
  .bind(name)
  .bind(TIER_CODE)
  .fetch_one(pool)
  .await?;
  Ok(id)
}

async fn grant(pool: &PgPool, principal_id: i64, scope: &str) -> TestResult {
  query(
    "INSERT INTO aircraft_auth.principal_scope_grants (principal_id, scope_code) VALUES ($1, $2)",
  )
  .bind(principal_id)
  .bind(scope)
  .execute(pool)
  .await?;
  Ok(())
}

/// Issued through the real service and store, so the stored digest is the one
/// issuance writes and not one this file computes.
async fn issue(pool: &PgPool, principal_id: i64) -> TestResult<IssuedCredential> {
  let service =
    CredentialIssuanceService::new(Arc::new(SqlxCredentialStore::from_pool(pool.clone())));
  Ok(service.issue(IssueCredential::new(principal_id, "ci-runner".to_owned())?).await?)
}

/// The digest the schema documents, computed here without the service.
fn sha256_hex(text: &str) -> String {
  use std::fmt::Write as _;

  Sha256::digest(text.as_bytes()).iter().fold(String::new(), |mut hex, byte| {
    let _ = write!(hex, "{byte:02x}");
    hex
  })
}

fn sqlstate(error: &SqlxError) -> Option<String> {
  error.as_database_error().and_then(DatabaseError::code).map(Cow::into_owned)
}

/// Grants are inserted out of order so the sorted array is the statement's
/// doing, not insertion order's.
#[tokio::test]
async fn one_lookup_returns_the_verifier_flags_sorted_scopes_and_tier() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "granted").await?;
  grant(&pool, owner, "CURATION_WRITE").await?;
  grant(&pool, owner, "CATALOG_READ").await?;
  let issued = issue(&pool, owner).await?;

  let record = SqlxCredentialLookup::from_pool(pool.clone())
    .resolve(issued.record.key_id)
    .await?
    .expect("an issued key must resolve");

  assert!(
    record.verifier.hex() == sha256_hex(issued.clear.expose_secret()),
    "the returned verifier must be the stored SHA-256 of the complete token"
  );
  assert!(!record.revoked, "a fresh credential is live");
  assert!(!record.disabled, "a fresh principal is enabled");
  assert_eq!(record.principal_id, owner);
  assert_eq!(record.scopes, [Scope::CatalogRead, Scope::CurationWrite], "sorted by code");
  assert_eq!(record.tier, TIER_CODE);
  Ok(())
}

/// The two absence shapes must not be confused: no grants is a record with an
/// empty list, and no credential is no record at all.
#[tokio::test]
async fn a_principal_without_grants_resolves_with_no_scopes_and_an_unknown_key_resolves_to_nothing()
-> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "ungranted").await?;
  let issued = issue(&pool, owner).await?;
  let lookup = SqlxCredentialLookup::from_pool(pool.clone());

  let record = lookup.resolve(issued.record.key_id).await?.expect("an issued key must resolve");
  assert_eq!(record.scopes, [], "no grants is an empty list, not an absent row");
  assert_eq!(record.principal_id, owner);

  assert!(lookup.resolve(Uuid::new_v4()).await?.is_none(), "an unissued key resolves to nothing");
  Ok(())
}

/// Revocation and disablement are the timestamps and nothing else; the flags
/// flip when the columns are set, and the row is still returned so the service
/// can compare before it reads them.
#[tokio::test]
async fn revocation_and_disablement_are_read_from_the_timestamps() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "flagged").await?;
  let issued = issue(&pool, owner).await?;
  let lookup = SqlxCredentialLookup::from_pool(pool.clone());

  query("UPDATE aircraft_auth.api_credentials SET revoked_at = now() WHERE key_id = $1")
    .bind(issued.record.key_id)
    .execute(&pool)
    .await?;
  let revoked = lookup.resolve(issued.record.key_id).await?.expect("a revoked row is returned");
  assert!(revoked.revoked && !revoked.disabled, "revocation sets only its own flag");

  query("UPDATE aircraft_auth.principals SET disabled_at = now() WHERE id = $1")
    .bind(owner)
    .execute(&pool)
    .await?;
  let disabled = lookup.resolve(issued.record.key_id).await?.expect("a disabled row is returned");
  assert!(disabled.revoked && disabled.disabled, "disablement is read from the principal");
  Ok(())
}

/// Stored state the schema forbids still fails closed if it ever appears, and
/// the failure names a class, not the value. Both rows need test-only DDL to
/// reach: the digest check and the scope foreign key make them unreachable
/// through the migrations.
#[tokio::test]
async fn stored_state_outside_the_contract_fails_closed_without_echoing_it() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let lookup = SqlxCredentialLookup::from_pool(pool.clone());

  let owner = principal(&pool, "experimental").await?;
  query("INSERT INTO aircraft_auth.scopes (code, label) VALUES ('EXPERIMENTAL', 'Not seeded')")
    .execute(&pool)
    .await?;
  grant(&pool, owner, "EXPERIMENTAL").await?;
  let issued = issue(&pool, owner).await?;
  let error = lookup
    .resolve(issued.record.key_id)
    .await
    .expect_err("a grant outside the closed vocabulary must not resolve");
  assert!(matches!(error, PersistenceError::Invariant(_)), "unexpected class: {error:?}");
  assert_absent(&error.to_string(), "EXPERIMENTAL", "the unknown scope code");

  let other = principal(&pool, "truncated").await?;
  let short = issue(&pool, other).await?;
  query("ALTER TABLE aircraft_auth.api_credentials DROP CONSTRAINT chk_apc_secret_digest")
    .execute(&pool)
    .await?;
  query("UPDATE aircraft_auth.api_credentials SET secret_digest = 'abcd' WHERE key_id = $1")
    .bind(short.record.key_id)
    .execute(&pool)
    .await?;
  let error = lookup
    .resolve(short.record.key_id)
    .await
    .expect_err("a digest that is not 32 bytes must not resolve");
  assert!(matches!(error, PersistenceError::Invariant(_)), "unexpected class: {error:?}");
  assert_absent(&error.to_string(), "abcd", "the stored digest");
  Ok(())
}

/// Both sides of the grant surface: the projection works as the runtime role,
/// and the columns and statements it was not granted are refused with `42501`
/// exactly. A wider grant passes the first half and fails the second.
#[tokio::test]
async fn the_runtime_role_executes_only_the_verification_projection() -> TestResult {
  const REFUSED: [&str; 6] = [
    "SELECT label FROM aircraft_auth.api_credentials",
    "SELECT created_at FROM aircraft_auth.api_credentials",
    "SELECT name FROM aircraft_auth.principals",
    "SELECT granted_at FROM aircraft_auth.principal_scope_grants",
    "SELECT code FROM aircraft_auth.scopes",
    "UPDATE aircraft_auth.api_credentials SET revoked_at = now()",
  ];
  let (container, admin) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&admin).await?;
  let owner = principal(&admin, "runtime").await?;
  grant(&admin, owner, "MILITARY_READ").await?;
  let issued = issue(&admin, owner).await?;
  run_psql(
    &container,
    CREATE_APP_ROLE_SQL,
    &[("app_role", RUNTIME_ROLE)],
    &[("API_ROLE_PASSWORD", RUNTIME_ROLE_PASSWORD)],
  )?;
  run_psql(&container, APP_GRANTS_SQL, &[("app_role", RUNTIME_ROLE)], &[])?;
  let url = container
    .database_url
    .replace("postgres:postgres@", &format!("{RUNTIME_ROLE}:{RUNTIME_ROLE_PASSWORD}@"));
  let runtime = connect(&url, 2, ACQUIRE_TIMEOUT_SECONDS, STATEMENT_TIMEOUT_SECONDS).await?;

  let record = SqlxCredentialLookup::from_pool(runtime.clone())
    .resolve(issued.record.key_id)
    .await?
    .expect("the runtime role must be able to verify a credential");
  assert!(
    record.verifier.hex() == sha256_hex(issued.clear.expose_secret()),
    "the runtime role must read the stored verifier"
  );
  assert_eq!(record.scopes, [Scope::MilitaryRead]);
  assert_eq!(record.tier, TIER_CODE);

  for statement in REFUSED {
    let error = query(statement)
      .execute(&runtime)
      .await
      .expect_err("the runtime role must be refused what the projection does not read");
    assert_eq!(
      sqlstate(&error).as_deref(),
      Some("42501"),
      "{statement} must be refused for insufficient privilege, not merely fail: {error}"
    );
  }
  Ok(())
}

/// The lookup travels through the application pool and inherits its bound: a
/// saturated pool of one ends the lookup at the configured acquire timeout with
/// a persistence failure, not a hang. Both sides of the elapsed time are
/// asserted, as `a_saturated_pool_gives_up_at_the_configured_acquire_timeout`
/// does for the pool itself.
#[tokio::test]
async fn a_saturated_pool_bounds_the_lookup_at_the_acquire_timeout() -> TestResult {
  const ACQUIRE_TIMEOUT: u64 = 1;
  let (container, _ready) = start_postgres(2, Duration::from_secs(2)).await?;
  let pool =
    connect(&container.database_url, 1, ACQUIRE_TIMEOUT, STATEMENT_TIMEOUT_SECONDS).await?;
  let _held = pool.acquire().await?;

  let started = Instant::now();
  let error = SqlxCredentialLookup::from_pool(pool.clone())
    .resolve(Uuid::new_v4())
    .await
    .expect_err("a lookup with no free connection must fail rather than wait indefinitely");
  let waited = started.elapsed();

  let PersistenceError::Database { message, .. } = &error else {
    return Err(format!("expected a database failure, got {error:?}").into());
  };
  assert!(message.starts_with("credential lookup:"), "the operation is named: {message}");
  assert!(waited >= Duration::from_secs(ACQUIRE_TIMEOUT), "gave up early, after {waited:?}");
  assert!(waited < Duration::from_secs(ACQUIRE_TIMEOUT + 4), "waited too long, {waited:?}");
  Ok(())
}
