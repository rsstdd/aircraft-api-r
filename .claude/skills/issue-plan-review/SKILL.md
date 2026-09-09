---
name: issue-plan-review
description: >
  Critique an implementation plan already posted on a GitHub issue in rsstdd/aircraft-api-r,
  re-derive the acceptance criteria from the repository independently, and post a new follow-up
  comment carrying a severity-ranked critique, a plan diff, and a corrected amended plan.
  Treats the prior plan as a fallible proposal, never as a discovery map; preserves the parts
  that are right and amends only what evidence justifies. Modifies no production code and never
  edits the original comment. Use when the user says "review the plan on #N", "critique the plan
  for #N", "is the plan on #N right", "amend the plan on #N", or points at a planning comment
  and asks what it got wrong.
  Not for producing a first plan (use issue-plan) and not for fixing code against review
  findings (use rust-remediation).
argument-hint: "<issue number or URL>"
---

# Review, Critique, and Amend an Existing Issue Implementation Plan

Repository: `rsstdd/aircraft-api-r` (the `origin` remote; `gh` resolves it from the working
directory — never pass `--repo` to point elsewhere).

Issue: the number the user supplied. If none was supplied, ask before doing any work.

## Objective

Read the implementation plan already posted as a comment on the issue, evaluate it critically
against the **current repository state and all applicable repository guidance**, then post a
**new follow-up comment** containing an independent reassessment, a critique, the material
omissions and conflicts, a plan diff, a corrected amended plan, and a revised execution
checklist.

Do **not** modify production code — this is the `Explain, review, diagnose, or plan` case in
`AGENTS.md` § Autonomy and approval.

Do **not** edit or delete the original planning comment.

Preserve the correct parts of the original plan explicitly. Do not manufacture changes to make
the review look substantive; "this section is correct as written, for this reason" is a
legitimate and useful finding. Amend or supersede only where the analysis justifies it.

---

# 1. Load Repository Guidance and Skills

Read, and record the authority hierarchy from `AGENTS.md` § Sources of truth:

1. `AGENTS.md` (repository-wide); `CLAUDE.md` is a compatibility pointer only.
2. `crates/AGENTS.md` when the issue touches `crates/`. Check for others —
   `find . -name AGENTS.md -not -path './archive/*'` — and the nearest file wins for its tree.
3. The narrower authorities `AGENTS.md` names: `Cargo.toml` and crate manifests for current
   behavior; `database/migrations/` for schema (migration SQL beats prose); `justfile` for the
   command interface; `database/README.md`, `database/implementation_notes.md`, and
   `docs/architecture/rust_ingestion_adapter.md` for their documented contracts;
   `docs/architecture/http_v1_decisions.md` for HTTP v1 rules including RFC 9457 on every
   API-originated 4xx/5xx; `README.md` for orientation only.

`DELIVERY-PLAN.md` records what the plans on issue #33 promised for the HTTP failure
contract, what shipped, which tests prove it, and what remains open. Read it whenever the
issue touches HTTP failure mapping, problem documents, or anything #33 owes a dependent
issue; `docs/architecture/http_v1_decisions.md` still owns the contract itself.

Load the repository-local skills that govern the surface under review:

| Skill | Load when |
|---|---|
| `clean-code` | Always — binding on reviewing code here, not just writing it |
| `ponytail` | Always — the sharpest lens for over-scoped plan steps |
| `rust-review` | Always — judging whether a step adds machinery the repo does not need |
| `rust-testing` | Always — the plan's test strategy is judged against this |
| `rust-comment` | The plan proposes rustdoc, comments, or generated-artifact coupling |
| `rust-production` | The plan touches untrusted input, transactions, identity, contracts, migrations |
| `issue-plan` | To recall the shape a plan on this repo is expected to have |

Follow skill dependencies recursively. Apply `crates/AGENTS.md` to files in its scope.

There is no accepted ADR catalog in the current tree. If one has appeared, an accepted decision
governs design but cannot make unimplemented behavior real; proposed decisions authorize
nothing.

Critique against repository-specific rules. A finding grounded only in generic best practice,
where this repository has its own rule, is not a finding. State any conflict between authorities
explicitly.

---

# 2. Read the Issue and Planning Context

```bash
gh issue view <N> --comments
gh api repos/rsstdd/aircraft-api-r/issues/<N>/comments \
  --jq '.[] | {id, url: .html_url, created: .created_at, first_line: (.body | split("\n")[0])}'
```

