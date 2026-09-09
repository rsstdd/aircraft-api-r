//! The reference-catalog adapter behind `aircraft_api`'s `/v1/reference` route.
//!
//! Implements [`CatalogReader`] over the pool built by
//! [`connect`](crate::pool::connect). The port is declared in `aircraft_app`
//! because `cargo run -p xtask -- boundaries` refuses `aircraft_db` and `SQLx`
//! inside `aircraft_api`.

use aircraft_app::{
  ingestion::PersistenceError,
  reference::{CatalogEntry, CatalogReader, MAX_CATALOG_ROWS},
};
use aircraft_domain::reference::Catalog;
use async_trait::async_trait;
use sqlx_core::{error::Error as SqlxError, query::query, row::Row};
use sqlx_postgres::PgPool;

use crate::repositories::ingestion_repository::database_error;

/// The complete statement for one catalog.
///
/// One literal per catalog rather than a table name interpolated into a shared
/// template. The identifier is compile-time either way, so this is not about
/// injection; it is that the catalogs do not share a shape, and a statement
/// written out per catalog absorbs that with no branching. Four of the
/// thirty-six differ, and each difference is a column
/// `database/migrations/002_core_reference_tables.sql` does not declare:
///
/// - `unit_categories` has no `is_active`, so it has no `WHERE`.
/// - `measurement_units` and `certification_authorities` have no `description`;
///   they carry `symbol` and `full_name` instead, which this contract does not
///   publish, so the projection supplies a typed `NULL`.
/// - `currencies` has neither, and has no `sort_order` either, so it orders by
///   `code` alone.
///
/// Every order ends in `code`, the primary key of all thirty-six tables, so each
/// is total and the row order is deterministic -- which is what
/// `docs/architecture/http_v1_decisions.md` means by a stable sort, and what
/// makes an `ETag` over the serialized page reproducible.
///
/// Migration `002` is applied and hashed in `database/migrations.lock.json`, and
/// no later migration alters an `aircraft_ref` table, so these shapes cannot
/// drift underneath the statements. `every_catalog_serves_the_rows_seeded_for_its_own_table`
/// in `crates/aircraft_db/tests/reference_catalogs.rs` runs all thirty-six
/// against the canonical schema anyway, because a statement copied to the wrong
/// table would pass every other test.
///
/// `too_many_lines` is allowed here, narrowly and for this item only. The lint
/// measures complexity, and a flat exhaustive lookup table has none: there is
/// one arm per catalog, no branch inside any of them, and nothing to follow. The
/// length is the thirty-six catalogs. Compressing an arm to one line is not
/// available either -- the longest statement is past the hundred-column limit on
/// its own -- and the alternatives all cost more than they save: interpolating a
/// table name into a shared template still needs a thirty-six-arm match to
/// choose the shape, unless it gains a `_` arm, and a `_` arm is exactly what
/// stops a thirty-seventh catalog from being a compile error here.
#[allow(clippy::too_many_lines)]
const fn statement(catalog: Catalog) -> &'static str {
  match catalog {
    Catalog::UnitCategories => {
      "SELECT code, label, description \
         FROM aircraft_ref.unit_categories \
         ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::MeasurementUnits => {
      "SELECT code, label, NULL::text AS description \
         FROM aircraft_ref.measurement_units \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::AircraftRoles => {
      "SELECT code, label, description \
         FROM aircraft_ref.aircraft_roles \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::ServiceStatuses => {
      "SELECT code, label, description \
         FROM aircraft_ref.service_statuses \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::VariantTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.variant_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::LandingGearTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.landing_gear_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::PropulsionCategories => {
      "SELECT code, label, description \
         FROM aircraft_ref.propulsion_categories \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::FuelTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.fuel_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::DimensionMetricTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.dimension_metric_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::WeightMetricTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.weight_metric_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::PerformanceMetricTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.performance_metric_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::CertificationAuthorities => {
      "SELECT code, label, NULL::text AS description \
         FROM aircraft_ref.certification_authorities \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::AirworthinessCategories => {
      "SELECT code, label, description \
         FROM aircraft_ref.airworthiness_categories \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::PilotCertificateTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.pilot_certificate_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::OperatingApprovalTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.operating_approval_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::MilitaryMissionTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.military_mission_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::WeaponCategories => {
      "SELECT code, label, description \
         FROM aircraft_ref.weapon_categories \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::HardpointPositionTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.hardpoint_position_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::StoresTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.stores_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::Currencies => {
      "SELECT code, label, NULL::text AS description \
         FROM aircraft_ref.currencies \
         WHERE is_active ORDER BY code LIMIT $1"
    }
    Catalog::CostItemTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.cost_item_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::AircraftConditionGrades => {
      "SELECT code, label, description \
         FROM aircraft_ref.aircraft_condition_grades \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::AdTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.ad_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::SbComplianceStatuses => {
      "SELECT code, label, description \
         FROM aircraft_ref.sb_compliance_statuses \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::AvailabilityGrades => {
      "SELECT code, label, description \
         FROM aircraft_ref.availability_grades \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::SourceTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.source_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::SourceReliabilityGrades => {
      "SELECT code, label, description \
         FROM aircraft_ref.source_reliability_grades \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::CurationFlagStatuses => {
      "SELECT code, label, description \
         FROM aircraft_ref.curation_flag_statuses \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::CurationEntityTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.curation_entity_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::AssertionStatuses => {
      "SELECT code, label, description \
         FROM aircraft_ref.assertion_statuses \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::MissionProfileTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.mission_profile_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::ComparisonCriterionTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.comparison_criterion_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::OrganizationTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.organization_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::OrgRelationshipTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.org_relationship_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::SystemsCategories => {
      "SELECT code, label, description \
         FROM aircraft_ref.systems_categories \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
    Catalog::EquipmentProvisionTypes => {
      "SELECT code, label, description \
         FROM aircraft_ref.equipment_provision_types \
         WHERE is_active ORDER BY sort_order, code LIMIT $1"
    }
  }
}

