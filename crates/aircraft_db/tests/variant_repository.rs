//! What the variant adapter owes: nine filters that each work alone and compose
//! conjunctively, a reference code no vocabulary carries reported as the
//! caller's error rather than an empty page, a bounded page in a total order,
//! and a detail lookup that separates absence from failure.
//!
//! Against the canonical schema, as `crates/AGENTS.md` requires: `install_schema`
//! runs the migrations and seeds, so the reference vocabularies and countries are
//! the seeded ones and the codes below are read from them rather than invented.
//! `aircraft_core.{families,models,variants}` are seeded by nothing, so each test
//! inserts the rows it asserts on. The restricted-role gate for this reader lives
//! in `family_repository.rs`, which holds the shared catalog role setup.

// A failing assertion is the point of a test.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use aircraft_app::{
  catalog::{VariantFilter, VariantReadError, VariantReader as _},
  pagination::PageLimit,
};
use aircraft_db::SqlxVariantReader;
use aircraft_domain::catalog::{CountryCode, FamilyId, LookupCode, ModelId, Slug};
use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use sqlx_core::{query::query, query_scalar::query_scalar};
use sqlx_postgres::PgPool;

/// Seeded codes, read back from the vocabularies rather than restated, so a
/// seed change is a failure here and not a silently wrong fixture.
const VARIANT_TYPE: &str = "PRODUCTION_STANDARD";
const SERVICE_STATUS: &str = "IN_PRODUCTION";
const LANDING_GEAR: &str = "FIXED_TRICYCLE";
const PROPULSION: &str = "PISTON_RECIPROCATING";
const COUNTRY: &str = "USA";

/// One variant's filterable columns, as a test states them.
#[derive(Clone, Debug, Default)]
struct VariantRow {
  variant_type: Option<&'static str>,
  service_status: Option<&'static str>,
  landing_gear: Option<&'static str>,
  propulsion: Option<&'static str>,
  country: Option<&'static str>,
  in_production: Option<bool>,
  production_start: Option<i16>,
  production_end: Option<i16>,
}

async fn insert_family(pool: &PgPool, slug: &str) -> TestResult<FamilyId> {
  let id: i64 =
    query_scalar("INSERT INTO aircraft_core.families (slug, name) VALUES ($1, $1) RETURNING id")
      .bind(slug)
      .fetch_one(pool)
      .await?;
  Ok(FamilyId::new(id))
}

async fn insert_model(pool: &PgPool, family: FamilyId, slug: &str) -> TestResult<ModelId> {
  let id: i64 = query_scalar(
    "INSERT INTO aircraft_core.models (family_id, slug, name) VALUES ($1, $2, $2) RETURNING id",
  )
  .bind(family.get())
  .bind(slug)
  .fetch_one(pool)
  .await?;
  Ok(ModelId::new(id))
}

async fn insert_variant(pool: &PgPool, model: ModelId, slug: &str, row: &VariantRow) -> TestResult {
  query(
    "INSERT INTO aircraft_core.variants
       (model_id, slug, name, variant_type_code, service_status_code,
        landing_gear_type_code, propulsion_category_code, country_of_origin_code,
        is_in_production, production_start_year, production_end_year)
     VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
  )
  .bind(model.get())
  .bind(slug)
  .bind(row.variant_type)
  .bind(row.service_status)
  .bind(row.landing_gear)
  .bind(row.propulsion)
  .bind(row.country)
  .bind(row.in_production)
  .bind(row.production_start)
  .bind(row.production_end)
  .execute(pool)
  .await?;
  Ok(())
}

fn slug(value: &str) -> Slug {
  Slug::try_from(value).expect("a slug_text value")
}

fn code(value: &str) -> LookupCode {
  LookupCode::try_from(value).expect("a lookup_code value")
}

fn limit(value: u16) -> PageLimit {
  PageLimit::from_requested(std::num::NonZeroU16::new(value))
}

