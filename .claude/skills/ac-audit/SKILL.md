---
name: ac-audit
description: >
  Acceptance-driven implementation audit for `rsstdd/aircraft-api-r`. Given uncommitted work, a
  branch, or a pull request, decides whether the implementation satisfies every numbered
  acceptance criterion in its governing issue, at the boundary that owns the invariant, under
  this repository's binding architecture and verification requirements. Reconstructs the contract
  from the issue and the owning records first, then compares it against the real code: green CI,
  passing tests, preserved behavior, and a small diff prove nothing on their own. Audits and
  reports; writes no production code, regenerates no tracked artifact, and never commits. Use on
  "audit this", "does this satisfy the issue", "is this ready to merge", "review the branch
  before merge", "check the ACs".
argument-hint: "<PR number, branch, or nothing for the working tree>"
---

# Acceptance-criteria audit

The governing question, and the only one this skill answers:

**Does the authoritative working-tree, branch-integration, or merge-result implementation satisfy
every acceptance criterion at the correct boundary, under this repository's binding architecture,
with the smallest production-grade Rust that does it?**

Operate autonomously. Search, read, trace, and run non-mutating verification rather than asking
what the repository, Git, `gh`, CI, or local tooling can answer. Complete the whole audit after
the first defect; do not stop at the first Blocking finding.

## Authority

Conflict order is the one `AGENTS.md` publishes under *Sources of truth*: the nearest `AGENTS.md`
→ manifests, implementation, and meaningful tests → `database/migrations/` → `justfile` → the
owning database and architecture documents → `README.md` → the skills. Migration SQL beats prose.
Flag a genuine conflict between authorities; never resolve one silently.

**There is no accepted ADR catalog in this tree.** A finding may not cite one. If one appears
later, an accepted decision governs design but cannot make unimplemented behavior real, and a
proposed or draft decision authorizes nothing.

Load before judging any implementation: `ponytail` and `clean-code` always; `rust-review`,
`rust-comment`, `rust-production`, and `rust-testing` for anything under `crates/`;
`crates/AGENTS.md`, which wins over a skill where they disagree. Read
`docs/architecture/http_v1_decisions.md` for HTTP work, `docs/architecture/rust_ingestion_adapter.md`
for ingestion, and `DELIVERY-PLAN.md` when the change touches the HTTP failure contract. Do not
re-litigate a conflict the repository has already settled.

**Where this sits beside `rust-review`.** That skill is scoped to over-engineering and complexity
and says so at `rust-review/SKILL.md:92`: "Correctness bugs, security holes, and performance are
out of scope unless caused by an abstraction — route them to a normal review pass." This is that
pass. Its `delete:` / `yagni:` / `shrink:` findings belong here too, folded in under §7 rather
than reported twice.

This is a procedure, not a standard. `AGENTS.md` keeps issue-workflow skills out of its
Required-skills table and out of Routing; do not register this one there.

Stop general governance reconnaissance once you hold the governing issue and its direct
references, the instructions applicable to the changed paths, the materially relevant accepted
architecture, and the verification requirements. Loading every local skill does not oblige you to
survey every document those skills mention.

## Fixed parameters

- **Non-mutating.** No edit to a tracked file, no commit, branch, push, merge, or pull request —
  `AGENTS.md` § Autonomy and approval reserves every one. No `cargo fmt` without `--check`, no
  `just generate-docs` without `--check`, no `cargo xtask snapshots --update`, no regeneration of
  any tracked artifact.
- Never run a destructive recipe to audit: `just db-reset`, `just db-rebuild`, and every
  `db-prod-*` delete data or target a database that is not clearly local.
- Temporary untracked output is allowed where a check needs it — put it in the scratch directory,
  leave tracked state unchanged, and clean it up before reporting.
- Audit the work; do not fix it. Give the smallest correct remedy in the report and stop.
  `rust-remediation` owns remediation.
