#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::{
  io::Write,
  path::{Path, PathBuf},
  process::ExitCode,
  sync::Arc,
};

use aircraft_app::curation::{
  CurationError, CurationOutcome, CurationService, PendingAssertion, PendingFilter,
};
use aircraft_app::ingestion::{
  ImportError, IngestionService, IngestionStore, IssueSeverity, REPORT_SCHEMA_VERSION, RunStatus,
  StatusFilter,
};
use aircraft_config::{IngestArtifactSettings, IngestSettings};
use aircraft_db::{SqlxCurationStore, SqlxIngestionStore};
use aircraft_ingest::{InputArtifact, PlanePhdAdapter, SourceAdapter, SourceError};
use anyhow::Error;
use clap::{Parser, Subcommand, ValueEnum};
use secrecy::ExposeSecret;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "aircraft-ingest", version, about = "Validated aircraft data ingestion")]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
  Validate {
    #[arg(long, value_enum)]
    source: SourceChoice,
    #[arg(long)]
    input: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
  },
  Import {
    #[arg(long, value_enum)]
    source: SourceChoice,
    #[arg(long)]
    input: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    #[arg(long)]
    report: Option<PathBuf>,
  },
  Status {
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..), conflicts_with = "sha256")]
    run_id: Option<i64>,
    #[arg(long, value_parser = parse_sha256, conflicts_with = "run_id")]
    sha256: Option<String>,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=200))]
    limit: u32,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
  },
  /// Accept or reject what ingestion left pending
  Curate {
    #[command(subcommand)]
    command: CurateCommand,
  },
}

#[derive(Debug, Subcommand)]
enum CurateCommand {
  /// Show assertions awaiting a decision
  List {
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
    entity_id: Option<i64>,
    #[arg(long)]
    field: Option<String>,
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=500))]
    limit: u32,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
  },
  /// Accept an assertion, promoting its value into the canonical read model
  Accept {
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
    assertion_id: i64,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
  },
  /// Reject an assertion, withdrawing its value from the canonical read model
  Reject {
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
    assertion_id: i64,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
  },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SourceChoice {
  Planephd,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
  Human,
  Json,
}

#[tokio::main]
async fn main() -> ExitCode {
  aircraft_observability::logging::init();
  match run(Cli::parse()).await {
    Ok(()) => ExitCode::SUCCESS,
    Err(failure) => {
      eprintln!("{}", failure.error);
      ExitCode::from(failure.exit_code)
    }
  }
}

async fn run(cli: Cli) -> Result<(), CliFailure> {
  match cli.command {
    Command::Validate { source, input, format } => {
      let (adapter, artifact, preflight) = prepare(source, &input)?;
      render_validate(format, adapter.descriptor(), artifact.descriptor(), &preflight)?;
      Ok(())
    }
    Command::Import { source, input, format, report } => {
      let settings = IngestSettings::load().map_err(|error| {
        CliFailure::new(2, Error::new(error).context("invalid ingestion configuration"))
      })?;
      let (adapter, artifact) =
        capture(source, &input, settings.max_input_bytes, settings.temp_dir.as_deref())?;
      let preflight = match adapter.preflight(&artifact) {
        Ok(preflight) => preflight,
        Err(error) => {
          match connect_store(&settings).await {
            Ok(store) => {
              if let Err(audit_error) = store
                .record_validation_failure(
                  &adapter.descriptor(),
                  artifact.descriptor(),
                  error.code(),
                  &error.to_string(),
                )
                .await
              {
                eprintln!("failed to persist validation audit: {audit_error}");
              }
            }
            Err(audit_error) => {
              eprintln!("failed to connect for validation audit: {}", audit_error.error);
            }
          }
          return Err(source_failure(error));
        }
      };
      let store = connect_store(&settings).await?;
      let service = IngestionService::new(Arc::new(store));
      let mut reader = adapter.open_records(&artifact).map_err(source_failure)?;

      let request = aircraft_app::ingestion::ImportRequest {
        source: adapter.descriptor(),
        artifact: artifact.descriptor().clone(),
        preflight,
      };
      let result = service.import(request, reader.as_mut()).await.map_err(import_failure)?;
      if let Some(path) = report {
        write_report(&path, &result)?;
      }
      render_import(format, &result)
    }
    Command::Status { run_id, sha256, limit, format } => {
      let settings = IngestSettings::load().map_err(|error| {
        CliFailure::new(2, Error::new(error).context("invalid ingestion configuration"))
      })?;
      let store = connect_store(&settings).await?;
      let service = IngestionService::new(Arc::new(store));
      let statuses = service
        .status(&StatusFilter { run_id, content_sha256: sha256, limit })
        .await
        .map_err(|error| CliFailure::new(6, Error::new(error)))?;
      render_status(format, &statuses)?;
      Ok(())
    }
    Command::Curate { command } => run_curate(command).await,
  }
}

