---
name: rust-testing
description: >
  How tests are written in this Rust workspace: TDD order, behavior-sentence naming with no tier
  prefixes, where a test and its helpers live, the disposable PostgreSQL harness, deterministic
  setup, table-driven cases, Drop-guard cleanup, exact-variant error assertions, proving a new
  test can fail, and which test crates are already available. Use whenever writing, reviewing,
  refactoring, or designing a Rust test. Triggers on "rust test", "cargo test", "nextest",
  "integration test", "test layout", "fixture", "property test", "proptest", "snapshot",
  "disposable postgres".
argument-hint: "[lite|full|exhaustive]"
license: MIT
---

# Rust tests in this workspace

A test is documentation of intended behavior that a machine can check. It earns its runtime by
pinning one externally meaningful invariant, deterministically, against the schema and contracts
that actually ship.

## Authority

This skill is the craft layer only. `crates/AGENTS.md` owns TDD order, naming, crate boundaries,
and the per-change verification table; `AGENTS.md` owns the repository-wide verification matrix
and the rule that a green run is not verification. Conflict order is the one `AGENTS.md` publishes
under *Sources of truth*. Flag a genuine conflict; never resolve one silently.

Also load `clean-code` (style), `rust-comment` (comments and rustdoc), `rust-review` (severity
scale), `rust-production` (the boundaries these tests pin), and `ponytail` — which decides whether
a test, helper, or dependency should exist at all.

## Decide where the check belongs, in this order

1. **The type system.** An invariant a newtype, private field, sealed trait, or validated domain
   constructor makes unrepresentable needs no test. Push it to the compiler first.
2. **The schema.** A `CHECK`, `UNIQUE`, foreign key, or trigger in `database/migrations/` is
   enforcement a Rust test cannot bypass — but `crates/AGENTS.md` still requires the domain
   constructor to validate, so this replaces neither the constructor nor its unit test.
3. **A `#[cfg(test)] mod tests` beside the code.** Private state transitions, bounded-diagnostic
   helpers, pure mappings. Default home. Examples: `sanitize_database_message` in
   `aircraft_db`, `sanitize_failure` in `aircraft_app`, the capture-and-limit test in
   `aircraft_ingest/src/artifact.rs`.
4. **`tests/<domain>.rs` in the owning crate.** The public boundary, one file per functional
   domain — `crates/aircraft_db/tests/ingestion_repository.rs`,
   `crates/aircraft_ingest/tests/normalization_properties.rs`,
   `crates/aircraft_testsupport/tests/migration_upgrade.rs`.
5. **`apps/ingest/tests/gates.rs`.** User-visible CLI behavior, driven through the real binary via
   `env!("CARGO_BIN_EXE_aircraft-ingest")` — exit codes, human and JSON output, redaction.
   `AGENTS.md` requires CLI behavior to be exercised this way when possible.
6. **Not a doctest.** `just test` is `cargo nextest run`, which does not execute doctests, and CI
   has no `--doc` step. A doctest here is unrun prose. See `rust-comment`.

## Binding in this workspace

- **TDD.** Write the failing test first, then the minimum change, then refactor green. If a test
  cannot precede the change, say why (`crates/AGENTS.md`).
- **Names are behavior sentences in `snake_case`** — `imported_values_stay_pending_and_out_of_the_read_model`,
  `a_hard_validation_failure_writes_no_aircraft_data_but_records_the_attempt`,
  `second_pass_key_mismatch_rolls_back`. The name states the externally meaningful outcome, never
  the function called. **Do not invent tier, milestone, or epic prefixes** — `crates/AGENTS.md`
  forbids prefixes the repository does not use.
- **One behavior per test.** Several assertions are right when together they prove that one
  behavior; put the distinguishing case in the assertion message.
- **Prove the test can fail.** `AGENTS.md` records two properties in this repository that passed
  vacuously — a loop body that never ran, a generator that could not produce the failing input.
  Mutate the production code and confirm the test goes red before counting it as coverage. The
  doc comment on `a_sentinel_yields_no_measurement_while_a_real_value_still_does` exists to record
  which half of the assertion is the anti-vacuity guard; imitate that.
- **Shared helpers live in `aircraft_testsupport`** — the disposable `postgres:16-alpine`
  container, `SCHEMA_STEPS`, `install_schema`, and the `ImportRequest` builders. It is
  dev-only: production crates depend on it from `[dev-dependencies]` and nowhere else. A
  `tests/common/mod.rs` is for helpers that genuinely must not leave one crate; a bare
  `tests/common.rs` is never right, since Cargo builds it as its own test binary.
- **A new migration updates the harness.** `SCHEMA_STEPS` and `COVERED_MIGRATIONS` embed the
  canonical install order with `include_str!`, mirroring `database/install.sql`;
  `schema_steps_cover_every_migration` is the drift guard. Adding SQL without updating both
  passes every existing test and is still wrong.
- **Runner of record:** `just test` → `cargo nextest run --workspace --locked`. The fast subset
  that starts no containers is `cargo test --workspace --lib --locked`. Report what actually ran.
- **Integration tests need Docker.** They start disposable `postgres:16-alpine` containers. A
  missing Docker daemon or `cargo-nextest` is an environment limitation to state plainly, never a
  pass. Never point one at a shared or production database.
- **Never weaken a test to make it pass**, and never weaken a validation, provenance, curation,
  migration, lint, or authorization control to make one pass (`AGENTS.md`). A flaky test is a
  defect: quarantining one needs an owner, an expiry, and an analysis of which gate it covered.

