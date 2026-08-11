# AGENTS.md

## Repository

Aircraft Management Engine is a Rust 2024 and PostgreSQL backend for storing,
curating, searching, and comparing aircraft data. The target design is a Cargo
workspace using hexagonal architecture, with Axum at the HTTP boundary, SQLx at
the persistence boundary, and SQL-first database migrations.

These instructions apply to the entire repository. A more specific `AGENTS.md`
inside a subdirectory takes precedence for work in that directory.

## Current branch status

The `chore/restructure-project-layout` branch is an incomplete restructuring
scaffold. Treat its files as evidence of the current state, not as a finished
implementation of every claim in `README.md`.

- The root `Cargo.toml` currently enables only `xtask`; the application and
  `aircraft_*` workspace members are commented out.
- `apps/server/src/` still contains legacy Actix/Diesel code and imports old
  crate names, while its manifest declares the intended Axum/SQLx-era crates.
- Many modules in `crates/aircraft_*` contain only declarations or placeholder
  comments.
- The three files under `tests/` are placeholder tests that only assert `true`.
- `xtask` exposes intended commands, but its execution functions are not yet
  implemented.
- `README.md` describes the target architecture. Confirm every feature and
  command against manifests, source files, and the `justfile` before relying on
  it.
- `archive/` contains migration snapshots and retired template-derived code. It
  is reference material, not active application code.

Do not describe a scaffolded module, test, command, or architecture boundary as
implemented until the corresponding code works and has meaningful verification.

## Working rules

- Read the relevant manifest, implementation, tests, and database documentation
  before editing.
- Make the smallest coherent change that advances the requested outcome.
- Preserve unrelated changes. Do not combine restructuring with incidental
  formatting, renaming, or cleanup.
- Follow the target crate boundaries below. Do not restore retired template
  dependencies merely to make legacy code compile.
- Prefer completing one vertical slice over adding more empty modules, stubs, or
  placeholder tests.
- Replace placeholder tests with behavioral assertions as functionality becomes
  real. Never count `assert!(true)` as coverage.
- Keep boundary mappings explicit: HTTP DTOs, domain types, and database rows are
  separate representations.
- Document non-obvious invariants and cross-crate coupling. Do not add comments
  that merely restate names or type signatures.
- Do not claim that a check passed unless it was run successfully.
- If the branch cannot yet pass a check, report the exact failure and distinguish
  pre-existing scaffold failures from regressions introduced by the task.

## Autonomy and approval

For requests to explain, review, diagnose, or plan, inspect the repository and
report the result without modifying it.

For requests to build, change, or fix, make the requested local changes and run
relevant non-destructive verification without pausing for routine approval.

Ask before:

- deleting data or Docker volumes, including `just db-reset` and
  `just db-rebuild`;
- running migrations, seeds, or validation against a database that is not clearly
  an isolated local instance;
- discarding uncommitted work or rewriting Git history;
- committing, pushing, merging, opening a pull request, deploying, or publishing;
- modifying generated database artifacts without the command that regenerates
  them;
- expanding the task into a broad schema or architecture redesign.

## Architectural invariants

The intended dependency direction is inward toward the domain:

```text
HTTP/API adapter ──> application use cases ──> domain
database adapter ──> application ports / domain
ingest adapter ────> application ports / domain
server ────────────> composes adapters and runtime infrastructure
```

- **`aircraft_domain` is the pure core.** Keep HTTP frameworks, SQLx, Tokio,
  configuration loading, and telemetry out of it. Domain behavior must remain
  deterministic and testable without external services.
- **`aircraft_app` owns use cases and ports.** It coordinates domain behavior but
  must not depend on HTTP DTOs or concrete database implementations.
- **`aircraft_api` owns transport concerns.** Axum handlers, request validation,
  response DTOs, status mapping, middleware, and OpenAPI definitions belong here.
  It must not issue SQL directly.
- **`aircraft_db` owns persistence.** SQLx repositories and explicit conversions
  between database rows and domain types belong here. Database layout must not
  leak into the domain model.
