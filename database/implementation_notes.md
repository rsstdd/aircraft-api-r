# Aircraft Encyclopedia — Implementation Notes

---

## 1. Migration Run Order and Dependency Map

### 1.1 Complete Install Sequence

Run from the repository root. All commands use `-v ON_ERROR_STOP=1` so any failure halts the pipeline rather than silently continuing with a broken state.

```bash
# ── Phase 1: Infrastructure ────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/001_extensions_schemas_domains_triggers.sql

# ── Phase 2: Lookup tables + seed data ────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/002_core_reference_tables.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/seeds/001_reference_units.sql       # MUST precede 001
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/seeds/002_lookup_seed_data.sql

# ── Phase 3: Geography + organizations ────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/003_geography_operators_organizations.sql

# ── Phase 4: Aircraft identity backbone ───────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/004_aircraft_identity_taxonomy.sql

# ── Phase 5: Certification ────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/005_certification_operating_approvals.sql

# ── Phase 6: Dimensions ───────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/006_dimensions_cabin_cargo_hangar_fit.sql

# ── Phase 7: Weights ──────────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/007_weight_balance_payload_loading.sql

# ── Phase 8: Performance metrics ──────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/008_performance_metrics_conditions.sql

# ── Phase 9: Propulsion ───────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/009_propulsion_engines_rotors_stcs.sql

# ── Phase 10: Avionics / systems ──────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/010_avionics_equipment_systems.sql

# ── Phase 11: Military reference ──────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/011_military_sensors_stores_loadouts.sql

# ── Phase 12: Market data ─────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/012_ownership_cost_valuation_market.sql

# ── Phase 13: Maintenance / reliability ───────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/013_maintenance_reliability_supportability.sql

# ── Phase 14: Provenance / curation ──────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/014_sources_provenance_curation_audit.sql

# ── Phase 15: Mission profiles + seed ─────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/015_mission_profiles_comparison_scoring.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/seeds/003_mission_profile_seed_data.sql

# ── Phase 16: Read models / views / indexes ───────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/016_read_models_views_indexes.sql

# ── Phase 17: Ingestion pipeline ──────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/901_seed_data_staging.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -v seed_json_path="'/absolute/path/to/aircraft_seed.json'" \
  -f database/migrations/902_server_side_json_ingestion.sql

# ── Populate materialized views (after promotion completes) ───────────────
psql "$DATABASE_URL" -c "SELECT aircraft_read.refresh_search_matviews(FALSE);"

# ── Phase 18: Example / smoke-test queries ────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/validation/002_comparison_query_smoke_tests.sql

# ── Phase 19: Validation ──────────────────────────────────────────────────
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/validation/001_integrity_checks.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/validation/003_seed_ingestion_validation.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -f database/migrations/903_post_bootstrap_validation.sql

# ── Phases 20–21: Documentation only ─────────────────────────────────────
# docs/data_dictionary.md and docs/implementation_notes.md — no psql step
```

### 1.2 Hard Dependency Rules

The following constraints are **not enforced by Postgres itself** but must be respected or the pipeline will fail with FK errors:

| Rule | Reason |
|---|---|
| `001_reference_units.sql` before `002_lookup_seed_data.sql` | `performance_metric_types`, `weight_metric_types`, and `propulsion_categories` have FK into `measurement_units` |
| All of Phase 2 before Phase 3 | `org_type_code`, `org_relationship_types` consumed by Phase 3 |
| Phase 3 before Phase 4 | `aircraft_core.families.manufacturer_org_id` → `aircraft_org.organizations` |
| Phase 4 before Phases 5–15 | All domain tables FK to `aircraft_core.variants.id` |
| Phase 14 before Phase 17 promotion | Promotion writes to `aircraft_prov.source_documents` and `source_assertions` |
| Phase 16 before matview population | `mv_variant_search` and `mv_ownership_cost_summary` must exist before `refresh_search_matviews()` |
| Matview refresh before Phase 18 smoke tests | Family 4 (mission suitability) queries `mv_variant_search` |

### 1.3 Re-running Migrations Safely

Every migration file uses `CREATE TABLE IF NOT EXISTS`, `CREATE OR REPLACE FUNCTION`, `INSERT ... ON CONFLICT DO NOTHING`, and is wrapped in `BEGIN ... COMMIT`. This means:

