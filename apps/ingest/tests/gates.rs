// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Deployment gates for the `PlanePHD` ingestion adapter.
//!
//! `docs/architecture/rust_ingestion_adapter.md` makes production promotion
//! conditional on clean-database import, migration, transaction rollback,
//! idempotency, status-history, and snapshot regression coverage. These tests
//! drive the shipped `aircraft-ingest` binary against a disposable database
//! carrying the canonical schema; golden snapshots live in `cargo xtask
//! snapshots`.

use std::{
  path::{Path, PathBuf},
  process::Output,
  time::Duration,
};

use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use serde_json::Value;
use sqlx_core::{query::query, query_scalar::query_scalar, raw_sql::raw_sql, row::Row};
use sqlx_postgres::PgPool;

/// Repository-root fixture, resolved from the manifest so the test does not
/// depend on the working directory cargo happens to choose.
fn fixture() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/planephd_minimal.json")
}

fn import_with_alternate_cruise(database_url: &str) -> TestResult<Output> {
  let input = tempfile::NamedTempFile::new()?;
  let source = std::fs::read_to_string(fixture())?.replace("124 KIAS", "125 KIAS");
  std::fs::write(input.path(), source)?;
  run_cli(Some(database_url), &as_args(&import_args(input.path())))
}

/// Runs the shipped binary with a clean `APP__` environment plus the database
/// under test, so a developer's `.env` cannot influence the result.
fn run_cli(database_url: Option<&str>, args: &[&str]) -> TestResult<Output> {
  let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_aircraft-ingest"));
  for (key, _) in std::env::vars() {
    if key.starts_with("APP__") {
      command.env_remove(key);
    }
  }
  if let Some(url) = database_url {
    command.env("APP__INGEST__DATABASE_URL", url);
  }
  Ok(command.args(args).output()?)
}

fn import_args(input: &Path) -> Vec<String> {
  vec![
    "import".to_owned(),
    "--source".to_owned(),
    "planephd".to_owned(),
    "--input".to_owned(),
    input.display().to_string(),
    "--format".to_owned(),
    "json".to_owned(),
  ]
}

fn as_args(owned: &[String]) -> Vec<&str> {
  owned.iter().map(String::as_str).collect()
}

fn stdout_json(output: &Output) -> TestResult<Value> {
  Ok(serde_json::from_slice(&output.stdout)?)
}

fn describe(output: &Output) -> String {
  format!(
    "exit {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
    output.status.code(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr)
  )
}

/// `table` may include an inline `WHERE` clause and is interpolated verbatim, so
/// callers must pass only literal SQL.
async fn count(pool: &PgPool, table: &str) -> TestResult<i64> {
  Ok(query_scalar(&format!("SELECT COUNT(*) FROM {table}")).fetch_one(pool).await?)
}

