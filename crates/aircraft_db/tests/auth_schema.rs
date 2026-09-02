// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Schema gates for the authentication model introduced by
//! `database/migrations/025_authentication_schema.sql`.
//!
//! These gates use the canonical install in a disposable `PostgreSQL` container.

use std::{borrow::Cow, time::Duration};

use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use sqlx_core::{
  error::{DatabaseError, Error as SqlxError},
  query::query,
  query_scalar::query_scalar,
};
use sqlx_postgres::PgPool;
use uuid::Uuid;

/// Allowed credential columns, sorted for the catalog query.
const CREDENTIAL_COLUMNS: [&str; 7] =
  ["created_at", "key_id", "label", "principal_id", "revoked_at", "secret_digest", "updated_at"];

/// Protected policies from `docs/architecture/http_v1_decisions.md`.
const ACCEPTED_SCOPES: [&str; 5] =
  ["ADMIN", "CATALOG_READ", "CURATION_READ", "CURATION_WRITE", "MILITARY_READ"];

const TIER_CODE: &str = "TEST_TIER";

/// Non-SHA-256 shapes that must not fit the verifier column.
const REJECTED_DIGESTS: [(&str, &str); 5] = [
  ("43-char base64url", "dGhpcy1pcy1hLWNsZWFyLWNyZWRlbnRpYWwtbm90LWFfXw"),
  ("uppercase hex", "A000000000000000000000000000000000000000000000000000000000000000"),
  ("63 hex characters", "a00000000000000000000000000000000000000000000000000000000000000"),
  ("65 hex characters", "a0000000000000000000000000000000000000000000000000000000000000000"),
  ("empty", ""),
];

const UNKNOWN_PRINCIPAL: i64 = -1;

fn sqlstate(error: &SqlxError) -> Option<String> {
  error.as_database_error().and_then(DatabaseError::code).map(Cow::into_owned)
}

fn digest(lead: char) -> String {
  format!("{lead}{}", "0".repeat(63))
}

async fn tier(pool: &PgPool) -> TestResult {
  query(
    "INSERT INTO aircraft_auth.rate_limit_tiers (code, label)
         VALUES ($1, 'Test tier') ON CONFLICT (code) DO NOTHING",
  )
  .bind(TIER_CODE)
  .execute(pool)
  .await?;
  Ok(())
}

async fn principal(pool: &PgPool, name: &str) -> TestResult<i64> {
  tier(pool).await?;

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

async fn insert_credential(
  pool: &PgPool,
  key_id: Uuid,
  principal_id: i64,
  secret_digest: &str,
  label: &str,
) -> Result<(), SqlxError> {
  query(
    "INSERT INTO aircraft_auth.api_credentials (key_id, principal_id, secret_digest, label)
         VALUES ($1, $2, $3, $4)",
  )
  .bind(key_id)
  .bind(principal_id)
  .bind(secret_digest)
  .bind(label)
  .execute(pool)
  .await
  .map(|_| ())
}

#[tokio::test]
async fn a_credential_stores_only_a_key_identifier_and_a_digest() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let columns: Vec<String> = query_scalar(
    "SELECT attname::text FROM pg_attribute
         WHERE attrelid = 'aircraft_auth.api_credentials'::regclass
           AND attnum > 0 AND NOT attisdropped
         ORDER BY attname",
  )
  .fetch_all(&pool)
  .await?;
  assert_eq!(
    columns, CREDENTIAL_COLUMNS,
    "api_credentials must store only a key identifier, a digest, ownership, timestamps, and a \
     non-secret label"
  );

  let owner = principal(&pool, "inventory").await?;

  for (case, candidate) in REJECTED_DIGESTS {
    let error = insert_credential(&pool, Uuid::new_v4(), owner, candidate, "ci-runner")
      .await
      .expect_err("a digest that is not 64 lowercase hex characters must be rejected");
    assert_eq!(
      sqlstate(&error).as_deref(),
      Some("23514"),
      "{case} must violate the digest check, not merely fail: {error}"
    );
  }

  // chk_apc_label bounds both ends: the label is the only thing telling one key
  // from another in an operator's list, so it may be neither blank nor unbounded.
  for (case, label) in
    [("empty", String::new()), ("blank", "  ".to_owned()), ("long", "x".repeat(201))]
  {
    let error = insert_credential(&pool, Uuid::new_v4(), owner, &digest('b'), &label)
      .await
      .expect_err("a blank or oversized label must be rejected");
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"), "{case} label: {error}");
  }

  insert_credential(&pool, Uuid::new_v4(), owner, &digest('a'), "ci-runner")
    .await
    .expect("64 lowercase hexadecimal characters and a usable label must be accepted");
  Ok(())
}

