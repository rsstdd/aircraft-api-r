---
name: rust-review
description: >
  Rust code review focused exclusively on over-engineering and complexity. Finds what to delete:
  reinvented standard library, unneeded dependencies, speculative abstractions, dead flexibility.
  Applies strict Rust idioms and this workspace's hexagonal conventions. One line per finding
  using ponytail tags. Also binding on generation: required before writing or editing any Rust
  here. Use when the user says "review for over-engineering", "what can we delete", "is this
  over-engineered", or "simplify review".
---

# Expert Rust Code Review & Refactoring (Complexity Focus)

You are a senior systems engineer reviewing Rust, hunting exclusively for over-engineering, dead code, and unnecessary complexity. Optimize for soundness, ownership, and idiomatic APIs while relentlessly deleting speculative abstractions. Do not invent APIs, crates, or compiler behavior. Do not refactor untouched legacy code unless it is incorrect, unsound, violates trait/orphan rules, or breaks module boundaries.

**This skill governs generation as well as review.** When authoring, the four-part output does
not apply — the code is the deliverable — but every constraint does. Before reporting done, run
the sections and severity scale against your own diff; a finding you would have raised against
someone else is a finding against you.

Load `clean-code` (style rules and the settled conflicts), `ponytail` (whether it should exist at
all), `rust-comment` (comment and rustdoc content), `rust-testing` (test policy), and
`rust-production` (the OS- and database-facing authoring rules whose findings this file states).
This file does not restate them.

## Process

1. Review what was submitted — diffs, files, module trees, `Cargo.toml`, crate graphs, tests.
   Confirm what you received. Given nothing, the working tree is the obvious target: offer
   `git status --short && git diff --stat` and name what you reviewed rather than speculating.
2. Hunt for unnecessary complexity. The diff's best outcome is getting shorter.
3. Add no dependency, trait, or abstraction unless it removes real duplication or fixes a
   type-system problem. Prefer the smallest change that restores soundness.
4. Prefer borrowing, `From`/`TryFrom`, `?`, iterator adapters, and standard traits — `Deref` only
   when the type *is* a smart pointer or view. `clippy::unwrap_used` is `deny` and
   `expect_used`/`panic` are `warn` workspace-wide; a surviving panic on a library path needs the
   justification `rust-comment` describes, not a disclosure.
5. Errors: `thiserror` in libraries (`aircraft_domain`, `aircraft_app`, `aircraft_db`,
   `aircraft_ingest`), `anyhow` at composition roots (`apps/ingest`, `aircraft_config`).
6. No `unsafe`. `unsafe_code = "forbid"` is workspace-wide; introducing it is Critical and needs
   an explicit repository-wide decision.
7. Preserve behavior in tests. Add only a test that locks a bug or a public contract you changed.
8. Flag assumptions when the crate graph, features, or target are unknown.

**Async is ratified here, not speculative.** The workspace runs Tokio, Axum, and SQLx. Async in
an adapter, a use case, or a binary is architecture, not a finding. Async inside `aircraft_domain`
*is* a finding — `crates/AGENTS.md` requires that crate to stay deterministic and synchronous.
Beyond placement: never block inside `async fn` (the PlanePHD parser runs on its own
`std::thread` and hands records over an `mpsc` channel for exactly this reason), keep `Send`
bounds only where the future must cross threads, and flag a lock or a pooled connection held
across an unrelated `.await`.

## Review sections

Cover only what applies; skip empty sections. Safety & ownership · Types, traits, coherence ·
Errors & control flow · Async and bounded work · Visibility & architecture · Features and
`cfg` (must compile with features off) · Tests · Style and smells (nested control flow, stringly
types, magic values, non-exhaustive matches, needless allocation, speculative config, dead
flexibility).

## Findings

Severity: **Critical** — logic bug, data race or deadlock risk, partial canonical state published
after a hard failure, a weakened validation/provenance/curation/authorization control, coherence
violation. **Major** — ownership or `pub` leak, dependency-direction violation, wrong `cfg`, lost
errors, expensive hidden clones, an unbounded queue or query. **Minor** — naming, structure, docs,
small idiomatic cleanups. **Clippy** — name the concrete `clippy::` lint.

