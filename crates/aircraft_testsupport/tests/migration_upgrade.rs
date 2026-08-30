//! Upgrade safety for the migrations that backfill an existing database.
//!
//! Migrations 019 and 020 put weight metrics, valuations, and cost snapshots
//! behind `is_canonical`. Each gate backfills only the rows its pre-migration
//! read model already served, so an upgrade must not publish anything that was
//! previously withheld. Migration 023 backfills in the other direction, filling
//! identity projections that Rust ingestion left empty. Both kinds are only
//! exercised by installing the schema up to the migration, seeding the state it
//! is written for, and then applying it -- a clean install reaches every one of
//! them with nothing to match. Each backfill is asserted on its own so a
//! regression names the table it broke.

use std::time::Duration;

use aircraft_testsupport::{DockerPostgres, SCHEMA_STEPS, TestResult, start_postgres};
use sqlx_core::{query_scalar::query_scalar, raw_sql::raw_sql};
use sqlx_postgres::PgPool;

/// Installs the schema up to the first curation gate, seeds a variant with rows
/// on both sides of every gate's backfill predicate, then applies the remaining
/// migrations. Returns the pool, keeping the container alive for the caller.
async fn upgrade_across_the_curation_gates() -> TestResult<(DockerPostgres, PgPool)> {
  let (container, pool) = start_postgres(2, Duration::from_secs(5)).await?;
  let gate_index = SCHEMA_STEPS
    .iter()
    .position(|sql| sql.contains("019_weight_metrics_curation_gate.sql"))
    .ok_or_else(|| std::io::Error::other("migration 019 is missing from SCHEMA_STEPS"))?;
  for sql in &SCHEMA_STEPS[..gate_index] {
    raw_sql(sql).execute(&pool).await?;
  }

  raw_sql(
    "WITH family AS (
         INSERT INTO aircraft_core.families(slug, name)
         VALUES('upgrade-safety', 'Upgrade Safety') RETURNING id
     ), model AS (
         INSERT INTO aircraft_core.models(family_id, slug, name)
         SELECT id, 'upgrade-safety', 'Upgrade Safety' FROM family RETURNING id
     )
     INSERT INTO aircraft_core.variants(model_id, slug, name)
     SELECT id, 'upgrade-safety', 'Upgrade Safety' FROM model;

     INSERT INTO aircraft_specs.weight_metrics(
         variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value, configuration)
     SELECT id, 'WEIGHT_MTOW', 2500, 'LBS', 2500, configuration
     FROM aircraft_core.variants
     CROSS JOIN (VALUES(NULL::text), ('FERRY')) configurations(configuration);

     INSERT INTO aircraft_market.valuations(
         variant_id, snapshot_date, source_name, papi_price_estimate)
     SELECT id, snapshot_date, source_name, price
     FROM aircraft_core.variants
     CROSS JOIN (VALUES
         (DATE '2025-01-01', 'older', 100000::numeric),
         (DATE '2026-01-01', 'newer', 110000::numeric)
     ) snapshots(snapshot_date, source_name, price);

     INSERT INTO aircraft_market.cost_snapshots(variant_id, snapshot_date, source_name)
     SELECT id, snapshot_date, source_name
     FROM aircraft_core.variants
     CROSS JOIN (VALUES
         (DATE '2025-01-01', 'older'),
         (DATE '2026-01-01', 'newer')
     ) snapshots(snapshot_date, source_name);",
  )
  .execute(&pool)
  .await?;

  for sql in &SCHEMA_STEPS[gate_index..] {
    raw_sql(sql).execute(&pool).await?;
  }
  Ok((container, pool))
}

/// Migration 016 served only `configuration IS NULL` weights, so migration 019
/// must leave a configured sibling pending.
#[tokio::test]
async fn the_weight_gate_leaves_configured_rows_pending() -> TestResult {
  let (_container, pool) = upgrade_across_the_curation_gates().await?;

  let configured_canonical_weights: i64 = query_scalar(
    "SELECT count(*) FROM aircraft_specs.weight_metrics
         WHERE is_canonical AND configuration IS NOT NULL",
  )
  .fetch_one(&pool)
  .await?;

  assert_eq!(configured_canonical_weights, 0);
  Ok(())
}

