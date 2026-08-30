use std::{
  collections::{BTreeMap, BTreeSet},
  ffi::OsStr,
  fs,
  path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::path_label;

const LOCK_SCHEMA_VERSION: u32 = 1;
const SQUAWK_BASELINE_LAST_VERSION: u16 = 16;
const VALIDATION_REQUIRED_FROM_VERSION: u16 = 17;
const HISTORY_LEDGER_FILE: &str = "000_migration_history_validation.sql";
const LEDGER_INSERT_PREFIX: &str =
  "INSERT INTO public.aircraft_schema_migrations(version) VALUES ('";
const LEDGER_DECLARATION_PREFIX: &str = "expected_versions CONSTANT TEXT[] := ARRAY[";

#[derive(Debug, Deserialize)]
struct MigrationLock {
  schema_version: u32,
  migrations: Vec<LockedMigration>,
}

#[derive(Debug, Deserialize)]
struct LockedMigration {
  file: String,
  sha256: String,
}

#[derive(Debug)]
struct Migration {
  file: String,
  path: PathBuf,
  version: u16,
}

pub fn check(workspace_root: &Path) -> Result<()> {
  let database = workspace_root.join("database");
  let migrations = read_migrations(&database.join("migrations"))?;
  let lock = read_lock(&database.join("migrations.lock.json"))?;
  let mut violations = Vec::new();

  check_names_and_versions(&migrations, &mut violations);
  check_lock(&migrations, &lock, &mut violations)?;
  check_transactions(&migrations, &mut violations)?;
  check_installer(&migrations, &database.join("install.sql"), &mut violations)?;
  check_validations(&migrations, &database.join("validation"), &mut violations)?;
  check_history_ledger(
    &migrations,
    &database.join("validation").join(HISTORY_LEDGER_FILE),
    &mut violations,
  )?;
  check_squawk_exclusions(&migrations, &workspace_root.join(".squawk.toml"), &mut violations)?;

  if !violations.is_empty() {
    violations.sort();
    bail!("migration policy failed:\n{}", violations.join("\n"));
  }

  println!("Migration history, installer, validations, and lint baseline are valid.");
  Ok(())
}

fn read_migrations(directory: &Path) -> Result<Vec<Migration>> {
  let entries =
    fs::read_dir(directory).with_context(|| format!("failed to read {}", path_label(directory)))?;
  let mut migrations = entries
    .map(|entry| {
      let path = entry?.path();
      let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("migration filename must be valid UTF-8")?
        .to_owned();
      Ok((file, path))
    })
    .collect::<Result<Vec<_>>>()?;
  migrations.retain(|(_, path)| has_sql_extension(path));
  migrations.sort_by(|left, right| left.0.cmp(&right.0));

  migrations
    .into_iter()
    .map(|(file, path)| {
      let version = migration_version(&file).unwrap_or_default();
      Ok(Migration { file, path, version })
    })
    .collect()
}

fn read_lock(path: &Path) -> Result<MigrationLock> {
  let label = path_label(path);
  let bytes = fs::read(path).with_context(|| format!("failed to read {label}"))?;
  let lock: MigrationLock =
    serde_json::from_slice(&bytes).with_context(|| format!("{label} is not valid JSON"))?;
  if lock.schema_version != LOCK_SCHEMA_VERSION {
    bail!(
      "unsupported migration lock schema version {}; expected {LOCK_SCHEMA_VERSION}",
      lock.schema_version
    );
  }
  Ok(lock)
}

fn check_names_and_versions(migrations: &[Migration], violations: &mut Vec<String>) {
  let mut versions = BTreeMap::new();
  for migration in migrations {
    let valid_name = migration_version(&migration.file).is_some()
      && migration
        .file
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.".contains(&byte));
    if !valid_name {
      violations.push(format!("{} must use NNN_lowercase_description.sql naming", migration.file));
      continue;
    }
    if let Some(previous) = versions.insert(migration.version, &migration.file) {
      violations.push(format!(
        "migration version {:03} is duplicated by {previous} and {}",
        migration.version, migration.file
      ));
    }
  }
}

fn migration_version(file: &str) -> Option<u16> {
  let (prefix, description) = file.split_once('_')?;
  if prefix.len() != 3 || description == ".sql" || !has_sql_extension(Path::new(description)) {
    return None;
  }
  prefix.parse().ok()
}

fn has_sql_extension(path: &Path) -> bool {
  path.extension() == Some(OsStr::new("sql"))
}

fn check_lock(
  migrations: &[Migration],
  lock: &MigrationLock,
  violations: &mut Vec<String>,
) -> Result<()> {
  let locked_by_file = lock
    .migrations
    .iter()
    .map(|migration| (migration.file.as_str(), migration.sha256.as_str()))
    .collect::<BTreeMap<_, _>>();
  let files = migrations.iter().map(|migration| migration.file.as_str()).collect::<BTreeSet<_>>();
  let locked_files = locked_by_file.keys().copied().collect::<BTreeSet<_>>();

  for missing in files.difference(&locked_files) {
    violations.push(format!("{missing} is missing from database/migrations.lock.json"));
  }
  for stale in locked_files.difference(&files) {
    violations.push(format!("database/migrations.lock.json references missing {stale}"));
  }

  for migration in migrations {
    let Some(expected) = locked_by_file.get(migration.file.as_str()) else {
      continue;
    };
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
      violations.push(format!("{} has an invalid locked SHA-256", migration.file));
      continue;
    }
    let bytes = fs::read(&migration.path)
      .with_context(|| format!("failed to read {}", path_label(&migration.path)))?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != *expected {
      violations
        .push(format!("{} checksum changed: expected {expected}, found {actual}", migration.file));
    }
  }
  Ok(())
}