/// Serves reference catalogs from the application pool.
#[derive(Debug)]
pub struct SqlxCatalogReader {
  pool: PgPool,
}

impl SqlxCatalogReader {
  /// Shares the pool the server already holds, for the reason
  /// [`PoolReadiness::new`](crate::readiness::PoolReadiness::new) gives: the
  /// bounds an operator configured are the process's.
  #[must_use]
  pub const fn new(pool: PgPool) -> Self {
    Self { pool }
  }
}

#[async_trait]
impl CatalogReader for SqlxCatalogReader {
  /// # Errors
  ///
  /// [`PersistenceError::Database`] carrying the sanitized `SQLx` failure, and
  /// [`PersistenceError::Invariant`] when the catalog exceeds
  /// [`MAX_CATALOG_ROWS`].
  async fn entries(&self, catalog: Catalog) -> Result<Vec<CatalogEntry>, PersistenceError> {
    // One row past the ceiling, so a catalog that has outgrown it is detected
    // rather than served short: `LIMIT MAX_CATALOG_ROWS` alone would return a
    // truncated catalog that reads exactly like a complete one. Same lookahead
    // convention as `PageLimit::query_size`, used here to prove completeness
    // instead of to page.
    let ceiling = i64::try_from(MAX_CATALOG_ROWS.saturating_add(1))
      .map_err(|_| PersistenceError::Invariant("the catalog ceiling exceeds i64".to_owned()))?;

    let rows = query(statement(catalog))
      .bind(ceiling)
      .fetch_all(&self.pool)
      .await
      .map_err(database_error)?;

    if rows.len() > MAX_CATALOG_ROWS {
      // The table name, not the row contents: this is an operational fact about
      // the schema, and no caller-supplied or source-derived text is in it.
      return Err(PersistenceError::Invariant(format!(
        "reference catalog {} holds more than {MAX_CATALOG_ROWS} rows",
        catalog.slug()
      )));
    }

    // `try_get` rather than `get`: the indexing form panics on a column whose
    // name or type does not match, and a panic on a request path is not an
    // answer. The failure class is the same one the query itself would raise,
    // so it goes through `database_error` and is sanitized with the rest.
    // `curation_repository` reads its rows the same way.
    rows
      .iter()
      .map(|row| {
        Ok(CatalogEntry {
          code: row.try_get("code")?,
          label: row.try_get("label")?,
          description: row.try_get("description")?,
        })
      })
      .collect::<Result<Vec<_>, SqlxError>>()
      .map_err(database_error)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The canonical schema, read rather than restated. Migration `002` is applied
  /// and hashed in `database/migrations.lock.json`, so `cargo xtask migrations`
  /// rejects an edit to it: this expectation cannot be bent to agree with a
  /// wrong statement. Compile-time and under `cfg(test)`, so nothing here
  /// reaches a runtime path -- the device `aircraft_testsupport` uses for the
  /// install order.
  const MIGRATION_002: &str =
    include_str!("../../../../database/migrations/002_core_reference_tables.sql");
  const SEED_UNITS: &str = include_str!("../../../../database/seeds/001_reference_units.sql");
  const SEED_LOOKUPS: &str = include_str!("../../../../database/seeds/002_lookup_seed_data.sql");

  /// The `aircraft_ref` tables `statement` names, in the order they appear.
  ///
  /// Anchored at the start of a line so a table named inside a `COMMENT ON`
  /// string cannot be counted, and stopping at the first character a table name
  /// cannot contain so that `INSERT INTO aircraft_ref.aircraft_roles (code, ...)`
  /// and a bare `CREATE TABLE aircraft_ref.currencies` both yield the name
  /// alone. Callers assert the length, so a pattern that stopped matching fails
  /// loudly instead of passing on an empty list.
  fn tables_named_by(keyword: &str, sql: &str) -> Vec<String> {
    let prefix = format!("{keyword} aircraft_ref.");
    sql
      .lines()
      .filter_map(|line| line.strip_prefix(&prefix))
      .map(|rest| {
        rest.chars().take_while(|c| c.is_ascii_lowercase() || *c == '_').collect::<String>()
      })
      .collect()
  }

  /// The table each catalog's statement actually reads.
  fn table_read_by(catalog: Catalog) -> String {
    statement(catalog)
      .split("FROM aircraft_ref.")
      .nth(1)
      .unwrap_or_default()
      .chars()
      .take_while(|c| c.is_ascii_lowercase() || *c == '_')
      .collect()
  }

  /// This is where the catalog vocabulary meets the schema, and the only place
  /// it does: `aircraft_domain` carries slugs and no relation names, so a
  /// statement pointed at the wrong table is caught here rather than there.
  ///
  /// Positional, not set-based. Two statements whose tables were swapped leave
  /// the set unchanged and every other test green while each catalog serves the
  /// other's rows.
  #[test]
  fn the_statements_read_the_tables_migration_002_creates() {
    let created = tables_named_by("CREATE TABLE", MIGRATION_002);
    let read: Vec<String> = Catalog::ALL.iter().map(|c| table_read_by(*c)).collect();

    assert_eq!(created.len(), 36, "migration 002 creates 36 aircraft_ref tables");
    assert_eq!(read, created, "each catalog reads the migration's table at its own position");
  }

  /// The acceptance criterion says *seeded* catalogs: a table migration `002`
  /// creates but nothing populates would be an empty route, and a seeded table
  /// no catalog reads would be unreachable.
  #[test]
  fn every_statement_reads_a_seeded_table_and_every_seeded_table_is_read() {
    let mut seeded = tables_named_by("INSERT INTO", SEED_UNITS);
    seeded.extend(tables_named_by("INSERT INTO", SEED_LOOKUPS));
    seeded.sort_unstable();
    seeded.dedup();

    let mut read: Vec<String> = Catalog::ALL.iter().map(|c| table_read_by(*c)).collect();
    read.sort_unstable();

    assert_eq!(seeded.len(), 36, "both seeds together populate 36 aircraft_ref tables");
    assert_eq!(seeded, read, "the seeded tables and the catalogs' tables are the same set");
  }

  /// Every statement binds its bound and projects the published shape. The
  /// predicate and the ordering are per-catalog behavior and are proved against
  /// the real schema in `crates/aircraft_db/tests/reference_catalogs.rs`.
  #[test]
  fn every_statement_binds_its_bound_and_projects_the_published_columns() {
    for catalog in Catalog::ALL {
      let sql = statement(catalog);

      assert!(sql.starts_with("SELECT code, label, "), "{} projection: {sql}", catalog.slug());
      assert!(sql.ends_with("code LIMIT $1"), "{} must bind its bound: {sql}", catalog.slug());
    }
  }

  /// The slug a caller sends and the table its statement reads are the same
  /// name in two spellings. Pinned here because this is the crate that knows
  /// both, and because `database/roles/app_grants.sql` grants per table while
  /// the route is addressed per slug.
  #[test]
  fn a_slug_is_its_table_with_hyphens_for_underscores() {
    for catalog in Catalog::ALL {
      assert_eq!(
        catalog.slug(),
        table_read_by(catalog).replace('_', "-"),
        "{} must address the table it reads",
        catalog.slug()
      );
    }
  }
}
