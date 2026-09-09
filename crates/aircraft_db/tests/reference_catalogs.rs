//! What the reference-catalog adapter owes: every catalog reads its own table,
//! serves only active rows, orders them totally, and refuses to answer a catalog
//! that has outgrown its ceiling rather than truncating one.
//!
//! Against canonical seeded data, as `crates/AGENTS.md` requires: `SCHEMA_STEPS`
//! installs `database/migrations/002_core_reference_tables.sql` and both
//! reference seeds, so all thirty-six catalogs are populated exactly as a
//! deployment would have them.
//!
//! Two conditions the acceptance criteria name cannot be observed against the
//! seeds as shipped, and are created here instead: neither seed file sets
//! `is_active` at all, so every seeded row takes the column's `DEFAULT TRUE`,
//! and no catalog is empty.

// A failing assertion is the point of a test.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use aircraft_app::{
  ingestion::PersistenceError,
  reference::{CatalogReader, MAX_CATALOG_ROWS},
};
use aircraft_db::{SqlxCatalogReader, pool::connect};
use aircraft_domain::reference::Catalog;
use aircraft_testsupport::{TestResult, install_schema, run_psql, start_postgres};
use sqlx_core::error::Error as SqlxError;
use sqlx_core::{query::query, query_scalar::query_scalar, row::Row};
use sqlx_postgres::PgPool;

/// The table a catalog's slug addresses, derived from the public spelling by the
/// documented convention rather than read out of the adapter.
///
/// Independent on purpose: every expectation below is built with this, so a
/// production statement pointed at the wrong table disagrees with it instead of
/// agreeing with itself.
fn table_of(catalog: Catalog) -> String {
  catalog.slug().replace('-', "_")
}

/// The grant files, read rather than restated, so the gate runs what
/// `just db-grant-app-role` runs.
const CREATE_APP_ROLE_SQL: &str = include_str!("../../../database/roles/create_app_role.sql");
const APP_GRANTS_SQL: &str = include_str!("../../../database/roles/app_grants.sql");
const RUNTIME_ROLE: &str = "aircraft_api_app";
const RUNTIME_ROLE_PASSWORD: &str = "gate-only-runtime-password";
const ACQUIRE_TIMEOUT_SECONDS: u64 = 2;
const STATEMENT_TIMEOUT_SECONDS: u64 = 5;

fn sqlstate(error: &SqlxError) -> Option<String> {
  match error {
    SqlxError::Database(database) => database.code().map(std::borrow::Cow::into_owned),
    _ => None,
  }
}

/// The restricted runtime role reads every catalog and can write none of them.
///
/// Every other test in this file connects as the container's owner, which holds
/// every privilege and therefore cannot see a missing grant: the route would
/// pass all of them and still answer `503` in production, where
/// `database/roles/app_grants.sql` is what the server actually connects with.
/// This is the only test that runs the real grant files and connects as the
/// role they describe, so it is the one that can fail for `42501`.
#[tokio::test]
async fn the_runtime_role_reads_every_catalog_and_writes_none() -> TestResult {
  let (container, admin) = start_postgres(2, Duration::from_secs(30)).await?;
  install_schema(&admin).await?;
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
  let reader = SqlxCatalogReader::new(runtime.clone());

  for catalog in Catalog::ALL {
    let entries = reader
      .entries(catalog)
      .await
      .unwrap_or_else(|error| panic!("the runtime role must read {}: {error}", table_of(catalog)));
    assert!(!entries.is_empty(), "{} is seeded", table_of(catalog));
  }

  // Reference data is curated through migrations and seeds, never by the API.
  // A read grant that carried a write with it would let a compromised request
  // path rewrite the vocabulary every other table's foreign keys point at.
  for statement in [
    "UPDATE aircraft_ref.fuel_types SET label = 'x'",
    "INSERT INTO aircraft_ref.fuel_types (code, label) VALUES ('X', 'x')",
    "DELETE FROM aircraft_ref.fuel_types",
  ] {
    let error = query(statement)
      .execute(&runtime)
      .await
      .expect_err("the runtime role must not write reference data");
    assert_eq!(
      sqlstate(&error).as_deref(),
      Some("42501"),
      "{statement} must be refused for insufficient privilege, not merely fail: {error}"
    );
  }
  Ok(())
}