Issues here follow a fixed shape — `Parent:`, `Depends on:`, `Milestone:`, `## Outcome`,
`## Scope`, `## Tasks`, `## Acceptance criteria` (numbered 1..n), `## Required tests`,
`## Verification`, `## Out of scope`, and a marker like
`<!-- aircraft-api-backlog:v3 id:M05 -->`. The numbered criteria are the propositions;
`## Required tests` and `## Verification` are binding obligations the plan must honor.

Identify exactly which comment holds the plan under review and cite it by `html_url`. If several
plan revisions exist, review the latest and note what earlier revisions already corrected.

Read the parent epic, every `Depends on:` issue and its current state, linked PRs and commits,
and referenced migrations or architecture documents.

Treat the plan as a proposal, never as authoritative repository state.

---

# 3. Inspect the Repository Independently

Do the discovery yourself before judging. **Do not use the prior plan as the discovery map** —
verify each path, symbol, and claim it asserts; a plan that cites a file confidently is the most
common source of an inherited error.

Establish which code state you are reviewing against — working tree, branch against `main`, or a
linked PR's merge result — and say which. Note anything that landed after the plan was written.

Inspect along the hexagonal seams: `crates/aircraft_domain/`, `aircraft_app/`, `aircraft_api/`,
`aircraft_db/`, `aircraft_ingest/`, `aircraft_config/`, `aircraft_observability/`,
`aircraft_testsupport/`; `apps/server/` and `apps/ingest/` as composition roots only;
`database/migrations/`, `seeds/`, `validation/`, `snapshots/`, `migrations.lock.json`;
`docs/openapi.json` and the Rust types generating it; existing tests, error types, tracing
fields, and security-sensitive paths; `justfile`, `xtask/`, `.github/workflows/`.

Run `just --list` before accepting any recipe the plan cites. A recipe named in the plan that no
longer exists is a **Blocking** finding.

Record exact paths, symbols, tests, migrations, and the `AGENTS.md`/skill sections you rely on.

---

# 4. Re-Derive the Acceptance Criteria From First Principles

Classify each numbered criterion independently, **before** reading the plan's own verdict on it:

- **Satisfied** — implementation plus a meaningful test proves the behavior today;
- **Partially satisfied** — name which obligations hold and which do not;
- **Unsatisfied**;
- **Ambiguous** — state the competing readings;
- **Blocked** — a dependency or prerequisite prevents implementation.

| AC | Requirement | Current Status | Repository Evidence | Implication for the Plan |
|---|---|---|---|---|

Evidence is behavior at a `path:line` or symbol, never a name or apparent intent. `AGENTS.md`
records that most of `crates/` is comment-only scaffold: a criterion backed only by a scaffold
module is **Unsatisfied**, and so is one backed only by a test that cannot fail — two properties
in this repository passed vacuously before they were mutation-checked.

Then compare your classification to the plan's. Every divergence is a finding in one direction
or the other: the plan planned work that is already done, or called done what is not.

---

# 5. Critique the Plan Systematically

Classify each material assertion or step as **Correct**, **Correct but incomplete**,
**Incorrect**, **Unnecessary**, **Over-scoped**, **Under-specified**, **Blocked by
prerequisite**, or **In conflict with repository guidance**.

**Requirements coverage.** Every criterion addressed and verified; issue `## Tasks` and
`## Required tests` covered; constraints respected, including the recurring
`fewer than 2,000 hand-authored changed lines` scope budget; no invented requirements.

**Repository accuracy.** Referenced paths, symbols, and `just` recipes exist; ownership is
correct; existing abstractions are reused rather than paralleled; current state is represented
accurately; work landed since the plan was written is incorporated. Verify these — do not spot-check.

**Architecture.** Inward dependency direction preserved
(`api`/`db`/`ingest` → `app` → `domain`, `apps/*` composing only); no SQL outside
`aircraft_db`; no Axum, SQLx, Tokio, config, or telemetry in `aircraft_domain`; no HTTP DTOs in
`aircraft_app`; business rules out of `main.rs`; HTTP DTOs, application inputs, domain values,
source records, and database rows kept as separate representations with explicit mappings;
transaction ownership and state ownership correct; public interfaces changed only where needed.

