---
name: rust-production
description: >
  The boundary-facing craft rules for this workspace, each one codified from a module that already
  proves it: admitting untrusted source bytes (capture once, bound, hash, sanitize), verifying a
  streamed second pass against its preflight, owning a database transaction (advisory lock on
  logical identity, one fate for promotion, durable audit outside the rollback, bounded pool
  admission), preserving provenance without granting it canonical status, and evolving a durable
  contract or the schema. REQUIRED before writing, generating, or editing any Rust here, and used
  to review a diff that reads untrusted input, opens a transaction, changes an identity, or
  changes a published format or migration.
---

# Production Rust in this workspace

`crates/AGENTS.md` says to begin from the closest working example rather than invent a shape.
This file is that instruction applied to the parts that face untrusted input, PostgreSQL, and
durable contracts — where inventing a shape costs a half-published aircraft, a lost run attempt,
or an identity that silently changes. Every rule below is already implemented and tested in this
tree; the citation is the specification.

## Authority

Conflict order is the one `AGENTS.md` publishes under *Sources of truth*: nearest `AGENTS.md` →
manifests and tests → `database/migrations/` → `justfile` → the owning document
(`docs/architecture/rust_ingestion_adapter.md`, `database/README.md`,
`database/implementation_notes.md`) → `README.md` → this skill. Migration SQL wins over this file
and over prose. Flag a genuine conflict; never resolve one silently.

Siblings own what this file does not restate: `clean-code` (style catalogue), `ponytail` (whether
it should exist), `rust-comment` (comment and rustdoc content), `rust-review` (review conduct and
the severity scale), `rust-testing` (test policy). This file states the *authoring* rule; where
`rust-review` states the matching review finding, it is not repeated here.

## Untrusted input boundary

Canonical: `crates/aircraft_ingest/src/artifact.rs`, `planephd.rs`, `normalization.rs`.

- **Capture once into an immutable artifact, then reopen it.** `InputArtifact::capture` streams
  file or stdin bytes into an owner-only temporary file while hashing them, and every later pass
  calls `reopen()`. Re-reading stdin is impossible and re-reading a path is a TOCTOU bug: the
  bytes you hashed must be the bytes you parse.
- **Enforce the byte ceiling while reading, not after.** The running total is
  `checked_add`-ed and compared against `max_bytes` on every chunk, so an oversized or
  length-lying input is refused before it is stored, not after. A limit checked at the end is not
  a limit.
- **Hash exact bytes.** SHA-256 over the captured stream is half of the logical import identity;
  it must not be computed from a re-serialization, a normalized form, or a parsed value.
- **Report a sanitized locator, never a host path.** `sanitize_locator` strips control characters
  and truncates to 255 characters, and the caller passes a file *basename* or `<stdin>`. A path
  in a report, a log, or a database column leaks the host.
- **Parse on a real thread and hand records over a bounded channel.** `open_records` spawns a
  named `std::thread` and sends over `mpsc::channel(16)`. Blocking `serde_json` work does not
  belong on an async executor thread, and an unbounded channel converts a large source into
  memory exhaustion.
- **Bound what a diagnostic retains.** Preflight keeps at most 100 error issues; a source that is
  wrong in a million places must not produce a million-entry report.
- **Unknown inbound shape is evidence, not a rejection.** An unrecognized PlanePHD field becomes
  an `UNSUPPORTED_RECORD_FIELD` warning and survives in the preserved raw JSON. This is deliberate
  and directional: the source is a live third-party scrape, and refusing the batch on a new field
  would lose records the raw JSON could have kept. It is the opposite of the rule for a format
  this project defines — see *Durable contracts*.
- **A sentinel is not a value and an unmapped unit is not a unit.** `parse_sentinel` and the unit
  mapping tables exist so an ambiguous optional value stays pending evidence. Never substitute a
  default, skip a malformed record, or canonicalize a guess.

## Two-pass consistency

Canonical: `PlanePhdAdapter::preflight`, `IngestionService::consume_records`.

- **Preflight the complete artifact before opening the import transaction.** Hard structural or
  domain errors reject the batch while nothing is held.
