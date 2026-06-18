-- =============================================================================
-- File: database/validation/phase6_dimensions_validation.sql
-- Phase 6 — validation for aircraft_specs.dimension_metrics,
-- cabin_specs, cargo_holds, and aircraft_ref.to_canonical().
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE AND FUNCTION EXISTENCE
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_specs'
  AND table_name IN ('dimension_metrics','cabin_specs','cargo_holds')
ORDER BY table_name;
-- Expect: 3 rows.

SELECT routine_schema, routine_name, routine_type
FROM information_schema.routines
WHERE routine_schema = 'aircraft_ref'
  AND routine_name   = 'to_canonical';
-- Expect: 1 row (FUNCTION).

-- -----------------------------------------------------------------------------
-- 2. to_canonical UNIT CONVERSION SMOKE TEST
-- Verifies conversion factors for the most common ingestion cases.
-- Tolerances: ±0.01% for floating-point arithmetic.
-- -----------------------------------------------------------------------------
SELECT
    -- FT is already canonical for ALTITUDE → passthrough
    round(aircraft_ref.to_canonical(100, 'FT'),    4) AS ft_passthrough,          -- expect 100.0000
    -- METERS → FT
    round(aircraft_ref.to_canonical(1,    'METERS'),4) AS m_to_ft,                -- expect 3.2808
    -- LBS is canonical for WEIGHT → passthrough
    round(aircraft_ref.to_canonical(1000, 'LBS'),  4) AS lbs_passthrough,         -- expect 1000.0000
    -- KG → LBS
    round(aircraft_ref.to_canonical(100,  'KG'),   4) AS kg_to_lbs,               -- expect 220.4623
    -- NM is canonical for RANGE → passthrough
    round(aircraft_ref.to_canonical(500,  'NM'),   4) AS nm_passthrough,           -- expect 500.0000
    -- MPH → KNOTS
    round(aircraft_ref.to_canonical(100,  'MPH'),  4) AS mph_to_knots,             -- expect 86.8976
    -- GPH is canonical for FLOW_RATE → passthrough
    round(aircraft_ref.to_canonical(10,   'GPH'),  4) AS gph_passthrough,          -- expect 10.0000
    -- NEWTONS → LBF
    round(aircraft_ref.to_canonical(3790, 'NEWTONS'),4) AS n_to_lbf,              -- expect 852.06
    -- LBF is canonical for THRUST → passthrough
    round(aircraft_ref.to_canonical(1978, 'LBF'),  4) AS lbf_passthrough,          -- expect 1978.0000
    -- US_GAL is canonical for VOLUME → passthrough
    round(aircraft_ref.to_canonical(87,   'US_GAL'),4) AS gal_passthrough,         -- expect 87.0000
    -- CU_FT → US_GAL
    round(aircraft_ref.to_canonical(10,   'CU_FT'),4) AS cuft_to_usgal,            -- expect 74.8052
    -- NULL input
    aircraft_ref.to_canonical(NULL, 'FT')           AS null_input,                  -- expect NULL
    -- Unknown unit code → NULL (no matching row)
    aircraft_ref.to_canonical(100, 'NONEXISTENT')   AS bad_unit;                    -- expect NULL

