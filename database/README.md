# Database

This directory owns the SQL lifecycle for the Aircraft Management Engine.

## Layout

- `migrations/`: ordered, one-time schema migrations.
- `seeds/`: idempotent canonical reference, mission-profile, and
  authentication-scope data only. Each seed is applied by `install.sql` at the
  point its tables exist, so `004_authentication_seed_data.sql` follows
  migration 025 rather than sitting with the other lookup seeds.
- `validation/`: post-install schema and behavioral verification.
- `snapshots/`: normalized business snapshot queries, plus the committed golden
  output in `snapshots/golden/<fixture>/` that `cargo xtask snapshots` diffs the
  adapter against.
- `roles/`: restricted ingestion and server role creation and grants, applied
  by an administrator, not by `install.sql`.
- `fixtures/`: test-only data.
- `docker/init/`: optional database-level initialization for an empty volume.
- `reconcile_local_legacy.sql`: local-only compatibility check for complete
  Phase 1/2 schemas created before migration tracking existed.
- `install.sql`: the canonical dependency-aware `psql` installer.
- `migrations.lock.json`: SHA-256 of every migration. Migrations are immutable
  once written; `cargo xtask migrations` fails if a file's contents change, so a
  correction goes in a new migration and in this documentation, never by editing
  an applied one.
- Credential storage is restricted by contract: `aircraft_auth.api_credentials`
  may hold a key identifier, a SHA-256 digest, ownership, timestamps, and a
  non-secret label, and nothing else. No DDL can forbid a column that does not
  exist yet, so the exact column list is asserted by
  `validation/025_authentication_schema_validation.sql` and by
  `crates/aircraft_db/tests/auth_schema.rs`. A migration that adds a
  clear-token, plaintext, or recovery column fails both.
- `validation/000_migration_history_validation.sql`: asserts that the applied
  ledger matches the shipped migrations exactly. Its version list is compared to
  `migrations/` by `cargo xtask migrations`, so adding a migration without
  extending it fails the policy check rather than the next `just db-validate`.
- `data_dictionary.md`: detailed reference for principal tables and read models;
  migrations remain authoritative for the complete schema.
- `implementation_notes.md`: dependency rules, curator workflows, known
  limitations, and deferred database decisions.

## Canonical seed audit policy

The canonical seed vocabulary was audited on 2026-09-10 and extended on
2026-09-11 by two `aircraft_ref.landing_gear_types` codes, `FIXED_UNSPECIFIED`
and `RETRACTABLE_UNSPECIFIED`. Every other wheeled code bundles retraction with
configuration, so a source that states "fixed landing gear" without saying
tricycle or tailwheel had nowhere to land and was discarded entirely. The two
codes keep the half the source gives; a curator narrows them once the
configuration is established. `crates/aircraft_ingest/src/normalization.rs`
maps them and checks for a stated configuration first, so a source that does
give one still resolves to the specific code.
 Reapplying the four
files under `seeds/` restores every seed-owned mutable column without replacing
keys or generated identities. Mission suitability caches are invalidated only
when the criteria policy for their profile differs. Unknown lookup rows and
user-created data are not deleted, with one deliberate exception: the criteria of
a seeded mission profile are wholly seed-owned, so a criterion added to one by
hand is removed on reapplication. That is what makes the policy converge rather
than accumulate, and `reapplying_seeds_repairs_drift_without_retaining_stale_scores`
asserts it. Extend a profile by editing the seed, not the table. Exact row-count
validation continues to flag vocabulary outside the canonical set.

Externally factual fields use these primary sources:

