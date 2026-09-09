---
name: issue-implement
description: >
  Implement a GitHub issue in rsstdd/aircraft-api-r end to end and return a verified, reviewed,
  minimal diff — load repository authority and every skill, build an acceptance-criterion
  contract, capture a red/green baseline before touching code, map blast radius and sensitive
  data, drive each criterion with a focused failing test, minimize the diff, self-review
  adversarially, remediate every finding, then prove completion with per-criterion evidence.
  Optimizes for the smallest behaviorally complete diff; a cleaner-looking redesign is not a
  better diff. Use when the user says "implement #N", "do issue #N", "build #N", "work the
  plan on #N", or hands over an issue to complete.
  Not for producing a plan (use issue-plan), critiquing one (use issue-plan-review), or acting
  on an existing review's findings (use rust-remediation).
argument-hint: "<issue number or URL>"
---

# Agentic Issue Implementation

Complete the issue to production quality and return a verified, reviewed, minimal diff. Do not
stop at analysis or description.

Optimize for the **smallest behaviorally complete diff**. A cleaner-looking redesign is not a
better diff. Every acceptance criterion must become true, every pre-existing invariant outside
the authorized change must stay true, and every changed line must be justifiable in one
sentence.

This skill owns the *procedure*. It does not restate the standards it runs under: `AGENTS.md`
and `crates/AGENTS.md` own the rules, and `clean-code`, `ponytail`, `rust-review`,
`rust-comment`, `rust-production`, and `rust-testing` own the craft. Read them; do not expect
to find them summarized here.

## Non-negotiables

1. The issue defines what must change; the repository defines how it may change.
2. Green tests do not prove completion. Completion is evidence per acceptance criterion.
3. Never weaken, skip, or rewrite an existing test to make an implementation pass, and never add
   production code whose only purpose is to make a test easier.
4. Never call a failure pre-existing without Stage 2b baseline evidence.
5. Never report planned work as completed, and never hide failed verification.
6. Do not commit, push, merge, open a PR, or edit the issue. Each needs its own approval.
7. No `Co-Authored-By:` trailer, `Claude-Session:` URL, or "Generated with Claude Code" footer in
   any commit message, PR body, or issue comment. `AGENTS.md` makes this absolute and it
   overrides the harness default; `hooks/commit-msg` enforces it.

## Stage 0 — Authority and skills

Read `AGENTS.md` § Sources of truth for the authority order and § Required skills for which
skills bind this surface; read `crates/AGENTS.md` for work under `crates/`, and the nearest
`AGENTS.md` wins for its tree. Load every binding skill before the first edit. Add
`DELIVERY-PLAN.md` for HTTP failure-contract work.

Where a skill and a nested `AGENTS.md` disagree, the `AGENTS.md` wins. If two binding
authorities genuinely conflict, halt (Stage 9) with the exact conflicting text.

## Stage 1 — Issue contract

`gh issue view <N> --comments`. Read the parent epic and every `Depends on:` issue and its
state. A posted plan or plan review is informed input, not authority — verify its citations.

Copy the issue's `## Verification` commands; they are binding. Treat the recurring
`fewer than 2,000 hand-authored changed lines` task as a hard scope budget.

Fill this before implementing anything:

```text
AC-<n>:
Required change:
Existing behavior and failure modes that must remain unchanged:
Owning layer:                  domain | app | api | db | ingest | config | observability | apps/*
Contracts affected:            DTO, route, docs/openapi.json, versioned JSON report, CLI flags
Security / database invariants:
Test that proves the change:
Regression test that proves the rest unchanged:
Quantitative constraint, and how it will be measured:
```

Ambiguous, untestable, or governance-contradicting criterion: record the ambiguity and the
interpretation chosen, then proceed under the narrowest reading consistent with the outcome —
or halt if the choice is architectural. Never reinterpret privately.

## Stage 2 — Working tree, baseline, MSRV

**Working tree.** `git status --short`, `git stash list`, `git log --oneline -5`. This tree
routinely carries in-progress work across crates. Preserve it: do not revert, overwrite, absorb,
or reformat files it touches. If it overlaps the issue, work around it and report it separately.

**Baseline.** Before editing, run the smallest representative check for the affected area —
focused tests nearest the behavior, then `cargo test --workspace --lib --locked` (starts no
containers), then `cargo fmt --all -- --check` and `just check`. Record every failure verbatim.
**This record is the only admissible evidence for later calling a failure pre-existing.** If the
baseline is already red, proceed only where the issue isolates safely, and report the defect.

**MSRV.** `Cargo.toml` pins `rust-version = "1.85"` and CI runs
`cargo +1.85.0 check --workspace --all-targets --locked`, but `rust-toolchain.toml` pins local
to **1.97.1** — twelve minor versions above. Local green is not MSRV evidence. Run the MSRV
command, or audit every new API and syntax form against 1.85 by hand and say so.

