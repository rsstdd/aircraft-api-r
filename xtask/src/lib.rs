#![allow(clippy::print_stdout)]

pub mod boundaries;
pub mod migrations;
pub mod snapshots;

use std::{
  fs,
  path::{Path, PathBuf},
  process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

pub use snapshots::{SnapshotOptions, snapshots};

#[derive(Debug, Clone, Copy)]
pub struct InstallDepsOptions {
  pub check_only: bool,
}

#[derive(Debug, Clone)]
pub struct GenerateDocsOptions {
  pub output: PathBuf,
  pub check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
  pub program: String,
  pub args: Vec<String>,
  pub current_dir: Option<PathBuf>,
}

impl CommandSpec {
  fn new(program: &str, args: &[&str]) -> Self {
    Self {
      program: program.to_owned(),
      args: args.iter().map(|arg| (*arg).to_owned()).collect(),
      current_dir: None,
    }
  }

  fn in_dir(mut self, directory: &Path) -> Self {
    self.current_dir = Some(directory.to_path_buf());
    self
  }
}

pub trait Runner {
  fn succeeds(&self, command: &CommandSpec) -> bool;
  fn run(&self, command: &CommandSpec) -> Result<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct SystemRunner;

impl Runner for SystemRunner {
  fn succeeds(&self, command: &CommandSpec) -> bool {
    build_command(command)
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status()
      .is_ok_and(|status| status.success())
  }

  fn run(&self, command: &CommandSpec) -> Result<()> {
    let status = build_command(command)
      .status()
      .with_context(|| format!("failed to start `{}`", display_command(command)))?;
    if !status.success() {
      bail!("`{}` exited with status {status}", display_command(command));
    }
    Ok(())
  }
}

fn build_command(spec: &CommandSpec) -> Command {
  let mut command = Command::new(&spec.program);
  command.args(&spec.args);
  if let Some(directory) = &spec.current_dir {
    command.current_dir(directory);
  }
  command
}

fn display_command(spec: &CommandSpec) -> String {
  std::iter::once(spec.program.as_str())
    .chain(spec.args.iter().map(String::as_str))
    .collect::<Vec<_>>()
    .join(" ")
}

struct ManagedTool {
  name: &'static str,
  probe: CommandSpec,
  install: CommandSpec,
}

pub fn install_deps(runner: &impl Runner, options: InstallDepsOptions) -> Result<()> {
  let prerequisites = [
    ("cargo", CommandSpec::new("cargo", &["--version"])),
    ("rustup", CommandSpec::new("rustup", &["--version"])),
    ("Docker Compose", CommandSpec::new("docker", &["compose", "version"])),
    ("PostgreSQL client", CommandSpec::new("psql", &["--version"])),
    ("npm", CommandSpec::new("npm", &["--version"])),
  ];
  let missing_prerequisites = prerequisites
    .iter()
    .filter(|(_, probe)| !runner.succeeds(probe))
    .map(|(name, _)| *name)
    .collect::<Vec<_>>();
  if !missing_prerequisites.is_empty() {
    bail!(
      "missing platform prerequisites: {}. Install them with the platform package manager and rerun this command",
      missing_prerequisites.join(", ")
    );
  }

  let tools = managed_tools();
  let missing = tools.iter().filter(|tool| !runner.succeeds(&tool.probe)).collect::<Vec<_>>();
  if missing.is_empty() {
    println!("All development tools are available.");
    return Ok(());
  }
  if options.check_only {
    bail!(
      "missing development tools: {}",
      missing.iter().map(|tool| tool.name).collect::<Vec<_>>().join(", ")
    );
  }

  for tool in missing {
    println!("Installing {}...", tool.name);
    runner.run(&tool.install)?;
    if !runner.succeeds(&tool.probe) {
      bail!("{} is still unavailable after installation", tool.name);
    }
  }
  println!("Development tools are ready.");
  Ok(())
}

fn managed_tools() -> Vec<ManagedTool> {
  vec![
    ManagedTool {
      name: "rustfmt",
      probe: CommandSpec::new("cargo", &["fmt", "--version"]),
      install: CommandSpec::new("rustup", &["component", "add", "rustfmt"]),
    },
    ManagedTool {
      name: "clippy",
      probe: CommandSpec::new("cargo", &["clippy", "--version"]),
      install: CommandSpec::new("rustup", &["component", "add", "clippy"]),
    },
    cargo_tool("just", &["just", "--version"], "just", &[]),
    cargo_tool("cargo-nextest", &["cargo", "nextest", "--version"], "cargo-nextest", &[]),
    cargo_tool("cargo-audit", &["cargo", "audit", "--version"], "cargo-audit", &[]),
    cargo_tool("cargo-deny", &["cargo", "deny", "--version"], "cargo-deny", &[]),
    cargo_tool(
      "sqlx-cli",
      &["cargo", "sqlx", "--version"],
      "sqlx-cli",
      &["--no-default-features", "--features", "rustls,postgres"],
    ),
  ]
}

fn cargo_tool(
  name: &'static str,
  probe: &[&str],
  package: &str,
  extra_install_args: &[&str],
) -> ManagedTool {
  let mut args = vec!["install", "--locked", package];
  args.extend_from_slice(extra_install_args);
  ManagedTool {
    name,
    probe: CommandSpec::new(probe[0], &probe[1..]),
    install: CommandSpec::new("cargo", &args),
  }
}

pub fn deny(runner: &impl Runner, workspace_root: &Path) -> Result<()> {
  runner.run(&CommandSpec::new("cargo", &["deny", "--locked", "check"]).in_dir(workspace_root))
}

pub fn generate_docs(workspace_root: &Path, options: GenerateDocsOptions) -> Result<()> {
  let output =
    if options.output.is_absolute() { options.output } else { workspace_root.join(options.output) };
  let mut rendered = serde_json::to_string_pretty(&aircraft_api::openapi())?;
  rendered.push('\n');

  if options.check {
    let existing = fs::read_to_string(&output)
      .with_context(|| format!("OpenAPI document is missing: {}", output.display()))?;
    if existing != rendered {
      bail!("OpenAPI document is stale: {}. Run `just generate-docs`", output.display());
    }
    println!("OpenAPI document is up to date: {}", output.display());
    return Ok(());
  }

  if let Some(parent) = output.parent() {
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
  }
  fs::write(&output, rendered).with_context(|| format!("failed to write {}", output.display()))?;
  println!("Wrote OpenAPI document to {}", output.display());
  Ok(())
}

pub fn workspace_root() -> Result<PathBuf> {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .parent()
    .map(Path::to_path_buf)
    .context("xtask must be located directly below the workspace root")
}

#[cfg(test)]
mod tests {
  use std::{cell::RefCell, collections::VecDeque};

  use super::*;

  #[derive(Default)]
  struct FakeRunner {
    probe_results: RefCell<VecDeque<bool>>,
    commands: RefCell<Vec<CommandSpec>>,
  }

  impl FakeRunner {
    fn with_probes(results: impl IntoIterator<Item = bool>) -> Self {
      Self {
        probe_results: RefCell::new(results.into_iter().collect()),
        commands: RefCell::default(),
      }
    }
  }

  impl Runner for FakeRunner {
    fn succeeds(&self, command: &CommandSpec) -> bool {
      self.commands.borrow_mut().push(command.clone());
      self.probe_results.borrow_mut().pop_front().unwrap_or(true)
    }

    fn run(&self, command: &CommandSpec) -> Result<()> {
      self.commands.borrow_mut().push(command.clone());
      Ok(())
    }
  }

  #[test]
  fn install_deps_installs_only_missing_managed_tools() -> Result<()> {
    let runner = FakeRunner::with_probes([
      true, true, true, true, true, true, false, true, true, true, true, true, true,
    ]);

    install_deps(&runner, InstallDepsOptions { check_only: false })?;

    let commands = runner.commands.borrow();
    assert!(commands.iter().any(|command| {
      command.program == "rustup" && command.args == ["component", "add", "clippy"]
    }));
    assert!(!commands.iter().any(|command| {
      command.program == "rustup" && command.args == ["component", "add", "rustfmt"]
    }));
    Ok(())
  }

  #[test]
  fn install_deps_check_reports_missing_tools_without_installing() -> Result<()> {
    let runner = FakeRunner::with_probes([
      true, true, true, true, true, true, true, false, true, true, true, true,
    ]);

    let error = install_deps(&runner, InstallDepsOptions { check_only: true })
      .err()
      .context("check mode should report the missing tool")?
      .to_string();

    assert!(error.contains("just"));
    assert!(!runner.commands.borrow().iter().any(|command| {
      command.program == "cargo" && command.args.first().is_some_and(|arg| arg == "install")
    }));
    Ok(())
  }

  #[test]
  fn deny_checks_the_locked_workspace_from_its_root() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let runner = FakeRunner::default();

    deny(&runner, temp.path())?;

    let commands = runner.commands.borrow();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, "cargo");
    assert_eq!(commands[0].args, ["deny", "--locked", "check"]);
    assert_eq!(commands[0].current_dir.as_deref(), Some(temp.path()));
    Ok(())
  }

  #[test]
  fn generate_docs_writes_and_checks_the_same_document() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let relative_output = PathBuf::from("generated/openapi.json");

    generate_docs(
      temp.path(),
      GenerateDocsOptions { output: relative_output.clone(), check: false },
    )?;
    generate_docs(
      temp.path(),
      GenerateDocsOptions { output: relative_output.clone(), check: true },
    )?;

    let document = fs::read_to_string(temp.path().join(relative_output))?;
    assert!(document.contains("\"/health\""));
    assert!(document.contains("\"HealthResponse\""));
    Ok(())
  }

  #[test]
  fn generate_docs_check_rejects_stale_output() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let output = temp.path().join("openapi.json");
    fs::write(&output, "{}\n")?;

    let error = generate_docs(temp.path(), GenerateDocsOptions { output, check: true })
      .err()
      .context("check mode should reject stale output")?
      .to_string();

    assert!(error.contains("stale"));
    Ok(())
  }
}