/// Revocation does not free either credential identity for reuse.
#[tokio::test]
async fn a_revoked_key_identifier_and_a_digest_cannot_be_reused() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let owner = principal(&pool, "reuse").await?;
  let key_id = Uuid::new_v4();
  insert_credential(&pool, key_id, owner, &digest('a'), "ci-runner").await?;

  query("UPDATE aircraft_auth.api_credentials SET revoked_at = now() WHERE key_id = $1")
    .bind(key_id)
    .execute(&pool)
    .await?;

  let error = insert_credential(&pool, key_id, owner, &digest('b'), "ci-runner")
    .await
    .expect_err("a revoked key identifier must stay taken");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23505"),
    "revocation must not free a key identifier for reissue: {error}"
  );

  let error = insert_credential(&pool, Uuid::new_v4(), owner, &digest('a'), "ci-runner")
    .await
    .expect_err("two credentials must not share a digest");
  assert_eq!(sqlstate(&error).as_deref(), Some("23505"), "a digest must be unique: {error}");
  Ok(())
}

/// Credential history outlives its principal.
#[tokio::test]
async fn a_credential_requires_a_known_principal_and_blocks_its_deletion() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let error = insert_credential(&pool, Uuid::new_v4(), -1, &digest('a'), "ci-runner")
    .await
    .expect_err("a credential must belong to a principal that exists");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23503"),
    "an unknown principal must violate the foreign key: {error}"
  );

  let owner = principal(&pool, "retained").await?;
  insert_credential(&pool, Uuid::new_v4(), owner, &digest('a'), "ci-runner").await?;

  let error = query("DELETE FROM aircraft_auth.principals WHERE id = $1")
    .bind(owner)
    .execute(&pool)
    .await
    .expect_err("a principal with credentials must not be deletable");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23503"),
    "credential history must restrict the delete rather than cascade: {error}"
  );

  let surviving: i64 =
    query_scalar("SELECT count(*) FROM aircraft_auth.api_credentials WHERE principal_id = $1")
      .bind(owner)
      .fetch_one(&pool)
      .await?;
  assert_eq!(surviving, 1, "the refused delete must leave the credential in place");
  Ok(())
}

#[tokio::test]
async fn a_scope_grant_requires_a_known_principal_and_a_known_scope() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let holder = principal(&pool, "grants").await?;

  let cases = [
    ("an unknown principal", UNKNOWN_PRINCIPAL, "CATALOG_READ"),
    ("an unknown scope", holder, "NO_SUCH_SCOPE"),
  ];
  for (case, principal_id, scope_code) in cases {
    let error = query(
      "INSERT INTO aircraft_auth.principal_scope_grants (principal_id, scope_code)
           VALUES ($1, $2)",
    )
    .bind(principal_id)
    .bind(scope_code)
    .execute(&pool)
    .await
    .expect_err("a grant must reference rows that exist");
    assert_eq!(
      sqlstate(&error).as_deref(),
      Some("23503"),
      "{case} must violate the foreign key: {error}"
    );
  }
  Ok(())
}

#[tokio::test]
async fn a_principal_requires_a_known_rate_limit_tier() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let error = query(
    "INSERT INTO aircraft_auth.principals (name, rate_limit_tier_code)
         VALUES ('orphan', 'NO_SUCH_TIER')",
  )
  .execute(&pool)
  .await
  .expect_err("a principal must name a rate-limit tier that exists");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23503"),
    "an unknown tier must violate fk_prn_rate_limit_tier: {error}"
  );
  Ok(())
}