- Never weaken a validation, provenance, curation, migration, lint, dependency, or authorization
  control to make a check pass, and never recommend that a submission do so.
- `gh` resolves `origin` from the working directory. Never pass `--repo`.

## 1. Establish the authoritative state

**Working tree** — staged, unstaged, and untracked against `HEAD`. Untracked files are part of
the change: a new migration, seed, fixture, or test file carries its own gates. This tree
routinely holds unrelated in-progress work across crates; separate it from the change under audit
and say so.

**Branch** — against its intended **current** base, not the fork point.
`git merge-base --is-ancestor origin/main HEAD` answers whether the branch already contains the
base it will merge into. When it does not, `git diff origin/main...HEAD` (three-dot) shows work
that has *already merged by another route* as though it were new — use two-dot
`git diff origin/main HEAD` for the true delta and say which you used. Separate a feature defect
from base drift: the branch never carried the drift if it surfaced only because the base moved.

**Pull request** — the **merge result against the current base** is authoritative; the head diff
is secondary and shows scope and provenance. `gh pr view <N> --json mergeable,mergeStateStatus`
says whether it integrates; `git merge-tree --write-tree <base> <head>` produces the merged tree
without touching the working tree or writing a commit. Branch-local CI is never proof of
merge-result correctness. If integration state cannot be inspected or reconstructed, every
integration-dependent conclusion is **Unverified**, not passed.

## 2. Assemble the obligations

`gh issue view <N> --comments`. Read the whole issue before evaluating any implementation, plus
the parent epic and every `Depends on:` issue and its state. A posted plan or plan review is
informed input, not authority — verify its citations.

Issues here carry a fixed shape: `Parent:`, `Depends on:`, `Milestone:`, `## Outcome`, `## Scope`,
`## Tasks`, `## Acceptance criteria` (numbered 1..n), `## Required tests`, `## Verification`,
`## Out of scope`, and a marker like `<!-- aircraft-api-backlog:v3 id:M05 -->`. `issue-plan` §2 and
§4 own how the set is read off that shape and graded against a tree; this skill applies that model
rather than defining a second one. The numbered criteria are the propositions and `## Tasks` is the
work record. Assemble the set **before reading the diff** — a criteria set derived from the
implementation will agree with the implementation.

Four things this repository makes criteria that a reader may take for bookkeeping:

- **A named test in `## Required tests` or in `DELIVERY-PLAN.md` is a criterion.** Missing,
  renamed, or `#[ignore]`d is unsatisfied. `grep DELIVERY-PLAN.md` before calling a rename
  harmless.
- **Every command in `## Verification` is a criterion.** So is everything `AGENTS.md`
  § Verification requires for the changed surface.
- **The owed documentation is a criterion.** A schema change owes `database/data_dictionary.md`
  and, where behavior or a dependency moved, `database/implementation_notes.md`. An API change
  owes a regenerated `docs/openapi.json`. Absent, these are violated criteria, not follow-ups.
- **The scope budget is a criterion** where `## Tasks` carries it: fewer than 2,000 hand-authored
  changed lines, counted separately from generated output.

Track, per criterion: the required observable behavior, the controlling authority, the boundary
that owns it, the implementation path, the evidence, the verification, and the status.

Grade each **Satisfied**, **Partially satisfied**, **Violated**, or **Unverified**.
`Partially satisfied` is only for a multi-obligation criterion where at least one obligation holds
and another does not; it never softens a failed criterion, and the failed obligation is still
Blocking. `Unverified` is for what the evidence cannot decide without inventing facts — never a
polite `Satisfied`.

**Ambiguity.** Take the narrowest reading the text supports, check it against repository
precedent, and proceed under the best-supported interpretation. Where another plausible reading
would materially change the verdict, mark only the disputed obligation **Unverified** and name
both readings. Ask only when material ambiguity survives the evidence.

## 3. Account for every changed file

