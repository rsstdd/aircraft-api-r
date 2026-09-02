// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Issuance gates for `aircraft_auth.api_credentials`, driven through the real
//! `CredentialIssuanceService` over `SqlxCredentialStore` against the canonical
//! install in a disposable `PostgreSQL` container.
//!
//! Every assertion that touches a token, a secret, a digest, a stored row, or
//! captured output fails with a fixed sentence and never renders the value:
//! these tests must be safe when they fail, not only when they pass.
//!
//! The tracing gates live in this binary rather than beside the unit tests for
//! the reason `crates/aircraft_api/tests/request_tracing.rs` records: a
//! callsite first reached with no dispatch is disabled for the process.

use std::{
  panic::{AssertUnwindSafe, catch_unwind},
  sync::{Arc, Mutex},
  time::Duration,
};

use aircraft_app::credential_issuance::{
  CredentialIssuanceError, CredentialIssuanceService, CredentialStore, CredentialVerifier,
  IssueCredential, IssuedCredential, NewCredential,
};
use aircraft_app::ingestion::PersistenceError;
use aircraft_db::SqlxCredentialStore;
use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use sha2::{Digest, Sha256};
use sqlx_core::{query::query, query_as::query_as, query_scalar::query_scalar};
use sqlx_postgres::PgPool;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

const TIER_CODE: &str = "TEST_TIER";
/// Positive, so it passes input validation and fails only at the foreign key.
const UNKNOWN_PRINCIPAL: i64 = 9_999;
/// A verifier-shaped value that `PostgreSQL` will echo in a unique-violation
/// detail; recognizable, and nothing a real issuance could produce.
const SENTINEL_DIGEST: [u8; 32] = [
  0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef,
  0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef,
];

/// Fails with a fixed sentence: rendering either argument would print the
/// very material the assertion exists to keep out of output.
fn assert_absent(haystack: &str, needle: &str, what: &str) {
  assert!(!haystack.contains(needle), "{what} was disclosed");
}

#[test]
fn a_failing_absence_assertion_discloses_nothing() {
  let failure = catch_unwind(AssertUnwindSafe(|| {
    assert_absent("before NEEDLE after", "NEEDLE", "the needle");
  }))
  .expect_err("the assertion must fail when the needle is present");

  let message = failure
    .downcast_ref::<String>()
    .cloned()
    .or_else(|| failure.downcast_ref::<&str>().map(|text| (*text).to_owned()))
    .expect("panic payload is text");
  assert!(message.contains("the needle was disclosed"), "the fixed sentence must be the message");
  assert!(!message.contains("NEEDLE"), "the panic message must not carry the needle");
  assert!(!message.contains("before"), "the panic message must not carry the haystack");
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

fn store(pool: &PgPool) -> SqlxCredentialStore {
  SqlxCredentialStore::from_pool(pool.clone())
}

fn service(pool: &PgPool) -> CredentialIssuanceService {
  CredentialIssuanceService::new(Arc::new(store(pool)))
}

fn request(principal_id: i64) -> TestResult<IssueCredential> {
  Ok(IssueCredential::new(principal_id, "ci-runner".to_owned())?)
}

async fn credential_count(pool: &PgPool) -> TestResult<i64> {
  Ok(query_scalar("SELECT count(*) FROM aircraft_auth.api_credentials").fetch_one(pool).await?)
}

/// Every column of the row as text, so the absence check covers columns this
/// file never names.
async fn row_text(pool: &PgPool, issued: &IssuedCredential) -> TestResult<String> {
  let text = query_scalar(
    "SELECT row_to_json(c)::text FROM aircraft_auth.api_credentials c WHERE key_id = $1",
  )
  .bind(issued.record.key_id)
  .fetch_one(pool)
  .await?;
  Ok(text)
}

/// The digest the schema documents, computed here without the service.
fn sha256_hex(text: &str) -> String {
  use std::fmt::Write as _;

  Sha256::digest(text.as_bytes()).iter().fold(String::new(), |mut hex, byte| {
    let _ = write!(hex, "{byte:02x}");
    hex
  })
}

fn secret_segment(issued: &IssuedCredential) -> String {
  issued.clear.expose_secret().rsplit('_').next().expect("token has a secret segment").to_owned()
}

/// Collects formatted `tracing` output so a test can read an event back; the
/// same twenty lines as `crates/aircraft_api/tests/request_tracing.rs`.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
  fn contents(&self) -> String {
    String::from_utf8_lossy(&self.0.lock().expect("the log buffer lock is poisoned")).into_owned()
  }
}

