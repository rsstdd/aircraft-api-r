# AGENTS.md — Rust Workspace

## Codebase

This directory contains the eight Rust library crates that form the target
hexagonal architecture:

- `aircraft_domain` owns deterministic domain values and invariants;
- `aircraft_app` owns use cases and ports;
- `aircraft_api` owns Axum transport contracts and OpenAPI;
- `aircraft_db` owns SQLx persistence and database mappings;
- `aircraft_ingest` owns source capture, parsing, and normalization;
- `aircraft_config` owns typed configuration and secret-bearing settings;
- `aircraft_observability` owns tracing setup; and
- `aircraft_testsupport` owns shared disposable-PostgreSQL test support.

The root `AGENTS.md` governs the whole repository. This file adds rules and
routing for work inside `crates/`. If a crate later gains its own `AGENTS.md`,
that file overrides this one only within that crate.

**Current state.** The working implementation is the PlanePHD ingestion vertical
slice across `aircraft_domain`, `aircraft_app`, `aircraft_ingest`, and
`aircraft_db`, with composition in `apps/ingest`. `aircraft_api` has a tested
health route and generated OpenAPI contract but no runnable server.
`aircraft_config`, `aircraft_observability`, and `aircraft_testsupport` provide
the portions needed by the current slice. Most aircraft search, comparison,
mission, middleware, claims, and general repository modules are empty or
comment-only scaffolds. Do not cite their names as evidence of implemented
behavior, imitate them as examples, or fill them with more placeholders.

## Rules

- Read the root `AGENTS.md`, the target crate manifest, its implementation and
  tests, and any database or architecture document coupled to the change before
  editing.
- Follow test-driven development for behavioral work: add or adjust a focused
  failing test, make the smallest production change that passes it, then
  refactor only within scope. If a test cannot precede the change, explain why.
- Name tests as observable behavior in `snake_case`, following the current crate
  idiom. Do not invent milestone or tier prefixes that the repository does not
  use.
- One test should establish one behavior. Multiple assertions are appropriate
  when together they prove that behavior.
- Public APIs need rustdoc when callers must understand invariants, error
  conditions, security boundaries, units, ownership, or durable contract
  semantics. Do not add doc comments that merely repeat a public name or type.
- Document non-trivial cross-file and code-to-SQL coupling at the owning points.
  Name the coupled migration, validation, generated artifact, or protocol when
  a future change could otherwise update only one side.
- Keep representations separate. Source JSON, prepared application records,
  domain values, SQL rows, HTTP DTOs, and generated OpenAPI schemas must not
  collapse into one shared convenience type.
- Prefer an existing port, typed error, value object, or repository boundary
  over a new abstraction. Do not introduce framework-general infrastructure for
  a single call site.
- Use distinct typed error variants for distinct caller-visible failures when
  callers or tests must react differently. Preserve safe source context at the
  composition boundary.
- Never make an empty module appear implemented with a stub, unconditional
  success, dummy value, or `todo!()`. Complete a tested vertical behavior or
  leave the scaffold visibly incomplete.
- Do not revive Actix, Diesel, template-era crate names, or archived source to
  make a target crate compile.
- Avoid new dependencies when the standard library or an existing workspace
  dependency is sufficient. Add shared versions through `[workspace.dependencies]`.
- Keep `Cargo.lock` committed. After manifest edits run
  `cargo metadata --no-deps` before compilation.
- Before reporting done, review the diff for dependency-direction violations,
  accidental public API expansion, hidden I/O, unbounded work, weak errors,
  placeholder behavior, and missing tests.

## Dependency boundaries

```text
aircraft_api ---------> aircraft_app ---------> aircraft_domain
aircraft_db ----------> aircraft_app ---------> aircraft_domain
aircraft_ingest ------> aircraft_app ---------> aircraft_domain
aircraft_config        (infrastructure utility; no business behavior)
aircraft_observability (infrastructure utility; no business behavior)
aircraft_testsupport -> test-facing ports and PostgreSQL harness only
```

The application binaries compose these libraries. Libraries do not depend on
`apps/`, `xtask`, or archived code.

### `aircraft_domain`

- Keep it deterministic, synchronous, and independent of Axum, SQLx, Tokio,
  configuration loaders, tracing setup, filesystems, and networks.