fn check_transactions(migrations: &[Migration], violations: &mut Vec<String>) -> Result<()> {
  for migration in migrations {
    let sql = fs::read_to_string(&migration.path)
      .with_context(|| format!("failed to read {}", path_label(&migration.path)))?;
    let statements = sql.lines().map(str::trim).collect::<BTreeSet<_>>();
    if !statements.contains("BEGIN;") || !statements.contains("COMMIT;") {
      violations
        .push(format!("{} must contain an explicit BEGIN/COMMIT transaction", migration.file));
    }
  }
  Ok(())
}

fn check_installer(
  migrations: &[Migration],
  installer_path: &Path,
  violations: &mut Vec<String>,
) -> Result<()> {
  let installer = fs::read_to_string(installer_path)
    .with_context(|| format!("failed to read {}", path_label(installer_path)))?;
  let directives = installer
    .lines()
    .enumerate()
    .filter_map(|(line_number, line)| {
      let line = line.trim();
      (!line.starts_with("--"))
        .then(|| line.strip_prefix("\\ir migrations/").map(|file| (line_number, file)))
        .flatten()
    })
    .collect::<Vec<_>>();
  let mut previous_position = None;
  let expected =
    migrations.iter().map(|migration| migration.file.as_str()).collect::<BTreeSet<_>>();

  for migration in migrations {
    let positions = directives
      .iter()
      .filter_map(|(line_number, file)| (*file == migration.file).then_some(*line_number))
      .collect::<Vec<_>>();
    if positions.len() != 1 {
      violations.push(format!(
        "database/install.sql must include {} exactly once; found {}",
        migration.file,
        positions.len()
      ));
      continue;
    }
    if previous_position.is_some_and(|previous| positions[0] <= previous) {
      violations.push(format!("database/install.sql applies {} out of order", migration.file));
    }
    previous_position = Some(positions[0]);
  }

  for (_, file) in directives {
    if !expected.contains(file) {
      violations.push(format!("database/install.sql references unknown migration {file}"));
    }
  }

  // The installer, not the migration files, writes the ledger rows that
  // `000_migration_history_validation.sql` asserts against. A migration applied
  // by an `\\ir` directive but never recorded here installs cleanly and then
  // fails validation, so the two lists are checked separately.
  let recorded = installer
    .lines()
    .map(str::trim)
    .filter(|line| !line.starts_with("--"))
    .filter_map(|line| line.strip_prefix(LEDGER_INSERT_PREFIX))
    .filter_map(|rest| rest.split_once('\'').map(|(version, _)| version.to_owned()))
    .collect::<Vec<_>>();
  let versions = shipped_versions(migrations);
  if recorded != versions {
    violations.push(format!(
      "database/install.sql records ledger versions {recorded:?}, but database/migrations/ ships \
       {versions:?}"
    ));
  }
  Ok(())
}

