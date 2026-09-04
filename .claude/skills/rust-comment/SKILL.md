---
name: rust-comment
description: The commenting and rustdoc standard for this Rust workspace — why-not-what prose, `///` and `//!` doc comments, the `# Examples` / `# Errors` / `# Panics` / `# Safety` sections, two-sided coupling comments to migrations and generated artifacts, and how debt is marked here. REQUIRED before writing, generating, or editing any Rust here, and used to review the comments in a diff, file, or crate.
---

# Rust comments and doc comments in this workspace

Every comment is code that a compiler cannot check. It earns its line by carrying something the
code cannot: intent, a trade-off, an invariant, a domain rule, a coupling to canonical SQL.
Anything else is future noise.

## Authority

`crates/AGENTS.md` §3 owns comment *mechanics* (sigils, one space after the marker, complete
sentences, no comments on closing braces or inside signatures, doc comments before attributes).
This skill owns *content*: what to say, where, and when a comment must exist. Conflict order is
the one `AGENTS.md` publishes under *Sources of truth*; `crates/AGENTS.md` wins over this file.
Flag a genuine conflict; never resolve one silently.

Load `clean-code` (style and structure) and `rust-review` (the severity scale and the conduct
rules that bind every skill here). This is the comment layer of both; it adds constraints and
changes neither.

## General

- **Explain WHY, not WHAT.** Design trade-offs, ordering that is load-bearing, domain rules, and
  the reason a deviation is proportionate. A comment that restates the code is a finding, and so
  is one that does not help its reader.
- **Terse and dense.** Legible to a human skimming a diff and to an LLM reading it cold.
- **Delete commented-out code.** Git tracks history; a commented block is noise that bit-rots.
- **An outdated comment is a bug.** Update or delete it in the same commit as the code it
  describes — never in a follow-up.
- **Keep locality.** Item-level prose goes above the item; a note about one expression goes
  inline beside it. A comment far from what it explains stops being maintained.
- Applies to code you write or touch, under the boy scout rule in `AGENTS.md`. It is not a
  mandate to retrofit untouched files.

## Doc comments

- **`///` is a judgment call here, not a lint.** `missing_docs` is *not* in the workspace lint
  table, and `clippy::missing_errors_doc` and `clippy::missing_panics_doc` are explicitly
  `allow`. `crates/AGENTS.md` sets the real bar: document a public item when callers must
  understand invariants, error conditions, security boundaries, units, ownership, or durable
  contract semantics. Do not add a doc comment that repeats a public name or type — that is a
  finding, not coverage.
- `//!` at the top of a module or `lib.rs` for architecture and scope — what the module owns,
  what it deliberately does not, and which document or migration governs it.
  `aircraft_testsupport/src/lib.rs` is the example to imitate: it says what the harness is, what
  it installs, and *why it is a crate rather than a `tests/` module*.
- Idiomatic Markdown. Link items with ``[`ImportRequest`]`` rather than naming them in plain
  prose. Backtick anything rustdoc would otherwise read as an intra-doc link — `PostgreSQL` and
  `SQLx` in prose are already written that way in this tree for exactly that reason.

## rustdoc sections

Use them in this order, and only when they apply.

| Section | When | Content |
|---|---|---|
| `# Examples` | Public API worth showing in use | Real usage, not a toy. **See the doctest warning below before adding one.** |
| `# Errors` | Public fn returning `Result` whose variants callers must distinguish | Which failure conditions produce which variant. |
| `# Panics` | Any path that can panic | The condition, and why no caller argument can reach it. |
| `# Safety` | Every `unsafe fn` | Currently unreachable — see below. |

**`# Errors` names variants, not categories.** `crates/AGENTS.md` requires one variant per
caller-visible distinction so a test can assert the exact failure; the doc must let a caller map
a condition to that variant. `ImportError::ParserRecordKeysMismatch` and
`ImportError::WarningCountMismatch` exist as separate variants precisely so a reader — and a test
— can tell a second-pass identity divergence from a second-pass count divergence.

**`# Panics` must justify, not just disclose.** `clippy::panic` and `clippy::expect_used` are
`warn` and `clippy::unwrap_used` is `deny` workspace-wide, so a surviving panic on a library path
is already unusual. A bare "panics if X" is not enough: argue from the concrete types why no
caller argument can reach it, or return a typed error instead. Test modules opt out locally
(`#![allow(clippy::expect_used)]` in a `#[cfg(test)] mod tests`) and that is the only accepted
place for it.

**Doctests run, but only as a second step.** nextest does not execute them, so `just test` and
`.github/workflows/ci.yml` follow it with `cargo test --workspace --doc --locked`. The route-policy
proofs in `aircraft_api` are `compile_fail` doctests -- a bare `Router` cannot enter
`router_with_routes`, and no route can be added to the `ApplicationRouter` it returns -- claims no
runtime test can make. An `# Examples` block is still prose until that step has run: run
`cargo test --workspace --doc --locked` yourself and say you did, and prefer pointing at a real
test by name over inventing an example.

