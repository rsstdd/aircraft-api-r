# Aircraft Management Engine

A high-throughput, production-grade asynchronous Rust backend platform built using Hexagonal (Clean) Architecture primitives. This engine enforces domain isolation, compile-time SQL validation, structured tracing diagnostics, and strict input boundary validation.

## Architectural Architecture & Ecosystem

```text
root/
├── Cargo.toml                  # Workspace dependencies & compiler lint matrix
├── Justfile                    # Human-facing local automation task runner
├── .sqlx/                      # Offline database query schema snapshots
├── apps/
│   ├── ingest/                 # PlanePHD ingestion CLI composition root
│   └── server/                 # HTTP composition root & application entry-point
├── crates/
│   ├── aircraft_api/           # HTTP Transport Layer (Axum & DTOs)
│   ├── aircraft_app/           # Use-case coordination & driving application services
│   ├── aircraft_domain/        # Pure pure business entities & domain invariants
│   ├── aircraft_db/            # Data persistence layer (SQLx repositories)
│   ├── aircraft_ingest/        # Non-blocking batch ingestion & data normalization
│   ├── aircraft_config/        # Strongly-typed environment configuration trees
│   └── aircraft_observability/ # Telemetry bootstrapping (JSON traces & OTLP)
├── database/                   # Schema evolution tracking, validations, & seeds
└── xtask/                      # Rust-powered administrative lifecycle automation
```

### Dependency Decoupling Strategy

To guarantee architectural safety, structural data boundaries are strictly mapped across separate layers:

* **DTO Layer (`aircraft_api`):** JSON payloads, HTTP status mapping, and semantic boundary inputs (`validator`).
* **Application Layer (`aircraft_app`):** Workflow transaction handling and domain trait orchestration.
* **Domain Layer (`aircraft_domain`):** Structurally pure entities, rules, and mathematical invariants. **Zero infrastructure dependencies.**
* **Persistence Layer (`aircraft_db`):** Strongly-typed row transformations mapping explicitly to PostgreSQL tables.

---

## Core Prerequisites

* **Rust Toolchain:** Version `1.85+` utilizing the **Rust 2024 Edition** compiler profiles.
* **Docker Daemon:** Required for the disposable local PostgreSQL workflows.
* **Just Utility:** Command runner for workspace workflow mechanics (`cargo binstall just`).

---

## Local Development Workflow

### 1. Initialization and Compilation

Bring up development containers, execute canonical schema evolutions, run semantic data validations, and compile the workspace tree:

```bash
just db-bootstrap
just build
```

`just db-bootstrap` is safe to rerun. For local volumes created before the
migration ledger was introduced, it verifies and adopts only a complete legacy
Phase 1/2 prefix before applying the remaining migrations. A partial legacy
schema is rejected with recovery guidance instead of being marked as migrated.
See `database/local_setup_and_testing.md` for the complete workflow.

### 2. Execution of Test Suites

Run the enabled workspace test suite:

```bash
just test
```

`just test` currently runs `cargo nextest run --workspace`; it does not run the
database validation scripts. Use `just db-validate` separately for the
container-backed database verification suite.

### 3. Schema Management and Offline Compilation Verification

After modifying SQL query profiles inside `aircraft_db`, verify the existing
offline metadata when that crate and its metadata are enabled:

```bash
# Execute local schema evolution steps
just db-migrate

# Ensure compliance with strict offline compilation assertions
just check-offline
```


## Production Database Workflow

Provision the target database and migration role through the deployment
platform, set `MIGRATION_DATABASE_URL`, then run:

```bash
just db-prod-bootstrap
```

This uses the host `psql` client, installs all migrations and canonical
seeds, and runs database validation. See `database/README.md` for individual
commands, required privileges, and database administration guidance.

## Aircraft Data Ingestion

The candidate ingestion path is the Rust `aircraft-ingest` CLI. It is not yet
approved as the production path; the deployment gates in the architecture
document must pass first. PlanePHD JSON is currently the supported source
format. Input may be a local file or `-` for standard input.

