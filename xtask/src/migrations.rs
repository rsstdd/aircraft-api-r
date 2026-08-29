use std::{
  collections::{BTreeMap, BTreeSet},
  ffi::OsStr,
  fs,
  path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LOCK_SCHEMA_VERSION: u32 = 1;
const SQUAWK_BASELINE_LAST_VERSION: u16 = 18;
const VALIDATION_REQUIRED_FROM_VERSION: u16 = 17;

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
    fs::read_dir(directory).with_context(|| format!("failed to read {}", directory.display()))?;
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
  let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
  let lock: MigrationLock = serde_json::from_slice(&bytes)
    .with_context(|| format!("{} is not valid JSON", path.display()))?;
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
      .with_context(|| format!("failed to read {}", migration.path.display()))?;
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
      .with_context(|| format!("failed to read {}", migration.path.display()))?;
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
    .with_context(|| format!("failed to read {}", installer_path.display()))?;
  let mut previous_position = None;
  let expected =
    migrations.iter().map(|migration| migration.file.as_str()).collect::<BTreeSet<_>>();

  for migration in migrations {
    let needle = format!("\\ir migrations/{}", migration.file);
    let positions =
      installer.match_indices(&needle).map(|(position, _)| position).collect::<Vec<_>>();
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

  for line in installer.lines().map(str::trim) {
    let Some(file) = line.strip_prefix("\\ir migrations/") else {
      continue;
    };
    if !expected.contains(file) {
      violations.push(format!("database/install.sql references unknown migration {file}"));
    }
  }
  Ok(())
}

fn check_validations(
  migrations: &[Migration],
  validations_directory: &Path,
  violations: &mut Vec<String>,
) -> Result<()> {
  let validation_files = fs::read_dir(validations_directory)
    .with_context(|| format!("failed to read {}", validations_directory.display()))?
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

fn check_squawk_exclusions(
  migrations: &[Migration],
  squawk_path: &Path,
  violations: &mut Vec<String>,
) -> Result<()> {
  let squawk = fs::read_to_string(squawk_path)
    .with_context(|| format!("failed to read {}", squawk_path.display()))?;
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
      "Squawk exclusions must equal the immutable 001-018 baseline; unexpected: {unexpected:?}; missing: {missing:?}"
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
    fs::write(database.join("install.sql"), format!("\\ir migrations/{MIGRATION_NAME}\n"))?;
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

  fn lock_document(sha256: &str) -> String {
    format!(
      "{{\n  \"schema_version\": 1,\n  \"migrations\": [\n    {{\n      \"file\": \"{MIGRATION_NAME}\",\n      \"sha256\": \"{sha256}\"\n    }}\n  ]\n}}\n"
    )
  }
}