/// Drives one curation decision through the shipped binary, so the exit codes
/// and rendering under test are the ones a caller actually scripts against.
fn curate(database_url: &str, decision: &str, assertion_id: i64) -> TestResult<Output> {
  run_cli(
    Some(database_url),
    &["curate", decision, "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )
}

/// The valuation price the read model currently serves, if any.
async fn served_price(pool: &PgPool) -> TestResult<Option<String>> {
  Ok(
    query_scalar("SELECT papi_price_usd::text FROM aircraft_read.mv_variant_search")
      .fetch_one(pool)
      .await?,
  )
}

/// Gate: clean-database import.
#[tokio::test]
async fn import_into_a_clean_database_promotes_the_fixture() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let args = import_args(&fixture());
  let output = run_cli(Some(&container.database_url), &as_args(&args))?;
  assert!(output.status.success(), "import should succeed: {}", describe(&output));

  let report = stdout_json(&output)?;
  assert_eq!(report["status"], "SUCCEEDED", "{report:#}");
  assert_eq!(report["promoted_records"], 1, "{report:#}");
  assert_eq!(report["already_imported"], false, "{report:#}");

  assert_eq!(count(&pool, "aircraft_core.variants").await?, 1);
  // Seed data already carries manufacturers, so assert the fixture's own
  // organization was resolved rather than counting the whole table.
  assert_eq!(
    count(&pool, "aircraft_org.organizations WHERE upper(name) = 'CESSNA'").await?,
    1,
    "the fixture manufacturer must resolve to exactly one organization"
  );
  assert!(count(&pool, "aircraft_specs.performance_metrics").await? > 0);
  assert!(count(&pool, "aircraft_prov.source_assertions").await? > 0);
  Ok(())
}

/// Everything this adapter writes is deliberately non-canonical and pending
/// (commit `bd68e46`).
///
/// `aircraft_read.mv_variant_search` selects from `aircraft_core.variants` and
/// applies `is_canonical` only inside its metric aggregates, so an ingested
/// variant *is* published to the read model while its measurements are withheld
/// until curation accepts them. This pins that split so neither side can start
/// publishing uncurated values by accident.
///
/// Weight metrics were exempt from that gate until migration 019 added
/// `is_canonical` to `aircraft_specs.weight_metrics`; asserting only the
/// performance columns here is what let that pass unnoticed.
#[tokio::test]
async fn imported_values_stay_pending_and_out_of_the_read_model() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let args = import_args(&fixture());
  let output = run_cli(Some(&container.database_url), &as_args(&args))?;
  assert!(output.status.success(), "{}", describe(&output));

  assert_eq!(
    count(&pool, "aircraft_specs.performance_metrics WHERE is_canonical").await?,
    0,
    "ingestion must not canonicalize its own performance values"
  );
  assert_eq!(
    count(&pool, "aircraft_specs.weight_metrics WHERE is_canonical").await?,
    0,
    "ingestion must not canonicalize its own weight values"
  );
  assert_eq!(
    count(&pool, "aircraft_prov.source_assertions WHERE is_accepted").await?,
    0,
    "ingestion must not accept its own assertions"
  );
  assert_eq!(
    count(&pool, "aircraft_read.mv_variant_search").await?,
    1,
    "the variant identity is published even while its values are pending"
  );
  // Every value column the matview exposes, not just the performance ones.
  // Weight metrics were ungated until migration 019 and market data until
  // migration 020; in both cases the columns this assertion omitted were exactly
  // the ones with no curation gate.
  assert_eq!(
    count(
      &pool,
      "aircraft_read.mv_variant_search \
             WHERE cruise_speed_kias IS NOT NULL OR range_nm IS NOT NULL \
                OR service_ceiling_ft IS NOT NULL OR rate_of_climb_fpm IS NOT NULL \
                OR stall_speed_kias IS NOT NULL OR takeoff_50ft_ft IS NOT NULL \
                OR landing_50ft_ft IS NOT NULL OR gross_weight_lb IS NOT NULL \
                OR empty_weight_lb IS NOT NULL OR fuel_capacity_gal IS NOT NULL \
                OR papi_price_usd IS NOT NULL OR for_sale_count IS NOT NULL \
                OR total_annual_cost_usd IS NOT NULL OR cost_per_hour_usd IS NOT NULL"
    )
    .await?,
    0,
    "no uncurated measurement may surface through the read model"
  );
  Ok(())
}

/// Gate: idempotency. A re-import of the same artifact is a replay, not a
/// second write.
#[tokio::test]
async fn reimporting_the_same_artifact_is_idempotent() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let args = import_args(&fixture());
  let first = run_cli(Some(&container.database_url), &as_args(&args))?;
  assert!(first.status.success(), "{}", describe(&first));
  let variants_after_first = count(&pool, "aircraft_core.variants").await?;
  let assertions_after_first = count(&pool, "aircraft_prov.source_assertions").await?;

  let second = run_cli(Some(&container.database_url), &as_args(&args))?;
  assert!(second.status.success(), "{}", describe(&second));

  let report = stdout_json(&second)?;
  assert_eq!(report["already_imported"], true, "replay must be reported as such: {report:#}");
  assert_eq!(
    count(&pool, "aircraft_core.variants").await?,
    variants_after_first,
    "a replay must not create additional variants"
  );
  assert_eq!(
    count(&pool, "aircraft_prov.source_assertions").await?,
    assertions_after_first,
    "a replay must not create additional assertions"
  );
  assert_eq!(
    count(&pool, "aircraft_ingest.ingest_runs").await?,
    1,
    "the same artifact is one logical run"
  );
  Ok(())
}

/// Gate: transaction rollback. A hard validation failure must leave no aircraft
/// data behind, while still committing its own audit trail.
#[tokio::test]
async fn a_hard_validation_failure_writes_no_aircraft_data_but_records_the_attempt() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  // Production years run backwards, which normalization reports as an
  // error-severity `INVALID_PRODUCTION_YEARS` issue.
  let directory = tempfile::tempdir()?;
  let invalid = directory.path().join("invalid.json");
  std::fs::write(
    &invalid,
    serde_json::to_vec(&serde_json::json!({
        "CESSNA": { "172S Skyhawk SP": { "start_year": 2006, "end_year": 1998 } }
    }))?,
  )?;

  let args = import_args(&invalid);
  let output = run_cli(Some(&container.database_url), &as_args(&args))?;
  assert_eq!(
    output.status.code(),
    Some(4),
    "validation failures use exit code 4: {}",
    describe(&output)
  );

  assert_eq!(count(&pool, "aircraft_core.variants").await?, 0);
  assert_eq!(count(&pool, "aircraft_ingest.staged_aircraft").await?, 0);
  assert_eq!(count(&pool, "aircraft_prov.source_assertions").await?, 0);

  let status: String =
    query_scalar("SELECT status FROM aircraft_ingest.ingest_runs").fetch_one(&pool).await?;
  assert_eq!(status, "VALIDATION_FAILED");
  let attempt: String =
    query_scalar("SELECT status FROM aircraft_ingest.ingest_run_attempts").fetch_one(&pool).await?;
  assert_eq!(attempt, "VALIDATION_FAILED", "the audit trail must survive the rollback");
  Ok(())
}

