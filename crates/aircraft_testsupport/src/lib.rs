//! Shared `PostgreSQL` harness for the ingestion integration tests.
//!
//! Each test gets its own disposable `postgres:16-alpine` container with the
//! canonical schema installed from `database/`, so the gates run against the
//! same DDL that ships rather than a test-local approximation.
//!
//! This lives in its own crate rather than a `tests/` module because the gates
//! that drive the `aircraft-ingest` binary must sit in `apps/ingest`, while the
//! repository tests sit in `aircraft_db`.

use std::{
  error::Error,
  io::Write as _,
  process::{Command, Stdio},
  time::Duration,
};

use aircraft_app::ingestion::{
  ArtifactDescriptor, ImportRequest, ImportStart, IngestionUnitOfWork, PreflightSummary,
  SourceDescriptor,
};
use chrono::Utc;
use sqlx_core::raw_sql::raw_sql;
use sqlx_postgres::{PgPool, PgPoolOptions};

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

/// The canonical install sequence, embedded at compile time so the tests cannot
/// drift from the SQL on disk.
///
/// Ordering mirrors `database/install.sql`: the reference seeds land after
/// migration 002 because later lookup data references measurement units, and
/// the mission-profile seed lands after migration 015.
/// `schema_steps_cover_every_migration` guards the list against a new migration
/// being added without being gated here.
pub const SCHEMA_STEPS: &[&str] = &[
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
  include_str!("../../../database/seeds/003_mission_profile_seed_data.sql"),
  include_str!("../../../database/migrations/016_read_models_views_indexes.sql"),
  include_str!("../../../database/migrations/017_rust_ingestion_adapter.sql"),
  include_str!("../../../database/migrations/018_staged_aircraft_variant_fk.sql"),
  include_str!("../../../database/migrations/019_weight_metrics_curation_gate.sql"),
  include_str!("../../../database/migrations/020_market_curation_gate.sql"),
  include_str!("../../../database/migrations/021_validate_measurement_assertion_foreign_keys.sql"),
  include_str!("../../../database/migrations/022_read_model_refresh_requests.sql"),
  include_str!("../../../database/migrations/023_backfill_ingestion_identity_projections.sql"),
  include_str!("../../../database/migrations/024_promote_existing_manufacturer_links.sql"),
  include_str!("../../../database/validation/017_rust_ingestion_adapter_validation.sql"),
  include_str!("../../../database/validation/018_staged_aircraft_variant_fk_validation.sql"),
  include_str!("../../../database/validation/019_weight_metrics_curation_gate_validation.sql"),
  include_str!("../../../database/validation/020_market_curation_gate_validation.sql"),
  include_str!(
    "../../../database/validation/021_validate_measurement_assertion_foreign_keys_validation.sql"
  ),
  include_str!("../../../database/validation/022_read_model_refresh_requests_validation.sql"),
  include_str!(
    "../../../database/validation/023_backfill_ingestion_identity_projections_validation.sql"
  ),
  include_str!(
    "../../../database/validation/024_promote_existing_manufacturer_links_validation.sql"
  ),
];

/// Filenames of the migrations covered by [`SCHEMA_STEPS`], in apply order.
///
/// Kept beside the list so a drift test can compare it against the directory
/// without re-reading the embedded SQL.
pub const COVERED_MIGRATIONS: &[&str] = &[
  "001_extensions_schemas_domains_triggers.sql",
  "002_core_reference_tables.sql",
  "003_geography_operators_organizations.sql",
  "004_aircraft_identity_taxonomy.sql",
  "005_certification_operating_approvals.sql",
  "006_dimensions_cabin_cargo_hangar_fit.sql",
  "007_weight_balance_payload_loading.sql",
  "008_performance_metrics_conditions.sql",
  "009_propulsion_engines_rotors_stcs.sql",
  "010_avionics_equipment_systems.sql",
  "011_military_sensors_stores_loadouts.sql",
  "012_ownership_cost_valuation_market.sql",
  "013_maintenance_reliability_supportability.sql",
  "014_sources_provenance_curation_audit.sql",
  "015_mission_profiles_comparison_scoring.sql",
  "016_read_models_views_indexes.sql",
  "017_rust_ingestion_adapter.sql",
  "018_staged_aircraft_variant_fk.sql",
  "019_weight_metrics_curation_gate.sql",
  "020_market_curation_gate.sql",
  "021_validate_measurement_assertion_foreign_keys.sql",
  "022_read_model_refresh_requests.sql",
  "023_backfill_ingestion_identity_projections.sql",
  "024_promote_existing_manufacturer_links.sql",
];

/// A disposable `PostgreSQL` container, force-removed when the guard drops.
#[derive(Debug)]
pub struct DockerPostgres {
  container_id: String,
  /// Connection string for clients outside this process, such as the CLI.
  pub database_url: String,
}

impl Drop for DockerPostgres {
  fn drop(&mut self) {
    let _ = Command::new("docker")
      .args(["rm", "--force", &self.container_id])
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
  }
}