- Model closed domain vocabularies and invalid states with dedicated types and
  enums rather than unchecked strings or primitives.
- Preserve explicit units and measurement conditions. Do not compare or
  aggregate values with ambiguous units.
- Domain constructors validate invariants and return typed errors. Do not rely
  on an adapter or database constraint as the only enforcement.
- Use pure unit and property tests. No async runtime or PostgreSQL belongs in
  domain tests.

### `aircraft_app`

- Define use cases, application inputs, reports, transaction abstractions, and
  ports here.
- Depend on domain types, not HTTP DTOs, SQLx pools, database rows, or
  source-specific JSON shapes.
- Keep transaction intent explicit while leaving concrete transaction mechanics
  to persistence adapters.
- Bound streams, queues, retries, and concurrency in application contracts.
- Test orchestration with port fakes, including failure ordering, rollback,
  invariant mismatches, and durable audit behavior.

### `aircraft_api`

- Own Axum routes, extractors, request validation, response DTOs, status/error
  mapping, middleware, and Utoipa declarations.
- Call application use cases; do not issue SQL or deserialize directly into
  domain or database row types.
- Treat request bodies, path/query parameters, headers, and authentication data
  as untrusted.
- Keep generated OpenAPI synchronized through `just generate-docs` and
  `just generate-docs --check`.
- A route is not complete without request, response, failure-status, and limit
  tests through the router.

### `aircraft_db`

- Implement application ports with SQLx. Keep pools, connections,
  transactions, query text, column names, and row types inside this crate.
- Bind every runtime value. Never interpolate source-controlled data into SQL.
- Map rows explicitly into application or domain representations. Do not leak
  SQLx types across the boundary.
- Make transaction ownership and commit/rollback points visible.
- Reconcile every schema-dependent change with canonical files under
  `database/migrations/`; migration SQL wins over Rust assumptions.
- Current queries are runtime-checked. Do not introduce SQLx compile-time macros
  without also introducing and verifying the required offline metadata workflow.
- Repository integration tests use a disposable PostgreSQL container and the
  canonical migration/seed order, never a test-local imitation of the schema.

### `aircraft_ingest`

- Own secure artifact capture, source adapters, parsing, normalization, stable
  issue codes, and source-specific record identity. Do not issue SQL.
- Capture source bytes once, enforce the configured limit, hash exact bytes, and
  parse the same immutable artifact during preflight and streaming.
- Keep parsing bounded. Do not retain the complete normalized dataset when a
  bounded record stream suffices.
- Preserve raw evidence and warnings. Ambiguous optional values stay pending
  rather than becoming canonical guesses.
- Keep PlanePHD-specific names and mappings out of domain types and application
  orchestration.
- Parser-version or identity-contract changes are durable compatibility changes;
  update tests and owning documentation deliberately.

### `aircraft_config`

- Parse file and environment input into typed settings. Environment variables
  use the `APP` prefix and `__` nesting.
- Store database URLs and future credentials in `SecretString` or an equivalent
  non-revealing type.
- Validate zero, range, relationship, path, and pool-size constraints before
  constructing runtime resources.
- Test defaults, environment precedence, invalid boundary values, and secret
  redaction when configuration behavior changes.
- Do not place business defaults or domain policy in infrastructure settings.

### `aircraft_observability`

- Own tracing subscriber and exporter initialization, not business events or
  policy.
- Libraries emit structured `tracing` events; they do not call `println!`,
  `eprintln!`, or `dbg!` in application paths.
- Never record secrets, database URLs, authorization headers, full source
  payloads, personal data, or unsanitized local paths.
- Initialization must be explicit and safe when tests or embedding applications
  already installed a subscriber.

### `aircraft_testsupport`

- Keep this crate test-only. Production dependencies must not depend on it.
- Provide reusable fixtures and infrastructure, not assertions that hide test
  intent.
- Install the canonical schema in its documented order. A new migration must
  update the coverage list and drift test.
- Every container must be disposable, isolated, bounded by readiness timeouts,
  and cleaned up even when a test fails.
- Do not embed private datasets or production credentials in fixtures.

## Best existing examples

When adding behavior, begin with the closest working example and preserve its
boundary, not necessarily every local formatting choice:

