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
    cargo build --workspace --all-targets

check:
    cargo check --workspace --all-targets

test:
    cargo nextest run --workspace

fmt:
    cargo fmt --all

lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo audit
    cargo deny check

check-offline:
    SQLX_OFFLINE=true cargo check --workspace --all-targets

# prepare-sqlx:
#     @echo "Generating SQLx metadata..."
#     @if [ -z "{{env_var_or_default('MIGRATION_DATABASE_URL', env_var_or_default('DATABASE_URL', ''))}}" ]; then \
#       echo "DATABASE_URL or MIGRATION_DATABASE_URL must be set"; exit 1; \
#     fi
#     DATABASE_URL="{{env_var_or_default('MIGRATION_DATABASE_URL', env_var('DATABASE_URL'))}}" cargo sqlx prepare --workspace -- --all-targets

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

db-reset:
    docker compose down -v
    docker compose up -d {{ DB_SERVICE }}

# ---------------------------------------------------------------------
# Local database migrations / seeds / validation
# ---------------------------------------------------------------------

db-migrate:
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
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

# The JSON path is server-side and must be below the database mount.
# Example: just db-ingest /workspace/database/staging/aircraft_seed.json
db-ingest container_json_path:
    json_path={{ quote(container_json_path) }}; \
    case "$json_path" in \
      /workspace/database/*) ;; \
      *) echo "container_json_path must be below /workspace/database"; exit 1 ;; \
    esac; \
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -f "/workspace/database/staging/901_seed_data_staging.sql"; \
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -v "seed_json_path=$json_path" \
        -f "/workspace/database/staging/902_server_side_json_ingestion.sql"; \
    docker compose exec -T {{ DB_SERVICE }} \
      psql -X -v ON_ERROR_STOP=1 \
        -U "{{ POSTGRES_USER }}" \
        -d "{{ POSTGRES_DB }}" \
        -f "/workspace/database/staging/903_post_bootstrap_validation.sql"

db-bootstrap: db-up db-wait db-migrate db-validate

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

# The path is read by the PostgreSQL server, not by the machine running Just.
# It must already exist on the production database server and be readable by
# the PostgreSQL service account.
db-prod-ingest server_json_path:
    db_url="${MIGRATION_DATABASE_URL:-${DATABASE_URL:-}}"; \
    if [ -z "$db_url" ]; then \
      echo "MIGRATION_DATABASE_URL or DATABASE_URL must be set"; exit 1; \
    fi; \
    json_path={{ quote(server_json_path) }}; \
    psql -X -v ON_ERROR_STOP=1 "$db_url" \
      -f database/staging/901_seed_data_staging.sql; \
    psql -X -v ON_ERROR_STOP=1 "$db_url" \
      -v "seed_json_path=$json_path" \
      -f database/staging/902_server_side_json_ingestion.sql; \
    psql -X -v ON_ERROR_STOP=1 "$db_url" \
      -f database/staging/903_post_bootstrap_validation.sql

db-prod-bootstrap: db-prod-ready db-prod-migrate db-prod-validate