- **Preflight computes an ordered fingerprint, and the second pass must reproduce it.**
  `record_keys_sha256` hashes each `source_record_key` followed by a `\0` separator, in order.
  The separator is load-bearing: without it, adjacent keys concatenate and distinct orderings
  collide.
- **Verify count, warnings, and fingerprint after streaming, and fail the import on divergence.**
  `ParserRecordKeysMismatch` and `WarningCountMismatch` are separate variants so a test — and a
  reader — can tell an identity divergence from a count divergence. Collapsing them loses the
  distinction that makes the check diagnosable.
- **Duplicate source record keys are rejected in preflight**, before anything is written, not
  discovered by a unique-constraint violation mid-transaction.

## Transaction ownership and durable audit

Canonical: `crates/aircraft_db/src/repositories/ingestion_repository.rs`,
`crates/aircraft_app/src/services/ingestion.rs`.

- **Logical import identity is source slug + content SHA-256 + parser name + parser version.**
  A parser-version change intentionally creates a new logical run: the same bytes read by
  different code are a different import. Do not add, drop, or reorder a component of that key
  without treating it as a durable compatibility change.
- **Serialize identical logical imports with a transaction-scoped advisory lock.**
  `pg_try_advisory_xact_lock(hashtextextended($1, 0))` over that key. `try` rather than blocking
  is the point: a concurrent identical import reports `Busy` instead of queueing behind a
  long-running transaction. A prior success is an idempotent replay
  (`ImportStart::AlreadySucceeded`), not a second import.
- **Bound pool admission explicitly wherever one operation holds two connections.** Startup owns
  the long-running import transaction *and* a second connection for the audit transaction, so an
  owned semaphore permit is acquired first. Without it, enough concurrent imports deadlock the
  pool at small sizes, and the failure looks like a hang rather than a limit. The permit lives in
  the unit of work so it is released with the transaction.
- **Everything that makes an import true commits together.** Staging, promotion, provenance,
  assertions, curation flags, run completion, and the read-model refresh share one transaction and
  one fate. Never publish partial canonical state after a hard failure.
- **The failure audit is a separate transaction, on purpose.** Rolling back the import must not
  erase the record that the attempt happened. A retry closes the stale `IMPORTING` attempt with
  `PROCESS_TERMINATED` rather than leaving it open forever.
- **Follow-up work is enqueued inside the transaction that makes it necessary, and performed
  after commit.** `CurationStore::decide` writes the refresh request in the same transaction as
  the decision, then refreshes. A post-commit refresh failure is *not* an error: the decision is
  durable, so the outcome carries `read_model_refresh_pending` and the enqueued request keeps the
  stale read model recoverable through `refresh_read_models`, which is idempotent and safe when
  nothing is pending. Reporting a failure there would tell the operator to redo work that already
  landed.
- **Bind every runtime value.** Never interpolate source-controlled data into SQL. Keep query
  text, column names, row types, and SQLx types inside `aircraft_db`.
- **Runtime uses the restricted ingest role.** It must not gain schema, extension, table-drop, or
  role-administration privileges; migrations use the privileged migration role
  (`database/roles/ingest_grants.sql`).

## Provenance, curation, and evidence

Canonical: `curation_repository.rs`, `services/curation.rs`, migration `017` onward.

- **Imported measurements stay non-canonical and outside canonical read models until explicit
  curation.** Ingestion never accepts its own assertions; `imported_values_stay_pending_and_out_of_the_read_model`
  is the gate that says so.
- **Preserve raw records, warnings, and normalization issues as first-class data.** They are
  curation work, not noise to be summarized away.
- **Never rewrite history.** Historical raw JSON, parser identity, and assertion provenance stay
  as recorded on the run that produced them.
- **A collision is flagged evidence, not an overwrite.** A slug collision is recorded and flagged
  rather than resolved by guessing.
- **Images are metadata only.** Ingestion does not download source image binaries.

## Diagnostics, secrets, and bounds

- **Every message that can reach a terminal, a log, or a database column is bounded and
  control-stripped.** `sanitize_database_message` caps at 1000 characters and drops control
  characters; `sanitize_failure` does the same for the run audit. A new error path that formats a
  database or source string must go through one of them — an escape sequence in a database error
  is a terminal-injection surface, and an unbounded message is a row that will not fit.
