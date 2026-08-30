use aircraft_app::ingestion::{
  AttemptStatus, ImportReport, ImportRequest, ImportStart, ImportStatus, IngestionStore,
  IngestionUnitOfWork, IssueSeverity, PersistenceError, PreparedAircraftRecord,
  REPORT_SCHEMA_VERSION, RecordDisposition, RecordOutcome, RunStatus, StatusFilter,
};
use std::sync::LazyLock;

use aircraft_domain::ingestion::Confidence;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx_core::{
  error::{DatabaseError, Error as SqlxError},
  executor::Executor,
  query::query,
  query_scalar::query_scalar,
  row::Row,
  transaction::Transaction,
};
use sqlx_postgres::{PgPool, PgPoolOptions, Postgres};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Bound into every provenance-bearing insert below, so the confidence written
/// to `PostgreSQL` cannot drift from the domain's definition of a scraped source.
///
/// Bound as text and cast in SQL because `aircraft_ref.confidence_score` is
/// `NUMERIC(3, 2)`; every other numeric in this repository is bound the same way
/// rather than relying on an implicit float-to-numeric assignment cast.
static SCRAPED_SOURCE_CONFIDENCE: LazyLock<String> =
  LazyLock::new(|| format!("{:.2}", Confidence::SCRAPED_SOURCE.get()));

#[derive(Clone, Debug)]
pub struct SqlxIngestionStore {
  pool: PgPool,
  import_permits: std::sync::Arc<Semaphore>,
}

impl SqlxIngestionStore {
  pub async fn connect(
    database_url: &str,
    max_connections: u32,
    lock_timeout_seconds: u64,
    statement_timeout_seconds: u64,
  ) -> Result<Self, PersistenceError> {
    let lock_timeout = format!("{}ms", lock_timeout_seconds.saturating_mul(1_000));
    let statement_timeout = format!("{}ms", statement_timeout_seconds.saturating_mul(1_000));
    let pool = PgPoolOptions::new()
      .max_connections(max_connections.max(2))
      .after_connect(move |connection, _metadata| {
        let lock_timeout = lock_timeout.clone();
        let statement_timeout = statement_timeout.clone();
        Box::pin(async move {
          query(
            "SELECT set_config('lock_timeout',$1,FALSE),
                                set_config('statement_timeout',$2,FALSE)",
          )
          .bind(lock_timeout)
          .bind(statement_timeout)
          .execute(connection)
          .await?;
          Ok(())
        })
      })
      .connect(database_url)
      .await
      .map_err(database_error)?;
    Ok(Self::from_pool(pool))
  }

  /// The underlying pool, so sibling stores can share its configured lock and
  /// statement timeouts instead of opening a second, untimed one.
  #[must_use]
  pub const fn pool(&self) -> &PgPool {
    &self.pool
  }

  #[must_use]
  pub fn from_pool(pool: PgPool) -> Self {
    let max_connections = pool.options().get_max_connections();
    let concurrent_imports = (max_connections / 2).max(1) as usize;
    Self { pool, import_permits: std::sync::Arc::new(Semaphore::new(concurrent_imports)) }
  }

  async fn successful_report(
    executor: impl Executor<'_, Database = Postgres>,
    request: &ImportRequest,
  ) -> Result<Option<ImportReport>, PersistenceError> {
    let row = query(
      "SELECT r.id, a.id AS attempt_id, r.staged_aircraft,
                    r.promoted_aircraft, r.flagged_aircraft,
                    r.warning_count
             FROM aircraft_ingest.ingest_runs r
             LEFT JOIN LATERAL (
                 SELECT id FROM aircraft_ingest.ingest_run_attempts
                 WHERE ingest_run_id = r.id AND status = 'SUCCEEDED'
                 ORDER BY id DESC LIMIT 1
             ) a ON TRUE
             WHERE r.source_slug = $1 AND r.content_sha256 = $2
               AND r.parser_name = $3 AND r.parser_version = $4
               AND r.status = 'SUCCEEDED'",
    )
    .bind(&request.source.slug)
    .bind(&request.artifact.content_sha256)
    .bind(&request.source.parser_name)
    .bind(&request.source.parser_version)
    .fetch_optional(executor)
    .await
    .map_err(database_error)?;

    let Some(row) = row else {
      return Ok(None);
    };
    let attempt_id =
      row.try_get::<Option<i64>, _>("attempt_id").map_err(database_error)?.ok_or_else(|| {
        PersistenceError::Invariant(
          "successful Rust ingestion run has no successful attempt".to_owned(),
        )
      })?;
    Ok(Some(ImportReport {
      schema_version: REPORT_SCHEMA_VERSION,
      run_id: row.get("id"),
      attempt_id,
      status: ImportStatus::Succeeded,
      content_sha256: request.artifact.content_sha256.clone(),
      staged_records: decoded_count(&row, "staged_aircraft")?,
      promoted_records: decoded_count(&row, "promoted_aircraft")?,
      flagged_records: decoded_count(&row, "flagged_aircraft")?,
      warning_count: decoded_count(&row, "warning_count")?,
      already_imported: true,
    }))
  }
}