| Need | Start with | Why |
|---|---|---|
| Pure validated value and typed invariant error | `aircraft_domain/src/ingestion.rs` | Deterministic constructor and exact error assertions |
| Application use case, ports, and failure orchestration | `aircraft_app/src/services/ingestion.rs` | Explicit stream and transaction boundaries with port fakes |
| Bounded immutable input capture | `aircraft_ingest/src/artifact.rs` | Size limit, exact hash, secure temporary file, sanitized locator |
| Streaming source adapter and preflight | `aircraft_ingest/src/planephd.rs` | Two-pass parse, stable identity, bounded channel |
| Source normalization and stable issues | `aircraft_ingest/src/normalization.rs` | Explicit mappings, pending warnings, focused fixtures |
| SQLx port implementation and transaction ownership | `aircraft_db/src/repositories/ingestion_repository.rs` | Parameter binding, advisory lock, audit and rollback semantics |
| Repository integration behavior | `aircraft_db/tests/ingestion_repository.rs` | Canonical schema in disposable PostgreSQL |
| Shared PostgreSQL harness | `aircraft_testsupport/src/lib.rs` | Migration drift coverage and cleanup guard |
| Axum route and OpenAPI declaration | `aircraft_api/src/routes/health.rs` and `aircraft_api/src/lib.rs` | Router-level behavior and generated contract |
| Typed ingestion configuration | `aircraft_config/src/settings/structs.rs` | `APP__` mapping, defaults, bounds, secret URL |

Do not imitate one-line comment scaffolds in aircraft/search/comparison modules,
empty legacy modules, or files under `archive/`.

## Routing table

| When looking for | Look in |
|---|---|
| Whole-repository rules and approval boundaries | `../AGENTS.md` |
| Workspace membership, dependencies, and lints | `../Cargo.toml`, each crate's `Cargo.toml` |
| Rust formatting configuration | `../.rustfmt.toml` |
| Architecture and current status | `../README.md` |
| Ingestion architecture and deployment gates | `../docs/architecture/rust_ingestion_adapter.md` |
| Canonical schema | `../database/migrations/` |
| Ingestion schema additions | `../database/migrations/017_rust_ingestion_adapter.sql` |
| Database validation for ingestion | `../database/validation/017_rust_ingestion_adapter_validation.sql` |
| Domain ingestion invariants | `aircraft_domain/src/ingestion.rs` |
| Ingestion use case and ports | `aircraft_app/src/services/ingestion.rs` |
| Input artifact capture and SHA-256 | `aircraft_ingest/src/artifact.rs` |
| PlanePHD format and source identity | `aircraft_ingest/src/planephd.rs` |
| PlanePHD field and unit mappings | `aircraft_ingest/src/normalization.rs` |
| SQLx ingestion implementation | `aircraft_db/src/repositories/ingestion_repository.rs` |
| Repository integration tests | `aircraft_db/tests/ingestion_repository.rs` |
| Disposable PostgreSQL harness and migration coverage | `aircraft_testsupport/src/lib.rs` |
| Configuration parsing | `aircraft_config/src/settings/structs.rs` |
| Tracing initialization | `aircraft_observability/src/logging.rs` |
| Axum health contract | `aircraft_api/src/routes/health.rs`, `aircraft_api/src/lib.rs` |
| Generated OpenAPI | `../docs/openapi.json`, generated from `aircraft_api` |
| CLI composition and deployment gates | `../apps/ingest/` |
| Source fixture | `../tests/fixtures/planephd_minimal.json` |

## Verification

Run the narrowest relevant test first, then broaden. Unit tests passing is not
completion for behavior that crosses crate or PostgreSQL boundaries.

| Change | Required verification |
|---|---|
| Documentation in `crates/` | Markdown structure, links, referenced paths, commands, and agreement with code/manifests |
| One pure domain invariant | Targeted unit test, `cargo test -p aircraft_domain`, fmt, targeted Clippy |
| Application orchestration | Focused port-fake test, `cargo test -p aircraft_app`, fmt, targeted Clippy |
| Parser or normalization | Focused fixture/boundary tests, `cargo test -p aircraft_ingest`, then library suite |
| Configuration | Focused default/invalid/redaction tests, `cargo test -p aircraft_config` |
| API DTO or route | Router tests, `cargo test -p aircraft_api`, `just generate-docs --check` |
| SQLx repository | Targeted Docker-backed repository test and canonical schema validation |
| Shared PostgreSQL harness | Harness drift test plus every integration suite that consumes the changed behavior |
| Public cross-crate contract | Producer and consumer tests, `cargo test --workspace --lib`, `just check` |
| Manifest or dependency | `cargo metadata --no-deps`, targeted check/test, workspace check, dependency gates as available |

