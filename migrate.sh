# =============================================================================
# 0. Create a safety branch and snapshot current state
# =============================================================================

git status
git checkout -b chore/restructure-project-layout

mkdir -p archive/restructure-snapshots
tree -a -I 'target|.git' > archive/restructure-snapshots/tree-before.txt
cp Cargo.toml archive/restructure-snapshots/Cargo.toml.before


# =============================================================================
# 1. Define safe move helpers
# =============================================================================

move_path() {
  local src="$1"
  local dest="$2"

  if [ ! -e "$src" ]; then
    echo "skip: $src does not exist"
    return 0
  fi

  mkdir -p "$(dirname "$dest")"

  if git ls-files --error-unmatch "$src" >/dev/null 2>&1; then
    git mv "$src" "$dest"
  else
    mv "$src" "$dest"
  fi
}

move_dir() {
  local src="$1"
  local dest="$2"

  if [ ! -d "$src" ]; then
    echo "skip: $src does not exist"
    return 0
  fi

  mkdir -p "$(dirname "$dest")"

  git mv "$src" "$dest" 2>/dev/null || mv "$src" "$dest"
}


# =============================================================================
# 2. Move root binary source into apps/server
# =============================================================================

mkdir -p apps/server/src

move_path src/main.rs apps/server/src/main.rs
move_path src/lib.rs apps/server/src/lib.rs
move_path src/api_routes.rs apps/server/src/api_routes.rs

# Remove old root src if empty.
rmdir src 2>/dev/null || true


# =============================================================================
# 3. Rename useful existing crates to domain-aligned names
# =============================================================================

move_dir crates/api crates/aircraft_api
move_dir crates/db_schema crates/aircraft_db


# =============================================================================
# 4. Archive template-derived crates instead of deleting them
# =============================================================================

mkdir -p archive/template_crates

move_dir crates/api_common archive/template_crates/api_common
move_dir crates/api_crud archive/template_crates/api_crud
move_dir crates/routes archive/template_crates/routes
move_dir crates/websocket archive/template_crates/websocket


# =============================================================================
# 5. Split the old utils crate into better boundaries
# =============================================================================

mkdir -p \
  crates/aircraft_config/src \
  crates/aircraft_observability/src \
  crates/aircraft_api/src/middleware \
  archive/template_crates/utils

# Move settings into config crate.
move_dir crates/utils/settings crates/aircraft_config/src/settings

# Move rate limiting into API middleware.
move_dir crates/utils/rate_limit crates/aircraft_api/src/middleware/rate_limit

# Move likely reusable source files.
move_path crates/utils/src/version.rs crates/aircraft_observability/src/version.rs
move_path crates/utils/src/claims.rs crates/aircraft_api/src/claims.rs
move_path crates/utils/src/email.rs archive/template_crates/utils/email.rs
move_path crates/utils/src/apub.rs archive/template_crates/utils/apub.rs
move_path crates/utils/src/utils.rs archive/template_crates/utils/utils.rs
move_path crates/utils/src/lib.rs archive/template_crates/utils/lib.rs
move_path crates/utils/Cargo.toml archive/template_crates/utils/Cargo.toml

# Remove old utils directory if empty.
find crates/utils -type d -empty -delete 2>/dev/null || true


# =============================================================================
# 6. Create new clean architecture crates
# =============================================================================

mkdir -p \
  crates/aircraft_app/src/services \
  crates/aircraft_domain/src \
  crates/aircraft_ingest/src \
  crates/aircraft_config/src \
  crates/aircraft_observability/src \
  crates/aircraft_db/src/repositories \
  crates/aircraft_db/src/models \
  crates/aircraft_db/src/schema \
  crates/aircraft_api/src/routes \
  crates/aircraft_api/src/dto \
  crates/aircraft_api/src/middleware


# =============================================================================
# 7. Add Cargo.toml files for new crates where missing
# =============================================================================

cat > crates/aircraft_app/Cargo.toml <<'EOF'
[package]
name = "aircraft_app"
version = "0.1.0"
edition = "2024"

[dependencies]
aircraft_domain = { path = "../aircraft_domain" }
aircraft_db = { path = "../aircraft_db" }
anyhow = "1"
thiserror = "2"
EOF

cat > crates/aircraft_domain/Cargo.toml <<'EOF'
[package]
name = "aircraft_domain"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
EOF

cat > crates/aircraft_ingest/Cargo.toml <<'EOF'
[package]
name = "aircraft_ingest"
version = "0.1.0"
edition = "2024"

[dependencies]
aircraft_domain = { path = "../aircraft_domain" }
aircraft_db = { path = "../aircraft_db" }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
EOF

cat > crates/aircraft_config/Cargo.toml <<'EOF'
[package]
name = "aircraft_config"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
config = "0.15"
thiserror = "2"
EOF

cat > crates/aircraft_observability/Cargo.toml <<'EOF'
[package]
name = "aircraft_observability"
version = "0.1.0"
edition = "2024"