Validate an artifact without connecting to PostgreSQL:

```bash
just ingest-validate tests/fixtures/planephd_minimal.json
```

Import it atomically using the dedicated ingestion database user:

```bash
export APP__INGEST__DATABASE_URL='postgresql://...?...sslmode=verify-full'
just ingest-import tests/fixtures/planephd_minimal.json
just ingest-status --limit 20
```

The import preserves raw records, provenance, assertions, and curation flags.
A hard validation or persistence failure rolls back all aircraft-data changes;
reimporting the same source/hash/parser identity is idempotent. Database
credentials are supplied only through configuration, never CLI arguments.

Before importing into Aiven, apply the canonical migrations with the migration
owner and have an administrator apply
`database/roles/ingest_grants.sql` to a dedicated ingestion role. The complete
architecture, command surface, configuration, transaction semantics, Aiven role
boundary, and retirement criteria for the legacy SQL loader are documented in
[`docs/architecture/rust_ingestion_adapter.md`](docs/architecture/rust_ingestion_adapter.md).

---

## Continuous Integration & Security Guardrails

The workspace enforces high-severity lint walls and secure compilation boundaries:

* **Unsafe Execution:** Forbidden explicitly (`#[forbid(unsafe_code)]`).
* **Memory Hygiene:** Private authorization components and access configurations are sealed within `secrecy::SecretString` enclosures to prevent leakage during format logging operations.
* **CI Gates:** Every code check evaluates via `cargo fmt` -> `cargo clippy` -> `cargo audit` -> `cargo deny` -> `cargo nextest`.

---

## 2. Application Binaries: `apps/server/README.md`

# Application Entry-Point: `server`

The central composition root for the entire platform workspace. This package coordinates structural bootstrapping lifecycles, parses infrastructure runtime configuration layers, initializes long-lived asynchronous database connection pools, and configures the system runtime context.

## Bootstrap Sequence Execution Flow

```text
[Load Configurations] -> [Bootstrap Diagnostics] -> [Initialize DB Pools] -> [Bind Transports]
```

1. **Environment Setup:** Ingests local execution variables safely via local-only `.env` fallbacks.
2. **Configuration Ingestion:** Evaluates strongly-typed settings objects mapping file states into memory.
3. **Observability Ingestion:** Initializes the global `tracing-subscriber` layer to capture JSON output strings.
4. **Resource Allocation:** Allocates the async `sqlx::PgPool` driver instance based on security scopes.
5. **Transport Routing:** composes sub-module HTTP routing tables from `aircraft_api`.
6. **Signal Capture:** Spawns asynchronous OS signal handlers (`SIGINT`, `SIGTERM`) to trigger clean shutdown cascades across processing tasks.

## Local Execution Primitive

To spin up the service entry-point manually in a local development context:

```bash
cargo run --package server
```

## 3. Boundary Transport: `crates/aircraft_api/README.md`

# Interface Transport Crate: `aircraft_api`

The inbound perimeter interface. This crate maps external network transport interactions safely into core execution mechanisms using `axum` routing graphs, standard HTTP request extractions, and uniform serialization schemas.

## Technical Implementations

### Perimeter Payload Hygiene (`ValidatedJson`)
To prevent structural ingestion errors from polluting internal coordination layers, this component overrides framework fallbacks using a unified verification extractor wrapper:

```rust
pub struct ValidatedJson<T>(pub T);
```

This ensures that incoming requests conform to target structural limits, verify schema constraints via `validator`, and output standardized RFC 7807 problem footprints back to calling consumers upon processing failures.

### Target Routing Architecture Discovery

API documentation runs out-of-band via inline macro scanning tools (`utoipa`), mapping active paths directly to local discovery endpoints:

* **Interactive Engine Workspace:** `/docs`
* **Raw Schema Contract Specification:** `/api-docs/openapi.json`

## Layer Security Controls

Every route pipeline is wrapped by explicit resource bounds:

* **Payload Caps:** Global request body length restrictions using `tower-http` limit definitions.
* **Trace Context Chains:** Transparent correlation identification injection mapping `request_id` spans to output diagnostics.

---

## 4. Application Logic: `crates/aircraft_app/README.md`

# Core Coordination Layer: `aircraft_app`

The implementation home for application use-cases, system orchestrators, and driving business process components. This package defines execution boundaries and coordinates transactional workflows.

## Key Responsibilities

* **Process Coordination:** Sequencing discrete tasks required to fulfill high-level system functions.
* **Execution Isolation:** Decoupling the system core from presentation layers (`aircraft_api`) and data-access subsystems (`aircraft_db`).
* **Data Boundary Mapping (Ports):** Defining the abstract trait profiles required to interface with external storage engines:

```rust
pub namespace ports {
    // Abstract boundaries implemented downstream within persistence layers
    pub trait AircraftRepository: Send + Sync { ... }
}
```

## Fault Propagation Policy

Errors within this package use explicit domain enumerations built on top of `thiserror`. Low-level infrastructure exceptions (such as network drops or physical database drops) are caught, packaged into clean semantic failures, and passed upward to protect boundary consumers from internal data layer patterns.

---

## 5. System Core Invariants: `crates/aircraft_domain/README.md`

# Domain Invariant Library: `aircraft_domain`

The pure structural core of the platform architecture. This package isolates corporate rules, value object models, and invariant equations from the outside world.

> **Architectural Guardrail:** This crate maintains a strict **zero-dependency policy** regarding asynchronous runtimes (`tokio`), transport layers (`axum`), and data storage engines (`sqlx`). It remains completely deterministic, synchronous, and memory-safe.

## Pure Domain Implementations

* **Core Primitives:** Primitive representations for operational assets, operational bounds, and computational frameworks.
* **Algorithmic Computations:** Pure functions covering asset score metrics, configuration checks, and normalization logic.
* **Structural Integrity:** Value objects that leverage Rust's type architecture to make illegal system states unrepresentable.

## Permitted Additions
Small, pure utility tools are declared selectively when their usage is justified:
* `serde` (Serialization definitions)
* `uuid` / `chrono` (Standardized mathematical values)
* `thiserror` (Pure domain failure models)

---

## 6. Infrastructure Storage: `crates/aircraft_db/README.md`

# Persistence Subsystem: `aircraft_db`

The infrastructure data-access package. It implements application ports with
parameterized SQLx queries and keeps database rows and transactions out of the
domain and application layers.

## Implementation Blueprint

* **Explicit transaction ownership:** ingestion staging, promotion, provenance,
  curation, run completion, and read-model refresh commit or roll back together.
* **Parameterized SQL:** runtime-checked SQLx queries bind all source-controlled
  values; source JSON and names are never interpolated into SQL.
* **Separation of models:** database layout remains inside `aircraft_db` and does
  not leak into application inputs or domain values.

Migration behavior is verified against disposable PostgreSQL instances using the
canonical installer and SQL validation scripts. This ingestion slice does not use
SQLx compile-time query macros, so it does not require `.sqlx` metadata.

---

## 7. Stream Aggregations: `crates/aircraft_ingest/README.md`

# Processing & Integration Engine: `crates/aircraft_ingest`

This crate is the source boundary for PlanePHD JSON. It securely captures file or
standard-input bytes, computes artifact identity, parses the two-level source
shape, normalizes supported values, and emits source-independent prepared
records. It performs no SQL.

## Architectural Flow Matrix

```text
[file or stdin] -> [immutable artifact] -> [preflight] -> [prepared record stream]
```

## System Constraints

* **Two-pass validation:** the complete artifact passes structural and domain
  preflight before the database import transaction begins.
* **Atomic hard failures:** malformed structure or invariant errors reject the
  entire batch; valid siblings are not partially committed.
* **Explicit curation:** optional ambiguous values are preserved in raw JSON and
  emitted as stable warnings that become curation flags during import.