fn check_validations(
  migrations: &[Migration],
  validations_directory: &Path,
  violations: &mut Vec<String>,
) -> Result<()> {
  let validation_files = fs::read_dir(validations_directory)
    .with_context(|| format!("failed to read {}", path_label(validations_directory)))?
    .map(|entry| {
      entry?
        .file_name()
        .into_string()
        .map_err(|_| anyhow::anyhow!("validation filename must be valid UTF-8"))
    })
    .collect::<Result<Vec<_>>>()?;

  for migration in
    migrations.iter().filter(|migration| migration.version >= VALIDATION_REQUIRED_FROM_VERSION)
  {
    let prefix = format!("{:03}_", migration.version);
    if !validation_files
      .iter()
      .any(|file| file.starts_with(&prefix) && has_sql_extension(Path::new(file)))
    {
      violations
        .push(format!("{} requires a database/validation/{prefix}*.sql companion", migration.file));
    }
  }
  Ok(())
}

/// Compares the hardcoded ledger assertion against the migrations directory.
///
/// `000_migration_history_validation.sql` enumerates the exact versions the
/// installer must have recorded, but it only executes against a live database,
/// so a stale array survives every static gate and fails much later during
/// `just db-validate`. Comparing it here turns that into a policy failure at the
/// same moment the migration is added.
fn check_history_ledger(
  migrations: &[Migration],
  ledger_path: &Path,
  violations: &mut Vec<String>,
) -> Result<()> {
  let label = path_label(ledger_path);
  let sql = fs::read_to_string(ledger_path).with_context(|| format!("failed to read {label}"))?;
  let Some(declared) = ledger_versions(&sql) else {
    violations.push(format!("{label} must declare expected_versions as an ARRAY[...] literal"));
    return Ok(());
  };

  // Compared as ordered sequences, not sets: the assertion aggregates with
  // ORDER BY version, so a correctly populated but misordered literal fails at
  // runtime just as a missing version does.
  let expected = shipped_versions(migrations);
  if declared != expected {
    violations.push(format!(
      "{label} expects versions {declared:?}, but database/migrations/ ships {expected:?}"
    ));
  }
  Ok(())
}

/// The zero-padded versions of the migrations that ship on disk, in apply order.
fn shipped_versions(migrations: &[Migration]) -> Vec<String> {
  migrations.iter().map(|migration| format!("{:03}", migration.version)).collect()
}

/// Extracts the quoted versions from the `expected_versions` array literal.
fn ledger_versions(sql: &str) -> Option<Vec<String>> {
  let active_sql = without_sql_comments(sql)?;
  let mut lines = active_sql.lines().map(str::trim);
  let mut literal = lines.find_map(|line| line.strip_prefix(LEDGER_DECLARATION_PREFIX))?.to_owned();
  while !literal.contains(']') {
    literal.push_str(lines.next()?);
  }

  let (entries, suffix) = literal.split_once(']')?;
  if !suffix.trim_start().starts_with(';') {
    return None;
  }

  entries
    .split(',')
    .map(|entry| {
      let version = entry.trim().strip_prefix('\'')?.strip_suffix('\'')?;
      (version.len() == 3 && version.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| version.to_owned())
    })
    .collect()
}

fn without_sql_comments(sql: &str) -> Option<String> {
  let mut active = String::with_capacity(sql.len());
  let mut characters = sql.chars().peekable();

  while let Some(character) = characters.next() {
    if character == '-' && characters.peek() == Some(&'-') {
      characters.next();
      for commented in characters.by_ref() {
        if commented == '\n' {
          active.push('\n');
          break;
        }
      }
    } else if character == '/' && characters.peek() == Some(&'*') {
      characters.next();
      let mut closed = false;
      while let Some(commented) = characters.next() {
        if commented == '\n' {
          active.push('\n');
        } else if commented == '*' && characters.peek() == Some(&'/') {
          characters.next();
          closed = true;
          break;
        }
      }
      if !closed {
        return None;
      }
    } else {
      active.push(character);
    }
  }

  Some(active)
}

