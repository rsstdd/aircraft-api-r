#![allow(clippy::print_stdout, clippy::print_stderr)]

// xtask/src/main.rs
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

#[derive(Parser)]
#[command(name = "cargo xtask")]
#[command(about = "Workspace automation commands for the Aircraft Management Engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run canonical database schema migrations
    DbMigrate,
    /// Wipe database, re-apply schemas, and inject reference seed data
    DbReset,
    /// Scan route macros and output openapi.json file to disk
    GenerateDocs,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::DbMigrate => {
            println!("Executing database migrations...");
            // Real logic: Ingest settings via aircraft_config, connect via aircraft_db, execute
            // SQL files
            execute_migrations();
        }
        Commands::DbReset => {
            println!("Resetting database to pristine state...");
            execute_db_reset();
        }
        Commands::GenerateDocs => {
            println!("Generating OpenAPI JSON specification...");
            execute_docs_generation();
        }
    }

    Ok(())
}

const fn execute_migrations() {
    // You can call your real configuration management crate here:
    // let settings = aircraft_config::load_settings()?;
    // sqlx::migrate!('../database/migrations').run(&settings.db.pool).await?;
}

const fn execute_db_reset() {
    // Logic to drop schemas, recreate, and run seed data scripts
}

const fn execute_docs_generation() {
    // Logic to write your Utoipa openapi string to database/openapi.json
}
