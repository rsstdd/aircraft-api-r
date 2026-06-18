-- =============================================================================
-- File: database/migrations/903_post_bootstrap_validation.sql
-- Phase 17c / Phase 19: Post-bootstrap validation.
--
-- Run AFTER 902 has completed (load + promotion both finished).
-- All queries are read-only SELECT statements that emit pass/fail signals.
-- A result set with any FAIL rows should be investigated before the database
-- is opened for production queries.
--
-- Structure:
--   Block 1  — Staging integrity (counts, status distribution)
--   Block 2  — Promotion completeness (no silent loss)
--   Block 3  — Canonical table coverage (variants, engines, costs)
--   Block 4  — Parsing quality (sentinel handling, unit mapping)
--   Block 5  — Provenance integrity (source_assertions coverage)
--   Block 6  — Curation flag summary (items needing curator attention)
--   Block 7  — Image staging integrity
--   Block 8  — Ingest run final state
--   Block 9  — Smoke-test sample output (human-readable spot check)
--   Block 10 — DO block: assert hard invariants (raises EXCEPTION on failure)
-- =============================================================================

-- =============================================================================
-- BLOCK 1: Staging integrity
-- =============================================================================

-- 1a. Status distribution across all staged_aircraft rows.
-- Expect: no rows in PENDING after promotion runs.
SELECT
    stage_status,
    COUNT(*)                                AS row_count,
    CASE stage_status
        WHEN 'PENDING'   THEN 'FAIL — promotion did not run or failed early'
        WHEN 'PROMOTED'  THEN 'OK   — clean auto-promote'
        WHEN 'FLAGGED'   THEN 'WARN — promoted with curation flags; curator review needed'
        WHEN 'SKIPPED'   THEN 'INFO — duplicates suppressed'
        ELSE                  'UNKNOWN'
    END                                     AS check_result
FROM aircraft_ingest.staged_aircraft
GROUP BY stage_status
ORDER BY stage_status;

-- 1b. Verify no staged rows are missing their ingest_run foreign key.
-- Expect: 0 rows.
SELECT COUNT(*) AS orphaned_staged_rows_expect_0
FROM aircraft_ingest.staged_aircraft sa
LEFT JOIN aircraft_ingest.ingest_runs ir ON ir.id = sa.ingest_run_id
WHERE ir.id IS NULL;

-- 1c. Manufacturer distribution: staged aircraft count per manufacturer.
-- Cross-check this against the source JSON's outer-key counts.
SELECT
    manufacturer_name_raw,
    COUNT(*)                    AS aircraft_count,
    SUM(CASE WHEN stage_status = 'PROMOTED' THEN 1 ELSE 0 END)  AS promoted,
    SUM(CASE WHEN stage_status = 'FLAGGED'  THEN 1 ELSE 0 END)  AS flagged,
    SUM(CASE WHEN stage_status = 'PENDING'  THEN 1 ELSE 0 END)  AS pending
FROM aircraft_ingest.staged_aircraft
GROUP BY manufacturer_name_raw
ORDER BY manufacturer_name_raw;

-- =============================================================================
-- BLOCK 2: Promotion completeness — no silent loss
-- =============================================================================

-- 2a. Every staged PROMOTED/FLAGGED row must have a non-NULL variant_id.
-- Expect: 0 rows.
SELECT id, manufacturer_name_raw, aircraft_name_raw, stage_status, promotion_notes
FROM aircraft_ingest.staged_aircraft
WHERE stage_status IN ('PROMOTED', 'FLAGGED')
  AND variant_id IS NULL
ORDER BY manufacturer_name_raw, aircraft_name_raw;

-- 2b. Every staged row's variant_id must reference a real aircraft_core.variants row.
-- Expect: 0 rows.
SELECT sa.id, sa.manufacturer_name_raw, sa.aircraft_name_raw
FROM aircraft_ingest.staged_aircraft sa
LEFT JOIN aircraft_core.variants v ON v.id = sa.variant_id
WHERE sa.variant_id IS NOT NULL
  AND v.id IS NULL;

