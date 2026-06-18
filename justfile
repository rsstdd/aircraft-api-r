# Root Justfile for Aircraft Management Engine

set shell := ["bash", "-cu"]
set dotenv-load
set dotenv-required

DB_SERVICE    := "postgres"
POSTGRES_USER := env_var_or_default("POSTGRES_USER", "aircraft")
POSTGRES_DB   := env_var_or_default("POSTGRES_DB", "aircraft")

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

generate-docs:
    cargo xtask generate-docs

# ---------------------------------------------------------------------
# Docker / PostgreSQL
# ---------------------------------------------------------------------

compose-config:
    docker compose config

compose-services:
    docker compose config --services

db-up:
    docker compose up -d {{DB_SERVICE}}

db-down:
    docker compose down

db-status:
    docker compose ps

db-logs:
    docker compose logs -f {{DB_SERVICE}}

db-restart:
    docker compose restart {{DB_SERVICE}}

db-wait:
    @echo "Waiting for PostgreSQL to accept connections..."
    for i in {1..60}; do \
      if docker compose exec -T {{DB_SERVICE}} pg_isready \
        -U "{{POSTGRES_USER}}" \
        -d "{{POSTGRES_DB}}" >/dev/null 2>&1; then \
        echo "PostgreSQL is ready."; \
        exit 0; \
      fi; \
      sleep 1; \
    done; \
    echo "PostgreSQL did not become ready within 60 seconds."; \
    exit 1

db-ready:
    docker compose exec -T {{DB_SERVICE}} pg_isready \
      -U "{{POSTGRES_USER}}" \
      -d "{{POSTGRES_DB}}"

db-psql:
    docker compose exec -it {{DB_SERVICE}} psql \
      -U "{{POSTGRES_USER}}" \
      -d "{{POSTGRES_DB}}"

db-reset:
    docker compose down -v
    docker compose up -d {{DB_SERVICE}}

# ---------------------------------------------------------------------
# Database migrations / seeds / validation
# ---------------------------------------------------------------------

db-migrate:
    for file in database/migrations/*.sql; do \
      echo "==> migrating $$file"; \
      docker compose exec -T postgres \
        psql -v ON_ERROR_STOP=1 \
          -U "{{POSTGRES_USER}}" \
          -d "{{POSTGRES_DB}}" \
             -f "/workspace/$$file" \
      || exit $$?; \
    done

db-seed:
    for file in database/seeds/*.sql; do \
      echo "==> seeding $file"; \
      docker compose exec -T {{DB_SERVICE}} \
        psql \
          -v ON_ERROR_STOP=1 \
          -U "{{POSTGRES_USER}}" \
          -d "{{POSTGRES_DB}}" \
          -f "/workspace/$file"; \
    done

db-validate:
    for file in database/validation/*.sql; do \
      echo "==> validating $file"; \
      docker compose exec -T {{DB_SERVICE}} \
        psql \
          -v ON_ERROR_STOP=1 \
          -U "{{POSTGRES_USER}}" \
          -d "{{POSTGRES_DB}}" \
          -f "/workspace/$file"; \
    done

db-bootstrap: db-up db-wait db-migrate db-seed db-validate

db-rebuild: db-reset db-wait db-migrate db-seed db-validate