#![allow(clippy::print_stdout)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};

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
    pub env: Vec<(String, String)>,
}

impl CommandSpec {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            current_dir: None,
            env: Vec::new(),
        }
    }

    fn in_dir(mut self, directory: &Path) -> Self {
        self.current_dir = Some(directory.to_path_buf());
        self
    }

    fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
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
    command.args(&spec.args).envs(spec.env.iter().cloned());
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

pub fn prepare_sqlx(runner: &impl Runner, workspace_root: &Path) -> Result<()> {
    let database_url = env::var("MIGRATION_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .context("MIGRATION_DATABASE_URL or DATABASE_URL must be set")?;
    prepare_sqlx_with_url(runner, workspace_root, &database_url)
}

fn prepare_sqlx_with_url(
    runner: &impl Runner,
    workspace_root: &Path,
    database_url: &str,
) -> Result<()> {
    let validation_dir = workspace_root.join("database/validation");
    let mut validation_files = fs::read_dir(&validation_dir)
        .with_context(|| format!("failed to read {}", validation_dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    validation_files.retain(|path| path.extension().is_some_and(|extension| extension == "sql"));
    validation_files.sort();
    if validation_files.is_empty() {
        bail!("no schema validation files found in {}", validation_dir.display());
    }

    for path in validation_files {
        println!("Validating schema with {}...", path.display());
        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("validation path is not valid UTF-8: {}", path.display()))?;
        runner.run(
            &CommandSpec::new("psql", &["-X", "-v", "ON_ERROR_STOP=1", "-f", path])
                .with_env("PGDATABASE", database_url),
        )?;
    }

    runner.run(
        &CommandSpec::new("cargo", &["sqlx", "prepare", "--workspace", "--", "--all-targets"])
            .in_dir(workspace_root)
            .with_env("DATABASE_URL", database_url)
            .with_env("SQLX_OFFLINE", "false"),
    )?;
    println!("SQLx offline metadata is up to date.");
    Ok(())
}

pub fn generate_docs(workspace_root: &Path, options: GenerateDocsOptions) -> Result<()> {
    let output = if options.output.is_absolute() {
        options.output
    } else {
        workspace_root.join(options.output)
    };
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
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&output, rendered)
        .with_context(|| format!("failed to write {}", output.display()))?;
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
            true, true, true, true, true, false, true, true, true, true, true, true,
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
            true, true, true, true, true, true, false, true, true, true, true,
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
    fn prepare_sqlx_validates_sql_files_in_order_then_prepares_metadata() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let validation_dir = temp.path().join("database/validation");
        fs::create_dir_all(&validation_dir)?;
        fs::write(validation_dir.join("002_second.sql"), "SELECT 2;")?;
        fs::write(validation_dir.join("001_first.sql"), "SELECT 1;")?;
        fs::write(validation_dir.join("README.md"), "ignored")?;
        let runner = FakeRunner::default();

        prepare_sqlx_with_url(&runner, temp.path(), "postgres://test")?;

        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 3);
        assert!(commands[0].args.last().is_some_and(|arg| arg.ends_with("001_first.sql")));
        assert!(commands[1].args.last().is_some_and(|arg| arg.ends_with("002_second.sql")));
        assert_eq!(commands[0].env, [("PGDATABASE".to_owned(), "postgres://test".to_owned())]);
        assert!(!commands[0].args.iter().any(|arg| arg == "postgres://test"));
        assert_eq!(commands[2].program, "cargo");
        assert_eq!(commands[2].args, ["sqlx", "prepare", "--workspace", "--", "--all-targets"]);
        assert_eq!(
            commands[2].env,
            [
                ("DATABASE_URL".to_owned(), "postgres://test".to_owned()),
                ("SQLX_OFFLINE".to_owned(), "false".to_owned()),
            ]
        );
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
