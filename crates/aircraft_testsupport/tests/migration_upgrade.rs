use std::time::Duration;

use aircraft_testsupport::{SCHEMA_STEPS, TestResult, start_postgres};
use sqlx_core::{query_scalar::query_scalar, raw_sql::raw_sql};

#[tokio::test]
async fn curation_gates_preserve_only_the_rows_previously_served() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(5)).await?;
  for sql in &SCHEMA_STEPS[..21] {
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

  for sql in &SCHEMA_STEPS[21..24] {
    raw_sql(sql).execute(&pool).await?;
  }

  let canonical_weight_configuration: Option<String> =
    query_scalar("SELECT configuration FROM aircraft_specs.weight_metrics WHERE is_canonical")
      .fetch_one(&pool)
      .await?;
  assert_eq!(canonical_weight_configuration, None);

  let canonical_valuation_source: String =
    query_scalar("SELECT source_name FROM aircraft_market.valuations WHERE is_canonical")
      .fetch_one(&pool)
      .await?;
  assert_eq!(canonical_valuation_source, "newer");

  let canonical_cost_source: String =
    query_scalar("SELECT source_name FROM aircraft_market.cost_snapshots WHERE is_canonical")
      .fetch_one(&pool)
      .await?;
  assert_eq!(canonical_cost_source, "newer");
  Ok(())
}
