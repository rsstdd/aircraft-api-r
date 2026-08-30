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
  history contains exactly versions `001` through `022`.
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
