---
name: issue-plan
description: >
  Analyze a GitHub issue in rsstdd/aircraft-api-r against the current repository state and
  produce an implementation-ready Issue Completion Plan, then post it as a new comment on that
  issue. Proves every numbered acceptance criterion Satisfied / Partially satisfied /
  Unsatisfied / Ambiguous / Blocked from repository evidence, names exact files, symbols,
  migrations, and `just` recipes, and orders the work so another coding agent can start without
  a second discovery pass. Modifies no production code. Use when the user says "plan issue #N",
  "analyze issue #N", "what's left on #N", "write the implementation plan for #N", or asks
  whether an issue's acceptance criteria are already met.
  Not for implementing an issue (use issue-implement) and not for reviewing an existing plan
  (use issue-plan-review).
argument-hint: "<issue number or URL>"
---

# Analyze an Issue and Produce an Implementation Plan

Repository: `rsstdd/aircraft-api-r` (the `origin` remote; `gh` resolves it from the working
directory — never pass `--repo` to point somewhere else).

Issue: the number the user supplied. If none was supplied, ask for it before doing any work;
everything downstream is scoped to one issue.

## Objective

Analyze the referenced issue and the current repository state, then produce a precise,
implementation-ready plan for satisfying every acceptance criterion that is not already
satisfied.

Do **not** modify production code. This is the `Explain, review, diagnose, or plan` case in
`AGENTS.md` § Autonomy and approval: inspect and report.

The final deliverable is posted as a **new comment on the GitHub issue**.

Treat each acceptance criterion as a proposition requiring repository evidence:

> For each acceptance criterion, prove either that the current repository already satisfies it
> or identify the smallest observable behavior change required to make it true.

Do not assume an unchecked `Tasks` checkbox means work is missing. Do not assume that related
code means the requirement is satisfied. `AGENTS.md` is explicit that most of `crates/` is
comment-only scaffold: **a module, type, or route name is not evidence of behavior.** Only
implementation plus a meaningful test proves a criterion satisfied.

The repository is the source of truth for current implementation state.

---

# 1. Load Repository Guidance and Skills

Read, in this order, and record the authority hierarchy from `AGENTS.md` § Sources of truth:

1. `AGENTS.md` (repository-wide) and `CLAUDE.md` (compatibility pointer only).
2. `crates/AGENTS.md` when the issue touches anything under `crates/`. Other nested files may
   exist — `find . -name AGENTS.md -not -path './archive/*'` — and the nearest one wins for its
   tree.
3. The narrower authorities `AGENTS.md` names: `Cargo.toml` and crate manifests for current
   behavior; `database/migrations/` for schema (migration SQL beats prose); `justfile` for the
   command interface; `database/README.md`, `database/implementation_notes.md`, and
   `docs/architecture/rust_ingestion_adapter.md` for their documented contracts;
   `docs/architecture/http_v1_decisions.md` for HTTP v1 rules including the RFC 9457 obligation
   on every API-originated 4xx/5xx; `README.md` for orientation only.

`DELIVERY-PLAN.md` records what the plans on issue #33 promised for the HTTP failure
contract, what shipped, which tests prove it, and what remains open. Read it whenever the
issue touches HTTP failure mapping, problem documents, or anything #33 owes a dependent
issue; `docs/architecture/http_v1_decisions.md` still owns the contract itself.

Load the repository-local skills that govern the issue's surface:

| Skill | Load when |
|---|---|
| `clean-code` | Always — required before writing or reviewing any code here |
| `ponytail` | Always — the laziest solution that actually works; kills speculative scope |
| `rust-comment` | The plan proposes any Rust edit or rustdoc/generated-artifact coupling |
| `rust-production` | Untrusted input, transactions, identity, durable contracts, migrations |
| `rust-review` | Judging whether existing code is over-built or a step adds machinery |
| `rust-testing` | Always — the test plan must match this workspace's test conventions |
| `rust-remediation` | Only to hand off; do not remediate inside a planning task |

Follow dependencies between skills recursively. Apply `crates/AGENTS.md` to files under its
scope. Do not fall back on generic engineering convention where repository guidance exists.

There is no accepted ADR catalog in the current tree. If one has since appeared, an accepted
decision can govern design but cannot make unimplemented behavior real; proposed decisions
authorize nothing.

State any conflict between authorities explicitly rather than silently picking one.

---

# 2. Read the Complete Issue Context