/// The codes a catalog holds, read independently of the adapter.
///
/// Deliberately not the adapter's own statement: this is the expectation the
/// adapter is compared against, so it is built here from [`table_of`] with no
/// `WHERE` and no ordering of its own. A statement pointed at the wrong table
/// agrees with itself and disagrees with this.
async fn codes_in_table(pool: &PgPool, catalog: Catalog) -> TestResult<Vec<String>> {
  let sql = format!("SELECT code FROM aircraft_ref.{} ORDER BY code", table_of(catalog));
  Ok(query(&sql).fetch_all(pool).await?.iter().map(|row| row.get("code")).collect())
}

/// Whether the catalog's table declares `column`, read from the live schema.
///
/// `information_schema` rather than `Catalog` or `statement`: the expectation
/// each sweep below compares against must come from the installed schema, not
/// from the Rust under test. A statement whose predicate or sort was copied from
/// a neighbouring catalog agrees with itself and disagrees with this.
async fn declares(pool: &PgPool, catalog: Catalog, column: &str) -> TestResult<bool> {
  Ok(
    query_scalar(
      "SELECT EXISTS (SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_ref' AND table_name = $1 AND column_name = $2)",
    )
    .bind(table_of(catalog))
    .bind(column)
    .fetch_one(pool)
    .await?,
  )
}

#[tokio::test]
async fn every_catalog_serves_the_rows_seeded_for_its_own_table() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxCatalogReader::new(pool.clone());

  for catalog in Catalog::ALL {
    let mut served: Vec<String> =
      reader.entries(catalog).await?.into_iter().map(|entry| entry.code).collect();
    served.sort_unstable();

    let expected = codes_in_table(&pool, catalog).await?;

    assert!(!expected.is_empty(), "{} is seeded, or this proves nothing", table_of(catalog));
    assert_eq!(served, expected, "catalog {} must serve its own rows", table_of(catalog));
  }
  Ok(())
}

/// Every catalog that has an `is_active` column withholds a deactivated row, and
/// every catalog that has none still serves all of its rows.
///
/// All thirty-six, not one: the per-catalog statements are thirty-six separate
/// literals, so a missing `WHERE is_active` is a per-catalog defect that a
/// single-catalog test cannot see. Deleting the predicate from any one of the
/// thirty-five previously left the whole suite green.
#[tokio::test]
async fn every_catalog_with_an_active_flag_withholds_a_deactivated_row() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxCatalogReader::new(pool.clone());
  let mut filtered = 0_usize;
  let mut unfiltered = 0_usize;

  for catalog in Catalog::ALL {
    let before = reader.entries(catalog).await?;
    let target =
      before.first().unwrap_or_else(|| panic!("{} is seeded", table_of(catalog))).code.clone();

    if !declares(&pool, catalog, "is_active").await? {
      let after = reader.entries(catalog).await?;
      assert_eq!(after.len(), before.len(), "{} has no active flag", table_of(catalog));
      unfiltered += 1;
      continue;
    }

    let sql =
      format!("UPDATE aircraft_ref.{} SET is_active = FALSE WHERE code = $1", table_of(catalog));
    query(&sql).bind(&target).execute(&pool).await?;
    let after: Vec<String> =
      reader.entries(catalog).await?.into_iter().map(|entry| entry.code).collect();

    assert!(!after.contains(&target), "{} still serves {target}", table_of(catalog));
    assert_eq!(after.len(), before.len() - 1, "{} withheld more than one row", table_of(catalog));
    filtered += 1;
  }

  assert_eq!(filtered, 35, "35 catalogs carry is_active; the sweep must have exercised each");
  assert_eq!(unfiltered, 1, "only unit_categories has no active flag");
  Ok(())
}

