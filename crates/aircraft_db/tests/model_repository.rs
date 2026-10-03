//! What the model adapter owes: a bounded, ordered page whose family filter
//! composes with its cursor, a detail lookup that separates absence from
//! failure, nullable years that survive as NULL, and a refusal -- not a
//! fabrication and not a silent omission -- when a model's family is gone.
//!
//! Against the canonical schema, as `crates/AGENTS.md` requires: `install_schema`
//! runs the migrations and seeds. `aircraft_core.models` and its parent
//! `aircraft_core.families` are populated by no seed, so each test inserts the
//! rows it asserts on. The restricted-role gate for this reader lives in
//! `family_repository.rs`, which already holds the shared catalog role setup.

// A failing assertion is the point of a test.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use aircraft_app::{
  catalog::{ModelFilter, ModelReader as _},
  ingestion::PersistenceError,
  pagination::PageLimit,
};
use aircraft_db::SqlxModelReader;
use aircraft_domain::catalog::{FamilyId, Slug};
use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use sqlx_core::{query::query, query_scalar::query_scalar};
use sqlx_postgres::PgPool;

/// Inserts one family and returns its id, which is what `ModelFilter::family`
/// carries: the port takes a resolved parent, not a slug.
async fn insert_family(pool: &PgPool, slug: &str, name: &str) -> TestResult<FamilyId> {
  let id: i64 =
    query_scalar("INSERT INTO aircraft_core.families (slug, name) VALUES ($1, $2) RETURNING id")
      .bind(slug)
      .bind(name)
      .fetch_one(pool)
      .await?;
  Ok(FamilyId::new(id))
}

/// Inserts one model with every optional column left NULL.
async fn insert_model(pool: &PgPool, family: FamilyId, slug: &str, name: &str) -> TestResult {
  query("INSERT INTO aircraft_core.models (family_id, slug, name) VALUES ($1, $2, $3)")
    .bind(family.get())
    .bind(slug)
    .bind(name)
    .execute(pool)
    .await?;
  Ok(())
}

fn slug(value: &str) -> Slug {
  Slug::try_from(value).expect("a slug_text value")
}

fn limit(value: u16) -> PageLimit {
  PageLimit::from_requested(std::num::NonZeroU16::new(value))
}

/// AC1. The family predicate and the cursor predicate are joined by `AND`, so
/// a filtered walk visits exactly the family's models, in slug order, once
/// each, and never a neighbour's.
///
/// The two families' slugs interleave on purpose: an unfiltered slug-ordered
/// page would mix them, so a page that contains only one family proves the
/// filter held on every page and not just the first. Five models under the
/// selected family with `limit(2)` forces three pages, so the cursor is
/// exercised twice.
#[tokio::test]
async fn each_family_filter_admits_only_its_models_across_pages() -> TestResult {
  const MAX_PAGES: usize = 32;

  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxModelReader::new(pool.clone());

  let cessna = insert_family(&pool, "cessna", "Cessna").await?;
  let piper = insert_family(&pool, "piper", "Piper").await?;
  let selected = ["c-150", "c-172", "c-182", "c-206", "c-210"];
  let neighbours = ["c-160", "c-175", "c-190"];
  // Inserted interleaved and out of slug order, so neither insertion order nor
  // an unfiltered scan could produce the asserted result by accident.
  insert_model(&pool, cessna, "c-210", "210").await?;
  insert_model(&pool, piper, "c-160", "PA-160").await?;
  insert_model(&pool, cessna, "c-150", "150").await?;
  insert_model(&pool, piper, "c-190", "PA-190").await?;
  insert_model(&pool, cessna, "c-182", "182").await?;
  insert_model(&pool, cessna, "c-172", "172").await?;
  insert_model(&pool, piper, "c-175", "PA-175").await?;
  insert_model(&pool, cessna, "c-206", "206").await?;

  let filter = ModelFilter { family: Some(cessna) };
  let mut visited: Vec<String> = Vec::new();
  let mut resume: Option<Slug> = None;
  let mut pages = 0;
  for _ in 0..MAX_PAGES {
    let page = reader.list_models(&filter, limit(2), resume.as_ref()).await?;
    pages += 1;
    for row in page.items() {
      assert_eq!(row.family.as_str(), "cessna", "a filtered page holds only that family: {row:?}");
      assert!(!neighbours.contains(&row.slug.as_str()), "a neighbour leaked: {row:?}");
    }
    visited.extend(page.items().iter().map(|row| row.slug.as_str().to_owned()));
    match page.next() {
      Some(next) => resume = Some(next.clone()),
      None => break,
    }
  }

  assert_eq!(visited, selected, "every selected model exactly once, in slug order");
  assert_eq!(pages, 3, "five rows at two per page is three pages, so the cursor ran twice");
  Ok(())
}