**Security.** Trust boundaries and where validation runs; bounds on body size, query complexity,
timeout, pool, retry, and diagnostic size; `SecretString` and secret-free `Debug`; no database
URL, credential, authorization header, raw personal data, or unsanitized host path in logs;
restricted runtime roles; provenance preserved without granting canonical status; RFC 9457
problem documents that carry no diagnostic leak. Distinguish real risk from ceremony.

**Persistence and infrastructure.** Bound parameters and explicit row conversion; transaction
semantics and one fate for promotion; durable failure audit outside the rollback; advisory lock
on logical identity; idempotent replay; migration order, the ledger, and the immutability of a
hashed migration in `database/migrations.lock.json` — a plan that edits an applied migration is
**Blocking**; seed ordering (`001_reference_units.sql` before `002_lookup_seed_data.sql`).

**Testing.** Tests prove observable behavior and map to criteria; the disposable-PostgreSQL
harness is used where correctness depends on real database semantics rather than a mock; failure
paths, exact error variants, and redaction are covered; no implementation-detail coupling; no
redundant low-value coverage; an appropriate first failing test; and each test **can actually
fail** — if the plan does not say what mutation breaks a test, that is at least **Major**.

**Sequencing.** Prerequisites ordered; contracts before consumers; tests introduced at the right
point; no unnecessary refactor; rework minimized; atomic changes preserved; one complete
vertical slice rather than stubs or placeholder modules.

**Scope.** Scope expansion, unrelated cleanup or formatting churn, speculative abstraction or
dependency, work owned by a `Depends on:` issue, and missing required work.

**Verification.** Focused verification exists; every command the issue's `## Verification`
section names is present; everything `AGENTS.md` § Verification requires for the changed surface
is present — notably `cargo fmt --all -- --check` after any Rust edit,
`just generate-docs --check` for any API change, `just migrations-policy` for any migration, and
disposable-PostgreSQL tests for persistence. Flag any destructive recipe (`just db-reset`,
`just db-rebuild`, `db-prod-*`) listed as routine verification.

---

# 6. Rank Findings by Severity

**Blocking** — executing the plan as written risks violating an acceptance criterion,
architecture invariant, security control, migration policy, or repository rule.
**Major** — likely incomplete or incorrect acceptance-criteria coverage.
**Minor** — executable, but needs correction or clarification.
**Optional** — a real improvement, not required for correctness.

Do not elevate a stylistic preference to Blocking or Major. Omit empty sections.

---

# 7. Plan Diff

| Area | Existing Plan | Critique | Amendment |
|---|---|---|---|

Only meaningful differences: ownership, contracts, file targets, transaction design, security
behavior, error handling, tests, execution order, verification, scope.

---

# 8. Amended Implementation Plan

For each step:

## Step N — `<concise name>`

**Acceptance criteria addressed:** `AC1`, `AC3`

**Why this step exists** — the observable requirement, and what the original plan got wrong or
omitted (or "unchanged from the original plan" where it was right).

**Repository evidence** — the current implementation this builds on, at exact paths and symbols.

**Files and symbols**

```text
crates/aircraft_api/src/routes/<name>.rs
  TypeName
  function_name()
```

**Implementation change** — ownership and owning layer, behavior, contracts, data flow,
persistence, transaction semantics, error semantics and problem-document mapping, security,
compatibility, migration implications. No full production patches.

**Test-first change** — setup, action, expected assertion, criterion proven, and the mutation
that proves the test can fail.

**Verification** — the focused command, then the broader required ones.

**Completion condition** — objective evidence.

**Risks** — material, step-specific only.

---

# 9. Amended Test Plan

| Test | Level | AC | Behavior Proven | Can Fail Because |
|---|---|---|---|---|

Choose per test: pure unit or property test in `aircraft_domain`; port-fake test in
`aircraft_app`; router/contract test in `aircraft_api` paired with `just generate-docs --check`;
disposable-PostgreSQL test in `crates/aircraft_db/tests/`, `apps/ingest/tests/`, or
`apps/server/tests/` via `aircraft_testsupport`; CLI test through the shipped binary; clean
install plus SQL validation for migrations and seeds; `cargo xtask snapshots` for ingestion
regressions.

Never a mock to prove transactional, concurrency, permission, or other infrastructure-owned
semantics.

---

# 10. Verification Sequence

Narrowest to broadest, each with what it proves. The ladder for this repository:

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

