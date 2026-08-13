use std::error::Error;

use aircraft_app::ingestion::{
    ArtifactDescriptor, ImportReport, ImportRequest, ImportStart, ImportStatus, IngestionStore,
    PreflightSummary, SourceDescriptor,
};
use aircraft_db::SqlxIngestionStore;
use aircraft_ingest::normalization::normalize_record;
use chrono::Utc;
use serde_json::json;
use sqlx_core::{query::query, query_scalar::query_scalar, raw_sql::raw_sql, row::Row};
use sqlx_postgres::{PgPool, PgPoolOptions};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ImageExt, runners::AsyncRunner},
};

const SCHEMA_STEPS: &[&str] = &[
    include_str!("../../../database/migrations/001_extensions_schemas_domains_triggers.sql"),
    include_str!("../../../database/migrations/002_core_reference_tables.sql"),
    include_str!("../../../database/seeds/001_reference_units.sql"),
    include_str!("../../../database/seeds/002_lookup_seed_data.sql"),
    include_str!("../../../database/migrations/003_geography_operators_organizations.sql"),
    include_str!("../../../database/migrations/004_aircraft_identity_taxonomy.sql"),
    include_str!("../../../database/migrations/005_certification_operating_approvals.sql"),
    include_str!("../../../database/migrations/006_dimensions_cabin_cargo_hangar_fit.sql"),
    include_str!("../../../database/migrations/007_weight_balance_payload_loading.sql"),
    include_str!("../../../database/migrations/008_performance_metrics_conditions.sql"),
    include_str!("../../../database/migrations/009_propulsion_engines_rotors_stcs.sql"),
    include_str!("../../../database/migrations/010_avionics_equipment_systems.sql"),
    include_str!("../../../database/migrations/011_military_sensors_stores_loadouts.sql"),
    include_str!("../../../database/migrations/012_ownership_cost_valuation_market.sql"),
    include_str!("../../../database/migrations/013_maintenance_reliability_supportability.sql"),
    include_str!("../../../database/migrations/014_sources_provenance_curation_audit.sql"),
    include_str!("../../../database/migrations/015_mission_profiles_comparison_scoring.sql"),
    include_str!("../../../database/migrations/016_read_models_views_indexes.sql"),
    include_str!("../../../database/migrations/017_rust_ingestion_adapter.sql"),
    include_str!("../../../database/validation/017_rust_ingestion_adapter_validation.sql"),
];

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn repository_preserves_ingestion_semantics_and_attempt_history() -> TestResult {
    let container = Postgres::default().with_tag("16-alpine").start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new().max_connections(5).connect(&database_url).await?;
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
            "performance": {"best_cruise_speed": "125 KIAS"}
        }),
    );
    import_record(&store, request('b', "1.1.0"), &second_record).await?;

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
    store
        .mark_failed(run_id, current_attempt_id, "TEST_CLEANUP", "integration test cleanup")
        .await?;
    Ok(())
}

async fn install_schema(pool: &PgPool) -> TestResult {
    for (index, sql) in SCHEMA_STEPS.iter().enumerate() {
        raw_sql(sql).execute(pool).await.map_err(|error| {
            std::io::Error::other(format!("schema step {} failed: {error:?}", index + 1))
        })?;
    }
    Ok(())
}

fn request(hash_character: char, parser_version: &str) -> ImportRequest {
    ImportRequest {
        source: SourceDescriptor {
            slug: "planephd".to_owned(),
            name: "PlanePHD".to_owned(),
            base_url: Some("https://planephd.com".to_owned()),
            parser_name: "planephd-json".to_owned(),
            parser_version: parser_version.to_owned(),
        },
        artifact: ArtifactDescriptor {
            content_sha256: hash_character.to_string().repeat(64),
            byte_length: 1,
            display_locator: "integration-fixture.json".to_owned(),
            captured_at: Utc::now(),
        },
        preflight: PreflightSummary {
            record_count: 1,
            warning_count: 0,
            record_keys_sha256: "d".repeat(64),
        },
    }
}

async fn import_record(
    store: &SqlxIngestionStore,
    request: ImportRequest,
    record: &aircraft_app::ingestion::PreparedAircraftRecord,
) -> TestResult {
    let (run_id, attempt_id, mut work) = ready(store.start_import(&request).await?)?;
    let outcome = work.stage_and_promote(record).await?;
    let report = ImportReport {
        schema_version: 1,
        run_id,
        attempt_id,
        status: ImportStatus::Succeeded,
        content_sha256: request.artifact.content_sha256,
        staged_records: u64::from(outcome.staged),
        promoted_records: u64::from(matches!(
            outcome.disposition,
            aircraft_app::ingestion::RecordDisposition::Promoted
        )),
        flagged_records: u64::from(matches!(
            outcome.disposition,
            aircraft_app::ingestion::RecordDisposition::Flagged
        )),
        skipped_records: 0,
        warning_count: outcome.warning_count,
        already_imported: false,
    };
    work.mark_succeeded(&report).await?;
    work.commit().await?;
    Ok(())
}

fn ready(
    start: ImportStart,
) -> TestResult<(i64, i64, Box<dyn aircraft_app::ingestion::IngestionUnitOfWork>)> {
    match start {
        ImportStart::Ready { run_id, attempt_id, unit_of_work } => {
            Ok((run_id, attempt_id, unit_of_work))
        }
        ImportStart::AlreadySucceeded(_) => {
            Err("test import unexpectedly already succeeded".into())
        }
        ImportStart::Busy => Err("test import unexpectedly reported busy".into()),
    }
}