/// Inserts one variant per filter, each distinguished by exactly the column its
/// filter reads, plus one carrying two filters' values for the conjunctive case.
///
/// Extracted so the test above states assertions rather than setup; the rows are
/// what make each filter independently observable, so they are documented here
/// rather than inline.
async fn seed_one_row_per_filter(pool: &PgPool) -> TestResult<(FamilyId, ModelId, ModelId)> {
  let cessna = insert_family(pool, "cessna").await?;
  let piper = insert_family(pool, "piper").await?;
  let c172_id = insert_model(pool, cessna, "c-172").await?;
  let pa28_id = insert_model(pool, piper, "pa-28").await?;

  // One row per filter, each distinguished by exactly the column its filter
  // reads, plus one row carrying everything for the conjunctive case.
  insert_variant(
    pool,
    c172_id,
    "typed",
    &VariantRow { variant_type: Some(VARIANT_TYPE), ..VariantRow::default() },
  )
  .await?;
  insert_variant(
    pool,
    c172_id,
    "status",
    &VariantRow { service_status: Some(SERVICE_STATUS), ..VariantRow::default() },
  )
  .await?;
  insert_variant(
    pool,
    c172_id,
    "geared",
    &VariantRow { landing_gear: Some(LANDING_GEAR), ..VariantRow::default() },
  )
  .await?;
  insert_variant(
    pool,
    c172_id,
    "powered",
    &VariantRow { propulsion: Some(PROPULSION), ..VariantRow::default() },
  )
  .await?;
  insert_variant(
    pool,
    c172_id,
    "placed",
    &VariantRow { country: Some(COUNTRY), ..VariantRow::default() },
  )
  .await?;
  insert_variant(
    pool,
    c172_id,
    "building",
    &VariantRow { in_production: Some(true), ..VariantRow::default() },
  )
  .await?;
  insert_variant(
    pool,
    c172_id,
    "ran-1960",
    &VariantRow {
      production_start: Some(1955),
      production_end: Some(1965),
      ..VariantRow::default()
    },
  )
  .await?;
  // Still in production: a start year and no end. The filter's open-ended arm
  // exists for exactly this row, and without one here that arm is untested --
  // dropping it from the statement left the suite green until this was added.
  insert_variant(
    pool,
    c172_id,
    "still-building",
    &VariantRow { production_start: Some(2010), ..VariantRow::default() },
  )
  .await?;
  insert_variant(pool, pa28_id, "other-model", &VariantRow::default()).await?;
  insert_variant(
    pool,
    c172_id,
    "both",
    &VariantRow {
      variant_type: Some(VARIANT_TYPE),
      service_status: Some(SERVICE_STATUS),
      ..VariantRow::default()
    },
  )
  .await?;

  Ok((piper, c172_id, pa28_id))
}

/// AC1 and AC2 together, because they fail apart: each of the nine filters
/// selects its own row when used alone, and two of them together select only
/// the row satisfying both.
///
/// Every filter is asserted against a *different* row, so a binding swapped
/// between two filters cannot satisfy both halves -- the defect
/// `each_filter_admits_only_the_families_that_match_it` was written to catch in
/// the family adapter, where three wrong implementations passed the suite until
/// each filter had its own witness.
#[tokio::test]
async fn each_filter_selects_only_its_own_rows_and_two_filters_narrow_to_their_intersection()
-> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxVariantReader::new(pool.clone());

  let (piper, _c172, pa28) = seed_one_row_per_filter(&pool).await?;
  let cases: [(&str, VariantFilter, &[&str]); 10] = [
    ("model", VariantFilter { model: Some(pa28), ..VariantFilter::default() }, &["other-model"]),
    ("family", VariantFilter { family: Some(piper), ..VariantFilter::default() }, &["other-model"]),
    (
      "variant_type",
      VariantFilter { variant_type: Some(code(VARIANT_TYPE)), ..VariantFilter::default() },
      &["both", "typed"],
    ),
    (
      "service_status",
      VariantFilter { service_status: Some(code(SERVICE_STATUS)), ..VariantFilter::default() },
      &["both", "status"],
    ),
    (
      "landing_gear_type",
      VariantFilter { landing_gear_type: Some(code(LANDING_GEAR)), ..VariantFilter::default() },
      &["geared"],
    ),
    (
      "propulsion_category",
      VariantFilter { propulsion_category: Some(code(PROPULSION)), ..VariantFilter::default() },
      &["powered"],
    ),
    (
      "country_of_origin",
      VariantFilter {
        country_of_origin: Some(CountryCode::try_from(COUNTRY).expect("a country code")),
        ..VariantFilter::default()
      },
      &["placed"],
    ),
    (
      "is_in_production",
      VariantFilter { is_in_production: Some(true), ..VariantFilter::default() },
      &["building"],
    ),
    (
      "produced_in_year",
      VariantFilter { produced_in_year: Some(1960), ..VariantFilter::default() },
      &["ran-1960"],
    ),
    // The open-ended arm: a run that started and has not ended still covers a
    // later year. `VariantFilter::produced_in_year` records why excluding a
    // NULL end year would drop the aircraft a recent-year query most wants.
    (
      "produced_in_year with no end year",
      VariantFilter { produced_in_year: Some(2015), ..VariantFilter::default() },
      &["still-building"],
    ),
  ];

  for (name, filter, expected) in cases {
    let page = reader.list_variants(&filter, limit(50), None).await?;
    let got: Vec<&str> = page.items().iter().map(|row| row.slug.as_str()).collect();
    assert_eq!(got, expected, "the {name} filter must select only its own rows");
  }

  // AC2: the two filters that share rows, together, admit only the row with
  // both. Each alone returns two rows, so a filter dropped on either side
  // fails here.
  let conjunctive = VariantFilter {
    variant_type: Some(code(VARIANT_TYPE)),
    service_status: Some(code(SERVICE_STATUS)),
    ..VariantFilter::default()
  };
  let page = reader.list_variants(&conjunctive, limit(50), None).await?;
  let got: Vec<&str> = page.items().iter().map(|row| row.slug.as_str()).collect();
  assert_eq!(got, ["both"], "two filters narrow to their intersection, not their union");

  // A third filter that excludes it, so the conjunction is not merely the
  // intersection of the two that happen to overlap.
  let narrowed = VariantFilter { is_in_production: Some(true), ..conjunctive };
  let page = reader.list_variants(&narrowed, limit(50), None).await?;
  assert!(page.items().is_empty(), "a third filter the row fails excludes it");
  Ok(())
}