- **Credentials live in `SecretString`** (`IngestSettings::database_url`) and come from
  `APP__`-prefixed configuration, never a CLI argument or a report. `expose_secret` appears once,
  at the point of connecting.
- **Never log a database URL, credential, authorization header, raw personal data, a full source
  payload, or an unsanitized local path.** Use structured `tracing` fields — run ID, attempt ID,
  stage, counts, timings, error class — and keep terminal output separate from the versioned JSON.
- **Keep retry, timeout, pool, statement-timeout, lock-timeout, input-size, record, channel, and
  diagnostic bounds explicit and configurable at the owning layer.** A bound that only exists as a
  literal in the middle of a function is a bound nobody can tune.

## Durable contracts

Canonical: `REPORT_SCHEMA_VERSION` in `aircraft_app`, `apps/ingest/src/main.rs`,
`crates/aircraft_api/`, `docs/openapi.json`.

- **A format this project defines is versioned and strict.** The JSON report carries
  `REPORT_SCHEMA_VERSION`; status and curation output use pinned `rename_all` spellings. Moving a
  shape means moving the version deliberately and preserving backward compatibility or documenting
  the migration — not editing a struct. Tool *output* and scraped source are the only lenient
  boundaries, and `rust-review` records why.
- **Exit codes are contract.** `apps/ingest/src/main.rs` maps failure classes to stable codes
  (2 configuration, 3 artifact capture, 4 input, 5 already running, 6 persistence, 7 parser
  consistency), and curation reuses the same vocabulary. `documented_exit_codes_hold` currently
  pins only 2 and 3 through the real binary; the rest are contract by the mapping alone, so
  renumbering one is a breaking change a test will not catch for you.
- **Issue codes are contract.** `DUPLICATE_SOURCE_RECORD_KEY`, `UNSUPPORTED_RECORD_FIELD`, and the
  rest are what a curator greps for. `only_the_documented_codes_are_error_severity` guards the
  severity half.
- **Generated artifacts are regenerated by their checked-in command, never hand-edited.**
  `docs/openapi.json` comes from `just generate-docs`; `just generate-docs --check` detects drift.

## Schema evolution

- **Migrations are immutable once hashed** in `database/migrations.lock.json`.
  `cargo run -p xtask -- migrations` rejects an edit to an applied file, and also checks numeric
  order, an explicit `BEGIN`/`COMMIT`, an installer entry, a validation companion under
  `database/validation/`, and the Squawk baseline. A stale comment in a migration is corrected in
  `database/data_dictionary.md` or `database/implementation_notes.md`.
- **Update SQL first, verify it in a clean disposable database, then update the prose.** Never
  change schema behavior from the data dictionary alone.
- **A new migration updates `aircraft_testsupport::SCHEMA_STEPS` and `COVERED_MIGRATIONS`.** They
  mirror `database/install.sql` with `include_str!`, and `schema_steps_cover_every_migration` is
  the only guard. Seed ordering is load-bearing: reference units before lookup data, mission
  profiles at the point the comparison and read-model migrations require.

## Not settled here

Named so they are visible, with no rule attached — do not infer one from this file:

- **The HTTP runtime.** `aircraft_api` has a health route and a generated contract; request-size
  limits, timeouts, rate limits, authentication, authorization, and middleware ordering are
  designed in `AGENTS.md` but not implemented. `apps/server` is a working composition root that
  boots, builds a bounded pool, and serves the health router; it is no longer excluded legacy
  source.
  Writing the first real route means proposing those rules, not inferring them from this file.
- **Performance method.** Nothing here says what earns a benchmark or how to measure before
  optimizing.
- **SQLx compile-time query checking.** `just check-offline` is currently equivalent to
  `just check` and the justfile says why. Introducing `sqlx::query!` means introducing and
  verifying the offline-metadata workflow first.
- **Retry and backoff policy for transient database failures**, beyond the retryable curation
  refresh described above.

Proposing a rule for any of these is a conversation with the user, not an edit to this file.
