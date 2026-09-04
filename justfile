# Root Justfile for Aircraft Management Engine

set shell := ["/usr/bin/env", "bash", "-cu"]
set dotenv-load

DB_SERVICE := "postgres"
POSTGRES_USER := env_var_or_default("POSTGRES_USER", "aircraft")
POSTGRES_DB := env_var_or_default("POSTGRES_DB", "aircraft_dev")

# ---------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------

default:
    @just --list

# ---------------------------------------------------------------------
# Workspace
# ---------------------------------------------------------------------

build:
    cargo build --workspace --all-targets --locked

check:
    cargo check --workspace --all-targets --locked

boundaries:
    cargo run --locked --package xtask -- boundaries

ingest-validate input:
    cargo run --locked --package aircraft-ingest -- validate --source planephd --input {{ quote(input) }}

ingest-import input:
    cargo run --locked --package aircraft-ingest -- import --source planephd --input {{ quote(input) }}

ingest-status *args:
    cargo run --locked --package aircraft-ingest -- status {{ args }}

ingest-status-json *args:
    cargo run --locked --package aircraft-ingest -- status --format json {{ args }}

# Show assertions ingestion left pending for a curator.
curate-list *args:
    cargo run --locked --package aircraft-ingest -- curate list {{ args }}

# Accept a pending assertion, publishing its value to the read model.
curate-accept assertion_id:
    cargo run --locked --package aircraft-ingest -- curate accept --assertion-id {{ assertion_id }}

# Withdraw a value from the read model.
curate-reject assertion_id:
    cargo run --locked --package aircraft-ingest -- curate reject --assertion-id {{ assertion_id }}

# Rebuild the read model for decisions whose refresh did not complete.
curate-refresh *args:
    cargo run --locked --package aircraft-ingest -- curate refresh {{ args }}

# Import each fixture through the Rust adapter into a disposable database and
# diff the normalized business snapshots against their committed golden output.
# With no arguments this runs every fixture the gate covers.
snapshots *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "{{ args }}" ]; then
        cargo run --locked --package xtask -- snapshots {{ args }}
    else
        for fixture in tests/fixtures/planephd_minimal.json \
                       tests/fixtures/planephd_edge_cases.json; do
            echo "==> $fixture"
            cargo run --locked --package xtask -- snapshots --fixture "$fixture"
        done
    fi

# nextest does not execute doctests, and the route-policy rule is proven by
# `compile_fail` doctests in `aircraft_api`.
test:
    cargo nextest run --workspace --locked
    cargo test --workspace --doc --locked

fmt:
    cargo fmt --all

lint:
    cargo fmt --all -- --check
    just static
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    just docs-check
    cargo audit
    just deny

deny:
    cargo run --locked --package xtask -- deny

# Reconcile the deny.toml build-script allowlist with the lockfile.
deny-pins *ARGS:
    cargo run --locked --package xtask -- deny-pins {{ARGS}}

# NOTE: this is currently equivalent to `just check`. The workspace depends on
# sqlx-core/sqlx-postgres directly, without the `sqlx` facade or its `macros`
# feature, and uses only runtime-checked query/query_scalar/raw_sql. SQLX_OFFLINE
# only affects the compile-time `sqlx::query!` family, so there is nothing here
# for it to verify. Kept so the recipe name stays valid; it becomes a real gate
# the moment a compile-time-checked query is introduced.
check-offline:
    SQLX_OFFLINE=true cargo check --workspace --all-targets --locked

install-deps *args:
    cargo run --locked --package xtask -- install-deps {{ args }}

generate-docs *args:
    cargo run --locked --package xtask -- generate-docs {{ args }}

api-contract:
    cargo run --locked --package xtask -- generate-docs --check
    npm exec --yes --package @stoplight/spectral-cli@6.15.0 -- spectral lint docs/openapi.json

docs-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked

migrations-policy:
    cargo run --locked --package xtask -- migrations

migrations-lint:
    scripts/migrations-lint.sh

