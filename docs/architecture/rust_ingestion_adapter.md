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
aircraft-ingest curate list [--entity-id ID] [--field FIELD] [--limit N] [--format human|json]
aircraft-ingest curate accept --assertion-id ID [--format human|json]
aircraft-ingest curate reject --assertion-id ID [--format human|json]
aircraft-ingest curate refresh [--format human|json]
```

Equivalent repository commands are `just ingest-validate`,
`just ingest-import`, `just ingest-status`, `just curate-list`,
`just curate-accept`, and `just curate-reject`. Status reports include the full
attempt history for every matching logical run, newest attempt first.

Validation is database-independent. Import and status require
`APP__INGEST__DATABASE_URL`. Database credentials are never accepted as CLI
arguments or included in reports.

Exit codes are: 0 success, 2 configuration/usage, 3 input I/O, 4 validation,
5 identical import busy, 6 database/transaction, 7 parser consistency, and 8 a
curation decision the current state does not permit.

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

PlanePHD source confidence is 0.20 and its reliability is `UNVERIFIED`.
Measurements are non-canonical and assertions remain pending until curation
explicitly accepts them. Images are metadata only; no binary download occurs.

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

Run migrations with the migration owner, create the restricted ingestion login
role, then grant it ingest rights as an administrator. `install.sql` creates no
roles, so both steps are explicit. The password is read from the environment so
it never reaches the process argument list:

```bash
export INGEST_ROLE_PASSWORD='...'

psql -X -v ON_ERROR_STOP=1 "$MIGRATION_DATABASE_URL" \
  -v ingest_role=aircraft_ingest_app \
  -f database/roles/create_ingest_role.sql

psql -X -v ON_ERROR_STOP=1 "$MIGRATION_DATABASE_URL" \
  -v ingest_role=aircraft_ingest_app \
  -f database/roles/ingest_grants.sql
```

Locally the same two steps are `just db-create-ingest-role` and `just db-grants`,
which default to the `aircraft_ingest_app` name used in `.env.example`. In Aiven,
create the service user in the console instead of running
`create_ingest_role.sql`, then apply the grants file unchanged.

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

### How to run the gates

```bash
just test      # unit tests plus the containerized gates below
just snapshots # golden-snapshot regression check against committed snapshots
```

`apps/ingest/tests/gates.rs` drives the shipped `aircraft-ingest` binary against
a disposable `postgres:16-alpine` database carrying the canonical schema, and
covers clean-database import, idempotency, hard-validation rollback, status
history, stale-attempt recovery, and the documented exit codes.
`crates/aircraft_db/tests/ingestion_repository.rs` covers rollback of the long
staging transaction and concurrent imports at the minimum pool size. The schema
those tests install is the real `database/migrations/` sequence, and
`schema_steps_cover_every_migration` fails if a new migration is not gated.

`cargo xtask snapshots` imports one fixture through the adapter into a disposable
database, then diffs the normalized business snapshots in `database/snapshots/`
against the committed golden output in `database/snapshots/golden/<fixture>/`. Snapshots exclude generated IDs, timestamps, and curation
state (`is_canonical`, `is_accepted`, `status_code`, `confidence`), because the
two paths differ there by design — see below.

### Gate status

All six gates pass on `feat/rust-ingestion`. Closing them required three fixes to
the legacy loader, which had stopped promoting anything against the current
schema:

- `ON CONFLICT (source_id, source_system_key)` in `902` no longer matched an
  index, because migration 017 replaced `uq_sd_source_key` with the narrower
  `uq_sd_source_key_legacy`. Promotion aborted per record. Introduced by this
  branch.
- `promote_staged_aircraft()` failed with `column reference "variant_id" is
  ambiguous`: its `RETURNS TABLE` output columns collide with the table columns
  it writes. Fixed with `#variable_conflict use_column`. Pre-existing.
- `aircraft_ingest.parse_numeric` read `regexp_match(...)[1]`, which is the first
  capture group — the *optional decimal tail* — not the whole match. `'14000 FT'`
  parsed as NULL and `'8.5 GPH'` parsed as `0.5`. Pre-existing, and silently
  wrong rather than merely absent.

Each failure was swallowed into a `notes` column, so the legacy recipes exited
zero while promoting nothing.

### Robustness: the legacy loader aborts on malformed input

Running parity against `tests/fixtures/planephd_edge_cases.json` exposed a defect
class in `load_seed_json`: it stages records with **unguarded casts and no
per-record exception handler**, unlike `promote_staged_aircraft`. A single
malformed value anywhere in the file aborted the entire load and staged nothing.
Two instances were found and fixed:

- `(v_record->>'in_production')::BOOLEAN`, plus the `start_year` / `end_year`
  `::SMALLINT` casts. One record with `"in_production": "maybe"` killed the
  import of all three. Now guarded, unparseable values becoming NULL.
- `staged_images.href_raw` is `NOT NULL`, so an image with no `href` — or an
  array entry that is not an object — did the same. Now skipped.

The Rust adapter imports all three records, flagging each problem as a warning.
This is a categorical difference in failure mode, not a value difference, so it
does not appear in the snapshots.

### Accepted divergences from the legacy loader

Each divergent row is recorded with its reason in
`database/snapshots/known_divergences/<fixture>.json`; the gate subtracts exactly
those rows, fails on any new difference, and also fails if an accepted row stops
differing. Records are per fixture because what differs depends on the input.

On the minimal fixture, six of ten snapshots match exactly. On the edge-case
fixture only propulsion matches outright — but every divergence is a case where
the adapter is *more* conservative or more structured than the legacy loader, and
none is data the adapter loses:

- **Refuses untrustworthy measurements.** Legacy writes an all-NULL measurement
  row for a sentinel (`"--"`, `"none"`), and canonicalizes `"213 MPH"` as though
  213 were already in the canonical unit. The adapter writes no measurement and
  keeps the value as a pending assertion with `UNKNOWN_MEASUREMENT_UNIT`.
- **Refuses invented values.** Legacy rounds `"for_sale_count": "2.5"` to 3 and
  creates an all-NULL valuation row when every field is absent. The adapter
  records neither and flags `INVALID_INTEGER_FIELD`.
- **Refuses an unusable URL.** Legacy stores an `ftp://` `source_url` verbatim;
  the adapter rejects the scheme with `INVALID_URL`.
- **Structured curation flags.** Legacy concatenates every warning for a variant
  into one free-text blob with a NULL `field_name`; the adapter raises one flag
  per issue with a field path and a stable code.
- **Richer provenance.** The adapter preserves `raw_unit`, `asserted_numeric`,
  `source_path`, and `captured_from_key`, and asserts unmapped fields that legacy
  drops entirely.

Two further divergences are neither better nor worse, only different:

- **Source-record identity.** Legacy keys a variant by concatenated raw names
  (`CESSNA::172S Skyhawk SP`); the adapter uses SHA-256 over
  `planephd\0<raw manufacturer key>\0<raw aircraft key>`. Because
  `aircraft_core.variants.slug` is separately unique, two distinct records whose
  names normalize to one slug would otherwise abort the batch. The later record
  keeps a slug suffixed with a digest of its own record key — stable across
  replays — and opens a `SLUG_COLLISION` curation flag rather than being merged
  or dropped.
- **`captured_from_key`.** The column holds one source key, but several aggregate
  keys can feed one snapshot. Legacy keeps the last writer, the adapter the
  first; the totals themselves match.

### Curation

Measurements are written `is_canonical = FALSE` and assertions `PENDING`, so a
variant reaches `aircraft_read.mv_variant_search` with its identity but without
its measurements until a curator accepts them.
`imported_values_stay_pending_and_out_of_the_read_model` pins that contract.

Two gaps were closed here, both found the same way — by asking which columns the
read model exposes that the gate test did not assert were NULL.

Migration 019: `aircraft_specs.weight_metrics` had no `is_canonical` column and
the matview's weight aggregate filtered only on `configuration IS NULL`, so empty
weight, MTOW, and usable fuel capacity were served the moment they were ingested.

Migration 020: no market table had one either. Verified on a live database — after
importing the edge-case fixture the read model served
`papi_price_usd = 1150000.00`, `for_sale_count = 7`, and annual and hourly costs
for all three variants, while `cruise_speed_kias` and `gross_weight_lb` were
correctly withheld. An uncurated scraped **price estimate** was published on
import.

Market data is gated at the snapshot rather than the row.
`mv_ownership_cost_summary` wraps its sums in `COALESCE(..., 0)`, so filtering
individual line items would publish a confident `$0.00 annual cost` for an
uncurated aircraft — worse than withholding it. Filtering the snapshot removes the
row entirely and the `LEFT JOIN` yields NULL. Accepting any assertion on a
snapshot publishes it; it stays published while any of its assertions remains
accepted.

Migration 020 also had to rebuild `mv_variant_search`, which reads
`aircraft_market.valuations` directly in a LATERAL rather than through
`v_current_valuation`, and which depends on `mv_ownership_cost_summary` — so the
dependent is dropped first and rebuilt last, with both index sets captured and
replayed.

