---
name: clean-code
description: Clean Code (Robert C. Martin) rules as applied in this repository, with the conflict order against binding repo conventions and the settled conflicts that must not be re-litigated. REQUIRED before writing, generating, editing, or reviewing any code here, and whenever a Clean Code rule appears to conflict with a repo convention.
---

# Clean Code in this repository

These rules govern all code written or edited here. They are style and structure guidance,
not authority: **`AGENTS.md`, `crates/AGENTS.md`, and the canonical SQL under
`database/migrations/` win on any genuine conflict.** Where they conflict, apply Clean Code
inside the repo convention's frame and say so in the summary — never resolve the conflict
silently.

Conflict order is the one `AGENTS.md` already publishes under *Sources of truth*: nearest
`AGENTS.md` → manifests and tests → `database/migrations/` → `justfile` → the owning document
(`database/README.md`, `database/implementation_notes.md`,
`docs/architecture/rust_ingestion_adapter.md`) → `README.md` → these rules. **There is no
accepted ADR catalog in this tree**; do not cite one, and do not let a proposed decision
authorize scope.

`ponytail` decides whether the code should exist at all; `rust-comment` owns comment content;
`rust-testing` owns Rust test policy; `rust-production` owns the OS- and database-facing shapes.

## Known conflicts, already settled

Do not re-litigate these. Flag anything new.

| Clean Code rule | Repo convention that wins | Why |
|---|---|---|
| One assert per test | One *behavior* per test, several assertions allowed | `crates/AGENTS.md`: "One test should establish one behavior. Multiple assertions are appropriate when together they prove that behavior." Put the failing case in the assertion message. |
| Prefer polymorphism to if/else | Exhaustive `enum` + `match` for closed domain vocabularies | `crates/AGENTS.md` requires closed vocabularies and invalid states to be dedicated types. A `match` refuses an unknown variant at compile time; a trait object cannot. Traits stay for real seams (`IngestionStore`, `PreparedRecordReader`, `SourceAdapter`), not closed rule sets. |
| DRY: one representation of a concept | Deliberately separate representations | `AGENTS.md`: source JSON, prepared application records, domain values, SQL rows, HTTP DTOs, and generated OpenAPI schemas "must not collapse into one shared convenience type" — even when their shapes coincide today. The duplication is the boundary. Explicit mappings, not a shared struct. |
| Avoid comments; explain in code | Two-sided coupling comments to migrations, validation SQL, and generated artifacts | A mirror between Rust and canonical SQL cannot be expressed in Rust. Both ends must name each other. See `aircraft_testsupport::SCHEMA_STEPS` ↔ `database/install.sql`. |
| Prefer polymorphism / non-static methods / no base class knows its derivatives | Free functions, associated functions, and enums | Rust has no inheritance. Read the OO rules as "one thing owns one behavior", not as a demand for a type hierarchy. |

## General

- Follow standard conventions. Keep it simple: simpler is always better.
- Boy scout rule: leave the code cleaner than you found it — within the change's scope. Do not
  reformat, rename, or reorganize unrelated code (`AGENTS.md` operating rules).
- Always find the root cause. No workaround without saying why and how it is bounded.

## Design

- Keep configurable data at high levels. Prevent over-configurability.
- Inject what a unit depends on; follow the Law of Demeter.
- Keep the dependency direction inward: adapter → application → domain. A cycle or an upward
  import is a design failure, not a style nit — `cargo run -p xtask -- boundaries` enforces it.
- Separate concurrency from the logic it runs. Bound every queue, retry, and pool.
- Prefer polymorphism to if/else or switch — subject to the settled conflicts above.

## Understandability

- Be consistent: one idea, one spelling, one shape.
- Use explanatory variables. Encapsulate boundary conditions.
- Prefer dedicated value objects to bare primitives — a validated `Confidence`, not a bare `f32`
  range-checked ad hoc at each call site, and not a unit code compared by string equality.
  Anything compared by string equality is a rule that stops being enforced.
- Preserve explicit units and measurement conditions. A number without its unit is not a value.
- Avoid logical dependency between methods. Avoid negative conditionals.

## Names

- Descriptive, unambiguous, pronounceable, searchable, meaningfully distinct.
- Match canonical schema vocabulary. A Rust name for a column, status, or issue code should be
  the schema's word for it.
- Named constants over magic numbers (`DEFAULT_MAX_INPUT_BYTES`, not `536870912`).
- No encodings, no type prefixes, no Hungarian notation.

## Functions

- Small. Do one thing. Descriptive name. Few arguments. No side effects. No flag arguments —
  a boolean parameter that selects behavior means two functions.

## Types and data

- Hide internal structure. Prefer plain data where there is no behavior; avoid hybrids.
- Small, doing one thing, with few fields.
- Prefer many functions to one function taking a parameter that selects behavior.

## Comments

- Explain yourself in code first.
- Comments carry intent, clarification, consequence, or warning — never a restatement of the
  code, never a closing-brace label, never commented-out code.
- In this repo a comment is also the right tool for: why an ordering is load-bearing, why a
  deviation is proportionate, and what a Rust↔SQL mirror is bound to.

## Source structure

- Separate concepts vertically; keep related code vertically dense.
- Declare variables close to use. Keep dependent and similar functions close.
- Functions read downward: callers above callees.
- No horizontal alignment. Use whitespace to associate, not to decorate. Do not break
  indentation. Widths belong to `.rustfmt.toml` (100 columns, two-space indentation).

## Tests

- Readable, fast, independent, repeatable. One behavior per test.
- Name the observable outcome in `snake_case`, following the current crate idiom. **Do not
  invent milestone or tier prefixes the repository does not use** (`crates/AGENTS.md`).
- **A test must not re-derive the implementation.** Expected values are a table a reviewer reads
  against the migration or the owning document, not a second copy of the code under test — a
  copied `matches!` passes for any policy, including a wrong one.
- Prefer an exhaustive `match` in the expectation table so a new enum variant is a compile error
  in the test rather than an untested one.
- A green run is not coverage. `AGENTS.md` records that two properties in this repository passed
  vacuously until they were mutation-checked: mutate the code and confirm the test fails.
- Pure logic tests must not require Tokio or PostgreSQL.

## Smells to refuse

Rigidity, fragility, immobility, needless complexity, needless repetition, opacity — plus the
unrequested abstractions, boilerplate, and cleverness that `ponytail` refuses.

In this repo, "opacity" specifically includes returning a catch-all error variant where the
failing subsystem is known and could be named, and silently substituting a default, skipping a
malformed record, or canonicalizing an uncertain value instead of preserving it as evidence.
