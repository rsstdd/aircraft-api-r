// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Deployment gates for the application pool that `apps/server` builds.
//!
//! These run against a disposable `postgres:16-alpine` container rather than a
//! fake, because everything worth pinning here is behavior `PostgreSQL` decides:
//! whether a session setting reached the connection, whether an unqualified name
//! still resolves, and whether a restricted role is actually refused.

use std::time::{Duration, Instant};

use aircraft_app::ingestion::PersistenceError;
use aircraft_db::pool::connect;
use aircraft_testsupport::{TestResult, run_psql, start_postgres};
use sqlx_core::{
  error::{DatabaseError, Error as SqlxError},
  query::query,
  query_scalar::query_scalar,
};

/// The shipped provisioning SQL, embedded so this gate cannot drift from the
/// files an administrator actually runs through `just db-create-app-role` and
/// `just db-grant-app-role`. Both files name this test in their headers.
const CREATE_APP_ROLE_SQL: &str = include_str!("../../../database/roles/create_app_role.sql");
const APP_GRANTS_SQL: &str = include_str!("../../../database/roles/app_grants.sql");

/// A password distinctive enough that a substring search cannot match it by
/// accident, and that must never appear in a diagnostic.
const PASSWORD: &str = "n0t-in-any-diagnostic";

/// The pool bounds these gates use. Small on purpose: none of them needs
/// concurrency, and a smaller pool starts faster.
const MAX_CONNECTIONS: u32 = 2;
const ACQUIRE_TIMEOUT_SECONDS: u64 = 5;
const STATEMENT_TIMEOUT_SECONDS: u64 = 17;

/// Acceptance criterion 2. The pool is only useful if a query can travel
/// through it, so the gate is a round trip rather than a constructed value.
#[tokio::test]
async fn the_application_pool_answers_select_one() -> TestResult {
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;

  let pool = connect(
    &container.database_url,
    MAX_CONNECTIONS,
    ACQUIRE_TIMEOUT_SECONDS,
    STATEMENT_TIMEOUT_SECONDS,
  )
  .await?;
  let answer: i32 = query_scalar("SELECT 1").fetch_one(&pool).await?;

  assert_eq!(answer, 1);
  Ok(())
}

