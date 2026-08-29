# Aircraft Encyclopedia — Implementation Notes

---

## 1. Migration Run Order and Dependency Map

### 1.1 Canonical Install and Validation

Run from the repository root. The installer is a real `psql` script, stops on
the first error, records each successful migration in
`public.aircraft_schema_migrations`, and interleaves the seed phases required by
foreign keys.

The local `just db-migrate` recipe first runs
`database/reconcile_local_legacy.sql`. This compatibility check handles Compose
volumes that applied the original transactional Phase 1 or Phase 2 files before
the migration ledger existed. It records a phase only after every expected
object is present, repairs the later-added Phase 1 array helper, and rejects
partial legacy states. Direct/production recipes intentionally skip automatic
legacy adoption.

```bash
# Container-backed local workflow
just db-up
just db-wait
just db-migrate
just db-validate

# Direct production workflow
MIGRATION_DATABASE_URL='postgresql://migration-role@db.example/aircraft' \
  just db-prod-bootstrap
```

`just db-seed` and `just db-prod-seed` reapply only the three canonical seed
files. They do not execute transient ingestion or validation scripts.

The Rust ingestion adapter is the primary operational path. It captures a local
file or standard input, performs complete preflight validation, and imports
through the restricted ingestion database role without server-side filesystem
access:

```bash
just ingest-validate ./aircraft_seed.json
just ingest-import ./aircraft_seed.json
just ingest-status
```

The legacy server-side SQL loader (`database/staging/901`-`903`) has been
retired. Ingestion runs through the `aircraft-ingest` CLI, and
`cargo xtask snapshots` guards its output against committed golden snapshots.

The direct recipes prefer `MIGRATION_DATABASE_URL` and fall back to
`DATABASE_URL`. They require a pre-provisioned database and a migration role
with permission to create all objects in the migration set. They do not create
cluster roles or databases because those operations are provider-specific and
usually require privileges that should not be present in an application
deployment.


### 1.2 Hard Dependency Rules

`database/install.sql` enforces these dependencies:

| Rule | Reason |
|---|---|
| `001_reference_units.sql` before `002_lookup_seed_data.sql` | Metric and propulsion lookup rows reference `measurement_units` |
| Phase 2 seeds before Phase 3 | Geography/organization rows reference organization lookup types |
| Phase 3 before Phase 4 | Aircraft families reference organizations |
| Phase 4 before Phases 5-15 | Domain tables reference aircraft_core.variants |
| Phase 14 before Phase 17 promotion | Promotion writes provenance documents and assertions |
| Phase 16 before ingestion refresh | The search materialized views must exist before refresh |

Do not run the migration glob directly; its lexical order cannot express the
required Phase 2 seed boundary.

### 1.3 Re-running Safely

- Re-running `database/install.sql` skips versions already recorded in
  `public.aircraft_schema_migrations` and reapplies idempotent canonical seeds.
- A session-level advisory lock serializes installers targeting the same
  database.
- Do not rerun individual migration files: most contain one-time DDL and rely on
  the installer history table for repeatability.
- The Rust run identity is the source, full content SHA-256, parser name, and parser
  version. Re-importing a successful identity returns its existing report; failed
  attempts may retry under the same logical run.
- Every entrypoint uses `ON_ERROR_STOP=1`; a SQL error cannot be reported as a
  successful bootstrap.

---

## 2. Known Limitations and Deferred Decisions

### 2.1 Performance Model: Single Source, Single Condition

**Current state.** The ingestion adapter inserts one `performance_metrics` row per source assertion with `is_canonical = FALSE`. Migration 020 links that row to the exact assertion through `source_assertion_id`; curation therefore changes only the value that its decision backs. `aircraft-ingest curate accept --assertion-id <n>` flips the assertion and measurement in one transaction, then refreshes the read model. Migrations 019 and 020 extended the same gate to weight metrics and market data. The `is_canonical` partial UNIQUE index enforces only one canonical row per `(variant_id, metric_type_code)`.

**Limitation.** PlanePHD provides only one performance figure per metric (e.g. one cruise speed value). The schema supports multiple test conditions and multiple sources per metric, but the ingestion pipeline does not yet populate `condition_weight_lbs`, `condition_altitude_ft`, or `condition_power_setting` because PlanePHD does not expose that data.

