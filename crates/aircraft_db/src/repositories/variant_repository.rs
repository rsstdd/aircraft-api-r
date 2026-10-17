//! The variant adapter behind `aircraft_app`'s catalog ports.
//!
//! Implements [`VariantReader`] over the pool built by
//! [`connect`](crate::pool::connect), reading `aircraft_core.variants` as
//! `database/migrations/004_aircraft_identity_taxonomy.sql` declares it. As with
//! [`family_repository`](crate::repositories::family_repository) and
//! [`model_repository`](crate::repositories::model_repository), the projections
//! and the port are `aircraft_app::catalog`'s; this module owns the statements
//! and the row conversion.
//!
//! The runtime role's access to `aircraft_core.variants` and the reference
//! vocabularies is the column-level `SELECT` in `database/roles/app_grants.sql`,
//! which names this file in turn. A column added to a statement here without a
//! grant there fails with `42501`, and
//! `the_runtime_role_reads_the_catalog_and_writes_none` in
//! `crates/aircraft_db/tests/family_repository.rs` is what catches it.

use aircraft_app::{
  catalog::{VariantDetail, VariantFilter, VariantReadError, VariantReader, VariantSummary},
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
};
use aircraft_domain::catalog::{CountryCode, FamilyId, LookupCode, ModelId, Slug};
use async_trait::async_trait;
use sqlx_core::{query::query, row::Row};
use sqlx_postgres::{PgPool, PgRow};

use crate::repositories::ingestion_repository::database_error;

/// The published summary columns, and the parent model's public identifier.
///
/// `JOIN` and not `LEFT JOIN`, unlike the model adapter's join to families:
/// `VariantSummary::model` is a non-optional [`Slug`], `model_id` is `NOT NULL`
/// with `ON DELETE RESTRICT` (`004:92-93`), and the family filter reads
/// `m.family_id`, so the join is load-bearing for the filter rather than only
/// for the published slug. A variant whose model were somehow gone could not
/// satisfy a family filter under any join, so there is no row an outer join
/// would usefully surface.
///
/// Ordered by `slug` then `id`. `slug` alone is already total -- it is
/// `NOT NULL UNIQUE` (`004:94`) -- so `id` adds no rows and changes no order.
/// It is named because `docs/architecture/http_v1_decisions.md` § "Pagination"
/// requires every order to carry a unique identifier as its final tiebreaker,
/// and because `database/roles/app_grants.sql` grants `id` for exactly this
/// `ORDER BY` and says so. Resuming on `slug` alone is exact for the same
/// uniqueness.
///
/// Every filter is `$n IS NULL OR column = $n`, so one statement serves all 512
/// combinations with every value bound and nothing assembled from caller state.
/// `produced_in_year` is the one that is not an equality: a run covers the year
/// when it started no later and either has not ended or ended no earlier, and
/// the open-ended arm is deliberate -- `VariantFilter::produced_in_year` records
/// why a NULL end year is still in production rather than excluded.
const LIST: &str = "\
SELECT v.slug, v.name, v.popular_name, m.slug AS model, \
       v.variant_type_code, v.service_status_code, v.country_of_origin_code, \
       v.first_flight_year, v.certification_year, \
       v.production_start_year, v.production_end_year, \
       v.passenger_capacity, v.crew_count, v.engine_count, \
       v.landing_gear_type_code, v.propulsion_category_code, v.is_in_production \
  FROM aircraft_core.variants v \
  JOIN aircraft_core.models m ON m.id = v.model_id \
 WHERE ($1::bigint IS NULL OR v.model_id = $1) \
   AND ($2::bigint IS NULL OR m.family_id = $2) \
   AND ($3::text IS NULL OR v.variant_type_code = $3) \
   AND ($4::text IS NULL OR v.service_status_code = $4) \
   AND ($5::text IS NULL OR v.landing_gear_type_code = $5) \
   AND ($6::text IS NULL OR v.propulsion_category_code = $6) \
   AND ($7::text IS NULL OR v.country_of_origin_code = $7) \
   AND ($8::boolean IS NULL OR v.is_in_production = $8) \
   AND ($9::smallint IS NULL OR (v.production_start_year <= $9 \
        AND (v.production_end_year IS NULL OR v.production_end_year >= $9))) \
   AND ($10::text IS NULL OR v.slug > $10) \
 ORDER BY v.slug, v.id \
 LIMIT $11";