/// Gate: status history. The machine-readable status output is a contract.
#[tokio::test]
async fn status_reports_the_attempt_history_as_json() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let args = import_args(&fixture());
  let imported = run_cli(Some(&container.database_url), &as_args(&args))?;
  assert!(imported.status.success(), "{}", describe(&imported));

  let output =
    run_cli(Some(&container.database_url), &["status", "--limit", "20", "--format", "json"])?;
  assert!(output.status.success(), "{}", describe(&output));

  let document = stdout_json(&output)?;
  let runs = document["runs"].as_array().expect("status must report a runs array");
  assert_eq!(runs.len(), 1, "{document:#}");
  assert_eq!(runs[0]["status"], "SUCCEEDED", "{document:#}");
  assert_eq!(runs[0]["source_slug"], "planephd", "{document:#}");

  let attempts = runs[0]["attempts"].as_array().expect("a run must carry its attempts");
  assert_eq!(attempts.len(), 1, "{document:#}");
  assert_eq!(attempts[0]["attempt_number"], 1, "{document:#}");
  assert_eq!(attempts[0]["status"], "SUCCEEDED", "{document:#}");
  assert!(
    document["schema_version"].as_u64().is_some_and(|version| version >= 2),
    "status output must declare its schema version: {document:#}"
  );
  Ok(())
}

/// A retry after an interrupted import must close the stale `IMPORTING` attempt
/// rather than leaving two attempts open.
///
/// The run and attempt rows are written by a short transaction that commits
/// before the long staging transaction begins, so a process killed mid-import
/// leaves exactly this state behind: an `IMPORTING` run with an open attempt and
/// no staged rows, because the long transaction rolled back. The test seeds that
/// state directly rather than staging data first — staged rows that survived a
/// crash would be a different (and impossible) scenario.
#[tokio::test]
async fn a_retry_closes_a_stale_importing_attempt() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let fixture = fixture();
  let validated = run_cli(
    Some(&container.database_url),
    &[
      "validate",
      "--source",
      "planephd",
      "--input",
      &fixture.display().to_string(),
      "--format",
      "json",
    ],
  )?;
  assert!(validated.status.success(), "{}", describe(&validated));
  let content_sha256 = stdout_json(&validated)?["artifact"]["content_sha256"]
    .as_str()
    .expect("validate must report the artifact hash")
    .to_owned();

  let run_id: i64 = query_scalar(
    "INSERT INTO aircraft_ingest.ingest_runs(
            run_label,source_name,source_slug,content_sha256,parser_name,parser_version,
            input_byte_length,input_locator,status)
         VALUES('planephd_interrupted','PlanePHD','planephd',$1,'planephd-json','1.0.0',
            1,'interrupted.json','IMPORTING')
         RETURNING id",
  )
  .bind(&content_sha256)
  .fetch_one(&pool)
  .await?;
  query(
    "INSERT INTO aircraft_ingest.ingest_run_attempts(ingest_run_id,attempt_number,status)
         VALUES($1,1,'IMPORTING')",
  )
  .bind(run_id)
  .execute(&pool)
  .await?;

  let retry = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture)))?;
  assert!(retry.status.success(), "the retry must succeed: {}", describe(&retry));

  let rows = query(
    "SELECT status, failure_code FROM aircraft_ingest.ingest_run_attempts
         ORDER BY attempt_number",
  )
  .fetch_all(&pool)
  .await?;
  assert_eq!(rows.len(), 2, "the retry must create a second attempt");
  assert_eq!(rows[0].get::<String, _>("status"), "FAILED", "the stale attempt must be closed");
  assert_eq!(
    rows[0].get::<Option<String>, _>("failure_code").as_deref(),
    Some("PROCESS_TERMINATED")
  );
  assert_eq!(rows[1].get::<String, _>("status"), "SUCCEEDED");
  assert_eq!(
    count(&pool, "aircraft_ingest.ingest_runs").await?,
    1,
    "the retry must reuse the interrupted logical run"
  );
  Ok(())
}