async fn run_curate(command: CurateCommand) -> Result<(), CliFailure> {
  let settings = IngestSettings::load().map_err(|error| {
    CliFailure::new(2, Error::new(error).context("invalid ingestion configuration"))
  })?;
  let store = connect_curation_store(&settings).await?;
  let service = CurationService::new(Arc::new(store));

  match command {
    CurateCommand::List { entity_id, field, limit, format } => {
      let pending = service
        .pending(&PendingFilter { entity_id, field_name: field, limit })
        .await
        .map_err(|error| CliFailure::new(6, Error::new(error)))?;
      render_pending(format, &pending)
    }
    CurateCommand::Accept { assertion_id, format } => {
      let outcome = service.accept(assertion_id).await.map_err(curation_failure)?;
      render_curation(format, &outcome)
    }
    CurateCommand::Reject { assertion_id, format } => {
      let outcome = service.reject(assertion_id).await.map_err(curation_failure)?;
      render_curation(format, &outcome)
    }
  }
}

/// Curation reuses the ingestion exit-code vocabulary: 6 for persistence
/// problems, 8 for a decision the current state does not permit.
fn curation_failure(error: CurationError) -> CliFailure {
  let code = match error {
    CurationError::Persistence(_) => 6,
    CurationError::UnknownAssertion(_)
    | CurationError::AlreadyDecided { .. }
    | CurationError::Conflict { .. } => 8,
  };
  CliFailure::new(code, Error::new(error))
}

fn render_pending(format: OutputFormat, pending: &[PendingAssertion]) -> Result<(), CliFailure> {
  match format {
    OutputFormat::Json => render_json(&serde_json::json!({
      "schema_version": REPORT_SCHEMA_VERSION,
      "pending": pending,
    })),
    OutputFormat::Human => {
      if pending.is_empty() {
        println!("No assertions are awaiting curation.");
      }
      for assertion in pending {
        println!(
          "{} {} {} = {} {} ({})",
          assertion.assertion_id,
          assertion.entity_label.as_deref().unwrap_or(&assertion.entity_type_code),
          assertion.field_name,
          assertion.raw_value.as_deref().unwrap_or("-"),
          assertion.raw_unit.as_deref().unwrap_or(""),
          assertion.source_slug.as_deref().unwrap_or("unknown source")
        );
      }
      Ok(())
    }
  }
}

fn render_curation(format: OutputFormat, outcome: &CurationOutcome) -> Result<(), CliFailure> {
  match format {
    OutputFormat::Json => render_json(outcome),
    OutputFormat::Human => {
      println!(
        "assertion {} {:?}: measurement canonical {}, {} flag(s) resolved, read model {}",
        outcome.assertion_id,
        outcome.decision,
        outcome.measurement_canonicalized,
        outcome.flags_resolved,
        if outcome.read_model_refreshed { "refreshed" } else { "unchanged" }
      );
      Ok(())
    }
  }
}

