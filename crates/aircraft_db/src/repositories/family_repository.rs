//! The family adapter behind `aircraft_app`'s catalog ports.
//!
//! Implements [`FamilyReader`] over the pool built by
//! [`connect`](crate::pool::connect), reading `aircraft_core.families` as
//! `database/migrations/004_aircraft_identity_taxonomy.sql` declares it. The
//! projections and the port are `aircraft_app::catalog`'s, which owns what this
//! publishes and what it withholds; this module owns only the statements and the
//! row conversion.
//!
//! The runtime role's access to `aircraft_core` and `aircraft_org` is the
//! column-level `SELECT` in `database/roles/app_grants.sql`, which names this
//! file in turn. A column added to a statement here without a grant there fails
//! with `42501`, and `the_runtime_role_reads_the_catalog_and_writes_none` in
//! `crates/aircraft_db/tests/family_repository.rs` is what catches it: every
//! other test connects as the container owner and cannot see a missing grant.

use aircraft_app::{
  catalog::{FamilyDetail, FamilyFilter, FamilyReader, FamilySummary},
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
};
use aircraft_domain::catalog::{CountryCode, Slug};
use async_trait::async_trait;
use sqlx_core::{query::query, row::Row};
use sqlx_postgres::{PgPool, PgRow};

use crate::repositories::ingestion_repository::database_error;

/// The published summary columns, and the manufacturer's public identifier.
///
/// `LEFT JOIN`, not an inner join: `manufacturer_org_id` is nullable, and an
/// inner join would silently drop every family that has no manufacturer rather
/// than returning it with `manufacturer` absent.
///
/// Ordered by `slug` alone and resumed by `slug > $3`. `slug` is
/// `NOT NULL UNIQUE` on this table (`004:35`), so the order is total and the
/// resume is exact; a name-ordered page would need the caller to carry the last
/// row's name, and `FamilyReader::list_families` resumes from a [`Slug`] and
/// nothing else. Changing that is a change to the port, not to this statement.
///
/// Both filters are `$n IS NULL OR column = $n` so one statement serves every
/// combination with every value bound, which is the shape
/// `curation_repository::PENDING_ASSERTIONS` already uses for its optional
/// filters. Nothing here is assembled from caller state.
const LIST: &str = "\
SELECT f.slug, f.name, f.common_name, o.slug AS manufacturer, \
       f.country_of_origin_code, f.first_flight_year \
  FROM aircraft_core.families f \
  LEFT JOIN aircraft_org.organizations o ON o.id = f.manufacturer_org_id \
 WHERE ($1::text IS NULL OR o.slug = $1) \
   AND ($2::text IS NULL OR f.country_of_origin_code = $2) \
   AND ($3::text IS NULL OR f.slug > $3) \
 ORDER BY f.slug \
 LIMIT $4";

/// The summary columns plus the two a detail adds.
const DETAIL: &str = "\
SELECT f.slug, f.name, f.common_name, o.slug AS manufacturer, \
       f.country_of_origin_code, f.first_flight_year, \
       f.name_aliases, f.description \
  FROM aircraft_core.families f \
  LEFT JOIN aircraft_org.organizations o ON o.id = f.manufacturer_org_id \
 WHERE f.slug = $1";

/// A stored value the catalog contract cannot represent.
///
/// Names the column and never the value: the row is operator data, and
/// `rust-production` keeps it out of a diagnostic that reaches a log. This is
/// reachable rather than theoretical -- `aircraft_geo.countries.code` is
/// `VARCHAR(3)` with no case constraint (`003:27`) while
/// [`CountryCode`] requires three upper-case ASCII letters -- so it is a typed
/// failure, not a panic and not a silently dropped row.
fn unrepresentable(column: &str) -> PersistenceError {
  PersistenceError::Invariant(format!(
    "{column} holds a value the catalog contract cannot represent"
  ))
}