/// The documented exit codes are a scripting contract; nothing pinned them.
#[tokio::test]
async fn documented_exit_codes_hold() -> TestResult {
  let missing_configuration = run_cli(None, &as_args(&import_args(&fixture())))?;
  assert_eq!(
    missing_configuration.status.code(),
    Some(2),
    "a missing database URL is a configuration failure: {}",
    describe(&missing_configuration)
  );

  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let absent = Path::new("/nonexistent/planephd.json");
  let unreadable_input = run_cli(Some(&container.database_url), &as_args(&import_args(absent)))?;
  assert_eq!(
    unreadable_input.status.code(),
    Some(3),
    "an unreadable artifact is a capture failure: {}",
    describe(&unreadable_input)
  );
  Ok(())
}

/// Curation is the step that makes ingested data visible. Accepting one
/// assertion must publish exactly that value and leave its siblings pending.
#[tokio::test]
async fn curating_an_assertion_publishes_only_that_value() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let imported = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(imported.status.success(), "{}", describe(&imported));
  let alternate = import_with_alternate_cruise(&container.database_url)?;
  assert!(alternate.status.success(), "{}", describe(&alternate));

  let listed = run_cli(
    Some(&container.database_url),
    &["curate", "list", "--limit", "50", "--format", "json"],
  )?;
  assert!(listed.status.success(), "{}", describe(&listed));
  let pending = stdout_json(&listed)?;
  let rows = pending["pending"].as_array().expect("curate list must report an array");
  assert!(!rows.is_empty(), "ingestion must leave assertions pending: {pending:#}");

  let cruise = rows
    .iter()
    .find(|row| {
      row["field_name"] == "performance.SPEED_CRUISE_BEST" && row["raw_value"] == "124 KIAS"
    })
    .expect("the fixture asserts a cruise speed");
  let assertion_id = cruise["assertion_id"].as_i64().expect("assertion id");

  let accepted = run_cli(
    Some(&container.database_url),
    &["curate", "accept", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert!(accepted.status.success(), "{}", describe(&accepted));
  let outcome = stdout_json(&accepted)?;
  assert_eq!(outcome["decision"], "ACCEPTED", "{outcome:#}");
  assert_eq!(outcome["measurement_canonicalized"], true, "{outcome:#}");
  assert_eq!(outcome["read_model_refreshed"], true, "{outcome:#}");

  // Exactly the accepted value is published; everything else stays withheld.
  let published: (Option<String>, Option<String>, Option<String>) = {
    let row = query(
      "SELECT cruise_speed_kias::text AS cruise, range_nm::text AS range,
                    gross_weight_lb::text AS gross
             FROM aircraft_read.mv_variant_search",
    )
    .fetch_one(&pool)
    .await?;
    (row.get("cruise"), row.get("range"), row.get("gross"))
  };
  assert!(
    published.0.as_deref().is_some_and(|value| value.starts_with("124")),
    "the accepted cruise speed must be served: {published:?}"
  );
  assert!(published.1.is_none(), "an unaccepted sibling must stay withheld");
  assert!(published.2.is_none(), "an unaccepted weight must stay withheld");

  // Accepting twice is not a silent no-op; the state machine rejects it.
  let repeated = run_cli(
    Some(&container.database_url),
    &["curate", "accept", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert_eq!(
    repeated.status.code(),
    Some(8),
    "a second decision on the same assertion must fail: {}",
    describe(&repeated)
  );
  Ok(())
}

/// Rejecting one pending source must not withdraw a different source's accepted
/// measurement for the same aircraft field.
#[tokio::test]
async fn rejecting_a_pending_sibling_keeps_the_accepted_measurement_published() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let imported = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(imported.status.success(), "{}", describe(&imported));
  let alternate = import_with_alternate_cruise(&container.database_url)?;
  assert!(alternate.status.success(), "{}", describe(&alternate));

  let assertions = query(
    "SELECT id, raw_value FROM aircraft_prov.source_assertions
         WHERE field_name = 'performance.SPEED_CRUISE_BEST'",
  )
  .fetch_all(&pool)
  .await?;
  let assertion_id = |raw_value: &str| {
    assertions
      .iter()
      .find(|row| row.get::<String, _>("raw_value") == raw_value)
      .map(|row| row.get::<i64, _>("id"))
      .expect("both cruise assertions must exist")
  };

  let accepted = run_cli(
    Some(&container.database_url),
    &[
      "curate",
      "accept",
      "--assertion-id",
      &assertion_id("124 KIAS").to_string(),
      "--format",
      "json",
    ],
  )?;
  assert!(accepted.status.success(), "{}", describe(&accepted));

  let rejected = run_cli(
    Some(&container.database_url),
    &[
      "curate",
      "reject",
      "--assertion-id",
      &assertion_id("125 KIAS").to_string(),
      "--format",
      "json",
    ],
  )?;
  assert!(rejected.status.success(), "{}", describe(&rejected));

  let published: Option<String> =
    query_scalar("SELECT cruise_speed_kias::text FROM aircraft_read.mv_variant_search")
      .fetch_one(&pool)
      .await?;
  assert!(
    published.as_deref().is_some_and(|value| value.starts_with("124")),
    "rejecting a pending sibling must leave the accepted value served: {published:?}"
  );
  Ok(())
}

/// Rejecting withdraws a previously published value, so a curator can undo.
#[tokio::test]
async fn rejecting_an_accepted_assertion_withdraws_it_from_the_read_model() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let imported = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(imported.status.success(), "{}", describe(&imported));

  let assertion_id: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE field_name = 'weight.WEIGHT_MTOW'",
  )
  .fetch_one(&pool)
  .await?;

  let accepted = run_cli(
    Some(&container.database_url),
    &["curate", "accept", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert!(accepted.status.success(), "{}", describe(&accepted));
  assert_eq!(count(&pool, "aircraft_specs.weight_metrics WHERE is_canonical").await?, 1);

  let published: Option<String> =
    query_scalar("SELECT gross_weight_lb::text FROM aircraft_read.mv_variant_search")
      .fetch_one(&pool)
      .await?;
  assert!(published.is_some(), "the accepted weight must be served before withdrawal");

  let rejected = run_cli(
    Some(&container.database_url),
    &["curate", "reject", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert!(rejected.status.success(), "a curator must be able to withdraw: {}", describe(&rejected));
  let outcome = stdout_json(&rejected)?;
  assert_eq!(outcome["decision"], "REJECTED", "{outcome:#}");
  assert_eq!(outcome["measurement_canonicalized"], false, "{outcome:#}");

  assert_eq!(
    count(&pool, "aircraft_specs.weight_metrics WHERE is_canonical").await?,
    0,
    "withdrawal must clear the canonical flag"
  );
  let withdrawn: Option<String> =
    query_scalar("SELECT gross_weight_lb::text FROM aircraft_read.mv_variant_search")
      .fetch_one(&pool)
      .await?;
  assert!(withdrawn.is_none(), "a withdrawn value must leave the read model");

  // Repeating the same decision is refused, so an unchanged state is never
  // reported as a successful change.
  let repeated = run_cli(
    Some(&container.database_url),
    &["curate", "reject", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert_eq!(
    repeated.status.code(),
    Some(8),
    "repeating a decision must fail: {}",
    describe(&repeated)
  );
  Ok(())
}

/// `DUPLICATE_SOURCE_RECORD_KEY` cannot live in the parity fixtures: it is
/// error-severity, so it aborts preflight and there is nothing for the two
/// loaders to compare. It still needs coverage, because the duplicate check is
/// what stops one record silently overwriting another.
#[tokio::test]
async fn a_duplicated_source_record_key_is_rejected_before_anything_is_written() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  // The same manufacturer/aircraft pair twice: a JSON object may repeat a key,
  // and the streaming parser visits both.
  let directory = tempfile::tempdir()?;
  let duplicated = directory.path().join("duplicated.json");
  std::fs::write(
    &duplicated,
    br#"{"CESSNA": {"172S Skyhawk SP": {"title": "first"}, "172S Skyhawk SP": {"title": "second"}}}"#,
  )?;

  let output = run_cli(Some(&container.database_url), &as_args(&import_args(&duplicated)))?;
  assert_eq!(
    output.status.code(),
    Some(4),
    "a duplicate source-record key is a validation failure: {}",
    describe(&output)
  );
  assert!(
    String::from_utf8_lossy(&output.stderr).contains("DUPLICATE_SOURCE_RECORD_KEY"),
    "the failure must name the code: {}",
    describe(&output)
  );
  assert_eq!(count(&pool, "aircraft_core.variants").await?, 0);
  assert_eq!(count(&pool, "aircraft_ingest.staged_aircraft").await?, 0);
  Ok(())
}

/// Market data is gated at the snapshot, not the field: one `is_canonical` flag
/// covers every column of a valuation and every line item of a cost snapshot,
/// which migration 020 records as "Published as a unit". A unit that publishes
/// together has to be curated together, so the snapshot appears only once every
/// field asserted on it is accepted and disappears when any one is withdrawn.
/// The half-curated assertions are the load-bearing part: publishing on the
/// first acceptance served an unaccepted `for_sale_count` and every pending
/// sibling cost.
#[tokio::test]
async fn a_market_snapshot_is_published_only_once_every_field_on_it_is_accepted() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let imported = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(imported.status.success(), "{}", describe(&imported));

  // Provenance for the price now exists; before migration 020 it did not.
  let price_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'VALUATION' AND field_name = 'papi_price_estimate'",
  )
  .fetch_one(&pool)
  .await?;

  assert!(served_price(&pool).await?.is_none(), "an uncurated price must not be served");

  // The fixture's valuation also carries a for_sale_count, so the price alone
  // does not complete the snapshot.
  let accepted_price = curate(&container.database_url, "accept", price_assertion)?;
  assert!(accepted_price.status.success(), "{}", describe(&accepted_price));
  assert!(
    served_price(&pool).await?.is_none(),
    "a snapshot with a pending for_sale_count must stay unpublished"
  );

  let for_sale_assertion: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'VALUATION' AND field_name = 'for_sale_count'",
  )
  .fetch_one(&pool)
  .await?;
  let accepted_count = curate(&container.database_url, "accept", for_sale_assertion)?;
  assert!(accepted_count.status.success(), "{}", describe(&accepted_count));
  let published_price = served_price(&pool).await?;
  assert!(
    published_price.as_deref().is_some_and(|value| value.starts_with("285000")),
    "the completed valuation must be served: {published_price:?}"
  );

  // The same rule over a snapshot with more than two fields: the fixture's four
  // ownership-cost entries publish on the last acceptance, not the first.
  let cost_assertions: Vec<i64> = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'COST_SNAPSHOT' ORDER BY id",
  )
  .fetch_all(&pool)
  .await?;
  assert!(cost_assertions.len() > 1, "the fixture must assert several cost fields to gate on");
  let last = cost_assertions.len() - 1;
  for (index, assertion_id) in cost_assertions.iter().copied().enumerate() {
    let accepted = curate(&container.database_url, "accept", assertion_id)?;
    assert!(accepted.status.success(), "{}", describe(&accepted));
    let published = count(&pool, "aircraft_market.cost_snapshots WHERE is_canonical").await?;
    assert_eq!(
      published,
      i64::from(index == last),
      "cost snapshot after accepting assertion {} of {}",
      index + 1,
      cost_assertions.len()
    );
  }

  // Withdrawing any one of them takes the whole snapshot back down.
  let withdrawn = curate(&container.database_url, "reject", cost_assertions[0])?;
  assert!(withdrawn.status.success(), "{}", describe(&withdrawn));
  assert_eq!(
    count(&pool, "aircraft_market.cost_snapshots WHERE is_canonical").await?,
    0,
    "withdrawing one accepted assertion must unpublish the snapshot it published"
  );
  Ok(())
}

/// `curation_failure` in `apps/ingest/src/main.rs` maps every decision the
/// current state does not permit to exit code 8. `SNAPSHOT_CONFLICT` and
/// `VALUE_MISMATCH` were only ever asserted at the repository boundary, so the
/// contract a caller actually scripts against went unexercised.
#[tokio::test]
async fn a_market_decision_the_state_refuses_exits_eight() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let imported = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(imported.status.success(), "{}", describe(&imported));

  // A second artifact the same day reuses the stored valuation -- the stored
  // price stays $285,000 -- while asserting its own $295,000 against it.
  let reprice = tempfile::NamedTempFile::new()?;
  std::fs::write(
    reprice.path(),
    std::fs::read_to_string(fixture())?.replace("285,000", "295,000"),
  )?;
  let repriced = run_cli(Some(&container.database_url), &as_args(&import_args(reprice.path())))?;
  assert!(repriced.status.success(), "{}", describe(&repriced));

  let mismatched: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE entity_type_code = 'VALUATION' AND field_name = 'papi_price_estimate'
           AND asserted_numeric = 295000",
  )
  .fetch_one(&pool)
  .await?;
  let refused = curate(&container.database_url, "accept", mismatched)?;
  assert_eq!(
    refused.status.code(),
    Some(8),
    "a value mismatch must exit 8: {}",
    describe(&refused)
  );

  // Publish the standing valuation, then offer the next day's snapshot for the
  // same variant: uq_val_canonical permits only one.
  for field in ["papi_price_estimate", "for_sale_count"] {
    let assertion: i64 = query_scalar(
      "SELECT id FROM aircraft_prov.source_assertions
           WHERE entity_type_code = 'VALUATION' AND field_name = $1
             AND asserted_numeric <> 295000
           ORDER BY id LIMIT 1",
    )
    .bind(field)
    .fetch_one(&pool)
    .await?;
    let accepted = curate(&container.database_url, "accept", assertion)?;
    assert!(accepted.status.success(), "{}", describe(&accepted));
  }

  let contender: i64 = query_scalar(
    "WITH snapshot AS (
             INSERT INTO aircraft_market.valuations(
                 variant_id, snapshot_date, source_name, papi_price_estimate,
                 currency_code, captured_at)
             SELECT variant_id, CURRENT_DATE + 1, 'PlanePHD', 305000, 'USD', now()
             FROM aircraft_market.valuations LIMIT 1
             RETURNING id)
         INSERT INTO aircraft_prov.source_assertions(
             source_document_id, entity_type_code, entity_id, field_name,
             raw_value, asserted_numeric, status_code, is_accepted)
         SELECT document.id, 'VALUATION', snapshot.id, 'papi_price_estimate',
                '$305,000', 305000, 'PENDING', FALSE
         FROM snapshot,
              (SELECT id FROM aircraft_prov.source_documents ORDER BY id LIMIT 1) AS document
         RETURNING id",
  )
  .fetch_one(&pool)
  .await?;
  let conflicted = curate(&container.database_url, "accept", contender)?;
  assert_eq!(
    conflicted.status.code(),
    Some(8),
    "a snapshot conflict must exit 8: {}",
    describe(&conflicted)
  );
  Ok(())
}

