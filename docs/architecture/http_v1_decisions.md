# HTTP v1 contract decisions

**Status:** Accepted

**Decision date:** 2026-08-30

**Tracks:** [Issue #18](https://github.com/rsstdd/aircraft-api-r/issues/18)

## Context

The HTTP backlog depends on shared decisions for versioning, errors, precision,
pagination, authentication, authorization, rate limiting, and ingestion ownership.
Those contracts must be stable before feature routes are added.

Accepted here means that later HTTP work must follow these decisions. It does not
mean the behavior is implemented. At base commit `9b980f7`, the only implemented
HTTP operation is the database-free `GET /health` contract in `aircraft_api`, and
`docs/openapi.json` describes only that operation. Authentication, authorization,
pagination, RFC 9457 problems, readiness, version reporting, and rate limiting
remain future work.

## Decisions

### Route versioning and collection behavior

- Public product and business-resource routes use the `/v1` prefix. Operational
  routes remain unversioned; `/health`, `/ready`, `/version`, and the OpenAPI
  operation use the `Public` route policy when they exist.
- Breaking public-contract changes require a new version prefix. Compatible
  additions may remain within `/v1`.
- A missing parent resource returns `404 Not Found`. A known parent with no child
  records returns a successful empty collection.
- Catalog responses expose canonical values by default. Pending assertions and
  other source evidence require curation-read access and do not silently become
  canonical.

### Exact decimals and measurements

- PostgreSQL `NUMERIC` values cross application and HTTP boundaries as validated,
  plain base-10 decimal strings. They are never serialized as JSON numbers or
  converted through binary floating point or scientific notation.
- A measurement representation carries the applicable unit code and, when the
  data has them, raw and canonical values, measurement conditions, and canonical
  status. An applicable unit is never omitted.
- A missing database row is omitted from the representation. It is not converted
  to zero, `not applicable`, or another inferred value.
- OpenAPI must describe decimal strings and optional fields as they appear on the
  wire. Rust does not introduce a second unit-conversion registry.

### Pagination

- Collection routes use keyset pagination with a `limit` query parameter. The
  default limit is 50 and the effective maximum is 200.
- Each endpoint owns an allowlist of deterministic sort orders. Every order
  includes a unique identifier as its final tiebreaker so duplicate sort values
  neither skip nor repeat rows.
- The next-page cursor is an opaque base64url value containing a cursor version,
  the selected sort, the last sort value, the unique-ID tiebreaker, and a hash of
  normalized filters. Clients must not construct or interpret it.
- Malformed cursors, unsupported cursor versions, and cursors used with different
  filters return `400 Bad Request`. Empty and final pages return a null next
  cursor. Offset pagination and a generic SQL query builder are not part of v1.

### Problem responses

- Every API-originated `4xx` or `5xx` response uses RFC 9457 problem details with
  media type `application/problem+json`.
- Each known failure class has one stable problem type and HTTP status. The shared
  mappings cover malformed input and validation (`400`), authentication (`401`),
  authorization (`403`), not found (`404`), conflict (`409`), payload too large
  (`413`), rate limit (`429`), dependency unavailable (`503`), timeout (`504`),
  and internal failure (`500`).
- Problems include a request ID. Rate-limit problems also include `Retry-After`,
  and authorization problems name the required scope.
- Internal failures use a generic client-facing detail. No problem exposes SQL,
  constraint names, credentials, credential digests, authorization headers, host
  paths, or unsanitized dependency errors.

A response is *API-originated* when it is produced inside this service's
correlation layer: every response the router, its middleware, its extractors, or
its route and method fallbacks emit. A failure the HTTP implementation answers
before the router sees a request, such as a malformed request line rejected by
the server library, is outside that boundary and outside this contract.

The stable problem types are:

| Failure | Status | Type |
|---|---:|---|
| Malformed input | 400 | `/problems/malformed-input` |
| Validation | 400 | `/problems/validation-failed` |
| Authentication | 401 | `/problems/authentication-required` |
| Authorization | 403 | `/problems/insufficient-scope` |
| Not found | 404 | `/problems/not-found` |
| Method not allowed | 405 | `/problems/method-not-allowed` |
| Conflict | 409 | `/problems/conflict` |
| Payload too large | 413 | `/problems/payload-too-large` |
| Rate limit | 429 | `/problems/rate-limit-exceeded` |
| Internal | 500 | `/problems/internal-error` |
| Database unavailable | 503 | `/problems/database-unavailable` |
| Shutting down | 503 | `/problems/shutting-down` |
| Overloaded | 503 | `/problems/overloaded` |
| Deadline exceeded | 504 | `/problems/deadline-exceeded` |

This table is enforced by `ProblemKind::contract` in
`crates/aircraft_api/src/problem.rs`, which names this section in turn, and is
pinned against it by `each_problem_serializes_to_its_published_document`. A row
added or changed here without that function changing publishes a type the
service never emits.

`405` is not one of the failure classes enumerated above. It is in the table
because a router that declares a path and not a method emits it whether or not
this contract names it, and an API-originated response with no problem document
would contradict the first rule in this section.

Authorization problems carry the allowlisted `required_scope` extension, whose
codes are the ones seeded into `aircraft_auth.scopes`. Rate-limit problems carry
retry timing in `Retry-After`; request correlation remains in `X-Request-Id`
rather than being duplicated in the JSON body.

### Authentication and route policies

- Protected operations use an opaque API credential in the HTTP
  `Authorization: Bearer` scheme. Issuance generates at least 256 bits of random
  secret material, returns the clear credential once, and stores only its key
  identifier, SHA-256 digest, ownership, timestamps, and non-secret metadata.
- Verification resolves the key identifier with at most one bounded database
  lookup and compares the digest in constant time. Credentials and digests never
  enter responses or traces.
- Missing, malformed, unknown, revoked, or disabled credentials produce the same
  `401 Unauthorized` contract. An authenticated principal without the required
  scope receives `403 Forbidden` and the response names that scope.
- Every route is registered with exactly one closed policy:
  `Public`, `CatalogRead`, `MilitaryRead`, `CurationRead`, `CurationWrite`, or
  `Admin`. Policy omission must be unavailable at the registration boundary.
  Enforcement and generated OpenAPI security requirements consume the same
  policy metadata.
- Health, readiness, version, and OpenAPI are `Public`. Ordinary catalog reads
  use `CatalogRead`; military data uses `MilitaryRead`; pending evidence uses
  `CurationRead`; curation decisions use `CurationWrite`; credential and other
  administrative operations use `Admin`.

### Single-replica rate limiting and perimeter bounds

- The supported initial deployment is one API replica with a bounded in-memory
  token bucket keyed by authenticated principal ID. Capacity and refill rate come
  from the principal's configured tier; quota values are operational configuration,
  not a versioned HTTP default.
- Principals have isolated buckets. Inactive buckets are evicted and the total
  bucket count is bounded. The limiter uses monotonic time.
- An exhausted bucket returns the shared `429 Too Many Requests` problem and a
  `Retry-After` header. Cross-replica enforcement and a distributed rate-limit
  service are explicitly deferred.
- The HTTP boundary must also enforce explicit request-byte, query-complexity,
  concurrency, and timeout bounds before handler work. Pools, retries, queues,
  record counts, and diagnostic messages remain bounded at their owning layers.

### Ingestion boundary

- Source artifact validation, import, run history, and ingestion status remain
  operations of the `aircraft-ingest` CLI. v1 does not accept source uploads or
  provide HTTP routes that start or supervise imports.
- Later HTTP curation routes may inspect or decide assertions already preserved by
  ingestion. That does not move source parsing, import transactions, provenance,
  or ingestion audit ownership into the API.

### Database migration and SQLx workflow

- Every migration in `database/migrations/` is checksum-locked and immutable.
  This includes and strengthens issue #18's original `001` through `019`
  requirement, which named a range that has since moved. Authentication schema
  work must take the next unused number, not the migration `020` named in the
  earlier backlog text -- that number is already taken by
  `020_market_curation_gate.sql`.
- Runtime-checked SQLx queries with bound parameters and explicit row conversion
  remain the supported database workflow. SQLx compile-time query macros are not
  required.
- `just check-offline` is not a meaningful metadata gate until checked SQLx query
  metadata exists. Runtime-query support must not be weakened merely to make that
  command appear stricter.

## Consequences

Future route stories must implement and test these contracts through the real
router and generate matching OpenAPI. Until a behavior has production code and
meaningful verification, project status documentation must continue to describe
it as planned rather than present.