**Resolved.** `SqlxCurationStore::decide` (`crates/aircraft_db/src/repositories/curation_repository.rs`) is the `promote_assertion` step this note anticipated: it flips `is_accepted` and the backing canonical row together, refuses a second accepted assertion for a field with `CURATION_CONFLICT` rather than racing `uq_assertion_accepted`, and supports withdrawal so a decision can be reversed. What remains deferred is *automatic* selection between competing sources by confidence; today a curator chooses.

### 2.2 Engine Manufacturer Matching

**Current state.** The Phase 17 promotion step creates `engine_variants` rows with `manufacturer_org_id = NULL` and `manufacturer_name_raw` set to the raw source string (e.g. `'Cont Motor'`, `'Ivchenko'`). A GIN-indexed `name_aliases TEXT[]` column on `aircraft_org.organizations` supports alias resolution, but the Phase 3 seed data only includes ~27 organizations.

**Limitation.** The source data uses abbreviated and variant manufacturer names (`Cont Motor` for Continental Motors, `Lyc` for Lycoming). Until a curator runs the matching pass, engine variants remain unlinked from their organization records.

**Deferred decision.** A Phase 22 curator tool should provide a `match_engine_manufacturers()` function that: (1) queries `engine_variants WHERE manufacturer_org_id IS NULL`, (2) attempts fuzzy-match against `organizations.name_aliases` using `pg_trgm` similarity, (3) presents candidates above a threshold similarity score for curator confirmation, (4) writes the matched `manufacturer_org_id` and raises no flag. Phase reference: Phase 9.

### 2.3 Ownership Cost Fuel Price Extraction

**Current state.** The `cost_snapshots.assumed_fuel_price_per_gal` column is left NULL during Phase 17 ingestion. The fuel price is embedded in the dynamic cost key name (e.g. `fuel_cost_per_hour_3_5_gallons_hr_5_40_gal` implies $5.40/gal) but extracting it requires parsing a third embedded number from an already-complex key pattern.

**Limitation.** Without the fuel price assumption, per-hour variable cost calculations cannot be verified or re-run at a different fuel price.

**Deferred decision.** Add a `parse_fuel_price_from_key(TEXT)` helper function in Phase 22 that uses `regexp_match(key, '_hr_([0-9]+_[0-9]+)_gal')` to extract the price token and converts the underscore-delimited decimal (`5_40` → `5.40`). Run as a curation pass on all `cost_snapshots WHERE assumed_fuel_price_per_gal IS NULL`. Phase reference: Phase 17.

### 2.4 Dimension and Hangar Fit Data

**Current state.** `aircraft_specs.dimension_metrics` is defined and indexed,
and `aircraft_read.v_hangar_fit` is defined over it. However, PlanePHD does not
expose wingspan, length, or height data in the JSON seed format used for Phase
17. The view still returns one row per variant because it uses LEFT JOINs; its
dimension and fit columns remain NULL until a secondary source supplies the
required measurements.

**Deferred decision.** Identify a secondary source (FAA type certificate data sheets, manufacturer specifications, or a structured Wikipedia/DBpedia extract) and build a Phase 22 ingestion adapter. The `aircraft_prov` pipeline is ready to accept multi-source dimension data with separate confidence scores. Phase reference: Phase 6, Phase 14.

### 2.5 Materialized View Refresh Cadence

**Current state.** `refresh_search_matviews()` is a manually called function. There is no automated trigger or scheduled job.

**Limitation.** After any write to canonical tables (new valuation snapshot, updated performance metric, accepted assertion), `mv_variant_search` becomes stale. In a read-heavy production environment, stale search results are visible to users until the function is called.

**Deferred decision.** Two viable approaches: (a) Schedule `refresh_search_matviews()` via `pg_cron` on a fixed cadence (e.g. every 15 minutes); (b) implement an event-based trigger that sets a `needs_refresh` flag and calls the function asynchronously from the application layer. Approach (b) is preferred because it avoids unnecessary refreshes during quiet periods. Phase reference: Phase 16, Phase 22.

### 2.6 `audit_log` Write Protection

**Current state.** `aircraft_prov.audit_log` is append-only by convention. The column `COMMENT ON TABLE` documents this, but there is no DDL-level enforcement preventing `UPDATE` or `DELETE`.

