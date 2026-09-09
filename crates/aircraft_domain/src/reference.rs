//! The reference catalogs `aircraft_ref` publishes, as a closed vocabulary.
//!
//! [`Catalog`] is the allowlist behind `GET /v1/reference/{catalog}`: a request
//! names a catalog only by parsing into this type, so no caller-supplied text
//! can reach a table name. `aircraft_app::reference` reads a catalog through it
//! and `aircraft_db` maps each variant to one complete statement.
//!
//! The *set* mirrors `database/migrations/002_core_reference_tables.sql`, which
//! `database/data_dictionary.md` names back, but no relation name appears here:
//! which table a catalog reads is `aircraft_db`'s question, and
//! `the_statements_read_the_tables_migration_002_creates` in
//! `reference_repository` is what pins the set and its order against that
//! migration. This crate carries only the slug a caller sends.

use thiserror::Error;

/// A seeded lookup catalog in the `aircraft_ref` schema.
///
/// Every variant is a catalog `database/migrations/002_core_reference_tables.sql`
/// defines and the reference seeds populate. There is no `Other`
/// variant and no wildcard arm anywhere, so a thirty-seventh catalog is a
/// compile error at each mapping rather than an unmapped value at runtime.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Catalog {
  UnitCategories,
  MeasurementUnits,
  AircraftRoles,
  ServiceStatuses,
  VariantTypes,
  LandingGearTypes,
  PropulsionCategories,
  FuelTypes,
  DimensionMetricTypes,
  WeightMetricTypes,
  PerformanceMetricTypes,
  CertificationAuthorities,
  AirworthinessCategories,
  PilotCertificateTypes,
  OperatingApprovalTypes,
  MilitaryMissionTypes,
  WeaponCategories,
  HardpointPositionTypes,
  StoresTypes,
  Currencies,
  CostItemTypes,
  AircraftConditionGrades,
  AdTypes,
  SbComplianceStatuses,
  AvailabilityGrades,
  SourceTypes,
  SourceReliabilityGrades,
  CurationFlagStatuses,
  CurationEntityTypes,
  AssertionStatuses,
  MissionProfileTypes,
  ComparisonCriterionTypes,
  OrganizationTypes,
  OrgRelationshipTypes,
  SystemsCategories,
  EquipmentProvisionTypes,
}

impl Catalog {
  /// Every catalog, in the order migration `002` declares them.
  ///
  /// Declaration order rather than any other, because
  /// `the_statements_read_the_tables_migration_002_creates` in `aircraft_db`
  /// compares this list against the migration positionally: two catalogs whose
  /// statements were swapped would leave the *set* unchanged and every other
  /// test green, while each served the other's rows.
  pub const ALL: [Self; 36] = [
    Self::UnitCategories,
    Self::MeasurementUnits,
    Self::AircraftRoles,
    Self::ServiceStatuses,
    Self::VariantTypes,
    Self::LandingGearTypes,
    Self::PropulsionCategories,
    Self::FuelTypes,
    Self::DimensionMetricTypes,
    Self::WeightMetricTypes,
    Self::PerformanceMetricTypes,
    Self::CertificationAuthorities,
    Self::AirworthinessCategories,
    Self::PilotCertificateTypes,
    Self::OperatingApprovalTypes,
    Self::MilitaryMissionTypes,
    Self::WeaponCategories,
    Self::HardpointPositionTypes,
    Self::StoresTypes,
    Self::Currencies,
    Self::CostItemTypes,
    Self::AircraftConditionGrades,
    Self::AdTypes,
    Self::SbComplianceStatuses,
    Self::AvailabilityGrades,
    Self::SourceTypes,
    Self::SourceReliabilityGrades,
    Self::CurationFlagStatuses,
    Self::CurationEntityTypes,
    Self::AssertionStatuses,
    Self::MissionProfileTypes,
    Self::ComparisonCriterionTypes,
    Self::OrganizationTypes,
    Self::OrgRelationshipTypes,
    Self::SystemsCategories,
    Self::EquipmentProvisionTypes,
  ];