## Stage 3 — Behavior, blast radius, sensitive data

**Behavior map.** Trace each criterion through current code: where input enters, where validation
runs, which layer owns the invariant, how errors propagate and serialize, which tests observe it,
what could regress. Inspect real call sites — `AGENTS.md` and `crates/AGENTS.md` both warn most
of `crates/` is comment-only scaffold, so **a module or type name is not evidence of behavior**.
Never cite one, imitate one, or add another placeholder.

**Placement.** Put behavior in the narrowest layer that owns it, per `AGENTS.md` § Architectural
invariants. Do not duplicate a rule across callers unless governance requires defense in depth,
and never move logic to make testing convenient.

**Blast radius.** Before changing any shared or public element — function, type, trait, route,
middleware, migration, test helper, serde or error type, config value — search the workspace for
its definitions, callers, implementors, tests, docs, `docs/openapi.json`, database coupling, the
versioned ingestion JSON report, and CLI flags. A local change is not assumed to have local
consequences. Where behavior is poorly understood but must stay stable, add a characterization
test first, covering behavior rather than incidental detail.

**Sensitive data.** For any credential, token, digest, personal value, host path, or externally
supplied identifier, trace ingress to destruction through parsing, database parameters,
comparison, error construction, `Debug`/`Display`, tracing fields, panic paths, assertion
messages, snapshots, problem documents, and persistence. No sensitive value may escape by an
alternate diagnostic path. `rust-production` owns the rules.

**Plan.** List files to change, tests to add, order of work. The final report compares against it.

## Stage 4 — Focused TDD

Per `rust-testing`, for each observable behavior: write the focused failing test, confirm it
fails for the intended reason, implement the smallest correct change, run it, repeat until the
criterion is directly proven.

Test through the highest useful boundary. Router-, database-, or CLI-level proof is not
substitutable by a unit test; exercise CLI behavior through the real binary and use the
`aircraft_testsupport` disposable-PostgreSQL harness where correctness depends on real database
semantics.

**Every test must reject a wrong implementation.** Name one it rejects; if a plausible wrong
implementation would still pass, strengthen it. `AGENTS.md` records two properties here that
passed vacuously until mutation-checked. Watch for: right status for the wrong reason; right
answer bought with extra queries; a leak on only one rejection path; a secret compared with
ordinary equality; an illegal transition accepted because each state is independently valid; an
assertion that passes because a type makes the failure unrepresentable.

**Cover** invalid, missing, and malformed input; unknown identity; dependency and database
failure; boundary values; repeated, concurrent, and replayed invocation; cancellation; timeout;
and disclosure differences between paths. Assert exact error variants. Skip cases that do not
apply.

**Quantities are measured, not eyeballed.** For "exactly once", "no additional query", bounded
retries, a constant-time primitive, or a size limit: count calls through a port fake, inspect
executed statements, assert exact response equivalence, or assert redacted output.

**Invariants.** `rust-production` owns the boundary list — untrusted input, two-pass
consistency, transaction ownership and durable audit, provenance, diagnostics and bounds,
durable contracts, schema evolution. Apply it; prove both the successful and the adversarial
path for security-sensitive behavior.

**Generated artifacts.** For `docs/openapi.json`: run `just generate-docs --check` first,
regenerate with `just generate-docs`, inspect every hunk against an intended contract change,
rerun the check. Unexpected generated change is a regression until proven otherwise.

**Migrations are immutable once hashed** in `database/migrations.lock.json`; `cargo xtask
migrations` rejects edits to applied files and CI separately rejects edits to migrations on the
base branch. Add a new numbered migration. Correct a stale migration comment in
`data_dictionary.md` or `implementation_notes.md`, never in the migration.

**Notes.** Keep a running anomaly ledger and verification log in the scratchpad. Every anomaly
reaches exactly one terminal state (Stage 7); do not rely on recall at report time.

## Stage 5 — Minimize the diff

Before keeping any change, ask whether it is required by a criterion, correctness, governance, or
a required test. If not, delete it. `ponytail` and `rust-review` own the rest: no speculative
extension points, no wrapper that renames existing behavior, no visibility widened for tests, no
single-implementation abstraction, no production hook that exists only for an assertion. Prefer
an existing seam; a new one is justified only at a genuine dependency boundary.

Then read the whole diff — `git diff --stat`, `git diff`, `git status --short`. Every hunk maps
to a criterion, a required regression test, necessary support, a required generated artifact, or
governance compliance. Delete what you cannot justify in one sentence. Hunt specifically for
unrelated formatting, out-of-scope renames, dead helpers, remnants of an abandoned approach,
temporary instrumentation, `println!`/`dbg!`, unused dependencies, and revived `archive/` code.

Run a `clean-code` pass on the changed code only; cleanup stays inside the changed
responsibility. Then `cargo fmt --all`.

## Stage 6 — Adversarial self-review