#[async_trait]
impl IngestionStore for SqlxIngestionStore {
  #[allow(clippy::too_many_lines)]
  async fn start_import(&self, request: &ImportRequest) -> Result<ImportStart, PersistenceError> {
    // Startup temporarily owns the long-running transaction connection and a
    // second connection for the durable audit transaction. Bound admitted
    // imports explicitly so pool acquisition cannot deadlock at any supported
    // pool size.
    let import_permit = self
      .import_permits
      .clone()
      .acquire_owned()
      .await
      .map_err(|_| PersistenceError::Invariant("import concurrency gate closed".to_owned()))?;
    let hash_prefix = request.artifact.content_sha256.get(..16).ok_or_else(|| {
      PersistenceError::Invariant("content hash is shorter than 16 characters".to_owned())
    })?;
    let mut transaction = self.pool.begin().await.map_err(database_error)?;

    let lock_key = format!(
      "{}:{}:{}:{}",
      request.source.slug,
      request.artifact.content_sha256,
      request.source.parser_name,
      request.source.parser_version
    );
    let locked: bool = query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
      .bind(lock_key)
      .fetch_one(&mut *transaction)
      .await
      .map_err(database_error)?;
    if !locked {
      transaction.rollback().await.map_err(database_error)?;
      return Ok(ImportStart::Busy);
    }

    if let Some(report) = Self::successful_report(&mut *transaction, request).await? {
      transaction.rollback().await.map_err(database_error)?;
      return Ok(ImportStart::AlreadySucceeded(report));
    }

    let run_label = format!(
      "{}_{}_{}",
      request.source.slug,
      hash_prefix,
      request.source.parser_version.replace('.', "_")
    );
    let mut audit = self.pool.begin().await.map_err(database_error)?;
    let run_id: i64 = query_scalar(
      "INSERT INTO aircraft_ingest.ingest_runs (
                 run_label, source_name, source_base_url, source_slug,
                 content_sha256, parser_name, parser_version,
                 input_byte_length, input_locator, status,
                 total_aircraft, warning_count
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'IMPORTING',$10,$11)
             ON CONFLICT (source_slug, content_sha256, parser_name, parser_version)
                 WHERE content_sha256 IS NOT NULL
             DO UPDATE SET started_at = now(), finished_at = NULL,
                 input_byte_length = EXCLUDED.input_byte_length,
                 input_locator = EXCLUDED.input_locator, status = 'IMPORTING',
                 failure_code = NULL, failure_message = NULL
             RETURNING id",
    )
    .bind(&run_label)
    .bind(&request.source.name)
    .bind(&request.source.base_url)
    .bind(&request.source.slug)
    .bind(&request.artifact.content_sha256)
    .bind(&request.source.parser_name)
    .bind(&request.source.parser_version)
    .bind(
      i64::try_from(request.artifact.byte_length)
        .map_err(|_| PersistenceError::Invariant("input byte length exceeds BIGINT".to_owned()))?,
    )
    .bind(&request.artifact.display_locator)
    .bind(
      i32::try_from(request.preflight.record_count)
        .map_err(|_| PersistenceError::Invariant("record count exceeds INT".to_owned()))?,
    )
    .bind(
      i32::try_from(request.preflight.warning_count)
        .map_err(|_| PersistenceError::Invariant("warning count exceeds INT".to_owned()))?,
    )
    .fetch_one(&mut *audit)
    .await
    .map_err(database_error)?;

    query(
            "UPDATE aircraft_ingest.ingest_run_attempts
             SET status = 'FAILED', finished_at = clock_timestamp(),
                 failure_code = 'PROCESS_TERMINATED',
                 failure_message = 'superseded by a later attempt after the previous process ended without finalizing'
             WHERE ingest_run_id = $1 AND status = 'IMPORTING'",
        )
        .bind(run_id)
        .execute(&mut *audit)
        .await
        .map_err(database_error)?;

