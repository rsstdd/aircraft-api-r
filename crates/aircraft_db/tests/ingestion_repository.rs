use std::time::Duration;

use aircraft_app::{
  curation::{CurationError, CurationStore, Decision},
  ingestion::{
    ImportReport, ImportRequest, ImportStatus, IngestionStore, PreparedAircraftRecord,
    REPORT_SCHEMA_VERSION, RecordDisposition,
  },
};
use aircraft_db::{SqlxCurationStore, SqlxIngestionStore};
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
async fn import_publishes_the_source_manufacturer_and_engine_count() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let record = normalize_record(
    "CESSNA",
    "310R",
    json!({
        "description": "six seats up to 6 plus 2 crew",
        "performance": {"horsepower": "2 x 285 HP"},
        "engine": {
            "manufacturer": "Continental",
            "model": "IO-520-M",
            "horsepower": "285 HP"
        }
    }),
  );
  import_record(&store, request('f', "1.0.0"), &record).await?;

  let published = query(
    "SELECT search.primary_manufacturer_name, variant.engine_count
         FROM aircraft_read.mv_variant_search AS search
         JOIN aircraft_core.variants AS variant ON variant.id = search.variant_id
         WHERE search.variant_name = '310R'",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(published.get::<String, _>("primary_manufacturer_name"), "Cessna");
  assert_eq!(published.get::<i16, _>("engine_count"), 2);
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

  let curation = SqlxCurationStore::from_pool(pool.clone());
  for field_name in
    ["performance.SPEED_CRUISE_BEST", "performance.RANGE_NORMAL", "weight.WEIGHT_MTOW"]
  {
    let assertion_id: i64 = query_scalar(
      "SELECT id FROM aircraft_prov.source_assertions
           WHERE field_name = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(field_name)
    .fetch_one(&pool)
    .await?;
    curation.decide(assertion_id, Decision::Accepted).await?;
  }

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

#[tokio::test]
async fn concurrent_competing_acceptances_report_one_conflict() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let ingestion = SqlxIngestionStore::from_pool(pool.clone());
  let first =
    normalize_record("CESSNA", "172S", json!({"performance": {"best_cruise_speed": "124 KIAS"}}));
  let second =
    normalize_record("CESSNA", "172S", json!({"performance": {"best_cruise_speed": "125 KIAS"}}));
  import_record(&ingestion, request('a', "1.0.0"), &first).await?;
  import_record(&ingestion, request('b', "1.0.0"), &second).await?;

  let assertion_ids: Vec<i64> = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE field_name = 'performance.SPEED_CRUISE_BEST' ORDER BY id",
  )
  .fetch_all(&pool)
  .await?;
  assert_eq!(assertion_ids.len(), 2);

  let curation = SqlxCurationStore::from_pool(pool.clone());
  let (first_result, second_result) = tokio::join!(
    curation.decide(assertion_ids[0], Decision::Accepted),
    curation.decide(assertion_ids[1], Decision::Accepted),
  );
  let results: [_; 2] = (first_result, second_result).into();
  assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
  assert_eq!(
    results.iter().filter(|result| matches!(result, Err(CurationError::Conflict { .. }))).count(),
    1,
  );

  let accepted: i64 = query_scalar(
    "SELECT count(*) FROM aircraft_prov.source_assertions
         WHERE field_name = 'performance.SPEED_CRUISE_BEST' AND is_accepted",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(accepted, 1);
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

/// Market snapshots are unique per variant while their assertions are keyed per
/// snapshot row, so a variant's second valuation -- backfilled by migration 020,
/// or imported the next day or from another source -- collides with
/// `uq_val_canonical` in a way `uq_assertion_accepted` cannot see. The collision
/// must arrive as a typed decision refusal naming the snapshot that stands,
/// leave the transaction rolled back, and clear once that snapshot is withdrawn.
#[tokio::test]
async fn a_second_market_snapshot_reports_the_snapshot_that_blocks_it() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let record = normalize_record(
    "CESSNA",
    "172S",
    json!({"description": "first market document", "papi_price_estimate": "$310,000"}),
  );
  import_record(&store, request('a', "1.0.0"), &record).await?;

  let standing =
    query("SELECT id, variant_id FROM aircraft_market.valuations").fetch_one(&pool).await?;
  let (standing_valuation, variant_id): (i64, i64) =
    (standing.get("id"), standing.get("variant_id"));
  let standing_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'VALUATION' AND field_name = 'papi_price_estimate'",
  )
  .fetch_one(&pool)
  .await?;

  let curation = SqlxCurationStore::from_pool(pool.clone());
  curation.decide(standing_assertion, Decision::Accepted).await?;

  // The next day's snapshot for the same variant, carrying its own assertion.
  let newer_valuation: i64 = query_scalar(
    "INSERT INTO aircraft_market.valuations(
            variant_id, snapshot_date, source_name, papi_price_estimate,
            currency_code, captured_at)
         VALUES ($1, CURRENT_DATE + 1, 'PlanePHD', 325000, 'USD', now())
         RETURNING id",
  )
  .bind(variant_id)
  .fetch_one(&pool)
  .await?;
  let document_id: i64 =
    query_scalar("SELECT id FROM aircraft_prov.source_documents LIMIT 1").fetch_one(&pool).await?;
  let newer_assertion: i64 = query_scalar(
    "INSERT INTO aircraft_prov.source_assertions(
            source_document_id, entity_type_code, entity_id, field_name,
            raw_value, asserted_numeric, status_code, is_accepted)
         VALUES ($1, 'VALUATION', $2, 'papi_price_estimate', '$325,000', 325000, 'PENDING', FALSE)
         RETURNING id",
  )
  .bind(document_id)
  .bind(newer_valuation)
  .fetch_one(&pool)
  .await?;

  let refusal = curation.decide(newer_assertion, Decision::Accepted).await;
  assert!(
    matches!(
      &refusal,
      Err(CurationError::SnapshotConflict { entity_type_code, blocking_id })
        if entity_type_code == "VALUATION" && *blocking_id == standing_valuation
    ),
    "expected a snapshot conflict naming the standing valuation, got {refusal:?}"
  );

  let refusal_changed_nothing: bool = query_scalar(
    "SELECT (SELECT is_canonical FROM aircraft_market.valuations WHERE id = $1)
                AND NOT (SELECT is_canonical FROM aircraft_market.valuations WHERE id = $2)
                AND (SELECT status_code = 'PENDING' AND NOT is_accepted
                     FROM aircraft_prov.source_assertions WHERE id = $3)",
  )
  .bind(standing_valuation)
  .bind(newer_valuation)
  .bind(newer_assertion)
  .fetch_one(&pool)
  .await?;
  assert!(refusal_changed_nothing, "a refused decision must roll back completely");

  // Withdrawing the standing snapshot is the documented way through.
  curation.decide(standing_assertion, Decision::Rejected).await?;
  curation.decide(newer_assertion, Decision::Accepted).await?;

  let newest_snapshot_is_served: bool = query_scalar(
    "SELECT NOT (SELECT is_canonical FROM aircraft_market.valuations WHERE id = $1)
                AND (SELECT is_canonical FROM aircraft_market.valuations WHERE id = $2)",
  )
  .bind(standing_valuation)
  .bind(newer_valuation)
  .fetch_one(&pool)
  .await?;
  assert!(newest_snapshot_is_served, "the withdrawn snapshot must yield to the newer one");
  Ok(())
}