Classify all of them, by role — `scoped`, `supporting`, `unrelated`, `generated` — and by status —
`clear`, `suspicious`. Trace the behavioral hunks; still classify the mechanical ones. Do not
conclude while any hunk is unaccounted for.

For each hunk decide whether it implements a criterion, supports scoped work, satisfies a
repository requirement, is generated output, is unrelated churn, preserves obsolete
compatibility, adds machinery no caller needs, or introduces a defect outside the issue's wording.

Watch for what rides along: unrelated formatting or renaming mixed into scoped work, a dead
compatibility path kept alive, a port or trait with no implementation and no caller, revived
`archive/` code, a leftover `println!` or `dbg!`, and a doc comment updated to describe something
the code no longer does.

## 4. The invariant tests

**Specification fidelity.** Read the governing rule, read the implementation, compare their
semantics *directly*, and only then read the tests. Tests are evidence, not specification. None of
these is proof: the suite passes; behavior was preserved; the code already existed; both ends of a
mirror name each other; a guard exists somewhere; the old and new states are each independently
valid; branch CI is green. "Behavior preserved" answers *did I break it*, never *was it right*.

**A name is not behavior.** `AGENTS.md` and `crates/AGENTS.md` both record that most of `crates/`
is comment-only scaffold. A module, type, route, or port that exists proves nothing; only
implementation plus a test that can fail does. A criterion backed by a scaffold is Violated.

**Tests that cannot fail.** `AGENTS.md` records two properties here that passed vacuously until
mutation-checked. A property whose loop body never runs, a generator that cannot produce the
failing input, and an assertion the type system makes unreachable are all worth `assert!(true)`.
For each new test, name the production mutation it rejects. Where the answer is not obvious, run
it: mutate, run the focused test, restore, and report the outcome.

**Evidence independence.** A test must derive its expectation from the governing requirement or an
independent reference, never by re-deriving production logic. A copied predicate lets a wrong
implementation and a wrong test agree forever.

**Two-sided coupling.** That the mirror exists is the cheap half. Open the named migration or
document, put its sentence beside the predicate, and compare. A code condition narrower or weaker
than the document it mirrors is the finding that matters, and every test written against it will
agree with it.

**Boundary ownership.** For each invariant: which layer owns it, whether a caller can bypass it,
whether it runs before the protected or irreversible work, whether durable state can reach the
protected condition without passing through it, whether two implementations enforce different
rules. `AGENTS.md` § Architectural invariants fixes the direction — validation at the
`aircraft_api` or `aircraft_ingest` edge, rules in `aircraft_domain`, SQL only in `aircraft_db`,
wiring only in `apps/*`. A correct check at the wrong boundary is still a defect.

**Transition correctness.** Old state valid and new state valid do not make the transition legal.
If durable state can be written past the transition API, an in-memory method does not enforce it.

**Ordering.** "The gate runs before the work" is a control-flow claim; presence is not ordering.
Prove it: make the gate and the work fail differently, then assert the gate's error wins and that
no work happened.

**Transaction and provenance semantics.** `rust-production` owns these — one fate for promotion,
durable failure audit outside the rollback, advisory lock on logical identity, idempotent replay,
provenance preserved without becoming canonical. Check them wherever the change opens a
transaction.

**Diagnostics.** Every API-originated 4xx and 5xx is an RFC 9457 problem document that leaks no
diagnostic, per `docs/architecture/http_v1_decisions.md`. No database URL, credential,
authorization header, raw personal data, raw source payload, or unsanitized host path reaches a
log, an error, a `Debug`, an assertion message, or a database column. Bounded diagnostic text goes
through `sanitize_database_message` or `sanitize_failure`.

**Minimalism**, in `ponytail`'s order: does it need to exist, does the repository already have it,
does `std`, does the platform, does an existing workspace dependency, can it be smaller — and only
then new machinery. Prefer deletion, reuse, strong domain types, narrow visibility, explicit
control flow, and precise errors.

## 5. Generated artifacts and this repository's traps