    let attempt_id: i64 = query_scalar(
      "INSERT INTO aircraft_ingest.ingest_run_attempts (
                 ingest_run_id, attempt_number, status
             ) SELECT $1, COALESCE(MAX(attempt_number), 0) + 1, 'IMPORTING'
               FROM aircraft_ingest.ingest_run_attempts WHERE ingest_run_id = $1
             RETURNING id",
    )
    .bind(run_id)
    .fetch_one(&mut *audit)
    .await
    .map_err(database_error)?;
    audit.commit().await.map_err(database_error)?;

    Ok(ImportStart::Ready {
      run_id,
      attempt_id,
      unit_of_work: Box::new(SqlxIngestionUnitOfWork {
        transaction,
        _import_permit: import_permit,
        run_id,
        attempt_id,
        source: request.source.clone(),
        run_label,
      }),
    })
  }

  async fn mark_failed(
    &self,
    run_id: i64,
    attempt_id: i64,
    failure_code: &str,
    failure_message: &str,
  ) -> Result<(), PersistenceError> {
    let mut transaction = self.pool.begin().await.map_err(database_error)?;
    query(
      "UPDATE aircraft_ingest.ingest_runs
             SET status = 'FAILED', finished_at = clock_timestamp(),
                 failure_code = $2, failure_message = $3
             WHERE id = $1 AND status <> 'SUCCEEDED'",
    )
    .bind(run_id)
    .bind(failure_code)
    .bind(failure_message)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    query(
      "UPDATE aircraft_ingest.ingest_run_attempts
             SET status = 'FAILED', finished_at = clock_timestamp(),
                 failure_code = $2, failure_message = $3
             WHERE id = $1 AND status <> 'SUCCEEDED'",
    )
    .bind(attempt_id)
    .bind(failure_code)
    .bind(failure_message)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)
  }

  async fn record_validation_failure(
    &self,
    source: &aircraft_app::ingestion::SourceDescriptor,
    artifact: &aircraft_app::ingestion::ArtifactDescriptor,
    failure_code: &str,
    failure_message: &str,
  ) -> Result<(), PersistenceError> {
    let hash_prefix = artifact.content_sha256.get(..16).ok_or_else(|| {
      PersistenceError::Invariant("content hash is shorter than 16 characters".to_owned())
    })?;
    let run_label = format!(
      "{}_{}_{}_validation",
      source.slug,
      hash_prefix,
      source.parser_version.replace('.', "_")
    );
    let mut transaction = self.pool.begin().await.map_err(database_error)?;
    let lock_key = format!(
      "{}:{}:{}:{}",
      source.slug, artifact.content_sha256, source.parser_name, source.parser_version
    );
    query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
      .bind(lock_key)
      .execute(&mut *transaction)
      .await
      .map_err(database_error)?;
    let run_id: i64 = query_scalar(
            "INSERT INTO aircraft_ingest.ingest_runs(
                run_label,source_name,source_base_url,source_slug,content_sha256,
                parser_name,parser_version,input_byte_length,input_locator,status,
                failure_code,failure_message)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'VALIDATION_FAILED',$10,$11)
             ON CONFLICT(source_slug,content_sha256,parser_name,parser_version)
                WHERE content_sha256 IS NOT NULL
             DO UPDATE SET
                status=CASE WHEN aircraft_ingest.ingest_runs.status='SUCCEEDED'
                    THEN aircraft_ingest.ingest_runs.status ELSE 'VALIDATION_FAILED' END,
                finished_at=CASE WHEN aircraft_ingest.ingest_runs.status='SUCCEEDED'
                    THEN aircraft_ingest.ingest_runs.finished_at ELSE clock_timestamp() END,
                failure_code=CASE WHEN aircraft_ingest.ingest_runs.status='SUCCEEDED'
                    THEN aircraft_ingest.ingest_runs.failure_code ELSE EXCLUDED.failure_code END,
                failure_message=CASE WHEN aircraft_ingest.ingest_runs.status='SUCCEEDED'
                    THEN aircraft_ingest.ingest_runs.failure_message ELSE EXCLUDED.failure_message END
             RETURNING id",
        )
        .bind(run_label)
        .bind(&source.name)
        .bind(&source.base_url)
        .bind(&source.slug)
        .bind(&artifact.content_sha256)
        .bind(&source.parser_name)
        .bind(&source.parser_version)
        .bind(i64::try_from(artifact.byte_length).map_err(|_| {
            PersistenceError::Invariant("input byte length exceeds BIGINT".to_owned())
        })?)
        .bind(&artifact.display_locator)
        .bind(failure_code)
        .bind(failure_message)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;

    query(
      "INSERT INTO aircraft_ingest.ingest_run_attempts(
                ingest_run_id,attempt_number,status,finished_at,failure_code,failure_message)
             SELECT $1,COALESCE(MAX(attempt_number),0)+1,'VALIDATION_FAILED',now(),$2,$3
             FROM aircraft_ingest.ingest_run_attempts WHERE ingest_run_id=$1",
    )
    .bind(run_id)
    .bind(failure_code)
    .bind(failure_message)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)
  }

  async fn status(&self, filter: &StatusFilter) -> Result<Vec<RunStatus>, PersistenceError> {
    let limit = i64::from(if filter.limit == 0 { 20 } else { filter.limit.min(200) });
    let rows = query(
            "SELECT r.id, a.id AS attempt_id, COALESCE(r.source_slug, lower(r.source_name)) source_slug,
                    COALESCE(r.parser_name, 'legacy-sql') parser_name,
                    COALESCE(r.parser_version, 'legacy') parser_version,
                    COALESCE(r.content_sha256, '') content_sha256,
                    r.status, COALESCE(r.input_locator, r.json_file_path, '<unknown>') input_locator,
                    COALESCE(r.staged_aircraft, 0) staged_aircraft,
                    COALESCE(r.promoted_aircraft, 0) promoted_aircraft,
                    COALESCE(r.flagged_aircraft, 0) flagged_aircraft,
                    COALESCE(r.warning_count, 0) warning_count,
                    COALESCE(a.failure_code, r.failure_code) failure_code,
                    COALESCE(a.failure_message, r.failure_message) failure_message,
                    r.started_at, r.finished_at
             FROM aircraft_ingest.ingest_runs r
             LEFT JOIN LATERAL (
                 SELECT * FROM aircraft_ingest.ingest_run_attempts
                 WHERE ingest_run_id = r.id ORDER BY id DESC LIMIT 1
             ) a ON TRUE
             WHERE ($1::BIGINT IS NULL OR r.id = $1)
               AND ($2::TEXT IS NULL OR r.content_sha256 = $2)
             ORDER BY r.started_at DESC LIMIT $3"
        )
        .bind(filter.run_id)
        .bind(&filter.content_sha256)
        .bind(limit)
        .fetch_all(&self.pool).await.map_err(database_error)?;

    let mut statuses = rows.iter().map(row_to_status).collect::<Result<Vec<_>, _>>()?;
    let run_ids = statuses.iter().map(|status| status.run_id).collect::<Vec<_>>();
    if run_ids.is_empty() {
      return Ok(statuses);
    }
    let attempts = query(
      "SELECT id, ingest_run_id, attempt_number, status,
                    staged_aircraft, promoted_aircraft, flagged_aircraft,
                    warning_count, failure_code, failure_message,
                    started_at, finished_at
             FROM aircraft_ingest.ingest_run_attempts
             WHERE ingest_run_id = ANY($1)
             ORDER BY ingest_run_id, attempt_number DESC",
    )
    .bind(&run_ids)
    .fetch_all(&self.pool)
    .await
    .map_err(database_error)?;
    for row in &attempts {
      let run_id: i64 = row.get("ingest_run_id");
      let status = statuses.iter_mut().find(|status| status.run_id == run_id).ok_or_else(|| {
        PersistenceError::Invariant("attempt query returned an unknown ingestion run".to_owned())
      })?;
      status.attempts.push(row_to_attempt_status(row)?);
    }
    Ok(statuses)
  }
}

struct SqlxIngestionUnitOfWork {
  transaction: Transaction<'static, Postgres>,
  _import_permit: OwnedSemaphorePermit,
  run_id: i64,
  attempt_id: i64,
  source: aircraft_app::ingestion::SourceDescriptor,
  run_label: String,
}

#[async_trait]
impl IngestionUnitOfWork for SqlxIngestionUnitOfWork {
  async fn stage_and_promote(
    &mut self,
    record: &PreparedAircraftRecord,
  ) -> Result<RecordOutcome, PersistenceError> {
    let warning_count =
      record.issues.iter().filter(|issue| issue.severity == IssueSeverity::Warning).count() as u64;
    let staged_id = self.stage(record).await?;
    let variant_id = self.promote_identity(record).await?;
    let document_id = self.promote_document(record, variant_id).await?;
    self.promote_measurements(record, variant_id, document_id).await?;
    self.promote_engine(record, variant_id, document_id).await?;
    self.promote_market(record, variant_id, document_id).await?;
    self.promote_issues(record, variant_id).await?;
    let status = if warning_count == 0 { "PROMOTED" } else { "FLAGGED" };
    query(
      "UPDATE aircraft_ingest.staged_aircraft
             SET stage_status = $2, variant_id = $3, promoted_at = now()
             WHERE id = $1",
    )
    .bind(staged_id)
    .bind(status)
    .bind(variant_id)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    query(
      "UPDATE aircraft_prov.source_documents
             SET processing_status = 'PROCESSED' WHERE id = $1",
    )
    .bind(document_id)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    Ok(RecordOutcome {
      staged: true,
      disposition: if warning_count > 0 {
        RecordDisposition::Flagged
      } else {
        RecordDisposition::Promoted
      },
      warning_count,
    })
  }