Common commands from the repository root:

```bash
cargo metadata --no-deps
cargo fmt --all -- --check
cargo test --workspace --lib
just check
just test
just lint
just generate-docs --check
```

`just test` runs cargo-nextest for the workspace and includes Docker-backed
integration tests that start disposable `postgres:16-alpine` containers. Docker
and cargo-nextest must be available. `just lint` also needs cargo-audit and
cargo-deny. Report a missing tool or daemon as an environment limitation and
state exactly what remains unverified.

Do not run integration tests against a shared or production database. Do not run
`just db-reset` or `just db-rebuild` without explicit approval.

## Quality bar

- Dependency direction remains inward and acyclic.
- Invalid input fails before database mutation.
- Hard ingestion failure does not publish partial aircraft data.
- Raw source evidence, provenance, assertions, and curation flags survive as
  designed.
- Pending source values never become canonical or enter canonical read models
  without an explicit curation path.
- Identical logical imports remain idempotent and concurrent duplicates remain
  bounded.
- Input, queues, database concurrency, timeouts, retries, diagnostics, and test
  infrastructure are bounded.
- Public machine-readable contracts are versioned and tested.
- Secrets and sensitive payloads do not enter errors, logs, fixtures, snapshots,
  or debug output.
- `cargo fmt --all -- --check` and relevant Clippy checks are clean after Rust
  changes.
- Unimplemented behavior is never stubbed or documented as implemented.

# Formatting rules

The checked-in `.rustfmt.toml` is the mechanical source of truth for Rust layout.
It selects Rust and style edition 2024, two-space indentation, a 100-column
maximum, Unix newlines, field-init shorthand, try shorthand, stable
import/module ordering, and maximum small-item heuristics. The Rust Style Guide
resolves cases rustfmt does not cover. Root and nested `AGENTS.md` rules win on
architecture, safety, comments, and repository conventions.

## 0. Operating rules

- Write code that already follows rustfmt's shape; do not defer basic layout to
  a final cleanup pass.
- After Rust edits run `cargo fmt --all`. Before completion run
  `cargo fmt --all -- --check`.
- Do not change `.rustfmt.toml`, add unstable options, use
  `#[rustfmt::skip]`, or hand-align code unless the task explicitly changes
  repository formatting policy.
- Do not reformat untouched files or combine formatting churn with a behavioral
  change.
- Keep generated files under their generator. Do not hand-format generated
  output.

## 1. Whitespace and width

- Use two spaces per indentation level and no tabs outside string literals.
- Keep code lines within 100 columns and avoid trailing whitespace.
- Use zero or one blank line between related items and statements; avoid repeated
  blank lines.
- Use block indentation rather than visual alignment:

  ```rust
  function(
    first,
    second,
  );
  ```

- Put a trailing comma on multiline comma-separated lists. Do not add one to an
  ordinary single-line list.
- Let rustfmt decide when a small struct literal, variant, closure, or call stays
  on one line.

## 2. Imports and items

- Let stable rustfmt order imports and module declarations according to the
  repository configuration.
- Keep semantically distinct import groups distinct. Do not reorganize imports
  in an untouched module merely for preference.
- Avoid `extern crate`; preserve `#[macro_use]` placement if legacy code still
  requires it because moving it can change semantics.
- Prefer one module per owned concern. A new module must contain real behavior,
  not only a declaration or placeholder comment.
- Keep callers above private helpers when it makes the file read top-to-bottom.
- Keep related types, implementations, errors, and tests vertically close.

## 3. Comments and rustdoc

- Prefer `//` for ordinary comments, `///` for item documentation, and `//!` for
  module or crate documentation.
- Put a single space after the comment marker. Write complete sentences with
  terminal punctuation when the comment is prose.
- Keep comments on their own line where practical. Do not place comments on
  closing-brace lines or inside function signatures.
- Doc comments precede attributes.
- Explain intent, invariant, external behavior, ordering, safety consequence, or
  non-obvious coupling. Do not narrate syntax or restate a name.