/// `uq_val_variant_date_source` allows one valuation per (variant, date,
/// source), so a second artifact imported the same day conflicts. Skipping the
/// conflicting row dropped the later document's price evidence entirely, which
/// left nothing for curation to accept.
#[tokio::test]
async fn a_second_same_day_import_still_asserts_its_valuation() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let first = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(first.status.success(), "{}", describe(&first));
  let second = import_with_alternate_cruise(&container.database_url)?;
  assert!(second.status.success(), "{}", describe(&second));

  assert_eq!(
    count(&pool, "aircraft_market.valuations").await?,
    1,
    "the unique index still permits one valuation per variant, date and source"
  );
  assert_eq!(
    count(
      &pool,
      "aircraft_prov.source_assertions
             WHERE entity_type_code = 'VALUATION' AND field_name = 'papi_price_estimate'"
    )
    .await?,
    2,
    "both documents must leave their price as curateable evidence"
  );

  let dangling: i64 = query_scalar(
    "SELECT COUNT(*) FROM aircraft_prov.source_assertions sa
         WHERE sa.entity_type_code = 'VALUATION'
           AND NOT EXISTS (
             SELECT 1 FROM aircraft_market.valuations v WHERE v.id = sa.entity_id)",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(dangling, 0, "every valuation assertion must address the surviving row");
  Ok(())
}