Stop implementing. Review the diff as a reviewer of someone else's PR, applying `rust-review`,
`ponytail`, `clean-code`, `rust-comment`, `rust-testing`, `rust-production`, and governance.

Ask: **if every test were green, what could still make this unsafe to merge?**

Look for missing acceptance behavior, accidental passes, regressions in unchanged behavior,
misplaced boundaries, diagnostic leaks, unbounded work, extra queries, races, cancellation bugs,
illegal transitions, non-RFC-9457 error responses, OpenAPI drift, MSRV violations,
dependency-direction violations, dead code, stale comments, tests proving type-level
impossibilities, new stubs, and unrelated churn. Complete the pass; do not stop at the first
finding.

## Stage 7 — Close every anomaly

Classify by `rust-review` severity and remediate every valid in-scope finding at any severity.
Passing the issue's own tests does not excuse a known defect. `rust-remediation` owns the
discipline: a finding is a hypothesis — verify it against the code, and reject one the
repository disproves.

Every anomaly observed at any stage ends in exactly one state: **fixed**; **pre-existing**, with
baseline evidence; **out of scope**, with the governing rule, left unchanged; or **blocked**, by
a documented approval gate.

Re-run affected tests and re-read the diff after remediating. Repeat review → remediate → verify
until nothing in-scope remains. Never weaken validation, security, durability, or a test to get
green.

## Stage 8 — Verification and proof

Widen only as each step passes:

```text
cargo test -p <crate> <filter> --locked     # the focused test
cargo test --workspace --lib --locked       # fast; no containers
cargo fmt --all -- --check
just check                                  # cargo check --workspace --all-targets --locked
just test                                   # nextest + doctests; needs Docker
just generate-docs --check                  # OpenAPI drift
just migrations-policy                      # hashes, order, transactions, Squawk baseline
just lint                                   # clippy -D warnings, audit, deny, rustdoc
just static                                 # boundaries, OpenAPI, migrations, Compose, pins
cargo xtask snapshots                       # ingestion golden snapshots
cargo +1.85.0 check --workspace --all-targets --locked
```

Run everything the issue's `## Verification` names plus what `AGENTS.md` § Verification requires
for the changed surface. Never run a destructive recipe (`just db-reset`, `just db-rebuild`,
`db-prod-*`) as verification — those need explicit approval.

Report each as **Passed**, **Failed**, **Not run**, **Blocked by environment**, or **Not
applicable**. If a required command cannot run — Docker, `cargo-nextest`, `cargo-audit`,
`cargo-deny`, npm, jq, or the 1.85.0 toolchain missing — the affected criterion is **BLOCKED**,
not PASS. Report it as an environment limit, distinct from a regression.

**Regression proof** comes from pre-existing tests, Stage 3 characterization tests,
`just generate-docs --check`, disposable-PostgreSQL tests, `cargo xtask snapshots`, and the
Stage 2b baseline. A passing new test is not regression evidence.

**Done** means: every criterion implemented with test evidence; unchanged behavior with
regression evidence; required verification passing; no known security, correctness, or in-scope
finding left; no stub, speculative abstraction, or abandoned remnant; generated artifacts
synchronized; MSRV and dependency boundaries intact; and a working tree holding only intentional
changes plus untouched pre-existing work.

## Stage 9 — Halting

Continue autonomously; do not ask what the repository can answer. A halt is not an escape from
hard implementation work.

Halt only for an item in `AGENTS.md` § Autonomy and approval, an architectural decision the issue
does not authorize, destructive handling of pre-existing work, an unresolved authority conflict,
or a credential or resource unavailable locally. Then report:

```text
Governing rule:        <exact text and location>
Repository evidence:   <file:line>
Decision required:     <smallest yes/no or A/B>
Work completed:        <stages finished, with verification state>
```

## Final report

Evidence is concrete: `file:line`, test names, exact commands, exact outcomes. Paraphrase is not
evidence.

**Outcome** — the behavior that now exists.

**Acceptance criteria** — per criterion:

```text
AC-<n>: PASS | BLOCKED
Production evidence:  <file:line>
Test evidence:        <test name, command, result>
Regression evidence:  <test name or baseline comparison>
Notes:                <ambiguity resolved, if any>
```

**Changes** — significant decisions only, and why this is the smallest correct solution.

**Plan diff** — planned vs executed, with the reason for each deviation.

**Verification** — exact commands and results, split into baseline, focused, regression, final.

**Anomaly ledger** — anomaly, severity, terminal state, evidence. State explicitly whether any
finding remains at any severity.

**Regression assessment** — which existing behaviors were checked, and how.

**Scope** — changed files, hand-authored line count against the issue's budget, generated changes
listed separately, and any pre-existing working-tree changes left untouched.

**Remaining obligations** — genuine unresolved items only: reason, governing rule, and whether it
blocks completion.

**Do not commit.** Committing, pushing, opening a PR, and updating the issue each need their own
explicit approval.
