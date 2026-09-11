//! Canonical seed completeness and convergence against disposable `PostgreSQL`.
//!
//! These tests bind the four files under `database/seeds/` to the completeness
//! assertions in `database/validation/002_core_reference_tables_validation.sql`,
//! `database/validation/phase15_16_comparison_readmodels_validation.sql`, and
//! `database/validation/005_certification_validation.sql`.
//!
//! Those three are the validation files whose assertions are about seeded
//! content. The rest of `database/validation/` runs only under
//! `just db-validate`, which needs a live compose database and reaches neither
//! the verification ladder nor CI -- so an assertion left out of this list is an
//! assertion nothing automated ever executes.

use std::time::Duration;

use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use sqlx_core::{query_scalar::query_scalar, raw_sql::raw_sql};

const REFERENCE_VALIDATION: &str =
  include_str!("../../../database/validation/002_core_reference_tables_validation.sql");
const MISSION_VALIDATION: &str =
  include_str!("../../../database/validation/phase15_16_comparison_readmodels_validation.sql");
const CERTIFICATION_VALIDATION: &str =
  include_str!("../../../database/validation/005_certification_validation.sql");
const REFERENCE_UNIT_SEED: &str = include_str!("../../../database/seeds/001_reference_units.sql");
const LOOKUP_SEED: &str = include_str!("../../../database/seeds/002_lookup_seed_data.sql");
const MISSION_SEED: &str =
  include_str!("../../../database/seeds/003_mission_profile_seed_data.sql");
const AUTHENTICATION_SEED: &str =
  include_str!("../../../database/seeds/004_authentication_seed_data.sql");

#[tokio::test]
async fn canonical_seed_rows_are_complete_and_semantically_valid() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  raw_sql(REFERENCE_VALIDATION).execute(&pool).await?;
  raw_sql(MISSION_VALIDATION).execute(&pool).await?;
  raw_sql(CERTIFICATION_VALIDATION).execute(&pool).await?;

  Ok(())
}