/// `variants.slug` is UNIQUE while the record's identity is `ingest_key`, so two
/// source names that normalize to one slug used to raise a constraint the
/// ON CONFLICT clause does not cover and roll the whole import back.
#[tokio::test]
async fn colliding_slugs_are_kept_as_flagged_evidence() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  // "172S/Skyhawk SP" and "172S Skyhawk SP" are distinct source records with
  // distinct record keys, but they slugify identically.
  let mut document: Value = serde_json::from_slice(&std::fs::read(fixture())?)?;
  let manufacturer = document
    .get_mut("CESSNA")
    .and_then(Value::as_object_mut)
    .expect("the fixture nests records under a manufacturer");
  let record = manufacturer["172S Skyhawk SP"].clone();
  manufacturer.insert("172S/Skyhawk SP".to_owned(), record);
  let input = tempfile::NamedTempFile::new()?;
  std::fs::write(input.path(), serde_json::to_vec(&document)?)?;

  let output = run_cli(Some(&container.database_url), &as_args(&import_args(input.path())))?;
  assert!(output.status.success(), "an ambiguous name is evidence: {}", describe(&output));

  assert_eq!(
    count(&pool, "aircraft_core.variants").await?,
    2,
    "both source records must survive rather than abort the batch"
  );
  let slugs: Vec<String> =
    query_scalar("SELECT slug FROM aircraft_core.variants ORDER BY id").fetch_all(&pool).await?;
  assert_eq!(slugs[0], "cessna-172s-skyhawk-sp-v1", "the first record keeps the plain slug");
  assert!(
    slugs[1].starts_with("cessna-172s-skyhawk-sp-v1-"),
    "the second record is disambiguated, not merged: {slugs:?}"
  );
  assert_eq!(
    count(&pool, "aircraft_prov.curation_flags WHERE issue_type = 'SLUG_COLLISION'").await?,
    1,
    "the ambiguity must reach a curator"
  );
  Ok(())
}