#[tokio::test]
async fn an_empty_table_is_a_successful_empty_page() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxModelReader::new(pool);

  let page = reader.list_models(&ModelFilter::default(), limit(10), None).await?;
  assert!(page.items().is_empty());
  assert!(page.next().is_none());
  Ok(())
}

/// AC1. The whole table, unfiltered, page by page: the visited slugs must equal
/// the stored order with no repeat and no gap. Inserted out of slug order so a
/// sequential scan without `ORDER BY` cannot return the asserted order by
/// accident -- a fixture inserted in the asserted order passed that mutation in
/// this repository.
#[tokio::test]
async fn paging_visits_every_model_exactly_once_in_slug_order() -> TestResult {
  const MAX_PAGES: usize = 32;

  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxModelReader::new(pool.clone());
  let family = insert_family(&pool, "cessna", "Cessna").await?;
  for name in ["c-206", "c-150", "c-210", "c-172", "c-182"] {
    insert_model(&pool, family, name, name).await?;
  }

  let mut visited: Vec<String> = Vec::new();
  let mut resume: Option<Slug> = None;
  for _ in 0..MAX_PAGES {
    let page = reader.list_models(&ModelFilter::default(), limit(2), resume.as_ref()).await?;
    visited.extend(page.items().iter().map(|row| row.slug.as_str().to_owned()));
    match page.next() {
      Some(next) => resume = Some(next.clone()),
      None => break,
    }
  }
  assert_eq!(visited, ["c-150", "c-172", "c-182", "c-206", "c-210"]);
  Ok(())
}

/// AC2. Present and absent asserted together so the pair cannot drift: a reader
/// that answered `Ok(None)` for everything would pass the absent half alone.
#[tokio::test]
async fn a_detail_lookup_returns_the_model_and_a_missing_slug_returns_none() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxModelReader::new(pool.clone());
  let family = insert_family(&pool, "cessna", "Cessna").await?;
  insert_model(&pool, family, "c-172", "172").await?;

  let detail = reader.model(&slug("c-172")).await?.expect("the stored model");
  assert_eq!(detail.summary.slug.as_str(), "c-172");
  assert_eq!(detail.summary.family.as_str(), "cessna");
  assert!(reader.model(&slug("c-999")).await?.is_none());
  Ok(())
}

/// AC3, both halves. A populated model reads every value back and an all-null
/// one reads `None`: a mapper returning `None` for everything passes the second
/// half alone, and one inventing defaults passes the first alone. The years are
/// the `first_flight_year` and `certification_year` columns of
/// `aircraft_core.models`, which is the reading of AC2's "production years"
/// the plan on #40 records: the model table has no production window.
#[tokio::test]
async fn every_nullable_column_survives_as_none_and_every_populated_one_as_its_value() -> TestResult
{
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxModelReader::new(pool.clone());
  let family = insert_family(&pool, "cessna", "Cessna").await?;
  query(
    "INSERT INTO aircraft_core.models
       (family_id, slug, name, display_name, name_aliases, series, generation,
        first_flight_year, certification_year, description)
     VALUES ($1, 'c-172', '172', 'Skyhawk', ARRAY['Skyhawk', 'C172'], 'Skyhawk', 3,
             1955, 1956, 'The one everyone learned in')",
  )
  .bind(family.get())
  .execute(&pool)
  .await?;
  insert_model(&pool, family, "c-x", "Unknown").await?;

  let full = reader.model(&slug("c-172")).await?.expect("the populated model");
  assert_eq!(full.summary.name, "172");
  assert_eq!(full.summary.display_name.as_deref(), Some("Skyhawk"));
  assert_eq!(full.summary.series.as_deref(), Some("Skyhawk"));
  assert_eq!(full.summary.generation, Some(3));
  assert_eq!(full.summary.first_flight_year, Some(1955));
  assert_eq!(full.summary.certification_year, Some(1956));
  assert_eq!(full.name_aliases, ["Skyhawk", "C172"]);
  assert_eq!(full.description.as_deref(), Some("The one everyone learned in"));

  let sparse = reader.model(&slug("c-x")).await?.expect("the sparse model");
  assert_eq!(sparse.summary.display_name, None);
  assert_eq!(sparse.summary.series, None);
  assert_eq!(sparse.summary.generation, None);
  assert_eq!(sparse.summary.first_flight_year, None);
  assert_eq!(sparse.summary.certification_year, None);
  assert!(sparse.name_aliases.is_empty(), "a NULL array reads as empty");
  assert_eq!(sparse.description, None);

  // The list projection carries the same years, not only the detail.
  let page = reader.list_models(&ModelFilter::default(), limit(10), None).await?;
  let years: Vec<(Option<i16>, Option<i16>)> =
    page.items().iter().map(|row| (row.first_flight_year, row.certification_year)).collect();
  assert_eq!(years, [(Some(1955), Some(1956)), (None, None)]);
  Ok(())
}