  /// The path segment `GET /v1/reference/{catalog}` accepts for this catalog.
  ///
  /// The catalog's identity, and the only spelling this crate carries. It is a
  /// `aircraft_ref.slug_text` value -- lower-case ASCII words joined by single
  /// hyphens, the shape migration `001` requires of a routing identifier.
  ///
  /// Deliberately not a table name. `aircraft_db` owns which relation each
  /// catalog reads, in `reference_repository::statement`; a domain that named
  /// tables would have to be edited when the schema renamed one, which is the
  /// leak root `AGENTS.md` forbids under "Database layout must not leak into
  /// the domain".
  #[must_use]
  pub const fn slug(self) -> &'static str {
    match self {
      Self::UnitCategories => "unit-categories",
      Self::MeasurementUnits => "measurement-units",
      Self::AircraftRoles => "aircraft-roles",
      Self::ServiceStatuses => "service-statuses",
      Self::VariantTypes => "variant-types",
      Self::LandingGearTypes => "landing-gear-types",
      Self::PropulsionCategories => "propulsion-categories",
      Self::FuelTypes => "fuel-types",
      Self::DimensionMetricTypes => "dimension-metric-types",
      Self::WeightMetricTypes => "weight-metric-types",
      Self::PerformanceMetricTypes => "performance-metric-types",
      Self::CertificationAuthorities => "certification-authorities",
      Self::AirworthinessCategories => "airworthiness-categories",
      Self::PilotCertificateTypes => "pilot-certificate-types",
      Self::OperatingApprovalTypes => "operating-approval-types",
      Self::MilitaryMissionTypes => "military-mission-types",
      Self::WeaponCategories => "weapon-categories",
      Self::HardpointPositionTypes => "hardpoint-position-types",
      Self::StoresTypes => "stores-types",
      Self::Currencies => "currencies",
      Self::CostItemTypes => "cost-item-types",
      Self::AircraftConditionGrades => "aircraft-condition-grades",
      Self::AdTypes => "ad-types",
      Self::SbComplianceStatuses => "sb-compliance-statuses",
      Self::AvailabilityGrades => "availability-grades",
      Self::SourceTypes => "source-types",
      Self::SourceReliabilityGrades => "source-reliability-grades",
      Self::CurationFlagStatuses => "curation-flag-statuses",
      Self::CurationEntityTypes => "curation-entity-types",
      Self::AssertionStatuses => "assertion-statuses",
      Self::MissionProfileTypes => "mission-profile-types",
      Self::ComparisonCriterionTypes => "comparison-criterion-types",
      Self::OrganizationTypes => "organization-types",
      Self::OrgRelationshipTypes => "org-relationship-types",
      Self::SystemsCategories => "systems-categories",
      Self::EquipmentProvisionTypes => "equipment-provision-types",
    }
  }
}

impl TryFrom<&str> for Catalog {
  type Error = UnknownCatalog;

  /// Resolves a URL path segment, exactly. An unknown segment is refused here
  /// so that nothing downstream has to decide what to do with one.
  fn try_from(slug: &str) -> Result<Self, Self::Error> {
    Self::ALL.into_iter().find(|catalog| catalog.slug() == slug).ok_or(UnknownCatalog)
  }
}

/// A path segment that names no catalog.
///
/// Carries nothing: the rejected text is caller-supplied, and a variant holding
/// it would invite echoing it into a log or a response.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("no reference catalog has that name")]
pub struct UnknownCatalog;

#[cfg(test)]
mod tests {
  use super::*;

  /// `aircraft_ref.slug_text` in
  /// `database/migrations/001_extensions_schemas_domains_triggers.sql` is
  /// `^[a-z0-9]+(-[a-z0-9]+)*$`. Checked without a regex crate: lower-case ASCII
  /// groups separated by single hyphens, with no leading, trailing, or doubled
  /// separator. A pure test, as `crates/AGENTS.md` requires of this crate --
  /// which relation each catalog reads is `aircraft_db`'s question, and
  /// `the_statements_read_the_tables_migration_002_creates` over there is what
  /// answers it.
  #[test]
  fn every_slug_matches_slug_text_and_resolves_back_to_its_catalog() {
    for catalog in Catalog::ALL {
      let slug = catalog.slug();

      assert!(
        slug.split('-').all(|part| {
          !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        }),
        "{slug} is not a slug_text value"
      );
      assert_eq!(Catalog::try_from(slug), Ok(catalog), "{slug} must resolve back");
    }
  }

  /// Two catalogs sharing a slug would make one of them unreachable, and the
  /// `find` in `TryFrom` would silently prefer whichever came first.
  #[test]
  fn no_two_catalogs_share_a_slug() {
    let mut slugs = Catalog::ALL.map(Catalog::slug).to_vec();
    slugs.sort_unstable();
    let total = slugs.len();
    slugs.dedup();

    assert_eq!(slugs.len(), total, "every catalog needs its own slug");
    assert_eq!(total, 36, "the closed vocabulary is 36 catalogs");
  }

  /// The refusal AC2 rests on. The underscore spelling matters on its own: it is
  /// what the schema calls these tables, and accepting it would give every
  /// catalog a second, undocumented spelling this contract never published.
  #[test]
  fn a_segment_that_names_no_catalog_is_refused() {
    for segment in ["", "aircraft_roles", "aircraft-role", "AIRCRAFT-ROLES", "pg_class", "../"] {
      assert_eq!(Catalog::try_from(segment), Err(UnknownCatalog), "segment {segment:?}");
    }
  }
}