/// Starts a throwaway `PostgreSQL` container and waits for it to accept
/// connections, returning the container guard alongside a connected pool.
pub async fn start_postgres(
  max_connections: u32,
  acquire_timeout: Duration,
) -> TestResult<(DockerPostgres, PgPool)> {
  let output = Command::new("docker")
    .args([
      "run",
      "--detach",
      "--rm",
      "--publish",
      "127.0.0.1::5432",
      "--env",
      "POSTGRES_PASSWORD=postgres",
      "postgres:16-alpine",
    ])
    .output()?;
  if !output.status.success() {
    return Err(
      std::io::Error::other(format!(
        "docker run failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
      ))
      .into(),
    );
  }

  let container_id = String::from_utf8(output.stdout)?.trim().to_owned();
  let port_output = Command::new("docker").args(["port", &container_id, "5432/tcp"]).output()?;
  if !port_output.status.success() {
    let _ = Command::new("docker")
      .args(["rm", "--force", &container_id])
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
    return Err(
      std::io::Error::other(format!(
        "docker port failed: {}",
        String::from_utf8_lossy(&port_output.stderr).trim()
      ))
      .into(),
    );
  }
  let port = String::from_utf8(port_output.stdout)?
    .trim()
    .rsplit_once(':')
    .map(|(_, port)| port)
    .ok_or_else(|| std::io::Error::other("docker returned an invalid PostgreSQL port"))?
    .parse::<u16>()?;
  let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
  let container = DockerPostgres { container_id, database_url: database_url.clone() };

  let pool = tokio::time::timeout(Duration::from_secs(30), async {
    loop {
      match PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(acquire_timeout)
        .connect(&database_url)
        .await
      {
        Ok(pool) => break pool,
        Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
      }
    }
  })
  .await
  .map_err(|_| std::io::Error::other("PostgreSQL container did not become ready"))?;

  Ok((container, pool))
}

/// Applies [`SCHEMA_STEPS`] in order, reporting which step failed.
pub async fn install_schema(pool: &PgPool) -> TestResult {
  for (index, sql) in SCHEMA_STEPS.iter().enumerate() {
    raw_sql(sql).execute(pool).await.map_err(|error| {
      std::io::Error::other(format!("schema step {} failed: {error:?}", index + 1))
    })?;
  }
  Ok(())
}

/// Runs a psql script inside the container, for SQL that `SQLx` cannot execute.
///
/// The role files under `database/roles/` are psql programs -- `\if`, `\getenv`,
/// `\gexec` -- so a test that wants to assert what the *shipped* provisioning
/// does has to run them the way an administrator does. The script arrives on
/// stdin rather than through a bind mount, so the caller embeds the shipped file
/// with `include_str!` and cannot drift from it.
///
/// `variables` become `-v name=value` psql variables. `environment` is passed by
/// **name only** to `docker exec`, with the value set on this process, so a
/// password reaches the container without ever entering an argument list -- the
/// rule `database/roles/create_app_role.sql` states for its own invocation.
///
/// # Errors
///
/// Returns an error if `docker exec` cannot be started, the script cannot be
/// written, or psql exits non-zero. `ON_ERROR_STOP` is set, so any failing
/// statement fails the call.
pub fn run_psql(
  container: &DockerPostgres,
  sql: &str,
  variables: &[(&str, &str)],
  environment: &[(&str, &str)],
) -> TestResult {
  let mut command = Command::new("docker");
  command.args(["exec", "--interactive"]);
  for (name, value) in environment {
    command.args(["--env", name]).env(name, value);
  }
  command.arg(&container.container_id).args([
    "psql",
    "-X",
    "-v",
    "ON_ERROR_STOP=1",
    "-U",
    "postgres",
    "-d",
    "postgres",
  ]);
  for (name, value) in variables {
    command.arg("-v").arg(format!("{name}={value}"));
  }
  let mut child = command
    .arg("-f")
    .arg("-")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

  child
    .stdin
    .take()
    .ok_or_else(|| std::io::Error::other("docker exec provided no stdin"))?
    .write_all(sql.as_bytes())?;

  let output = child.wait_with_output()?;
  if !output.status.success() {
    return Err(
      std::io::Error::other(format!(
        "psql failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
      ))
      .into(),
    );
  }
  Ok(())
}

/// Builds an import request whose artifact hash is `hash_character` repeated,
/// so distinct logical runs are easy to construct in a test.
#[must_use]
pub fn request(hash_character: char, parser_version: &str) -> ImportRequest {
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

/// Unwraps a [`ImportStart::Ready`], turning the other outcomes into failures.
pub fn ready(start: ImportStart) -> TestResult<(i64, i64, Box<dyn IngestionUnitOfWork>)> {
  match start {
    ImportStart::Ready { run_id, attempt_id, unit_of_work } => {
      Ok((run_id, attempt_id, unit_of_work))
    }
    ImportStart::AlreadySucceeded(_) => Err("test import unexpectedly already succeeded".into()),
    ImportStart::Busy => Err("test import unexpectedly reported busy".into()),
  }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "a failing assertion is the point of a test")]
mod tests {
  use super::COVERED_MIGRATIONS;

  /// A migration added to `database/migrations/` but not to [`SCHEMA_STEPS`]
  /// would never be installed by the integration gates, so the gates would
  /// keep passing against a schema that no longer matches what ships.
  ///
  /// [`SCHEMA_STEPS`]: super::SCHEMA_STEPS
  #[test]
  fn schema_steps_cover_every_migration() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../../database/migrations");
    let mut on_disk: Vec<String> = std::fs::read_dir(directory)
      .expect("database/migrations must be readable")
      .map(|entry| entry.expect("readable directory entry").file_name())
      .filter_map(|name| name.into_string().ok())
      .filter(|name| {
        std::path::Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("sql"))
      })
      .collect();
    on_disk.sort();

    let covered: Vec<String> = COVERED_MIGRATIONS.iter().map(|&name| name.to_owned()).collect();
    assert_eq!(
      covered, on_disk,
      "SCHEMA_STEPS and COVERED_MIGRATIONS must list every file in \
             database/migrations/ in apply order"
    );
  }
}
