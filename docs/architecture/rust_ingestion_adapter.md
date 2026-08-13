# Rust ingestion adapter

## Purpose

The Rust ingestion adapter imports untrusted aircraft source data without allowing
source formats, SQL rows, or transport concerns to cross architectural borders.
PlanePHD JSON is the first concrete source. The adapter preserves raw records,
field provenance, normalization warnings, and curation work.

## Architecture

```text
apps/ingest
  -> aircraft_ingest (input capture, PlanePHD parser, normalization)
  -> aircraft_app (ingestion use case and ports)
  -> aircraft_domain (source-independent invariants)
  -> aircraft_db (SQLx implementation)
  -> PostgreSQL
```

- `aircraft_domain` owns deterministic confidence and production-year invariants.
- `aircraft_app` owns prepared records, issue severity, reports, streaming input,
  audit and transaction ports, and all-or-nothing orchestration.
- `aircraft_ingest` owns secure file/stdin capture, SHA-256 identity, streaming
  PlanePHD parsing, source mappings, and raw-record preservation. It does not use SQLx.
- `aircraft_db` owns SQLx rows, advisory locks, staging, promotion, provenance,
  curation flags, run audit, and materialized-view refresh.
- `apps/ingest` is the composition root. It owns CLI rendering, exit codes,
  configuration, and tracing.

The legacy server-side loader remains available only as a parity reference. The
Rust path never calls `pg_read_file`.

## Public command surface

```bash
aircraft-ingest validate --source planephd --input FILE_OR_DASH [--format human|json]
aircraft-ingest import --source planephd --input FILE_OR_DASH [--format human|json] [--report PATH]
aircraft-ingest status [--run-id ID | --sha256 HASH] [--limit N] [--format human|json]
```

Equivalent repository commands are `just ingest-validate`,
`just ingest-import`, and `just ingest-status`. Status reports include the full
attempt history for every matching logical run, newest attempt first.

Validation is database-independent. Import and status require
`APP__INGEST__DATABASE_URL`. Database credentials are never accepted as CLI
arguments or included in reports.

Exit codes are: 0 success, 2 configuration/usage, 3 input I/O, 4 validation,
5 identical import busy, 6 database/transaction, and 7 parser consistency.

## Processing and transaction semantics

Input is copied once to an owner-only temporary file while enforcing the byte
limit and computing SHA-256 over the exact bytes. File reports contain only a
sanitized basename; stdin is reported as `<stdin>`.

Preflight parses the immutable artifact before the database transaction. It
validates the two-level manufacturer-to-aircraft structure, required identity,
source-record uniqueness, domain invariants, supported scalar shapes, and every
normalizable record. Stable warnings are counted but do not fail preflight.

Import reopens the same artifact and streams one aircraft record at a time through
a bounded channel. The application verifies record count, warning count, and the
ordered source-record-key fingerprint against preflight. A mismatch is an
internal hard failure.

A PostgreSQL transaction-scoped advisory lock is keyed by source, content hash,
parser name, and parser version. A successful prior run returns its report with
`already_imported=true`; a concurrent identical import fails immediately.

Staging, canonical identity, specifications, engines, market data, source
documents, assertions, curation flags, run success, and a non-concurrent
materialized-view refresh share one transaction. Any hard error rolls them all
back. A separate short transaction then records the failed attempt. Import-mode
preflight failures also create durable validation-failure run and attempt rows.

## Validation and curation rules

Structural problems and invalid required identity are errors. Optional data that
cannot safely become canonical produces a warning and remains in raw JSON.

Known PlanePHD performance, weight, unit, engine, valuation, and operating-cost
fields map to canonical codes. Unknown measurement fields, units, cost keys,
invalid image dimensions, and unsupported record fields produce stable warning
codes. They are retained in staging and become open curation flags.

PlanePHD source confidence is 0.20 and its reliability is `UNVERIFIED`. The first
assertion for an entity field can be accepted; later assertions remain pending,
preserving the single-accepted-assertion database invariant. Images are metadata
only; no binary download occurs.

Logical run identity is:

```text
source slug + SHA-256 + parser name + parser version
```

PlanePHD source-record identity is SHA-256 over the byte sequence
`planephd\0<raw manufacturer key>\0<raw aircraft key>`. The namespace prefix,
keys, and NUL separators are part of the identity contract. Parser-version
changes intentionally create new logical runs.

Each Rust logical run inserts a distinct source-document row linked by
`ingest_run_id`. A later artifact or parser version may reuse the stable
source-record identity, but it never updates the raw JSON, parser version, or
batch label behind historical assertions.

## PostgreSQL and Aiven

Migration 017 brings the run, attempt, staging, image, raw JSON, issue, hash, and
idempotency structures into canonical migration history.

Create a dedicated Aiven service user, run migrations with the migration owner,
then grant ingest rights as an administrator:

```bash
psql "$MIGRATION_DATABASE_URL" \
  -v ingest_role=aircraft_ingest_app \
  -f database/roles/ingest_grants.sql
```

Set the dedicated user's TLS-verified Aiven URL in
`APP__INGEST__DATABASE_URL`. The role can read only the required lookup and
reference tables, write only the tables needed by ingestion, and execute the
controlled read-model refresh function. It cannot migrate, create schemas,
manage extensions, drop tables, or administer roles.

Relevant settings are:

```text
APP__INGEST__DATABASE_URL
APP__INGEST__MAX_INPUT_BYTES
APP__INGEST__LOCK_TIMEOUT_SECONDS
APP__INGEST__STATEMENT_TIMEOUT_SECONDS
APP__INGEST__TEMP_DIR
APP__INGEST__MAX_CONNECTIONS
```

The default input limit is 512 MiB. The system temporary directory is used unless
a temp directory is explicitly configured.

## Verification and retirement

The synthetic fixture is `tests/fixtures/planephd_minimal.json`. Parser,
normalization, domain, warning-commit, hard-error rollback, migration, clean
database import, idempotency, and status behavior must pass before deployment.

Legacy SQL retirement requires a separate parity run comparing normalized
business snapshots for identity, measurements, propulsion, market data,
provenance, assertions, and flags while excluding generated IDs and timestamps.
Do not remove the legacy loader or recipes until unexplained differences are zero.

The Rust adapter is a deployment candidate, not yet the production ingestion
path. Production promotion requires the clean-database import, migration,
transaction rollback, idempotency, status-history, and SQL-versus-Rust parity
gates above to run successfully in the target release environment.