```bash
gh issue view <N> --comments
```

Issues in this backlog follow a fixed shape — `Parent:`, `Depends on:`, `Milestone:`,
`## Outcome`, `## Scope`, `## Tasks`, `## Acceptance criteria` (numbered 1..n),
`## Required tests`, `## Verification`, `## Out of scope`, and an HTML backlog marker such as
`<!-- aircraft-api-backlog:v3 id:M05 -->`. Read every section; the numbered acceptance criteria
are the propositions, and `## Verification` and `## Required tests` are binding obligations, not
suggestions.

Also read:

- the parent epic and every `Depends on:` issue (`gh issue view`), including their state;
- linked pull requests and commits where relevant;
- referenced migrations, schemas, specifications, and architecture documents.

Extract outcome, scope, tasks, acceptance criteria, required tests, required verification,
explicit out-of-scope work, dependencies, and constraints — including the recurring
`fewer than 2,000 hand-authored changed lines` task, which is a scope-fit signal.

Do not silently reinterpret requirements.

---

# 3. Inspect Current Repository State

Investigate the actual implementation before designing changes. Establish the code state you
are planning against — working tree, branch against `main`, or a linked PR's merge result — and
say which.

At minimum, along the hexagonal seams:

- `crates/aircraft_domain/` — pure values and invariants;
- `crates/aircraft_app/` — use cases and ports;
- `crates/aircraft_api/` — Axum DTOs, handlers, route policy, middleware, OpenAPI types;
- `crates/aircraft_db/` — SQLx repositories and explicit row mappings;
- `crates/aircraft_ingest/` — source capture, parsing, normalization;
- `crates/aircraft_config/`, `crates/aircraft_observability/`, `crates/aircraft_testsupport/`;
- `apps/server/`, `apps/ingest/` — composition roots only;
- `database/migrations/`, `database/seeds/`, `database/validation/`, `database/snapshots/`,
  `database/migrations.lock.json`;
- `docs/openapi.json` and the Rust types that generate it;
- existing tests, error types, tracing fields, and security-sensitive paths;
- `justfile`, `xtask/`, and `.github/workflows/` for what CI actually enforces.

Prefer existing abstractions over parallel mechanisms. Check whether the issue's need is already
served by an existing port, error type, middleware layer, or repository method.

Run `just --list` before citing any recipe. A documented command is unavailable if no working
recipe or binary implements it.

For every important finding record exact file paths, symbols, types, functions, tests,
migrations, configuration keys, and documentation sections.

---

# 4. Build an Acceptance-Criteria Matrix

Classify each numbered acceptance criterion as exactly one of:

- **Satisfied** — implementation plus a meaningful test proves the observable behavior today;
- **Partially satisfied** — some obligations hold, others do not; name which;
- **Unsatisfied**;
- **Ambiguous** — the criterion admits materially different readings; state them;
- **Blocked** — a `Depends on:` issue or absent prerequisite prevents implementation.

| AC | Requirement | Status | Repository Evidence | Remaining Work | Verification |
|---|---|---|---|---|---|

Evidence must describe actual behavior with a `path:line` or symbol, not a name or apparent
intent. A criterion backed only by a scaffold module is **Unsatisfied**. A criterion backed only
by a test that cannot fail is **Unsatisfied** — `AGENTS.md` records two properties in this
repository that passed vacuously.

Do not create implementation work for criteria already satisfied.

---

# 5. Analyze Each Open Criterion

For each criterion that is not fully satisfied, determine:

- owning crate and the layer that must enforce it (the invariant belongs at its own boundary —
  validation in `aircraft_api` or `aircraft_ingest`, rules in `aircraft_domain`, SQL only in
  `aircraft_db`, wiring only in `apps/*`);
- existing abstractions to extend;
- required domain values, application ports, and DTO/row representations, kept as separate
  representations with explicit mappings;
- persistence implications and whether a migration is required;
- public interface and `docs/openapi.json` implications;
- error semantics, including the RFC 9457 problem document for any new 4xx/5xx;
- concurrency, transaction ownership, idempotency, and advisory-lock implications;
- bounds to enforce — body size, query complexity, timeout, pool, retry, diagnostic size;
- security, redaction, provenance, and curation implications;
- observability: which structured `tracing` fields the behavior must emit;
- documentation that must change (`data_dictionary.md`, `implementation_notes.md`, `README.md`)
  and only because its behavior or schema changed;