fn prepare(
  source: SourceChoice,
  input: &str,
) -> Result<(PlanePhdAdapter, InputArtifact, aircraft_app::ingestion::PreflightSummary), CliFailure>
{
  let settings = IngestArtifactSettings::load().map_err(|error| {
    CliFailure::new(2, Error::new(error).context("invalid ingestion configuration"))
  })?;
  let (adapter, artifact) =
    capture(source, input, settings.max_input_bytes, settings.temp_dir.as_deref())?;
  let preflight = adapter.preflight(&artifact).map_err(source_failure)?;
  Ok((adapter, artifact, preflight))
}

fn capture(
  source: SourceChoice,
  input: &str,
  max_bytes: u64,
  temp_dir: Option<&Path>,
) -> Result<(PlanePhdAdapter, InputArtifact), CliFailure> {
  let adapter = match source {
    SourceChoice::Planephd => PlanePhdAdapter,
  };
  let artifact = if input == "-" {
    InputArtifact::capture_stdin(temp_dir, max_bytes)
  } else {
    InputArtifact::capture_path(Path::new(input), temp_dir, max_bytes)
  }
  .map_err(|error| CliFailure::new(3, Error::new(error)))?;
  Ok((adapter, artifact))
}

fn render_validate(
  format: OutputFormat,
  source: aircraft_app::ingestion::SourceDescriptor,
  artifact: &aircraft_app::ingestion::ArtifactDescriptor,
  preflight: &aircraft_app::ingestion::PreflightSummary,
) -> Result<(), CliFailure> {
  #[derive(Serialize)]
  struct ValidationReport<'a> {
    schema_version: u16,
    valid: bool,
    source: aircraft_app::ingestion::SourceDescriptor,
    artifact: &'a aircraft_app::ingestion::ArtifactDescriptor,
    record_count: u64,
    warning_count: u64,
  }
  let report = ValidationReport {
    schema_version: REPORT_SCHEMA_VERSION,
    valid: true,
    source,
    artifact,
    record_count: preflight.record_count,
    warning_count: preflight.warning_count,
  };
  match format {
    OutputFormat::Json => render_json(&report),
    OutputFormat::Human => {
      println!(
        "valid: {} records, {} warnings, sha256 {}",
        report.record_count, report.warning_count, report.artifact.content_sha256
      );
      Ok(())
    }
  }
}

fn render_import(
  format: OutputFormat,
  report: &aircraft_app::ingestion::ImportReport,
) -> Result<(), CliFailure> {
  match format {
    OutputFormat::Json => render_json(report),
    OutputFormat::Human => {
      if report.already_imported {
        println!(
          "already imported: run {}, {} promoted, {} flagged, sha256 {}",
          report.run_id, report.promoted_records, report.flagged_records, report.content_sha256
        );
      } else {
        println!(
          "imported: run {}, attempt {}, {} promoted, {} flagged, {} warnings",
          report.run_id,
          report.attempt_id,
          report.promoted_records,
          report.flagged_records,
          report.warning_count
        );
      }
      Ok(())
    }
  }
}

fn render_status(format: OutputFormat, statuses: &[RunStatus]) -> Result<(), CliFailure> {
  match format {
    OutputFormat::Json => render_json(&serde_json::json!({
        "schema_version": REPORT_SCHEMA_VERSION,
        "runs": statuses,
    })),
    OutputFormat::Human => {
      if statuses.is_empty() {
        println!("No ingestion runs found.");
      }
      for status in statuses {
        println!(
          "run {} {:?}: {} promoted, {} flagged, {} warnings ({})",
          status.run_id,
          status.status,
          status.promoted_records,
          status.flagged_records,
          status.warning_count,
          status.content_sha256
        );
        if let (Some(code), Some(message)) = (&status.failure_code, &status.failure_message) {
          println!("  failure {code}: {message}");
        }
        for attempt in &status.attempts {
          println!(
            "  attempt {} ({}) {:?}: {} promoted, {} flagged, {} warnings",
            attempt.attempt_number,
            attempt.attempt_id,
            attempt.status,
            attempt.promoted_records,
            attempt.flagged_records,
            attempt.warning_count
          );
          if let (Some(code), Some(message)) = (&attempt.failure_code, &attempt.failure_message) {
            println!("    failure {code}: {message}");
          }
        }
      }
      Ok(())
    }
  }
}