-- 2c. Count of variants created vs staged (promoted + flagged).
-- Expect: these numbers to be equal or close (duplicates suppress variant creation).
SELECT
    (SELECT COUNT(*) FROM aircraft_ingest.staged_aircraft
     WHERE stage_status IN ('PROMOTED','FLAGGED'))  AS staged_promoted_plus_flagged,
    (SELECT COUNT(DISTINCT variant_id)
     FROM aircraft_ingest.staged_aircraft
     WHERE variant_id IS NOT NULL)                  AS distinct_variants_linked,
    (SELECT COUNT(*) FROM aircraft_core.variants)   AS total_canonical_variants;

-- =============================================================================
-- BLOCK 3: Canonical table coverage
-- =============================================================================

-- 3a. Variants without any performance_metrics rows.
-- These are legitimately possible (L-39 best_cruise_speed = None) but should
-- be reviewed.  Expect: only records where the source had all-None performance.
SELECT
    v.id, v.name,
    f.name AS manufacturer
FROM aircraft_core.variants v
JOIN aircraft_core.models   m ON m.id = v.model_id
JOIN aircraft_core.families f ON f.id = m.family_id
LEFT JOIN aircraft_specs.performance_metrics pm ON pm.variant_id = v.id
WHERE pm.id IS NULL
ORDER BY f.name, v.name;

-- 3b. Variants without a variant_powerplants row.
-- Expect: 0 — every PlanePHD record has an engine object.
SELECT
    v.id, v.name
FROM aircraft_core.variants v
LEFT JOIN aircraft_power.variant_powerplants vp ON vp.variant_id = v.id
WHERE vp.id IS NULL
ORDER BY v.name;

-- 3c. Variants without any cost_line_items.
-- These are OK if ownership_costs was empty or only contained pilot_salary_taxes
-- (not a mapped numeric cost).  The L-39 Albatross is the known case.
SELECT
    v.id, v.name,
    sa.ownership_costs_json
FROM aircraft_core.variants v
JOIN aircraft_ingest.staged_aircraft sa ON sa.variant_id = v.id
LEFT JOIN aircraft_market.cost_snapshots cs ON cs.variant_id = v.id
WHERE cs.id IS NULL
  AND sa.ownership_costs_json <> '{}'::JSONB
ORDER BY v.name;

-- 3d. engine_variants with manufacturer_org_id still NULL.
-- These are stubs created during ingestion; curator must match them to
-- aircraft_org.organizations rows.
SELECT
    ev.id,
    ev.manufacturer_name_raw,
    ev.model_designation,
    COUNT(vp.id) AS used_by_variants
FROM aircraft_power.engine_variants ev
LEFT JOIN aircraft_power.variant_powerplants vp ON vp.engine_variant_id = ev.id
WHERE ev.manufacturer_org_id IS NULL
GROUP BY ev.id, ev.manufacturer_name_raw, ev.model_designation
ORDER BY ev.manufacturer_name_raw, ev.model_designation;

-- =============================================================================
-- BLOCK 4: Parsing quality
-- =============================================================================

-- 4a. Performance metric rows where raw_value IS NULL.
-- These are expected for 'None KIAS' sentinels.  Any non-sentinel source
-- values that produced NULL warrant investigation.
SELECT
    pm.metric_type_code,
    COUNT(*)                       AS null_value_count,
    MIN(sa.manufacturer_name_raw)  AS sample_manufacturer,
    MIN(sa.aircraft_name_raw)      AS sample_aircraft
FROM aircraft_specs.performance_metrics pm
JOIN aircraft_core.variants v    ON v.id  = pm.variant_id
JOIN aircraft_ingest.staged_aircraft sa ON sa.variant_id = v.id
WHERE pm.raw_value IS NULL
GROUP BY pm.metric_type_code
ORDER BY null_value_count DESC;

