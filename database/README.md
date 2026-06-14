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