  async fn refresh_read_models(&mut self) -> Result<(), PersistenceError> {
    query("SELECT aircraft_read.refresh_search_matviews(FALSE)")
      .execute(&mut *self.transaction)
      .await
      .map_err(database_error)?;
    Ok(())
  }

  async fn mark_succeeded(&mut self, report: &ImportReport) -> Result<(), PersistenceError> {
    query(
      "UPDATE aircraft_ingest.ingest_runs SET status='SUCCEEDED', finished_at=clock_timestamp(),
                 staged_aircraft=$2, promoted_aircraft=$3, flagged_aircraft=$4,
                 skipped_aircraft=$5, warning_count=$6 WHERE id=$1",
    )
    .bind(self.run_id)
    .bind(to_i32(report.staged_records)?)
    .bind(to_i32(report.promoted_records)?)
    .bind(to_i32(report.flagged_records)?)
    .bind(0_i32)
    .bind(to_i32(report.warning_count)?)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    query(
      "UPDATE aircraft_ingest.ingest_run_attempts
             SET status='SUCCEEDED', finished_at=clock_timestamp(), staged_aircraft=$2,
                 promoted_aircraft=$3, flagged_aircraft=$4, skipped_aircraft=$5,
                 warning_count=$6 WHERE id=$1",
    )
    .bind(self.attempt_id)
    .bind(to_i32(report.staged_records)?)
    .bind(to_i32(report.promoted_records)?)
    .bind(to_i32(report.flagged_records)?)
    .bind(0_i32)
    .bind(to_i32(report.warning_count)?)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    Ok(())
  }

  async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
    self.transaction.commit().await.map_err(database_error)
  }

  async fn rollback(self: Box<Self>) -> Result<(), PersistenceError> {
    self.transaction.rollback().await.map_err(database_error)
  }
}

impl SqlxIngestionUnitOfWork {
  async fn stage(&mut self, record: &PreparedAircraftRecord) -> Result<i64, PersistenceError> {
    let performance = serde_json::to_value(&record.performance).map_err(serialization_error)?;
    let weights = serde_json::to_value(&record.weights).map_err(serialization_error)?;
    let costs = serde_json::to_value(&record.operating_costs).map_err(serialization_error)?;
    let engine = serde_json::to_value(&record.propulsion).map_err(serialization_error)?;
    let issues = serde_json::to_value(&record.issues).map_err(serialization_error)?;
    let staged_id: i64 = query_scalar(
      "INSERT INTO aircraft_ingest.staged_aircraft (
                ingest_run_id, source_record_key, manufacturer_name_raw, aircraft_name_raw,
                source_link, page_url, title, description, papi_price_estimate_raw,
                for_sale_count_raw, start_year, end_year, in_production,
                performance_json, weights_json, ownership_costs_json, engine_json,
                raw_json, issues
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
             RETURNING id",
    )
    .bind(self.run_id)
    .bind(&record.source_record_key)
    .bind(&record.identity.manufacturer_name)
    .bind(&record.identity.aircraft_name)
    .bind(&record.identity.source_link)
    .bind(&record.identity.page_url)
    .bind(&record.identity.title)
    .bind(&record.identity.description)
    .bind(&record.valuation.papi_price_estimate)
    .bind(record.valuation.for_sale_count.map(|value| value.to_string()))
    .bind(record.lifecycle.production_start_year)
    .bind(record.lifecycle.production_end_year)
    .bind(record.lifecycle.is_in_production)
    .bind(performance)
    .bind(weights)
    .bind(costs)
    .bind(engine)
    .bind(&record.raw_document)
    .bind(issues)
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    for image in &record.images {
      query(
        "INSERT INTO aircraft_ingest.staged_images (
                    staged_aircraft_id, array_position, href_raw, href_resolved,
                    title, holder, dimensions_raw, width_px, height_px, is_primary
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
      )
      .bind(staged_id)
      .bind(image.array_position)
      .bind(&image.href_raw)
      .bind(&image.href_resolved)
      .bind(&image.title)
      .bind(&image.holder)
      .bind(&image.dimensions_raw)
      .bind(image.width_px)
      .bind(image.height_px)
      .bind(image.is_primary)
      .execute(&mut *self.transaction)
      .await
      .map_err(database_error)?;
    }
    Ok(staged_id)
  }