/// AC3. A reference code the vocabulary does not carry is the caller's error,
/// not an empty page, and the refusal names the filter so a client can fix the
/// request.
///
/// Each of the five code filters is driven, because one shared check could
/// otherwise pass for the first and silently skip the rest. The accepted value
/// is asserted alongside, so an implementation rejecting every code would fail
/// rather than satisfy the negatives.
#[tokio::test]
async fn a_reference_code_no_vocabulary_carries_is_refused_as_the_callers_error() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxVariantReader::new(pool.clone());

  let family = insert_family(&pool, "cessna").await?;
  let model = insert_model(&pool, family, "c-172").await?;
  insert_variant(
    &pool,
    model,
    "c-172s",
    &VariantRow {
      variant_type: Some(VARIANT_TYPE),
      service_status: Some(SERVICE_STATUS),
      landing_gear: Some(LANDING_GEAR),
      propulsion: Some(PROPULSION),
      country: Some(COUNTRY),
      ..VariantRow::default()
    },
  )
  .await?;

  // Well-formed for their types and absent from every vocabulary, so only an
  // existence check can tell them apart from a filter that matches nothing.
  let unknown = code("NO_SUCH_CODE");
  let rejected: [(&str, VariantFilter); 5] = [
    (
      "variant_type",
      VariantFilter { variant_type: Some(unknown.clone()), ..VariantFilter::default() },
    ),
    (
      "service_status",
      VariantFilter { service_status: Some(unknown.clone()), ..VariantFilter::default() },
    ),
    (
      "landing_gear_type",
      VariantFilter { landing_gear_type: Some(unknown.clone()), ..VariantFilter::default() },
    ),
    (
      "propulsion_category",
      VariantFilter { propulsion_category: Some(unknown), ..VariantFilter::default() },
    ),
    (
      "country_of_origin",
      VariantFilter {
        country_of_origin: Some(CountryCode::try_from("ZZZ").expect("a well-formed code")),
        ..VariantFilter::default()
      },
    ),
  ];

  for (name, filter) in rejected {
    match reader.list_variants(&filter, limit(50), None).await {
      Err(VariantReadError::UnknownFilterCode(filter_name)) => {
        assert_eq!(filter_name, name, "the refusal names the filter the caller sent");
      }
      other => panic!("{name} must be refused as an unknown code, got {other:?}"),
    }
  }

  // The seeded codes still work, so the check rejects unknown values and not
  // every value.
  let accepted = VariantFilter {
    variant_type: Some(code(VARIANT_TYPE)),
    country_of_origin: Some(CountryCode::try_from(COUNTRY).expect("a country code")),
    ..VariantFilter::default()
  };
  let page = reader.list_variants(&accepted, limit(50), None).await?;
  assert_eq!(page.items().len(), 1, "a seeded code is not rejected");
  Ok(())
}

