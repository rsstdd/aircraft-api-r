use std::{collections::BTreeSet, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const INTERNAL_RULES: &[(&str, &[&str])] = &[
  ("aircraft_domain", &[]),
  ("aircraft_app", &["aircraft_domain"]),
  ("aircraft_api", &["aircraft_app", "aircraft_domain"]),
  ("aircraft_db", &["aircraft_app", "aircraft_domain"]),
  ("aircraft_ingest", &["aircraft_app", "aircraft_domain"]),
  ("aircraft_config", &[]),
  ("aircraft_observability", &[]),
  ("aircraft_testsupport", &["aircraft_app"]),
];

const EXTERNAL_RULES: &[(&str, &[&str])] = &[
  (
    "aircraft_domain",
    &["axum", "config", "sqlx-core", "sqlx-postgres", "tokio", "tower", "tracing"],
  ),
  ("aircraft_app", &["axum", "config", "sqlx-core", "sqlx-postgres", "tower"]),
  ("aircraft_api", &["sqlx-core", "sqlx-postgres"]),
  ("aircraft_ingest", &["sqlx-core", "sqlx-postgres"]),
];

#[derive(Debug, Deserialize)]
struct Metadata {
  packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
struct Package {
  name: String,
  dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
  name: String,
  kind: Option<String>,
  path: Option<String>,
}

pub fn check(workspace_root: &Path) -> Result<()> {
  let output = Command::new("cargo")
    .args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
    .current_dir(workspace_root)
    .output()
    .context("failed to start `cargo metadata`")?;
  if !output.status.success() {
    bail!("`cargo metadata` failed: {}", String::from_utf8_lossy(&output.stderr).trim());
  }

  let metadata: Metadata = serde_json::from_slice(&output.stdout)
    .context("cargo metadata returned an invalid JSON document")?;
  let violations = find_violations(&metadata);
  if !violations.is_empty() {
    bail!("dependency boundaries failed:\n{}", violations.join("\n"));
  }

  println!("Dependency boundaries are valid.");
  Ok(())
}

fn find_violations(metadata: &Metadata) -> Vec<String> {
  let workspace_packages =
    metadata.packages.iter().map(|package| package.name.as_str()).collect::<BTreeSet<_>>();
  let mut violations = Vec::new();

  for package in &metadata.packages {
    for dependency in
      package.dependencies.iter().filter(|dependency| dependency.kind.as_deref() != Some("dev"))
    {
      if dependency.name == "aircraft_testsupport" && package.name != "aircraft_testsupport" {
        violations
          .push(format!("{} must use aircraft_testsupport only as a dev-dependency", package.name));
        continue;
      }

      if dependency.path.is_some() && workspace_packages.contains(dependency.name.as_str()) {
        check_internal_dependency(package, dependency, &mut violations);
      } else {
        check_external_dependency(package, dependency, &mut violations);
      }
    }
  }

  violations.sort();
  violations.dedup();
  violations
}

fn check_internal_dependency(
  package: &Package,
  dependency: &Dependency,
  violations: &mut Vec<String>,
) {
  let Some((_, allowed)) =
    INTERNAL_RULES.iter().find(|(package_name, _)| *package_name == package.name)
  else {
    return;
  };
  if !allowed.contains(&dependency.name.as_str()) {
    violations.push(format!("{} must not depend on {}", package.name, dependency.name));
  }
}

fn check_external_dependency(
  package: &Package,
  dependency: &Dependency,
  violations: &mut Vec<String>,
) {
  let Some((_, forbidden)) =
    EXTERNAL_RULES.iter().find(|(package_name, _)| *package_name == package.name)
  else {
    return;
  };
  if forbidden.contains(&dependency.name.as_str()) {
    violations.push(format!("{} must not depend on {}", package.name, dependency.name));
  }
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;

  #[test]
  fn valid_hexagonal_dependencies_are_accepted() -> Result<()> {
    let metadata: Metadata = serde_json::from_value(json!({
      "packages": [
        {
          "name": "aircraft_domain",
          "dependencies": [dependency("serde", None, None)]
        },
        {
          "name": "aircraft_app",
          "dependencies": [dependency(
            "aircraft_domain",
            None,
            Some("/workspace/domain")
          )]
        },
        {
          "name": "aircraft_db",
          "dependencies": [
            dependency("aircraft_app", None, Some("/workspace/app")),
            dependency("aircraft_testsupport", Some("dev"), Some("/workspace/testsupport"))
          ]
        },
        {
          "name": "aircraft_testsupport",
          "dependencies": [dependency("aircraft_app", None, Some("/workspace/app"))]
        }
      ]
    }))?;

    assert!(find_violations(&metadata).is_empty());
    Ok(())
  }

  #[test]
  fn forbidden_framework_and_internal_dependencies_are_reported() -> Result<()> {
    let metadata: Metadata = serde_json::from_value(json!({
      "packages": [
        {
          "name": "aircraft_domain",
          "dependencies": [dependency("tokio", Some("build"), None)]
        },
        {
          "name": "aircraft_api",
          "dependencies": [dependency("aircraft_db", None, Some("/workspace/db"))]
        },
        {
          "name": "aircraft_db",
          "dependencies": [dependency(
            "aircraft_testsupport",
            None,
            Some("/workspace/testsupport")
          )]
        },
        { "name": "aircraft_testsupport", "dependencies": [] }
      ]
    }))?;

    assert_eq!(
      find_violations(&metadata),
      [
        "aircraft_api must not depend on aircraft_db",
        "aircraft_db must use aircraft_testsupport only as a dev-dependency",
        "aircraft_domain must not depend on tokio",
      ]
    );
    Ok(())
  }

  fn dependency(name: &str, kind: Option<&str>, path: Option<&str>) -> serde_json::Value {
    json!({ "name": name, "kind": kind, "path": path })
  }
}
