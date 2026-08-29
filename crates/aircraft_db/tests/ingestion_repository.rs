use std::time::Duration;

use aircraft_app::ingestion::{
  ImportReport, ImportRequest, ImportStatus, IngestionStore, PreparedAircraftRecord,
  REPORT_SCHEMA_VERSION, RecordDisposition,
};
use aircraft_db::SqlxIngestionStore;
use aircraft_ingest::normalization::normalize_record;
use aircraft_testsupport::{TestResult, install_schema, ready, request, start_postgres};
use serde_json::json;
use sqlx_core::{query::query, query_scalar::query_scalar, row::Row};

/// Drives one prepared record through a full staged-and-promoted import,
/// committing the transaction.
async fn import_record(
  store: &SqlxIngestionStore,
  request: ImportRequest,
  record: &PreparedAircraftRecord,
) -> TestResult {
  let (run_id, attempt_id, mut work) = ready(store.start_import(&request).await?)?;
  let outcome = work.stage_and_promote(record).await?;
  let report = ImportReport {
    schema_version: REPORT_SCHEMA_VERSION,
    run_id,
    attempt_id,
    status: ImportStatus::Succeeded,
    content_sha256: request.artifact.content_sha256,
    staged_records: u64::from(outcome.staged),
    promoted_records: u64::from(matches!(outcome.disposition, RecordDisposition::Promoted)),
    flagged_records: u64::from(matches!(outcome.disposition, RecordDisposition::Flagged)),
    warning_count: outcome.warning_count,
    already_imported: false,
  };
  work.refresh_read_models().await?;
  work.mark_succeeded(&report).await?;
  work.commit().await?;
  Ok(())
}