/// Migration 020 backfills only the valuation the pre-migration read model
/// selected as current, so historical time-series siblings stay pending.
#[tokio::test]
async fn the_valuation_gate_leaves_historical_rows_pending() -> TestResult {
  let (_container, pool) = upgrade_across_the_curation_gates().await?;

  let historical_canonical_valuations: i64 = query_scalar(
    "SELECT count(*) FROM aircraft_market.valuations
         WHERE is_canonical AND source_name <> 'newer'",
  )
  .fetch_one(&pool)
  .await?;

  assert_eq!(historical_canonical_valuations, 0);
  Ok(())
}

/// The same backfill rule applies to cost snapshots, which are gated as whole
/// snapshots rather than line items.
#[tokio::test]
async fn the_cost_snapshot_gate_leaves_historical_rows_pending() -> TestResult {
  let (_container, pool) = upgrade_across_the_curation_gates().await?;

  let historical_canonical_costs: i64 = query_scalar(
    "SELECT count(*) FROM aircraft_market.cost_snapshots
         WHERE is_canonical AND source_name <> 'newer'",
  )
  .fetch_one(&pool)
  .await?;

  assert_eq!(historical_canonical_costs, 0);
  Ok(())
}

/// Installs the schema up to migration 023, seeds the pre-023 state a Rust
/// import used to leave behind — a variant with a primary powerplant, a source
/// document, no `engine_count`, and no manufacturer link — alongside a manually
/// curated variant with the same gap and no import, then applies 023.
///
/// The mid-way assertion is the anti-vacuity guard: without it every test below
/// would also pass against a migration whose `UPDATE` and `INSERT` matched
/// nothing.
async fn upgrade_across_the_identity_backfill() -> TestResult<(DockerPostgres, PgPool)> {
  let (container, pool) = start_postgres(2, Duration::from_secs(5)).await?;
  let backfill_index = SCHEMA_STEPS
    .iter()
    .position(|sql| sql.contains("Phase 23: Complete source-backed identity projections"))
    .ok_or_else(|| std::io::Error::other("migration 023 is missing from SCHEMA_STEPS"))?;
  for sql in &SCHEMA_STEPS[..backfill_index] {
    raw_sql(sql).execute(&pool).await?;
  }

  raw_sql(
    "INSERT INTO aircraft_org.organizations(slug, name, org_type_code)
     VALUES('backfill-aviation', 'Backfill Aviation', 'MANUFACTURER');

     INSERT INTO aircraft_core.families(slug, name, manufacturer_org_id)
     SELECT 'backfill-family', 'Backfill Family', id
     FROM aircraft_org.organizations WHERE slug = 'backfill-aviation';

     INSERT INTO aircraft_core.models(family_id, slug, name)
     SELECT id, 'backfill-model', 'Backfill Model'
     FROM aircraft_core.families WHERE slug = 'backfill-family';

     INSERT INTO aircraft_core.variants(model_id, slug, name)
     SELECT model.id, variants.slug, variants.name
     FROM aircraft_core.models AS model
     CROSS JOIN (VALUES
         ('backfill-imported', 'Backfill Imported'),
         ('backfill-curated', 'Backfill Curated')
     ) variants(slug, name)
     WHERE model.slug = 'backfill-model';

     INSERT INTO aircraft_power.engine_variants(slug, model_designation)
     VALUES('backfill-engine', 'BF-1');

     INSERT INTO aircraft_power.variant_powerplants(
         variant_id, engine_variant_id, engine_count, is_standard, is_primary)
     SELECT variant.id, engine.id, 2, TRUE, TRUE
     FROM aircraft_core.variants AS variant
     CROSS JOIN aircraft_power.engine_variants AS engine
     WHERE variant.slug IN ('backfill-imported', 'backfill-curated')
       AND engine.slug = 'backfill-engine';

     INSERT INTO aircraft_prov.sources(slug, name)
     VALUES('backfill-source', 'Backfill Source');

     INSERT INTO aircraft_ingest.ingest_runs(run_label, source_name, source_slug, status)
     VALUES('backfill-run', 'Backfill Source', 'backfill-source', 'SUCCEEDED');

     INSERT INTO aircraft_prov.source_documents(source_id, variant_id, ingest_run_id)
     SELECT source.id, variant.id, run.id
     FROM aircraft_prov.sources AS source,
          aircraft_core.variants AS variant,
          aircraft_ingest.ingest_runs AS run
     WHERE source.slug = 'backfill-source'
       AND variant.slug = 'backfill-imported'
       AND run.run_label = 'backfill-run';",
  )
  .execute(&pool)
  .await?;

  let projections_are_missing: bool = query_scalar(
    "SELECT NOT EXISTS(
             SELECT 1 FROM aircraft_core.variants WHERE engine_count IS NOT NULL)
         AND NOT EXISTS(SELECT 1 FROM aircraft_core.variant_manufacturers)",
  )
  .fetch_one(&pool)
  .await?;
  assert!(projections_are_missing, "the seeded pre-023 state must have both projections missing");

  for sql in &SCHEMA_STEPS[backfill_index..] {
    raw_sql(sql).execute(&pool).await?;
  }
  Ok((container, pool))
}

