//! What the family adapter owes: a bounded, ordered page that resumes without a
//! gap or a repeat, a detail lookup that separates absence from failure, values
//! that survive as the schema stores them, and a statement whose parameters are
//! bound rather than interpolated.
//!
//! Against the canonical schema, as `crates/AGENTS.md` requires: `install_schema`
//! runs the migrations and seeds, so `aircraft_geo.countries`,
//! `aircraft_org.organizations`, and the `aircraft_ref` lookups are the ones a
//! deployment has. `aircraft_core.families` is populated by no seed, so each
//! test inserts the rows it asserts on.

// A failing assertion is the point of a test.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use aircraft_app::{
  catalog::{FamilyFilter, FamilyReader as _},
  ingestion::PersistenceError,
  pagination::PageLimit,
};
use aircraft_db::{SqlxFamilyReader, pool::connect};
use aircraft_domain::catalog::{CountryCode, Slug};
use aircraft_testsupport::{TestResult, install_schema, run_psql, start_postgres};
use sqlx_core::{error::Error as SqlxError, query::query, query_scalar::query_scalar};
use sqlx_postgres::PgPool;

/// The grant files, read rather than restated, so the gate runs what
/// `just db-grant-app-role` runs.
const CREATE_APP_ROLE_SQL: &str = include_str!("../../../database/roles/create_app_role.sql");
const APP_GRANTS_SQL: &str = include_str!("../../../database/roles/app_grants.sql");
const RUNTIME_ROLE: &str = "aircraft_api_app";
const RUNTIME_ROLE_PASSWORD: &str = "gate-only-runtime-password";

/// Inserts one family, with every optional column left NULL.
async fn insert_family(pool: &PgPool, slug: &str, name: &str) -> TestResult {
  query("INSERT INTO aircraft_core.families (slug, name) VALUES ($1, $2)")
    .bind(slug)
    .bind(name)
    .execute(pool)
    .await?;
  Ok(())
}

fn sqlstate(error: &SqlxError) -> Option<String> {
  match error {
    SqlxError::Database(database) => database.code().map(std::borrow::Cow::into_owned),
    _ => None,
  }
}

fn slug(value: &str) -> Slug {
  Slug::try_from(value).expect("a slug_text value")
}

fn limit(value: u16) -> PageLimit {
  PageLimit::from_requested(std::num::NonZeroU16::new(value))
}

#[tokio::test]
async fn an_empty_table_is_a_successful_empty_page() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxFamilyReader::new(pool.clone());

  let page = reader.list_families(&FamilyFilter::default(), limit(50), None).await?;

  assert!(page.items().is_empty(), "no families are seeded, so the page is empty");
  assert!(page.next().is_none(), "an empty page has no continuation");
  Ok(())
}

/// AC1. The whole table, page by page: the visited slugs must equal the stored
/// order with no repeat and no gap. A single-boundary assertion would pass while
/// every later page skipped a row, which is why this walks to exhaustion --
/// `crates/aircraft_db/tests/keyset_pagination.rs` established the shape.
#[tokio::test]
async fn paging_visits_every_family_exactly_once_in_slug_order() -> TestResult {
  const MAX_PAGES: usize = 32;

  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxFamilyReader::new(pool.clone());

  let stored = ["airbus-a320", "boeing-737", "cessna-172", "embraer-e175", "piper-pa28"];
  for name in stored {
    insert_family(&pool, name, name).await?;
  }

  let mut visited: Vec<String> = Vec::new();
  let mut resume: Option<Slug> = None;
  for _ in 0..MAX_PAGES {
    let page = reader.list_families(&FamilyFilter::default(), limit(2), resume.as_ref()).await?;
    visited.extend(page.items().iter().map(|row| row.slug.as_str().to_owned()));
    match page.next() {
      Some(next) => resume = Some(next.clone()),
      None => break,
    }
  }

  assert_eq!(visited, stored, "every family exactly once, in slug order");
  Ok(())
}