**Deferred decision.** Implement a `BEFORE UPDATE OR DELETE` trigger that raises
an exception, and restrict service-account privileges to INSERT and SELECT as
defence in depth. Do not use a silent `DO INSTEAD NOTHING` rule: callers must be
told that an attempted audit-history mutation was rejected.

Phase reference: Phase 14.

### 2.7 Ownership-Cost Read Model

**Current state.** Migration 016 identifies fuel with
`FUEL_COST_PER_HOUR`, while the Phase 2 seed and Phase 17 mapper use the
canonical code `FUEL`. As a result, `hourly_fuel_cost_usd` is NULL and fuel is
included in `hourly_maintenance_reserve_usd`. The computed annual-total
expression also uses aggregate-level `COALESCE`, which can omit hourly-only
variable items when another variable item supplies `amount_annual`.

**Required fix.** Change the read model to use the seeded `FUEL` code and
compute the annual contribution per line item before summing. Add behavioral
validation containing fuel plus a mix of annual and hourly variable items.
Until that SQL fix is deployed and validated, treat the affected read-model
columns as unreliable.


---

## 3. Curator Workflow Guide

### 3.1 Overview

After Phase 17 promotion runs, some staged aircraft are marked `FLAGGED` rather than `PROMOTED`. A `FLAGGED` record means the variant was still created in canonical tables — no data was lost — but one or more fields produced a parse failure, unmapped key, or ambiguous value. Each flag generates a `curation_flags` row in `aircraft_prov`.

The curator's job is to:
1. Triage open flags
2. Resolve data quality issues by either accepting, rejecting, or superseding source assertions
3. Match stub organizations (engine manufacturers, operators) to canonical organization records
4. Mark resolved flags as `RESOLVED` or `DISMISSED`

### 3.2 Daily Triage Query

Run this at the start of each curator session to see what needs attention:

```sql
-- Open flags by type, newest first
SELECT
    cf.id,
    cf.issue_type,
    cf.entity_type_code,
    cf.entity_id,
    cf.issue_description,
    cf.created_at,
    v.name AS variant_name,
    o.name AS manufacturer
FROM aircraft_prov.curation_flags cf
LEFT JOIN aircraft_core.variants v
       ON cf.entity_type_code = 'AIRCRAFT_VARIANT' AND cf.entity_id = v.id
LEFT JOIN aircraft_core.models m ON m.id = v.model_id
LEFT JOIN aircraft_core.families f ON f.id = m.family_id
LEFT JOIN aircraft_org.organizations o ON o.id = f.manufacturer_org_id
WHERE cf.status_code = 'OPEN'
ORDER BY cf.created_at DESC
LIMIT 50;
```

### 3.3 Resolving an Ingestion Warning Flag

**Scenario:** A variant was flagged because an unmapped performance key appeared in the source JSON (e.g. a future PlanePHD field `glide_ratio`).

**Step 1.** Inspect the flag and any explicitly linked assertion. Current Phase
17 `INGESTION_WARNING` flags set `source_assertion_id` to NULL, so an unmapped
field normally has no assertion to accept yet:

```sql
SELECT cf.*, sa.*
FROM aircraft_prov.curation_flags cf
LEFT JOIN aircraft_prov.source_assertions sa
       ON sa.id = cf.source_assertion_id
WHERE cf.id = :flag_id;
```

**Step 2a. Dismiss** if the value is genuinely irrelevant (e.g. a duplicate of
a mapped field). Reject a linked assertion only when one exists:

```sql
UPDATE aircraft_prov.source_assertions
SET status_code = 'REJECTED', is_accepted = FALSE
WHERE id = (SELECT source_assertion_id
            FROM aircraft_prov.curation_flags
            WHERE id = :flag_id)
  AND status_code = 'PENDING';

UPDATE aircraft_prov.curation_flags
SET status_code = 'DISMISSED',
    resolved_at = now(),
    resolution_notes = 'Field is a duplicate of SPEED_CRUISE_BEST; rejected.'
WHERE id = :flag_id;
```

**Step 2b. Promote** only after the field maps to a metric type that has been
added to the canonical Phase 2 seed and deployed. Perform the canonical write,
provenance write, flag resolution, and audit write atomically:

```sql
BEGIN;

-- 1. Insert the canonical value (example: glide_ratio -> a new metric type).
INSERT INTO aircraft_specs.performance_metrics
    (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
     is_canonical, confidence)
VALUES
    (:variant_id, 'GLIDE_RATIO', :parsed_value, NULL, :parsed_value,
     TRUE, 0.20);

-- 2. Record the accepted assertion. Ingestion does not create this row for an
-- unmapped field, so the curator must supply the source document. For a mapped
-- field, prefer `aircraft-ingest curate accept` over hand-written SQL.
INSERT INTO aircraft_prov.source_assertions
    (source_document_id, entity_type_code, entity_id, field_name,
     raw_value, asserted_value, asserted_numeric, status_code,
     is_accepted, confidence)
VALUES
    (:source_document_id, 'AIRCRAFT_VARIANT', :variant_id,
     'performance.GLIDE_RATIO', :raw_value, :parsed_value::text,
     :parsed_value, 'ACCEPTED', TRUE, 0.20);

-- 3. Resolve the flag.
UPDATE aircraft_prov.curation_flags
SET status_code = 'RESOLVED', resolved_at = now(),
    resolution_notes = 'Mapped to new GLIDE_RATIO metric type and promoted.'
WHERE id = :flag_id;

-- 4. Record the curator action.
INSERT INTO aircraft_prov.audit_log
    (entity_type_code, entity_id, field_name, new_value,
     change_source, changed_by, change_reason)
VALUES
    ('AIRCRAFT_VARIANT', :variant_id, 'performance.GLIDE_RATIO',
     :parsed_value::text, 'CURATOR', current_user,
     'Mapped previously unmapped source field to GLIDE_RATIO');

COMMIT;

-- If a read model is extended to expose GLIDE_RATIO, refresh it only after
-- the transaction commits:
-- SELECT aircraft_read.refresh_search_matviews();
```

### 3.4 Resolving Conflicting Source Assertions

**Scenario:** A second source provides a different cruise speed for a variant already in the database. PlanePHD says 72 KIAS; the POH says 75 KIAS.

The `is_accepted` partial UNIQUE index enforces that only one assertion per `(entity_type, entity_id, field_name)` can be `is_accepted = TRUE` at a time. To switch the canonical value:

```sql
BEGIN;

-- Step 1: Demote the current accepted assertion
UPDATE aircraft_prov.source_assertions
SET status_code = 'SUPERSEDED',
    is_accepted = FALSE
WHERE entity_type_code = 'AIRCRAFT_VARIANT'
  AND entity_id        = :variant_id
  AND field_name       = 'performance.SPEED_CRUISE_BEST'
  AND is_accepted      = TRUE;

-- Step 2: Accept the new higher-confidence assertion
UPDATE aircraft_prov.source_assertions
SET status_code = 'ACCEPTED',
    is_accepted = TRUE
WHERE id = :new_assertion_id;

-- Step 3: Update the canonical performance_metrics row
UPDATE aircraft_specs.performance_metrics
SET raw_value       = :new_raw_value,
    raw_unit_code   = :new_unit_code,
    canonical_value = aircraft_ref.to_canonical(:new_raw_value, :new_unit_code),
    confidence      = :new_confidence
WHERE variant_id        = :variant_id
  AND metric_type_code  = 'SPEED_CRUISE_BEST'
  AND is_canonical      = TRUE;

-- Step 4: Log the change
INSERT INTO aircraft_prov.audit_log
    (entity_type_code, entity_id, field_name, old_value, new_value,
     change_source, changed_by, change_reason)
VALUES (
    'AIRCRAFT_VARIANT', :variant_id, 'performance.SPEED_CRUISE_BEST',
    '72 KIAS', '75 KIAS',
    'CURATOR', current_user, 'Accepted higher-confidence POH assertion'
);

COMMIT;
```

### 3.5 Matching Engine Manufacturer Stubs

After ingestion, run this query weekly to surface unmatched engine manufacturers and attempt fuzzy resolution:

```sql
-- Candidates: unmatched engine manufacturer stubs with similarity scores
SELECT
    ev.manufacturer_name_raw,
    COUNT(ev.id)           AS engine_variants,
    o.id                   AS candidate_org_id,
    o.name             AS candidate_name,
    similarity(ev.manufacturer_name_raw, o.name) AS sim_score
FROM aircraft_power.engine_variants ev
CROSS JOIN aircraft_org.organizations o
WHERE ev.manufacturer_org_id IS NULL
  AND o.org_type_code IN ('MANUFACTURER', 'DESIGN_BUREAU')
  AND similarity(ev.manufacturer_name_raw, o.name) > 0.3
GROUP BY ev.manufacturer_name_raw, o.id, o.name
ORDER BY ev.manufacturer_name_raw, sim_score DESC;
```