- Re-running a migration file after a partial failure is safe
- The seed files are idempotent (all INSERTs use `ON CONFLICT DO NOTHING`)
- Phase 17 `load_seed_json()` deduplicates on `(run_label)` and `(ingest_run_id, manufacturer_name_raw, aircraft_name_raw)` — running with the same JSON file twice will not create duplicate staged rows

---

## 2. Known Limitations and Deferred Decisions

### 2.1 Performance Model: Single Source, Single Condition

**Current state.** Phase 17 ingestion inserts one `performance_metrics` row per metric type per variant with `is_canonical = TRUE` on the first write. The `is_canonical` partial UNIQUE index enforces only one canonical row per `(variant_id, metric_type_code)`.

**Limitation.** PlanePHD provides only one performance figure per metric (e.g. one cruise speed value). The schema supports multiple test conditions and multiple sources per metric, but the ingestion pipeline does not yet populate `condition_weight_lbs`, `condition_altitude_ft`, or `condition_power_setting` because PlanePHD does not expose that data.

**Deferred decision.** When a second source (e.g. the FAA TCDS or a manufacturer POH) is ingested, it may assert different performance values under different conditions. The promotion pipeline's `is_canonical` switching logic will need a curator decision step: either automatically accept the higher-confidence source's value, or surface a `curation_flags` row for manual review. Recommended approach: implement a `promote_assertion(assertion_id BIGINT)` function that atomically flips `is_accepted` and updates the canonical table value. Phase reference: Phase 14 / Phase 17.

### 2.2 Engine Manufacturer Matching

**Current state.** The Phase 17 promotion step creates `engine_variants` rows with `manufacturer_org_id = NULL` and `manufacturer_name_raw` set to the raw source string (e.g. `'Cont Motor'`, `'Ivchenko'`). A GIN-indexed `name_aliases TEXT[]` column on `aircraft_org.organizations` supports alias resolution, but the Phase 3 seed data only includes ~27 organizations.

**Limitation.** The source data uses abbreviated and variant manufacturer names (`Cont Motor` for Continental Motors, `Lyc` for Lycoming). Until a curator runs the matching pass, engine variants remain unlinked from their organization records.

**Deferred decision.** A Phase 22 curator tool should provide a `match_engine_manufacturers()` function that: (1) queries `engine_variants WHERE manufacturer_org_id IS NULL`, (2) attempts fuzzy-match against `organizations.name_aliases` using `pg_trgm` similarity, (3) presents candidates above a threshold similarity score for curator confirmation, (4) writes the matched `manufacturer_org_id` and raises no flag. Phase reference: Phase 9.

### 2.3 Ownership Cost Fuel Price Extraction

**Current state.** The `cost_snapshots.assumed_fuel_price_per_gal` column is left NULL during Phase 17 ingestion. The fuel price is embedded in the dynamic cost key name (e.g. `fuel_cost_per_hour_3_5_gallons_hr_5_40_gal` implies $5.40/gal) but extracting it requires parsing a third embedded number from an already-complex key pattern.

**Limitation.** Without the fuel price assumption, per-hour variable cost calculations cannot be verified or re-run at a different fuel price.

**Deferred decision.** Add a `parse_fuel_price_from_key(TEXT)` helper function in Phase 22 that uses `regexp_match(key, '_hr_([0-9]+_[0-9]+)_gal')` to extract the price token and converts the underscore-delimited decimal (`5_40` → `5.40`). Run as a curation pass on all `cost_snapshots WHERE assumed_fuel_price_per_gal IS NULL`. Phase reference: Phase 17.

### 2.4 Dimension and Hangar Fit Data

**Current state.** `aircraft_specs.dimension_metrics` and `aircraft_read.v_hangar_fit` are fully defined and indexed. However, PlanePHD does not expose wingspan, length, or height data in the JSON seed format used for Phase 17. The `v_hangar_fit` view will return zero rows until this data is populated from a secondary source.

**Deferred decision.** Identify a secondary source (FAA type certificate data sheets, manufacturer specifications, or a structured Wikipedia/DBpedia extract) and build a Phase 22 ingestion adapter. The `aircraft_prov` pipeline is ready to accept multi-source dimension data with separate confidence scores. Phase reference: Phase 6, Phase 14.

### 2.5 Materialized View Refresh Cadence

**Current state.** `refresh_search_matviews()` is a manually called function. There is no automated trigger or scheduled job.

**Limitation.** After any write to canonical tables (new valuation snapshot, updated performance metric, accepted assertion), `mv_variant_search` becomes stale. In a read-heavy production environment, stale search results are visible to users until the function is called.