[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
EOF


# =============================================================================
# 8. Create minimal lib.rs files for new crates
# =============================================================================

cat > crates/aircraft_app/src/lib.rs <<'EOF'
pub mod services;
EOF

cat > crates/aircraft_app/src/services/mod.rs <<'EOF'
pub mod aircraft_search;
pub mod comparison;
pub mod ingestion;
EOF

cat > crates/aircraft_app/src/services/aircraft_search.rs <<'EOF'
// Application service boundary for aircraft search use cases.
EOF

cat > crates/aircraft_app/src/services/comparison.rs <<'EOF'
// Application service boundary for mission-profile comparison use cases.
EOF

cat > crates/aircraft_app/src/services/ingestion.rs <<'EOF'
// Application service boundary for controlled import and normalization workflows.
EOF

cat > crates/aircraft_domain/src/lib.rs <<'EOF'
pub mod aircraft;
pub mod manufacturer;
pub mod mission;
pub mod units;
pub mod validation;
EOF

cat > crates/aircraft_domain/src/aircraft.rs <<'EOF'
// Aircraft domain types.
EOF

cat > crates/aircraft_domain/src/manufacturer.rs <<'EOF'
// Manufacturer domain types.
EOF

cat > crates/aircraft_domain/src/mission.rs <<'EOF'
// Mission profile domain types.
EOF

cat > crates/aircraft_domain/src/units.rs <<'EOF'
// Unit and normalized-measurement domain types.
EOF

cat > crates/aircraft_domain/src/validation.rs <<'EOF'
// Domain-level validation rules.
EOF

cat > crates/aircraft_ingest/src/lib.rs <<'EOF'
pub mod json_import;
pub mod normalization;
pub mod validation;
EOF

cat > crates/aircraft_ingest/src/json_import.rs <<'EOF'
// Server-side JSON import entrypoints.
EOF

cat > crates/aircraft_ingest/src/normalization.rs <<'EOF'
// Source-data normalization rules.
EOF

cat > crates/aircraft_ingest/src/validation.rs <<'EOF'
// Ingest-time validation rules.
EOF

cat > crates/aircraft_config/src/lib.rs <<'EOF'
pub mod settings;
EOF

cat > crates/aircraft_observability/src/lib.rs <<'EOF'
pub mod logging;
pub mod version;
EOF

cat > crates/aircraft_observability/src/logging.rs <<'EOF'
pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}
EOF


# =============================================================================
# 9. Normalize aircraft_db crate structure
# =============================================================================

# Keep existing generated schema if present.
if [ -f crates/aircraft_db/src/schema.rs ]; then
  move_path crates/aircraft_db/src/schema.rs crates/aircraft_db/src/schema/generated.rs
fi

cat > crates/aircraft_db/src/schema/mod.rs <<'EOF'
pub mod generated;

// Future split generated schema modules can live here:
// pub mod aircraft_core;
// pub mod aircraft_specs;
// pub mod aircraft_market;
// pub mod aircraft_compare;
EOF

cat > crates/aircraft_db/src/repositories/mod.rs <<'EOF'
pub mod aircraft_repository;
pub mod comparison_repository;
EOF

cat > crates/aircraft_db/src/repositories/aircraft_repository.rs <<'EOF'
// Database access for aircraft read/query operations.
EOF

cat > crates/aircraft_db/src/repositories/comparison_repository.rs <<'EOF'
// Database access for comparison/scoring operations.
EOF

cat > crates/aircraft_db/src/models/mod.rs <<'EOF'
pub mod aircraft;
EOF

cat > crates/aircraft_db/src/models/aircraft.rs <<'EOF'
// Diesel-backed database row models for aircraft entities.
EOF

# Replace lib.rs only if it is empty or minimal; otherwise save it first.
cp crates/aircraft_db/src/lib.rs archive/restructure-snapshots/aircraft_db.lib.rs.before 2>/dev/null || true

cat > crates/aircraft_db/src/lib.rs <<'EOF'
pub mod impls;
pub mod models;
pub mod newtypes;
pub mod repositories;
pub mod schema;
pub mod source;
pub mod traits;
EOF


# =============================================================================
# 10. Normalize aircraft_api crate structure
# =============================================================================

mkdir -p crates/aircraft_api/src/routes crates/aircraft_api/src/dto crates/aircraft_api/src/middleware

# If legacy route files exist from the archived routes crate, move useful ones back.
move_path archive/template_crates/routes/src/general.rs crates/aircraft_api/src/routes/health.rs
move_path archive/template_crates/routes/src/images.rs crates/aircraft_api/src/routes/images.rs

cat > crates/aircraft_api/src/routes/mod.rs <<'EOF'
pub mod aircraft;
pub mod compare;
pub mod health;
pub mod manufacturers;
EOF

cat > crates/aircraft_api/src/routes/aircraft.rs <<'EOF'
// HTTP handlers for aircraft queries.
EOF