#[tokio::test]
async fn concurrent_distinct_imports_complete_with_minimum_pool() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let first_record =
    normalize_record("CESSNA", "172S", json!({"description": "first concurrent artifact"}));
  let second_record =
    normalize_record("PIPER", "PA-28", json!({"description": "second concurrent artifact"}));

  tokio::time::timeout(Duration::from_secs(30), async {
    tokio::try_join!(
      import_record(&store, request('d', "1.0.0"), &first_record),
      import_record(&store, request('e', "1.0.0"), &second_record),
    )
  })
  .await
  .map_err(|_| std::io::Error::other("concurrent imports did not complete"))??;

  let succeeded: i64 =
    query_scalar("SELECT count(*) FROM aircraft_ingest.ingest_runs WHERE status = 'SUCCEEDED'")
      .fetch_one(&pool)
      .await?;
  assert_eq!(succeeded, 2);
  Ok(())
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn repository_preserves_ingestion_semantics_and_attempt_history() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let first_record = normalize_record(
    "CESSNA",
    "172S",
    json!({
        "description": "first raw document",
        "performance": {
            "best_cruise_speed": "124 FURLONGS",
            "ceiling": "14000 FT"
        },
        "ownership_costs": {
            "annual_inspection_cost": "$2,200",
            "fuel_cost_per_hour": "$45.90"
        }
    }),
  );
  assert!(first_record.issues.iter().any(|issue| issue.code == "UNKNOWN_MEASUREMENT_UNIT"));

  import_record(&store, request('a', "1.0.0"), &first_record).await?;

  let canonical_speed_count: i64 = query_scalar(
    "SELECT count(*) FROM aircraft_specs.performance_metrics
         WHERE metric_type_code = 'SPEED_CRUISE_BEST'",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(canonical_speed_count, 0);

  let unsafe_assertion_is_pending: bool = query_scalar(
    "SELECT status_code = 'PENDING' AND NOT is_accepted
         FROM aircraft_prov.source_assertions
         WHERE field_name = 'performance.SPEED_CRUISE_BEST'",
  )
  .fetch_one(&pool)
  .await?;
  assert!(unsafe_assertion_is_pending);

  let planephd_known_unit_value_is_pending: bool = query_scalar(
    "SELECT NOT metric.is_canonical
                AND assertion.status_code = 'PENDING'
                AND NOT assertion.is_accepted
         FROM aircraft_specs.performance_metrics AS metric
         JOIN aircraft_prov.source_assertions AS assertion
           ON assertion.entity_type_code = 'AIRCRAFT_VARIANT'
          AND assertion.entity_id = metric.variant_id
          AND assertion.field_name = 'performance.CEILING_SERVICE'
         WHERE metric.metric_type_code = 'CEILING_SERVICE'",
  )
  .fetch_one(&pool)
  .await?;
  assert!(planephd_known_unit_value_is_pending);

  let fuel_is_hourly: bool = query_scalar(
    "SELECT amount_annual IS NULL AND amount_per_hour = 45.90
         FROM aircraft_market.cost_line_items
         WHERE cost_item_type_code = 'FUEL'",
  )
  .fetch_one(&pool)
  .await?;
  assert!(fuel_is_hourly);

  let inspection_is_annual: bool = query_scalar(
    "SELECT amount_annual = 2200 AND amount_per_hour IS NULL
         FROM aircraft_market.cost_line_items
         WHERE cost_item_type_code = 'ANNUAL_INSPECTION'",
  )
  .fetch_one(&pool)
  .await?;
  assert!(inspection_is_annual);

  let second_record = normalize_record(
    "CESSNA",
    "172S",
    json!({
        "description": "second raw document",
        "performance": {
            "best_cruise_speed": "125 KIAS",
            "best_range_i": "640 NM"
        },
        "weights": {"gross_weight": "2550 LBS"}
    }),
  );
  import_record(&store, request('b', "1.1.0"), &second_record).await?;

  // Before migration 019 this asserted the imported weight was immediately
  // searchable, which encoded the missing curation gate as expected behavior.
  // Nothing this adapter writes may be served before curation accepts it.
  let uncurated_values_are_withheld: bool = query_scalar(
    "SELECT cruise_speed_kias IS NULL
                AND range_nm IS NULL
                AND gross_weight_lb IS NULL
         FROM aircraft_read.mv_variant_search
         WHERE variant_name = '172S'",
  )
  .fetch_one(&pool)
  .await?;
  assert!(uncurated_values_are_withheld, "imported values must stay out of the read model");

  // Stand in for the curation step that does not exist yet: accept both metric
  // families, which is what a curator will ultimately do.
  query(
    "UPDATE aircraft_specs.performance_metrics
         SET is_canonical = TRUE
         WHERE metric_type_code IN ('SPEED_CRUISE_BEST', 'RANGE_NORMAL')",
  )
  .execute(&pool)
  .await?;
  query(
    "UPDATE aircraft_specs.weight_metrics
         SET is_canonical = TRUE
         WHERE metric_type_code = 'WEIGHT_MTOW'",
  )
  .execute(&pool)
  .await?;
  query("SELECT aircraft_read.refresh_search_matviews(FALSE)").execute(&pool).await?;

  let curated_import_values_are_searchable: bool = query_scalar(
    "SELECT cruise_speed_kias = 125
                AND range_nm = 640
                AND gross_weight_lb = 2550
         FROM aircraft_read.mv_variant_search
         WHERE variant_name = '172S'",
  )
  .fetch_one(&pool)
  .await?;
  assert!(curated_import_values_are_searchable);

  let document_evidence = query(
    "SELECT count(*) AS document_count,
                count(DISTINCT raw_json ->> 'description') AS raw_version_count,
                count(DISTINCT ingest_run_id) AS run_count
         FROM aircraft_prov.source_documents
         WHERE source_system_key = $1",
  )
  .bind(&first_record.source_record_key)
  .fetch_one(&pool)
  .await?;
  assert_eq!(document_evidence.get::<i64, _>("document_count"), 2);
  assert_eq!(document_evidence.get::<i64, _>("raw_version_count"), 2);
  assert_eq!(document_evidence.get::<i64, _>("run_count"), 2);

  let asserted_document_count: i64 = query_scalar(
    "SELECT count(DISTINCT source_document_id)
         FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'AIRCRAFT_VARIANT' AND field_name = 'name'",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(asserted_document_count, 2);

  let retry_request = request('c', "1.0.0");
  let (run_id, stale_attempt_id, stale_work) = ready(store.start_import(&retry_request).await?)?;
  stale_work.rollback().await?;

  let (_, current_attempt_id, current_work) = ready(store.start_import(&retry_request).await?)?;
  let attempts = query(
    "SELECT id, status, failure_code, finished_at IS NOT NULL AS finished
         FROM aircraft_ingest.ingest_run_attempts
         WHERE ingest_run_id = $1 ORDER BY attempt_number",
  )
  .bind(run_id)
  .fetch_all(&pool)
  .await?;
  assert_eq!(attempts.len(), 2);
  assert_eq!(attempts[0].get::<i64, _>("id"), stale_attempt_id);
  assert_eq!(attempts[0].get::<String, _>("status"), "FAILED");
  assert_eq!(
    attempts[0].get::<Option<String>, _>("failure_code").as_deref(),
    Some("PROCESS_TERMINATED")
  );
  assert!(attempts[0].get::<bool, _>("finished"));
  assert_eq!(attempts[1].get::<i64, _>("id"), current_attempt_id);
  assert_eq!(attempts[1].get::<String, _>("status"), "IMPORTING");

  current_work.rollback().await?;
  store.mark_failed(run_id, current_attempt_id, "TEST_CLEANUP", "integration test cleanup").await?;
  Ok(())
}

/// Gate: transaction rollback, at the boundary where it actually happens.
///
/// The CLI gate covers a preflight failure, which aborts before staging begins.
/// This covers the other half: records that were already staged and promoted
/// inside the long transaction must vanish when that transaction rolls back,
/// while the run and attempt rows — written by a separate, already-committed
/// transaction — survive so the failure remains auditable.
#[tokio::test]
async fn rolling_back_the_import_transaction_discards_staged_and_promoted_rows() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let record = normalize_record(
    "CESSNA",
    "172S Skyhawk SP",
    json!({
        "description": "rollback fixture",
        "performance": {"best_cruise_speed": "124 KIAS"},
        "weights": {"gross_weight": "2550 LBS"}
    }),
  );

  let (run_id, attempt_id, mut work) = ready(store.start_import(&request('f', "1.0.0")).await?)?;
  work.stage_and_promote(&record).await?;

  // Uncommitted writes are invisible to the pool's other connections, so a
  // non-zero count here would mean the work escaped its transaction.
  let staged_before: i64 =
    query_scalar("SELECT COUNT(*) FROM aircraft_ingest.staged_aircraft").fetch_one(&pool).await?;
  assert_eq!(staged_before, 0, "staged rows must not be visible before commit");

  work.rollback().await?;
  store.mark_failed(run_id, attempt_id, "TEST_ROLLBACK", "deliberate rollback").await?;

  let staged: i64 =
    query_scalar("SELECT COUNT(*) FROM aircraft_ingest.staged_aircraft").fetch_one(&pool).await?;
  assert_eq!(staged, 0, "rollback must discard staged rows");
  let variants: i64 =
    query_scalar("SELECT COUNT(*) FROM aircraft_core.variants WHERE ingest_key IS NOT NULL")
      .fetch_one(&pool)
      .await?;
  assert_eq!(variants, 0, "rollback must discard promoted identity rows");
  let assertions: i64 =
    query_scalar("SELECT COUNT(*) FROM aircraft_prov.source_assertions").fetch_one(&pool).await?;
  assert_eq!(assertions, 0, "rollback must discard provenance assertions");

  let attempt: (String, Option<String>) = {
    let row = query("SELECT status, failure_code FROM aircraft_ingest.ingest_run_attempts")
      .fetch_one(&pool)
      .await?;
    (row.get("status"), row.get("failure_code"))
  };
  assert_eq!(attempt.0, "FAILED", "the audit trail survives the rollback");
  assert_eq!(attempt.1.as_deref(), Some("TEST_ROLLBACK"));
  Ok(())
}
