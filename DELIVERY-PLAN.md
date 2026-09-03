# Delivery Plan

A record of delivery commitments for the HTTP failure contract, compressed from
the plans posted on issue #33. The full plans stay at their comment URLs; this
file answers what was promised, whether it was completed, what durable behavior
or decision resulted, which tests prove it, and what remains open. Acceptance
criteria are quoted as issued and are not reinterpreted here.

Status date: 2026-09-03. Branch: `feat/rfc9457-problem-mapping`, uncommitted,
based on `a489f06` (`origin/main`).

Sources:

- Issue #33: <https://github.com/rsstdd/aircraft-api-r/issues/33>
- Original completion plan:
  <https://github.com/rsstdd/aircraft-api-r/issues/33#issuecomment-5514577807>
- Review of the #29 plan, which fixes the interface #33 owes #29:
  <https://github.com/rsstdd/aircraft-api-r/issues/33#issuecomment-5514771278>
- Accepted contract: `docs/architecture/http_v1_decisions.md`. It owns the
  stable type table, the definition of "API-originated", the `required_scope`
  extension, and header-versus-body placement for `Retry-After` and
  `X-Request-Id`. Those are referenced here, not repeated.

## D01 (#33) — Add RFC 9457 problem mapping — Implemented, unmerged

Issue: #33. Parent: #7. Depends on: #25 (complete, PR #125) and #18 (complete,
PR #111). Milestone: M1 - Secure API Foundation.

### Promised

Acceptance criteria, as issued:

1. Every API-originated 4xx or 5xx uses `application/problem+json`.
2. Known application errors map to one stable type and status.
3. Unknown errors return generic 500 detail while retaining a request ID.
4. Responses reveal no SQL, constraint names, credentials, or host paths.

Required tests, as issued: malformed JSON, invalid query, not found, conflict,
dependency failure, internal error; content type and request ID on every
problem.

Required verification, as issued: `cargo test -p aircraft_api --locked`,
`just generate-docs --check`, `just check`.

Out of scope, as issued: pagination or authentication mechanics.

### Completion statement

All four criteria are implemented and proven on the working tree. The change
is not committed or merged. The AC3 diagnostic proof runs in the isolated
request-tracing integration target, where tracing callsite interest cannot be
disabled by subscriber-free router tests in the shared library-test process.
The hand-authored diff remains under the 2,000-line ceiling.

### Delivered

- `ApiProblem` in `crates/aircraft_api/src/problem.rs`: a closed `ProblemKind`
  with fifteen variants (the fourteen published types plus `ShutdownCancelled`,
  which shares `/problems/shutting-down` and differs only in `detail`), one
  exhaustive `contract()` match with no wildcard, and constructors that accept
  only the refused path plus typed metadata. No constructor accepts error text.
- `ProblemDetails` remains the only wire DTO. Base members are unchanged;
  `required_scope` is an optional, never-null extension; `instance` is
  path-only, bounded to 255 characters, and replaced by
  `/request-path-too-long` or `/invalid-request-path` rather than truncated or
  reflected.
- Router fallbacks `not_found` and `method_not_allowed` in
  `crates/aircraft_api/src/lib.rs`, reading `OriginalUri`; Axum's `Allow`
  header is preserved on 405. Axum's default body limit is disabled so the
  configured perimeter is the single size authority.
- `ApiJson<T>` and `ApiQuery<T>` extractors: JSON syntax, missing content type,
  and body-read failures map to 400 malformed-input; JSON data and query
  deserialization failures map to 400 validation-failed, normalizing Axum's 422;
  an extractor-level oversize maps to 413; an unknown upstream rejection variant
  maps to the wrapper's class and logs only a stable rejection class.
- Readiness, shutdown, cancellation, body-limit, concurrency, and deadline
  refusals delegate to `ApiProblem` with unchanged wire literals.
  `database_unavailable` and `shutting_down` now take the refused path.
- `RateLimited` emits `Retry-After`; `InsufficientScope` carries one
  allowlisted `RequiredScope` mirroring the five codes in
  `database/seeds/004_authentication_seed_data.sql`, with two-sided coupling
  comments.