/// `uq_val_variant_date_source` allows one valuation per (variant, date, source),
/// so a second artifact imported the same day reuses the stored row while still
/// asserting its own price against it. Accepting that assertion must not publish
/// the older row's price under the newer one's evidence.
#[tokio::test]
async fn a_price_assertion_cannot_publish_a_different_stored_price() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let first = normalize_record(
    "CESSNA",
    "172S",
    json!({"description": "morning artifact", "papi_price_estimate": "$310,000"}),
  );
  import_record(&store, request('a', "1.0.0"), &first).await?;
  let second = normalize_record(
    "CESSNA",
    "172S",
    json!({"description": "afternoon artifact", "papi_price_estimate": "$325,000"}),
  );
  import_record(&store, request('b', "1.0.0"), &second).await?;

  // One stored row, still holding the morning price, and two competing assertions.
  let stored_price: String =
    query_scalar("SELECT papi_price_estimate::text FROM aircraft_market.valuations")
      .fetch_one(&pool)
      .await?;
  assert_eq!(stored_price, "310000.00", "the stored row keeps the value it was created with");

  let newer_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'VALUATION' AND field_name = 'papi_price_estimate'
           AND asserted_numeric = 325000",
  )
  .fetch_one(&pool)
  .await?;

  let curation = SqlxCurationStore::from_pool(pool.clone());
  let refusal = curation.decide(newer_assertion, Decision::Accepted).await;
  assert!(
    matches!(
      &refusal,
      Err(CurationError::ValueMismatch { assertion_id, field_name, asserted, stored })
        if *assertion_id == newer_assertion
          && field_name == "papi_price_estimate"
          && asserted == "325000"
          && stored == "310000.00"
    ),
    "expected the mismatch to name both values, got {refusal:?}"
  );

  let nothing_was_published: bool = query_scalar(
    "SELECT NOT EXISTS(SELECT 1 FROM aircraft_market.valuations WHERE is_canonical)
                AND (SELECT status_code = 'PENDING' AND NOT is_accepted
                     FROM aircraft_prov.source_assertions WHERE id = $1)",
  )
  .bind(newer_assertion)
  .fetch_one(&pool)
  .await?;
  assert!(nothing_was_published, "a refused decision must roll back completely");

  // The assertion that does describe the stored row still publishes normally.
  let matching_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'VALUATION' AND field_name = 'papi_price_estimate'
           AND asserted_numeric = 310000",
  )
  .fetch_one(&pool)
  .await?;
  curation.decide(matching_assertion, Decision::Accepted).await?;
  let published_price: String = query_scalar(
    "SELECT papi_price_estimate::text FROM aircraft_market.valuations WHERE is_canonical",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(published_price, "310000.00", "the published price is the one that was accepted");
  Ok(())
}