- **`aircraft_ingest` owns import boundaries.** Parse, validate, normalize, and
  report source-data failures here; use application or persistence ports rather
  than bypassing provenance and canonicalization rules.
- **`aircraft_config` owns typed configuration.** Environment variables use the
  `APP` prefix with `__` for nesting. Keep secrets out of debug output.
- **`aircraft_observability` owns telemetry setup.** Use `tracing`; do not add
  `println!` or `dbg!` to application paths.
- **`apps/server` is the composition root.** Wire configuration, telemetry,
  database pools, routes, and shutdown there. Business rules do not belong in
  `main.rs`.
- **Unsafe Rust is forbidden.** The workspace lint matrix sets
  `unsafe_code = "forbid"`.
- **Production binaries are not published as crates.** Workspace packages remain
  `publish = false` unless release policy changes explicitly.

## Database invariants

- `database/migrations/` is the canonical schema history. The project currently
  uses SQL-first migrations; do not switch to Diesel migrations unless the SQL
  files are deliberately converted to Diesel's directory format.
- Preserve numeric migration order and foreign-key dependencies.
- `database/seeds/001_reference_units.sql` must run before
  `database/seeds/002_lookup_seed_data.sql` because later lookup data references
  measurement units.
- Keep canonical seed data in `database/seeds/`, transient ingestion work in
  `database/staging/`, test-only rows in `database/fixtures/`, and post-install
  assertions in `database/validation/`.
- Do not edit schema behavior based only on `database/data_dictionary.md`; update
  the SQL first, verify it, then update the dictionary and implementation notes.
- Treat provenance, source assertions, normalization, and curation status as
  first-class data. Do not silently promote uncertain source values into
  canonical aircraft records.
- Preserve explicit units and conditions for physical and performance values.
  Do not compare or aggregate values whose units or measurement conditions are
  ambiguous.
- Database documentation and automation currently disagree about some Phase 17+
  paths. Inspect the actual files before execution; do not run `database/install.sql`
  blindly.

## Repository map

| Path | Responsibility | Status |
|---|---|---|
| `Cargo.toml` | Workspace membership, shared dependencies, lints, profiles | Only `xtask` is currently enabled |
| `apps/server/` | Runtime composition and server binary | Legacy source does not match target manifest |
| `crates/aircraft_domain/` | Entities, value objects, units, domain validation | Mostly scaffolded |
| `crates/aircraft_app/` | Use cases, services, and ports | Mostly scaffolded |
| `crates/aircraft_api/` | Axum routes, DTOs, middleware, OpenAPI | Mostly scaffolded; some legacy files remain |
| `crates/aircraft_db/` | SQLx models, repositories, and schema mappings | Mostly scaffolded |
| `crates/aircraft_ingest/` | JSON import, normalization, and ingest validation | Scaffolded |
| `crates/aircraft_config/` | Typed file and environment configuration | Partial implementation |
| `crates/aircraft_observability/` | Structured tracing and telemetry | Partial implementation |
| `database/migrations/` | Ordered canonical schema changes | Active source of truth |
| `database/seeds/` | Canonical reference and mission-profile data | Active source of truth |
| `database/staging/` | Transient JSON ingestion pipeline | Active, but automation paths need reconciliation |
| `database/validation/` | Post-migration and data-integrity checks | Active SQL verification |
| `database/fixtures/` | Test-only database data | Test scope only |
| `xtask/` | Typed repository automation | Commands exist; logic is stubbed |
| `archive/` | Restructure snapshots and retired template crates | Read-only reference; do not compile or extend |

## Routing table