- OpenAPI: `RequiredScope` schema and fourteen reusable `components.responses`,
  each using the shared `ProblemDetails` schema, publishing `X-Request-Id`, and
  carrying a generated example; `Retry-After` on `RateLimitedProblem` and
  `Allow` on `MethodNotAllowedProblem`.
  Per-operation responses are unchanged (the four perimeter refusals only).
- `docs/architecture/http_v1_decisions.md` gained the type table, the
  "API-originated" definition, and the 405 and extension-placement notes.
- `router_with_routes(state, routes)` is the public extension point tests use to
  mount a handler under the real layer stack.
- Dev-only: `aircraft_db`, `aircraft_testsupport`, and `sqlx-core` added to
  `aircraft_api` dev-dependencies; the production graph is unchanged.

### Verification

Run on 2026-09-03 against the working tree:

| Gate | Result |
|---|---|
| `cargo test -p aircraft_api --locked` | Passed: 59 library, 4 request-tracing, and 2 database-failure tests |
| `cargo test -p aircraft_api --test problem_database_failures --locked` | Passed, 2 tests, 14 s, Docker available |
| `cargo nextest run --workspace --locked` (`just test`) | Passed, 251 tests |
| `just generate-docs --check` | Passed, `docs/openapi.json` current |
| `just check` | Passed |
| `cargo +1.85.0 check --workspace --all-targets --locked` | Passed |
| `just boundaries` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `git diff --check` | Passed |
| `just static` | Passed; 14 Spectral `oas3-unused-component` warnings, 0 errors (accepted, see Decisions) |
| `just lint` | Passed: formatting, static contracts, Clippy, rustdoc, audit, and dependency policy |
| 20 repeated API library and request-tracing runs | Passed: 59 library and 4 request-tracing tests in every run |

Evidence, by test name:

- AC2 wire form: `problem::tests::each_problem_serializes_to_its_published_document`
  is exhaustive over `ProblemKind` and checks literals against the decisions
  document rather than the code under test.
- AC1 routing: `tests::an_unknown_route_returns_a_correlated_problem_document`,
  `tests::a_wrong_method_returns_a_correlated_problem_and_preserves_allow`.
- AC1, AC2, AC4 extractors:
  `tests::malformed_json_returns_a_malformed_input_problem`,
  `tests::invalid_json_data_returns_a_validation_problem`,
  `tests::json_without_a_content_type_returns_a_malformed_input_problem`,
  `tests::invalid_query_returns_a_validation_problem_without_echoing_the_query`,
  `tests::a_route_local_json_size_limit_returns_a_payload_too_large_problem`,
  `tests::the_configured_perimeter_replaces_axums_default_json_limit`.
- AC2 semantic classes:
  `tests::known_semantic_failures_have_stable_problem_types_and_statuses`,
  `tests::authorization_and_rate_limit_problems_carry_their_typed_metadata`,
  `problem::tests::rate_limit_responses_carry_retry_after`.
- AC3: `request_tracing::an_unknown_error_returns_a_generic_correlated_500`
  (fixed detail, caller request ID, sentinel absent from body and trace,
  structured class and code under the request span).
- AC4 real PostgreSQL, `crates/aircraft_api/tests/problem_database_failures.rs`:
  `a_stopped_database_returns_a_correlated_redacted_dependency_problem` and
  `a_real_unique_violation_maps_to_a_correlated_redacted_conflict_problem`. Each
  proves the sensitive diagnostic (`DATABASE_23505`, `uq_apc_secret_digest`,
  URL, credentials, host, SQL text, host path) existed server-side before
  asserting it is absent from the response.
- `instance` bounding:
  `tests::an_oversized_reflected_request_path_uses_a_valid_bounded_sentinel`
  (renamed from `a_reflected_request_path_is_bounded`),
  `problem::tests::an_invalid_percent_encoding_is_not_reflected_as_an_instance`,
  `problem::tests::only_an_origin_relative_path_can_be_reflected_as_an_instance`.
