// xtask/src/main.rs
use clap::{Parser, Subcommand};

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
            // Real logic: Ingest settings via aircraft_config, connect via aircraft_db, execute sql files
            execute_migrations().await?;
        }
        Commands::DbReset => {
            println!("Resetting database to pristine state...");
            execute_db_reset().await?;
        }
        Commands::GenerateDocs => {
            println!("Generating OpenAPI JSON specification...");
            execute_docs_generation()?;
        }
    }

    Ok(())
}

async fn execute_migrations() -> anyhow::Result<()> {
    // You can call your real configuration management crate here:
    // let settings = aircraft_config::load_settings()?;
    // sqlx::migrate!("../database/migrations").run(&settings.db.pool).await?;
    Ok(())
}

async fn execute_db_reset() -> anyhow::Result<()> {
    // Logic to drop schemas, recreate, and run seed data scripts
    Ok(())
}

fn execute_docs_generation() -> anyhow::Result<()> {
    // Logic to write your Utoipa openapi string to database/openapi.json
    Ok(())
}