  async fn promote_identity(
    &mut self,
    record: &PreparedAircraftRecord,
  ) -> Result<i64, PersistenceError> {
    let organization = query_scalar::<_, i64>(
      "SELECT id FROM aircraft_org.organizations
             WHERE name_aliases @> ARRAY[$1] OR upper(name)=upper($1) LIMIT 1",
    )
    .bind(&record.identity.manufacturer_name)
    .fetch_optional(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    let organization = match organization {
      Some(id) => id,
      None => query_scalar(
        "INSERT INTO aircraft_org.organizations(name,slug,org_type_code,name_aliases)
                 VALUES (initcap($1),aircraft_ref.slugify($1),'MANUFACTURER',ARRAY[$1])
                 ON CONFLICT(slug) DO UPDATE SET name_aliases = (
                    SELECT ARRAY(SELECT DISTINCT unnest(
                        aircraft_org.organizations.name_aliases || EXCLUDED.name_aliases)))
                 RETURNING id",
      )
      .bind(&record.identity.manufacturer_name)
      .fetch_one(&mut *self.transaction)
      .await
      .map_err(database_error)?,
    };
    let family: i64 = query_scalar(
      "INSERT INTO aircraft_core.families(manufacturer_org_id,name,slug)
             VALUES($1,initcap($2),aircraft_ref.slugify($2 || '-family'))
             ON CONFLICT(slug) DO UPDATE SET manufacturer_org_id=EXCLUDED.manufacturer_org_id
             RETURNING id",
    )
    .bind(organization)
    .bind(&record.identity.manufacturer_name)
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    let model: i64 = query_scalar(
      "INSERT INTO aircraft_core.models(family_id,name,slug)
             VALUES($1,$2,aircraft_ref.slugify($3 || '-' || $2))
             ON CONFLICT(slug) DO UPDATE SET family_id=EXCLUDED.family_id RETURNING id",
    )
    .bind(family)
    .bind(&record.identity.aircraft_name)
    .bind(&record.identity.manufacturer_name)
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    // variants.slug is UNIQUE while the record's identity is ingest_key, so two
    // distinct source records whose names slugify identically ("A/B" and "A B")
    // would collide on a constraint the ON CONFLICT clause does not cover and
    // abort the whole import. Suffixing the second record's slug with a digest
    // of its own ingest_key keeps it — deterministically, so a replay resolves
    // to the same slug — and the curation flag below records that two source
    // names may name the same aircraft.
    let row = query(
      "INSERT INTO aircraft_core.variants(
                model_id,name,slug,description,production_start_year,production_end_year,
                is_in_production,passenger_capacity,crew_count,engine_count,source_path,ingest_key)
             VALUES($1,$2,
                (SELECT CASE
                    WHEN EXISTS (
                        SELECT 1 FROM aircraft_core.variants existing
                        WHERE existing.slug = candidate.slug
                          AND existing.ingest_key IS DISTINCT FROM $12)
                    THEN candidate.slug || '-' || substr(md5($12),1,8)
                    ELSE candidate.slug
                 END
                 FROM (SELECT aircraft_ref.slugify($3 || '-' || $2 || '-v1') AS slug)
                    AS candidate),
                $4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT(ingest_key) WHERE ingest_key IS NOT NULL DO UPDATE SET
                description=EXCLUDED.description,
                engine_count=COALESCE(aircraft_core.variants.engine_count,EXCLUDED.engine_count)
             RETURNING id,
                slug <> aircraft_ref.slugify($3 || '-' || $2 || '-v1') AS disambiguated,
                slug",
    )
    .bind(model)
    .bind(&record.identity.aircraft_name)
    .bind(&record.identity.manufacturer_name)
    .bind(&record.identity.description)
    .bind(record.lifecycle.production_start_year)
    .bind(record.lifecycle.production_end_year)
    .bind(record.lifecycle.is_in_production)
    .bind(record.lifecycle.passenger_capacity)
    .bind(record.lifecycle.crew_count)
    .bind(record.propulsion.engine_count)
    .bind(&record.identity.source_link)
    .bind(&record.source_record_key)
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    let variant_id: i64 = row.get("id");
    self.link_primary_manufacturer(variant_id, organization).await?;
    if row.get::<bool, _>("disambiguated") {
      let slug: String = row.get("slug");
      self.flag_slug_collision(variant_id, &slug).await?;
    }
    Ok(variant_id)
  }

  /// The first manufacturer recorded for a variant keeps the primary role, so a
  /// replay or a second source adds a link without displacing it. Migration 023
  /// backfills this projection for variants imported before it existed.
  async fn link_primary_manufacturer(
    &mut self,
    variant_id: i64,
    organization: i64,
  ) -> Result<(), PersistenceError> {
    query(
      "INSERT INTO aircraft_core.variant_manufacturers(
                variant_id,org_id,role,is_primary)
             SELECT $1,$2,'MANUFACTURER',TRUE
             WHERE NOT EXISTS (
                SELECT 1 FROM aircraft_core.variant_manufacturers
                WHERE variant_id=$1 AND is_primary)
             ON CONFLICT(variant_id,org_id) DO NOTHING",
    )
    .bind(variant_id)
    .bind(organization)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    Ok(())
  }