/// The summary columns every family read projects.
///
/// `try_get` rather than indexing, for the reason
/// `reference_repository::SqlxCatalogReader::entries` gives: the indexing form
/// panics on a column whose name or type does not match, and a panic on a
/// request path is not an answer.
fn summary(row: &PgRow) -> Result<FamilySummary, PersistenceError> {
  let slug: String = row.try_get("slug").map_err(database_error)?;
  let manufacturer: Option<String> = row.try_get("manufacturer").map_err(database_error)?;
  let country: Option<String> = row.try_get("country_of_origin_code").map_err(database_error)?;

  Ok(FamilySummary {
    slug: Slug::try_from(slug.as_str())
      .map_err(|_| unrepresentable("aircraft_core.families.slug"))?,
    name: row.try_get("name").map_err(database_error)?,
    common_name: row.try_get("common_name").map_err(database_error)?,
    manufacturer: manufacturer
      .map(|value| Slug::try_from(value.as_str()))
      .transpose()
      .map_err(|_| unrepresentable("aircraft_org.organizations.slug"))?,
    country_of_origin: country
      .map(|value| CountryCode::try_from(value.as_str()))
      .transpose()
      .map_err(|_| unrepresentable("aircraft_core.families.country_of_origin_code"))?,
    first_flight_year: row.try_get("first_flight_year").map_err(database_error)?,
  })
}

/// Serves families from the application pool.
#[derive(Debug)]
pub struct SqlxFamilyReader {
  pool: PgPool,
}

impl SqlxFamilyReader {
  /// Shares the pool the server already holds, as
  /// [`SqlxCatalogReader::new`](crate::SqlxCatalogReader::new) does and for the
  /// same reason: the bounds an operator configured are the process's.
  #[must_use]
  pub const fn new(pool: PgPool) -> Self {
    Self { pool }
  }
}

#[async_trait]
impl FamilyReader for SqlxFamilyReader {
  /// # Errors
  ///
  /// [`PersistenceError::Database`] carrying the sanitized `SQLx` failure, and
  /// [`PersistenceError::Invariant`] when a stored value cannot become the
  /// domain type the contract publishes.
  async fn list_families(
    &self,
    filter: &FamilyFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<FamilySummary, Slug>, PersistenceError> {
    let rows = query(LIST)
      .bind(filter.manufacturer.as_ref().map(Slug::as_str))
      .bind(filter.country_of_origin.as_ref().map(CountryCode::as_str))
      .bind(after.map(Slug::as_str))
      // One row past the page, so `Page::from_overfetched` can tell a full page
      // from a final one without a second query.
      .bind(i64::from(limit.query_size()))
      .fetch_all(&self.pool)
      .await
      .map_err(database_error)?;

    let summaries =
      rows.iter().map(summary).collect::<Result<Vec<FamilySummary>, PersistenceError>>()?;

    Ok(Page::from_overfetched(summaries, limit, |row: &FamilySummary| row.slug.clone()))
  }

  /// # Errors
  ///
  /// As [`Self::list_families`]. A slug no family carries is `Ok(None)`:
  /// absence is not a failure, and turning it into `404` is the HTTP boundary's
  /// decision.
  async fn family(&self, slug: &Slug) -> Result<Option<FamilyDetail>, PersistenceError> {
    let Some(row) =
      query(DETAIL).bind(slug.as_str()).fetch_optional(&self.pool).await.map_err(database_error)?
    else {
      return Ok(None);
    };

    let aliases: Option<Vec<String>> = row.try_get("name_aliases").map_err(database_error)?;

    Ok(Some(FamilyDetail {
      summary: summary(&row)?,
      // A NULL array and an empty one say the same thing to a reader, which is
      // what `FamilyDetail::name_aliases` being a `Vec` rather than an `Option`
      // already promises.
      name_aliases: aliases.unwrap_or_default(),
      description: row.try_get("description").map_err(database_error)?,
    }))
  }
}
