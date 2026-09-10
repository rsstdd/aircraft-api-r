# Repository skills

Twelve skills live here, one directory each, all invocable as `/<name>`. This file routes: it says
which skill loads when and whether it binds. Each `SKILL.md` frontmatter stays authoritative for
what that skill covers, so nothing here restates a skill's content and nothing here can drift from
it.

[`AGENTS.md`](../../AGENTS.md#required-skills) §Required skills is the binding table and wins over
this index. `CLAUDE.md` carries no copy by design: it is compatibility-only and forbids duplicating
repository-wide instructions, so it inherits the table through `@AGENTS.md`.

## Binding standards — load before the first edit, not after

| Skill | Loads when | Owns |
|---|---|---|
| [`clean-code`](clean-code/SKILL.md) | Any code, any language | Clean Code as applied here, and the settled conflicts against repo convention |
| [`ponytail`](ponytail/SKILL.md) | Any code, and before auditing or editing existing Rust | Whether the code should exist at all: YAGNI, reuse, stdlib before a dependency |
| [`rust-review`](rust-review/SKILL.md) | Any Rust, written or reviewed | The severity scale the code is judged against, and what to delete |
| [`rust-comment`](rust-comment/SKILL.md) | Any Rust | Why-not-what prose, rustdoc sections, two-sided coupling comments, debt markers |
| [`rust-production`](rust-production/SKILL.md) | Rust that admits untrusted input, opens a transaction, computes an identity, or changes a published format or migration | Untrusted-input capture, two-pass consistency, transaction ownership and durable audit, provenance without canonical status, durable contracts, schema evolution |
| [`rust-testing`](rust-testing/SKILL.md) | Any Rust test, which under TDD is nearly every Rust change | TDD order, behavior-sentence naming with no tier prefixes, test placement, the disposable PostgreSQL harness, deterministic setup, proving a new test can fail |

These are standards, not advice. They are not architectural authority: the conflict order is the
nearest `AGENTS.md` → manifests, implementation, and meaningful tests → `database/migrations/` →
`justfile` → the owning database and architecture documents → `README.md` → these skills. Where a
skill and a nested `AGENTS.md` disagree, the `AGENTS.md` wins. Flag a genuine conflict; never
resolve one silently.

There is no accepted ADR catalog in this tree. If one is introduced, an accepted decision can
govern design but cannot make unimplemented behavior real, and it enters the order above
`AGENTS.md`.

## Issue workflow — invoked explicitly, one per stage

| Skill | Invoke when | Produces | Touches code |
|---|---|---|---|
| [`issue-plan`](issue-plan/SKILL.md) | A story needs a plan before work starts | An implementation plan posted as an issue comment | No |
| [`issue-plan-review`](issue-plan-review/SKILL.md) | A plan comment exists and needs auditing | A critique and amended plan posted as a follow-up comment | No |
| [`issue-implement`](issue-implement/SKILL.md) | The plan is settled and the story is being built | A verified, reviewed, minimal diff and an evidence report | Yes |
| [`ac-audit`](ac-audit/SKILL.md) | Work exists — a tree, a branch, or a PR — and must be judged against its issue before merge | Criteria graded Satisfied / Partially satisfied / Violated / Unverified, findings by root cause as `B1`/`M1`/`m1`, one merge verdict | No |
| [`rust-remediation`](rust-remediation/SKILL.md) | A review has produced findings against work governed by an issue | Root causes fixed at their owning boundary, every criterion re-graded, a completion gate | Yes |

They compose in that order and share one criteria model: `issue-plan` §2 owns how the criteria set
is read off the issue — the numbered `## Acceptance criteria`, plus `## Required tests` and
`## Verification` as binding obligations — and §4 owns grading each one against the tree. The other
four defer to that rather than restating it. All five load the binding standards above; none of
them commits, branches, pushes, merges, or opens a pull request.

`ac-audit` is the one that may be pointed at work it did not plan. It re-derives the criteria set
independently — a set read off the implementation agrees with the implementation — and it is the
correctness pass `rust-review` routes to, that skill being scoped to over-engineering alone
(`rust-review/SKILL.md:92`). It also carries this repository's catalogue of shipped defect classes:
a runtime role missing a schema grant, an ordering assertion whose fixture cannot detect a deleted
`ORDER BY`, a schema gate pointing the direction other tests already cover, and a helper duplicated
across suites that `aircraft_testsupport` already owns.

`rust-remediation` then closes the loop: it treats every finding as a hypothesis to verify before
editing, rejects the ones repository evidence disproves, and re-reviews the result against every
criterion rather than stopping when the findings list is empty.

## Procedures — emitted, never executed

| Skill | Invoke when | Produces | Touches code |
|---|---|---|---|
| [`commit-plan`](commit-plan/SKILL.md) | The tree holds finished work that needs grouping into commits | A commit plan and the exact `git add` / `git commit` commands, as text | No |

`commit-plan` never runs what it writes: committing, branching, and pushing each need the user's
approval under `AGENTS.md` §Autonomy and approval.

Two records sit outside the skills but inside the workflow: [`DELIVERY-PLAN.md`](../../DELIVERY-PLAN.md)
records what the plans on issue #33 promised for the HTTP failure contract and what shipped, and
`docs/architecture/http_v1_decisions.md` owns the contract itself. The planning skills read both
whenever an issue touches HTTP failure mapping.

## Adding a skill

Match the frontmatter `name` to the directory name, keep it as short as the six standards are, cite
the file in this tree that proves each rule rather than arguing it, and add a row above. A skill
that binds also needs a row in `AGENTS.md` §Required skills; a workflow skill does not, and is
deliberately absent from that table and from §Routing.

Skills carry no `license:` field. This workspace is `UNLICENSED` with no license text, so a license
line here would assert a grant the repository withholds; add one to all eleven only if the project
adopts an explicit license.