  /// An ambiguous identity is evidence, not a hard failure: the record is kept
  /// under a disambiguated slug and handed to curation to merge or keep apart.
  async fn flag_slug_collision(
    &mut self,
    variant_id: i64,
    slug: &str,
  ) -> Result<(), PersistenceError> {
    query(
      "INSERT INTO aircraft_prov.curation_flags(
                entity_type_code,entity_id,field_name,issue_type,
                issue_description,status_code,priority)
             SELECT 'AIRCRAFT_VARIANT',$1,'slug','SLUG_COLLISION',
                'Another variant already holds the slug this record normalizes to; '
                || 'it was stored as ' || $2 || '. Confirm whether the two records '
                || 'describe the same aircraft.','OPEN',2
             WHERE NOT EXISTS (
                SELECT 1 FROM aircraft_prov.curation_flags
                WHERE entity_type_code='AIRCRAFT_VARIANT' AND entity_id=$1
                  AND issue_type='SLUG_COLLISION' AND status_code='OPEN')",
    )
    .bind(variant_id)
    .bind(slug)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    Ok(())
  }

  async fn promote_document(
    &mut self,
    record: &PreparedAircraftRecord,
    variant_id: i64,
  ) -> Result<i64, PersistenceError> {
    let source_id: i64 = query_scalar(
      "INSERT INTO aircraft_prov.sources(name,slug,source_type_code,
                reliability_grade_code,base_url,default_confidence,notes)
             VALUES($1,$2,'SCRAPED_WEB','UNVERIFIED',$3,$4::numeric,
                'Values require corroboration before curation acceptance.')
             ON CONFLICT(slug) DO UPDATE SET base_url=EXCLUDED.base_url RETURNING id",
    )
    .bind(&self.source.name)
    .bind(&self.source.slug)
    .bind(&self.source.base_url)
    .bind(SCRAPED_SOURCE_CONFIDENCE.as_str())
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    let document_id: i64 = query_scalar(
      "INSERT INTO aircraft_prov.source_documents(
                source_id,variant_id,source_system_key,source_url,source_path,
                raw_json,ingest_batch_label,parser_version,ingest_run_id)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
             RETURNING id",
    )
    .bind(source_id)
    .bind(variant_id)
    .bind(&record.provenance.source_system_key)
    .bind(&record.provenance.source_url)
    .bind(&record.provenance.source_path)
    .bind(&record.raw_document)
    .bind(&self.run_label)
    .bind(&self.source.parser_version)
    .bind(self.run_id)
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    self
      .assert_value(document_id, variant_id, "name", &record.identity.aircraft_name, None, None)
      .await?;
    Ok(document_id)
  }

  async fn promote_measurements(
    &mut self,
    record: &PreparedAircraftRecord,
    variant_id: i64,
    document_id: i64,
  ) -> Result<(), PersistenceError> {
    for metric in &record.performance.measurements {
      let field =
        format!("performance.{}", metric.metric_code.as_deref().unwrap_or(&metric.source_field));
      let assertion_id = self
        .assert_entity_value(
          document_id,
          "AIRCRAFT_VARIANT",
          variant_id,
          &field,
          &metric.raw_value,
          metric.numeric_value.as_deref(),
          metric.raw_unit.as_deref(),
        )
        .await?;
      if let (Some(code), Some(value), Some(unit)) =
        (&metric.metric_code, &metric.numeric_value, &metric.unit_code)
      {
        query(
          "INSERT INTO aircraft_specs.performance_metrics(
                        variant_id,metric_type_code,raw_value,raw_unit_code,canonical_value,
                        is_canonical,confidence,source_assertion_id)
                     SELECT $1,$2,$3::numeric,$4,
                        trim_scale(aircraft_ref.to_canonical($3::numeric,$4)),
                        FALSE,$5::numeric,$6",
        )
        .bind(variant_id)
        .bind(code)
        .bind(value)
        .bind(unit)
        .bind(SCRAPED_SOURCE_CONFIDENCE.as_str())
        .bind(assertion_id)
        .execute(&mut *self.transaction)
        .await
        .map_err(database_error)?;
      }
    }
    for metric in &record.weights.measurements {
      let field =
        format!("weight.{}", metric.metric_code.as_deref().unwrap_or(&metric.source_field));
      let assertion_id = self
        .assert_entity_value(
          document_id,
          "AIRCRAFT_VARIANT",
          variant_id,
          &field,
          &metric.raw_value,
          metric.numeric_value.as_deref(),
          metric.raw_unit.as_deref(),
        )
        .await?;
      if let (Some(code), Some(value), Some(unit)) =
        (&metric.metric_code, &metric.numeric_value, &metric.unit_code)
      {
        query(
          "INSERT INTO aircraft_specs.weight_metrics(
                        variant_id,metric_type_code,raw_value,raw_unit_code,canonical_value,
                        confidence,source_assertion_id)
                     VALUES($1,$2,$3::numeric,$4,
                        trim_scale(aircraft_ref.to_canonical($3::numeric,$4)),$5::numeric,$6)",
        )
        .bind(variant_id)
        .bind(code)
        .bind(value)
        .bind(unit)
        .bind(SCRAPED_SOURCE_CONFIDENCE.as_str())
        .bind(assertion_id)
        .execute(&mut *self.transaction)
        .await
        .map_err(database_error)?;
      }
    }
    Ok(())
  }

  async fn promote_engine(
    &mut self,
    record: &PreparedAircraftRecord,
    variant_id: i64,
    document_id: i64,
  ) -> Result<(), PersistenceError> {
    let engine = &record.propulsion;
    let Some(model) = engine.model.as_deref() else { return Ok(()) };
    let manufacturer = engine.manufacturer.as_deref().unwrap_or("unknown");
    let engine_id: i64 = query_scalar(
      "INSERT INTO aircraft_power.engine_variants(
                slug,manufacturer_name_raw,model_designation,hp_rated,
                rated_thrust_n,thrust_lbf_dry,tbo_hours,tbo_years)
             VALUES(aircraft_ref.slugify($1 || '-' || $2),$1,$2,$3::numeric,$4::numeric,
                CASE WHEN $4::text IS NULL THEN NULL ELSE round($4::numeric*0.224809,2) END,$5,$6)
             ON CONFLICT(slug) DO UPDATE SET
                hp_rated=COALESCE(EXCLUDED.hp_rated,aircraft_power.engine_variants.hp_rated),
                rated_thrust_n=COALESCE(EXCLUDED.rated_thrust_n,
                    aircraft_power.engine_variants.rated_thrust_n),
                thrust_lbf_dry=COALESCE(EXCLUDED.thrust_lbf_dry,
                    aircraft_power.engine_variants.thrust_lbf_dry),
                tbo_hours=COALESCE(EXCLUDED.tbo_hours,aircraft_power.engine_variants.tbo_hours),
                tbo_years=COALESCE(EXCLUDED.tbo_years,aircraft_power.engine_variants.tbo_years)
             RETURNING id",
    )
    .bind(manufacturer)
    .bind(model)
    .bind(&engine.horsepower)
    .bind(&engine.thrust_newtons)
    .bind(engine.tbo_hours)
    .bind(engine.tbo_years)
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    query(
      "INSERT INTO aircraft_power.variant_powerplants(
                variant_id,engine_variant_id,engine_count,is_standard,is_optional,is_primary,
                source_document_id)
             VALUES($1,$2,$3,TRUE,FALSE,TRUE,$4)
             ON CONFLICT(variant_id,engine_variant_id) DO NOTHING",
    )
    .bind(variant_id)
    .bind(engine_id)
    .bind(engine.engine_count.unwrap_or(1))
    .bind(document_id)
    .execute(&mut *self.transaction)
    .await
    .map_err(database_error)?;
    Ok(())
  }

  #[allow(clippy::too_many_lines)]
  async fn promote_market(
    &mut self,
    record: &PreparedAircraftRecord,
    variant_id: i64,
    document_id: i64,
  ) -> Result<(), PersistenceError> {
    if let Some(price) = record.valuation.papi_price_estimate.as_ref() {
      let inserted: Option<i64> = query_scalar(
        "INSERT INTO aircraft_market.valuations(
                    variant_id,snapshot_date,source_name,papi_price_estimate,
                    for_sale_count,currency_code,captured_at)
                 VALUES($1,CURRENT_DATE,$2,$3::numeric,$4,'USD',now())
                 ON CONFLICT DO NOTHING
                 RETURNING id",
      )
      .bind(variant_id)
      .bind(&self.source.name)
      .bind(price)
      .bind(record.valuation.for_sale_count)
      .fetch_optional(&mut *self.transaction)
      .await
      .map_err(database_error)?;
      // uq_val_variant_date_source allows one row per (variant, date, source),
      // so a second artifact imported the same day conflicts. Resolve the
      // existing row instead of skipping: its stored values stand — overwriting
      // them would silently republish a row a curator already accepted — but the
      // new document's price still has to be asserted, or its evidence is lost
      // and there is nothing for curation to act on.
      let valuation_id = match inserted {
        Some(id) => id,
        None => query_scalar(
          "SELECT id FROM aircraft_market.valuations
                   WHERE variant_id=$1 AND snapshot_date=CURRENT_DATE AND source_name=$2",
        )
        .bind(variant_id)
        .bind(&self.source.name)
        .fetch_one(&mut *self.transaction)
        .await
        .map_err(database_error)?,
      };

      // Market values were the only thing this adapter wrote without provenance,
      // which also left them uncurateable: migration 020 gates valuations on
      // is_canonical, and curation flips that flag by accepting these assertions.
      self
        .assert_entity_value(
          document_id,
          "VALUATION",
          valuation_id,
          "papi_price_estimate",
          price,
          Some(price),
          None,
        )
        .await?;
      if let Some(count) = record.valuation.for_sale_count {
        let count = count.to_string();
        self
          .assert_entity_value(
            document_id,
            "VALUATION",
            valuation_id,
            "for_sale_count",
            &count,
            Some(&count),
            None,
          )
          .await?;
      }
    }
    if record.operating_costs.items.is_empty() {
      return Ok(());
    }
    let snapshot_id: i64 = query_scalar(
      "INSERT INTO aircraft_market.cost_snapshots(
                variant_id,snapshot_date,currency_code,source_name,extra_attributes)
             VALUES($1,CURRENT_DATE,'USD',$2,'{}'::jsonb)
             ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(variant_id)
    .bind(&self.source.name)
    .fetch_optional(&mut *self.transaction)
    .await
    .map_err(database_error)?
    .unwrap_or(
      query_scalar(
        "SELECT id FROM aircraft_market.cost_snapshots
                 WHERE variant_id=$1 AND snapshot_date=CURRENT_DATE AND source_name=$2",
      )
      .bind(variant_id)
      .bind(&self.source.name)
      .fetch_one(&mut *self.transaction)
      .await
      .map_err(database_error)?,
    );

    let mut extras = Map::new();
    for item in &record.operating_costs.items {
      if let (Some(code), Some(amount)) = (&item.mapped_code, &item.numeric_value) {
        if item.is_aggregate {
          query(
            // captured_from_key records which source key produced the total,
            // so a curator can trace an aggregate back to the raw field.
            "INSERT INTO aircraft_market.cost_snapshot_totals(
                        snapshot_id,total_annual_usd,total_fixed_usd,total_variable_usd,
                        captured_from_key)
                     VALUES(
                        $1,
                        CASE WHEN $2='TOTAL_COST_ANNUAL' THEN $3::numeric END,
                        CASE WHEN $2='TOTAL_FIXED_COST' THEN $3::numeric END,
                        CASE WHEN $2='TOTAL_VARIABLE_COST' THEN $3::numeric END,
                        $4)
                     ON CONFLICT(snapshot_id) DO UPDATE SET
                        total_annual_usd=COALESCE(
                            aircraft_market.cost_snapshot_totals.total_annual_usd,
                            EXCLUDED.total_annual_usd),
                        total_fixed_usd=COALESCE(
                            aircraft_market.cost_snapshot_totals.total_fixed_usd,
                            EXCLUDED.total_fixed_usd),
                        total_variable_usd=COALESCE(
                            aircraft_market.cost_snapshot_totals.total_variable_usd,
                            EXCLUDED.total_variable_usd),
                        captured_from_key=COALESCE(
                            aircraft_market.cost_snapshot_totals.captured_from_key,
                            EXCLUDED.captured_from_key)",
          )
          .bind(snapshot_id)
          .bind(code)
          .bind(amount)
          .bind(&item.source_key)
          .execute(&mut *self.transaction)
          .await
          .map_err(database_error)?;
        } else {
          query(
            "INSERT INTO aircraft_market.cost_line_items(
                        snapshot_id,cost_item_type_code,amount_annual,amount_per_hour,currency_code)
                     SELECT $1,$2,
                        CASE WHEN is_fixed THEN $3::numeric END,
                        CASE WHEN NOT is_fixed THEN $3::numeric END,
                        'USD'
                     FROM aircraft_ref.cost_item_types WHERE code=$2
                     ON CONFLICT(snapshot_id,cost_item_type_code) DO NOTHING",
          )
          .bind(snapshot_id)
          .bind(code)
          .bind(amount)
          .execute(&mut *self.transaction)
          .await
          .map_err(database_error)?;
        }
      } else {
        extras.insert(item.source_key.clone(), Value::String(item.raw_value.clone()));
      }

      let assertion_field =
        item.mapped_code.clone().unwrap_or_else(|| format!("EXTRA:{}", item.source_key));
      self
        .assert_entity_value(
          document_id,
          "COST_SNAPSHOT",
          snapshot_id,
          &assertion_field,
          &item.raw_value,
          item.numeric_value.as_deref(),
          None,
        )
        .await?;
    }
    if !extras.is_empty() {
      query(
        "UPDATE aircraft_market.cost_snapshots
                 SET extra_attributes=extra_attributes || $2 WHERE id=$1",
      )
      .bind(snapshot_id)
      .bind(Value::Object(extras))
      .execute(&mut *self.transaction)
      .await
      .map_err(database_error)?;
    }
    Ok(())
  }

  async fn promote_issues(
    &mut self,
    record: &PreparedAircraftRecord,
    variant_id: i64,
  ) -> Result<(), PersistenceError> {
    for issue in record.issues.iter().filter(|issue| issue.severity == IssueSeverity::Warning) {
      query(
        "INSERT INTO aircraft_prov.curation_flags(
                    entity_type_code,entity_id,field_name,issue_type,
                    issue_description,status_code,priority)
                 VALUES('AIRCRAFT_VARIANT',$1,$2,$3,$4,'OPEN',3)",
      )
      .bind(variant_id)
      .bind(&issue.field_path)
      .bind(&issue.code)
      .bind(&issue.message)
      .execute(&mut *self.transaction)
      .await
      .map_err(database_error)?;
    }
    Ok(())
  }

  async fn assert_value(
    &mut self,
    document_id: i64,
    variant_id: i64,
    field: &str,
    raw: &str,
    numeric: Option<&str>,
    raw_unit: Option<&str>,
  ) -> Result<(), PersistenceError> {
    self
      .assert_entity_value(
        document_id,
        "AIRCRAFT_VARIANT",
        variant_id,
        field,
        raw,
        numeric,
        raw_unit,
      )
      .await
      .map(|_| ())
  }

  #[allow(clippy::too_many_arguments)]
  async fn assert_entity_value(
    &mut self,
    document_id: i64,
    entity_type: &str,
    entity_id: i64,
    field: &str,
    raw: &str,
    numeric: Option<&str>,
    raw_unit: Option<&str>,
  ) -> Result<i64, PersistenceError> {
    query_scalar(
      "INSERT INTO aircraft_prov.source_assertions(
                source_document_id,entity_type_code,entity_id,field_name,
                raw_value,raw_unit,asserted_value,asserted_numeric,
                status_code,is_accepted,confidence)
             SELECT $1,$2,$3,$4,$5,$6,$5,$7::numeric,
                'PENDING',FALSE,$8::numeric
             RETURNING id",
    )
    .bind(document_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(field)
    .bind(raw)
    .bind(raw_unit)
    .bind(numeric)
    .bind(SCRAPED_SOURCE_CONFIDENCE.as_str())
    .fetch_one(&mut *self.transaction)
    .await
    .map_err(database_error)
  }
}

