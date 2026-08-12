use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use xtask::{GenerateDocsOptions, InstallDepsOptions, SystemRunner};

#[derive(Parser)]
#[command(name = "cargo xtask")]
#[command(about = "Workspace automation commands for the Aircraft Management Engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check and install the development tools used by repository recipes
    InstallDeps(InstallDepsArgs),
    /// Check dependency advisories, licenses, bans, and sources
    Deny,
    /// Compile API contracts and write an API schema document
    GenerateDocs(GenerateDocsArgs),
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
        Commands::InstallDeps(args) => {
            xtask::install_deps(&runner, InstallDepsOptions { check_only: args.check })
        }
        Commands::Deny => xtask::deny(&runner, &workspace_root),
        Commands::GenerateDocs(args) => xtask::generate_docs(
            &workspace_root,
            GenerateDocsOptions { output: args.output, check: args.check },
        ),
    }
}