/// Both session settings are applied by the same `after_connect` statement, so
/// one gate proves the statement ran and that each value landed.
///
/// The `search_path` half deliberately does not assert `current_setting`: an
/// empty path reads back as an empty string whether it was emptied or never
/// touched, so that assertion would pass against a pool that sets nothing. What
/// can only be true once the path is empty is that an unqualified name stops
/// resolving while its qualified form still does -- which is the property the
/// setting exists for.
#[tokio::test]
async fn every_pooled_connection_carries_the_configured_statement_timeout_and_an_empty_search_path()
-> TestResult {
  let (container, ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;
  query("CREATE TABLE public.probe (id INT)").execute(&ready).await?;

  let pool = connect(
    &container.database_url,
    MAX_CONNECTIONS,
    ACQUIRE_TIMEOUT_SECONDS,
    STATEMENT_TIMEOUT_SECONDS,
  )
  .await?;

  let timeout: String =
    query_scalar("SELECT current_setting('statement_timeout')").fetch_one(&pool).await?;
  assert_eq!(timeout, "17s", "the configured statement timeout must reach every session");

  let qualified: i64 = query_scalar("SELECT count(*) FROM public.probe").fetch_one(&pool).await?;
  assert_eq!(qualified, 0, "a schema-qualified name must still resolve");

  let error = query("SELECT count(*) FROM probe")
    .fetch_one(&pool)
    .await
    .expect_err("an unqualified name must not resolve under an empty search_path");
  assert_eq!(
    error.as_database_error().and_then(DatabaseError::code).as_deref(),
    Some("42P01"),
    "an unqualified name must fail as an undefined table: {error}"
  );
  Ok(())
}

/// Bad credentials are the failure most likely to be pasted into a bug report,
/// so the gate asserts the exact failure class *and* that the report is safe to
/// paste. Asserting only the absence would pass against a pool that reported
/// nothing at all.
#[tokio::test]
async fn a_bad_password_fails_without_echoing_the_credential() -> TestResult {
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;
  let url = container.database_url.replace("postgres:postgres@", &format!("postgres:{PASSWORD}@"));

  let error = connect(&url, MAX_CONNECTIONS, ACQUIRE_TIMEOUT_SECONDS, STATEMENT_TIMEOUT_SECONDS)
    .await
    .expect_err("a bad password must not open a pool");

  let PersistenceError::Database { code, message } = &error else {
    return Err(format!("expected a database failure, got {error:?}").into());
  };
  assert_eq!(code, "DATABASE_28P01", "a rejected password must report invalid_password");
  // Deliberately does not render the message: printing it on failure would put
  // the credential in the very output this gate exists to keep clean.
  assert!(!message.contains(PASSWORD), "a rejected password was echoed in the diagnostic");
  Ok(())
}

/// The two remaining bounds in one gate, because the only observable difference
/// between them is which one fires: a pool that ignored `max_connections` would
/// hand out a second connection rather than making the caller wait, and one that
/// ignored `acquire_timeout` would wait on `SQLx`'s own default instead.
///
/// The elapsed time is asserted from both sides. An upper bound alone would pass
/// against a pool that gave up immediately, which is a different failure with
/// the same error variant.
#[tokio::test]
async fn a_saturated_pool_gives_up_at_the_configured_acquire_timeout() -> TestResult {
  const ACQUIRE_TIMEOUT: u64 = 1;
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;

  let pool =
    connect(&container.database_url, 1, ACQUIRE_TIMEOUT, STATEMENT_TIMEOUT_SECONDS).await?;
  let _held = pool.acquire().await?;

  let started = Instant::now();
  let error =
    pool.acquire().await.expect_err("a pool of one must not hand out a second connection");
  let waited = started.elapsed();

  assert!(matches!(error, SqlxError::PoolTimedOut), "a saturated pool must time out: {error}");
  assert!(
    waited >= Duration::from_secs(ACQUIRE_TIMEOUT),
    "the pool gave up before its configured timeout, after {waited:?}"
  );
  assert!(
    waited < Duration::from_secs(ACQUIRE_TIMEOUT + 4),
    "the pool waited past its configured timeout, for {waited:?}"
  );
  Ok(())
}

/// The login role the server connects as, and the password the gate provisions
/// it with. Distinct from [`PASSWORD`]: this one is meant to work.
const RUNTIME_ROLE: &str = "aircraft_api_app";
const RUNTIME_ROLE_PASSWORD: &str = "gate-only-runtime-password";

/// Acceptance criterion 3, asserted against the shipped role SQL rather than a
/// role the test invents, so it fails if `database/roles/` ever widens.
///
/// Every statement names its schema on purpose. Under the empty `search_path`
/// this pool sets, an unqualified `CREATE TABLE` or `CREATE EXTENSION` fails
/// with `3F000` -- "no schema has been selected to create in" -- which is a
/// name-resolution failure, not a privilege refusal. A gate asserting only that
/// the statement errored would pass while proving nothing about what the role
/// may do, so each case asserts `42501` exactly. `CREATE EXTENSION` reached
/// `3F000` on the first run of this gate; the exact-code assertion is what
/// caught it.
///
/// The temporary-table case is the one statement that needs no qualification: a
/// temp table resolves into `pg_temp` without consulting `search_path`. It is
/// here because the other four passed while the criterion did not hold --
/// `PostgreSQL` grants TEMPORARY on a database to PUBLIC, so the restricted
/// role created temporary tables until `app_grants.sql` revoked that grant.
#[tokio::test]
async fn the_runtime_role_cannot_create_schemas_extensions_tables_or_roles() -> TestResult {
  const REFUSED: [&str; 5] = [
    "CREATE SCHEMA escalation",
    "CREATE EXTENSION pgcrypto SCHEMA public",
    "CREATE TABLE public.escalation (id INT)",
    "CREATE ROLE escalation",
    "CREATE TEMP TABLE escalation (id INT)",
  ];
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;
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
  let pool = connect(&url, MAX_CONNECTIONS, ACQUIRE_TIMEOUT_SECONDS, STATEMENT_TIMEOUT_SECONDS)
    .await
    .expect("the restricted role must still be able to connect");

  for statement in REFUSED {
    let error = query(statement)
      .execute(&pool)
      .await
      .expect_err("the runtime role must not be able to escalate");

    assert_eq!(
      error.as_database_error().and_then(DatabaseError::code).as_deref(),
      Some("42501"),
      "{statement} must be refused for insufficient privilege, not merely fail: {error}"
    );
  }
  Ok(())
}

/// The provisioning scripts must fail loudly rather than exit zero having done
/// nothing.
///
/// Both refusals used to end in a bare `\quit`, which psql reports as exit
/// status 0 -- so [`run_psql`] returned `Ok`, the justfile recipe returned
/// success, and an operator was told a role was provisioned that either does not
/// exist or cannot authenticate. The absence assertion is the half that proves
/// the refusal happened before `CREATE ROLE`, not after it.
#[tokio::test]
async fn provisioning_refuses_a_missing_role_name_or_an_empty_password() -> TestResult {
  let (container, ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;

  run_psql(&container, CREATE_APP_ROLE_SQL, &[], &[("API_ROLE_PASSWORD", RUNTIME_ROLE_PASSWORD)])
    .expect_err("creating a role without a name must fail");
  run_psql(
    &container,
    CREATE_APP_ROLE_SQL,
    &[("app_role", RUNTIME_ROLE)],
    &[("API_ROLE_PASSWORD", "")],
  )
  .expect_err("an empty password must be refused rather than stored as a null password");
  run_psql(&container, APP_GRANTS_SQL, &[], &[])
    .expect_err("granting without a role name must fail");

  let created: bool = query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
    .bind(RUNTIME_ROLE)
    .fetch_one(&ready)
    .await?;
  assert!(!created, "a refused provisioning run must not leave a role behind");
  Ok(())
}

/// The conformance guard decides whether `app_grants.sql` may run against an
/// account that already exists, so it has to reject one that can already create
/// database objects.
///
/// Role attributes do not carry that: this role is `NOSUPERUSER NOCREATEDB
/// NOCREATEROLE NOREPLICATION NOBYPASSRLS` and holds no memberships, so the
/// attribute-only check that shipped first accepted it -- while `CREATE` on the
/// database still let it add schemas and trusted extensions after the grant.
#[tokio::test]
async fn provisioning_refuses_an_existing_role_that_already_holds_create_rights() -> TestResult {
  let (container, ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;

  // The role name is a constant in this file, not runtime data; an identifier
  // cannot be a bind parameter in DDL.
  query(&format!(
    "CREATE ROLE {RUNTIME_ROLE} LOGIN PASSWORD '{RUNTIME_ROLE_PASSWORD}'
     NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"
  ))
  .execute(&ready)
  .await?;
  query(&format!("GRANT CREATE ON DATABASE postgres TO {RUNTIME_ROLE}")).execute(&ready).await?;

  run_psql(
    &container,
    CREATE_APP_ROLE_SQL,
    &[("app_role", RUNTIME_ROLE)],
    &[("API_ROLE_PASSWORD", RUNTIME_ROLE_PASSWORD)],
  )
  .expect_err("a role that can already create schemas must not be accepted as restricted");
  Ok(())
}
