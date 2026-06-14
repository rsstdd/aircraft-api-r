use anyhow::Result;
use std::env;
use std::process::Command;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);

    match (args.next().as_deref(), args.next().as_deref()) {
        (Some("db"), Some("validate")) => run(
            "psql",
            &["-f", "database/validation/001_extensions_schemas_domains_triggers_validation.sql"],
        )?,
        (Some("db"), Some("install")) => run("psql", &["-f", "database/install.sql"])?,
        _ => {
            eprintln!("usage:");
            eprintln!("  cargo run -p xtask -- db install");
            eprintln!("  cargo run -p xtask -- db validate");
        }
    }

    Ok(())
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program).args(args).status()?;

    if !status.success() {
        anyhow::bail!("{program} failed with status {status}");
    }

    Ok(())
}