/// Renders a machine-readable document. Every call site pre-branches on the
/// output format, so this is only ever reached for `OutputFormat::Json`.
fn render_json<T: Serialize>(value: &T) -> Result<(), CliFailure> {
  let json =
    serde_json::to_string_pretty(value).map_err(|error| CliFailure::new(7, Error::new(error)))?;
  println!("{json}");
  Ok(())
}

fn write_report<T: Serialize>(path: &Path, value: &T) -> Result<(), CliFailure> {
  let parent = path.parent().unwrap_or_else(|| Path::new("."));
  let mut temporary =
    tempfile::Builder::new().prefix(".aircraft-report-").tempfile_in(parent).map_err(|error| {
      CliFailure::new(3, Error::new(error).context("could not create temporary report"))
    })?;
  serde_json::to_writer_pretty(&mut temporary, value)
    .map_err(|error| CliFailure::new(7, Error::new(error)))?;
  temporary.write_all(b"\n").map_err(|error| CliFailure::new(3, Error::new(error)))?;
  temporary.persist(path).map_err(|error| {
    CliFailure::new(3, Error::new(error.error).context("could not atomically persist report"))
  })?;
  Ok(())
}

fn source_failure(error: SourceError) -> CliFailure {
  let exit_code = match error {
    SourceError::Artifact(_) => 3,
    SourceError::Json(_) | SourceError::EmptyDocument | SourceError::Validation { .. } => 4,
    SourceError::ParserThread(_) | SourceError::ConsumerClosed => 7,
  };
  if let SourceError::Validation { issues } = &error {
    for issue in issues.iter().filter(|issue| issue.severity == IssueSeverity::Error) {
      eprintln!("{} at {}: {}", issue.code, issue.field_path, issue.message);
    }
  }
  CliFailure::new(exit_code, Error::new(error))
}

fn import_failure(error: ImportError) -> CliFailure {
  let exit_code = match error {
    ImportError::AlreadyRunning => 5,
    ImportError::Input(_) => 4,
    ImportError::Persistence(_) => 6,
    ImportError::ParserConsistency { .. }
    | ImportError::WarningCountMismatch { .. }
    | ImportError::ParserRecordKeysMismatch => 7,
  };
  CliFailure::new(exit_code, Error::new(error))
}

async fn connect_store(settings: &IngestSettings) -> Result<SqlxIngestionStore, CliFailure> {
  SqlxIngestionStore::connect(
    settings.database_url.expose_secret(),
    settings.max_connections,
    settings.lock_timeout_seconds,
    settings.statement_timeout_seconds,
  )
  .await
  .map_err(|error| CliFailure::new(6, Error::new(error)))
}

async fn connect_curation_store(
  settings: &IngestSettings,
) -> Result<SqlxCurationStore, CliFailure> {
  // Reuses the ingestion pool builder so curation inherits the same lock and
  // statement timeouts rather than opening an unbounded connection.
  let store = SqlxIngestionStore::connect(
    settings.database_url.expose_secret(),
    settings.max_connections,
    settings.lock_timeout_seconds,
    settings.statement_timeout_seconds,
  )
  .await
  .map_err(|error| CliFailure::new(6, Error::new(error)))?;
  Ok(SqlxCurationStore::from_pool(store.pool().clone()))
}

fn parse_sha256(value: &str) -> Result<String, String> {
  (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| value.to_ascii_lowercase())
    .ok_or_else(|| "SHA-256 must contain exactly 64 hexadecimal characters".to_owned())
}

#[derive(Debug)]
struct CliFailure {
  exit_code: u8,
  error: Error,
}

impl CliFailure {
  const fn new(exit_code: u8, error: Error) -> Self {
    Self { exit_code, error }
  }
}