/// The variant-level `engine_count` `mv_variant_search` serves comes from the
/// primary powerplant an import already recorded.
#[tokio::test]
async fn the_identity_backfill_completes_engine_counts_for_imported_variants() -> TestResult {
  let (_container, pool) = upgrade_across_the_identity_backfill().await?;

  let declared_engine_count: Option<i16> = query_scalar(
    "SELECT declared_engine_count FROM aircraft_read.mv_variant_search
         WHERE slug = 'backfill-imported'",
  )
  .fetch_one(&pool)
  .await?;

  assert_eq!(declared_engine_count, Some(2));
  Ok(())
}

/// The manufacturer is unambiguous through the family an import already
/// attached, so 023 projects it onto the variant and into the read model.
#[tokio::test]
async fn the_identity_backfill_publishes_the_manufacturer_of_imported_variants() -> TestResult {
  let (_container, pool) = upgrade_across_the_identity_backfill().await?;

  let linked_as_primary: bool = query_scalar(
    "SELECT EXISTS(
             SELECT 1 FROM aircraft_core.variant_manufacturers AS link
             JOIN aircraft_core.variants AS variant ON variant.id = link.variant_id
             JOIN aircraft_org.organizations AS org ON org.id = link.org_id
             WHERE variant.slug = 'backfill-imported'
               AND org.slug = 'backfill-aviation'
               AND link.is_primary)",
  )
  .fetch_one(&pool)
  .await?;
  assert!(linked_as_primary, "the imported variant must carry a primary manufacturer link");

  let published_manufacturer: Option<String> = query_scalar(
    "SELECT primary_manufacturer_slug FROM aircraft_read.mv_variant_search
         WHERE slug = 'backfill-imported'",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(published_manufacturer.as_deref(), Some("backfill-aviation"));
  Ok(())
}

/// The backfill is bounded to variants with a Rust ingestion document, so a
/// manually curated variant with the same gap keeps its existing semantics.
#[tokio::test]
async fn the_identity_backfill_leaves_manually_curated_variants_alone() -> TestResult {
  let (_container, pool) = upgrade_across_the_identity_backfill().await?;

  let curated_variant_untouched: bool = query_scalar(
    "SELECT (SELECT engine_count IS NULL FROM aircraft_core.variants
              WHERE slug = 'backfill-curated')
         AND NOT EXISTS(
             SELECT 1 FROM aircraft_core.variant_manufacturers AS link
             JOIN aircraft_core.variants AS variant ON variant.id = link.variant_id
             WHERE variant.slug = 'backfill-curated')",
  )
  .fetch_one(&pool)
  .await?;

  assert!(curated_variant_untouched, "023 must not touch a variant with no ingestion document");
  Ok(())
}