- Measurement conversion factors follow [NIST Special Publication
  811](https://www.nist.gov/pml/special-publication-811). Values are rounded to
  the nearest value representable by `NUMERIC(18,10)`. Fahrenheit is affine and
  PPH volume conversion depends on fuel density, so neither is assigned a fake
  universal factor.
- The six seeded currency codes, names, and minor units follow [ISO
  4217](https://www.iso.org/iso-4217-currency-codes.html). Display symbols are
  repository policy rather than ISO assertions.
- EASA jurisdictions follow the [EASA member-state
  register](https://www.easa.europa.eu/en/light/topics/easa-member-states) and
  use the complete current ISO 3166-1 alpha-3 member set rather than a regional
  pseudo-code.
- FAA light-sport and sport-pilot descriptions follow the performance-based
  [MOSAIC rule](https://www.faa.gov/aircraft/MOSAIC), whose aircraft
  certification provisions took effect July 24, 2026. Approach category text
  follows the FAA [runway visual range
  guidance](https://www.faa.gov/about/office_org/headquarters_offices/ato/service_units/techops/navservices/lsg/rvr).

Repository-owned policy includes display labels and symbols, descriptions that
do not quote an external rule, grouping, ordering, activity flags, mission
profile metadata, and all mission criterion choices, weights, required flags,
bounds, and notes. The first five configured profiles retain their original
criteria and weights with complete bounds. The ten former stubs use the v1
six-criterion matrix in `seeds/003_mission_profile_seed_data.sql`; the complete
policy contains exactly 88 criteria and every profile weight sum is `1.000`.

Canonical seed fields are populated unless absence has defined semantics. The
complete semantic-`NULL` allowlist is:

- canonical-unit key/factor pairs on units that are their own canonical
  representation;
- SI fields for affine or context-dependent conversion (`DEG_F` and `PPH`);
- unpowered (`NONE_GLIDER`) `primary_power_unit`;
- fuel density for non-liquid or state-dependent carriers;
- canonical units for dimensionless metrics;
- context-dependent comparison direction for wingspan and the documented
  V-speeds;
- `MILITARY_SPEC.authority_code`, because no single civil authority owns it;
- metric foreign keys for computed comparison criteria.

All other seeded descriptions, labels, notes, applicable URLs and arrays, and
mission scoring bounds must be present and non-blank. The assertions in
`validation/002_core_reference_tables_validation.sql` and
`validation/phase15_16_comparison_readmodels_validation.sql` enforce this
policy, and `crates/aircraft_testsupport/tests/seed_data.rs` runs both against a
disposable canonical installation.

Non-blank is the floor, not the standard. A lookup `description` must add
information its `label` does not already carry: what the value means, and where
its boundary against the neighbouring codes falls. It has to survive having the
label deleted from it. This is enforceable rather than advisory because
`GET /v1/reference/{catalog}` publishes the column verbatim — twelve of these
tables briefly carried `label || ' <suffix>'` text, which satisfied every
completeness assertion while telling an API client nothing. The second loop in
`validation/002_core_reference_tables_validation.sql` now rejects any table
whose descriptions are uniformly its labels plus one shared suffix. Descriptions
that do not quote an external rule remain repository-owned policy, per the
paragraph above; a description that does quote one belongs in the primary-source
list with its citation.

## What the ingested data may be used for

`aircraft_prov.sources.license_notes` records the terms each source publishes, and
migration `026_source_license_terms.sql` fills it for `planephd` from
<https://planephd.com/terms> and <https://planephd.com/robots.txt> as they stood on
2026-09-12. Read the column rather than this paragraph; it is the copy that ships with the
data, and `database/validation/026_source_license_terms_validation.sql` fails the install
if it goes missing.

The short version, which does not replace the column: PlanePHD reserves its data
absolutely, forbids incorporating it into another database or redistributing it in any
form, and its robots.txt carries `ai-train=no, use=reference` as an express reservation of
rights. No vendor or client agreement is on file here. Treat every PlanePHD-derived value
as reference-only evidence.

A new source must record its own terms the same way. `promote_document` upserts a source
row with only `base_url` in its `ON CONFLICT` clause, so an auto-created source starts with
`license_notes` NULL and nothing will tell you.

## Catalog columns the ingested data leaves empty

Measured against the 1,005 variants, 1,005 models, and 75 families the PlanePHD
import publishes. These are not defects to close by writing a value: `AGENTS.md`
forbids inventing one, and nothing in the source states them.

| Column | Populated | Why |
|---|---|---|
| `variants.service_status_code` | 1,005/1,005 | derived from the production range |
| `variants.production_start_year` | 1,005/1,005 | parsed from the aircraft name |
| `variants.is_in_production` | 1,005/1,005 | `(1997 - present)` versus a closed range |
| `variants.propulsion_category_code` | 855/1,005 | stated in prose; turbofan stays unstated |
| `variants.landing_gear_type_code` | 832/1,005 | fixed or retractable only; no tricycle/tailwheel wording |
| `variants.variant_type_code` | 1,005/1,005, one code | every row is `PRODUCTION_STANDARD`; no source distinguishes a second |
| `variants.country_of_origin_code` | 0/1,005 | PlanePHD states no country |
| `families.country_of_origin_code` | 0/75 | the same |
| `variants.first_flight_year` | 0/1,005 | PlanePHD states no first-flight date |
| `models.first_flight_year`, `models.certification_year` | 0/1,005 | the same |
| `aircraft_core.variant_manufacturers.production_country_code` | 0/1,005 | no per-variant manufacturing country in the source |

Country of origin and first flight are obtainable, and were verified against live
Wikidata (CC0): `P495` → `P298` gives an ISO alpha-3, `P606` gives a first-flight
year. Matching is the work — Wikidata models notable types where this catalog
models year-range variants — so the usable join is at the manufacturer, assigning
the country its aircraft agree on when that agreement covers at least 90% of at
least 10 Wikidata aircraft. Ten manufacturers qualify, covering 537 of the 1,005
variants. The threshold is load-bearing: relaxing it to 80%/5 admits
`Learjet → CAN`, which reflects Bombardier's ownership rather than where the
aircraft were designed.

That work belongs behind a Wikidata source of its own, with its own
`aircraft_prov.sources` row and its own provenance, not inside the PlanePHD
parser — a second source's facts must not reach the catalog under the first
source's identity. It is planned, not built.

Landing-gear class beyond fixed-versus-retractable and variant type are
obtainable from **neither** source: `Q15896132`'s complete claim set carries no
landing-gear and no variant-class property. Their emptiness is the correct state.

Two consequences for the catalog read routes. The `country_of_origin` filter on
families and variants binds correctly and returns nothing, because no row carries
a code — a data gap, not a query defect, and
`every_catalog_filter_vocabulary_offers_more_than_one_code` in
`crates/aircraft_testsupport/tests/seed_data.rs` proves the vocabulary behind it
is seeded and able to discriminate. The `variant_type` filter matches every row
or none until a second code has a source.

## Local workflow

```bash
just db-bootstrap
```

`db-bootstrap` starts and waits for the Compose database, then installs and
validates it. Before `db-migrate` uses `install.sql`, the local-only
`reconcile_local_legacy.sql` compatibility check adopts complete migration 001
and 002 structures created before migration tracking was introduced. It refuses
partial or ambiguous legacy structures. Production recipes do not perform this
automatic adoption. The installer tracks migrations in
`public.aircraft_schema_migrations` and applies required seed phases at their
foreign-key boundaries. Do not execute `database/migrations/*.sql` as a simple
glob: Phase 3 requires the Phase 2 lookup seed rows.

The reconciliation step never runs in `db-prod-migrate` or
`db-prod-bootstrap`. Production legacy databases must be reviewed and baselined
deliberately.

`db-validate` begins by checking the exact migration ledger before running the
phase-specific schema and behavioral validations.

To reapply only canonical data, run `just db-seed`.

`just db-reset` deletes the local PostgreSQL volume and starts a fresh empty
container. `just db-rebuild` performs that destructive reset and then installs,
seeds, and validates the database. Do not use either command when local data
must be preserved.

## Explore a seeded local database

Check the Compose database and open its configured `psql` session:

```bash
just db-status
just db-ready
just db-psql
```

The following `psql` cheat sheet runs inside a read-only transaction. Canonical
seeding populates reference and mission-profile data; aircraft, ingestion, and
curation queries remain empty until source data has been imported.

```sql
\conninfo
\timing on
\x auto
\pset null '<null>'

BEGIN READ ONLY;
SET LOCAL statement_timeout = '10s';

-- Discover schemas and relations.
\dn
\dt aircraft_ref.*
\dt aircraft_core.*
\dt aircraft_compare.*
\dt aircraft_ingest.*
\dt aircraft_prov.*
\dv aircraft_read.*
\dm aircraft_read.*

-- Confirm the installed schema history.
SELECT version, applied_at
FROM public.aircraft_schema_migrations
ORDER BY version;

-- Inspect canonical reference data.
SELECT unit_category_code, count(*) AS units
FROM aircraft_ref.measurement_units
GROUP BY unit_category_code
ORDER BY unit_category_code;

SELECT code, label, canonical_unit_code
FROM aircraft_ref.performance_metric_types
ORDER BY sort_order, code;

SELECT code, label, role_group
FROM aircraft_ref.aircraft_roles
ORDER BY sort_order, code;

-- Check mission profiles and their criterion weights.
SELECT
    mp.slug,
    mp.title,
    count(mc.id) AS criteria,
    coalesce(sum(mc.weight), 0) AS total_weight
FROM aircraft_compare.mission_profiles AS mp
LEFT JOIN aircraft_compare.mission_criteria AS mc
    ON mc.mission_profile_id = mp.id
GROUP BY mp.id, mp.slug, mp.title, mp.sort_order
ORDER BY mp.sort_order;

-- Find populated application tables.
SELECT schemaname, relname, n_live_tup AS estimated_rows
FROM pg_stat_user_tables
WHERE schemaname LIKE 'aircraft_%'
ORDER BY n_live_tup DESC, schemaname, relname;

-- Browse imported variants and the published read model.
SELECT id, slug, name, service_status_code, passenger_capacity, engine_count
FROM aircraft_core.variants
ORDER BY slug
LIMIT 50;

SELECT
    slug,
    variant_name,
    primary_manufacturer_name,
    cruise_speed_kias,
    range_nm,
    service_ceiling_ft,
    gross_weight_lb,
    papi_price_usd
FROM aircraft_read.mv_variant_search
ORDER BY primary_manufacturer_name, variant_name
LIMIT 50;

-- Inspect ingestion history.
SELECT
    id,
    source_slug,
    status,
    staged_aircraft,
    promoted_aircraft,
    flagged_aircraft,
    warning_count,
    started_at,
    finished_at
FROM aircraft_ingest.ingest_runs
ORDER BY started_at DESC
LIMIT 20;

-- Inspect pending curation work.
SELECT status_code, count(*)
FROM aircraft_prov.source_assertions
GROUP BY status_code
ORDER BY status_code;

SELECT
    id,
    entity_type_code,
    entity_id,
    field_name,
    raw_value,
    asserted_numeric,
    status_code
FROM aircraft_prov.source_assertions
WHERE status_code = 'PENDING'
ORDER BY created_at, id
LIMIT 50;

SELECT
    priority,
    entity_type_code,
    entity_id,
    field_name,
    issue_type,
    issue_description
FROM aircraft_prov.curation_flags
WHERE status_code = 'OPEN'
ORDER BY priority, created_at
LIMIT 50;

-- Check whether a read-model refresh is outstanding.
SELECT
    id,
    requested_by,
    reason,
    status_code,
    attempts,
    requested_at,
    completed_at,
    last_error
FROM aircraft_read.read_model_refresh_requests
ORDER BY requested_at DESC
LIMIT 50;

ROLLBACK;
\q
```

The CLI also exposes read-only ingestion and curation summaries:

```bash
just ingest-status --limit 20
just ingest-status-json --limit 20
just curate-list --limit 20
```

Use the Rust ingestion adapter for source JSON:

```bash
just ingest-validate ./aircraft_seed.json
just ingest-import ./aircraft_seed.json
just ingest-status
```

It reads a local file or standard input, preserves raw records and audit history,
and does not require database-server filesystem access. The legacy
server-side SQL loader that used to live in `database/staging/` has been
retired; `database/snapshots/` now holds the snapshot queries and the committed
golden output that guard the Rust path against regressions.

Diesel may generate Rust schema types from PostgreSQL, but SQL files in this
directory remain the canonical schema history.
