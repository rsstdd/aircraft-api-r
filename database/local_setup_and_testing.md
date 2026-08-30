# Local Database Setup

## Prerequisites

Install and start:

- Docker with Docker Compose support.
- `just`.
- PostgreSQL client tools if direct `psql` access is required.
- Rust 1.85 or newer for application checks.

Run all commands from the repository root:

```bash
cd /home/rsstdd/dev/aircraft/aircraft-api-r
```

Confirm the required tools:

```bash
docker --version
docker compose version
just --version
```

## 1. Create the local environment file

Copy the example configuration:

```bash
cp .env.example .env
```

The `.env` file is ignored by Git. Do not commit it.

The example configuration creates:

```text
Database: aircraft
User:     aircraft_app
Port:     5432
```

The values are suitable only for local development. If port `5432` is already in use, change `POSTGRES_PORT`, `DATABASE_URL`, and `MIGRATION_DATABASE_URL` together.

The database can also run without `.env`. In that case, Compose uses these defaults:

```text
Database: aircraft_dev
User:     aircraft
Password: aircraft_dev_password
Port:     5432
```

## 2. Validate the Compose configuration

Render the effective configuration:

```bash
just compose-config
```

Confirm that the expected service exists:

```bash
just compose-services
```

Expected output:

```text
postgres
```

Review the rendered database name, user, port, image, and mounted database directory before continuing.

## 3. Bootstrap the database

Run the complete local setup:

```bash
just db-bootstrap
```

This command performs the following operations in order:

1. Starts PostgreSQL.
2. Waits until PostgreSQL accepts connections.
3. Reconciles a verified legacy Phase 1/2 prefix when necessary.
4. Runs the dependency-aware installer.
5. Applies every migration file in `database/migrations/`, the source of truth.
6. Applies canonical seeds at their required dependency boundaries.
7. Runs every database validation script, beginning with exact migration-history validation.

A separate `just db-seed` call is not required during initial bootstrap because `database/install.sql` applies the canonical seeds in the correct order.

For local volumes created before migration tracking was introduced,
`just db-bootstrap` verifies and adopts a complete legacy Phase 1 or Phase 2
prefix before continuing. It stops with an actionable error if it finds only
part of either phase. This compatibility behavior is local-only; production
databases require deliberate baselining.

## 4. Confirm PostgreSQL is healthy

Check the container:

```bash
just db-status
```

Check database readiness:

```bash
just db-ready
```

Expected result:

```text
accepting connections
```

If PostgreSQL does not become ready, inspect its logs:

```bash
just db-logs
```

Press `Ctrl+C` to stop following the logs.

## 5. Inspect the installed database

Open an interactive PostgreSQL session:

```bash
just db-psql
```

Check migration history:

```sql
SELECT version, applied_at
FROM public.aircraft_schema_migrations
ORDER BY version;
```

The result should contain one row per file in `database/migrations/`, in order.

Check the migration count:

```sql
SELECT COUNT(*) AS migration_count
FROM public.aircraft_schema_migrations;
```

The count must equal the number of migration files in `database/migrations/`.

Check representative canonical seed counts:

```sql
SELECT COUNT(*) AS measurement_units
FROM aircraft_ref.measurement_units;

SELECT COUNT(*) AS mission_profiles
FROM aircraft_compare.mission_profiles;
```

Expected results for the current seed set:

```text
measurement_units: 38
mission_profiles:  15
```

Exit PostgreSQL:

```text
\q
```

## 6. Load an aircraft JSON dataset

Ingestion runs through the `aircraft-ingest` CLI, which reads the file from your
own filesystem. The retired server-side SQL loader required the JSON to sit under
`/workspace/database/` so the PostgreSQL server process could read it; that
constraint is gone, and any readable path works.

Check a file without touching the database:

```bash
just ingest-validate tests/fixtures/planephd_minimal.json
```