/// AC1. Two families share a name, so a `name`-ordered page would be
/// non-deterministic. Ordering by the unique slug keeps it total.
#[tokio::test]
async fn families_sharing_a_name_are_still_totally_ordered() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxFamilyReader::new(pool.clone());

  insert_family(&pool, "cessna-172-early", "Cessna 172").await?;
  insert_family(&pool, "cessna-172-late", "Cessna 172").await?;

  let first = reader.list_families(&FamilyFilter::default(), limit(1), None).await?;
  let resume = first.next().expect("a continuation").clone();
  let second = reader.list_families(&FamilyFilter::default(), limit(1), Some(&resume)).await?;

  assert_eq!(first.items()[0].slug.as_str(), "cessna-172-early");
  assert_eq!(second.items()[0].slug.as_str(), "cessna-172-late", "the second page does not repeat");
  Ok(())
}

/// AC2. Present and absent asserted together so the pair cannot drift: a reader
/// that answered `Ok(None)` for everything would pass the absent half alone.
#[tokio::test]
async fn a_detail_lookup_returns_the_family_and_a_missing_slug_returns_none() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxFamilyReader::new(pool.clone());

  insert_family(&pool, "cessna-172", "Cessna 172").await?;

  let found = reader.family(&slug("cessna-172")).await?.expect("the family exists");

  assert_eq!(found.summary.name, "Cessna 172");
  assert!(reader.family(&slug("no-such-family")).await?.is_none(), "an absent slug is Ok(None)");
  Ok(())
}

/// AC3, both halves. A populated family reads every value back and an all-null
/// one reads `None`: a mapper returning `None` for everything passes the second
/// half alone, and one inventing defaults passes the first alone.
#[tokio::test]
async fn every_nullable_column_survives_as_none_and_every_populated_one_as_its_value() -> TestResult
{
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxFamilyReader::new(pool.clone());

  // A manufacturer to point at. `organization_types` is seeded, so the code is
  // read from the database rather than guessed.
  let org_type: String =
    query_scalar("SELECT code FROM aircraft_ref.organization_types ORDER BY code LIMIT 1")
      .fetch_one(&pool)
      .await?;
  query("INSERT INTO aircraft_org.organizations (slug, name, org_type_code) VALUES ($1, $2, $3)")
    .bind("cessna")
    .bind("Cessna Aircraft Company")
    .bind(&org_type)
    .execute(&pool)
    .await?;
  query(
    "INSERT INTO aircraft_core.families
       (slug, name, common_name, name_aliases, manufacturer_org_id,
        country_of_origin_code, first_flight_year, description)
     VALUES ($1, $2, $3, $4, (SELECT id FROM aircraft_org.organizations WHERE slug = 'cessna'),
             $5, $6, $7)",
  )
  .bind("cessna-172")
  .bind("Cessna 172")
  .bind("Skyhawk")
  .bind(vec!["Skyhawk".to_owned(), "C172".to_owned()])
  .bind("USA")
  .bind(1955_i16)
  .bind("A four-seat single-engine aircraft.")
  .execute(&pool)
  .await?;
  insert_family(&pool, "sparse-family", "Sparse").await?;

  let populated = reader.family(&slug("cessna-172")).await?.expect("the populated family");
  let sparse = reader.family(&slug("sparse-family")).await?.expect("the sparse family");

  assert_eq!(populated.summary.common_name.as_deref(), Some("Skyhawk"));
  assert_eq!(populated.summary.manufacturer.as_ref().map(Slug::as_str), Some("cessna"));
  assert_eq!(populated.summary.country_of_origin.as_ref().map(CountryCode::as_str), Some("USA"));
  assert_eq!(populated.summary.first_flight_year, Some(1955));
  assert_eq!(populated.name_aliases, ["Skyhawk", "C172"]);
  assert_eq!(populated.description.as_deref(), Some("A four-seat single-engine aircraft."));

  assert_eq!(sparse.summary.common_name, None);
  assert_eq!(sparse.summary.manufacturer, None, "a family with no manufacturer is still returned");
  assert_eq!(sparse.summary.country_of_origin, None);
  assert_eq!(sparse.summary.first_flight_year, None);
  assert!(sparse.name_aliases.is_empty(), "a NULL array reads as an empty list");
  assert_eq!(sparse.description, None);
  Ok(())
}

