# Aircraft Management Engine

A high-throughput, production-grade asynchronous Rust backend platform built using Hexagonal (Clean) Architecture primitives. This engine enforces domain isolation, compile-time SQL validation, structured tracing diagnostics, and strict input boundary validation.

## Architectural Architecture & Ecosystem

```text
root/
├── Cargo.toml                  # Workspace dependencies & compiler lint matrix
├── Justfile                    # Human-facing local automation task runner
├── .sqlx/                      # Offline database query schema snapshots
├── apps/
│   └── server/                 # Composition root & application entry-point
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
* **Docker Daemon:** Required locally to support containerized database testing passes via `testcontainers`.
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
commands, required privileges, and server-side JSON ingestion.

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

The infrastructure data-access package. This library bridges domain layer requests to physical storage systems using asynchronous, compile-time verified SQL execution pipelines.

## Implementation Blueprint

* **Compile-Time Query Guarantees:** Database interactions execute through `sqlx` query macros, verifying SQL formatting patterns against target engine definitions during compilation blocks.
* **Separation of Models:** Database rows are represented via local target models, keeping table layout tracking decoupled from domain transformations.

## Integration Testing Profiles

Integration tracking hooks up to live engine lifecycles using `testcontainers`:

```rust
#[tokio::test]
async fn verify_asset_persistence_lifecycle() {
    let container = Postgres::default().start().await;
    // Executes isolated validation tasks against clean database states
}
```

To maintain continuous integration performance and permit offline verification, query meta-footprints must be snapshotted to disk prior to deployment pushes:

```bash
cargo sqlx prepare --workspace
```

---

## 7. Stream Aggregations: `crates/aircraft_ingest/README.md`

# Processing & Integration Engine: `crates/aircraft_ingest`

The asynchronous background processing engine. This crate handles high-volume ingestion flows, unpacks incoming payload streams, validates formats, and coordinates structural normalization passes.

## Architectural Flow Matrix

```text
[External Data Pipeline] -> [Raw Schema Parsing] -> [Structural Invariant Processing] -> [Persistence Pipeline]
```

## System Constraints

* **Memory Efficiency:** Processing streams manage large payloads incrementally using memory-bounded iteration blocks.
* **Idempotence Protections:** Processing loops assert operational run states at the storage perimeter to prevent redundant evaluations of matching datasets.
* **Fault Tolerance:** Processing tasks isolate structural format failures, allowing valid sub-records to process cleanly while logging parsing faults for audit inspection.

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

### Scaffold status

The `xtask` binary currently compiles, but its execution functions are empty.
It is not exposed through working Just recipes. Implement and behaviorally test
each operation before treating it as available automation.

### Planned command routines

These are design targets, not implemented commands. Database lifecycle work is
provided by the verified `db-*` and `db-prod-*` Just recipes above:

* `install-deps` — Checks local environments and provisions missing development tools.
* `prepare-sqlx` — Executes schema verification routines and updates the offline metadata file tree (`.sqlx/`).
* `generate-docs` — Compiles core application API endpoints and outputs OpenAPI schema artifacts directly onto local disk targets.

The working Just recipe `just db-reset` is separate from these planned xtask
commands. It deletes the local Compose PostgreSQL volume and starts an empty
database container; it does not migrate, seed, or validate. Use the destructive
`just db-rebuild` recipe when an explicitly disposable local database should be
reset, migrated, seeded, and validated in one workflow.