Importing needs the restricted ingestion role. `install.sql` creates no roles,
so create it once and grant it, using the password from your `.env`:

```bash
just db-create-ingest-role
just db-grants
```

Both recipes default to `aircraft_ingest_app`, the role named in
`APP__INGEST__DATABASE_URL`. `db-create-ingest-role` reads the password from
`INGEST_ROLE_PASSWORD` so it stays out of the process argument list, and both
recipes are safe to re-run. Confirm the role can reach the database:

```bash
just ingest-status --limit 1
```

Then import:

```bash
just ingest-import tests/fixtures/planephd_minimal.json
just ingest-status --limit 20
```

`just` loads `.env`, so `APP__INGEST__DATABASE_URL` is already set; export it
only to point at a different database.

The command captures the input, hashes it, validates the whole document before
writing anything, then stages and promotes every record in one transaction and
refreshes the read models. Re-running it with the same file is a no-op: the run
is identified by source, content SHA-256, and parser version.

Raw datasets are ignored by Git. Sanitized examples must use the `.json.example`
extension if they need to be committed.

### Nothing is searchable until it is curated

This surprises people. Ingested values are written non-canonical and their
assertions `PENDING`, so a freshly imported variant appears in
`aircraft_read.mv_variant_search` with its identity but **no** speeds, weights,
prices, or costs. That is deliberate: nothing from an uncurated scraped source is
served until a person accepts it.

```bash
just curate-list                  # what is awaiting a decision
just curate-accept <assertion-id> # publish that value
just curate-reject <assertion-id> # withdraw it again
```

Each decision moves the assertion, the row it backs, any curation flags it
closes, and the read-model refresh together in one transaction.

## 7. Stop the database

Stop the Compose services while preserving database data:

```bash
just db-down
```

Restart later with:

```bash
just db-up
just db-wait
```

Do not use `just db-reset` or `just db-rebuild` merely to stop PostgreSQL. Those commands delete the local PostgreSQL volume.

# Local Database Testing Guide

## 1. Validate repository configuration

Check the Justfile:

```bash
just --unstable --fmt --check
just --list
```

Check Compose:

```bash
just compose-config
just compose-services
```

These checks do not modify database data.

## 2. Confirm database readiness

Start PostgreSQL if necessary:

```bash
just db-up
just db-wait
just db-ready
```

## 3. Test migration repeatability

Apply the installer:

```bash
just db-migrate
```

Run it again:

```bash
just db-migrate
```

The second run should skip migrations already recorded in:

```text
public.aircraft_schema_migrations
```

It should still complete successfully because the canonical seeds are idempotent.

Confirm that the migration history still contains one row per file in
`database/migrations/`:

```bash
just db-psql
```

```sql
SELECT COUNT(*)
FROM public.aircraft_schema_migrations;
```

Exit with `\q`.

## 4. Test seed repeatability

Apply all canonical seed files:

```bash
just db-seed
```

Run the same command again:

```bash
just db-seed
```

Both runs should succeed. Confirm that representative counts remain stable:

```bash
just db-psql
```

```sql
SELECT COUNT(*) FROM aircraft_ref.measurement_units;
SELECT COUNT(*) FROM aircraft_compare.mission_profiles;
```

Expected current counts:

```text
38 measurement units
15 mission profiles
```

## 5. Run the complete database validation suite

Run:

```bash
just db-validate
```

This executes every SQL file under:

```text
database/validation/
```

The first file, `000_migration_history_validation.sql`, requires the ledger to
contain exactly the versions shipped in `database/migrations/`. This prevents a
structurally partial or unexpectedly versioned database from passing the broader
suite.

The command uses `ON_ERROR_STOP=1`, so any SQL error or failed hard invariant stops the recipe with a nonzero exit code.

Warnings should be reviewed, but they are not necessarily failures. Errors and raised exceptions are failures.

## 6. Test JSON ingestion