**Deferred decision.** Two viable approaches: (a) Schedule `refresh_search_matviews()` via `pg_cron` on a fixed cadence (e.g. every 15 minutes); (b) implement an event-based trigger that sets a `needs_refresh` flag and calls the function asynchronously from the application layer. Approach (b) is preferred because it avoids unnecessary refreshes during quiet periods. Phase reference: Phase 16, Phase 22.

### 2.6 `audit_log` Write Protection

**Current state.** `aircraft_prov.audit_log` is append-only by convention. The column `COMMENT ON TABLE` documents this, but there is no DDL-level enforcement preventing `UPDATE` or `DELETE`.

**Deferred decision.** Implement a PostgreSQL row-level security (RLS) policy or a `BEFORE UPDATE/DELETE` trigger that raises `EXCEPTION` if any row modification is attempted. Example:

```sql
CREATE RULE audit_log_no_update AS ON UPDATE TO aircraft_prov.audit_log DO INSTEAD NOTHING;
CREATE RULE audit_log_no_delete AS ON DELETE TO aircraft_prov.audit_log DO INSTEAD NOTHING;
```

Phase reference: Phase 14.

### 2.7 `mv_variant_search` ADS-B Column

**Current state.** The `has_ads_b_out` column in `mv_variant_search` maps to the `IFR` operating approval type code as a placeholder. This is documented inline in Phase 16. ADS-B equipment is correctly modeled in `aircraft_systems.variant_equipment` (linked to the `ADS-B OUT TRANSPONDER` equipment catalog entry seeded in Phase 10).

**Deferred decision.** After real equipment data is loaded for a meaningful number of variants, update the `mv_variant_search` definition to use:

```sql
EXISTS (
    SELECT 1 FROM aircraft_systems.variant_equipment ve
    JOIN aircraft_systems.equipment_catalog ec ON ec.id = ve.equipment_id
    WHERE ve.variant_id = v.id
      AND ec.slug = 'ads-b-out-transponder'
) AS has_ads_b_out
```

Phase reference: Phase 10, Phase 16.

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
    cf.flag_type,
    cf.entity_type_code,
    cf.entity_id,
    cf.description,
    cf.created_at,
    v.variant_name,
    f.family_name AS manufacturer
FROM aircraft_prov.curation_flags cf
LEFT JOIN aircraft_core.variants v
       ON cf.entity_type_code = 'VARIANT' AND cf.entity_id = v.id
LEFT JOIN aircraft_core.models m ON m.id = v.model_id
LEFT JOIN aircraft_core.families f ON f.id = m.family_id
WHERE cf.status_code = 'OPEN'
ORDER BY cf.created_at DESC
LIMIT 50;
```

### 3.3 Resolving an Ingestion Warning Flag

**Scenario:** A variant was flagged because an unmapped performance key appeared in the source JSON (e.g. a future PlanePHD field `glide_ratio`).

**Step 1.** Identify the source assertion:
```sql
SELECT sa.*
FROM aircraft_prov.source_assertions sa
JOIN aircraft_prov.curation_flags cf
     ON cf.entity_id = sa.entity_id
    AND cf.entity_type_code = sa.entity_type_code
WHERE cf.id = :flag_id
  AND sa.status_code = 'PENDING';
```

**Step 2a. Dismiss** if the value is genuinely irrelevant (e.g. a duplicate of a mapped field):
```sql
UPDATE aircraft_prov.source_assertions
SET status_code = 'REJECTED', is_accepted = FALSE
WHERE id = :assertion_id;

UPDATE aircraft_prov.curation_flags
SET status_code = 'DISMISSED',
    resolved_at = now(),
    resolution_notes = 'Field is a duplicate of CRUISE_SPEED; rejected.'
WHERE id = :flag_id;
```

**Step 2b. Accept and promote** if the field maps to a known metric type:
```sql
-- 1. Accept the assertion
UPDATE aircraft_prov.source_assertions
SET status_code = 'ACCEPTED', is_accepted = TRUE
WHERE id = :assertion_id;

-- 2. Insert the canonical value (example: glide_ratio → a new metric type)
INSERT INTO aircraft_specs.performance_metrics
    (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
     is_canonical, confidence, source_document_id)
VALUES
    (:variant_id, 'GLIDE_RATIO', :parsed_value, NULL, :parsed_value,
     TRUE, 0.20, :source_document_id)
ON CONFLICT DO NOTHING;