/// AC4 and page stability. Variants sharing a `name` are still totally ordered
/// and visited exactly once across pages, which is what the accepted decision's
/// tiebreaker rule protects: duplicate sort values must neither skip nor repeat
/// a row.
///
/// Inserted out of slug order so a sequential scan without `ORDER BY` cannot
/// produce the asserted order by accident -- a fixture inserted in the asserted
/// order passed that mutation elsewhere in this repository.
#[tokio::test]
async fn variants_sharing_a_name_are_totally_ordered_and_paged_exactly_once() -> TestResult {
  const MAX_PAGES: usize = 32;

  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxVariantReader::new(pool.clone());

  let family = insert_family(&pool, "cessna").await?;
  let model = insert_model(&pool, family, "c-172").await?;
  // Every row carries the same `name`, so nothing but the slug and id can order
  // them.
  for name in ["c-172s", "c-172m", "c-172r", "c-172n", "c-172p"] {
    query("INSERT INTO aircraft_core.variants (model_id, slug, name) VALUES ($1, $2, 'Skyhawk')")
      .bind(model.get())
      .bind(name)
      .execute(&pool)
      .await?;
  }

  let mut visited: Vec<String> = Vec::new();
  let mut resume: Option<Slug> = None;
  let mut pages = 0;
  for _ in 0..MAX_PAGES {
    let page = reader.list_variants(&VariantFilter::default(), limit(2), resume.as_ref()).await?;
    pages += 1;
    visited.extend(page.items().iter().map(|row| row.slug.as_str().to_owned()));
    match page.next() {
      Some(next) => resume = Some(next.clone()),
      None => break,
    }
  }

  assert_eq!(
    visited,
    ["c-172m", "c-172n", "c-172p", "c-172r", "c-172s"],
    "every row exactly once, in slug order, despite one shared name"
  );
  assert_eq!(pages, 3, "five rows at two per page is three pages, so the cursor ran twice");
  Ok(())
}

/// An empty result is a successful empty page, and so is a filter that matches
/// nothing -- which is the answer `UnknownFilterCode` is deliberately *not*.
#[tokio::test]
async fn a_filter_that_matches_nothing_is_a_successful_empty_page() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxVariantReader::new(pool.clone());

  let empty = reader.list_variants(&VariantFilter::default(), limit(10), None).await?;
  assert!(empty.items().is_empty(), "an empty table is an empty page");
  assert!(empty.next().is_none());

  let family = insert_family(&pool, "cessna").await?;
  let model = insert_model(&pool, family, "c-172").await?;
  insert_variant(&pool, model, "c-172s", &VariantRow::default()).await?;

  // A seeded code that no stored row carries: valid filter, no matches.
  let unmatched =
    VariantFilter { service_status: Some(code(SERVICE_STATUS)), ..VariantFilter::default() };
  let page = reader.list_variants(&unmatched, limit(10), None).await?;
  assert!(page.items().is_empty(), "a known code matching no row is an empty page, not an error");
  assert!(page.next().is_none());
  Ok(())
}

