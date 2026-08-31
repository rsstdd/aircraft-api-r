// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Deployment gates for the readiness adapter behind `/ready`.
//!
//! The interesting cases are all ones `PostgreSQL` and the pool decide together:
//! whether a live database answers, whether a stopped one is reported rather
//! than hung on, and whether a pool with no free connection gives up at the
//! readiness deadline instead of the pool's own acquire timeout. None of that is
//! observable against a fake, so these run against a disposable container.

use std::time::{Duration, Instant};

use aircraft_app::{ingestion::PersistenceError, readiness::ReadinessProbe};
use aircraft_db::{pool::connect, readiness::PoolReadiness};
use aircraft_testsupport::{TestResult, start_postgres};

const MAX_CONNECTIONS: u32 = 1;
const ACQUIRE_TIMEOUT_SECONDS: u64 = 5;
const STATEMENT_TIMEOUT_SECONDS: u64 = 17;

/// The readiness deadline in `aircraft_db::readiness`. Duplicated here on
/// purpose: the constant is private, and a test that imported it would still
/// pass if both moved together.
const DEADLINE: Duration = Duration::from_millis(250);

/// Acceptance criterion 2.
#[tokio::test]
async fn a_reachable_database_reports_ready() -> TestResult {
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;
  let pool = connect(
    &container.database_url,
    MAX_CONNECTIONS,
    ACQUIRE_TIMEOUT_SECONDS,
    STATEMENT_TIMEOUT_SECONDS,
  )
  .await?;

  PoolReadiness::new(pool).check().await.expect("a live database must report ready");
  Ok(())
}

/// Acceptance criterion 3, the unreachable half. The container is removed after
/// the pool is built, so this exercises a database that went away underneath a
/// working pool -- the failure an operator actually sees -- rather than one that
/// was never reachable. Dropping the guard is what removes it;
/// `DockerPostgres` owns that in `Drop`.
#[tokio::test]
async fn a_stopped_database_reports_unavailable() -> TestResult {
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;
  let pool = connect(
    &container.database_url,
    MAX_CONNECTIONS,
    ACQUIRE_TIMEOUT_SECONDS,
    STATEMENT_TIMEOUT_SECONDS,
  )
  .await?;
  let probe = PoolReadiness::new(pool);
  probe.check().await.expect("the database must be ready before it is stopped");

  drop(container);

  let error = probe.check().await.expect_err("a stopped database must not report ready");
  assert!(
    matches!(error, PersistenceError::Database { .. }),
    "readiness must fail as a database error: {error}"
  );
  Ok(())
}

/// Acceptance criterion 3, the saturated half, and the gate that proves the
/// readiness deadline is the bound that fires.
///
/// The pool holds one connection and the test keeps it, so the probe can only
/// wait. Elapsed time is asserted from both sides: an upper bound alone would
/// pass against a probe that gave up instantly, and the lower bound alone would
/// pass against one that waited for the pool's five-second acquire timeout. Only
/// the pair identifies the 250 ms deadline as the thing that ended the wait.
#[tokio::test]
async fn a_saturated_pool_reports_unavailable_within_the_readiness_deadline() -> TestResult {
  let (container, _ready) = start_postgres(MAX_CONNECTIONS, Duration::from_secs(2)).await?;
  let pool = connect(
    &container.database_url,
    MAX_CONNECTIONS,
    ACQUIRE_TIMEOUT_SECONDS,
    STATEMENT_TIMEOUT_SECONDS,
  )
  .await?;
  let held = pool.acquire().await?;
  let probe = PoolReadiness::new(pool.clone());

  let started = Instant::now();
  let error = probe.check().await.expect_err("a saturated pool must not report ready");
  let waited = started.elapsed();

  assert!(
    matches!(&error, PersistenceError::Database { code, .. } if code == "DATABASE_UNAVAILABLE"),
    "a saturated pool must report DATABASE_UNAVAILABLE: {error}"
  );
  assert!(waited >= DEADLINE, "readiness gave up before its deadline, after {waited:?}");
  assert!(
    waited < Duration::from_secs(ACQUIRE_TIMEOUT_SECONDS),
    "readiness waited for the pool's acquire timeout rather than its own, for {waited:?}"
  );

  drop(held);
  Ok(())
}
