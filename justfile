# Root Justfile for Aircraft Management Engine
set shell := ["bash", "-c"]

# Display all available automated commands
default:
    @just --list

# Initialize local environment, spin up infrastructure, and run schema setups
bootstrap:
    docker compose up -d postgres
    @echo "Waiting for PostgreSQL to stabilize..."
    sleep 2
    just migrate
    cargo xtask db-seed

# Compile the entire workspace including binaries, libraries, and tests
build:
    cargo build --workspace --all-targets

# Execute the parallel testing suite using cargo-nextest
test:
    cargo nextest run --workspace

# Run canonical schema migrations against the local development instance
migrate:
    cargo xtask db-migrate

# Wipe the local database, re-apply migrations, and re-inject reference data
db-reset:
    cargo xtask db-reset

# Re-generate the offline compile-time SQLx query cache (.sqlx/ metadata directory)
prepare-sqlx:
    @echo "Generating SQLx metadata utilizing administrative migration credentials..."
    DATABASE_URL="postgresql://aircraft_owner:admin_password@localhost:5432/aircraft" cargo sqlx prepare --workspace -- --all-targets

# Assert that the workspace compiles cleanly without an active database connection
check-offline:
    SQLX_OFFLINE=true cargo check --workspace --all-targets

# Run formatting, clippy lints, audit checks, and dependency policy sweeps
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo audit
    cargo deny check

# Generate and export the OpenAPI contract v3 JSON blueprint to disk
generate-docs:
    cargo xtask generate-docs