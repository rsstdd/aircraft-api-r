-- =============================================================================
-- File: database/validation/phase8_performance_validation.sql
-- Phase 8 — validation for aircraft_specs.performance_metrics,
-- runway_limitations, and the additional V-speed metric types.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE AND NEW V-SPEED METRIC TYPE COUNT
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_specs'
  AND table_name IN ('performance_metrics','runway_limitations')
ORDER BY table_name;
-- Expect: 2 rows.

SELECT count(*) AS total_perf_metric_types
FROM aircraft_ref.performance_metric_types
WHERE is_active;
-- Expect: 36 (25 Phase 2 + 11 V-speed additions from Phase 8).

-- Spot-check new V-speed types
SELECT code, label, canonical_unit_code, is_speed, is_higher_better
FROM aircraft_ref.performance_metric_types
WHERE code IN ('SPEED_VX','SPEED_VY','SPEED_VA','SPEED_VNO','SPEED_VMC',
               'SPEED_VYSE','SPEED_VAPP','SPEED_ROTATE')
ORDER BY sort_order;
-- Expect: 8 rows, all with canonical_unit_code = 'KNOTS', is_speed = TRUE.

-- -----------------------------------------------------------------------------
-- 2. to_canonical SPEED CONVERSIONS (most common PlanePHD ingestion cases)
-- -----------------------------------------------------------------------------
SELECT
    round(aircraft_ref.to_canonical(130, 'KIAS'),  2) AS kias_to_knots,   -- 130.00
    round(aircraft_ref.to_canonical(150, 'MPH'),   2) AS mph_to_knots,    -- 130.35
    round(aircraft_ref.to_canonical(240, 'KMH'),   2) AS kmh_to_knots,    -- 129.59
    round(aircraft_ref.to_canonical(0.85,'MACH'),  2) AS mach_passthrough, -- 0.85
    round(aircraft_ref.to_canonical(5000,'FPM'),   2) AS fpm_passthrough,  -- 5000.00
    round(aircraft_ref.to_canonical(200, 'MPS'),   2) AS mps_to_fpm,      -- 39370.08
    round(aircraft_ref.to_canonical(1200,'NM'),    2) AS nm_passthrough,   -- 1200.00
    round(aircraft_ref.to_canonical(2220,'KM'),    2) AS km_to_nm,         -- 1198.71
    round(aircraft_ref.to_canonical(12.0,'GPH'),   2) AS gph_passthrough;  -- 12.00