-- 4b. Performance metric rows with a raw_unit_code that did not map
-- (raw_unit_code IS NULL but canonical_value IS NULL, i.e. unit was present
-- but unknown — distinct from sentinel NULLs where raw_value is also NULL).
-- Expect: 0 rows.
SELECT
    pm.id,
    pm.metric_type_code,
    v.name,
    pm.raw_value,
    pm.raw_unit_code
FROM aircraft_specs.performance_metrics pm
JOIN aircraft_core.variants v ON v.id = pm.variant_id
WHERE pm.raw_value  IS NOT NULL
  AND pm.raw_unit_code IS NULL
ORDER BY pm.metric_type_code, v.name;

-- 4c. Ownership cost line items per cost_item_type_code.
-- Useful to verify that the prefix-matching logic captured all fuel cost rows.
SELECT
    cost_item_type_code,
    COUNT(*)             AS line_item_count,
    MIN(amount_annual)   AS min_amount,
    MAX(amount_annual)   AS max_amount
FROM aircraft_market.cost_line_items
GROUP BY cost_item_type_code
ORDER BY cost_item_type_code;

-- 4d. Cost snapshots where extra_attributes is non-empty.
-- These are records with unmapped or ambiguous cost keys; each entry is a
-- candidate for a new cost_item_type seed row or a curation flag.
SELECT
    cs.id       AS snapshot_id,
    v.name,
    cs.extra_attributes
FROM aircraft_market.cost_snapshots cs
JOIN aircraft_core.variants v ON v.id = cs.variant_id
WHERE cs.extra_attributes <> '{}'::JSONB
ORDER BY v.name
LIMIT 20;  -- cap output; curator can run without LIMIT for full list

-- 4e. Image rows where href_resolved IS NULL (unresolvable href).
-- Expect: 0 rows (every href is either absolute or /static/... relative).
SELECT
    si.id,
    si.href_raw,
    sa.manufacturer_name_raw,
    sa.aircraft_name_raw
FROM aircraft_ingest.staged_images si
JOIN aircraft_ingest.staged_aircraft sa ON sa.id = si.staged_aircraft_id
WHERE si.href_resolved IS NULL
ORDER BY sa.manufacturer_name_raw;

-- 4f. Image rows where dimension parsing failed (dimensions_raw present but
-- width_px or height_px is NULL).
-- Expect: 0 rows (all observed patterns match 'NNNxNNN').
SELECT
    si.id,
    si.dimensions_raw,
    si.width_px,
    si.height_px,
    sa.aircraft_name_raw
FROM aircraft_ingest.staged_images si
JOIN aircraft_ingest.staged_aircraft sa ON sa.id = si.staged_aircraft_id
WHERE si.dimensions_raw IS NOT NULL
  AND (si.width_px IS NULL OR si.height_px IS NULL)
ORDER BY sa.aircraft_name_raw;

-- =============================================================================
-- BLOCK 5: Provenance integrity
-- =============================================================================

-- 5a. Variants without a source_document.
-- Every promoted variant must have exactly one PlanePHD source_document.
-- Expect: 0 rows.
SELECT v.id, v.name
FROM aircraft_core.variants v
LEFT JOIN aircraft_prov.source_documents sd ON sd.variant_id = v.id
WHERE sd.id IS NULL
ORDER BY v.name;

-- 5b. Source assertion coverage: assertions per entity type.
SELECT
    entity_type_code,
    status_code,
    COUNT(*)    AS assertion_count,
    AVG(confidence)::NUMERIC(4,2) AS avg_confidence
FROM aircraft_prov.source_assertions
GROUP BY entity_type_code, status_code
ORDER BY entity_type_code, status_code;

-- 5c. Variants with zero ACCEPTED performance assertions but at least one
-- performance_metrics row (assertion/metrics mismatch).
-- Expect: 0 rows.
SELECT
    v.id, v.name,
    COUNT(pm.id) AS metric_rows,
    COUNT(sa_a.id) AS accepted_perf_assertions