/// `uq_cs_variant_date_source` allows one cost snapshot per (variant, date,
/// source), so a second artifact imported the same day reuses the stored
/// snapshot and its totals row while still asserting its own aggregate totals
/// against them. Aggregates land in `cost_snapshot_totals` rather than
/// `cost_line_items`, so they need their own stored-versus-asserted comparison.
#[tokio::test]
async fn a_total_cost_assertion_cannot_publish_a_different_stored_total() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let first = normalize_record(
    "CESSNA",
    "172S",
    json!({"description": "morning artifact", "ownership_costs": {"total_fixed_cost": "$12,000"}}),
  );
  import_record(&store, request('a', "1.0.0"), &first).await?;
  let second = normalize_record(
    "CESSNA",
    "172S",
    json!({
        "description": "afternoon artifact",
        "ownership_costs": {"total_fixed_cost": "$13,000"}
    }),
  );
  import_record(&store, request('b', "1.0.0"), &second).await?;

  let stored_total: String =
    query_scalar("SELECT total_fixed_usd::text FROM aircraft_market.cost_snapshot_totals")
      .fetch_one(&pool)
      .await?;
  assert_eq!(stored_total, "12000.00", "the stored totals row keeps the value it was created with");

  let newer_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'COST_SNAPSHOT' AND field_name = 'TOTAL_FIXED_COST'
           AND asserted_numeric = 13000",
  )
  .fetch_one(&pool)
  .await?;

  let curation = SqlxCurationStore::from_pool(pool.clone());
  let refusal = curation.decide(newer_assertion, Decision::Accepted).await;
  assert!(
    matches!(
      &refusal,
      Err(CurationError::ValueMismatch { assertion_id, field_name, asserted, stored })
        if *assertion_id == newer_assertion
          && field_name == "TOTAL_FIXED_COST"
          && asserted == "13000"
          && stored == "12000.00"
    ),
    "expected the mismatch to name both totals, got {refusal:?}"
  );

  let nothing_was_published: bool = query_scalar(
    "SELECT NOT EXISTS(SELECT 1 FROM aircraft_market.cost_snapshots WHERE is_canonical)
                AND (SELECT status_code = 'PENDING' AND NOT is_accepted
                     FROM aircraft_prov.source_assertions WHERE id = $1)",
  )
  .bind(newer_assertion)
  .fetch_one(&pool)
  .await?;
  assert!(nothing_was_published, "a refused decision must roll back completely");

  // The assertion that does describe the stored totals still publishes normally.
  let matching_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'COST_SNAPSHOT' AND field_name = 'TOTAL_FIXED_COST'
           AND asserted_numeric = 12000",
  )
  .fetch_one(&pool)
  .await?;
  curation.decide(matching_assertion, Decision::Accepted).await?;
  let served_total: Option<String> = query_scalar(
    "SELECT source_total_fixed_usd::text FROM aircraft_read.mv_ownership_cost_summary",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(
    served_total.as_deref(),
    Some("12000.00"),
    "the served total is the one that was accepted"
  );
  Ok(())
}

/// `uq_variant_primary_mfr` allows one primary manufacturer per variant, so the
/// import guards its insert with "this variant has no primary link". A curator
/// who demotes the standing link satisfies that guard while the row itself still
/// collides on `(variant_id, org_id)`, and `ON CONFLICT DO NOTHING` used to leave
/// the variant with no primary manufacturer at all -- a successful import whose
/// read-model manufacturer had silently gone missing.
#[tokio::test]
async fn a_demoted_manufacturer_link_is_promoted_by_the_next_import() -> TestResult {
  let (_container, pool) = start_postgres(5, Duration::from_secs(30)).await?;
  install_schema(&pool).await?;

  let store = SqlxIngestionStore::from_pool(pool.clone());
  let record =
    normalize_record("CESSNA", "172S", json!({"description": "first identity document"}));
  import_record(&store, request('a', "1.0.0"), &record).await?;

  let demoted = query("UPDATE aircraft_core.variant_manufacturers SET is_primary = FALSE")
    .execute(&pool)
    .await?
    .rows_affected();
  assert_eq!(demoted, 1, "the first import must leave exactly one link to demote");

  let second =
    normalize_record("CESSNA", "172S", json!({"description": "second identity document"}));
  import_record(&store, request('b', "1.0.0"), &second).await?;

  let links: Vec<bool> =
    query_scalar("SELECT is_primary FROM aircraft_core.variant_manufacturers ORDER BY org_id")
      .fetch_all(&pool)
      .await?;
  assert_eq!(links, vec![true], "the standing link must be promoted, not duplicated or skipped");

  let published_manufacturer: Option<i64> =
    query_scalar("SELECT primary_manufacturer_id FROM aircraft_read.mv_variant_search")
      .fetch_one(&pool)
      .await?;
  assert!(
    published_manufacturer.is_some(),
    "the read model must serve the manufacturer the import resolved"
  );
  Ok(())
}