/// The summary columns plus the one a detail adds.
const DETAIL: &str = "\
SELECT v.slug, v.name, v.popular_name, m.slug AS model, \
       v.variant_type_code, v.service_status_code, v.country_of_origin_code, \
       v.first_flight_year, v.certification_year, \
       v.production_start_year, v.production_end_year, \
       v.passenger_capacity, v.crew_count, v.engine_count, \
       v.landing_gear_type_code, v.propulsion_category_code, v.is_in_production, \
       v.description \
  FROM aircraft_core.variants v \
  JOIN aircraft_core.models m ON m.id = v.model_id \
 WHERE v.slug = $1";

/// Whether each supplied reference code names a row in its own vocabulary.
///
/// One statement rather than one per filter, and asked *before* the page rather
/// than folded into it: a code no vocabulary carries must be the caller's error
/// and not an empty page, which is what `VariantReadError::UnknownFilterCode`
/// exists for and what issue #42 requires. Folding the check into `LIST` could
/// only ever produce zero rows, which is the answer this distinguishes itself
/// from.
///
/// Each column is `$n IS NULL OR EXISTS(...)`, so an absent filter is vacuously
/// present and only a supplied one can be rejected. The vocabularies are the
/// tables `aircraft_domain::reference::Catalog` publishes and
/// `database/seeds/002_lookup_seed_data.sql` fills.
const FILTER_CODES_EXIST: &str = "\
SELECT ($1::text IS NULL OR EXISTS ( \
         SELECT 1 FROM aircraft_ref.variant_types WHERE code = $1)) AS variant_type, \
       ($2::text IS NULL OR EXISTS ( \
         SELECT 1 FROM aircraft_ref.service_statuses WHERE code = $2)) AS service_status, \
       ($3::text IS NULL OR EXISTS ( \
         SELECT 1 FROM aircraft_ref.landing_gear_types WHERE code = $3)) AS landing_gear_type, \
       ($4::text IS NULL OR EXISTS ( \
         SELECT 1 FROM aircraft_ref.propulsion_categories WHERE code = $4)) \
         AS propulsion_category, \
       ($5::text IS NULL OR EXISTS ( \
         SELECT 1 FROM aircraft_geo.countries WHERE code = $5)) AS country_of_origin";

/// The filter names this adapter can reject, spelled as the wire spells them.
///
/// Each name is also the column alias [`FILTER_CODES_EXIST`] reads back, so the
/// failing column maps to the name a caller sent. Nothing in the type system
/// ties the two together: a column added to that statement without a name here
/// would compile and leave that filter silently unchecked, which is what
/// `the_existence_statement_aliases_every_filter_this_adapter_can_reject`
/// exists to catch.
const CODE_FILTERS: [&str; 5] = [
  "variant_type",
  "service_status",
  "landing_gear_type",
  "propulsion_category",
  "country_of_origin",
];

/// A stored value the catalog contract cannot represent.
///
/// Names the column and never the value, as the family and model adapters do:
/// the row is operator data and stays out of a diagnostic that reaches a log.
fn unrepresentable(column: &str) -> PersistenceError {
  PersistenceError::Invariant(format!(
    "{column} holds a value the catalog contract cannot represent"
  ))
}