fn check_squawk_exclusions(
  migrations: &[Migration],
  squawk_path: &Path,
  violations: &mut Vec<String>,
) -> Result<()> {
  let squawk = fs::read_to_string(squawk_path)
    .with_context(|| format!("failed to read {}", path_label(squawk_path)))?;
  let actual = squawk
    .lines()
    .filter_map(|line| line.trim().strip_prefix("\"**/"))
    .filter_map(|line| line.strip_suffix("\","))
    .collect::<BTreeSet<_>>();
  let expected = migrations
    .iter()
    .filter(|migration| migration.version <= SQUAWK_BASELINE_LAST_VERSION)
    .map(|migration| migration.file.as_str())
    .collect::<BTreeSet<_>>();

  if actual != expected {
    let unexpected = actual.difference(&expected).copied().collect::<Vec<_>>();
    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    violations.push(format!(
      "Squawk exclusions must equal the immutable 001-016 baseline; unexpected: {unexpected:?}; missing: {missing:?}"
    ));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::{fs, path::Path};

  use anyhow::Result;

  use super::check;

  const MIGRATION: &str = "BEGIN;\nCOMMIT;\n";
  const MIGRATION_SHA256: &str = "cd9cf0971f79cb424a67ff0d93db0b805cb4d461ed4a2c3cf496fdf46ea7b7ae";
  const MIGRATION_NAME: &str = "019_weight_metrics_curation_gate.sql";
  const SECOND_MIGRATION_NAME: &str = "020_market_curation_gate.sql";

  #[test]
  fn valid_migration_policy_is_accepted() -> Result<()> {
    let repository = valid_repository()?;

    check(repository.path())?;

    Ok(())
  }

  #[test]
  fn changed_locked_migration_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/migrations").join(MIGRATION_NAME),
      "BEGIN;\nSELECT 1;\nCOMMIT;\n",
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("checksum"));
    Ok(())
  }

  #[test]
  fn migration_missing_from_installer_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(repository.path().join("database/install.sql"), "")?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("install.sql"));
    Ok(())
  }

  #[test]
  fn commented_installer_directive_is_ignored() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/install.sql"),
      format!("-- \\ir migrations/{MIGRATION_NAME}\n{}", installer(&[MIGRATION_NAME])),
    )?;

    check(repository.path())?;

    Ok(())
  }

  #[test]
  fn future_squawk_exclusion_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join(".squawk.toml"),
      format!("excluded_paths = [\n  \"**/{MIGRATION_NAME}\",\n]\n"),
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("Squawk"));
    Ok(())
  }

  #[test]
  fn migration_without_validation_companion_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::remove_file(
      repository.path().join("database/validation/019_weight_metrics_curation_gate_validation.sql"),
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("validation"));
    Ok(())
  }

  #[test]
  fn migration_applied_but_not_recorded_in_the_ledger_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/install.sql"),
      format!("\\ir migrations/{MIGRATION_NAME}\n"),
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("records ledger versions"));
    Ok(())
  }

  #[test]
  fn stale_history_ledger_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/validation").join(super::HISTORY_LEDGER_FILE),
      history_ledger("'018', '019'"),
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("expects versions"));
    Ok(())
  }

  #[test]
  fn commented_history_ledger_declarations_are_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/validation").join(super::HISTORY_LEDGER_FILE),
      "-- expected_versions CONSTANT TEXT[] := ARRAY['019'];\n/*\nexpected_versions CONSTANT TEXT[] := ARRAY['019'];\n*/\n",
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("must declare expected_versions"));
    Ok(())
  }

  #[test]
  fn unrelated_array_after_malformed_history_declaration_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/validation").join(super::HISTORY_LEDGER_FILE),
      "expected_versions TEXT[];\nunrelated_versions CONSTANT TEXT[] := ARRAY['019'];\n",
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("must declare expected_versions"));
    Ok(())
  }

  #[test]
  fn misordered_history_ledger_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(
      repository.path().join("database/migrations").join(SECOND_MIGRATION_NAME),
      MIGRATION,
    )?;
    fs::write(
      repository.path().join("database/validation/020_market_curation_gate_validation.sql"),
      "SELECT true;\n",
    )?;
    fs::write(
      repository.path().join("database/install.sql"),
      installer(&[MIGRATION_NAME, SECOND_MIGRATION_NAME]),
    )?;
    fs::write(
      repository.path().join("database/migrations.lock.json"),
      two_migration_lock_document(),
    )?;
    fs::write(
      repository.path().join("database/validation").join(super::HISTORY_LEDGER_FILE),
      history_ledger("'020', '019'"),
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("expects versions"));
    Ok(())
  }

  #[test]
  fn non_transactional_migration_is_rejected() -> Result<()> {
    let repository = valid_repository()?;
    fs::write(repository.path().join("database/migrations").join(MIGRATION_NAME), "SELECT 1;\n")?;
    fs::write(
      repository.path().join("database/migrations.lock.json"),
      lock_document("b4e0497804e46e0a0b0b8c31975b062152d551bac49c3c2e80932567b4085dcd"),
    )?;

    let error = require_policy_error(repository.path())?;

    assert!(error.to_string().contains("transaction"));
    Ok(())
  }

  fn valid_repository() -> Result<tempfile::TempDir> {
    let repository = tempfile::tempdir()?;
    let database = repository.path().join("database");
    fs::create_dir_all(database.join("migrations"))?;
    fs::create_dir_all(database.join("validation"))?;
    fs::write(database.join("migrations").join(MIGRATION_NAME), MIGRATION)?;
    fs::write(
      database.join("validation/019_weight_metrics_curation_gate_validation.sql"),
      "SELECT true;\n",
    )?;
    fs::write(
      database.join("validation").join(super::HISTORY_LEDGER_FILE),
      history_ledger("'019'"),
    )?;
    fs::write(database.join("install.sql"), installer(&[MIGRATION_NAME]))?;
    fs::write(database.join("migrations.lock.json"), lock_document(MIGRATION_SHA256))?;
    fs::write(repository.path().join(".squawk.toml"), "excluded_paths = []\n")?;
    Ok(repository)
  }

  fn require_policy_error(repository: &Path) -> Result<anyhow::Error> {
    match check(repository) {
      Ok(()) => anyhow::bail!("migration policy unexpectedly passed"),
      Err(error) => Ok(error),
    }
  }

  /// Builds an installer that applies `files` and records the matching versions,
  /// mirroring the shape of the checked-in `database/install.sql`.
  fn installer(files: &[&str]) -> String {
    use std::fmt::Write as _;

    let mut document = String::new();
    for file in files {
      let version = &file[..3];
      let prefix = super::LEDGER_INSERT_PREFIX;
      let _ = write!(document, "\\ir migrations/{file}\n{prefix}{version}');\n");
    }
    document
  }

  /// Builds a ledger assertion whose array literal holds `versions`, matching the
  /// shape of the checked-in `000_migration_history_validation.sql`.
  fn history_ledger(versions: &str) -> String {
    format!(
      "DO $validation$\nDECLARE\n    expected_versions CONSTANT TEXT[] := ARRAY[\n        {versions}\n    ];\nBEGIN\nEND\n$validation$;\n"
    )
  }

  fn two_migration_lock_document() -> String {
    format!(
      "{{\n  \"schema_version\": 1,\n  \"migrations\": [\n    {{\n      \"file\": \"{MIGRATION_NAME}\",\n      \"sha256\": \"{MIGRATION_SHA256}\"\n    }},\n    {{\n      \"file\": \"{SECOND_MIGRATION_NAME}\",\n      \"sha256\": \"{MIGRATION_SHA256}\"\n    }}\n  ]\n}}\n"
    )
  }

  fn lock_document(sha256: &str) -> String {
    format!(
      "{{\n  \"schema_version\": 1,\n  \"migrations\": [\n    {{\n      \"file\": \"{MIGRATION_NAME}\",\n      \"sha256\": \"{sha256}\"\n    }}\n  ]\n}}\n"
    )
  }
}