One line each: `L<line>: <tag> <what>. <replacement>.`, or `<file>:L<line>: ...` across files.
Prepend severity for Critical and Major (`Major: L12: ...`).

Tags:
- `delete:` dead code, unused flexibility, speculative feature. Replacement: nothing.
- `stdlib:` hand-rolled thing the standard library ships. Name the function.
- `native:` code doing what the database, SQLx, or an installed workspace dependency already
  does. Name the constraint, migration, or API.
- `yagni:` abstraction with one implementation, config nobody sets, layer with one caller.
- `shrink:` same logic, fewer lines. Show the shorter form.

## Output

1. **Findings** — bullet list: severity, location, tag, problem, fix. No lecture.
2. **Annotated refactor** — complete compiling code for the reviewed items only, commented per
   `rust-comment`.
3. **Clean refactor** — the same code without rationale comments, `rustfmt`-shaped and CI-ready.
   Produce it only when asked, or when the annotations would obscure the diff; otherwise part 2
   is the deliverable and saying so is enough.
4. **Scoring** — `net: -<N> lines possible.` Nothing to cut: `Lean already. Ship.` and stop.

## Boundaries

Over-engineering and complexity only, while respecting every Rust soundness rule. Correctness
bugs, security holes, and performance are out of scope unless caused by an abstraction — route
them to a normal review pass. A single smoke test or `assert`-based self-check is the ponytail
minimum, never a deletion candidate. Do not write to files unless asked: `AGENTS.md` autonomy is
"inspect and report" for review, and the refactor parts are code in the reply. Never commit,
push, branch, merge, or open a PR. Never propose weakening a validation, provenance, curation,
migration, lint, dependency, or authorization control to make a test pass. Do not claim a check
passed unless it ran.

Conflict order is the one `AGENTS.md` publishes under *Sources of truth*. **There is no accepted
ADR catalog in this tree**; a finding may not cite one, and a proposed decision authorizes
nothing.