When a match is confirmed:
```sql
UPDATE aircraft_power.engine_variants
SET manufacturer_org_id  = :confirmed_org_id,
    manufacturer_name_raw = NULL   -- clear stub once matched
WHERE manufacturer_name_raw = :raw_name
  AND manufacturer_org_id IS NULL;
```

### 3.6 Adding a New Lookup Value

When a new aircraft role, propulsion type, or cost category appears that is not in the seed data:

```sql
-- Example: adding a new propulsion category for hydrogen fuel cell
INSERT INTO aircraft_ref.propulsion_categories
    (code, label, is_jet, is_rotating, description, sort_order)
VALUES
    ('HYDROGEN_FC', 'Hydrogen Fuel Cell', FALSE, FALSE,
     'Electric propulsion powered by hydrogen fuel cell', 99)
ON CONFLICT (code) DO NOTHING;

-- Annotate the seed file for reproducibility
-- Add an equivalent INSERT to database/seeds/002_lookup_seed_data.sql
-- with a comment block: -- Phase 22 addition: YYYY-MM-DD
```

---

## 4. Performance Tuning Notes

### 4.1 Index Strategy Summary

| Table | Key Index | Type | Purpose |
|---|---|---|---|
| `aircraft_core.variants` | `description_tsv` | GIN | Full-text search |
| `aircraft_core.families` | `name_tsv` | GIN | Full-text search |
| `aircraft_core.variant_aliases` | `alias_text` | `gin_trgm_ops` | Fuzzy alias search |
| `aircraft_org.organizations` | `name_aliases` | GIN | Array `@>` alias resolution |
| `aircraft_specs.performance_metrics` | `(variant_id, metric_type_code) WHERE is_canonical` | Unique B-tree partial | `uq_perf_canonical`; enforces one canonical metric |
| `aircraft_specs.performance_metrics` | `(variant_id, metric_type_code) WHERE is_canonical` | B-tree partial | `idx_pm_variant_canonical_all`; supports read-model aggregation |
| `aircraft_read.mv_variant_search` | `search_vector` | GIN | Full-text search on matview |
| `aircraft_read.mv_variant_search` | 20 indexes total | GIN and B-tree | Three GIN/trigram indexes and 17 B-tree/partial indexes |
| `aircraft_prov.source_assertions` | `(entity_type_code, entity_id, field_name) WHERE is_accepted` | Unique B-tree partial | `uq_assertion_accepted`; canonical assertion lookup |
| `aircraft_power.engine_variants` | `name_aliases` | GIN | Ingestion alias resolution |

### 4.2 Query Plans to Watch

**Faceted search on `mv_variant_search`:** Postgres will often choose a bitmap index scan combining multiple partial indexes. If the query planner falls back to a sequential scan on the matview, check that `enable_seqscan = TRUE` (it should be the default) and that `ANALYZE aircraft_read.mv_variant_search` has been run after the last refresh. The matview has 48 columns; a seqscan on a large dataset (>50,000 rows) will be significantly slower than the bitmap scan.

**`to_canonical()` in WHERE clauses:** The function is `STABLE`, meaning Postgres can fold constant-unit calls into index expressions. However, using `aircraft_ref.to_canonical(pm.raw_value, pm.raw_unit_code) > 100` in a WHERE clause will not use an index on `canonical_value`. Always filter on the stored `canonical_value` column:
```sql
-- Correct (uses index on canonical_value)
WHERE pm.canonical_value > 100 AND pm.metric_type_code = 'SPEED_CRUISE_BEST' AND pm.is_canonical

-- Wrong (function call prevents index use)
WHERE aircraft_ref.to_canonical(pm.raw_value, pm.raw_unit_code) > 100
```