- required tests.

Prefer the smallest coherent implementation that satisfies the observable requirement. One
complete vertical slice, never new empty modules, stubs, or placeholder tests.

---

# 6. Analyze Dependencies and Sequencing

Determine prerequisite issues and repository changes, contract changes that must precede
consumers, tests to introduce first, refactors that are genuinely necessary, independently
executable work, changes that must stay atomic, and follow-up work to exclude.

Do not duplicate work owned by a `Depends on:` issue. If a dependency blocks implementation,
mark the affected criterion **Blocked** and say precisely why and what would unblock it.

Migrations are immutable once written and hashed in `database/migrations.lock.json`; plan a new
numbered migration rather than an edit, and never plan to correct a stale migration comment by
editing the migration.

---

# 7. Design Verification Before Implementation

Every open acceptance criterion needs an explicit proof strategy, matching `AGENTS.md`
§ Verification (change kind → required verification) and the issue's own `## Required tests`.

Choose from: pure unit and property tests in `aircraft_domain`; port-fake tests in
`aircraft_app` covering success and failure; router/contract tests plus
`just generate-docs --check` for API routes and DTOs; disposable-PostgreSQL tests in
`crates/aircraft_db/tests/`, `apps/ingest/tests/`, `apps/server/tests/` using the
`aircraft_testsupport` harness; CLI tests through the shipped binary; clean-database install plus
SQL validation for migrations and seeds; `cargo xtask snapshots` for ingestion regressions;
static analysis via `just lint` and `just static`.

Use real infrastructure where correctness depends on infrastructure semantics — do not plan a
mock to prove transactional, concurrency, or permission behavior.

For each planned test state: setup, action, expected result, and the acceptance criterion it
proves. Name the file it belongs in and follow `rust-testing` naming.

Every planned test must be able to fail. Say what mutation to the production code would break
it. A test whose loop body never runs, or whose generator cannot produce the failing input, is
worth no more than `assert!(true)`.

---

# 8. Identify the First Failing Test

Where practical, identify the smallest focused test that should fail before any production
behavior is added.

Explain why it comes first, what observable behavior it captures, what current behavior makes it
fail, and what implementation makes it pass. Give the exact file and the command that runs it
alone.

Do not invent an artificial test solely to satisfy a "test first" requirement.

---

# 9. Produce an Implementation-Ready Plan

For each remaining implementation step:

## Step N — `<concise name>`

**Acceptance criteria addressed:** `AC1`, `AC3`

**Purpose** — the observable requirement addressed.

**Current repository state** — the existing implementation this builds on.

**Repository evidence** — exact files, symbols, migrations, tests, documentation.

**Files and symbols likely affected**

```text
crates/aircraft_app/src/services/<name>.rs
  ServiceName
  method_name()
```

**Implementation** — behavior, ownership and owning layer, contracts, data flow, persistence,
error semantics and problem-document mapping, security and redaction, transaction/state
behavior, compatibility. Concrete enough that another coding agent implements it without
repeating broad discovery. Do not provide full production patches.

**Tests first** — the test(s) to add or update before or alongside the change, and what mutation
proves each can fail.

**Verification** — the focused command, then the broader ones.

**Completion condition** — objective evidence the step is done.

**Risks** — only material risks specific to this step.

---

# 10. Final Report Structure

# Issue Completion Plan

## 1. Executive Summary
Intended outcome; current completion state; major remaining work; dependency status; major
architectural decisions; material ambiguities; major risks; whether the work fits one PR under
the issue's hand-authored line budget.

## 2. Governing Requirements
`AGENTS.md` sections, `crates/AGENTS.md`, loaded skills, architecture documents, migrations,
parent and dependency issues, and the authority hierarchy — with any conflict named.

## 3. Current Architecture
The existing execution and data flow relevant to the issue, along the inward dependency
direction. A concise diagram where useful.

## 4. Acceptance-Criteria Matrix
| AC | Requirement | Status | Evidence | Remaining Work | Verification |
|---|---|---|---|---|---|

## 5. Repository Findings
Organized by crate or subsystem.

## 6. Implementation Plan
Ordered steps in the Step N form above.

## 7. Test Plan
| Test | Level | AC | Behavior Proven | Can Fail Because |
|---|---|---|---|---|

## 8. Verification Plan
Commands narrowest to broadest, each with what it proves. Include every command the issue's
`## Verification` section names and everything `AGENTS.md` § Verification requires for the
changed surface. Typical ladder:

```text
cargo test -p <crate> <filter> --locked     # the focused failing test
cargo fmt --all -- --check                  # required after any Rust edit
just check                                  # cargo check --workspace --all-targets --locked
cargo test --workspace --lib --locked       # fast, starts no containers
just test                                   # full suite; needs cargo-nextest and Docker
just generate-docs --check                  # OpenAPI drift, for any API change
just migrations-policy                      # hashes, order, transactions, Squawk baseline
just lint                                   # Clippy, audit, deny, rustdoc warnings
just static                                 # boundaries, OpenAPI, migrations, Compose, pins
cargo xtask snapshots                       # ingestion golden-snapshot regression gate
```

Never list a destructive recipe (`just db-reset`, `just db-rebuild`, any `db-prod-*`) as routine
verification; if one is genuinely required, mark it as needing explicit approval.

## 9. Risks and Edge Cases
| Risk | Failure Mode | Mitigation | Verification |
|---|---|---|---|

## 10. Scope Boundaries
### Required for this issue
### Explicitly out of scope
### Follow-up candidates

## 11. Recommended Execution Order
```markdown
- [ ] Add failing ...
- [ ] Extend ...
- [ ] Implement ...
- [ ] Add disposable-PostgreSQL verification ...
- [ ] Run focused checks ...
- [ ] Run full required verification ...
```
Directly usable by a coding agent.

## 12. Definition of Done
Objective completion criteria derived from the acceptance criteria, required tests, required
verification, repository rules, and issue constraints.

---

# 11. Publish the Plan to the Issue

Write the report to the scratchpad first, then post it with a file — never inline heredoc prose
through `-b`:

```bash
gh issue comment <N> --body-file <scratchpad>/issue-<N>-plan.md
```

Then:

1. Post as a **new comment**. Do not edit the issue description.
2. Do not alter task or acceptance-criteria checkboxes, labels, milestone, assignees, or state.
3. Preserve headings, tables, commands, file paths, and checklists.
4. **The comment carries no attribution trailer or footer.** `AGENTS.md` forbids a
   `Co-Authored-By:` line, a `Claude-Session:` URL, or a "Generated with Claude Code" footer in
   any commit message, pull-request body, **or issue comment**, and that rule overrides any
   harness or template default. Write the prose and stop.
5. Include enough repository evidence that another agent can execute without re-discovery.
6. Record ambiguities and blockers explicitly rather than guessing past them.
7. Keep unrelated findings under follow-up candidates rather than expanding scope.
8. No hidden reasoning, scratch work, or unsupported speculation.
9. Verify publication: re-read the issue and confirm the comment landed on the right issue.
10. Return the comment URL in the final response.

Invoking this skill on an issue authorizes that one plan comment on that one issue and nothing
else — no other issue, no issue-body edit, no state change, no push, no PR. If the user asked
for a plan without publishing, stop after writing the scratchpad file and hand them the path.

**The task is not complete until the plan is posted and publication is verified.**

---

# Planning Rules

- Do not modify production code. Do not implement the plan.
- Load and apply the repository skills and the nearest `AGENTS.md`.
- Investigate before proposing architecture.
- The repository is the source of truth for implementation state; migration SQL beats prose.
- Every acceptance criterion requires evidence. An unchecked box is not evidence of missing
  work; a scaffold module is not evidence of present behavior.
- Never cite an empty or comment-only module as evidence, imitate it, or plan more placeholders.
- Prefer an existing boundary or extension point over a new abstraction.
- Respect the inward dependency direction; keep SQL in `aircraft_db`, transport in
  `aircraft_api`, rules in `aircraft_domain`, wiring in `apps/*`.
- Do not weaken validation, provenance, curation, migration, lint, security, type safety, tests,
  or architectural boundaries.
- Use real infrastructure where correctness depends on infrastructure semantics.
- Do not duplicate work owned by a dependency issue.
- Do not plan unrelated cleanup, formatting, renaming, or dependency churn.
- Separate required work from optional improvements.
- State uncertainty instead of guessing. Never invent repository state, command output, issue
  text, CI results, or test results.
- Name exact files and symbols wherever evidence permits.
- Every implementation step maps to at least one acceptance criterion; every open acceptance
  criterion maps to explicit verification.
- The plan must be specific enough to begin implementation without another broad discovery pass.