compose-check:
    docker compose config --quiet

github-policy-local:
    scripts/github-repository-policy.sh --local

# core.hooksPath is local git config and cannot be committed, so each clone runs
# this once.
# Point this clone at the checked-in hooks in hooks/.
hooks-install:
    git config core.hooksPath hooks
    @echo "core.hooksPath = $(git config --get core.hooksPath)"

# Fail if this clone is not using the checked-in hooks.
hooks-check:
    @test "$(git config --get core.hooksPath)" = "hooks" \
      || { echo "core.hooksPath is not 'hooks'; run: just hooks-install" >&2; exit 1; }
    @test -x hooks/commit-msg || { echo "hooks/commit-msg is not executable" >&2; exit 1; }
    @echo "Checked-in git hooks are active."

static: boundaries api-contract migrations-policy migrations-lint compose-check github-policy-local

github-policy-check:
    scripts/github-repository-policy.sh --check

# Mutates GitHub-hosted settings and requires repository administrator access.
github-policy-apply:
    scripts/github-repository-policy.sh --apply

# ---------------------------------------------------------------------
# Local Docker / PostgreSQL
# ---------------------------------------------------------------------

compose-config:
    docker compose config

compose-services:
    docker compose config --services

db-up:
    docker compose up -d {{ DB_SERVICE }}

db-down:
    docker compose down

db-status:
    docker compose ps

db-logs:
    docker compose logs -f {{ DB_SERVICE }}

db-restart:
    docker compose restart {{ DB_SERVICE }}

db-wait:
    @echo "Waiting for PostgreSQL to accept connections..."
    for i in {1..60}; do \
      if docker compose exec -T {{ DB_SERVICE }} pg_isready \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" >/dev/null 2>&1; then \
        echo "PostgreSQL is ready."; \
        exit 0; \
      fi; \
      sleep 1; \
    done; \
    echo "PostgreSQL did not become ready within 60 seconds."; \
    exit 1

db-ready:
    docker compose exec -T {{ DB_SERVICE }} pg_isready \
      -U "{{ POSTGRES_USER }}" \
      -d "{{ POSTGRES_DB }}"

db-psql:
    docker compose exec -it {{ DB_SERVICE }} psql \
      -U "{{ POSTGRES_USER }}" \
      -d "{{ POSTGRES_DB }}"

# Destructive: deletes the local PostgreSQL volume and starts an empty DB.
db-reset:
    docker compose down -v
    docker compose up -d {{ DB_SERVICE }}

# ---------------------------------------------------------------------
# Local database migrations / seeds / validation
# ---------------------------------------------------------------------

# Local-only compatibility reconciliation runs before the canonical installer.
db-migrate:
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -f "/workspace/database/reconcile_local_legacy.sql" \
        -f "/workspace/database/install.sql"