fn row_to_status(row: &sqlx_postgres::PgRow) -> Result<RunStatus, PersistenceError> {
  let status: String = row.get("status");
  Ok(RunStatus {
    run_id: row.get("id"),
    attempt_id: row.try_get("attempt_id").ok(),
    source_slug: row.get("source_slug"),
    parser_name: row.get("parser_name"),
    parser_version: row.get("parser_version"),
    content_sha256: row.get("content_sha256"),
    status: parse_status(&status)?,
    input_locator: row.get("input_locator"),
    staged_records: decoded_count(row, "staged_aircraft")?,
    promoted_records: decoded_count(row, "promoted_aircraft")?,
    flagged_records: decoded_count(row, "flagged_aircraft")?,
    warning_count: decoded_count(row, "warning_count")?,
    attempts: Vec::new(),
    failure_code: row.try_get("failure_code").ok(),
    failure_message: row.try_get("failure_message").ok(),
    started_at: row.get::<DateTime<Utc>, _>("started_at"),
    finished_at: row.try_get("finished_at").ok(),
  })
}

fn row_to_attempt_status(row: &sqlx_postgres::PgRow) -> Result<AttemptStatus, PersistenceError> {
  let status: String = row.get("status");
  Ok(AttemptStatus {
    attempt_id: row.get("id"),
    attempt_number: u32::try_from(row.get::<i32, _>("attempt_number"))
      .map_err(|_| PersistenceError::Invariant("attempt number must be positive".to_owned()))?,
    status: parse_status(&status)?,
    staged_records: decoded_count(row, "staged_aircraft")?,
    promoted_records: decoded_count(row, "promoted_aircraft")?,
    flagged_records: decoded_count(row, "flagged_aircraft")?,
    warning_count: decoded_count(row, "warning_count")?,
    failure_code: row.try_get("failure_code").ok(),
    failure_message: row.try_get("failure_message").ok(),
    started_at: row.get("started_at"),
    finished_at: row.try_get("finished_at").ok(),
  })
}