## Writing the test

- **Deterministic setup.** No network, no ambient environment, nothing outside the assigned
  container or temporary directory. Inject time and randomness rather than reading a clock.
- **No shared mutable state, with no exception.** Tests run concurrently under nextest, so do not
  mutate a global. The process environment is not an escape hatch: Rust 2024 makes
  `std::env::set_var` `unsafe` and `Cargo.toml` sets `unsafe_code = "forbid"`, so an
  environment-mutating test cannot compile here at all. Inject it instead — `aircraft_config`'s
  precedence tests pass a map to `config::Environment::source`, which routes it through the same
  prefix and separator handling as `env::vars_os`, so the shipped mapping is still what runs.
- **`Result` return so setup propagates with `?`.** The workspace type is
  `aircraft_testsupport::TestResult` (`Result<T, Box<dyn Error + Send + Sync>>`). Reserve
  `expect` for the assertion itself with a message that names the invariant, and keep the
  `#![allow(clippy::expect_used)]` opt-out local to the test module — `unwrap_used` is `deny`
  workspace-wide.
- **Assert the exact error variant and the exact issue code.** Match the enum, never `.is_err()`;
  a new variant must break the test. Stable issue codes (`DUPLICATE_SOURCE_RECORD_KEY`,
  `UNSUPPORTED_RECORD_FIELD`) and status strings are contract — pin them literally. Pair every
  `#[should_panic]` with `expected = "..."`.
- **Assert the absence, not only the presence.** The characteristic gate in this repo asserts both
  that a failure was reported *and* that nothing was published: no promoted rows, no accepted
  assertion, no read-model entry. A rollback test that only checks the error proves half of it.
- **Table-driven over near-identical functions**, with the case in the failure message, and an
  exhaustive `match` in the expectation table so a new variant is a compile error rather than an
  untested one:
  ```rust
  #[test]
  fn sentinels_and_numeric_prefixes_match_legacy_contract() {
    const CASES: [(&str, Option<&str>); 3] =
      [("n/a", None), ("  ", None), ("$1,250", Some("1250"))];

    for (raw, expected) in CASES {
      assert_eq!(parse_numeric(raw).as_deref(), expected, "input {raw:?}");
    }
  }
  ```
- **Expected values come from the migration or the owning document, not from the code.** A copied
  `matches!` passes for any policy, including a wrong one.
- **Cleanup by `Drop`, never by trailing statements** — an assertion failure skips them.
  `aircraft_testsupport::DockerPostgres` implements `Drop` for exactly this; follow it for temp
  roots and spawned children.
- **No sleeps for synchronization** (`crates/AGENTS.md` §7). Use readiness checks and deadlines.
- **Fake the boundary, not the logic.** `aircraft_app` tests drive the real use case through port
  fakes (`IngestionStore`, `IngestionUnitOfWork`, `PreparedRecordReader`) and assert the failure
  *ordering* — rollback before audit, audit surviving rollback. Do not reimplement the use case in
  the fake.
- **`Debug` + `PartialEq` on domain types** so `assert_eq!` prints a usable diff;
  `pretty_assertions` is available where the diff is large.
- **Arrange / Act / Assert**, separated by blank lines. Gate feature-dependent tests with
  `#[cfg(feature = "...")]` so `--no-default-features` still builds warning-free.

## What is already available, and what is not

In `[workspace.dependencies]` and in use: **`proptest`** (`aircraft_ingest` invariants),
**`pretty_assertions`** (`aircraft_db`), **`tempfile`** (`xtask`, `aircraft_config`), and `tokio`
with `macros` for `#[tokio::test]`. Use them; they cost nothing new. `serial_test` is deliberately
**not** here: it was declared for the environment-mutating tests the rule above rules out, was
never used, and was removed.

Declared but unused: **`insta`**. Snapshot regression in this repo currently runs through
`cargo xtask snapshots`, which imports a fixture into a disposable database and diffs normalized
business snapshots against committed golden output in `database/snapshots/`. Prefer that gate for
anything schema-shaped; reach for `insta` only with a reason the golden gate cannot cover.

Not in the tree: `rstest`, `quickcheck`, `loom`, `mockall`, `wiremock`. Each would be a new
dependency under `AGENTS.md` ("add only when they remove more risk or code than they add") and
`ponytail`: name the risk it removes and the hand-written alternative, then let the user decide —
never add one silently to satisfy this skill.

**A golden snapshot cannot prove correctness.** The module comment on
`normalization_properties.rs` says it plainly: snapshots are a record of the adapter's own past
output, so they prove *unchanged*, never *right*. Regenerating one to make a diff green converts
a regression into a passing build. Property tests close part of that gap; nothing but a
specification or an independent implementation closes the rest, and the mapping tables themselves
remain unproven. Say so rather than implying the gate covers more than it does.

## Reporting

Say where the test lives, which invariant it pins, and how you confirmed it can fail — one line,
in the change summary. Name any suite you could not run and why (no Docker, no `cargo-nextest`).
No separate strategy table.

## Intensity

| Level | Behavior |
| :--- | :--- |
| **lite** | Colocated unit tests, behavior-sentence names, deterministic setup, exact error variants. |
| **full** | Everything under *Binding* and *Writing the test*, including the mutation check and the absence assertions. Default. |
| **exhaustive** | Adds property and boundary coverage, failure-path and concurrency cases (idempotent replay, busy duplicate, rollback ordering), and an explicit dependency proposal where a missing tool would genuinely pay for itself. |