cat > crates/aircraft_api/src/routes/compare.rs <<'EOF'
// HTTP handlers for aircraft comparison workflows.
EOF

cat > crates/aircraft_api/src/routes/manufacturers.rs <<'EOF'
// HTTP handlers for manufacturer queries.
EOF

cat > crates/aircraft_api/src/dto/mod.rs <<'EOF'
pub mod aircraft;
pub mod compare;
EOF

cat > crates/aircraft_api/src/dto/aircraft.rs <<'EOF'
// API response/request DTOs for aircraft resources.
EOF

cat > crates/aircraft_api/src/dto/compare.rs <<'EOF'
// API response/request DTOs for comparison resources.
EOF

# Preserve old aircraft_api lib.rs before replacing.
cp crates/aircraft_api/src/lib.rs archive/restructure-snapshots/aircraft_api.lib.rs.before 2>/dev/null || true

cat > crates/aircraft_api/src/lib.rs <<'EOF'
pub mod claims;
pub mod dto;
pub mod middleware;
pub mod routes;
EOF

cat > crates/aircraft_api/src/middleware/mod.rs <<'EOF'
pub mod rate_limit;
EOF


# =============================================================================
# 11. Add database docs and fixture directory
# =============================================================================

mkdir -p database/fixtures database/scripts

cat > database/README.md <<'EOF'
# Database

This directory owns the SQL lifecycle for the aircraft encyclopedia database.

## Directories

- `migrations/`: ordered schema migrations.
- `seeds/`: canonical seed/reference data.
- `staging/`: transient ingest and bootstrap helpers.
- `validation/`: post-migration validation SQL.
- `fixtures/`: test-only data and harnesses.
- `scripts/`: database utility scripts.

## Notes

The project currently uses SQL-first migrations. Diesel can still generate Rust schema
types from PostgreSQL, but `diesel migration run` should not become the canonical
migration runner unless migrations are converted to Diesel's `up.sql` / `down.sql`
directory format.
EOF

# Move test harness out of canonical seeds if it exists.
move_path database/seeds/013_comprehensive_test_harness.sql database/fixtures/013_comprehensive_test_harness.sql


# =============================================================================
# 12. Add integration test placeholders
# =============================================================================

mkdir -p tests

cat > tests/health_check_tests.rs <<'EOF'
#[test]
fn health_check_placeholder() {
    assert!(true);
}
EOF

cat > tests/migration_smoke_tests.rs <<'EOF'
#[test]
fn migration_smoke_placeholder() {
    assert!(true);
}
EOF

cat > tests/comparison_scoring_tests.rs <<'EOF'
#[test]
fn comparison_scoring_placeholder() {
    assert!(true);
}
EOF


# =============================================================================
# 13. Add an xtask crate for repo automation
# =============================================================================

mkdir -p xtask/src

cat > xtask/Cargo.toml <<'EOF'
[package]
name = "xtask"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1"
EOF

cat > xtask/src/main.rs <<'EOF'
use anyhow::Result;
use std::env;
use std::process::Command;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);

    match (args.next().as_deref(), args.next().as_deref()) {
        (Some("db"), Some("validate")) => run("psql", &["-f", "database/validation/001_extensions_schemas_domains_triggers_validation.sql"])?,
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
EOF


# =============================================================================
# 14. Create a proposed root Cargo.toml workspace file
# =============================================================================
# This does not overwrite Cargo.toml. Review and apply manually.

cat > archive/restructure-snapshots/Cargo.toml.proposed <<'EOF'
[workspace]
resolver = "3"
members = [
    "apps/server",
    "crates/aircraft_api",
    "crates/aircraft_app",
    "crates/aircraft_config",
    "crates/aircraft_db",
    "crates/aircraft_domain",
    "crates/aircraft_ingest",
    "crates/aircraft_observability",
    "xtask",
]

[workspace.package]
edition = "2024"
version = "0.1.0"
license = "UNLICENSED"
publish = false

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
EOF


# =============================================================================
# 15. Create a proposed apps/server Cargo.toml
# =============================================================================
# Adjust dependency versions to match the existing project.

cat > apps/server/Cargo.toml <<'EOF'
[package]
name = "aircraft_server"
version = "0.1.0"
edition = "2024"

[dependencies]
aircraft_api = { path = "../../crates/aircraft_api" }
aircraft_app = { path = "../../crates/aircraft_app" }
aircraft_config = { path = "../../crates/aircraft_config" }
aircraft_observability = { path = "../../crates/aircraft_observability" }
anyhow = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
EOF


# =============================================================================
# 16. Capture after-state
# =============================================================================

tree -a -I 'target|.git' > archive/restructure-snapshots/tree-after.txt

echo "Restructure complete. Next steps:"
echo "1. Review archive/restructure-snapshots/Cargo.toml.proposed"
echo "2. Update root Cargo.toml manually"
echo "3. Update crate Cargo.toml dependencies"
echo "4. Run: cargo fmt --all"
echo "5. Run: cargo check --workspace"