impl std::io::Write for CapturedLogs {
  fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
    self.0.lock().expect("the log buffer lock is poisoned").extend_from_slice(buffer);
    Ok(buffer.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
  type Writer = Self;

  fn make_writer(&'writer self) -> Self::Writer {
    self.clone()
  }
}

/// Runs one issuance with a JSON subscriber attached to that future alone.
async fn traced_issue(
  pool: &PgPool,
  principal_id: i64,
) -> TestResult<(Result<IssuedCredential, CredentialIssuanceError>, String)> {
  let logs = CapturedLogs::default();
  let subscriber = tracing_subscriber::fmt()
    .json()
    .flatten_event(true)
    .with_max_level(tracing::Level::TRACE)
    .with_writer(logs.clone())
    .finish();

  let outcome = service(pool)
    .issue(request(principal_id)?)
    .with_subscriber(tracing::Dispatch::new(subscriber))
    .await;
  Ok((outcome, logs.contents()))
}

/// True when `text` carries a run of hexadecimal digits at least `length` long,
/// the shape of a digest or a raw secret, wherever it came from.
fn has_hex_run(text: &str, length: usize) -> bool {
  text.split(|character: char| !character.is_ascii_hexdigit()).any(|run| run.len() >= length)
}

#[tokio::test]
async fn issuance_persists_the_verifier_and_never_the_token() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "inventory").await?;

  let issued = service(&pool).issue(request(owner)?).await?;

  let token = issued.clear.expose_secret();
  let secret = secret_segment(&issued);
  assert_eq!(secret.len(), 64, "32 secret bytes encode to 64 hex characters");
  let (stored_digest, revoked): (String, Option<DateTime<Utc>>) = query_as(
    "SELECT secret_digest, revoked_at FROM aircraft_auth.api_credentials WHERE key_id = $1",
  )
  .bind(issued.record.key_id)
  .fetch_one(&pool)
  .await?;
  assert!(
    stored_digest == sha256_hex(token),
    "the stored digest must be SHA-256 of the whole returned token"
  );
  assert_eq!(revoked, None, "a fresh credential is live");
  assert_eq!(issued.record.principal_id, owner);
  assert_eq!(issued.record.label, "ci-runner");
  assert!(issued.record.updated_at >= issued.record.created_at, "timestamps come from the row");

  let row = row_text(&pool, &issued).await?;
  assert_absent(&row, token, "the clear token");
  assert_absent(&row, &secret, "the secret");
  assert_eq!(credential_count(&pool).await?, 1);
  Ok(())
}

/// AC3 by absence: the failure is typed *and* the table stays empty.
#[tokio::test]
async fn issuing_against_an_unknown_principal_persists_nothing() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let error = service(&pool)
    .issue(request(UNKNOWN_PRINCIPAL)?)
    .await
    .expect_err("a credential must belong to a principal that exists");

  assert!(
    matches!(error, CredentialIssuanceError::UnknownPrincipal(UNKNOWN_PRINCIPAL)),
    "the foreign-key violation must surface as the typed variant"
  );
  assert_eq!(credential_count(&pool).await?, 0, "a refused issuance must leave no row");
  Ok(())
}