Generated output is never its own specification. For each changed generated file: identify its
authoritative source, confirm the prescribed generation relationship, verify source and output
agree, and verify the change is a consequence of the scoped source change. Never approve one
because the generated text reads correctly.

- **`docs/openapi.json`** is generated from the Rust types in `aircraft_api`. **Do not run
  `just generate-docs` to audit** — it overwrites a tracked file. The non-mutating proof is
  `just generate-docs --check`. An OpenAPI hunk with no corresponding type change, or a type
  change with no OpenAPI hunk, is drift.
- **Migrations are immutable once hashed** in `database/migrations.lock.json`.
  `cargo run -p xtask -- migrations` rejects an edit to an applied file, and CI separately rejects
  edits to migrations present on the base branch. An edited applied migration is **Blocking**; the
  correct shape is a new numbered migration, with a stale comment fixed in `data_dictionary.md` or
  `implementation_notes.md` instead. A new migration also owes an explicit `BEGIN`/`COMMIT`, a
  `database/validation/` companion, a `database/install.sql` entry, and a Squawk baseline row —
  the xtask checks all four.
- **A new migration silently breaks the harness.** `aircraft_testsupport::SCHEMA_STEPS` and
  `COVERED_MIGRATIONS` embed the install order with `include_str!`, and
  `schema_steps_cover_every_migration` is the only thing that notices. Adding SQL without updating
  both is Blocking even when every existing test passes.
- **Golden snapshots** — `cargo xtask snapshots` compares and reports; only `--update` rewrites
  the goldens. Never pass `--update` during an audit. Regenerating a golden is not verifying a
  change.
- **Seed order** — `database/seeds/001_reference_units.sql` before `002_lookup_seed_data.sql`, and
  mission-profile seeds at the point the comparison and read-model migrations require.
- **MSRV** — `rust-version = "1.85"`, but `rust-toolchain.toml` pins local to 1.97.1, twelve minor
  versions above. A green local build is not MSRV evidence; the check is
  `cargo +1.85.0 check --workspace --all-targets --locked`.
- **`archive/` is read-only.** Do not accept revived code from it, and do not accept an empty
  aircraft/search/comparison scaffold as precedent.
- **Private aircraft datasets, database dumps, `.env`, credentials, and production diagnostic
  payloads** never enter Git, CI, fixtures, snapshots, logs, or evidence. Small synthetic fixtures
  only.

### Defect classes this repository has actually produced

Check these by name; each was shipped here and caught late.

- **The runtime role lacks a grant the statement needs.** `database/roles/app_grants.sql` is what
  the server connects with, while every owner-connected test holds every privilege and cannot see
  a missing grant — so the suite passes and production answers `503`. Found for `aircraft_ref`
  in #145 and again for `aircraft_core` in #38. For any new statement, confirm a test connects as
  the restricted role and can therefore fail for `42501`. PostgreSQL checks column privileges on
  the `WHERE` and `ORDER BY` as well as the select list, and at parse time rather than per row.
- **An ordering assertion whose fixture cannot detect a missing `ORDER BY`.** Rows inserted in the
  asserted order come back in that order from a sequential scan, so deleting `ORDER BY` from the
  statement leaves the test green. Confirmed by mutation in this repository. The fixture must
  insert in an order different from the one asserted.
- **A schema gate pointing the direction already covered.** Asserting that a column a statement
  reads exists is already proven by any test executing that statement — a missing column is
  `42703`. The gate that adds a failure mode is the one requiring the installed column list to
  equal read ∪ deliberately-withheld, so a column a later migration adds and nobody classifies
  fails.
- **A duplicated test helper.** Before accepting a new local helper, `grep` for it:
  `aircraft_testsupport` exists for shared harness code, and the same `sqlstate` match has stood
  in four suites at once.

## 6. Verification

