---
name: issue-implement-review
description: >
  Acceptance-driven implementation review for `rsstdd/aircraft-api-r`: given uncommitted work, a
  branch, or a pull request, decide whether the implementation satisfies every criterion in its
  governing issue, at the correct enforcement boundary, under the repository's binding
  architecture. Reconstructs the contract from the issue and the owning records first, then
  compares it against the real code — green CI, passing tests, preserved behavior, and a small
  diff prove nothing on their own. Reviews and reports; writes no production code and never
  commits. Use on "review this work", "does this satisfy the issue", "is this PR ready", "audit
  the branch before merge".
argument-hint: "<PR number, branch, or nothing for the working tree>"
---

# Implementation review — does the work satisfy its issue?

The governing question, and the only one this skill answers:

**Does the authoritative working-tree, branch-integration, or merge-result implementation satisfy
every criterion at the correct boundary under the repository's binding architecture, with the
smallest production-grade Rust that does it?**

## Authority

Conflict order is the repository's, from `AGENTS.md` §Sources of truth: the nearest `AGENTS.md` →
manifests, implementation, and meaningful tests → `database/migrations/` → `justfile` → the owning
database and architecture documents → `README.md` → the skills. Migration SQL beats prose. There is
no accepted ADR catalog in this tree; if one appears, an accepted decision governs design but
cannot make unimplemented behavior real, and a proposed one authorizes nothing. Flag a genuine
conflict; never resolve one silently.

Load before judging any implementation: `ponytail` and `clean-code` always; `rust-review`,
`rust-comment`, `rust-production`, and `rust-testing` when the change touches `crates/`;
`crates/AGENTS.md` for anything under it, which wins over a skill where they disagree. Read
`docs/architecture/http_v1_decisions.md` for HTTP work, `docs/architecture/rust_ingestion_adapter.md`
for ingestion, and `DELIVERY-PLAN.md` when the change touches the HTTP failure contract.

**Where this sits beside `rust-review`.** That skill is scoped to over-engineering and complexity
and says so at `rust-review/SKILL.md:92`: "Correctness bugs, security holes, and performance are
out of scope unless caused by an abstraction — route them to a normal review pass." This is that
pass. Its `delete:` / `yagni:` / `shrink:` findings belong here too, folded in under §Findings
rather than reported twice.

## Fixed parameters here

- **Non-mutating.** No edit to tracked files, no commit, branch, push, merge, or pull request —
  `AGENTS.md` §Autonomy and approval reserves every one of those. No `cargo fmt` without
  `--check`, no `just generate-docs` without `--check`, no `cargo xtask snapshots --update`, no
  regeneration of any tracked artifact.
- Never run a destructive recipe to review: `just db-reset`, `just db-rebuild`, and every
  `db-prod-*` delete data or target a non-local database.
- Temporary untracked output is allowed when a check needs it, in the scratch directory, cleaned
  up before the report.
- Review the work; do not fix it. Offer the remedy in the report and stop. Remediation is
  `rust-remediation`'s job.
- Never weaken a validation, provenance, curation, migration, lint, or authorization control to
  make a check pass, and never recommend that a submission do so.

## 1. Establish the authoritative state

**Working tree** — staged, unstaged, and untracked against `HEAD`. Untracked files matter: a new
migration, seed, fixture, or test file is part of the change and carries its own gates. This tree
routinely holds unrelated in-progress work; separate it from the change under review and say so.

**Branch** — against its intended *current* base, not the fork point. Separate a feature defect
from base drift and say which. `git merge-base --is-ancestor origin/main HEAD` answers whether the
branch has the base it will merge into.

**Pull request** — the **merge result against the current base** is authoritative; the head diff is
secondary and shows scope and provenance. `gh pr view <N> --json mergeable,mergeStateStatus` says
whether it integrates, and `git merge-tree --write-tree <base> <head>` produces the merged tree
without touching the working tree or writing a commit. Branch-local CI is never proof of
merge-result correctness. If integration state cannot be inspected, integration-dependent
conclusions are **Unverified**, not passed.

## 2. Assemble the criteria set

`issue-plan` §2 and §4 own this and it is not restated: the criteria are the issue's numbered
`## Acceptance criteria`, with `## Required tests` and `## Verification` as binding obligations and
`## Tasks` as the work record. Assemble the set independently, **before reading the diff** — a
criteria set derived from the implementation will agree with the implementation.