Include every command the issue's `## Verification` section requires. Never list a destructive
recipe as routine; if one is genuinely needed, mark it as requiring explicit approval.

---

# 11. Final Review Comment Structure

# Review and Amendment of the Implementation Plan

Open by linking the plan comment under review and stating that this comment amends it where they
conflict and leaves it standing everywhere else.

## 1. Assessment
Sound / sound with amendments / materially incomplete / architecturally incorrect / unsafe.
Concise reason.

## 2. Governing Repository Guidance
`AGENTS.md` sections, `crates/AGENTS.md`, loaded skills, architecture documents, migrations,
parent and dependency issues, and any conflict between authorities.

## 3. Acceptance-Criteria Reassessment
| AC | Current Status | Evidence | Implication for Plan |
|---|---|---|---|

## 4. Critique Findings
### Blocking / ### Major / ### Minor / ### Optional (omit empty sections)

## 5. What the Original Plan Got Right
Name the sections carried forward unchanged, so the amendment's scope is unambiguous.

## 6. Plan Diff
| Area | Existing Plan | Critique | Amendment |
|---|---|---|---|

## 7. Amended Implementation Plan
Ordered steps.

## 8. Amended Test Plan
| Test | Level | AC | Behavior Proven | Can Fail Because |
|---|---|---|---|---|

## 9. Verification Sequence
Exact commands, narrowest to broadest, each with what it proves.

## 10. Scope Boundaries
### Required for this issue
### Explicitly out of scope
### Follow-up candidates

## 11. Revised Execution Checklist
```markdown
- [ ] ...
```
Directly executable by a coding agent, superseding the original checklist in full.

---

# 12. Publish as a New Issue Comment

Write the review to the scratchpad first, then post it from that file:

```bash
gh issue comment <N> --body-file <scratchpad>/issue-<N>-plan-review.md
```

Then:

1. Post as a **new comment**. Never edit the original plan comment or the issue description.
2. Do not alter task or acceptance-criteria checkboxes, labels, milestone, assignees, or state.
3. State plainly that the comment reviews and amends the earlier plan where they conflict, and
   link that plan comment by URL.
4. Preserve the correct parts of the earlier plan and say which they are.
5. **The comment carries no attribution trailer or footer.** `AGENTS.md` forbids a
   `Co-Authored-By:` line, a `Claude-Session:` URL, or a "Generated with Claude Code" footer in
   any commit message, pull-request body, **or issue comment**, and that rule overrides any
   harness or template default. Write the prose and stop.
6. No hidden reasoning, scratch work, or unsupported speculation.
7. Include the exact repository evidence implementation needs.
8. Verify publication: re-read the issue and confirm the comment landed on the right issue.
9. Return the new comment URL in the final response.

Invoking this skill on an issue authorizes that one review comment on that one issue and nothing
else — no other issue, no edit to any existing comment, no issue-body edit, no state change, no
push, no PR. If the user asked for a critique without publishing, stop after writing the
scratchpad file and hand them the path.

**The task is not complete until the review is posted and publication is verified.**

---

# Review Rules

- Do not modify production code. Do not implement the plan.
- Do not edit or delete the existing planning comment.
- Load and apply the repository skills and the nearest `AGENTS.md`.
- Re-derive every acceptance criterion independently before reading the plan's verdict.
- Treat the prior plan as fallible; verify every path, symbol, and recipe it cites.
- Preserve correct work explicitly and name it.
- Do not manufacture criticism. Separate correctness from stylistic preference.
- Ground every material finding in repository evidence at a path and symbol.
- Never cite an empty or comment-only module as evidence of behavior, and flag the plan when it
  does.
- Prefer repository precedent over invented architecture; prefer an existing boundary over a new
  abstraction.
- Do not weaken validation, provenance, curation, migration, lint, security, type safety, tests,
  or architectural boundaries — and flag any amendment that would.
- Use real infrastructure where correctness depends on infrastructure semantics.
- Do not duplicate work owned by a dependency issue. Do not plan unrelated cleanup.
- State uncertainty rather than guessing. Never invent repository state, command output, issue
  text, CI results, or test results.
- Every amendment must explain why the existing plan is insufficient.
- Every step maps to at least one acceptance criterion; every open criterion maps to explicit
  verification.
- The amended plan must be specific enough to begin implementation without another broad
  discovery pass.