FROM aircraft_core.variants v
LEFT JOIN aircraft_specs.performance_metrics pm ON pm.variant_id = v.id
LEFT JOIN aircraft_prov.source_assertions sa_a
       ON sa_a.entity_id = v.id
      AND sa_a.entity_type_code = 'VARIANT'
      AND sa_a.field_name LIKE 'performance.%'
      AND sa_a.is_accepted = TRUE
WHERE pm.id IS NOT NULL
GROUP BY v.id, v.name
HAVING COUNT(pm.id) > 0
   AND COUNT(sa_a.id) = 0
ORDER BY v.name;

-- =============================================================================
-- BLOCK 6: Curation flag summary
-- =============================================================================

-- 6a. Open curation flags by type.
SELECT
    flag_type,
    entity_type_code,
    COUNT(*)            AS open_flags,
    MIN(created_at)     AS oldest_flag
FROM aircraft_prov.curation_flags
WHERE status_code = 'OPEN'
GROUP BY flag_type, entity_type_code
ORDER BY open_flags DESC;

-- 6b. Top 20 flagged variants with their notes (for triage).
SELECT
    v.name,
    sa.manufacturer_name_raw,
    sa.promotion_notes
FROM aircraft_ingest.staged_aircraft sa
JOIN aircraft_core.variants v ON v.id = sa.variant_id
WHERE sa.stage_status = 'FLAGGED'
ORDER BY sa.manufacturer_name_raw, v.name
LIMIT 20;

-- =============================================================================
-- BLOCK 7: Image staging integrity
-- =============================================================================

-- 7a. Total images staged and status breakdown.
SELECT
    stage_status,
    COUNT(*)                                AS image_count,
    SUM(CASE WHEN is_primary THEN 1 END)    AS primary_images
FROM aircraft_ingest.staged_images
GROUP BY stage_status;

-- 7b. Staged aircraft with no images at all.
-- Expected: records like 7CCM Champion (empty images array in source JSON).
SELECT
    sa.manufacturer_name_raw,
    sa.aircraft_name_raw
FROM aircraft_ingest.staged_aircraft sa
LEFT JOIN aircraft_ingest.staged_images si ON si.staged_aircraft_id = sa.id
WHERE si.id IS NULL
  AND sa.stage_status IN ('PROMOTED', 'FLAGGED')
ORDER BY sa.manufacturer_name_raw, sa.aircraft_name_raw;

-- 7c. Multiple primary images per aircraft (is_primary constraint check).
-- Expect: 0 rows.
SELECT
    staged_aircraft_id,
    COUNT(*) AS primary_image_count
FROM aircraft_ingest.staged_images
WHERE is_primary = TRUE
GROUP BY staged_aircraft_id
HAVING COUNT(*) > 1;

-- =============================================================================
-- BLOCK 8: Ingest run final state
-- =============================================================================
SELECT
    id                  AS run_id,
    run_label,
    started_at,
    finished_at,
    total_manufacturers,
    total_aircraft,
    staged_aircraft,
    promoted_aircraft,
    CASE
        WHEN finished_at IS NULL          THEN 'FAIL — run never completed'
        WHEN staged_aircraft  IS NULL     THEN 'FAIL — staging count not recorded'
        WHEN promoted_aircraft IS NULL    THEN 'FAIL — promotion count not recorded'
        WHEN staged_aircraft = 0          THEN 'FAIL — no aircraft staged'
        WHEN promoted_aircraft = 0        THEN 'FAIL — no aircraft promoted'
        WHEN promoted_aircraft < staged_aircraft * 0.9
                                          THEN 'WARN — >10% of staged records not promoted'
        ELSE                                   'OK'
    END                 AS run_health
FROM aircraft_ingest.ingest_runs
ORDER BY started_at DESC
LIMIT 5;

-- =============================================================================
-- BLOCK 9: Human-readable spot check
-- Verify the two representative records from the design brief:
--   AERO VODOCHODY L-39 Albatross (jet, all-None performance, no papi)
--   AERONCA 11AC Chief            (piston, full data set)
-- =============================================================================