db-seed:
    for file in database/seeds/*.sql; do \
      echo "==> seeding $file"; \
      docker compose exec -T {{ DB_SERVICE }} \
        psql -X -v ON_ERROR_STOP=1 \
          -U "{{ POSTGRES_USER }}" \
          -d "{{ POSTGRES_DB }}" \
          -f "/workspace/$file" \
      || exit $?; \
    done

db-validate:
    for file in database/validation/*.sql; do \
      echo "==> validating $file"; \
      docker compose exec -T {{ DB_SERVICE }} \
        psql -X -v ON_ERROR_STOP=1 \
          -U "{{ POSTGRES_USER }}" \
          -d "{{ POSTGRES_DB }}" \
          -f "/workspace/$file" \
      || exit $?; \
    done

# Create the restricted ingestion login role that db-grants expects.
# install.sql deliberately does not create roles. The password comes from
# INGEST_ROLE_PASSWORD in the environment, never from a recipe argument, so it
# stays out of the process argument list. Re-running this leaves an existing
# role and its password untouched.
db-create-ingest-role ingest_role="aircraft_ingest_app":
    docker compose exec -T -e INGEST_ROLE_PASSWORD {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -v "ingest_role={{ ingest_role }}" \
        -f "/workspace/database/roles/create_ingest_role.sql"

# Grant the dedicated ingestion role. Requires an administrator connection and
# an existing role; run db-create-ingest-role first. The default matches the
# role name in .env and the architecture guide.
db-grants ingest_role="aircraft_ingest_app":
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -v "ingest_role={{ ingest_role }}" \
        -f "/workspace/database/roles/ingest_grants.sql"

# Create the restricted server login role that db-grant-app-role expects.
# install.sql deliberately does not create roles. The password comes from
# API_ROLE_PASSWORD in the environment, never from a recipe argument, so it
# stays out of the process argument list. Re-running this leaves an existing
# role and its password untouched.
db-create-app-role app_role="aircraft_api_app":
    docker compose exec -T -e API_ROLE_PASSWORD {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -v "app_role={{ app_role }}" \
        -f "/workspace/database/roles/create_app_role.sql"

# Grant the dedicated server role. Requires an administrator connection, an
# existing role, and an installed schema: run db-create-app-role and
# db-bootstrap first, because the grants name the aircraft_auth tables the
# verification lookup reads. The default matches the role name in .env.example.
# This also revokes the database's default TEMPORARY grant from PUBLIC, which
# reaches every non-superuser on that database, not just app_role.
db-grant-app-role app_role="aircraft_api_app":
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -v "app_role={{ app_role }}" \
        -f "/workspace/database/roles/app_grants.sql"

db-bootstrap: db-up db-wait db-migrate db-validate

# Destructive full local rebuild: reset, install/seeds, then validate.
db-rebuild: db-reset db-wait db-migrate db-validate

# ---------------------------------------------------------------------
# Direct / production database lifecycle
# ---------------------------------------------------------------------

# Direct commands prefer MIGRATION_DATABASE_URL so deployments can use a
# privileged migration role while the application keeps a restricted URL.
# DATABASE_URL is accepted as a fallback for simpler environments.
db-prod-ready:
    db_url="${MIGRATION_DATABASE_URL:-${DATABASE_URL:-}}"; \
    if [ -z "$db_url" ]; then \
      echo "MIGRATION_DATABASE_URL or DATABASE_URL must be set"; exit 1; \
    fi; \
    psql -X -v ON_ERROR_STOP=1 "$db_url" \
      -c "SELECT current_database() AS database, current_user AS migration_role;"

# Production intentionally does not auto-adopt untracked legacy schemas.
db-prod-migrate:
    db_url="${MIGRATION_DATABASE_URL:-${DATABASE_URL:-}}"; \
    if [ -z "$db_url" ]; then \
      echo "MIGRATION_DATABASE_URL or DATABASE_URL must be set"; exit 1; \
    fi; \
    psql -X -v ON_ERROR_STOP=1 "$db_url" -f database/install.sql

db-prod-seed:
    db_url="${MIGRATION_DATABASE_URL:-${DATABASE_URL:-}}"; \
    if [ -z "$db_url" ]; then \
      echo "MIGRATION_DATABASE_URL or DATABASE_URL must be set"; exit 1; \
    fi; \
    for file in database/seeds/*.sql; do \
      echo "==> seeding $file"; \
      psql -X -v ON_ERROR_STOP=1 "$db_url" -f "$file" || exit $?; \
    done

db-prod-validate:
    db_url="${MIGRATION_DATABASE_URL:-${DATABASE_URL:-}}"; \
    if [ -z "$db_url" ]; then \
      echo "MIGRATION_DATABASE_URL or DATABASE_URL must be set"; exit 1; \
    fi; \
    for file in database/validation/*.sql; do \
      echo "==> validating $file"; \
      psql -X -v ON_ERROR_STOP=1 "$db_url" -f "$file" || exit $?; \
    done

db-prod-bootstrap: db-prod-ready db-prod-migrate db-prod-validate