Three things this repository makes criteria that a reader may take for bookkeeping:

- **A named test in an issue's `## Required tests` or in `DELIVERY-PLAN.md` is a criterion.**
  Missing, renamed, or `#[ignore]`d is unsatisfied. `grep DELIVERY-PLAN.md` before calling a rename
  harmless.
- **The owed documentation is a criterion.** A schema change owes `database/data_dictionary.md`
  and, where behavior or a dependency moved, `database/implementation_notes.md`. An API change owes
  a regenerated `docs/openapi.json`. Absent, these are violated criteria, not follow-ups.
- **The scope budget is a criterion** where the issue's `## Tasks` carries it: fewer than 2,000
  hand-authored changed lines, counted separately from generated output.

Grade each criterion **Satisfied**, **Partially satisfied**, **Violated**, or **Unverified**.
`Partially satisfied` is only for a multi-obligation criterion where one obligation holds and
another does not; it never softens a failed criterion, and the failed obligation is still Blocking.
`Unverified` is for what the evidence cannot decide without inventing facts — never a polite
`Satisfied`.

## 3. Account for every changed file

Classify all of them, by role — `scoped`, `supporting`, `unrelated`, `generated` — and by status —
`clear`, `suspicious`. Trace the behavioral hunks; still classify the mechanical ones. Do not
conclude while any hunk is unaccounted for.

Watch for what rides along: unrelated formatting or renaming mixed into scoped work, an obsolete
compatibility path kept alive, machinery for a caller that does not exist, revived `archive/` code,
a leftover `println!` or `dbg!`, and a doc comment updated to describe something the code no longer
does.

## 4. The invariant tests

**Specification fidelity.** Read the governing rule, read the implementation, compare their
semantics *directly*, and only then read the tests. Tests are evidence, not specification. None of
these is proof: the suite passes; behavior was preserved; the code already existed; both ends of a
mirror name each other; a guard exists somewhere; the old and new states are each independently
valid; branch CI is green. "Behavior preserved" answers *did I break it*, never *was it right*.

**A name is not behavior.** `AGENTS.md` and `crates/AGENTS.md` both record that most of `crates/`
is comment-only scaffold. A module, type, route, or port that exists proves nothing; only
implementation plus a test that can fail does. Treat a criterion backed by a scaffold as Violated.

**Tests that cannot fail.** `AGENTS.md` records two properties here that passed vacuously until
mutation-checked. A property whose loop body never runs, a generator that cannot produce the
failing input, and an assertion made unreachable by the type system are all worth `assert!(true)`.
Ask what mutation to the production code each new test would catch.

**Two-sided coupling.** That the mirror exists is the cheap half. Open the named migration or
document, put its sentence beside the predicate, and compare. A code condition narrower or weaker
than the document it mirrors is the finding that matters, and every test written against it will
agree with it.

**Boundary ownership.** For each invariant: which layer owns it, whether a caller can bypass it,
whether it runs before the protected or irreversible work, whether durable state can reach the
protected state without passing through it. `AGENTS.md` §Architectural invariants fixes the
direction — validation at the `aircraft_api` or `aircraft_ingest` edge, rules in
`aircraft_domain`, SQL only in `aircraft_db`, wiring only in `apps/*`. A correct check at the
wrong boundary is still a defect.

**Transition correctness.** Old state valid and new state valid do not make the transition legal.
If durable state can be written past the transition API, the in-memory method does not enforce it.

**Ordering.** "The gate runs before the work" is a control-flow claim; presence is not ordering.
Prove it: make the gate and the work fail differently, then assert the gate's error wins and that
no work happened.

**Transaction and provenance semantics.** `rust-production` owns these — one fate for promotion,
durable failure audit outside the rollback, advisory lock on logical identity, idempotent replay,
provenance preserved without becoming canonical. Check them where the change opens a transaction.

**Diagnostics.** Every API-originated 4xx and 5xx is an RFC 9457 problem document that leaks no
diagnostic, per `docs/architecture/http_v1_decisions.md`. No database URL, credential,
authorization header, raw personal data, or unsanitized host path reaches a log, an error, a
`Debug`, or an assertion message.

**Minimalism**, in `ponytail`'s order: does it need to exist, does the repository already have it,
does `std`, does the platform, does an existing workspace dependency, can it be smaller — and only
then new machinery.