#[tokio::test]
async fn principal_and_grant_identities_are_unique() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let holder = principal(&pool, "duplicates").await?;
  let error =
    query("INSERT INTO aircraft_auth.principals (name, rate_limit_tier_code) VALUES ($1, $2)")
      .bind("duplicates")
      .bind(TIER_CODE)
      .execute(&pool)
      .await
      .expect_err("two principals must not share a name");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23505"),
    "a principal name must be unique: {error}"
  );

  let grant = "INSERT INTO aircraft_auth.principal_scope_grants (principal_id, scope_code)
       VALUES ($1, 'CATALOG_READ')";
  query(grant).bind(holder).execute(&pool).await?;
  let error = query(grant)
    .bind(holder)
    .execute(&pool)
    .await
    .expect_err("a scope must not be granted to the same principal twice");
  assert_eq!(sqlstate(&error).as_deref(), Some("23505"), "a grant pair must be unique: {error}");
  Ok(())
}

/// Principal deletion cascades grants; scope deletion remains restricted.
#[tokio::test]
async fn deleting_a_principal_discards_its_grants_but_a_granted_scope_is_retained() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let holder = principal(&pool, "cascade").await?;
  query(
    "INSERT INTO aircraft_auth.principal_scope_grants (principal_id, scope_code)
         VALUES ($1, 'CATALOG_READ')",
  )
  .bind(holder)
  .execute(&pool)
  .await?;

  let error = query("DELETE FROM aircraft_auth.scopes WHERE code = 'CATALOG_READ'")
    .execute(&pool)
    .await
    .expect_err("a granted scope must not be deletable");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23503"),
    "a scope still granted must restrict the delete: {error}"
  );

  query("DELETE FROM aircraft_auth.principals WHERE id = $1")
    .bind(holder)
    .execute(&pool)
    .await
    .expect("a principal holding only grants must be deletable");

  let remaining: i64 = query_scalar(
    "SELECT count(*) FROM aircraft_auth.principal_scope_grants WHERE principal_id = $1",
  )
  .bind(holder)
  .fetch_one(&pool)
  .await?;
  assert_eq!(remaining, 0, "deleting a principal must discard its grants");
  Ok(())
}

#[tokio::test]
async fn disablement_and_revocation_cannot_precede_creation() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let subject = principal(&pool, "temporal").await?;
  let key_id = Uuid::new_v4();
  insert_credential(&pool, key_id, subject, &digest('a'), "ci-runner").await?;

  let error = query(
    "UPDATE aircraft_auth.principals
         SET disabled_at = created_at - interval '1 second' WHERE id = $1",
  )
  .bind(subject)
  .execute(&pool)
  .await
  .expect_err("a principal must not be disabled before it existed");
  assert_eq!(
    sqlstate(&error).as_deref(),
    Some("23514"),
    "disabled_at must be constrained: {error}"
  );

  let error = query(
    "UPDATE aircraft_auth.api_credentials
         SET revoked_at = created_at - interval '1 second' WHERE key_id = $1",
  )
  .bind(key_id)
  .execute(&pool)
  .await
  .expect_err("a credential must not be revoked before it was issued");
  assert_eq!(sqlstate(&error).as_deref(), Some("23514"), "revoked_at must be constrained: {error}");
  Ok(())
}

/// Backdating prevents a missing trigger from passing vacuously.
#[tokio::test]
async fn disabling_a_principal_stamps_updated_at() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  tier(&pool).await?;
  let subject: i64 = query_scalar(
    "INSERT INTO aircraft_auth.principals (name, rate_limit_tier_code, created_at, updated_at)
         VALUES ('stamped', $1, now() - interval '1 day', now() - interval '1 day')
         RETURNING id",
  )
  .bind(TIER_CODE)
  .fetch_one(&pool)
  .await?;

  query("UPDATE aircraft_auth.principals SET disabled_at = now() WHERE id = $1")
    .bind(subject)
    .execute(&pool)
    .await?;

  let stamped: bool = query_scalar(
    "SELECT updated_at > created_at AND disabled_at IS NOT NULL
         FROM aircraft_auth.principals WHERE id = $1",
  )
  .bind(subject)
  .fetch_one(&pool)
  .await?;
  assert!(stamped, "trg_prn_updated must advance updated_at when a principal is disabled");
  Ok(())
}

#[tokio::test]
async fn every_accepted_route_policy_scope_is_seeded() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let seeded: Vec<String> =
    query_scalar("SELECT code::text FROM aircraft_auth.scopes ORDER BY code")
      .fetch_all(&pool)
      .await?;
  assert_eq!(
    seeded, ACCEPTED_SCOPES,
    "the seeded scopes must be exactly the protected policies of \
     docs/architecture/http_v1_decisions.md; Public requires no scope"
  );
  Ok(())
}