/// The INSERT succeeds, then decoding the returned row fails, and the store
/// must still commit nothing.
///
/// The type change is test-only DDL, the narrowest way to make a decode fail
/// after a successful INSERT: `created_at` is filled by its default, so the
/// statement is untouched, while `DateTime<Utc>` decodes only `timestamptz`.
#[tokio::test]
async fn a_decoding_failure_after_the_insert_leaves_no_row() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "undecodable").await?;
  query("ALTER TABLE aircraft_auth.api_credentials ALTER COLUMN created_at TYPE timestamp")
    .execute(&pool)
    .await?;

  let error = service(&pool)
    .issue(request(owner)?)
    .await
    .expect_err("a row that cannot be decoded must not be reported as issued");

  assert!(
    matches!(error, CredentialIssuanceError::Persistence(PersistenceError::Database { .. })),
    "a decode failure is a persistence failure"
  );
  assert_eq!(credential_count(&pool).await?, 0, "a decode failure must roll the insert back");
  Ok(())
}

#[tokio::test]
async fn two_issuances_have_distinct_identifiers_and_secrets() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "pair").await?;
  let service = service(&pool);

  let first = service.issue(request(owner)?).await?;
  let second = service.issue(request(owner)?).await?;

  assert_ne!(first.record.key_id, second.record.key_id, "identifiers must differ");
  assert!(
    first.clear.expose_secret() != second.clear.expose_secret(),
    "two issuances must not return the same token"
  );
  assert!(
    secret_segment(&first) != secret_segment(&second),
    "two issuances must not share a secret segment"
  );
  let digests: Vec<String> =
    query_scalar("SELECT DISTINCT secret_digest FROM aircraft_auth.api_credentials")
      .fetch_all(&pool)
      .await?;
  assert_eq!(digests.len(), 2, "two rows with two distinct digests");
  Ok(())
}

#[tokio::test]
async fn issuance_logs_carry_no_credential_material() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "traced").await?;

  let (outcome, raw) = traced_issue(&pool, owner).await?;

  let issued = outcome?;
  let token = issued.clear.expose_secret();
  // The identifier proves the subscriber saw the issuance event at all.
  assert!(raw.contains(&issued.record.key_id.to_string()), "the issuance event was not captured");
  assert_absent(&raw, token, "the clear token");
  assert_absent(&raw, &secret_segment(&issued), "the secret");
  assert_absent(&raw, &sha256_hex(token), "the verifier");
  Ok(())
}

/// A unique violation on `secret_digest` is the one path where `PostgreSQL`
/// itself writes a verifier into a diagnostic, in the detail field. The mapped
/// error must keep the constraint and SQLSTATE and drop the value.
#[tokio::test]
async fn a_duplicate_digest_diagnostic_redacts_the_stored_verifier() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "collision").await?;
  let sentinel = CredentialVerifier::from_digest(SENTINEL_DIGEST);
  query(
    "INSERT INTO aircraft_auth.api_credentials (key_id, principal_id, secret_digest, label)
         VALUES ($1, $2, $3, 'existing')",
  )
  .bind(Uuid::new_v4())
  .bind(owner)
  .bind(sentinel.hex())
  .execute(&pool)
  .await?;

  let error = store(&pool)
    .persist(NewCredential {
      key_id: Uuid::new_v4(),
      principal_id: owner,
      verifier: sentinel.clone(),
      label: "colliding".to_owned(),
    })
    .await
    .expect_err("a second row with the same digest must be refused");

  let rendered = format!("{error} {error:?}");
  assert_eq!(error.code(), "DATABASE_23505", "the SQLSTATE must survive redaction");
  assert!(rendered.contains("credential insert"), "the operation must be named");
  assert!(rendered.contains("uq_apc_secret_digest"), "the violated constraint must be named");
  assert_absent(&rendered, &sentinel.hex(), "the stored verifier");
  assert!(!has_hex_run(&rendered, 32), "no digest-shaped value may survive in the diagnostic");
  Ok(())
}

/// The failure event names the outcome so an operator can act on it, and
/// carries nothing digest-shaped, whichever diagnostic the database produced.
#[tokio::test]
async fn a_failed_issuance_is_traced_without_credential_material() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let (outcome, raw) = traced_issue(&pool, UNKNOWN_PRINCIPAL).await?;

  assert!(outcome.is_err(), "issuing to an unknown principal must fail");
  assert!(raw.contains("UNKNOWN_PRINCIPAL"), "the failure event was not captured");
  assert!(!has_hex_run(&raw, 32), "no digest-shaped value may reach the trace");
  Ok(())
}