/// Every catalog is served in the order its own columns define -- not merely in
/// some deterministic order.
///
/// The expectation is built per catalog from the live schema, so a statement
/// that dropped `sort_order` and ordered by `code` alone fails here even though
/// its result is still totally ordered. Deleting `sort_order` from any one arm
/// previously left the suite green.
#[tokio::test]
async fn every_catalog_is_served_in_the_order_its_own_columns_define() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxCatalogReader::new(pool.clone());
  let mut sorted = 0_usize;

  for catalog in Catalog::ALL {
    let filter = if declares(&pool, catalog, "is_active").await? { "WHERE is_active" } else { "" };
    let order = if declares(&pool, catalog, "sort_order").await? {
      sorted += 1;
      "sort_order, code"
    } else {
      "code"
    };
    let sql =
      format!("SELECT code FROM aircraft_ref.{} {filter} ORDER BY {order}", table_of(catalog));

    let expected: Vec<String> =
      query(&sql).fetch_all(&pool).await?.iter().map(|row| row.get("code")).collect();
    let served: Vec<String> =
      reader.entries(catalog).await?.into_iter().map(|entry| entry.code).collect();

    assert!(!expected.is_empty(), "{} is seeded, or this proves nothing", table_of(catalog));
    assert_eq!(served, expected, "{} must serve its own declared order", table_of(catalog));
  }

  assert_eq!(sorted, 35, "35 catalogs carry sort_order; the sweep must have exercised each");
  Ok(())
}

/// A repeated `sort_order` is the only place an order without its tiebreaker
/// stops being deterministic, and the sweep above cannot say whether any fixture
/// actually has one. `measurement_units` does --
/// `crates/aircraft_db/tests/keyset_pagination.rs` records 14 rows sharing `10`
/// and 13 sharing `20` -- so this asserts the fixture earns the sweep.
#[tokio::test]
async fn the_ordered_fixture_actually_repeats_a_sort_value() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let repeated = query(
    "SELECT count(*) AS n FROM aircraft_ref.measurement_units a
     WHERE EXISTS (SELECT 1 FROM aircraft_ref.measurement_units b
                   WHERE b.sort_order = a.sort_order AND b.code <> a.code)",
  )
  .fetch_one(&pool)
  .await?
  .get::<i64, _>("n");

  assert!(repeated > 0, "no catalog shares a sort value, so the tiebreaker is untested");
  Ok(())
}

/// The three catalogs whose tables have no `description` column serve the member
/// as absent rather than failing to compile in `PostgreSQL`, and a catalog that
/// has one still carries its text.
#[tokio::test]
async fn a_catalog_without_a_description_column_serves_none_for_it() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxCatalogReader::new(pool.clone());

  for catalog in [Catalog::MeasurementUnits, Catalog::CertificationAuthorities, Catalog::Currencies]
  {
    let entries = reader.entries(catalog).await?;
    assert!(!entries.is_empty(), "{} is seeded", table_of(catalog));
    assert!(
      entries.iter().all(|entry| entry.description.is_none()),
      "{} has no description column",
      table_of(catalog)
    );
  }

  let described = reader.entries(Catalog::ServiceStatuses).await?;
  assert!(
    described.iter().any(|entry| entry.description.is_some()),
    "a catalog that has descriptions must serve them, or the assertion above is vacuous"
  );
  Ok(())
}

/// The accepted decision's "A known parent with no child records returns a
/// successful empty collection": an emptied catalog is `Ok(vec![])`, never an
/// error and never a `404`. No seeded catalog is empty, so this one is emptied.
#[tokio::test]
async fn a_catalog_with_no_rows_is_served_as_an_empty_collection() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxCatalogReader::new(pool.clone());

  query("DELETE FROM aircraft_ref.availability_grades").execute(&pool).await?;

  assert_eq!(reader.entries(Catalog::AvailabilityGrades).await?, Vec::new());
  Ok(())
}

/// A catalog past its ceiling is refused, not truncated. `AGENTS.md` requires
/// the bound; this proves the bound reports itself rather than quietly serving a
/// short catalog that reads exactly like a complete one.
#[tokio::test]
async fn a_catalog_past_its_ceiling_is_refused_rather_than_truncated() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxCatalogReader::new(pool.clone());

  // `ad_types` is the smallest seeded catalog and has no foreign keys pointing
  // in, so it can be grown without disturbing anything else in the schema.
  query(
    "INSERT INTO aircraft_ref.ad_types (code, label, sort_order)
     SELECT 'BULK_' || n, 'bulk ' || n, 0 FROM generate_series(1, $1) AS n",
  )
  .bind(i32::try_from(MAX_CATALOG_ROWS)?)
  .execute(&pool)
  .await?;

  match reader.entries(Catalog::AdTypes).await {
    Err(PersistenceError::Invariant(message)) => {
      assert!(message.contains("ad-types"), "the refusal names the catalog: {message}");
    }
    other => panic!("an overgrown catalog must be refused, got {other:?}"),
  }
  Ok(())
}