fn parse_status(status: &str) -> Result<ImportStatus, PersistenceError> {
  match status {
    "RECEIVED" => Ok(ImportStatus::Received),
    "IMPORTING" => Ok(ImportStatus::Importing),
    "SUCCEEDED" => Ok(ImportStatus::Succeeded),
    "VALIDATION_FAILED" => Ok(ImportStatus::ValidationFailed),
    "FAILED" => Ok(ImportStatus::Failed),
    value => Err(PersistenceError::Invariant(format!("unknown run status {value}"))),
  }
}

fn nonnegative(value: Option<i32>) -> u64 {
  value.and_then(|value| u64::try_from(value).ok()).unwrap_or_default()
}

fn decoded_count(row: &sqlx_postgres::PgRow, column: &str) -> Result<u64, PersistenceError> {
  let value = row.try_get::<Option<i32>, _>(column).map_err(database_error)?;
  Ok(nonnegative(value))
}

fn to_i32(value: u64) -> Result<i32, PersistenceError> {
  i32::try_from(value)
    .map_err(|_| PersistenceError::Invariant("report count exceeds INT".to_owned()))
}

#[allow(clippy::needless_pass_by_value)]
fn serialization_error(error: serde_json::Error) -> PersistenceError {
  PersistenceError::Invariant(format!("could not serialize prepared record: {error}"))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn database_error(error: SqlxError) -> PersistenceError {
  let code = error
    .as_database_error()
    .and_then(DatabaseError::code)
    .map_or_else(|| "DATABASE_ERROR".to_owned(), |code| format!("DATABASE_{code}"));
  let message = sanitize_database_message(&error.to_string());
  PersistenceError::Database { code, message }
}

fn sanitize_database_message(message: &str) -> String {
  message.chars().filter(|character| !character.is_control()).take(1_000).collect()
}

#[cfg(test)]
mod tests {
  use super::sanitize_database_message;

  #[test]
  fn database_messages_are_bounded_and_remove_controls() {
    let message = format!("bad\n\u{1b}[31m{}", "x".repeat(2_000));
    let sanitized = sanitize_database_message(&message);
    assert!(!sanitized.contains('\n'));
    assert!(!sanitized.contains('\u{1b}'));
    assert_eq!(sanitized.chars().count(), 1_000);
  }
}