/// AC3. `aircraft_geo.countries.code` is `VARCHAR(3)` with no case constraint,
/// while `CountryCode` requires three upper-case letters, so a legal stored row
/// can fail conversion. It must be a typed failure that names the column and not
/// the value -- never a panic, never a silently dropped row.
#[tokio::test]
async fn a_stored_country_code_the_domain_refuses_is_an_invariant_failure() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxFamilyReader::new(pool.clone());

  query("INSERT INTO aircraft_geo.countries (code, name) VALUES ('us1', 'Lower Case Land')")
    .execute(&pool)
    .await?;
  query(
    "INSERT INTO aircraft_core.families (slug, name, country_of_origin_code)
         VALUES ('odd-country', 'Odd', 'us1')",
  )
  .execute(&pool)
  .await?;

  match reader.family(&slug("odd-country")).await {
    Err(PersistenceError::Invariant(message)) => {
      assert!(
        message.contains("country_of_origin_code"),
        "the refusal names the column: {message}"
      );
      assert!(!message.contains("us1"), "the refusal must not echo the stored value: {message}");
    }
    other => panic!("a value the domain refuses must be an invariant failure, got {other:?}"),
  }
  Ok(())
}

/// The restricted runtime role reads families and can write none.
///
/// Every other test here connects as the container owner, which holds every
/// privilege and therefore cannot see a missing grant: the route would pass all
/// of them and still answer `503` in production, where
/// `database/roles/app_grants.sql` is what the server connects with. This is the
/// only test that can fail for `42501`.
#[tokio::test]
async fn the_runtime_role_reads_families_and_writes_none() -> TestResult {
  let (container, admin) = start_postgres(2, Duration::from_secs(30)).await?;
  install_schema(&admin).await?;
  insert_family(&admin, "cessna-172", "Cessna 172").await?;
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
  let runtime = connect(&url, 2, 2, 5).await?;
  let reader = SqlxFamilyReader::new(runtime.clone());

  let page = reader.list_families(&FamilyFilter::default(), limit(50), None).await?;
  assert_eq!(page.items().len(), 1, "the runtime role must read families");
  assert!(reader.family(&slug("cessna-172")).await?.is_some(), "and read one by slug");

  for statement in [
    "UPDATE aircraft_core.families SET name = 'x'",
    "INSERT INTO aircraft_core.families (slug, name) VALUES ('x', 'x')",
    "DELETE FROM aircraft_core.families",
  ] {
    let error = query(statement)
      .execute(&runtime)
      .await
      .expect_err("the runtime role must not write catalog data");
    assert_eq!(
      sqlstate(&error).as_deref(),
      Some("42501"),
      "{statement} must be refused for insufficient privilege: {error}"
    );
  }
  Ok(())
}

/// The check #37 deferred: every column the family projection reads exists in
/// the *installed* schema. A migration-file parser cannot see this -- migration
/// `004` is immutable, so a rename would arrive in a later migration -- which is
/// why it belongs here, against the database the adapter will really query.
#[tokio::test]
async fn every_column_the_family_statements_read_exists_in_the_installed_schema() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  for (schema, table, columns) in [
    (
      "aircraft_core",
      "families",
      &[
        "slug",
        "name",
        "common_name",
        "manufacturer_org_id",
        "country_of_origin_code",
        "first_flight_year",
        "name_aliases",
        "description",
      ][..],
    ),
    ("aircraft_org", "organizations", &["id", "slug"][..]),
  ] {
    for column in columns {
      let present: bool = query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns
          WHERE table_schema = $1 AND table_name = $2 AND column_name = $3)",
      )
      .bind(schema)
      .bind(table)
      .bind(column)
      .fetch_one(&pool)
      .await?;
      assert!(present, "{schema}.{table}.{column} is read by a statement but is not in the schema");
    }
  }
  Ok(())
}