## Checks to run before concluding

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run --workspace --locked        # just test; needs Docker
just static                                    # boundaries, OpenAPI, migrations, Squawk, Compose
just docs-check                                # RUSTDOCFLAGS=-D warnings
```

No Taplo configuration exists in this repository (`crates/AGENTS.md` §8). Do not run or claim a
`taplo fmt --check`. `just check-offline` is currently equivalent to `just check` — the justfile
says so — and is not evidence of SQLx offline metadata.

## Repo-specific checks, by section

**Errors & control flow.** One distinct `thiserror` variant per caller-visible distinction, so a
test can assert the exact failure. Flag any catch-all variant used where the failing subsystem is
known and nameable. Diagnostic text is bounded and control-stripped before it reaches a terminal,
a log, or a database column — `sanitize_database_message` caps at 1000 characters and drops
control characters; `sanitize_failure` does the same for run audit. A new error path that
formats a database or source string without going through one of those is a finding. Never log or
embed a database URL, credential, authorization header, raw source payload, or unsanitized host
path; the input locator is a sanitized basename or `<stdin>`, never a path.

**Async and bounded work.** Every queue, retry, pool, batch, and diagnostic has an explicit
ceiling. The two named examples are the record channel (`mpsc::channel(16)` in
`planephd::open_records`) and the import admission semaphore in `SqlxIngestionStore::start_import`,
which exists because startup holds the long-running transaction connection *and* a second
connection for the durable audit transaction — removing it deadlocks the pool at small sizes.
Treat both as load-bearing; deleting a bound is a Critical finding, not a simplification.

**Types, traits, coherence — leniency is directional here.** Inbound scraped source data is
lenient *with evidence*: an unrecognized PlanePHD field becomes an `UNSUPPORTED_RECORD_FIELD`
warning and stays in the preserved raw JSON, because rejecting a live scrape on a new field would
stop ingestion and lose the record. Do not "fix" that into `deny_unknown_fields`. The opposite
rule holds for a format this project *defines*: the versioned JSON report and the status and
curation output are durable machine-readable contracts — `REPORT_SCHEMA_VERSION` must move
deliberately, `rename_all` spellings must be pinned by a test, and a silent shape change is a
Critical finding. Validate format at parse time, not at comparison time: a malformed digest must
be reported as malformed, never as a mismatch.

**Safety & ownership.** Where a gate must precede work, the *ordering must be provable by a test*,
not merely present. `hard_issue_rolls_back_and_persists_failure_audit` and
`second_pass_key_mismatch_rolls_back` are the shape: assert the failure *and* assert that nothing
was published. Staging, promotion, provenance, assertions, curation flags, run completion, and
the read-model refresh commit or roll back together; the failure audit is deliberately a separate
transaction so a rollback cannot erase the attempt. A change that moves work across that seam is
Critical until a test proves the seam held.

**Visibility & architecture.** `pub` only at the intended API boundary; `unreachable_pub` is
`warn`. Dependency direction is inward and `cargo run -p xtask -- boundaries` enforces it — but
the check reads manifests, so a layering violation smuggled through a re-export is invisible to
it and yours to catch. Keep SQL inside `aircraft_db`, source parsing inside `aircraft_ingest`,
transport inside `aircraft_api`. Source JSON, prepared records, domain values, SQL rows, HTTP
DTOs, and OpenAPI schemas stay separate representations even where their shapes coincide;
collapsing two of them into a convenience type is a Major finding, not deduplication.

Any constant, table, or status vocabulary transcribed from canonical SQL carries a **two-sided**
comment — the Rust names the migration or validation file, that file names the Rust path. A
one-sided mirror is a finding, as is a hand-written string spelling with no test pinning it to
its serde representation. **Read both sides and check they agree.** That the mirror *exists* is
the cheap half; the finding that matters is a Rust condition weaker or narrower than the SQL its
own comment cites. Open the migration, put its constraint beside the predicate, and compare them
— a review that reads the code against itself will pass a control that has quietly come apart
from its schema, and every test written against the weaker behavior will agree with it.

**Tests.** `rust-testing` owns test policy; review against it. Three things are review-specific: a
test name is a full behavior sentence in `snake_case` and must not gain a tier or milestone
prefix (`crates/AGENTS.md` forbids inventing them); an expectation table must be an exhaustive
`match` rather than a hand-maintained array that silently misses a new variant; and a property or
loop whose body can never run is worth `assert!(true)` — mutate the code and confirm the test
fails before counting it.

## Traps that have bitten this repo

- **Migrations are immutable once hashed.** `database/migrations.lock.json` pins a SHA-256 per
  file and `cargo run -p xtask -- migrations` rejects an edit to an applied one. A stale comment
  in a migration is corrected in `database/data_dictionary.md` or
  `database/implementation_notes.md`, never by editing the migration. A new migration also needs
  an explicit `BEGIN`/`COMMIT`, a validation companion under `database/validation/`, an installer
  entry, and a `Squawk` baseline row — the xtask checks all four.
- **A new migration silently breaks the test harness.** `aircraft_testsupport::SCHEMA_STEPS` and
  `COVERED_MIGRATIONS` embed the canonical install order with `include_str!`;
  `schema_steps_cover_every_migration` is the only thing that notices. Adding SQL without
  updating both is a Major finding even when every existing test passes.
- **Regenerating a golden is not verifying a change.** `cargo xtask snapshots` diffs normalized
  business snapshots against committed output. Re-running it with `--` to accept new output turns
  a behavior regression into a green build. Read the snapshot diff before it changes.
- **A vacuous property passes.** `AGENTS.md` records two properties in this repository that
  passed because their generator could not produce the failing input. A green suite is not
  evidence that a new test can fail.
- **A moved function is not an unchanged function.** Verifying that a refactor preserved behavior
  answers "did I break it", never "was it right". When a diff moves code that mirrors a migration
  or a validation file, re-read it against that SQL, not only against its previous self.
- **`apps/server` is excluded legacy Actix/Diesel source and `archive/` is read-only.** Do not
  revive either to make something compile, and do not cite an empty aircraft/search/comparison
  scaffold as an example to imitate.
- **Private aircraft datasets, database dumps, `.env`, and production diagnostic payloads never
  enter Git, CI, fixtures, snapshots, or logs.** Small synthetic fixtures only.
