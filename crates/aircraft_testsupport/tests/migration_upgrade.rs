//! Upgrade safety for the curation gates.
//!
//! Migrations 019 and 020 put weight metrics, valuations, and cost snapshots
//! behind `is_canonical`. Each gate backfills only the rows its pre-migration
//! read model already served, so an upgrade must not publish anything that was
//! previously withheld. Each gate is asserted on its own so a regression names
//! the table it broke.

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