- Contract: `tests::openapi_publishes_every_reusable_problem_response`,
  `tests::openapi_publishes_the_scope_vocabulary_as_an_optional_never_null_member`,
  and the pre-existing
  `tests::every_published_failure_is_a_problem_document_carrying_a_request_id`.
- Every problem assertion goes through `assert_refusal`, which now requires a
  usable `X-Request-Id` and the problem media type on every case.

### Decisions retained

Durable rationale not owned by the decisions document or a code comment:

- Type URIs stay origin-relative under `/problems/`. Moving to an absolute
  origin is a versioned compatibility decision that waits on a public-host
  policy; it is not a refactor.
- No blanket `From<PersistenceError>` or `From<anyhow::Error>` into
  `ApiProblem`. Context decides between dependency, conflict, not-found, and
  internal, so each route maps its application enum exhaustively. Guidance for
  those routes: input errors map to validation; unknown resource or principal
  to not found; duplicate or current-state violation to conflict; unavailable
  database to dependency; invariant, entropy, or unclassified failure to
  internal.
- No global response-rewriting middleware. It would hide the source
  classification, risk dropping `Allow` and `Retry-After`, and create a second
  mapping path.
- Internal failures return the fixed 500 and record a structured class and code
  under the existing request span. Source errors are never traced through
  `Display` or `Debug`; correlation comes from span inheritance, so the request
  ID is not duplicated into extensions or the body.
- The fourteen reusable responses are published as components and attached only
  to operations that can emit them. Spectral's `oas3-unused-component` warnings
  are accepted on that basis. Router-global 404 and 405 are documented in the
  decisions document because OpenAPI has no global fallback slot.
- `/problems/database-unavailable` is the only dependency type. A non-database
  dependency gets its own reviewed type rather than reusing it.
- The Docker-backed redaction tests live in `aircraft_api`'s integration target,
  not `apps/server`, so the issue-required package command exercises the
  security gate. Measured cost: two containers, about 14 s.
- HTTP parse errors answered by the server library before the router sees a
  request are outside the contract (owned by the decisions document; noted here
  because the plan's AC1 reading depends on it).

### Follow-up

- Mutation probes run during remediation proved that the focused gates fail
  when their protected behavior is removed or misclassified: missing JSON
  content type, unknown-error diagnostics and request-span correlation,
  stopped-database readiness, real unique-conflict redaction, OpenAPI response
  registration, route-level 413 handling, and replacement of Axum's default
  body limit.
- Commit and open the PR for #33 with no attribution trailers
  (`just hooks-check`).
- Interface handed to #29: `ApiProblem::authentication_required(path)` and
  `ApiProblem::database_unavailable(path)`. The #29 plan review named these
  `unauthenticated` and `dependency_unavailable` with type
  `/problems/unauthenticated`; the delivered names and
  `/problems/authentication-required` are the contract. #29 owns
  `WWW-Authenticate: Bearer`; the request ID remains a header only.
- #31 consumes `ApiProblem::insufficient_scope(path, RequiredScope)`. #32
  consumes `ApiProblem::rate_limited(path, seconds)`. #30, #93, and later routes
  add their own exhaustive application-error mapping and attach only the
  OpenAPI responses they can emit.
- Once merged, the `AGENTS.md` state paragraph and `README.md` may need one
  sentence noting that fallbacks and extractor rejections are problem documents.
- Rerun `just lint` where the advisory database can be fetched.

Scope exclusions, unchanged from the issue: authentication mechanics (#29),
authorization enforcement and route policy (#30, #31), rate-limit mechanics
(#32), business routes (#34-#36, #63, #93), migrations, grants, runtime SQL, and
pagination.

## #29 — Bearer credential verification — Not started

Owned by issue #29 and the amended plan in the second #33 comment linked above.
That plan stands as written and is not duplicated here. Sequencing decision by
the owner on 2026-09-02: #29 waits on #33 merging. The constructors #33 delivers
for it, and the name difference from the plan, are recorded under the D01
Follow-up.