// Keeping the four drift sites and both cache outcomes in one narrative makes
// the cross-seed behavior auditable; extracting the SQL would hide the setup.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn reapplying_seeds_repairs_drift_without_retaining_stale_scores() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let personal_profile_id: i64 = query_scalar(
    "SELECT id FROM aircraft_compare.mission_profiles WHERE slug = 'personal-vfr-touring'",
  )
  .fetch_one(&pool)
  .await?;
  let personal_price_criterion_id: i64 = query_scalar(
    "SELECT mc.id
       FROM aircraft_compare.mission_criteria AS mc
       JOIN aircraft_compare.mission_profiles AS mp ON mp.id = mc.mission_profile_id
      WHERE mp.slug = 'personal-vfr-touring'
        AND mc.criterion_type_code = 'CRITERION_PRICE'",
  )
  .fetch_one(&pool)
  .await?;
  let business_profile_id: i64 =
    query_scalar("SELECT id FROM aircraft_compare.mission_profiles WHERE slug = 'business-travel'")
      .fetch_one(&pool)
      .await?;

  raw_sql(
    "UPDATE aircraft_ref.unit_categories
        SET label = 'Drifted speed'
      WHERE code = 'SPEED';
     UPDATE aircraft_ref.aircraft_roles
        SET description = 'Drifted role'
      WHERE code = 'LIGHT_SPORT';
     UPDATE aircraft_compare.mission_criteria
        SET weight = 0.049,
            scoring_lower_bound = NULL,
            notes = 'Drifted policy'
      WHERE mission_profile_id = (
              SELECT id FROM aircraft_compare.mission_profiles
               WHERE slug = 'personal-vfr-touring')
        AND criterion_type_code = 'CRITERION_FUEL_EFFICIENCY';
     INSERT INTO aircraft_compare.mission_criteria(
         mission_profile_id, criterion_type_code, weight, scoring_lower_bound,
         scoring_upper_bound, notes)
     SELECT id, 'CRITERION_WINGSPAN', 0.001, 20, 80, 'Stale policy row'
       FROM aircraft_compare.mission_profiles
      WHERE slug = 'personal-vfr-touring';
     UPDATE aircraft_auth.scopes
        SET label = 'Drifted catalog scope'
      WHERE code = 'CATALOG_READ';
     INSERT INTO aircraft_auth.scopes(code, label, description, sort_order)
     VALUES('LOCAL_EXTENSION', 'Local extension', 'User-owned scope row.', 900);

     WITH family AS (
       INSERT INTO aircraft_core.families(slug, name)
       VALUES('seed-convergence', 'Seed Convergence') RETURNING id
     ), model AS (
       INSERT INTO aircraft_core.models(family_id, slug, name)
       SELECT id, 'seed-convergence', 'Seed Convergence' FROM family RETURNING id
     )
     INSERT INTO aircraft_core.variants(model_id, slug, name)
     SELECT id, 'seed-convergence', 'Seed Convergence' FROM model;

     INSERT INTO aircraft_compare.variant_suitability(
         variant_id, mission_profile_id, overall_score, scored_criteria_count,
         total_criteria_count)
     SELECT variant.id, profile.id, 0.500, 1, 6
       FROM aircraft_core.variants AS variant
       CROSS JOIN aircraft_compare.mission_profiles AS profile
      WHERE variant.slug = 'seed-convergence'
        AND profile.slug IN ('personal-vfr-touring', 'business-travel');

     INSERT INTO aircraft_compare.criterion_scores(
         variant_suitability_id, criterion_type_code, raw_canonical_value,
         raw_score, weighted_score, meets_minimum, notes)
     SELECT suitability.id, 'CRITERION_PRICE', 100000, 0.500, 0.150, TRUE,
            'Derived before seed reapplication.'
       FROM aircraft_compare.variant_suitability AS suitability;",
  )
  .execute(&pool)
  .await?;

  for seed in [REFERENCE_UNIT_SEED, LOOKUP_SEED, MISSION_SEED, AUTHENTICATION_SEED] {
    raw_sql(seed).execute(&pool).await?;
  }

  let canonical_values_are_restored: bool = query_scalar(
    "SELECT (SELECT label = 'Speed'
               FROM aircraft_ref.unit_categories WHERE code = 'SPEED')
        AND (SELECT description = 'Operated under a light-sport certification '
                                  'standard; the governing weight, seating, and '
                                  'speed limits live in airworthiness_categories.'
               FROM aircraft_ref.aircraft_roles WHERE code = 'LIGHT_SPORT')
        AND (SELECT label = 'Catalog read'
               FROM aircraft_auth.scopes WHERE code = 'CATALOG_READ')
        AND (SELECT weight = 0.050
                    AND scoring_lower_bound = 5
                    AND scoring_upper_bound = 25
                    AND notes = 'NM/gal; 5 minimum, 25 ideal.'
               FROM aircraft_compare.mission_criteria AS mc
               JOIN aircraft_compare.mission_profiles AS mp
                 ON mp.id = mc.mission_profile_id
              WHERE mp.slug = 'personal-vfr-touring'
                AND mc.criterion_type_code = 'CRITERION_FUEL_EFFICIENCY')",
  )
  .fetch_one(&pool)
  .await?;
  assert!(canonical_values_are_restored, "all four seed files must repair their owned columns");

  let profile_id_after: i64 = query_scalar(
    "SELECT id FROM aircraft_compare.mission_profiles WHERE slug = 'personal-vfr-touring'",
  )
  .fetch_one(&pool)
  .await?;
  let criterion_id_after: i64 = query_scalar(
    "SELECT mc.id
       FROM aircraft_compare.mission_criteria AS mc
       JOIN aircraft_compare.mission_profiles AS mp ON mp.id = mc.mission_profile_id
      WHERE mp.slug = 'personal-vfr-touring'
        AND mc.criterion_type_code = 'CRITERION_PRICE'",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(profile_id_after, personal_profile_id, "profile identity must remain stable");
  assert_eq!(
    criterion_id_after, personal_price_criterion_id,
    "standing criterion identity must remain stable",
  );

  let cache_and_extension_state_is_correct: bool = query_scalar(
    "SELECT NOT EXISTS(
              SELECT 1 FROM aircraft_compare.mission_criteria AS mc
              WHERE mc.mission_profile_id = $1
                AND mc.criterion_type_code = 'CRITERION_WINGSPAN')
        AND NOT EXISTS(
              SELECT 1 FROM aircraft_compare.variant_suitability
              WHERE mission_profile_id = $1)
        AND EXISTS(
              SELECT 1 FROM aircraft_compare.variant_suitability AS suitability
              JOIN aircraft_compare.criterion_scores AS score
                ON score.variant_suitability_id = suitability.id
              WHERE suitability.mission_profile_id = $2)
        AND EXISTS(
              SELECT 1 FROM aircraft_auth.scopes WHERE code = 'LOCAL_EXTENSION')",
  )
  .bind(personal_profile_id)
  .bind(business_profile_id)
  .fetch_one(&pool)
  .await?;
  assert!(
    cache_and_extension_state_is_correct,
    "changed-profile scores must be removed while unchanged scores and unknown rows remain",
  );

  Ok(())
}