-- 9a. L-39 Albatross: should have 590 NM range and 4000 FPM climb but
--     NULL best_cruise_speed and NULL fuel_burn (sentinel values).
SELECT
    v.name,
    pm.metric_type_code,
    pm.raw_value,
    pm.raw_unit_code,
    pm.canonical_value,
    pm.is_canonical
FROM aircraft_core.variants v
JOIN aircraft_specs.performance_metrics pm ON pm.variant_id = v.id
WHERE v.ingest_key = 'AERO VODOCHODY::L-39 Albatross'
ORDER BY pm.metric_type_code;

-- 9b. 11AC Chief: should have cruise_speed=72 KIAS, empty_weight=786 LBS,
--     papi=$27921, and fuel cost line item.
SELECT
    v.name,
    pm.metric_type_code,
    pm.raw_value,
    pm.raw_unit_code
FROM aircraft_core.variants v
JOIN aircraft_specs.performance_metrics pm ON pm.variant_id = v.id
WHERE v.ingest_key = 'AERONCA::11AC Chief'
ORDER BY pm.metric_type_code;

SELECT
    v.name,
    wm.metric_type_code,
    wm.raw_value,
    wm.raw_unit_code
FROM aircraft_core.variants v
JOIN aircraft_specs.weight_metrics wm ON wm.variant_id = v.id
WHERE v.ingest_key = 'AERONCA::11AC Chief'
ORDER BY wm.metric_type_code;

SELECT
    v.name,
    val.papi_price_estimate,
    val.for_sale_count,
    val.currency_code
FROM aircraft_core.variants v
JOIN aircraft_market.valuations val ON val.variant_id = v.id
WHERE v.ingest_key = 'AERONCA::11AC Chief';

SELECT
    v.name,
    cli.cost_item_type_code,
    cli.amount_annual
FROM aircraft_core.variants v
JOIN aircraft_market.cost_snapshots cs  ON cs.variant_id = v.id
JOIN aircraft_market.cost_line_items cli ON cli.snapshot_id = cs.id
WHERE v.ingest_key = 'AERONCA::11AC Chief'
ORDER BY cli.cost_item_type_code;

-- 9c. Engine spot check: Cont Motor A-65-8 linked to 11AC Chief.
SELECT
    v.name,
    ev.manufacturer_name_raw,
    ev.model_designation,
    ev.hp_rated,
    ev.tbo_hours,
    vp.engine_count,
    vp.is_primary
FROM aircraft_core.variants v
JOIN aircraft_power.variant_powerplants vp ON vp.variant_id = v.id
JOIN aircraft_power.engine_variants ev     ON ev.id = vp.engine_variant_id
WHERE v.ingest_key = 'AERONCA::11AC Chief';

-- 9d. L-39 thrust engine: Ivchenko AI-25TL.
SELECT
    v.name,
    ev.manufacturer_name_raw,
    ev.model_designation,
    ev.hp_rated,
    ev.rated_thrust_n,
    ev.tbo_hours
FROM aircraft_core.variants v
JOIN aircraft_power.variant_powerplants vp ON vp.variant_id = v.id
JOIN aircraft_power.engine_variants ev     ON ev.id = vp.engine_variant_id
WHERE v.ingest_key = 'AERO VODOCHODY::L-39 Albatross';

-- =============================================================================
-- BLOCK 10: Hard invariant assertions
-- Any EXCEPTION here means a fundamental data-integrity rule was violated.
-- =============================================================================
DO $$
DECLARE
    v_count INT;
    v_pending INT;