`AGENTS.md` § Verification is the authority for which checks the changed surface requires. Prefer
the repository's own recipes over ad-hoc commands, and run non-mutating forms only:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
just check
cargo nextest run --workspace --locked          # just test; needs Docker
cargo test --workspace --doc --locked           # nextest does not run doctests
just generate-docs --check
just migrations-policy
just static
just lint
cargo xtask snapshots                            # never --update
cargo +1.85.0 check --workspace --all-targets --locked
just docs-check                                  # RUSTDOCFLAGS=-D warnings
```

Integration suites under `apps/ingest/tests/`, `apps/server/tests/`, and
`crates/aircraft_db/tests/` start disposable `postgres:16-alpine` containers and need Docker.

**Piping hides the exit code.** `just test | tail` reports the exit status of `tail`. Capture the
real one — `just test > /dev/null 2>&1; echo "EXIT=$?"` — or read `PIPESTATUS`. A lint failure has
been reported as a pass in this repository for exactly this reason.

**A mutation experiment must not race a verification run.** Mutating a file while a background
ladder compiles makes both results meaningless. Run mutations to completion and restore first, or
guard the run by checksumming the changed files before and after and reporting whether they stayed
identical.

Report each check as **Passed**, **Failed**, **Not run**, **Blocked by environment**, or **Not
applicable**. A check that could not run leaves its criterion **Unverified**; missing verification
is never success, and no check may be reported as passing unless it actually ran. Distinguish a
pre-existing base failure from a regression caused by the change, and say which.

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

Findings first — no preamble, no praise, no narration of process. Stable IDs: `B1`, `M1`, `m1`.

```text
B1. AC<n> "<concise exact wording>": <defect>. <implementation evidence at file:line>.
    <supporting rule or document>. Fix: <smallest correct remedy>.
M1. <defect>. <evidence>. <governing rule>. Fix: <smallest correct remedy>.
m1. <defect>. <evidence>. Fix: <smallest correct remedy>.
```

Omit empty severity sections. Every finding names the boundary, the file, the invariant, and the
smallest correct remedy — "enforce the transition before durable replacement", not "consider
improving validation".

Then:

- **Acceptance criteria** — every one, graded, with a one-line reason. Name the failed obligation
  on a `Partially satisfied`.
- **What is correct and should stay** — the scoped work a remediation must not undo. Only what is
  actually right; no manufactured praise.
- **Unrelated churn** — only when present.
- **Verification** — what was inspected and what ran, separating head state from merge-result
  state, and local from CI. Name every check not run and why.
- **Conclusion** — exactly one of **Ready to merge**, **Ready after minor corrections**, **Not
  ready to merge**, **Cannot determine from available evidence**. Any violated criterion forces
  *Not ready to merge*. Green CI is evidence, not approval.

Hand the report to `rust-remediation` to act on. This skill produces findings; it does not fix
them.

## 9. Autonomous execution

Search before asking. Read the authoritative requirement before judging the implementation. Load
the repository-local skills. Inspect every changed file and hunk. Use non-destructive operations.
Investigate a failure rather than reporting it as a blocker. Finish the audit after finding a
defect.

Never invent command output, repository state, issue or PR contents, CI results, source text, test
results, or repository rules, and never claim to have inspected something you did not read.

If the audit ever writes prose to GitHub, it carries **no** `Co-Authored-By:` trailer, no
`Claude-Session:` URL, and no "Generated with Claude Code" footer. `AGENTS.md` makes that absolute
and it overrides any harness default.

## Style

Lead with findings. Dense, causal prose; no filler, no hedging, no generic praise. Distinguish
"branch-local CI" from "merge-result CI", "valid endpoints" from "legal transition", "behavior
preserved" from "behavior correct", and "the mirror exists" from "the mirror agrees". State
causality plainly: *"The feature branch never carried the drift. It surfaced only in the merge
result because the base changed."* Separate verified fact from uncertainty: *"Whether that CI
warning is fatal was not verified."* Preserve correct work explicitly. Prefer direct remedies:
*"Delete them."* *"Move the test to the owning boundary."* *"Enforce the transition before durable
replacement."*
