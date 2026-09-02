use std::{fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Version of the machine-readable report and status payloads.
///
/// Bumped to 2 when the never-populated `skipped_records` field was removed;
/// consumers pinned to version 1 must not be fed version 2 documents.
pub const REPORT_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDescriptor {
  pub slug: String,
  pub name: String,
  pub base_url: Option<String>,
  pub parser_name: String,
  pub parser_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDescriptor {
  pub content_sha256: String,
  pub byte_length: u64,
  pub display_locator: String,
  pub captured_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightSummary {
  pub record_count: u64,
  pub warning_count: u64,
  pub record_keys_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRequest {
  pub source: SourceDescriptor,
  pub artifact: ArtifactDescriptor,
  pub preflight: PreflightSummary,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AircraftIdentityInput {
  pub manufacturer_name: String,
  pub aircraft_name: String,
  pub source_link: Option<String>,
  pub page_url: Option<String>,
  pub title: Option<String>,
  pub description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleInput {
  pub production_start_year: Option<i16>,
  pub production_end_year: Option<i16>,
  pub is_in_production: Option<bool>,
  pub passenger_capacity: Option<i16>,
  pub crew_count: Option<i16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasurementInput {
  pub source_field: String,
  pub metric_code: Option<String>,
  pub raw_value: String,
  pub numeric_value: Option<String>,
  pub raw_unit: Option<String>,
  pub unit_code: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceInput {
  pub measurements: Vec<MeasurementInput>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightInput {
  pub measurements: Vec<MeasurementInput>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropulsionInput {
  pub manufacturer: Option<String>,
  pub model: Option<String>,
  pub horsepower: Option<String>,
  pub thrust_newtons: Option<String>,
  pub engine_count: Option<i16>,
  pub tbo_hours: Option<i32>,
  pub tbo_years: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValuationInput {
  pub papi_price_estimate: Option<String>,
  pub for_sale_count: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostItemInput {
  pub source_key: String,
  pub raw_value: String,
  pub mapped_code: Option<String>,
  pub numeric_value: Option<String>,
  pub is_aggregate: bool,
  pub is_numeric: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatingCostInput {
  pub items: Vec<CostItemInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageMetadataInput {
  pub array_position: i16,
  pub href_raw: String,
  pub href_resolved: Option<String>,
  pub title: Option<String>,
  pub holder: Option<String>,
  pub dimensions_raw: Option<String>,
  pub width_px: Option<i16>,
  pub height_px: Option<i16>,
  pub is_primary: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceInput {
  pub source_system_key: String,
  pub source_url: Option<String>,
  pub source_path: Option<String>,
  pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IssueSeverity {
  Warning,
  Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestIssue {
  pub code: String,
  pub severity: IssueSeverity,
  pub field_path: String,
  pub message: String,
  pub raw_value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedAircraftRecord {
  pub source_record_key: String,
  pub identity: AircraftIdentityInput,
  pub lifecycle: LifecycleInput,
  pub performance: PerformanceInput,
  pub weights: WeightInput,
  pub propulsion: PropulsionInput,
  pub valuation: ValuationInput,
  pub operating_costs: OperatingCostInput,
  pub images: Vec<ImageMetadataInput>,
  pub provenance: ProvenanceInput,
  pub issues: Vec<IngestIssue>,
  pub raw_document: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportStatus {
  Received,
  Importing,
  Succeeded,
  ValidationFailed,
  Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
  pub schema_version: u16,
  pub run_id: i64,
  pub attempt_id: i64,
  pub status: ImportStatus,
  pub content_sha256: String,
  pub staged_records: u64,
  pub promoted_records: u64,
  pub flagged_records: u64,
  pub warning_count: u64,
  pub already_imported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptStatus {
  pub attempt_id: i64,
  pub attempt_number: u32,
  pub status: ImportStatus,
  pub staged_records: u64,
  pub promoted_records: u64,
  pub flagged_records: u64,
  pub warning_count: u64,
  pub failure_code: Option<String>,
  pub failure_message: Option<String>,
  pub started_at: DateTime<Utc>,
  pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStatus {
  pub run_id: i64,
  pub attempt_id: Option<i64>,
  pub source_slug: String,
  pub parser_name: String,
  pub parser_version: String,
  pub content_sha256: String,
  pub status: ImportStatus,
  pub input_locator: String,
  pub staged_records: u64,
  pub promoted_records: u64,
  pub flagged_records: u64,
  pub warning_count: u64,
  pub attempts: Vec<AttemptStatus>,
  pub failure_code: Option<String>,
  pub failure_message: Option<String>,
  pub started_at: DateTime<Utc>,
  pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusFilter {
  pub run_id: Option<i64>,
  pub content_sha256: Option<String>,
  pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordDisposition {
  Promoted,
  Flagged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordOutcome {
  pub staged: bool,
  pub disposition: RecordDisposition,
  pub warning_count: u64,
}

#[derive(Debug, Error)]
pub enum PreparedInputError {
  #[error("{code}: {message}")]
  Invalid { code: String, message: String },
}

impl PreparedInputError {
  #[must_use]
  pub fn code(&self) -> &str {
    match self {
      Self::Invalid { code, .. } => code,
    }
  }
}

#[async_trait]
pub trait PreparedRecordReader: Send {
  async fn next_record(&mut self) -> Result<Option<PreparedAircraftRecord>, PreparedInputError>;
}

#[derive(Debug, Error)]
pub enum PersistenceError {
  #[error("{code}: {message}")]
  Database { code: String, message: String },
  #[error("ingestion invariant failed: {0}")]
  Invariant(String),
}

impl PersistenceError {
  #[must_use]
  pub fn code(&self) -> &str {
    match self {
      Self::Database { code, .. } => code,
      Self::Invariant(_) => "PERSISTENCE_INVARIANT",
    }
  }
}

#[async_trait]
pub trait IngestionUnitOfWork: Send {
  async fn stage_and_promote(
    &mut self,
    record: &PreparedAircraftRecord,
  ) -> Result<RecordOutcome, PersistenceError>;
  async fn refresh_read_models(&mut self) -> Result<(), PersistenceError>;
  async fn mark_succeeded(&mut self, report: &ImportReport) -> Result<(), PersistenceError>;
  async fn commit(self: Box<Self>) -> Result<(), PersistenceError>;
  async fn rollback(self: Box<Self>) -> Result<(), PersistenceError>;
}

pub enum ImportStart {
  AlreadySucceeded(ImportReport),
  Busy,
  Ready { run_id: i64, attempt_id: i64, unit_of_work: Box<dyn IngestionUnitOfWork> },
}

impl fmt::Debug for ImportStart {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::AlreadySucceeded(report) => {
        formatter.debug_tuple("AlreadySucceeded").field(report).finish()
      }
      Self::Busy => formatter.write_str("Busy"),
      Self::Ready { run_id, attempt_id, .. } => formatter
        .debug_struct("Ready")
        .field("run_id", run_id)
        .field("attempt_id", attempt_id)
        .finish_non_exhaustive(),
    }
  }
}

#[async_trait]
pub trait IngestionStore: Send + Sync {
  async fn start_import(&self, request: &ImportRequest) -> Result<ImportStart, PersistenceError>;
  async fn mark_failed(
    &self,
    run_id: i64,
    attempt_id: i64,
    failure_code: &str,
    failure_message: &str,
  ) -> Result<(), PersistenceError>;
  async fn record_validation_failure(
    &self,
    source: &SourceDescriptor,
    artifact: &ArtifactDescriptor,
    failure_code: &str,
    failure_message: &str,
  ) -> Result<(), PersistenceError>;

  async fn status(&self, filter: &StatusFilter) -> Result<Vec<RunStatus>, PersistenceError>;
}

pub struct IngestionService {
  store: Arc<dyn IngestionStore>,
}
impl fmt::Debug for IngestionService {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.debug_struct("IngestionService").finish_non_exhaustive()
  }
}

impl IngestionService {
  #[must_use]
  pub fn new(store: Arc<dyn IngestionStore>) -> Self {
    Self { store }
  }

  pub async fn import(
    &self,
    request: ImportRequest,
    records: &mut dyn PreparedRecordReader,
  ) -> Result<ImportReport, ImportError> {
    let (run_id, attempt_id, mut unit_of_work) = match self.store.start_import(&request).await? {
      ImportStart::AlreadySucceeded(report) => return Ok(report),
      ImportStart::Busy => return Err(ImportError::AlreadyRunning),
      ImportStart::Ready { run_id, attempt_id, unit_of_work } => (run_id, attempt_id, unit_of_work),
    };

    let result =
      self.consume_records(&request, records, unit_of_work.as_mut(), run_id, attempt_id).await;

    match result {
      Ok(report) => {
        if let Err(error) = unit_of_work.mark_succeeded(&report).await {
          let error = ImportError::Persistence(error);
          self.rollback_and_audit(unit_of_work, run_id, attempt_id, &error).await;
          return Err(error);
        }
        match unit_of_work.commit().await {
          Ok(()) => Ok(report),
          Err(error) => {
            let error = ImportError::Persistence(error);
            self.audit_failure(run_id, attempt_id, &error).await;
            Err(error)
          }
        }
      }
      Err(error) => {
        self.rollback_and_audit(unit_of_work, run_id, attempt_id, &error).await;
        Err(error)
      }
    }
  }

  async fn rollback_and_audit(
    &self,
    unit_of_work: Box<dyn IngestionUnitOfWork>,
    run_id: i64,
    attempt_id: i64,
    error: &ImportError,
  ) {
    if let Err(rollback_error) = unit_of_work.rollback().await {
      tracing::error!(error = %rollback_error, "failed to roll back ingestion transaction");
    }
    self.audit_failure(run_id, attempt_id, error).await;
  }

  async fn audit_failure(&self, run_id: i64, attempt_id: i64, error: &ImportError) {
    let code = error.code();
    let message = sanitize_failure(&error.to_string());
    if let Err(audit_error) = self.store.mark_failed(run_id, attempt_id, code, &message).await {
      tracing::error!(error = %audit_error, "failed to persist ingestion failure audit");
    }
  }

  async fn consume_records(
    &self,
    request: &ImportRequest,
    records: &mut dyn PreparedRecordReader,
    unit_of_work: &mut dyn IngestionUnitOfWork,
    run_id: i64,
    attempt_id: i64,
  ) -> Result<ImportReport, ImportError> {
    let mut count = 0_u64;
    let mut record_keys = Sha256::new();
    let mut report = ImportReport {
      schema_version: REPORT_SCHEMA_VERSION,
      run_id,
      attempt_id,
      status: ImportStatus::Succeeded,
      content_sha256: request.artifact.content_sha256.clone(),
      staged_records: 0,
      promoted_records: 0,
      flagged_records: 0,
      warning_count: 0,
      already_imported: false,
    };

    while let Some(record) = records.next_record().await? {
      record_keys.update(record.source_record_key.as_bytes());
      record_keys.update(b"\0");
      if let Some(issue) = record.issues.iter().find(|issue| issue.severity == IssueSeverity::Error)
      {
        return Err(ImportError::Input(PreparedInputError::Invalid {
          code: issue.code.clone(),
          message: issue.message.clone(),
        }));
      }

      let outcome = unit_of_work.stage_and_promote(&record).await?;
      count += 1;
      report.staged_records += u64::from(outcome.staged);
      match outcome.disposition {
        RecordDisposition::Promoted => report.promoted_records += 1,
        RecordDisposition::Flagged => {
          report.promoted_records += 1;
          report.flagged_records += 1;
        }
      }
      report.warning_count += outcome.warning_count;
    }

    if count != request.preflight.record_count {
      return Err(ImportError::ParserConsistency {
        expected: request.preflight.record_count,
        actual: count,
      });
    }
    let actual_record_keys = hex_digest(&record_keys.finalize());
    if actual_record_keys != request.preflight.record_keys_sha256 {
      return Err(ImportError::ParserRecordKeysMismatch);
    }
    if report.warning_count != request.preflight.warning_count {
      return Err(ImportError::WarningCountMismatch {
        expected: request.preflight.warning_count,
        actual: report.warning_count,
      });
    }

    unit_of_work.refresh_read_models().await?;
    Ok(report)
  }

  pub async fn status(&self, filter: &StatusFilter) -> Result<Vec<RunStatus>, PersistenceError> {
    self.store.status(filter).await
  }
}

#[derive(Debug, Error)]
pub enum ImportError {
  #[error("an identical import is already running")]
  AlreadyRunning,
  #[error(transparent)]
  Input(#[from] PreparedInputError),
  #[error(transparent)]
  Persistence(#[from] PersistenceError),
  #[error("preflight contained {expected} records but import produced {actual}")]
  ParserConsistency { expected: u64, actual: u64 },
  #[error("preflight contained {expected} warnings but import produced {actual}")]
  WarningCountMismatch { expected: u64, actual: u64 },
  #[error("preflight and import produced different source-record keys")]
  ParserRecordKeysMismatch,
}

impl ImportError {
  #[must_use]
  pub fn code(&self) -> &str {
    match self {
      Self::AlreadyRunning => "IMPORT_ALREADY_RUNNING",
      Self::Input(error) => error.code(),
      Self::Persistence(error) => error.code(),
      Self::ParserConsistency { .. } => "PARSER_RECORD_COUNT_MISMATCH",
      Self::WarningCountMismatch { .. } => "PARSER_WARNING_COUNT_MISMATCH",
      Self::ParserRecordKeysMismatch => "PARSER_RECORD_KEYS_MISMATCH",
    }
  }
}

/// Lowercase hexadecimal, the encoding every digest column in the schema
/// expects; shared with credential issuance.
pub(crate) fn hex_digest(bytes: &[u8]) -> String {
  use std::fmt::Write as _;

  let mut output = String::with_capacity(bytes.len() * 2);
  for byte in bytes {
    let _ = write!(output, "{byte:02x}");
  }
  output
}

fn sanitize_failure(message: &str) -> String {
  message.chars().filter(|character| !character.is_control()).take(1_000).collect()
}

#[cfg(test)]
mod tests {
  #![allow(
    clippy::expect_used,
    clippy::significant_drop_tightening,
    clippy::struct_excessive_bools
  )]
  use super::*;

  #[test]
  fn failure_messages_are_bounded_and_remove_terminal_controls() {
    let message = format!("bad\u{1b}[31m{}", "x".repeat(2_000));
    let sanitized = sanitize_failure(&message);
    assert!(!sanitized.contains('\u{1b}'));
    assert_eq!(sanitized.chars().count(), 1_000);
  }
  #[derive(Default)]
  struct State {
    staged: bool,
    refreshed: bool,
    committed: bool,
    rolled_back: bool,
    failed_audited: bool,
    fail_mark_succeeded: bool,
    fail_commit: bool,
  }

  struct FakeStore {
    state: Arc<std::sync::Mutex<State>>,
  }

  #[async_trait]
  impl IngestionStore for FakeStore {
    async fn start_import(
      &self,
      _request: &ImportRequest,
    ) -> Result<ImportStart, PersistenceError> {
      Ok(ImportStart::Ready {
        run_id: 7,
        attempt_id: 11,
        unit_of_work: Box::new(FakeUnitOfWork { state: Arc::clone(&self.state) }),
      })
    }

    async fn mark_failed(
      &self,
      _run_id: i64,
      _attempt_id: i64,
      _failure_code: &str,
      _failure_message: &str,
    ) -> Result<(), PersistenceError> {
      self.state.lock().expect("state lock").failed_audited = true;
      Ok(())
    }

    async fn record_validation_failure(
      &self,
      _source: &SourceDescriptor,
      _artifact: &ArtifactDescriptor,
      _failure_code: &str,
      _failure_message: &str,
    ) -> Result<(), PersistenceError> {
      self.state.lock().expect("state lock").failed_audited = true;
      Ok(())
    }

    async fn status(&self, _filter: &StatusFilter) -> Result<Vec<RunStatus>, PersistenceError> {
      Ok(Vec::new())
    }
  }

  struct FakeUnitOfWork {
    state: Arc<std::sync::Mutex<State>>,
  }

  #[async_trait]
  impl IngestionUnitOfWork for FakeUnitOfWork {
    async fn stage_and_promote(
      &mut self,
      record: &PreparedAircraftRecord,
    ) -> Result<RecordOutcome, PersistenceError> {
      self.state.lock().expect("state lock").staged = true;
      let warnings =
        record.issues.iter().filter(|issue| issue.severity == IssueSeverity::Warning).count()
          as u64;
      Ok(RecordOutcome {
        staged: true,
        disposition: if warnings > 0 {
          RecordDisposition::Flagged
        } else {
          RecordDisposition::Promoted
        },
        warning_count: warnings,
      })
    }

    async fn refresh_read_models(&mut self) -> Result<(), PersistenceError> {
      self.state.lock().expect("state lock").refreshed = true;
      Ok(())
    }

    async fn mark_succeeded(&mut self, _report: &ImportReport) -> Result<(), PersistenceError> {
      if self.state.lock().expect("state lock").fail_mark_succeeded {
        return Err(PersistenceError::Invariant("mark failed".to_owned()));
      }
      Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
      if self.state.lock().expect("state lock").fail_commit {
        return Err(PersistenceError::Invariant("commit failed".to_owned()));
      }
      self.state.lock().expect("state lock").committed = true;
      Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<(), PersistenceError> {
      self.state.lock().expect("state lock").rolled_back = true;
      Ok(())
    }
  }

  struct FakeReader {
    records: std::collections::VecDeque<PreparedAircraftRecord>,
  }

  #[async_trait]
  impl PreparedRecordReader for FakeReader {
    async fn next_record(&mut self) -> Result<Option<PreparedAircraftRecord>, PreparedInputError> {
      Ok(self.records.pop_front())
    }
  }

  fn request(warning_count: u64) -> ImportRequest {
    ImportRequest {
      source: SourceDescriptor {
        slug: "fixture".to_owned(),
        name: "Fixture".to_owned(),
        base_url: None,
        parser_name: "fixture-json".to_owned(),
        parser_version: "1".to_owned(),
      },
      artifact: ArtifactDescriptor {
        content_sha256: "a".repeat(64),
        byte_length: 2,
        display_locator: "fixture.json".to_owned(),
        captured_at: Utc::now(),
      },
      preflight: PreflightSummary {
        record_count: 1,
        warning_count,
        record_keys_sha256: record_key_fingerprint(&["b".repeat(64)]),
      },
    }
  }

  fn record(severity: IssueSeverity) -> PreparedAircraftRecord {
    PreparedAircraftRecord {
      source_record_key: "b".repeat(64),
      identity: AircraftIdentityInput {
        manufacturer_name: "Acme".to_owned(),
        aircraft_name: "A1".to_owned(),
        ..AircraftIdentityInput::default()
      },
      lifecycle: LifecycleInput::default(),
      performance: PerformanceInput::default(),
      weights: WeightInput::default(),
      propulsion: PropulsionInput::default(),
      valuation: ValuationInput::default(),
      operating_costs: OperatingCostInput::default(),
      images: Vec::new(),
      provenance: ProvenanceInput {
        source_system_key: "b".repeat(64),
        source_url: None,
        source_path: None,
        confidence: 0.2,
      },
      issues: vec![IngestIssue {
        code: "FIXTURE_ISSUE".to_owned(),
        severity,
        field_path: "fixture".to_owned(),
        message: "fixture issue".to_owned(),
        raw_value: None,
      }],
      raw_document: serde_json::json!({}),
    }
  }

  #[tokio::test]
  async fn warning_records_commit_and_remain_flagged() {
    let state = Arc::new(std::sync::Mutex::new(State::default()));
    let service = IngestionService::new(Arc::new(FakeStore { state: Arc::clone(&state) }));
    let mut reader = FakeReader { records: [record(IssueSeverity::Warning)].into() };

    let report =
      service.import(request(1), &mut reader).await.expect("warning-only record should commit");

    assert_eq!(report.flagged_records, 1);
    let state = state.lock().expect("state lock");
    assert!(state.staged);
    assert!(state.refreshed);
    assert!(state.committed);
    assert!(!state.rolled_back);
  }

  #[tokio::test]
  async fn hard_issue_rolls_back_and_persists_failure_audit() {
    let state = Arc::new(std::sync::Mutex::new(State::default()));
    let service = IngestionService::new(Arc::new(FakeStore { state: Arc::clone(&state) }));
    let mut reader = FakeReader { records: [record(IssueSeverity::Error)].into() };

    assert!(service.import(request(0), &mut reader).await.is_err());

    let state = state.lock().expect("state lock");
    assert!(!state.staged);
    assert!(!state.committed);
    assert!(state.rolled_back);
    assert!(state.failed_audited);
  }

  #[tokio::test]
  async fn success_marker_failure_rolls_back_and_persists_failure_audit() {
    let state =
      Arc::new(std::sync::Mutex::new(State { fail_mark_succeeded: true, ..State::default() }));
    let service = IngestionService::new(Arc::new(FakeStore { state: Arc::clone(&state) }));
    let mut reader = FakeReader { records: [record(IssueSeverity::Warning)].into() };

    assert!(service.import(request(1), &mut reader).await.is_err());

    let state = state.lock().expect("state lock");
    assert!(state.rolled_back);
    assert!(!state.committed);
    assert!(state.failed_audited);
  }

  #[tokio::test]
  async fn commit_failure_persists_failure_audit() {
    let state = Arc::new(std::sync::Mutex::new(State { fail_commit: true, ..State::default() }));
    let service = IngestionService::new(Arc::new(FakeStore { state: Arc::clone(&state) }));
    let mut reader = FakeReader { records: [record(IssueSeverity::Warning)].into() };

    assert!(service.import(request(1), &mut reader).await.is_err());

    let state = state.lock().expect("state lock");
    assert!(!state.committed);
    assert!(state.failed_audited);
  }

  #[tokio::test]
  async fn second_pass_key_mismatch_rolls_back() {
    let state = Arc::new(std::sync::Mutex::new(State::default()));
    let service = IngestionService::new(Arc::new(FakeStore { state: Arc::clone(&state) }));
    let mut reader = FakeReader { records: [record(IssueSeverity::Warning)].into() };
    let mut mismatched_request = request(1);
    mismatched_request.preflight.record_keys_sha256 = "0".repeat(64);

    let error = service
      .import(mismatched_request, &mut reader)
      .await
      .expect_err("different second-pass keys must fail");
    assert!(matches!(error, ImportError::ParserRecordKeysMismatch));

    let state = state.lock().expect("state lock");
    assert!(state.rolled_back);
    assert!(!state.committed);
    assert!(state.failed_audited);
  }

  fn record_key_fingerprint(keys: &[String]) -> String {
    let mut hash = Sha256::new();
    for key in keys {
      hash.update(key.as_bytes());
      hash.update(b"\0");
    }
    hex_digest(&hash.finalize())
  }
}