-- -----------------------------------------------------------------------------
-- 3. COMPREHENSIVE SMOKE TEST — is_canonical, condition checks, constraints
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_pm1  BIGINT; v_pm2 BIGINT;
BEGIN
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('pm8-smoke-fam', 'Pm8 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'pm8-smoke-mod', 'Pm8 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'pm8-smoke-var', 'Pm8 Variant') RETURNING id INTO v_var;

    -- Insert canonical cruise speed (is_canonical = TRUE)
    INSERT INTO aircraft_specs.performance_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
         is_canonical, condition_altitude_ft, condition_weight_lbs,
         condition_weight_label, condition_power_setting, confidence)
    VALUES
        (v_var, 'SPEED_CRUISE_BEST', 122, 'KIAS',
         aircraft_ref.to_canonical(122, 'KIAS'),
         TRUE, 8000, 2400, 'MTOW', '75_PCT', 0.85)
    RETURNING id INTO v_pm1;

    -- Insert second cruise speed from different source (is_canonical = FALSE)
    INSERT INTO aircraft_specs.performance_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
         is_canonical, condition_altitude_ft, condition_weight_label,
         condition_power_setting, source_notes)
    VALUES
        (v_var, 'SPEED_CRUISE_BEST', 118, 'KIAS',
         aircraft_ref.to_canonical(118, 'KIAS'),
         FALSE, 6000, 'MTOW', '75_PCT', 'Alternate source: Jane''s 2023')
    RETURNING id INTO v_pm2;

    -- Attempt to insert a second canonical cruise speed → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.performance_metrics
            (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value, is_canonical)
        VALUES (v_var, 'SPEED_CRUISE_BEST', 125, 'KIAS',
                aircraft_ref.to_canonical(125, 'KIAS'), TRUE);
        RAISE EXCEPTION 'uq_perf_canonical should have rejected second canonical cruise speed';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Different metric type may have its own canonical row (no conflict)
    INSERT INTO aircraft_specs.performance_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
         is_canonical, condition_altitude_ft, condition_power_setting)
    VALUES
        (v_var, 'CEILING_SERVICE', 14500, 'FT',
         aircraft_ref.to_canonical(14500, 'FT'),
         TRUE, NULL, 'MAX_CONTINUOUS');

    -- Invalid condition_power_setting → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.performance_metrics
            (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
             condition_power_setting)
        VALUES (v_var, 'RANGE_NORMAL', 900, 'NM',
                aircraft_ref.to_canonical(900, 'NM'), 'WARP_SPEED');
        RAISE EXCEPTION 'chk_pm_power_setting should have rejected unknown power setting';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Invalid condition_surface_type → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.performance_metrics
            (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value,
             condition_surface_type)
        VALUES (v_var, 'DIST_TO_GROUND_ROLL', 900, 'FT',
                aircraft_ref.to_canonical(900, 'FT'), 'LAVA');
        RAISE EXCEPTION 'chk_pm_surface_type should have rejected unknown surface type';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Negative canonical_value → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.performance_metrics
            (variant_id, metric_type_code, canonical_value, is_canonical)
        VALUES (v_var, 'RANGE_FERRY', -500, FALSE);
        RAISE EXCEPTION 'chk_pm_canonical_nonneg should have rejected negative canonical';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- MPH speed → verify canonical conversion
    INSERT INTO aircraft_specs.performance_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value, is_canonical)
    VALUES (v_var, 'SPEED_VNE', 168, 'MPH',
            aircraft_ref.to_canonical(168, 'MPH'), FALSE);

    PERFORM 1 FROM aircraft_specs.performance_metrics
    WHERE variant_id = v_var
      AND metric_type_code = 'SPEED_VNE'
      AND ABS(canonical_value - 145.99) < 1.0;  -- 168 mph * 0.868976 ≈ 146.0 kts
    IF NOT FOUND THEN
        RAISE EXCEPTION 'MPH → KNOTS conversion produced unexpected result';
    END IF;

    -- Runway limitations test
    INSERT INTO aircraft_specs.runway_limitations
        (variant_id, approved_surfaces, max_crosswind_ktas, min_runway_length_ft,
         hot_high_notes, density_alt_notes)
    VALUES
        (v_var,
         ARRAY['PAVED','GRASS','GRAVEL'],
         15.0, 1200,
         'Performance degrades 3-4% per 1000 ft PA above sea level above ISA+15°C.',
         'Add 10% to takeoff distance for each 1000 ft density altitude above sea level.');

    -- 1:1 enforced by UNIQUE on variant_id
    BEGIN
        INSERT INTO aircraft_specs.runway_limitations (variant_id, approved_surfaces)
        VALUES (v_var, ARRAY['PAVED']);
        RAISE EXCEPTION 'UNIQUE on variant_id should reject second runway_limitations row';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Invalid surface → should be stored without CHECK (approved_surfaces is TEXT[])
    -- The approved_surfaces array has no CHECK constraint on element values.
    -- Validate via: SELECT * FROM runway_limitations WHERE NOT (approved_surfaces <@ ARRAY[...valid...])
    -- This is intentional: array element CHECK is verbose; validated in curation queries.

    -- Negative crosswind → rejected
    BEGIN
        INSERT INTO aircraft_specs.runway_limitations
            (variant_id, max_crosswind_ktas)
        SELECT v_var + 9999, -5  -- non-existent variant; just test CHECK
        WHERE FALSE;

        UPDATE aircraft_specs.runway_limitations
        SET max_crosswind_ktas = -5
        WHERE variant_id = v_var;
        RAISE EXCEPTION 'chk_rl_crosswind_nonneg should have rejected negative crosswind';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 8 smoke test passed: '
                 'canonical uniqueness, power_setting/surface_type CHECKs, '
                 'canonical_nonneg, MPH→KNOTS conversion, '
                 'runway_limitations 1:1, crosswind nonneg — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 4. INDEX EXISTENCE AND TYPES
