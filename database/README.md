# Database

This directory owns the SQL lifecycle for the Aircraft Management Engine.

## Layout

- `migrations/`: ordered, one-time schema migrations.
- `seeds/`: idempotent canonical reference and mission-profile data only.
- `staging/`: transient JSON ingestion DDL, promotion, and post-ingestion checks.
- `validation/`: post-install schema and behavioral verification.
- `fixtures/`: test-only data.
- `docker/init/`: optional database-level initialization for an empty volume.
- `reconcile_local_legacy.sql`: local-only compatibility check for complete
  Phase 1/2 schemas created before migration tracking existed.
- `install.sql`: the canonical dependency-aware `psql` installer.
- `validation/000_migration_history_validation.sql`: asserts that migration
  history contains exactly versions `001` through `016`.
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

For source JSON ingestion, place the private dataset under
`database/staging/` (JSON files there are ignored by Git) and run:

```bash
just db-ingest /workspace/database/staging/aircraft_seed.json
```

This loads staging, promotes records, refreshes materialized views, and executes
post-ingestion invariants. It fails if the server-side JSON path is unreadable.


## Production workflow

Production commands use the host `psql` client and never invoke Compose:

```bash
export MIGRATION_DATABASE_URL='postgresql://migration-role@db.example/aircraft'
just db-prod-bootstrap
```

`MIGRATION_DATABASE_URL` is preferred so the application can retain a
least-privilege `DATABASE_URL`; the latter is used as a fallback. The target
database and migration role must already be provisioned by the platform. The
role needs permission to create the schemas, extensions, tables, functions,
views, indexes, and triggers defined by the migrations.

`db-prod-bootstrap` first prints the target database and role, then installs
all migrations and canonical seeds and runs the complete validation suite.
Individual operations are available as `db-prod-ready`, `db-prod-migrate`,
`db-prod-seed`, and `db-prod-validate`. The installer serializes concurrent
runs with a database advisory lock.

Production JSON ingestion is intentionally separate because `pg_read_file`
reads from the database server's filesystem:

```bash
just db-prod-ingest /srv/aircraft-data/aircraft_seed.json
```

The file must exist on the database server and be readable by PostgreSQL.

Diesel may generate Rust schema types from PostgreSQL, but SQL files in this
directory remain the canonical schema history.