BEGIN
    -- Invariant 1: No staged rows still PENDING after promotion
    SELECT COUNT(*) INTO v_pending
    FROM aircraft_ingest.staged_aircraft
    WHERE stage_status = 'PENDING';

    IF v_pending > 0 THEN
        RAISE WARNING 'INVARIANT 1 FAILED: % staged_aircraft rows still PENDING. '
            'Promotion may not have run or encountered a fatal error.', v_pending;
    ELSE
        RAISE NOTICE 'INVARIANT 1 PASSED: 0 staged rows in PENDING status.';
    END IF;

    -- Invariant 2: Every PROMOTED/FLAGGED staged row has a variant_id
    SELECT COUNT(*) INTO v_count
    FROM aircraft_ingest.staged_aircraft
    WHERE stage_status IN ('PROMOTED','FLAGGED')
      AND variant_id IS NULL;

    IF v_count > 0 THEN
        RAISE EXCEPTION 'INVARIANT 2 FAILED: % PROMOTED/FLAGGED rows have NULL variant_id. '
            'Promotion pipeline did not complete correctly.', v_count;
    ELSE
        RAISE NOTICE 'INVARIANT 2 PASSED: all PROMOTED/FLAGGED rows have variant_id set.';
    END IF;

    -- Invariant 3: No orphaned performance_metrics (variant_id references a deleted variant)
    SELECT COUNT(*) INTO v_count
    FROM aircraft_specs.performance_metrics pm
    LEFT JOIN aircraft_core.variants v ON v.id = pm.variant_id
    WHERE v.id IS NULL;

    IF v_count > 0 THEN
        RAISE EXCEPTION 'INVARIANT 3 FAILED: % orphaned performance_metrics rows '
            '(variant_id not in aircraft_core.variants).', v_count;
    ELSE
        RAISE NOTICE 'INVARIANT 3 PASSED: no orphaned performance_metrics rows.';
    END IF;

    -- Invariant 4: No orphaned cost_line_items
    SELECT COUNT(*) INTO v_count
    FROM aircraft_market.cost_line_items cli
    LEFT JOIN aircraft_market.cost_snapshots cs ON cs.id = cli.snapshot_id
    WHERE cs.id IS NULL;

    IF v_count > 0 THEN
        RAISE EXCEPTION 'INVARIANT 4 FAILED: % orphaned cost_line_items rows.', v_count;
    ELSE
        RAISE NOTICE 'INVARIANT 4 PASSED: no orphaned cost_line_items rows.';
    END IF;

    -- Invariant 5: At least one variant was promoted
    SELECT COUNT(*) INTO v_count
    FROM aircraft_core.variants;

    IF v_count = 0 THEN
        RAISE EXCEPTION 'INVARIANT 5 FAILED: no rows in aircraft_core.variants. '
            'Promotion pipeline produced no output.';
    ELSE
        RAISE NOTICE 'INVARIANT 5 PASSED: % variants in aircraft_core.variants.', v_count;
    END IF;

    -- Invariant 6: Multiple primary images per aircraft must not exist
    SELECT COUNT(*) INTO v_count
    FROM (
        SELECT staged_aircraft_id
        FROM aircraft_ingest.staged_images
        WHERE is_primary = TRUE
        GROUP BY staged_aircraft_id
        HAVING COUNT(*) > 1
    ) x;

    IF v_count > 0 THEN
        RAISE EXCEPTION 'INVARIANT 6 FAILED: % aircraft have multiple is_primary images.', v_count;
    ELSE
        RAISE NOTICE 'INVARIANT 6 PASSED: no aircraft have multiple primary images.';
    END IF;

    -- Invariant 7: source_assertions coverage — every promoted variant has
    -- at least one ACCEPTED assertion
    SELECT COUNT(*) INTO v_count
    FROM aircraft_core.variants v
    LEFT JOIN aircraft_prov.source_assertions sa
           ON sa.entity_id       = v.id
          AND sa.entity_type_code = 'AIRCRAFT_VARIANT'
          AND sa.is_accepted      = TRUE
    WHERE sa.id IS NULL;

    IF v_count > 0 THEN
        RAISE WARNING 'INVARIANT 7 WARN: % variants have no ACCEPTED source_assertions. '
            'This may indicate provenance gaps.', v_count;
    ELSE
        RAISE NOTICE 'INVARIANT 7 PASSED: all variants have at least one ACCEPTED assertion.';
    END IF;

    RAISE NOTICE '=== 903 validation complete ===';
END;
$$;