- Name the other file, migration, validation, or generated artifact when a
  cross-file contract must change in lockstep.
- Prefer types, names, structure, tests, and owning documentation over comments.
- Do not leave commented-out code. Use a tracked issue or owning document for
  deferred work rather than vague TODO prose.

## 4. Attributes, types, and declarations

- Put one attribute per line and exactly one `#[derive(...)]` per item.
- Preserve derive ordering unless the task has a semantic reason to change it.
- Use `UpperCamelCase` for types and variants, `snake_case` for functions,
  methods, modules, fields, and locals, and `SCREAMING_SNAKE_CASE` for constants
  and immutable statics.
- Prefer dedicated types for validated identifiers, bounded values, units, and
  closed vocabularies.
- Keep generic parameters purposeful. Prefer a concrete type or trait object
  when a generic adds no useful compile-time relationship.
- Prefer `where` clauses when several bounds would make a declaration hard to
  scan.
- Keep public signatures explicit. Do not expose concrete infrastructure types
  through domain or application APIs.

## 5. Functions and expressions

- Keep functions focused, with descriptive names, few arguments, and visible
  side effects.
- Avoid boolean flag arguments that select unrelated behavior; use distinct
  functions or a typed mode when the distinction is real.
- Declare values close to use and give boundary conditions explanatory names.
- Prefer expression-oriented Rust when it improves clarity, not when it hides
  control flow.
- Use early returns for invalid boundary conditions when they keep the main path
  readable.
- Do not use `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, or
  stdout/stderr printing in production library paths. Respect the configured
  lint levels and return typed errors.
- Never discard a `Result`, task failure, process status, or transaction error.
- Keep blocking parsing and file work off async executor threads. Do not hold
  unrelated locks or database connections across waits.
- Bound spawned tasks, channels, batches, retries, and concurrency.

## 6. Errors and boundary mapping

- Define errors at the layer that owns the failure vocabulary.
- Use one variant for each distinction callers or tests must observe.
- Retain source errors for diagnostics without exposing secrets or database
  implementation through upper layers.
- Map source errors to application errors, application errors to HTTP/CLI
  responses, and database rows to domain/application types explicitly.
- Never silently substitute a default, skip a malformed record, canonicalize an
  uncertain value, or fall back to legacy behavior.
- Keep error and audit messages bounded and remove control characters before
  persistence or terminal output.

## 7. Tests

- Keep tests readable, deterministic, independent, and focused on observable
  behavior.
- Arrange inputs, perform one action, and assert the contract without
  reimplementing production logic in the expected value.
- Assert exact typed errors and stable issue codes where they are contractual.
- Include malformed and boundary cases, not only the success path.
- Pure logic tests must not require Tokio or PostgreSQL.
- Application tests use port fakes. Database behavior uses isolated PostgreSQL.
- Test fixtures remain small, synthetic, non-sensitive, and checked in only when
  their provenance is clear.
- Do not add sleeps for synchronization. Use readiness checks, deadlines, or
  deterministic coordination.
- A passing placeholder assertion is not a test.

## 8. Cargo.toml

- Follow the existing manifest layout: `[package]`, workspace-derived package
  fields, `[lints]`, dependencies, then development dependencies.
- Use `workspace = true` for dependencies managed at the root.
- Keep internal dependency direction consistent with this file.
- Keep package publishing disabled.
- Do not add wildcard dependencies, unapproved registries, or Git dependencies
  without explicit review.
- No Taplo configuration exists in the current repository. Do not claim a Taplo
  check passed unless one is deliberately added and run.
- After manifest edits run `cargo metadata --no-deps`, the targeted package
  check, and then the workspace check.

## 9. Pre-completion checklist

- Dependency direction and layer ownership remain correct.
- The change contains real behavior, not another scaffold.
- Names, types, errors, and tests make the contract discoverable.
- Inputs, resource use, async work, and diagnostics are bounded.
- Units, provenance, and curation status remain explicit.
- Secrets and raw sensitive data cannot appear in logs or errors.
- No unrelated file was reformatted or renamed.
- Targeted tests pass.
- Cross-crate or Docker-backed tests pass when the behavior requires them.
- `cargo fmt --all -- --check` passes.
- Relevant Clippy and workspace checks pass.
- Any missing tool, database, Docker daemon, or unrun verification is reported
  precisely.
