use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use xtask::{GenerateDocsOptions, InstallDepsOptions, SnapshotOptions, SystemRunner};

#[derive(Parser)]
#[command(name = "cargo xtask")]
#[command(about = "Workspace automation commands for the Aircraft Management Engine", long_about = None)]
struct Cli {
  #[command(subcommand)]
  command: Commands,
}

#[derive(Subcommand)]
enum Commands {
  /// Enforce dependency direction between architectural layers
  Boundaries,
  /// Check and install the development tools used by repository recipes
  InstallDeps(InstallDepsArgs),
  /// Check dependency advisories, licenses, bans, and sources
  Deny,
  /// Reconcile the `deny.toml` build-script allowlist with the lockfile
  DenyPins(DenyPinsArgs),
  /// Verify immutable migration history and database integration contracts
  Migrations,
  /// Compile API contracts and write an API schema document
  GenerateDocs(GenerateDocsArgs),
  /// Check ingestion output against the committed golden snapshots
  Snapshots(SnapshotArgs),
}

#[derive(Debug, Args)]
struct DenyPinsArgs {
  /// Rewrite the allowlist instead of only reporting the drift
  #[arg(long)]
  fix: bool,
}

#[derive(Debug, Args)]
struct SnapshotArgs {
  /// `PlanePHD` JSON imported before snapshotting
  #[arg(long, default_value = "tests/fixtures/planephd_minimal.json")]
  fixture: PathBuf,

  /// Leave the database running after the report for manual inspection
  #[arg(long)]
  keep: bool,

  /// Rewrite the golden snapshots instead of comparing against them
  #[arg(long)]
  update: bool,
}

#[derive(Debug, Args)]
struct InstallDepsArgs {
  /// Report missing tools without installing them
  #[arg(long)]
  check: bool,
}

#[derive(Debug, Args)]
struct GenerateDocsArgs {
  /// Destination for the generated API schema JSON document
  #[arg(long, default_value = "docs/openapi.json")]
  output: PathBuf,

  /// Fail when the destination is missing or stale instead of writing it
  #[arg(long)]
  check: bool,
}

fn main() -> anyhow::Result<()> {
  let cli = Cli::parse();
  let workspace_root = xtask::workspace_root()?;
  let runner = SystemRunner;

  match cli.command {
    Commands::Boundaries => xtask::boundaries::check(&workspace_root),
    Commands::InstallDeps(args) => {
      xtask::install_deps(&runner, InstallDepsOptions { check_only: args.check })
    }
    Commands::Deny => xtask::deny(&runner, &workspace_root),
    Commands::DenyPins(args) => {
      xtask::deny_pins::check(&workspace_root, &xtask::deny_pins::Options { fix: args.fix })
    }
    Commands::Migrations => xtask::migrations::check(&workspace_root),
    Commands::GenerateDocs(args) => xtask::generate_docs(
      &workspace_root,
      GenerateDocsOptions { output: args.output, check: args.check },
    ),
    Commands::Snapshots(args) => xtask::snapshots(
      &workspace_root,
      &SnapshotOptions { fixture: args.fixture, keep: args.keep, update: args.update },
    ),
  }
}