-- 3. Resolve the flag
UPDATE aircraft_prov.curation_flags
SET status_code = 'RESOLVED', resolved_at = now(),
    resolution_notes = 'Mapped to new GLIDE_RATIO metric type and promoted.'
WHERE id = :flag_id;

-- 4. Refresh search matview to include the new data
SELECT aircraft_read.refresh_search_matviews();
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
WHERE entity_type_code = 'VARIANT'
  AND entity_id        = :variant_id
  AND field_name       = 'performance.CRUISE_SPEED'
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
    confidence      = :new_confidence,
    source_document_id = :new_doc_id
WHERE variant_id        = :variant_id
  AND metric_type_code  = 'CRUISE_SPEED'
  AND is_canonical      = TRUE;

-- Step 4: Log the change
INSERT INTO aircraft_prov.audit_log
    (entity_type_code, entity_id, action, old_value, new_value, changed_by)
VALUES (
    'VARIANT', :variant_id,
    'CANONICAL_VALUE_CHANGED',
    jsonb_build_object('field', 'performance.CRUISE_SPEED',
                       'old_value', '72 KIAS', 'old_source', 'PlanePHD'),
    jsonb_build_object('field', 'performance.CRUISE_SPEED',
                       'new_value', '75 KIAS', 'new_source', 'POH Rev 3'),
    current_user
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
    o.org_name             AS candidate_name,
    similarity(ev.manufacturer_name_raw, o.org_name) AS sim_score
FROM aircraft_power.engine_variants ev
CROSS JOIN aircraft_org.organizations o
WHERE ev.manufacturer_org_id IS NULL
  AND o.org_type_code IN ('MANUFACTURER', 'DESIGN_BUREAU')
  AND similarity(ev.manufacturer_name_raw, o.org_name) > 0.3
GROUP BY ev.manufacturer_name_raw, o.id, o.org_name
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
| `aircraft_specs.performance_metrics` | `(variant_id, metric_type_code) WHERE is_canonical` | B-tree partial | Canonical comparison queries |
| `aircraft_specs.performance_metrics` | `(variant_id, metric_type_code) WHERE is_canonical` | B-tree partial | Named `idx_pm_variant_canonical_all` — feeds `mv_variant_search` FILTER aggregation |
| `aircraft_read.mv_variant_search` | `search_vector` | GIN | Full-text search on matview |
| `aircraft_read.mv_variant_search` | 20 B-tree columns | B-tree | Faceted filter combinations |
| `aircraft_prov.source_assertions` | `(entity_type, entity_id, field_name) WHERE is_accepted` | B-tree partial | Canonical assertion lookup |
| `aircraft_power.engine_variants` | `name_aliases` | GIN | Ingestion alias resolution |

### 4.2 Query Plans to Watch

**Faceted search on `mv_variant_search`:** Postgres will often choose a bitmap index scan combining multiple partial indexes. If the query planner falls back to a sequential scan on the matview, check that `enable_seqscan = TRUE` (it should be the default) and that `ANALYZE aircraft_read.mv_variant_search` has been run after the last refresh. The matview has approximately 45 columns; a seqscan on a large dataset (>50,000 rows) will be significantly slower than the bitmap scan.

**`to_canonical()` in WHERE clauses:** The function is `STABLE`, meaning Postgres can fold constant-unit calls into index expressions. However, using `aircraft_ref.to_canonical(pm.raw_value, pm.raw_unit_code) > 100` in a WHERE clause will not use an index on `canonical_value`. Always filter on the stored `canonical_value` column:
```sql
-- Correct (uses index on canonical_value)
WHERE pm.canonical_value > 100 AND pm.metric_type_code = 'CRUISE_SPEED' AND pm.is_canonical

-- Wrong (function call prevents index use)
WHERE aircraft_ref.to_canonical(pm.raw_value, pm.raw_unit_code) > 100
```

**JSONB `extra_attributes` queries:** Avoid `WHERE extra_attributes @> '{"key": "value"}'` on large tables without a GIN index. `aircraft_core.variants`, `aircraft_market.cost_snapshots`, and `aircraft_power.engine_variants` all have GIN indexes on their JSONB columns. Other tables do not. Add targeted GIN indexes before running JSONB-based analytics queries on tables added in Phase 22+.

### 4.3 Materialized View Refresh Performance

`mv_variant_search` is the most expensive object in the schema to refresh because it joins 14 tables and aggregates metrics across potentially millions of rows. Benchmark observations from the design session:

- **Initial population** (`FALSE` / non-concurrent): blocks all reads on the matview during refresh. Acceptable only at bootstrap or during a maintenance window.
- **Concurrent refresh** (`TRUE`): requires a `UNIQUE` index on the matview. Phase 16 creates `uq_mvvs_variant_id` for this purpose. Concurrent refresh takes approximately 2–3× longer than non-concurrent but does not block reads.
- **Partial refresh (future):** PostgreSQL does not support partial matview refresh natively. If refresh latency becomes a problem at scale, consider splitting `mv_variant_search` into a `mv_variant_core` (stable identity data, refreshed rarely) and `mv_variant_metrics` (spec data, refreshed frequently) and JOINing them in the read layer.

### 4.4 `aircraft_prov.source_assertions` at Scale

At one assertion per canonical field per variant, with ~40 performance/weight/cost fields per aircraft, 10,000 variants will produce ~400,000 assertion rows. At 100,000 variants, this reaches 4 million rows — well within Postgres capacity, but the following indexes become critical:

```sql
-- Already defined in Phase 14:
CREATE INDEX idx_sa_entity ON aircraft_prov.source_assertions (entity_type_code, entity_id);
CREATE INDEX idx_sa_field  ON aircraft_prov.source_assertions (field_name);
```

If provenance queries slow down at scale, consider adding a composite index:
```sql
CREATE INDEX idx_sa_entity_field ON aircraft_prov.source_assertions
    (entity_type_code, entity_id, field_name)
    WHERE is_accepted = TRUE;
```
This directly serves the most common provenance lookup: "what is the accepted assertion for this field on this entity?"

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

| File | Phase | Lines | Purpose |
|---|---|---|---|
| `001_extensions_schemas_domains_triggers.sql` | 1 | ~130 | Extensions, schemas, domains, utility functions |
| `002_core_reference_tables.sql` | 2 | 607 | 35 lookup tables |
| `001_reference_units.sql` (seed) | 2 | 261 | 15 unit categories + 38 units |
| `002_lookup_seed_data.sql` (seed) | 2 | 953 | ~305 lookup rows across 33 tables |
| `003_geography_operators_organizations.sql` | 3 | 823 | Countries, regions, organizations + seed |
| `004_aircraft_identity_taxonomy.sql` | 4 | 543 | 7 identity tables + generated tsvector columns |
| `005_certification_operating_approvals.sql` | 5 | 410 | 6 certification tables |
| `006_dimensions_cabin_cargo_hangar_fit.sql` | 6 | 372 | 3 dimension tables + `to_canonical()` |
| `007_weight_balance_payload_loading.sql` | 7 | 291 | 3 weight/CG tables |
| `008_performance_metrics_conditions.sql` | 8 | 319 | Performance fact table + runway limits |
| `009_propulsion_engines_rotors_stcs.sql` | 9 | 453 | 7 propulsion tables |
| `010_avionics_equipment_systems.sql` | 10 | 377 | 5 systems tables + 18 catalog seed rows |
| `011_military_sensors_stores_loadouts.sql` | 11 | 502 | 7 military tables + 17 weapons seed rows |
| `012_ownership_cost_valuation_market.sql` | 12 | 313 | 3 market tables |
| `013_maintenance_reliability_supportability.sql` | 13 | 394 | 6 maintenance tables |
| `014_sources_provenance_curation_audit.sql` | 14 | 436 | 5 provenance tables + 3 source seed rows |
| `015_mission_profiles_comparison_scoring.sql` | 15 | ~290 | 4 comparison tables |
| `003_mission_profile_seed_data.sql` (seed) | 15 | ~180 | 15 profiles + 27 criteria |
| `016_read_models_views_indexes.sql` | 16 | ~420 | 3 views + 2 matviews + 25 indexes |
| `901_seed_data_staging.sql` | 17a | 192 | 3 staging tables |
| `902_server_side_json_ingestion.sql` | 17b | 1,148 | 11 parsers + load + promote functions |
| `903_post_bootstrap_validation.sql` | 17c/19 | 553 | 10 validation blocks + 7 invariants |
| `002_comparison_query_smoke_tests.sql` (val) | 18 | ~310 | 5 query families, 17 queries |
| `001_integrity_checks.sql` (val) | 19 | ~180 | Structural integrity monitoring |
| `003_seed_ingestion_validation.sql` (val) | 19 | ~200 | Known-value spot checks |
| `data_dictionary.md` | 20 | — | This document's companion |
| `implementation_notes.md` | 21 | — | This document |
| **Total migration lines** | | **~8,725** | |
