-- =============================================================================
-- File: database/validation/phase7_weight_balance_validation.sql
-- Phase 7 — validation for aircraft_specs weight/loading tables.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_specs'
  AND table_name IN ('weight_metrics','cg_envelopes','payload_range_points')
ORDER BY table_name;
-- Expect: 3 rows.

-- -----------------------------------------------------------------------------
-- 2. to_canonical WEIGHT UNIT CONVERSIONS
-- Verify the conversions most commonly seen in PlanePHD seed data.
-- -----------------------------------------------------------------------------
SELECT
    round(aircraft_ref.to_canonical(1000, 'LBS'), 4) AS lbs_passthrough,  -- 1000.0
    round(aircraft_ref.to_canonical(500,  'KG'),  4) AS kg_to_lbs,        -- 1102.3113
    round(aircraft_ref.to_canonical(100,  'US_GAL'),4) AS gal_passthrough,-- 100.0
    round(aircraft_ref.to_canonical(100,  'LITERS'),4) AS l_to_gal,       -- 26.4172
    -- PPH → GPH (approximate avgas conversion, ~6.02 lbs/gal)
    round(aircraft_ref.to_canonical(60.2, 'PPH'),  4) AS pph_to_gph;      -- ~10.0

-- -----------------------------------------------------------------------------
-- 3. WEIGHT METRIC TYPES COVERAGE
-- Verify all 17 metric types seeded in Phase 2 are present and have
-- appropriate canonical units.
-- -----------------------------------------------------------------------------
SELECT
    wmt.code,
    wmt.label,
    wmt.canonical_unit_code,
    mu.symbol AS unit_symbol
FROM aircraft_ref.weight_metric_types wmt
LEFT JOIN aircraft_ref.measurement_units mu ON mu.code = wmt.canonical_unit_code
ORDER BY wmt.sort_order;
-- Expect 17 rows:
--   Mass metrics: canonical_unit_code = 'LBS'
--   Fuel volume : canonical_unit_code = 'US_GAL'
--   Dimensionless (LOAD_FACTOR_POS, LOAD_FACTOR_NEG,
--                  WING_LOADING, POWER_LOADING): NULL

SELECT count(*) FILTER (WHERE canonical_unit_code = 'LBS')    AS mass_metrics,
       count(*) FILTER (WHERE canonical_unit_code = 'US_GAL') AS volume_metrics,
       count(*) FILTER (WHERE canonical_unit_code IS NULL)     AS dimensionless_metrics,
       count(*)                                                 AS total
FROM aircraft_ref.weight_metric_types;
-- Expect: 11 LBS, 2 US_GAL, 4 NULL, 17 total.