/// Reads one optional lookup code, refusing a stored value the domain rejects.
fn lookup(row: &PgRow, column: &str) -> Result<Option<LookupCode>, PersistenceError> {
  let raw: Option<String> = row.try_get(column).map_err(database_error)?;
  raw
    .map(|value| LookupCode::try_from(value.as_str()))
    .transpose()
    .map_err(|_| unrepresentable(&format!("aircraft_core.variants.{column}")))
}

/// The summary columns every variant read projects.
///
/// `try_get` rather than indexing, for the reason
/// `reference_repository::SqlxCatalogReader::entries` gives: the indexing form
/// panics on a column whose name or type does not match.
fn summary(row: &PgRow) -> Result<VariantSummary, PersistenceError> {
  let slug: String = row.try_get("slug").map_err(database_error)?;
  let model: String = row.try_get("model").map_err(database_error)?;
  let country: Option<String> = row.try_get("country_of_origin_code").map_err(database_error)?;

  Ok(VariantSummary {
    slug: Slug::try_from(slug.as_str())
      .map_err(|_| unrepresentable("aircraft_core.variants.slug"))?,
    name: row.try_get("name").map_err(database_error)?,
    popular_name: row.try_get("popular_name").map_err(database_error)?,
    model: Slug::try_from(model.as_str())
      .map_err(|_| unrepresentable("aircraft_core.models.slug"))?,
    variant_type: lookup(row, "variant_type_code")?,
    service_status: lookup(row, "service_status_code")?,
    country_of_origin: country
      .map(|value| CountryCode::try_from(value.as_str()))
      .transpose()
      .map_err(|_| unrepresentable("aircraft_core.variants.country_of_origin_code"))?,
    first_flight_year: row.try_get("first_flight_year").map_err(database_error)?,
    certification_year: row.try_get("certification_year").map_err(database_error)?,
    production_start_year: row.try_get("production_start_year").map_err(database_error)?,
    production_end_year: row.try_get("production_end_year").map_err(database_error)?,
    passenger_capacity: row.try_get("passenger_capacity").map_err(database_error)?,
    crew_count: row.try_get("crew_count").map_err(database_error)?,
    engine_count: row.try_get("engine_count").map_err(database_error)?,
    landing_gear_type: lookup(row, "landing_gear_type_code")?,
    propulsion_category: lookup(row, "propulsion_category_code")?,
    is_in_production: row.try_get("is_in_production").map_err(database_error)?,
  })
}

/// Serves variants from the application pool.
#[derive(Debug)]
pub struct SqlxVariantReader {
  pool: PgPool,
}

impl SqlxVariantReader {
  /// Shares the pool the server already holds, as the sibling catalog readers
  /// do and for the same reason: the bounds an operator configured are the
  /// process's.
  #[must_use]
  pub const fn new(pool: PgPool) -> Self {
    Self { pool }
  }

  /// Refuses a reference-code filter that names no row in its vocabulary.
  ///
  /// # Errors
  ///
  /// [`VariantReadError::UnknownFilterCode`] naming the first filter that fails,
  /// in [`CODE_FILTERS`] order so the answer is deterministic for a request
  /// carrying two bad codes.
  async fn reject_unknown_codes(&self, filter: &VariantFilter) -> Result<(), VariantReadError> {
    let row = query(FILTER_CODES_EXIST)
      .bind(filter.variant_type.as_ref().map(LookupCode::as_str))
      .bind(filter.service_status.as_ref().map(LookupCode::as_str))
      .bind(filter.landing_gear_type.as_ref().map(LookupCode::as_str))
      .bind(filter.propulsion_category.as_ref().map(LookupCode::as_str))
      .bind(filter.country_of_origin.as_ref().map(CountryCode::as_str))
      .fetch_one(&self.pool)
      .await
      .map_err(database_error)?;

    for name in CODE_FILTERS {
      let present: bool = row.try_get(name).map_err(database_error)?;
      if !present {
        return Err(VariantReadError::UnknownFilterCode(name));
      }
    }
    Ok(())
  }
}