/// AC3. `ModelSummary::family` is a `Slug`, not an option, because
/// `family_id` is `NOT NULL` with `ON DELETE RESTRICT`. The only way a model can
/// lose its parent is past the constraint, so the constraint is dropped here to
/// reach that row. The read must then be a typed refusal naming the column --
/// never a fabricated parent, and never a row that is silently not there, which
/// an inner join would produce. Both statements are asserted, since each carries
/// its own `LEFT JOIN`.
#[tokio::test]
async fn a_model_whose_family_is_gone_is_an_invariant_failure_not_an_omission() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxModelReader::new(pool.clone());
  let family = insert_family(&pool, "cessna", "Cessna").await?;
  insert_model(&pool, family, "c-172", "172").await?;

  // Read the constraint's name rather than assuming PostgreSQL's auto-generated
  // one, as `authentication.rs` does for `chk_apc_secret_digest`'s siblings.
  let constraint: String = query_scalar(
    "SELECT conname FROM pg_constraint
      WHERE conrelid = 'aircraft_core.models'::regclass
        AND contype = 'f'
        AND confrelid = 'aircraft_core.families'::regclass",
  )
  .fetch_one(&pool)
  .await?;
  query(&format!("ALTER TABLE aircraft_core.models DROP CONSTRAINT {constraint}"))
    .execute(&pool)
    .await?;
  query("DELETE FROM aircraft_core.families WHERE slug = 'cessna'").execute(&pool).await?;

  for outcome in [
    reader.list_models(&ModelFilter::default(), limit(10), None).await.map(|_| ()),
    reader.model(&slug("c-172")).await.map(|_| ()),
  ] {
    match outcome {
      Err(PersistenceError::Invariant(message)) => {
        assert!(message.contains("family_id"), "the refusal names the column: {message}");
      }
      other => panic!("a model without a family must be an invariant failure, got {other:?}"),
    }
  }
  Ok(())
}

/// Every column of `aircraft_core.models` is one a model statement reads or one
/// the catalog contract deliberately withholds. The direction and the reason are
/// `every_family_column_is_read_or_deliberately_withheld`'s: the column this
/// gate exists for is the one a later migration adds that nobody classifies.
/// `database/data_dictionary.md` records both halves and names this test.
#[tokio::test]
async fn every_model_column_is_read_or_deliberately_withheld() -> TestResult {
  const READ: &[&str] = &[
    "family_id",
    "slug",
    "name",
    "display_name",
    "name_aliases",
    "series",
    "generation",
    "first_flight_year",
    "certification_year",
    "description",
  ];
  // The surrogate key, the open-ended attribute bag, and the row timestamps.
  const WITHHELD: &[&str] = &["id", "extra_attributes", "created_at", "updated_at"];

  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let installed: Vec<String> = query_scalar(
    "SELECT column_name FROM information_schema.columns
      WHERE table_schema = 'aircraft_core' AND table_name = 'models'
      ORDER BY column_name",
  )
  .fetch_all(&pool)
  .await?;

  let mut classified: Vec<String> =
    READ.iter().chain(WITHHELD).copied().map(str::to_owned).collect();
  classified.sort();

  assert_eq!(
    installed, classified,
    "every column of aircraft_core.models must be read by a model statement or listed as \
     deliberately withheld"
  );
  Ok(())
}