-- -----------------------------------------------------------------------------
-- 4. FUNCTIONAL UNIQUE INDEX AND NEGATIVE LOAD FACTOR SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
BEGIN
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('wt7-smoke-fam', 'Wt7 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'wt7-smoke-mod', 'Wt7 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'wt7-smoke-var', 'Wt7 Variant') RETURNING id INTO v_var;

    -- Insert MTOW (mass metric, LBS, canonical = raw)
    INSERT INTO aircraft_specs.weight_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
    VALUES (v_var, 'WEIGHT_MTOW', 2400, 'LBS',
            aircraft_ref.to_canonical(2400, 'LBS'));

    -- Insert MTOW again with same NULL config → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.weight_metrics
            (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
        VALUES (v_var, 'WEIGHT_MTOW', 2450, 'LBS',
                aircraft_ref.to_canonical(2450, 'LBS'));
        RAISE EXCEPTION 'uq_weight_metrics_config should have rejected duplicate MTOW';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Insert MTOW again with a named config → must be allowed
    INSERT INTO aircraft_specs.weight_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value, configuration)
    VALUES (v_var, 'WEIGHT_MTOW', 2600, 'LBS',
            aircraft_ref.to_canonical(2600, 'LBS'), 'WITH_AUXILIARY_TANK');

    -- Insert negative load factor — must be accepted (no non-neg CHECK)
    INSERT INTO aircraft_specs.weight_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
    VALUES (v_var, 'LOAD_FACTOR_NEG', -1.52, NULL, -1.52);

    -- Insert fuel capacity in KG → should convert to US_GAL via KG → LBS, not GPH
    -- (Actually for fuel volume, raw_unit should be volume units.)
    -- Test KG → LBS for mass metric:
    INSERT INTO aircraft_specs.weight_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
    VALUES (v_var, 'WEIGHT_OEW', 680, 'KG',
            aircraft_ref.to_canonical(680, 'KG'));

    -- Verify canonical_value = 680 * 2.2046 ≈ 1499.1
    PERFORM 1 FROM aircraft_specs.weight_metrics
    WHERE variant_id = v_var
      AND metric_type_code = 'WEIGHT_OEW'
      AND ABS(canonical_value - 1499.1) < 1.0;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'KG → LBS canonical conversion produced unexpected result';
    END IF;

    -- CG envelope test
    INSERT INTO aircraft_specs.cg_envelopes
        (variant_id, config_label, cg_unit, is_primary,
         fwd_limit_points, aft_limit_points,
         min_weight_lbs, max_weight_lbs, most_fwd_cg, most_aft_cg)
    VALUES
        (v_var, 'NORMAL', 'INCHES_AFT_DATUM', TRUE,
         '[{"w":1200,"cg":35.5},{"w":2050,"cg":38.2},{"w":2400,"cg":40.1}]'::jsonb,
         '[{"w":1200,"cg":44.1},{"w":2050,"cg":47.5},{"w":2400,"cg":47.5}]'::jsonb,
         1200, 2400, 35.5, 47.5);

    -- Second primary envelope → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.cg_envelopes
            (variant_id, config_label, cg_unit, is_primary)
        VALUES (v_var, 'UTILITY', 'INCHES_AFT_DATUM', TRUE);
        RAISE EXCEPTION 'uq_cge_primary should have rejected second primary envelope';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Invalid cg_unit → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.cg_envelopes
            (variant_id, config_label, cg_unit)
        VALUES (v_var, 'TEST', 'FUSELAGE_STATION');
        RAISE EXCEPTION 'chk_cge_cg_unit should have rejected unknown cg_unit';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Payload-range curve test
    INSERT INTO aircraft_specs.payload_range_points
        (variant_id, sequence_order, payload_lbs, range_nm, conditions_text)
    VALUES
        (v_var, 1, 800,    0,    'Max payload, zero range (structural limit)'),
        (v_var, 2, 750, 1200,    'Full pax, ISA, 8000 ft, 45-min reserves'),
        (v_var, 3, 350, 1800,    'Reduced payload for max range'),
        (v_var, 4,   0, 2200,    'Zero payload, ferry range');

    -- Duplicate sequence position → rejected
    BEGIN
        INSERT INTO aircraft_specs.payload_range_points
            (variant_id, sequence_order, payload_lbs, range_nm)
        VALUES (v_var, 2, 760, 1210);
        RAISE EXCEPTION 'uq_payload_range_point should have rejected duplicate sequence_order';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- sequence_order = 0 → rejected by chk_prp_sequence
    BEGIN
        INSERT INTO aircraft_specs.payload_range_points
            (variant_id, sequence_order, payload_lbs, range_nm)
        VALUES (v_var, 0, 100, 100);
        RAISE EXCEPTION 'chk_prp_sequence should have rejected sequence_order = 0';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Negative range → rejected
    BEGIN
        INSERT INTO aircraft_specs.payload_range_points
            (variant_id, sequence_order, range_nm)
        VALUES (v_var, 5, -100);
        RAISE EXCEPTION 'chk_prp_range_nonneg should have rejected negative range';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 7 smoke test passed: weight metric UNIQUE, negative load factor, '
                 'KG→LBS conversion, CG envelope primary unique + cg_unit check, '
                 'payload-range sequence unique + range nonneg — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 5. INDEX EXISTENCE
-- -----------------------------------------------------------------------------
SELECT i.relname AS index_name, t.relname AS table_name,
       ix.indisunique, pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid  = ix.indexrelid
JOIN pg_class t ON t.oid  = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_specs'
  AND t.relname IN ('weight_metrics','cg_envelopes','payload_range_points')
ORDER BY t.relname, i.relname;
-- Expected indexes:
--   weight_metrics:         pk, uq_weight_metrics_config (functional), idx_wm_type_canonical, idx_wm_variant
--   cg_envelopes:           pk, idx_cge_variant, uq_cge_primary (partial), idx_cge_fwd/aft_points (gin)
--   payload_range_points:   pk, uq_payload_range_point (functional), idx_prp_variant_curve

-- -----------------------------------------------------------------------------
-- 6. CG ENVELOPE JSON STRUCTURE VALIDATION HELPER
-- When real envelope data exists post-ingestion, run this to check
-- that fwd_limit_points arrays contain the expected key structure.
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
SELECT id, variant_id, config_label,
       jsonb_array_length(fwd_limit_points) AS fwd_points,
       jsonb_array_length(aft_limit_points) AS aft_points,
       -- Verify every element has 'w' and 'cg' keys
       bool_and(
           (elem ? 'w') AND (elem ? 'cg')
       ) OVER (PARTITION BY id) AS all_points_valid
FROM aircraft_specs.cg_envelopes,
     jsonb_array_elements(fwd_limit_points) AS elem
WHERE fwd_limit_points IS NOT NULL;
*/

-- -----------------------------------------------------------------------------
-- 7. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_specs.weight_metrics)       AS weight_metric_rows,
    (SELECT count(*) FROM aircraft_specs.cg_envelopes)         AS cg_envelope_rows,
    (SELECT count(*) FROM aircraft_specs.payload_range_points) AS payload_range_rows;
-- All zero before Phase 17 ingestion.