**Rustdoc warnings are a merge blocker in their own right.** `just docs-check` runs
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`, and it is
part of `just lint` and of CI's `rustdoc` job. A broken intra-doc link or malformed section fails
the build.

**`# Safety` is currently unreachable.** `unsafe_code = "forbid"` is set workspace-wide in the
root `Cargo.toml`: there is no `unsafe` here, and adding it is a Critical finding needing an
explicit repository-wide decision, not a `// SAFETY:` block. If that forbid is ever lifted, the
rules written back here are: `// SAFETY:` above every block explaining why the invariants hold at
that point; raw-pointer, layout, non-null, alignment, aliasing, and thread-safety assumptions
spelled out; and the block kept minimal so the comment covers exactly what it justifies.

## Logic, boundaries, and performance

Document what a reader cannot recover from the code:

- Why a bound is the number it is — an input-byte ceiling, a channel depth, a retained-error cap,
  a diagnostic truncation length. `AGENTS.md` requires these bounds to be explicit; the comment
  is where "explicit" becomes "explained".
- Why an ordering is load-bearing. The clearest example in this tree is
  `SqlxIngestionStore::start_import`, whose comment states that startup holds two pooled
  connections at once and that the admission semaphore exists so pool acquisition cannot deadlock
  at any supported pool size. Nothing in the signatures says that.
- Transaction ownership and what commits or rolls back together. A reader must be able to see,
  from the comment or the structure, that staging, promotion, provenance, curation flags, run
  completion, and the read-model refresh share one fate.
- Non-obvious allocations, and zero-copy or borrow-based designs that look accidental.
- Determinism-carrying choices. Where an ordering, a hash input, or a serialization shape feeds a
  content digest, a record-key fingerprint, or a committed snapshot, say so.
- A hand-placed `#[inline]` — say what measurement motivated it.

## Coupling comments

Any constant, table, SQL string, or status vocabulary that mirrors something outside the file
carries a **two-sided** comment: the Rust names the migration, validation file, seed, or
generated artifact, and that file names the Rust path. A one-sided mirror is a finding.

`crates/AGENTS.md` states this as a rule — "Name the coupled migration, validation, generated
artifact, or protocol when a future change could otherwise update only one side." The load-bearing
example is `aircraft_testsupport::SCHEMA_STEPS` ↔ `database/install.sql`: the doc comment names
the installer, explains why the reference seeds land after migration 002 and the mission-profile
seed after 015, and names the test that guards the list (`schema_steps_cover_every_migration`).

This is the settled exception to "avoid comments; explain in code" recorded in `clean-code`: a
mirror between Rust and canonical SQL cannot be expressed in Rust, so both ends must name each
other. `grep` is the discovery tool — anything implied only by git history is invisible.

Migrations are immutable once hashed in `database/migrations.lock.json`. When a migration's
comment is wrong, `AGENTS.md` requires the correction to land in `database/data_dictionary.md` or
`database/implementation_notes.md` — never by editing the migration.

## Debt and status

- `// TODO(author): action` — a named owner and the concrete action. An unowned `TODO` is
  anonymous debt and will not be done.
- `// FIXME: bug` — known-wrong behavior, described so a reader can reproduce it.
- `crates/AGENTS.md` prefers a tracked issue or the owning document over "vague TODO prose".
  There is no delivery-plan file in this tree to reference by ID, so a marker that cannot name an
  owner and an action does not belong in the diff at all.
- Neither marker is a substitute for a test. Do not confuse them with `clippy::todo`, which is
  `warn` in the workspace lint table and catches the `todo!()` *macro*: shipping a stub as if it
  were implemented is forbidden by `AGENTS.md` and `crates/AGENTS.md` regardless of the comment
  beside it.
- Phasing out an API: `#[deprecated(note = "...")]` carrying the replacement, plus a `///` line
  saying since when and why. The attribute is what callers see; the comment is what a maintainer
  needs.

## Reviewing comments

Run the sections above against your own diff before reporting done, and map what you find onto
the `rust-review` severity scale:

- **Major** — a wrong or stale doc on a public item, an undocumented or unjustified panic on a
  library path, a one-sided coupling comment whose other end then drifts silently, or a comment
  that claims an ordering or bound the code no longer has.
- **Minor** — a missing `# Errors` where variants are genuinely distinguishable, a comment that
  restates the code, an unowned `TODO`.

Two checks are not readable off the diff: `grep` the migration, validation file, or installer for
the Rust path before calling a coupling comment two-sided, and actually run `just docs-check` if a
doc comment changed.
