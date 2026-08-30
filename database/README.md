# Database

This directory owns the SQL lifecycle for the Aircraft Management Engine.

## Layout

- `migrations/`: ordered, one-time schema migrations.
- `seeds/`: idempotent canonical reference and mission-profile data only.
- `validation/`: post-install schema and behavioral verification.
- `snapshots/`: normalized business snapshot queries, plus the committed golden
  output in `snapshots/golden/<fixture>/` that `cargo xtask snapshots` diffs the
  adapter against.
- `roles/`: ingestion role creation and grants, applied by an administrator,
  not by `install.sql`.
- `fixtures/`: test-only data.
- `docker/init/`: optional database-level initialization for an empty volume.
- `reconcile_local_legacy.sql`: local-only compatibility check for complete
  Phase 1/2 schemas created before migration tracking existed.
- `install.sql`: the canonical dependency-aware `psql` installer.
- `migrations.lock.json`: SHA-256 of every migration. Migrations are immutable
  once written; `cargo xtask migrations` fails if a file's contents change, so a
  correction goes in a new migration and in this documentation, never by editing
  an applied one.
- `validation/000_migration_history_validation.sql`: asserts that migration
  history contains exactly versions `001` through `023`.
- `data_dictionary.md`: detailed reference for principal tables and read models;
  migrations remain authoritative for the complete schema.
- `implementation_notes.md`: dependency rules, curator workflows, known
  limitations, and deferred database decisions.

## Local workflow

```bash
just db-bootstrap
```

`db-bootstrap` starts and waits for the Compose database, then installs and
validates it. Before `db-migrate` uses `install.sql`, the local-only
`reconcile_local_legacy.sql` compatibility check adopts complete migration 001
and 002 structures created before migration tracking was introduced. It refuses
partial or ambiguous legacy structures. Production recipes do not perform this
automatic adoption. The installer tracks migrations in
`public.aircraft_schema_migrations` and applies required seed phases at their
foreign-key boundaries. Do not execute `database/migrations/*.sql` as a simple
glob: Phase 3 requires the Phase 2 lookup seed rows.

The reconciliation step never runs in `db-prod-migrate` or
`db-prod-bootstrap`. Production legacy databases must be reviewed and baselined
deliberately.

`db-validate` begins by checking the exact migration ledger before running the
phase-specific schema and behavioral validations.

To reapply only canonical data, run `just db-seed`.

`just db-reset` deletes the local PostgreSQL volume and starts a fresh empty
container. `just db-rebuild` performs that destructive reset and then installs,
seeds, and validates the database. Do not use either command when local data
must be preserved.

## Explore a seeded local database

Check the Compose database and open its configured `psql` session:

```bash
just db-status
just db-ready
just db-psql
```

The following `psql` cheat sheet runs inside a read-only transaction. Canonical
seeding populates reference and mission-profile data; aircraft, ingestion, and
curation queries remain empty until source data has been imported.

```sql
\conninfo
\timing on
\x auto
\pset null '<null>'

BEGIN READ ONLY;
SET LOCAL statement_timeout = '10s';

-- Discover schemas and relations.
\dn
\dt aircraft_ref.*
\dt aircraft_core.*
\dt aircraft_compare.*
\dt aircraft_ingest.*
\dt aircraft_prov.*
\dv aircraft_read.*
\dm aircraft_read.*

-- Confirm the installed schema history.
SELECT version, applied_at
FROM public.aircraft_schema_migrations
ORDER BY version;

-- Inspect canonical reference data.
SELECT unit_category_code, count(*) AS units
FROM aircraft_ref.measurement_units
GROUP BY unit_category_code
ORDER BY unit_category_code;

SELECT code, label, canonical_unit_code
FROM aircraft_ref.performance_metric_types
ORDER BY sort_order, code;

SELECT code, label, role_group
FROM aircraft_ref.aircraft_roles
ORDER BY sort_order, code;

-- Check mission profiles and their criterion weights.
SELECT
    mp.slug,
    mp.title,
    count(mc.id) AS criteria,
    coalesce(sum(mc.weight), 0) AS total_weight
FROM aircraft_compare.mission_profiles AS mp
LEFT JOIN aircraft_compare.mission_criteria AS mc
    ON mc.mission_profile_id = mp.id
GROUP BY mp.id, mp.slug, mp.title, mp.sort_order
ORDER BY mp.sort_order;

-- Find populated application tables.
SELECT schemaname, relname, n_live_tup AS estimated_rows
FROM pg_stat_user_tables
WHERE schemaname LIKE 'aircraft_%'
ORDER BY n_live_tup DESC, schemaname, relname;

-- Browse imported variants and the published read model.
SELECT id, slug, name, service_status_code, passenger_capacity, engine_count
FROM aircraft_core.variants
ORDER BY slug
LIMIT 50;

SELECT
    slug,
    variant_name,
    primary_manufacturer_name,
    cruise_speed_kias,
    range_nm,
    service_ceiling_ft,
    gross_weight_lb,
    papi_price_usd
FROM aircraft_read.mv_variant_search
ORDER BY primary_manufacturer_name, variant_name
LIMIT 50;

-- Inspect ingestion history.
SELECT
    id,
    source_slug,
    status,
    staged_aircraft,
    promoted_aircraft,
    flagged_aircraft,
    warning_count,
    started_at,
    finished_at
FROM aircraft_ingest.ingest_runs
ORDER BY started_at DESC
LIMIT 20;

-- Inspect pending curation work.
SELECT status_code, count(*)
FROM aircraft_prov.source_assertions
GROUP BY status_code
ORDER BY status_code;

SELECT
    id,
    entity_type_code,
    entity_id,
    field_name,
    raw_value,
    asserted_numeric,
    status_code
FROM aircraft_prov.source_assertions
WHERE status_code = 'PENDING'
ORDER BY created_at, id
LIMIT 50;

SELECT
    priority,
    entity_type_code,
    entity_id,
    field_name,
    issue_type,
    issue_description
FROM aircraft_prov.curation_flags
WHERE status_code = 'OPEN'
ORDER BY priority, created_at
LIMIT 50;

-- Check whether a read-model refresh is outstanding.
SELECT
    id,
    requested_by,
    reason,
    status_code,
    attempts,
    requested_at,
    completed_at,
    last_error
FROM aircraft_read.read_model_refresh_requests
ORDER BY requested_at DESC
LIMIT 50;

ROLLBACK;
\q
```

The CLI also exposes read-only ingestion and curation summaries:

```bash
just ingest-status --limit 20
just ingest-status-json --limit 20
just curate-list --limit 20
```

Use the Rust ingestion adapter for source JSON:

```bash
just ingest-validate ./aircraft_seed.json
just ingest-import ./aircraft_seed.json
just ingest-status
```

It reads a local file or standard input, preserves raw records and audit history,
and does not require database-server filesystem access. The legacy
server-side SQL loader that used to live in `database/staging/` has been
retired; `database/snapshots/` now holds the snapshot queries and the committed
golden output that guard the Rust path against regressions.

Diesel may generate Rust schema types from PostgreSQL, but SQL files in this
directory remain the canonical schema history.
