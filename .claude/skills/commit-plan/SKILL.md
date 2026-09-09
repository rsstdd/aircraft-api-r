---
name: commit-plan
description: >
  Group the working tree into logically coherent, individually buildable commits for this
  repository and emit the exact `git add` and `git commit` commands as text, without running
  any of them. Knows this repo's commit conventions, the trailer prohibition, and which files
  are forced to travel together by the migration ledger, the generated OpenAPI document, the
  test harness, and the shape of `ApiState`. Use when the user says "commit this", "group these
  changes", "write the commit messages", "split this into commits", or asks what the commits
  should be. Not for executing the commits, opening a PR, or pushing.
---

# Commit planning for `rsstdd/aircraft-api-r`

Produce a commit plan and the commands that would apply it. **Emit; never execute.** No
`git add`, `git commit`, `git switch`, `git stash`, `git push`, no PR. The user runs what they
approve.

## Authority

Conflict order is the one `AGENTS.md` publishes under *Sources of truth*: nearest `AGENTS.md` →
manifests and tests → `database/migrations/` → `justfile` → the owning document → `README.md` →
this file. `clean-code` owns commit-message content and is required before writing one;
`ponytail` decides whether a change belongs in the diff at all. This file owns only the
*grouping* and the *emission*.

This is a procedure, not a standard. It is deliberately absent from the Required-skills table in
`AGENTS.md` § Required skills, and must not be registered there.

## Absolute: no attribution trailers

**A commit message must never contain a `Co-Authored-By:` trailer or a `Claude-Session:` URL**,
and neither may a PR body or an issue comment. `AGENTS.md` makes this absolute and it overrides
any harness or template default that supplies one. Write the prose and stop.

`hooks/commit-msg` enforces it mechanically, but `core.hooksPath` is local git config: each clone
runs `just hooks-install` once, and `just hooks-check` reports whether this clone is covered. A
plan may remind the user to check; it may not claim the hook ran.

Re-read every message you emit before handing it over and confirm neither line is present.

## Preflight, reported not performed

- `git status --short`, `git diff --stat`, `git stash list`, `git log --oneline -5`.
- **Branch.** Never plan commits onto the default branch; propose `git switch -c <branch> main`
  and say that changing branches needs the user's approval under `AGENTS.md` § Autonomy. If the
  current branch already carries unrelated commits, say so and propose a fresh branch off `main`.
- **Pre-existing work.** A stash or an unrelated modified file stays out of the plan. List it
  under *Left unstaged* rather than sweeping it in.
- **Never stage** `.env`, credentials, database dumps, private datasets, or generated runtime
  data. `.env.example` is the documented example and is fine.

## Grouping rules

One commit is one reviewable idea that **builds and passes its own narrowest check**. A commit
that does not compile at its own tree state is a defect, not a smaller commit: `git bisect` and
CI both run at every commit.

Files this repository forces to travel together:

| Together in one commit | Why |
|---|---|
| A new migration, its `database/validation/` companion, its `database/install.sql` entry, its Squawk baseline row, and the `SCHEMA_STEPS` / `COVERED_MIGRATIONS` update in `aircraft_testsupport` | `cargo run -p xtask -- migrations` and `schema_steps_cover_every_migration` fail at any intermediate state. Migrations are immutable once hashed in `database/migrations.lock.json` |
| `docs/openapi.json` and the Rust that generates it | `just generate-docs --check` gates drift; the document is generated, never hand-edited |
| An `aircraft_config` setting and its `.env.example` entry | The example is the operator-facing half of the setting |
| A changed public struct or fn signature and every construction or call site | Otherwise the intermediate commit does not compile. `ApiState` is the recurring one: adding a field touches `apps/server/src/main.rs`, `apps/server/tests/shutdown.rs`, and the state helpers in `crates/aircraft_api/src/lib.rs`, `src/authentication.rs`, and `tests/*.rs` |
| A two-sided coupling comment and the file it names | `rust-comment` treats a one-sided mirror as a finding, so `docs/architecture/*.md` moves with the code that cites it |
| Behavior and the test that proves it | `AGENTS.md` requires tests proportional to risk; a commit that adds behavior with no test is incomplete, not minimal |

Files that get their **own** commit:

- Documentation-only prose (`README.md`, `AGENTS.md` status rows) with no code change.
- Dependency bumps (`chore(deps): …`).
- A pure refactor with no behavior change, never mixed with a feature.
- Formatting or renaming — and `AGENTS.md` forbids combining these with feature work at all.

**Do not split what only `git add -p` could split.** Interactive staging is unavailable in this
environment, so two ideas living in one file are one commit. Say so explicitly rather than
emitting a plan that cannot be run.

## Message form

The repository uses `type(scope): imperative subject`, lower case, no trailing period, subject
under ~72 characters. Read `git log --format='%s' -20` before choosing, and match what is there.

- **type**: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `ci`.
- **scope**: the crate or app the change belongs to, short — `api`, `app`, `db`, `domain`,
  `ingest`, `config`, `server`, `xtask`, `auth`, `deps`. Omit only when genuinely repo-wide.
- **body**: why, not what — the trade-off, the ordering that is load-bearing, the invariant, the
  decision document or migration the change answers to. Wrap at ~76 columns. `clean-code` and
  `rust-comment` govern the prose; the diff already shows the what.
- Reference an issue as `#N` in the body when one governs the change. Never a trailer.

## Output

For each commit, in order:

### Commit N — `<subject>`

**Rationale** — one or two sentences on why these files are one idea.

**Verification at this commit** — the narrowest command that must pass here (`cargo test -p
aircraft_config --locked`, `just check`, `just generate-docs --check`, …).

```bash
git add <explicit paths, never -A and never .>
git commit -F - <<'MSG'
<subject>

<body>
MSG
```

Explicit paths only: `git add -A` and `git add .` sweep in whatever else is in the tree.
Untracked files must be named — `git add -u` will not stage them.

Close with:

- **Left unstaged** — every path deliberately excluded, and why.
- **Not run** — state plainly that nothing was executed and that committing, branching, and
  pushing each need the user's approval under `AGENTS.md` § Autonomy and approval.