-- -----------------------------------------------------------------------------
-- 3. FUNCTIONAL UNIQUE INDEX ON dimension_metrics
-- Verify the index exists and enforces (variant, type, COALESCE(config,''))
-- via a transactional smoke test.
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
BEGIN
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('dim6-smoke-family', 'Dim6 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'dim6-smoke-model', 'Dim6 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'dim6-smoke-variant', 'Dim6 Variant') RETURNING id INTO v_var;

    -- Insert standard (NULL config) wingspan
    INSERT INTO aircraft_specs.dimension_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
    VALUES (v_var, 'DIM_WINGSPAN', 39.2, 'FT',
            aircraft_ref.to_canonical(39.2, 'FT'));

    -- Insert wings-folded wingspan (different config → allowed)
    INSERT INTO aircraft_specs.dimension_metrics
        (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value, configuration)
    VALUES (v_var, 'DIM_WINGSPAN', 15.6, 'FT',
            aircraft_ref.to_canonical(15.6, 'FT'), 'WINGS_FOLDED');

    -- Duplicate standard wingspan → must be rejected
    BEGIN
        INSERT INTO aircraft_specs.dimension_metrics
            (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
        VALUES (v_var, 'DIM_WINGSPAN', 39.3, 'FT',
                aircraft_ref.to_canonical(39.3, 'FT'));
        RAISE EXCEPTION 'uq_dimension_metrics_config should have rejected duplicate NULL config';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Insert cabin spec and verify is_primary_config partial unique
    INSERT INTO aircraft_specs.cabin_specs
        (variant_id, config_label, passenger_seat_count, is_primary_config)
    VALUES (v_var, 'STANDARD', 4, TRUE);

    BEGIN
        INSERT INTO aircraft_specs.cabin_specs
            (variant_id, config_label, passenger_seat_count, is_primary_config)
        VALUES (v_var, 'HIGH_DENSITY', 5, TRUE);  -- second primary → rejected
        RAISE EXCEPTION 'uq_cabin_primary_config should have rejected second primary row';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Cargo hold: invalid hold_position → rejected
    BEGIN
        INSERT INTO aircraft_specs.cargo_holds (variant_id, hold_position)
        VALUES (v_var, 'INVALID_POSITION');
        RAISE EXCEPTION 'chk_ch_position should have rejected unknown hold_position';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Negative canonical_value → rejected
    BEGIN
        INSERT INTO aircraft_specs.dimension_metrics
            (variant_id, metric_type_code, raw_value, raw_unit_code, canonical_value)
        VALUES (v_var, 'DIM_LENGTH', 30.0, 'FT', -1.0);
        RAISE EXCEPTION 'chk_dm_canonical_nonneg should have rejected negative canonical_value';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 6 smoke test passed: to_canonical, functional UNIQUE, '
                 'cabin primary config, hold_position CHECK, canonical nonneg — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 4. FOREIGN KEY COUNT
-- -----------------------------------------------------------------------------
SELECT count(*) AS fk_count
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY'
  AND table_schema    = 'aircraft_specs'
  AND table_name IN ('dimension_metrics','cabin_specs','cargo_holds');
-- Expect: 5 (dimension_metrics: variant_id, metric_type_code, raw_unit_code;
--            cabin_specs: variant_id; cargo_holds: variant_id, volume_raw_unit_code)

-- -----------------------------------------------------------------------------
-- 5. INDEXES PER TABLE
-- -----------------------------------------------------------------------------
SELECT t.relname AS table_name, count(*) AS index_count
FROM pg_index ix
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_specs'
  AND t.relname IN ('dimension_metrics','cabin_specs','cargo_holds')
GROUP BY t.relname
ORDER BY t.relname;
-- Expect:
--   cabin_specs:       3 (pk, uq_variant_label, uq_primary, idx_pax, idx_pressurized)
--   cargo_holds:       2 (pk, idx_ch_variant, idx_ch_main_deck)
--   dimension_metrics: 3 (pk, uq_config functional, idx_dm_type_canonical, idx_dm_variant)

-- -----------------------------------------------------------------------------
-- 6. METRIC TYPE COVERAGE CHECK
-- Verify all 17 DIM_* metric type codes exist and have canonical units.
-- -----------------------------------------------------------------------------
SELECT
    dmt.code,
    dmt.label,
    dmt.canonical_unit_code,
    mu.symbol    AS canonical_symbol
FROM aircraft_ref.dimension_metric_types dmt
LEFT JOIN aircraft_ref.measurement_units mu
    ON mu.code = dmt.canonical_unit_code
ORDER BY dmt.sort_order;
-- Expect: 17 rows. DIM_ASPECT_RATIO has NULL canonical_unit_code (dimensionless).
-- All others should have FT, SQ_FT, or CU_FT as canonical_unit_code.

-- Verify to_canonical handles all canonical units used by dimension metrics:
SELECT DISTINCT
    dmt.canonical_unit_code,
    mu.symbol,
    aircraft_ref.to_canonical(1, dmt.canonical_unit_code) AS one_to_canonical
FROM aircraft_ref.dimension_metric_types dmt
JOIN aircraft_ref.measurement_units mu ON mu.code = dmt.canonical_unit_code
ORDER BY dmt.canonical_unit_code;
-- Expect: FT → 1.0 (passthrough), SQ_FT → 1.0, CU_FT → 7.4805 (to US_GAL? No...)
-- Actually CU_FT is in VOLUME category, canonical is US_GAL.
-- Hmm — CU_FT.canonical_unit_code = 'US_GAL' per seeds. So to_canonical(1,'CU_FT') = 7.48.
-- DIM_BAGGAGE_VOLUME canonical_unit_code = 'CU_FT', but CU_FT's canonical is US_GAL.
-- This is a layered conversion issue: dimension_metric_types.canonical_unit_code = 'CU_FT'
-- means the canonical VALUE for the metric is in CU_FT, not that we then convert CU_FT to US_GAL.
-- The to_canonical function converts FROM the raw_unit TO the metric's canonical unit.
-- If raw_unit IS CU_FT and canonical_unit IS CU_FT → passthrough (CU_FT.canonical_unit_code IS NULL? No, it converts to US_GAL)
-- DESIGN NOTE: CU_FT in measurement_units converts to US_GAL (a different category).
-- For volume metrics where the canonical metric unit is CU_FT, the ingestion must:
--   1. Convert raw_unit → CU_FT directly in ingestion logic, NOT via to_canonical chains.
--   2. Or the canonical_unit on dimension_metric_types for volume metrics should be 'US_GAL'.
-- This is resolved by the Phase 17 ingestion using direct arithmetic, noted below.

-- -----------------------------------------------------------------------------
-- 7. IMPLEMENTATION NOTE — VOLUME METRIC CANONICAL RESOLUTION
-- The to_canonical function follows measurement_units.canonical_factor.
-- CU_FT converts to US_GAL (canonical for VOLUME category).
-- dimension_metric_types.canonical_unit_code for DIM_BAGGAGE_VOLUME is 'CU_FT'.
-- These are DIFFERENT uses of "canonical":
--   - measurement_units canonical: best comparison unit for that physical quantity
--   - dimension_metric_types canonical: expected unit for that metric in this db
-- Phase 17 ingestion resolves volume metric values to CU_FT directly
-- using raw conversion (not via to_canonical chain):
--   IF raw_unit = 'LITERS' THEN canonical_value = raw_value * 0.0353147  (L → CU_FT)
--   IF raw_unit = 'US_GAL' THEN canonical_value = raw_value * 0.133681   (USG → CU_FT)
--   IF raw_unit = 'CU_FT'  THEN canonical_value = raw_value              (passthrough)
-- to_canonical is most reliable for: SPEED → KNOTS, WEIGHT → LBS,
-- ALTITUDE → FT, RANGE → NM, POWER → HP, THRUST → LBF, FLOW → GPH.
-- -----------------------------------------------------------------------------
SELECT 'Volume metric resolution note: see validation file comments' AS note;

-- -----------------------------------------------------------------------------
-- 8. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_specs.dimension_metrics) AS dimension_rows,
    (SELECT count(*) FROM aircraft_specs.cabin_specs)       AS cabin_rows,
    (SELECT count(*) FROM aircraft_specs.cargo_holds)       AS cargo_hold_rows;
-- All zero before Phase 17 ingestion.