Closing that gate exposed a second defect: the adapter wrote **no `VALUATION`
assertions at all**. Price and for-sale count were the only values it recorded
without provenance, which also meant that once gated they could never be curated.
The adapter now asserts both, so they can be accepted like anything else.

`aircraft-ingest curate` applies the decisions:

```bash
just curate-list                  # what is awaiting a decision
just curate-accept <assertion-id> # publish that value
just curate-reject <assertion-id> # withdraw it again
```

Each decision is one transaction covering the assertion's status, the
`is_canonical` flag on the measurement it backs, the curation flags it closes,
and the read-model refresh, so the read model can never advertise a value whose
assertion was not accepted. Decisions are reversible — accepting in error can be
withdrawn — but repeating a decision already recorded is refused, so an unchanged
state is never reported as a change. Accepting a second assertion for a field
that already has one fails with `CURATION_CONFLICT` rather than silently racing
`uq_assertion_accepted`; withdraw the standing decision first.

Migration 020 gives each ingested performance or weight row a nullable,
uniquely indexed `source_assertion_id`. New ingestion always fills it, so two
sources can report the same metric without accepting one source publishing or
withdrawing another source's row. Historical measurements remain unlinked
rather than guessing a provenance association during upgrade. Migration 021
validates those foreign keys in a separate transaction so migration 020 does
not hold its stronger table locks during the validation scan.

A decision commits before the read model is rebuilt, so the rebuild never holds
its `ACCESS EXCLUSIVE` locks inside the curation transaction — and can therefore
fail with the decision already durable. Each decision that changes what the read
model serves enqueues a `aircraft_read.read_model_refresh_requests` row
(migration 022) in its own transaction, and that row is closed only by a rebuild
that succeeded. A failed rebuild is consequently not an error: the outcome
reports `read_model_refresh_pending`, and `aircraft-ingest curate refresh`
drains what is outstanding. Without it a stale read model would be
unrecoverable, because repeating the decision is refused as `ALREADY_DECIDED`.

The `curate` subcommands exit 8 for a decision the current state does not permit,
alongside the ingestion codes (2 configuration, 3 artifact, 4 validation,
5 already running, 6 persistence, 7 consistency).

### Retirement of the legacy SQL loader

`database/staging/901`–`903` and the `db-ingest` / `db-prod-ingest` recipes have
been removed. The retirement criteria in this document were met: parity ran clean
on two fixtures, with every remaining difference recorded and reviewed, and none
of them data the adapter loses.

With the second implementation gone, `cargo xtask snapshots` no longer validates the
adapter against anything else. It now imports a fixture and diffs the same
snapshot queries against committed golden output in
`database/snapshots/golden/<fixture>/`. That keeps regression coverage and loses
cross-implementation validation — a deliberate trade, since the loader it
compared against turned out to be the less correct of the two.

Add a fixture with `cargo xtask snapshots --fixture <path> --update`; a golden file
changing in review means the adapter's output changed, and the diff has to be
justified before it is re-recorded.

Golden snapshots prove the adapter still writes what it wrote before; they cannot
prove it writes the right thing, because they are a record of its own output.
`crates/aircraft_ingest/tests/normalization_properties.rs` recovers part of that
assurance by asserting the rules the adapter claims to follow, over generated
input rather than fixed fixtures: a sentinel never becomes a measurement, an
unmapped unit never reaches a canonical unit code, normalization is
deterministic, source-record identity is stable and collision-free, and
unrecognized input is preserved and flagged.

Those properties are mutation-checked. Mapping `MPH` to `KIAS`, or dropping
`"none"` from the sentinel set, each makes a property fail with a minimal
counterexample. The unit generator deliberately draws from a list of real
aviation units the adapter does not map, because a purely random `[A-Z]{2,6}`
generator would never produce `MPH` by chance and the property would pass while
the bug shipped.

What none of this covers is whether the mapping tables are themselves correct —
that `best_cruise_speed` should mean `SPEED_CRUISE_BEST`, or `GAL` should mean
`US_GAL`. Only a specification or an independent implementation can establish
that, and the independent implementation is the thing that was retired.

**The four fixes made to the legacy loader while closing these gates still matter
to `main`,** which retains it: `parse_numeric`'s capture-group bug, the
`variant_id` ambiguity, and the two unguarded-cast crashes. They are isolated in
`legacy-loader-main-hotfix.patch` at the repository root, verified against
`main`'s schema, and should land there independently of this branch.
