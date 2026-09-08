//! What keyset pagination is worth: a scan that neither skips nor repeats a row
//! when the sort value is not unique, and one that stays stable when rows are
//! inserted between page requests.
//!
//! This is a **contract and reference test**, not proof of a production query —
//! no collection repository exists yet. It pins the two `PostgreSQL` semantics
//! `aircraft_app::pagination` depends on, and the row-comparison idiom the
//! endpoints in #36 onward will copy. Each of those still owes its own
//! repository test over its own table, filters, and row mapping.
//!
//! The fixture is canonical seeded data rather than an invented table, as
//! `crates/AGENTS.md` requires: `database/seeds/001_reference_units.sql` loads
//! 38 rows into `aircraft_ref.measurement_units` whose `sort_order` repeats
//! heavily — 14 rows share `10`, 13 share `20`, 6 share `30`. A predicate that
//! forgot the tiebreaker would lose a whole group of those.

// A failing assertion is the point of a test.
#![allow(clippy::expect_used, clippy::panic)]

use std::{num::NonZeroU16, time::Duration};

use aircraft_app::pagination::{Page, PageLimit};
use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use sqlx_core::{query::query, row::Row};
use sqlx_postgres::PgPool;

/// Complete statements rather than assembled fragments: the sort is fixed by
/// the endpoint, so there is nothing to build at runtime and every value is
/// bound. `sort_order` is `SMALLINT` in
/// `database/migrations/002_core_reference_tables.sql:47`, so it binds as `i16`
/// with no cast.
const FIRST_PAGE: &str = "SELECT sort_order, code FROM aircraft_ref.measurement_units
                          ORDER BY sort_order, code
                          LIMIT $1";

/// The row-value comparison is the whole idiom: `(sort_order, code)` is ordered
/// lexicographically as a tuple, so rows sharing a `sort_order` are separated
/// by `code` instead of being skipped or repeated.
const NEXT_PAGE: &str = "SELECT sort_order, code FROM aircraft_ref.measurement_units
                         WHERE (sort_order, code) > ($1, $2)
                         ORDER BY sort_order, code
                         LIMIT $3";

type Key = (i16, String);

/// Pages the whole table, returning every row in visit order.
///
/// Bounded by `MAX_PAGES` so a predicate bug that fails to advance ends the
/// test instead of hanging it.
async fn visit_all(pool: &PgPool, limit: PageLimit) -> TestResult<Vec<Key>> {
  const MAX_PAGES: usize = 64;

  let mut visited: Vec<Key> = Vec::new();
  let mut resume: Option<Key> = None;

  for _ in 0..MAX_PAGES {
    let rows = match &resume {
      None => query(FIRST_PAGE).bind(i64::from(limit.query_size())).fetch_all(pool).await?,
      Some((sort_order, code)) => {
        query(NEXT_PAGE)
          .bind(sort_order)
          .bind(code)
          .bind(i64::from(limit.query_size()))
          .fetch_all(pool)
          .await?
      }
    };
    let keys: Vec<Key> = rows.iter().map(|row| (row.get("sort_order"), row.get("code"))).collect();

    let page = Page::from_overfetched(keys, limit, Clone::clone);
    visited.extend(page.items().iter().cloned());
    match page.next() {
      Some(key) => resume = Some(key.clone()),
      None => return Ok(visited),
    }
  }
  panic!("paging did not terminate within {MAX_PAGES} pages");
}

/// Adds a unit row at an explicit sort position.
///
/// `canonical_unit_code` and `canonical_factor` are both left null, which is
/// what `chk_mu_canonical_pair` in migration `002` requires of a row that is
/// its own canonical unit.
async fn insert_unit(pool: &PgPool, code: &str, sort_order: i16) -> TestResult {
  query(
    "INSERT INTO aircraft_ref.measurement_units (code, label, unit_category_code, sort_order)
     VALUES ($1, $2, 'SPEED', $3)",
  )
  .bind(code)
  .bind(format!("{code} test unit"))
  .bind(sort_order)
  .execute(pool)
  .await?;
  Ok(())
}

#[tokio::test]
async fn paging_rows_with_duplicate_sort_values_visits_every_row_exactly_once() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  // Small enough that the 14 rows sharing `sort_order = 10` span several pages,
  // which is where a predicate without the tiebreaker loses them.
  let limit = PageLimit::from_requested(NonZeroU16::new(3));
  let visited = visit_all(&pool, limit).await?;

  let expected: Vec<Key> = query(
    "SELECT sort_order, code FROM aircraft_ref.measurement_units
                                  ORDER BY sort_order, code",
  )
  .fetch_all(&pool)
  .await?
  .iter()
  .map(|row| (row.get("sort_order"), row.get("code")))
  .collect();

  assert_eq!(visited, expected, "paging must reproduce the full ordering exactly once");

  let duplicated_group = expected.iter().filter(|(sort_order, _)| *sort_order == 10).count();
  assert!(
    duplicated_group > usize::from(limit.get()),
    "the fixture must span pages within one sort value, or this proves nothing: \
     {duplicated_group} rows at sort_order 10 against a limit of {}",
    limit.get()
  );
  Ok(())
}

#[tokio::test]
async fn a_row_inserted_behind_the_cursor_is_not_revisited_and_one_ahead_is_seen() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let limit = PageLimit::from_requested(NonZeroU16::new(3));
  let rows = query(FIRST_PAGE).bind(i64::from(limit.query_size())).fetch_all(&pool).await?;
  let keys: Vec<Key> = rows.iter().map(|row| (row.get("sort_order"), row.get("code"))).collect();
  let first = Page::from_overfetched(keys, limit, Clone::clone);
  let (sort_order, code) = first.next().expect("the seeded table has more than one page").clone();

  // Committed between the two page requests. No threads and no sleeps: the
  // guarantee keyset pagination gives is positional, so it is provable without
  // a race, and `crates/AGENTS.md` forbids synchronizing a test by sleeping.
  // The seeded minimum sort order is 10 and the maximum is 40.
  insert_unit(&pool, "ZZ_BEHIND", 0).await?;
  insert_unit(&pool, "ZZ_AHEAD", 99).await?;

  let remainder = query(NEXT_PAGE)
    .bind(sort_order)
    .bind(&code)
    .bind(i64::from(u32::from(u16::MAX)))
    .fetch_all(&pool)
    .await?;
  let seen: Vec<String> = remainder.iter().map(|row| row.get("code")).collect();

  assert!(seen.contains(&"ZZ_AHEAD".to_owned()), "a row inserted ahead of the cursor is served");
  assert!(
    !seen.contains(&"ZZ_BEHIND".to_owned()),
    "a row inserted behind the cursor is not revisited"
  );
  assert!(!seen.contains(&code), "the row the cursor names is not repeated: {code}");
  Ok(())
}