## 5. Generated artifacts and this repository's traps

Generated output is never its own specification.

- **`docs/openapi.json`** is generated from the Rust types in `aircraft_api`. **Do not run
  `just generate-docs` to review** — it overwrites a tracked file. The non-mutating proof is
  `just generate-docs --check`. An OpenAPI hunk with no corresponding type change, or a type
  change with no OpenAPI hunk, is drift.
- **Migrations are immutable once hashed** in `database/migrations.lock.json`. `cargo xtask
  migrations` rejects an edit to an applied file, and CI separately rejects edits to migrations
  present on the base branch. An edited applied migration is **Blocking**; the correct shape is a
  new numbered migration, with a stale comment fixed in `data_dictionary.md` or
  `implementation_notes.md` instead.
- **Golden snapshots** — `cargo xtask snapshots` compares and reports; only `--update` rewrites the
  golden files. Never pass `--update` during a review. CI runs the gate over every fixture, not
  just the default one.
- **Seed order** — `database/seeds/001_reference_units.sql` must run before
  `002_lookup_seed_data.sql`, and mission-profile seeds stay at the point the comparison and
  read-model migrations require.
- **Database roles** — runtime ingestion uses the restricted role and must not gain schema,
  extension, table-drop, or role-administration privilege.
- **MSRV** — `rust-version = "1.85"`, but `rust-toolchain.toml` pins local to 1.97.1. A green local
  build is not MSRV evidence; the check is
  `cargo +1.85.0 check --workspace --all-targets --locked`.

## 6. Verification

`AGENTS.md` §Verification is the authority for which checks the changed surface requires. Run
non-mutating forms only:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
just check
cargo nextest run --workspace --locked          # needs Docker
cargo test --workspace --doc --locked
just generate-docs --check
just migrations-policy
just static
just lint
cargo xtask snapshots                            # never --update
cargo +1.85.0 check --workspace --all-targets --locked
cargo doc --workspace --all-features --no-deps --locked
```

Integration suites under `apps/ingest/tests/`, `apps/server/tests/`, and `crates/aircraft_db/tests/`
start disposable `postgres:16-alpine` containers and need Docker.

Report each as **Passed**, **Failed**, **Not run**, **Blocked by environment**, or **Not
applicable**. A check that could not run leaves its criterion **Unverified**; missing verification
is never success, and no check may be reported as passing unless it actually ran. Distinguish a
pre-existing branch failure from a regression caused by the change, and say which.

## 7. Findings

One finding per **root cause**, not per violated authority. A defect breaking both a criterion and
a binding rule is one Blocking finding that cites the criterion first and the rule second. One
cause breaking several criteria is one finding when one remedy fixes them all. Split only when the
causes or the remedies are materially independent.

- **Blocking** — any violated criterion, a failed required check, a contradiction with binding
  architecture, an edited applied migration, unsoundness, a secret or diagnostic leak, or a
  concurrency, durability, or merge-result defect.
- **Major** — correctness, boundary, transaction, protocol, visibility, governance, or
  over-engineering defect outside the criteria set.
- **Minor** — localized docs, structure, comments, test placement, idiom, unrelated churn.
- **Clippy** — only with the concrete `clippy::` lint named.

Do not inflate severity, and do not report a hypothetical without evidence.

## 8. Report shape

Findings first — no preamble, no praise, no narration of process. Then:

- **Criteria** — every one, graded, with a one-line reason. Name the failed obligation on a
  `Partially satisfied`.
- **What is correct and should stay** — the scoped work a remediation must not undo. Only what is
  actually right; no manufactured praise.
- **Unrelated churn** — only when present.
- **Verification** — what was inspected and what ran, separating head state from merge-result
  state, and local from CI. Name every check not run and why.
- **Conclusion** — exactly one of **Ready to merge**, **Ready after minor corrections**, **Not
  ready to merge**, **Cannot determine from available evidence**. Any violated criterion forces
  *Not ready to merge*.

Finding IDs are stable: `B1`, `M1`, `m1`. Say the boundary, the file, the invariant, and the
smallest correct remedy — "enforce the transition before durable replacement", not "consider
improving validation".

Hand the report to `rust-remediation` to act on. This skill produces findings; it does not fix
them.