* **Idempotence:** exact input bytes, source identity, parser name, and parser
  version form the logical import identity.
* **Bounded record flow:** normalized records stream through a bounded channel;
  the full normalized dataset is never retained in memory.

See [`docs/architecture/rust_ingestion_adapter.md`](docs/architecture/rust_ingestion_adapter.md)
for the complete contract and Aiven deployment boundary.

---

## 8. Configuration Systems: `crates/aircraft_config/README.md`

# Hierarchical Settings Subsystem: `aircraft_config`

Strongly-typed runtime configuration architecture. This package manages environment settings and guards infrastructure access credentials.

## Operational Inheritance Layer

Settings are aggregated using a multi-tiered file inheritance tree:
1.  `default.hjson` — Global workspace fallbacks.
2.  `local.hjson` — Development overrides (Ignored by source control tools).
3.  `production.hjson` — Live cluster environmental bindings.
4.  `ENVIRONMENT VARIABLES` — Final overrides mapping directly to targeted deployment slots (e.g., `APP_DATABASE__URL`).

## Secret Hygiene Controls

```rust
#[derive(Debug, serde::Deserialize)]
pub struct DatabaseSettings {
    pub url: secrecy::SecretString, // Protected against accidental logging leaks
}
```

Credentials, tokens, and verification strings are encapsulated inside `secrecy` wrappers. This deactivates standard formatting evaluation vectors, blocking inadvertent leakage through system logs, error responses, or diagnostic trace streams.

---

## 9. Diagnostic Engines: `crates/aircraft_observability/README.md`

# Structured Diagnostics Engine: `aircraft_observability`

The unified operational telemetry subsystem. This component manages application instrumentation, trace collection, and structured diagnostic reporting.

## Features

* **Structured JSON Output:** Formats application logs into computer-scannable JSON strings for direct pipeline ingestions.
* **Contextual Tracing:** Captures execution scopes cleanly across asynchronous tasks, binding structural execution metadata (`request_id`, `db.operation`) uniformly to event contexts.
* **Distributed Exporters:** Implements hooks for distributed tracing collection engines using standard `opentelemetry` export protocols.

## Production Instrumentation Rules

Avoid utilizing generic text tracing macros (`println!`) across application layers. Ensure all execution logging passes through standard semantic level definitions (`event!`, `span!` macros):

```rust
tracing::info!(request_id = %id, path = %url.path(), "Processing inbound HTTP transaction");
```

---

## 10. Development Task Management: `xtask/`

The `xtask` binary provides behaviorally tested development automation through
root Just recipes:

* `just install-deps` checks the platform prerequisites (`cargo`, `rustup`,
  Docker Compose, and the PostgreSQL client) and installs missing Rust
  development tools used by this repository:
  rustfmt, Clippy, Just, cargo-nextest, cargo-audit, cargo-deny, and SQLx CLI.
  Use `just install-deps --check` to report missing tools without installing
  anything.
* `just deny` runs the tested `xtask deny` wrapper against the locked workspace.
  The root `deny.toml` enforces the approved license set, rejects wildcard
  dependencies and unapproved registry or Git sources, and checks RustSec
  advisories. `just lint` includes the same command.
* `just generate-docs` compiles the API-owned Utoipa contract and writes
  `docs/openapi.json`. Use `just generate-docs --check` to fail when the checked
  artifact is missing or stale, or `just generate-docs --output <path>` to
  select another local destination.

`prepare-sqlx` remains unavailable while `aircraft_db` is a scaffold without
compile-time checked SQLx queries. Add the command only after the persistence
crate is an active workspace member with real queries whose offline metadata can
be generated and verified.

The working Just recipe `just db-reset` is separate from these xtask commands.
It deletes the local Compose PostgreSQL volume and starts an empty database
container; it does not migrate, seed, or validate. Use the destructive
`just db-rebuild` recipe when an explicitly disposable local database should be
reset, migrated, seeded, and validated in one workflow.