Point the CLI at a PlanePHD JSON file, then run:

```bash
just ingest-import tests/fixtures/planephd_minimal.json
```

Inspect the result:

```bash
just db-psql
```

```sql
SELECT
    run_label,
    total_manufacturers,
    total_aircraft,
    staged_aircraft,
    promoted_aircraft,
    started_at,
    finished_at
FROM aircraft_ingest.ingest_runs
ORDER BY started_at DESC;

SELECT stage_status, COUNT(*)
FROM aircraft_ingest.staged_aircraft
GROUP BY stage_status
ORDER BY stage_status;

SELECT COUNT(*) AS canonical_variants
FROM aircraft_core.variants;
```

No staged row should remain in `PENDING` after a successful promotion.

Run the same ingestion command again:

```bash
just ingest-import tests/fixtures/planephd_minimal.json
```

The second run should complete without duplicating the content-derived ingestion run or canonical aircraft records.

## 7. Test input rejection

The retired server-side loader required its JSON to sit under
`/workspace/database/` and rejected anything else. The CLI reads from your own
filesystem, so that guard is gone; what replaced it is validation of the document
itself, before anything is written.

An unreadable file is an artifact-capture failure (exit code 3):

```bash
just ingest-import /nonexistent/planephd.json; echo "exit $?"
```

A document that fails validation is exit code 4, and must leave the database
untouched:

```bash
printf '{"CESSNA":{"172S":{"start_year":2006,"end_year":1998}}}' > /tmp/invalid.json
just ingest-import /tmp/invalid.json; echo "exit $?"
just ingest-status --limit 5
```

Expect `INVALID_PRODUCTION_YEARS` on stderr, exit code 4, and a run recorded as
`VALIDATION_FAILED` with no aircraft rows created. The documented exit codes are
2 configuration, 3 artifact, 4 validation, 5 already running, 6 persistence,
7 parser consistency, and 8 a curation decision the current state does not allow.

## 8. Test installation from a clean database

This is the strongest local database test, but it is destructive.

`just db-rebuild` deletes the complete local PostgreSQL volume, including all manually loaded or curated data. Run it only when the local database is disposable.

```bash
just db-rebuild
```

This performs:

1. `docker compose down -v`
2. PostgreSQL startup
3. Readiness polling
4. Fresh migration and seeding
5. Full validation

Afterward, confirm the migration and seed counts again:

```bash
just db-psql
```

```sql
SELECT COUNT(*) FROM public.aircraft_schema_migrations;
SELECT COUNT(*) FROM aircraft_ref.measurement_units;
SELECT COUNT(*) FROM aircraft_compare.mission_profiles;
```

The migration count must equal the number of migration files in
`database/migrations/`. Expected seed results:

```text
38 measurement units
15 mission profiles
```

## 9. Run Rust-side checks

Check workspace compilation:

```bash
just check
```

Check SQLx offline compilation:

```bash
just check-offline
```

Run the configured test suite:

```bash
just test
```

`just test` requires `cargo-nextest`. If it is unavailable, run:

```bash
cargo test --workspace --all-targets
```

Run formatting and Clippy checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The full lint recipe additionally requires `cargo-audit` and `cargo-deny`:

```bash
just lint
```

A missing external tool is an environment limitation, not a successful lint result.

## 10. Final acceptance checklist

The local database is ready when all of the following are true:

- `just compose-config` succeeds.
- `just db-ready` reports that PostgreSQL accepts connections.
- `just db-migrate` succeeds twice.
- `just db-seed` succeeds twice.
- `just db-validate` succeeds.
- Migration history contains exactly the versions shipped in `database/migrations/`.
- The canonical seed counts are 38 measurement units and 15 mission profiles.
- JSON ingestion completes without pending staged records.
- Re-ingesting identical JSON does not duplicate runs or canonical variants.
- `just check` succeeds.
- Formatting and Clippy checks succeed.