-- -----------------------------------------------------------------------------
SELECT i.relname AS index_name, t.relname AS table_name,
       ix.indisunique, am.amname AS index_type,
       CASE WHEN pg_get_indexdef(ix.indexrelid) LIKE '%WHERE%' THEN 'partial' ELSE 'full' END AS scope
FROM pg_index ix
JOIN pg_class i  ON i.oid  = ix.indexrelid
JOIN pg_class t  ON t.oid  = ix.indrelid
JOIN pg_am    am ON am.oid  = i.relam
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_specs'
  AND t.relname IN ('performance_metrics','runway_limitations')
ORDER BY t.relname, i.relname;
-- Expected:
--   performance_metrics:
--     pk (btree, full), uq_perf_canonical (btree, partial WHERE is_canonical),
--     idx_pm_type_canonical (btree, partial), idx_pm_variant (btree),
--     idx_pm_variant_type (btree), idx_pm_altitude (btree, partial)
--   runway_limitations:
--     pk (btree), uq on variant_id (btree),
--     idx_rl_surfaces (gin, partial), idx_rl_crosswind (btree, partial)

-- -----------------------------------------------------------------------------
-- 5. PERFORMANCE METRIC TYPES — COMPLETE COVERAGE CHECK
-- Every is_active metric type should be usable for is_canonical comparisons.
-- Verify all types have a canonical_unit_code.
-- (DIM_ASPECT_RATIO, LOAD_FACTOR_* are in OTHER tables and are NULL — fine.)
-- -----------------------------------------------------------------------------
SELECT code, label, canonical_unit_code
FROM aircraft_ref.performance_metric_types
WHERE is_active
  AND canonical_unit_code IS NULL
ORDER BY code;
-- Expect: 0 rows — all performance metric types should have a canonical unit.
-- (Load factors and dimensionless ratios live in weight_metric_types, not here.)

-- -----------------------------------------------------------------------------
-- 6. RUNWAY LIMITATIONS — approved_surfaces GIN INDEX VALIDATION QUERY
-- Pattern for post-ingestion buyer-filter queries:
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
-- "Find all variants approved for grass-strip operations"
SELECT v.slug, v.name, rl.approved_surfaces, rl.max_crosswind_ktas
FROM aircraft_specs.runway_limitations rl
JOIN aircraft_core.variants v ON v.id = rl.variant_id
WHERE rl.approved_surfaces @> ARRAY['GRASS']
ORDER BY rl.max_crosswind_ktas DESC NULLS LAST
LIMIT 20;

-- "Find variants approved for both grass AND gravel"
SELECT v.slug
FROM aircraft_specs.runway_limitations rl
JOIN aircraft_core.variants v ON v.id = rl.variant_id
WHERE rl.approved_surfaces @> ARRAY['GRASS','GRAVEL']
ORDER BY v.slug;
*/

-- -----------------------------------------------------------------------------
-- 7. CANONICAL VALUE COVERAGE POST-INGESTION (run after Phase 17)
-- Identifies (variant, metric_type) pairs that have data but no canonical row.
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
SELECT metric_type_code,
       count(*)                                           AS total_rows,
       count(*) FILTER (WHERE is_canonical)              AS canonical_rows,
       count(*) FILTER (WHERE NOT is_canonical)          AS non_canonical_rows,
       count(*) FILTER (WHERE canonical_value IS NULL)   AS missing_canonical_value
FROM aircraft_specs.performance_metrics
GROUP BY metric_type_code
ORDER BY metric_type_code;
*/

-- -----------------------------------------------------------------------------
-- 8. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_ref.performance_metric_types WHERE is_active) AS perf_types_total,
    (SELECT count(*) FROM aircraft_specs.performance_metrics)                     AS perf_metric_rows,
    (SELECT count(*) FROM aircraft_specs.performance_metrics WHERE is_canonical)  AS canonical_rows,
    (SELECT count(*) FROM aircraft_specs.runway_limitations)                      AS runway_limit_rows;
-- Expect: 36 perf types; 0 rows (data arrives in Phase 17).