#[async_trait]
impl VariantReader for SqlxVariantReader {
  /// # Errors
  ///
  /// [`VariantReadError::UnknownFilterCode`] when a reference-code filter names
  /// no row in its vocabulary, and [`VariantReadError::Persistence`] carrying
  /// the sanitized `SQLx` failure or an invariant a stored value broke.
  async fn list_variants(
    &self,
    filter: &VariantFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<VariantSummary, Slug>, VariantReadError> {
    // Before the page, so a request naming a code no vocabulary carries is the
    // caller's error rather than an empty collection.
    self.reject_unknown_codes(filter).await?;

    let rows = query(LIST)
      .bind(filter.model.map(ModelId::get))
      .bind(filter.family.map(FamilyId::get))
      .bind(filter.variant_type.as_ref().map(LookupCode::as_str))
      .bind(filter.service_status.as_ref().map(LookupCode::as_str))
      .bind(filter.landing_gear_type.as_ref().map(LookupCode::as_str))
      .bind(filter.propulsion_category.as_ref().map(LookupCode::as_str))
      .bind(filter.country_of_origin.as_ref().map(CountryCode::as_str))
      .bind(filter.is_in_production)
      .bind(filter.produced_in_year)
      .bind(after.map(Slug::as_str))
      // One row past the page, so `Page::from_overfetched` can tell a full page
      // from a final one without a second query.
      .bind(i64::from(limit.query_size()))
      .fetch_all(&self.pool)
      .await
      .map_err(database_error)?;

    let summaries =
      rows.iter().map(summary).collect::<Result<Vec<VariantSummary>, PersistenceError>>()?;

    Ok(Page::from_overfetched(summaries, limit, |row: &VariantSummary| row.slug.clone()))
  }

  /// # Errors
  ///
  /// [`PersistenceError::Database`] carrying the sanitized `SQLx` failure, and
  /// [`PersistenceError::Invariant`] when a stored value cannot become the
  /// domain type the contract publishes. A slug no variant carries is
  /// `Ok(None)`: absence is not a failure, and turning it into `404` is the HTTP
  /// boundary's decision.
  ///
  /// No filters apply here, so this keeps [`PersistenceError`] rather than the
  /// collection's wider error.
  async fn variant(&self, slug: &Slug) -> Result<Option<VariantDetail>, PersistenceError> {
    let Some(row) =
      query(DETAIL).bind(slug.as_str()).fetch_optional(&self.pool).await.map_err(database_error)?
    else {
      return Ok(None);
    };

    Ok(Some(VariantDetail {
      summary: summary(&row)?,
      description: row.try_get("description").map_err(database_error)?,
    }))
  }
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use super::{CODE_FILTERS, FILTER_CODES_EXIST};

  /// The only thing binding [`CODE_FILTERS`] to [`FILTER_CODES_EXIST`].
  ///
  /// `reject_unknown_codes` reads each name as a column alias, so a filter
  /// whose column the statement does not alias fails at run time with a column
  /// lookup error, and -- worse -- a column the statement aliases but this array
  /// omits is never inspected, leaving that filter unchecked with no symptom at
  /// all. Comparing the two as sets in both directions is what makes the array's
  /// doc comment true.
  #[test]
  fn the_existence_statement_aliases_every_filter_this_adapter_can_reject() {
    let mut aliased: Vec<&str> = FILTER_CODES_EXIST
      .split(" AS ")
      .skip(1)
      .map(|tail| tail.split([',', ' ']).next().expect("an alias follows AS").trim())
      .collect();
    aliased.sort_unstable();
    let mut declared: Vec<&str> = CODE_FILTERS.to_vec();
    declared.sort_unstable();

    assert_eq!(
      aliased, declared,
      "every alias in FILTER_CODES_EXIST must be a name in CODE_FILTERS and the reverse; \
       otherwise a reference-code filter is checked by nobody"
    );
    assert_eq!(declared.len(), 5, "the scan is not vacuous");
  }
}
