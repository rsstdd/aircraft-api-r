//! Ingestion snapshot gate.
//!
//! This started as a SQL-versus-Rust parity run against the legacy server-side
//! loader. That loader has been retired, so the gate now compares the adapter's
//! output against committed golden snapshots instead: the same normalized
//! business queries in `database/snapshots/`, diffed against
//! `database/snapshots/golden/<fixture>/<query>.txt`.
//!
//! It no longer validates the adapter against a second implementation — it
//! catches regressions. A golden file changing is a review event: read the diff
//! and decide whether the new output is correct before running with `--update`.

use std::{
  fmt::Write as _,
  path::{Path, PathBuf},
  process::{Command, Stdio},
  thread::sleep,
  time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};

const POSTGRES_IMAGE: &str = "postgres:16-alpine";
const CONTAINER_WORKSPACE: &str = "/workspace";
const READY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct SnapshotOptions {
  /// `PlanePHD` JSON to import before snapshotting.
  pub fixture: PathBuf,
  /// Keep the database running after the report, for manual inspection.
  pub keep: bool,
  /// Rewrite the golden files instead of comparing against them.
  pub update: bool,
}

/// A disposable `PostgreSQL` container, removed on drop unless explicitly kept.
struct Container {
  id: String,
  label: &'static str,
  keep: bool,
}