/// Every projected value survives the round trip, and an all-null row reads as
/// `None` throughout.
///
/// Both halves, because a mapper returning `None` for everything passes the
/// second alone and one inventing defaults passes the first alone.
#[tokio::test]
async fn every_nullable_column_survives_as_none_and_every_populated_one_as_its_value() -> TestResult
{
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;
  let reader = SqlxVariantReader::new(pool.clone());

  let family = insert_family(&pool, "cessna").await?;
  let model = insert_model(&pool, family, "c-172").await?;
  query(
    "INSERT INTO aircraft_core.variants
       (model_id, slug, name, popular_name, variant_type_code, service_status_code,
        country_of_origin_code, first_flight_year, certification_year,
        production_start_year, production_end_year, passenger_capacity, crew_count,
        engine_count, landing_gear_type_code, propulsion_category_code,
        is_in_production, description)
     VALUES ($1, 'c-172s', '172S', 'Skyhawk SP', $2, $3, $4, 1955, 1956, 1998, 2005,
             3, 1, 1, $5, $6, TRUE, 'The common trainer')",
  )
  .bind(model.get())
  .bind(VARIANT_TYPE)
  .bind(SERVICE_STATUS)
  .bind(COUNTRY)
  .bind(LANDING_GEAR)
  .bind(PROPULSION)
  .execute(&pool)
  .await?;
  insert_variant(&pool, model, "c-172x", &VariantRow::default()).await?;

  let full = reader.variant(&slug("c-172s")).await?.expect("the populated variant");
  assert_eq!(full.summary.name, "172S");
  assert_eq!(full.summary.popular_name.as_deref(), Some("Skyhawk SP"));
  assert_eq!(full.summary.model.as_str(), "c-172");
  assert_eq!(full.summary.variant_type.as_ref().map(LookupCode::as_str), Some(VARIANT_TYPE));
  assert_eq!(full.summary.service_status.as_ref().map(LookupCode::as_str), Some(SERVICE_STATUS));
  assert_eq!(full.summary.country_of_origin.as_ref().map(CountryCode::as_str), Some(COUNTRY));
  assert_eq!(full.summary.first_flight_year, Some(1955));
  assert_eq!(full.summary.certification_year, Some(1956));
  assert_eq!(full.summary.production_start_year, Some(1998));
  assert_eq!(full.summary.production_end_year, Some(2005));
  assert_eq!(full.summary.passenger_capacity, Some(3));
  assert_eq!(full.summary.crew_count, Some(1));
  assert_eq!(full.summary.engine_count, Some(1));
  assert_eq!(full.summary.landing_gear_type.as_ref().map(LookupCode::as_str), Some(LANDING_GEAR));
  assert_eq!(full.summary.propulsion_category.as_ref().map(LookupCode::as_str), Some(PROPULSION));
  assert_eq!(full.summary.is_in_production, Some(true));
  assert_eq!(full.description.as_deref(), Some("The common trainer"));

  let sparse = reader.variant(&slug("c-172x")).await?.expect("the sparse variant");
  assert_eq!(sparse.summary.popular_name, None);
  assert_eq!(sparse.summary.variant_type, None);
  assert_eq!(sparse.summary.service_status, None);
  assert_eq!(sparse.summary.country_of_origin, None);
  assert_eq!(sparse.summary.first_flight_year, None);
  assert_eq!(sparse.summary.production_start_year, None);
  assert_eq!(sparse.summary.passenger_capacity, None);
  assert_eq!(sparse.summary.engine_count, None);
  assert_eq!(sparse.summary.landing_gear_type, None);
  assert_eq!(sparse.summary.is_in_production, None);
  assert_eq!(sparse.description, None);

  assert!(reader.variant(&slug("c-999")).await?.is_none(), "absence is not a failure");
  Ok(())
}

/// Every column of `aircraft_core.variants` is one a variant statement reads or
/// one the catalog contract deliberately withholds.
///
/// The direction and the reason are
/// `every_family_column_is_read_or_deliberately_withheld`'s: the column this
/// gate exists for is the one a later migration adds that nobody classifies.
/// `database/data_dictionary.md` records both halves and names this test.
#[tokio::test]
async fn every_variant_column_is_read_or_deliberately_withheld() -> TestResult {
  const READ: &[&str] = &[
    "model_id",
    "slug",
    "name",
    "popular_name",
    "variant_type_code",
    "service_status_code",
    "country_of_origin_code",
    "first_flight_year",
    "certification_year",
    "production_start_year",
    "production_end_year",
    "passenger_capacity",
    "crew_count",
    "engine_count",
    "landing_gear_type_code",
    "propulsion_category_code",
    "is_in_production",
    "description",
  ];
  // The surrogate key -- read only as the `ORDER BY` tiebreaker and never
  // published -- the generated search vector, the ingestion staging columns, the
  // open-ended attribute bag, and the row timestamps.
  const WITHHELD: &[&str] = &[
    "id",
    "description_tsv",
    "ingest_key",
    "source_path",
    "extra_attributes",
    "created_at",
    "updated_at",
  ];

  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let installed: Vec<String> = query_scalar(
    "SELECT column_name FROM information_schema.columns
      WHERE table_schema = 'aircraft_core' AND table_name = 'variants'
      ORDER BY column_name",
  )
  .fetch_all(&pool)
  .await?;

  let mut classified: Vec<String> =
    READ.iter().chain(WITHHELD).copied().map(str::to_owned).collect();
  classified.sort();

  assert_eq!(
    installed, classified,
    "every column of aircraft_core.variants must be read by a variant statement or listed as \
     deliberately withheld"
  );
  Ok(())
}