| When looking for | Start here | Authority |
|---|---|---|
| Current workspace membership and lints | `Cargo.toml` | Manifest, not README prose |
| Available local commands | `justfile` | Recipe must exist and be implemented |
| Target architecture | `README.md` plus crate manifests | Verify against source before claiming completion |
| Server composition | `apps/server/` | Target composition root |
| Domain vocabulary and behavior | `crates/aircraft_domain/` | Domain code once implemented |
| HTTP contracts | `crates/aircraft_api/` | DTOs, handlers, and generated OpenAPI once implemented |
| Persistence implementation | `crates/aircraft_db/` | Repository code and migration SQL |
| Schema definitions | `database/migrations/` | Canonical DDL |
| Schema meaning and namespace map | `database/data_dictionary.md` | Documentation; reconcile with DDL |
| Migration dependencies and limitations | `database/implementation_notes.md` | Guidance; reconcile paths with the tree |
| Database lifecycle policy | `database/README.md` | Directory ownership and SQL-first policy |
| Runtime configuration | `config/`, `.env.example`, `aircraft_config` | Never commit `.env` |
| Historical pre-restructure code | `archive/` | Reference only |

## Commands and verification

Rust 1.85 or newer is required. The workspace uses Rust 2024 edition. The
`justfile` loads `.env` and currently requires it, so copy `.env.example` to a
local ignored `.env` before using recipes that need environment variables.

Use the `justfile` as the command interface, but confirm that the named recipe
exists. `README.md` currently mentions `just bootstrap` and `just prepare-sqlx`,
which are not active recipes.

```bash
# Fast structural checks
cargo metadata --no-deps
cargo fmt --all -- --check
just check

# Tests and quality gates
just test
just lint

# SQLx offline compilation, once metadata and workspace crates are complete
just check-offline

# Inspect local infrastructure without changing it
just compose-config
just compose-services

# Isolated local database lifecycle
just db-up
just db-wait
just db-migrate
just db-seed
just db-validate
```

Use `just fmt` only when formatting changes are authorized. `just lint` also
requires the external `cargo-audit` and `cargo-deny` tools; report a missing tool
as an environment limitation rather than a successful lint result.

`just db-reset` and `just db-rebuild` delete the local PostgreSQL volume. They
require explicit approval.

During the restructure, verification is incremental:

1. Run `cargo metadata --no-deps` after manifest changes.
2. Enable or repair one workspace crate at a time.
3. Run formatting and a targeted package check before the whole workspace.
4. Replace placeholder tests with meaningful tests for the repaired behavior.
5. Run the full workspace checks only after all enabled members are coherent.

Use these test boundaries:

- Domain rules: pure unit and property tests without Tokio or PostgreSQL.
- Application services: use port fakes or mocks; test orchestration and errors.
- API routes: request validation, status mapping, response contracts, and limits.
- Database repositories: PostgreSQL integration tests in isolated containers.
- Migrations: install into a clean database, then run every relevant validation
  SQL file.
- User-visible API flows: exercise the running server end to end once the server
  is bootable.

## Security and data

- Never commit `.env`, credentials, tokens, private aircraft datasets, or database
  dumps.
- Values in `.env.example` and `docker-compose.yml` are local-development
  defaults, not production credentials.
- Validate external input in the API or ingestion boundary before it reaches
  application and domain code.
- Enforce authentication and authorization before protected operations. Do not
  infer security from the legacy route code.
- Do not log secrets, authorization headers, raw personal data, or full sensitive
  source payloads.
- Keep request-size limits, timeouts, rate limits, and structured tracing at the
  HTTP boundary.
- Do not weaken lint, validation, migration, or authorization controls merely to
  make the scaffold compile.

## Completion criteria

Before reporting a task complete:

- Confirm that the requested behavior is implemented rather than scaffolded.
- Review the final diff for unrelated changes and accidentally revived legacy
  code.
- Run the narrowest relevant checks, then broader checks where the branch permits.
- State all commands run and their results.
- Identify pre-existing branch failures separately from failures caused by the
  change.
- Confirm that no placeholder assertions, empty implementations, temporary logs,
  credentials, or copied template concepts remain in the completed path.
- Update `README.md`, the data dictionary, or implementation notes only when the
  corresponding implementation or database behavior changed.