impl Container {
  fn start(label: &'static str) -> Result<Self> {
    let output = Command::new("docker")
      .args([
        "run",
        "--detach",
        "--env",
        "POSTGRES_PASSWORD=postgres",
        "--publish",
        "127.0.0.1::5432",
        POSTGRES_IMAGE,
      ])
      .output()
      .context("failed to start a PostgreSQL container; is docker running?")?;
    if !output.status.success() {
      bail!("docker run failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let id = String::from_utf8(output.stdout)?.trim().to_owned();
    Ok(Self { id, label, keep: false })
  }

  /// Host connection string, for clients running outside the container.
  fn database_url(&self) -> Result<String> {
    let output = Command::new("docker").args(["port", &self.id, "5432/tcp"]).output()?;
    if !output.status.success() {
      bail!("docker port failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let mapping = String::from_utf8(output.stdout)?;
    let port = mapping
      .trim()
      .lines()
      .next()
      .and_then(|line| line.rsplit_once(':'))
      .map(|(_, port)| port.trim().to_owned())
      .ok_or_else(|| anyhow!("docker returned an unexpected port mapping: {mapping:?}"))?;
    Ok(format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
  }

  fn wait_until_ready(&self) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
      let ready = Command::new("docker")
        .args(["exec", &self.id, "pg_isready", "-U", "postgres", "-d", "postgres"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
      if ready {
        return Ok(());
      }
      sleep(Duration::from_millis(250));
    }
    bail!("the {} database did not become ready within {READY_TIMEOUT:?}", self.label)
  }

  /// Copies `database/` in so the server can read both the install scripts and
  /// the fixture; the legacy loader reads its JSON server-side by path.
  fn copy_database_tree(&self, workspace_root: &Path) -> Result<()> {
    // `docker cp` will not create intermediate directories.
    let status = Command::new("docker")
      .args(["exec", &self.id, "mkdir", "-p", CONTAINER_WORKSPACE])
      .status()?;
    if !status.success() {
      bail!("failed to create {CONTAINER_WORKSPACE} in the {} container", self.label);
    }
    let status = Command::new("docker")
      .arg("cp")
      .arg(workspace_root.join("database"))
      .arg(format!("{}:{CONTAINER_WORKSPACE}/database", self.id))
      .status()?;
    if !status.success() {
      bail!("failed to copy database/ into the {} container", self.label);
    }
    Ok(())
  }

  fn psql(&self, args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("docker");
    command.args(["exec", &self.id, "psql", "-X", "-v", "ON_ERROR_STOP=1"]);
    command.args(["-U", "postgres", "-d", "postgres"]);
    command.args(args);
    Ok(command.output()?)
  }

  fn run_sql_file(&self, container_path: &str, variables: &[(&str, &str)]) -> Result<()> {
    let mut args: Vec<String> = Vec::new();
    for (key, value) in variables {
      args.push("-v".to_owned());
      args.push(format!("{key}={value}"));
    }
    args.push("-f".to_owned());
    args.push(container_path.to_owned());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = self.psql(&borrowed)?;
    if !output.status.success() {
      bail!(
        "{} failed on the {} database:\n{}",
        container_path,
        self.label,
        String::from_utf8_lossy(&output.stderr).trim()
      );
    }
    Ok(())
  }

  /// Runs a query unaligned and tuples-only, so the output is a stable,
  /// diffable set of `|`-separated rows.
  fn snapshot(&self, sql: &str) -> Result<String> {
    let output = self.psql(&["-A", "-t", "-F", "|", "-c", sql])?;
    if !output.status.success() {
      bail!(
        "snapshot query failed on the {} database:\n{}",
        self.label,
        String::from_utf8_lossy(&output.stderr).trim()
      );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim_end().to_owned())
  }
}

impl Drop for Container {
  fn drop(&mut self) {
    if self.keep {
      println!("kept the {} container: {}", self.label, self.id);
      return;
    }
    let _ = Command::new("docker")
      .args(["rm", "--force", &self.id])
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
  }
}

fn install_canonical_schema(container: &Container, workspace_root: &Path) -> Result<()> {
  container.copy_database_tree(workspace_root)?;
  container.run_sql_file(&format!("{CONTAINER_WORKSPACE}/database/install.sql"), &[])
}

fn load_through_rust_adapter(
  workspace_root: &Path,
  database_url: &str,
  fixture: &Path,
) -> Result<()> {
  let mut command = Command::new("cargo");
  command
    .current_dir(workspace_root)
    .args(["run", "--quiet", "--package", "aircraft-ingest", "--"])
    .args(["import", "--source", "planephd", "--input"])
    .arg(fixture)
    .args(["--format", "json"]);
  for (key, _) in std::env::vars() {
    if key.starts_with("APP__") {
      command.env_remove(key);
    }
  }
  command.env("APP__INGEST__DATABASE_URL", database_url);

  let output = command.output().context("failed to run the aircraft-ingest binary")?;
  if !output.status.success() {
    bail!(
      "aircraft-ingest import failed (exit {:?}):\n{}",
      output.status.code(),
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(())
}

/// Where the committed output of one query, for one fixture, lives.
fn golden_path(workspace_root: &Path, fixture: &Path, query: &str) -> PathBuf {
  let fixture_stem = fixture.file_stem().and_then(|stem| stem.to_str()).unwrap_or("unknown");
  let query_stem = query.strip_suffix(".sql").unwrap_or(query);
  workspace_root.join(format!("database/snapshots/golden/{fixture_stem}/{query_stem}.txt"))
}

/// Reports the first differing line, so a large snapshot does not bury the
/// change that matters.
fn describe_difference(expected: &str, actual: &str) -> String {
  let expected_lines: Vec<&str> = expected.lines().collect();
  let actual_lines: Vec<&str> = actual.lines().collect();
  let mut report = String::new();
  for row in &expected_lines {
    if !actual_lines.contains(row) {
      let _ = writeln!(report, "  missing: {row}");
    }
  }
  for row in &actual_lines {
    if !expected_lines.contains(row) {
      let _ = writeln!(report, "  added:   {row}");
    }
  }
  if report.is_empty() {
    let _ = writeln!(
      report,
      "  same rows in a different order ({} expected, {} actual)",
      expected_lines.len(),
      actual_lines.len()
    );
  }
  report
}

fn snapshot_queries(workspace_root: &Path) -> Result<Vec<(String, String)>> {
  let directory = workspace_root.join("database/snapshots");
  let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
    .with_context(|| format!("failed to read {}", directory.display()))?
    .map(|entry| entry.map(|entry| entry.path()))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("sql")))
    .collect();
  files.sort();
  if files.is_empty() {
    bail!("no snapshot queries found in {}", directory.display());
  }

  files
    .into_iter()
    .map(|path| {
      let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
      let sql = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
      Ok((name, sql))
    })
    .collect()
}

/// Imports the fixture into a disposable database and compares every normalized
/// business snapshot against its committed golden file.
///
/// # Errors
///
/// Returns an error when docker is unavailable, the import fails, a snapshot
/// query fails, or any snapshot differs from its golden file.
pub fn snapshots(workspace_root: &Path, options: &SnapshotOptions) -> Result<()> {
  let fixture = if options.fixture.is_absolute() {
    options.fixture.clone()
  } else {
    workspace_root.join(&options.fixture)
  };
  if !fixture.is_file() {
    bail!("fixture {} does not exist", fixture.display());
  }
  let queries = snapshot_queries(workspace_root)?;

  println!("starting a disposable PostgreSQL database");
  let mut database = Container::start("snapshot")?;
  database.keep = options.keep;
  database.wait_until_ready()?;

  println!("installing the canonical schema");
  install_canonical_schema(&database, workspace_root)?;

  println!("importing {}", fixture.display());
  let database_url = database.database_url()?;
  load_through_rust_adapter(workspace_root, &database_url, &fixture)?;

  println!("checking {} snapshots", queries.len());
  let mut differences = Vec::new();
  for (name, sql) in queries {
    let actual = database.snapshot(&sql)?;
    let path = golden_path(workspace_root, &fixture, &name);

    if options.update {
      if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
          .with_context(|| format!("failed to create {}", parent.display()))?;
      }
      std::fs::write(&path, format!("{actual}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
      println!("  wrote      {name}");
      continue;
    }

    let Ok(expected) = std::fs::read_to_string(&path) else {
      bail!(
        "no golden snapshot at {}. Review the output, then record it with \
         `cargo xtask snapshots --fixture {} --update`.",
        path.display(),
        options.fixture.display()
      );
    };
    if expected.trim_end() == actual.trim_end() {
      println!("  ok         {name}");
    } else {
      println!("  DIFFERS    {name}");
      differences.push((name, expected, actual));
    }
  }

  if options.update {
    println!("\nsnapshots updated; review the diff before committing it");
    return Ok(());
  }
  if differences.is_empty() {
    println!("\nsnapshots: no changes");
    return Ok(());
  }

  for (name, expected, actual) in &differences {
    println!("\n--- {name} ---");
    print!("{}", describe_difference(expected, actual));
  }
  println!(
    "\nA changed snapshot means the adapter now writes different data. Decide whether \
     that is correct before re-recording it with `--update`; the golden files are the \
     record of what this ingestion path is expected to produce."
  );
  bail!("{} snapshot(s) differ from their golden files", differences.len())
}