/// A curation decision commits before the read model is rebuilt, so the rebuild
/// can fail with the decision already durable. The decision must survive, the
/// stale read model must be recorded, and a retry must be able to rebuild it
/// without a second decision — which the state machine would refuse anyway.
#[tokio::test]
async fn a_failed_refresh_after_a_committed_decision_stays_retryable() -> TestResult {
  let (container, pool) = start_postgres(5, Duration::from_secs(5)).await?;
  install_schema(&pool).await?;

  let imported = run_cli(Some(&container.database_url), &as_args(&import_args(&fixture())))?;
  assert!(imported.status.success(), "{}", describe(&imported));

  let assertion_id: i64 = query_scalar(
    "SELECT id FROM aircraft_prov.source_assertions
         WHERE field_name = 'performance.SPEED_CRUISE_BEST' AND status_code = 'PENDING'
         ORDER BY id LIMIT 1",
  )
  .fetch_one(&pool)
  .await?;

  // Take the matview out from under refresh_search_matviews. Renaming makes the
  // refresh fail exactly the way a lock or statement timeout would, and unlike a
  // drop it can be undone so the retry has something to rebuild.
  raw_sql("ALTER MATERIALIZED VIEW aircraft_read.mv_variant_search RENAME TO mv_search_offline")
    .execute(&pool)
    .await?;

  let accepted = run_cli(
    Some(&container.database_url),
    &["curate", "accept", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert!(
    accepted.status.success(),
    "a committed decision must not be reported as a failure: {}",
    describe(&accepted)
  );
  let outcome = stdout_json(&accepted)?;
  assert_eq!(outcome["decision"], "ACCEPTED", "{outcome:#}");
  assert_eq!(outcome["read_model_refreshed"], false, "{outcome:#}");
  assert_eq!(outcome["read_model_refresh_pending"], true, "{outcome:#}");

  // The decision itself is durable, and repeating it is still refused.
  assert_eq!(
    count(
      &pool,
      "aircraft_specs.performance_metrics WHERE is_canonical
           AND metric_type_code = 'SPEED_CRUISE_BEST'"
    )
    .await?,
    1,
    "the accepted measurement must stay canonical after the failed refresh"
  );
  let repeated = run_cli(
    Some(&container.database_url),
    &["curate", "accept", "--assertion-id", &assertion_id.to_string(), "--format", "json"],
  )?;
  assert_eq!(repeated.status.code(), Some(8), "{}", describe(&repeated));

  // The staleness is recorded, with the failure attributed to it.
  let outstanding: (i64, i64, Option<String>) = {
    let row = query(
      "SELECT count(*) AS pending, coalesce(max(attempts), 0) AS attempts,
                  max(last_error) AS last_error
           FROM aircraft_read.read_model_refresh_requests
           WHERE status_code = 'PENDING'",
    )
    .fetch_one(&pool)
    .await?;
    (row.get("pending"), row.get("attempts"), row.get("last_error"))
  };
  assert_eq!(outstanding.0, 1, "the unfinished refresh must be recorded");
  assert_eq!(outstanding.1, 1, "the failed attempt must be counted");
  assert!(outstanding.2.is_some(), "the failure must be attributed to the request");

  raw_sql("ALTER MATERIALIZED VIEW aircraft_read.mv_search_offline RENAME TO mv_variant_search")
    .execute(&pool)
    .await?;

  let refreshed =
    run_cli(Some(&container.database_url), &["curate", "refresh", "--format", "json"])?;
  assert!(refreshed.status.success(), "{}", describe(&refreshed));
  let settled = stdout_json(&refreshed)?;
  assert_eq!(settled["read_model_refreshed"], true, "{settled:#}");
  assert_eq!(settled["requests_completed"], 1, "{settled:#}");

  assert_eq!(
    count(&pool, "aircraft_read.read_model_refresh_requests WHERE status_code = 'PENDING'").await?,
    0,
    "a successful rebuild closes the request it satisfied"
  );
  let published: Option<String> =
    query_scalar("SELECT cruise_speed_kias::text FROM aircraft_read.mv_variant_search")
      .fetch_one(&pool)
      .await?;
  assert!(
    published.as_deref().is_some_and(|value| value.starts_with("124")),
    "the retry must publish the decision the failed refresh withheld: {published:?}"
  );

  // Nothing outstanding: the retry is idempotent rather than an unconditional
  // rebuild, so it can be run safely without checking first.
  let again = run_cli(Some(&container.database_url), &["curate", "refresh", "--format", "json"])?;
  assert!(again.status.success(), "{}", describe(&again));
  assert_eq!(stdout_json(&again)?["read_model_refreshed"], false, "{}", describe(&again));
  Ok(())
}