**JSONB `extra_attributes` queries:** Avoid containment queries such as
`WHERE extra_attributes @> '{"key": "value"}'` on large tables without a GIN
index. Of the three commonly queried `extra_attributes` columns, only
`aircraft_core.variants` currently has a GIN index. `aircraft_market.cost_snapshots` and
`aircraft_power.engine_variants` do not. Add a targeted GIN index only after a
measured query requirement justifies it.

### 4.3 Materialized View Refresh Performance

`mv_variant_search` is expected to be the most expensive object in the schema to
refresh because it joins many normalized sources and aggregates metrics. The
repository contains no recorded refresh benchmark, so refresh-time ratios and
production thresholds must be measured against representative data rather than
assumed.

- **Initial population** (`FALSE` / non-concurrent): blocks all reads on the matview during refresh. Acceptable only at bootstrap or during a maintenance window.
- **Concurrent refresh** (`TRUE`): requires a `UNIQUE` index on the matview. Phase 16 creates `uq_mvvs_variant_id` for this purpose. It avoids blocking reads, but its runtime overhead must be benchmarked with representative data.
- **Partial refresh (future):** PostgreSQL does not support partial matview refresh natively. If refresh latency becomes a problem at scale, consider splitting `mv_variant_search` into a `mv_variant_core` (stable identity data, refreshed rarely) and `mv_variant_metrics` (spec data, refreshed frequently) and JOINing them in the read layer.

### 4.4 `aircraft_prov.source_assertions` at Scale

At one assertion per canonical field per variant, with ~40
performance/weight/cost fields per aircraft, 10,000 variants will produce
~400,000 assertion rows. At 100,000 variants, this reaches 4 million rows. These
are planning estimates, not measured production volumes.

Phase 14 already defines:

- `idx_sa_entity (entity_type_code, entity_id)`;
- `idx_sa_entity_field (entity_type_code, entity_id, field_name)`; and
- partial unique `uq_assertion_accepted` on the same three columns where
  `is_accepted` is true.

Do not add another accepted entity/field index; it would duplicate
`uq_assertion_accepted`. If measured cross-entity queries by `field_name` alone
become slow, consider the following distinct index:

```sql
CREATE INDEX idx_sa_field ON aircraft_prov.source_assertions (field_name);
```

### 4.5 `pg_trgm` Similarity Search Threshold

The default `pg_trgm.similarity_threshold` is `0.3`. For aircraft name searches this may return too many false positives (e.g. "Beechcraft" matching "Beagle"). Raise the threshold per-session for name-specific searches:

```sql
SET pg_trgm.similarity_threshold = 0.5;
SELECT * FROM aircraft_read.mv_variant_search
WHERE variant_name % 'Bonanza';
```

For manufacturer alias resolution in the ingestion pipeline, keep the threshold at `0.3` to catch abbreviations (`Cont Motor` → `Continental Motors`), but always require curator confirmation before committing the match.

### 4.6 Recommended Autovacuum Settings

The following tables have high insert rates during bulk ingestion and moderate update rates during curation. Consider explicit autovacuum tuning for production:

```sql
-- source_assertions: high insert volume, low update rate
ALTER TABLE aircraft_prov.source_assertions SET (
    autovacuum_vacuum_scale_factor = 0.01,
    autovacuum_analyze_scale_factor = 0.005
);

-- mv_variant_search: rebuilt on each refresh, not incrementally updated
-- autovacuum is less relevant; manual VACUUM ANALYZE after refresh is sufficient
```

---

## Appendix: File Inventory

| Area | Files | Purpose |
|---|---|---|
| Local legacy reconciliation | database/reconcile_local_legacy.sql | Verify and adopt complete pre-ledger Phase 1/2 local schemas; reject partial states |
| Installer | database/install.sql | Dependency-aware migration and canonical-seed orchestration |
| Migrations | database/migrations/001_*.sql through 021_*.sql | Canonical ordered schema history; immutable once written, hashed in migrations.lock.json |
| Canonical seeds | database/seeds/001_*.sql through 003_*.sql | Units, lookup data, and mission profiles |
| Ingestion | apps/ingest + database/snapshots/ | Rust CLI import, snapshot queries, and committed golden output |
| Verification | database/validation/000_migration_history_validation.sql and remaining database/validation/*.sql | Exact 001-021 ledger assertion plus phase-specific structural and behavioral checks |
| Documentation | database/README.md, data_dictionary.md, implementation_notes.md | Lifecycle, schema meaning, and operational guidance |
