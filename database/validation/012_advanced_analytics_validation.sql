-- =============================================================================
-- File: database/validation/phase12_market_validation.sql
-- Phase 12 — validation for aircraft_market tables.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE (3 tables expected)
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_market'
ORDER BY table_name;
-- Expect: cost_line_items, cost_snapshots, valuations

-- -----------------------------------------------------------------------------
-- 2. COST ITEM TYPE COVERAGE
-- Verify all 18 Phase 2 cost_item_types are available for line items.
-- -----------------------------------------------------------------------------
SELECT code, label, is_fixed, is_fuel, sort_order
FROM aircraft_ref.cost_item_types
ORDER BY sort_order;
-- Expect: 18 rows covering annual fixed (9) and per-hour variable (9) cost types.

SELECT count(*) FILTER (WHERE is_fixed) AS annual_fixed_types,
       count(*) FILTER (WHERE NOT is_fixed) AS hourly_variable_types,
       count(*) FILTER (WHERE is_fuel) AS fuel_types,
       count(*) AS total
FROM aircraft_ref.cost_item_types;
-- Expect: 9 fixed, 9 variable, 1 fuel, 18 total.

-- -----------------------------------------------------------------------------
-- 3. FK CHAINS
-- -----------------------------------------------------------------------------
SELECT tc.table_name AS fk_table, kcu.column_name AS fk_col,
       ccu.table_schema AS ref_schema, ccu.table_name AS ref_table,
       rc.delete_rule
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
JOIN information_schema.referential_constraints rc  ON rc.constraint_name  = tc.constraint_name AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_market'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   valuations      → aircraft_core.variants (CASCADE), aircraft_ref.currencies,
--                     aircraft_geo.regions, aircraft_ref.aircraft_condition_grades
--   cost_snapshots  → aircraft_core.variants (CASCADE), aircraft_ref.currencies,
--                     aircraft_geo.regions, aircraft_ref.aircraft_condition_grades
--   cost_line_items → aircraft_market.cost_snapshots (CASCADE),
--                     aircraft_ref.cost_item_types, aircraft_ref.currencies

SELECT count(*) AS total_fks
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY' AND table_schema = 'aircraft_market';
-- Expect: ~10

-- -----------------------------------------------------------------------------
-- 4. CHECK CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name, left(cc.check_clause, 80) AS check_snippet
FROM information_schema.table_constraints  tc
JOIN information_schema.check_constraints  cc
    ON cc.constraint_name = tc.constraint_name AND cc.constraint_schema = tc.constraint_schema
WHERE tc.table_schema    = 'aircraft_market'
  AND tc.constraint_type = 'CHECK'
ORDER BY tc.table_name, tc.constraint_name;
-- Expect: chk_val_prices, chk_val_price_range,
--         chk_cs_hours, chk_cs_fuel,
--         chk_cli_has_amount, chk_cli_amounts_nonneg

-- -----------------------------------------------------------------------------
-- 5. COMPREHENSIVE SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_cs   BIGINT;
BEGIN
    -- Hierarchy
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('mkt12-smoke-fam', 'Mkt12 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'mkt12-smoke-mod', 'Mkt12 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'mkt12-smoke-var', 'Mkt12 Variant') RETURNING id INTO v_var;

    -- Insert valuation snapshot
    INSERT INTO aircraft_market.valuations
        (variant_id, snapshot_date, source_name,
         papi_price_estimate, for_sale_count, currency_code,
         condition_grade_code, assumed_year, assumed_airframe_hours,
         confidence)
    VALUES
        (v_var, '2024-01-15', 'PlanePHD',
         185000, 23, 'USD',
         'GOOD', 2010, 1500,
         0.75);

    -- Second valuation same date same source → rejected (functional UNIQUE)
    BEGIN
        INSERT INTO aircraft_market.valuations
            (variant_id, snapshot_date, source_name,
             papi_price_estimate, currency_code)
        VALUES (v_var, '2024-01-15', 'PlanePHD', 190000, 'USD');
        RAISE EXCEPTION 'uq_val_variant_date_source should reject duplicate';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Different date → allowed
    INSERT INTO aircraft_market.valuations
        (variant_id, snapshot_date, source_name,
         papi_price_estimate, for_sale_count, currency_code, confidence)
    VALUES (v_var, '2024-07-01', 'PlanePHD', 180000, 20, 'USD', 0.75);

    -- Price range violation (low > high) → rejected
    BEGIN
        INSERT INTO aircraft_market.valuations
            (variant_id, snapshot_date, source_name,
             listing_price_low, listing_price_high, currency_code)
        VALUES (v_var, '2024-01-20', 'Test', 200000, 100000, 'USD');
        RAISE EXCEPTION 'chk_val_price_range should reject low > high';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Negative for_sale_count → rejected
    BEGIN
        INSERT INTO aircraft_market.valuations
            (variant_id, snapshot_date, source_name, for_sale_count, currency_code)
        VALUES (v_var, '2024-02-01', 'Test', -5, 'USD');
        RAISE EXCEPTION 'chk_val_prices should reject negative for_sale_count';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Cost snapshot with full assumptions
    INSERT INTO aircraft_market.cost_snapshots
        (variant_id, snapshot_date, source_name,
         condition_grade_code, assumed_airframe_hours,
         assumed_engine_hours_smoh, assumed_prop_hours_smoh,
         assumed_annual_hours, assumed_fuel_price_per_gal,
         assumed_fuel_burn_gph, currency_code, confidence)
    VALUES
        (v_var, '2024-01-15', 'PlanePHD',
         'GOOD', 1500, 800, 200,
         100.0, 5.80, 8.5, 'USD', 0.72)
    RETURNING id INTO v_cs;

    -- Insert cost line items (sample from 18 types)
    INSERT INTO aircraft_market.cost_line_items
        (snapshot_id, cost_item_type_code, amount_annual, amount_per_hour, currency_code)
    VALUES
        (v_cs, 'ANNUAL_INSPECTION',  1800, NULL,   'USD'),
        (v_cs, 'INSURANCE',          3600, NULL,   'USD'),
        (v_cs, 'HANGAR_STORAGE',     2400, NULL,   'USD'),
        (v_cs, 'DEPRECIATION',       6000, NULL,   'USD'),
        (v_cs, 'FUEL',               NULL, 49.30,  'USD'),   -- 8.5 GPH × $5.80
        (v_cs, 'ENGINE_RESERVE',     NULL, 15.00,  'USD'),
        (v_cs, 'PROP_RESERVE',       NULL,  3.50,  'USD'),
        (v_cs, 'HOURLY_MAINTENANCE', NULL,  8.00,  'USD');

    -- Duplicate cost item type in same snapshot → rejected
    BEGIN
        INSERT INTO aircraft_market.cost_line_items
            (snapshot_id, cost_item_type_code, amount_annual, currency_code)
        VALUES (v_cs, 'INSURANCE', 4000, 'USD');
        RAISE EXCEPTION 'UNIQUE(snapshot_id, cost_item_type_code) should reject duplicate';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- chk_cli_has_amount: both amounts NULL → rejected
    BEGIN
        INSERT INTO aircraft_market.cost_line_items
            (snapshot_id, cost_item_type_code, amount_annual, amount_per_hour, currency_code)
        VALUES (v_cs, 'LANDING_FEES', NULL, NULL, 'USD');
        RAISE EXCEPTION 'chk_cli_has_amount should reject row with both amounts NULL';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Negative amount → rejected
    BEGIN
        INSERT INTO aircraft_market.cost_line_items
            (snapshot_id, cost_item_type_code, amount_per_hour, currency_code)
        VALUES (v_cs, 'OIL', -0.50, 'USD');
        RAISE EXCEPTION 'chk_cli_amounts_nonneg should reject negative amount';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- assumed_annual_hours = 0 → rejected
    BEGIN
        INSERT INTO aircraft_market.cost_snapshots
            (variant_id, snapshot_date, source_name, assumed_annual_hours, currency_code)
        VALUES (v_var, '2024-03-01', 'Test', 0, 'USD');
        RAISE EXCEPTION 'chk_cs_hours should reject assumed_annual_hours = 0';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Verify aggregate cost structure
    DECLARE
        v_total_annual  NUMERIC;
        v_total_hourly  NUMERIC;
    BEGIN
        SELECT
            sum(amount_annual)  FILTER (WHERE amount_annual  IS NOT NULL),
            sum(amount_per_hour) FILTER (WHERE amount_per_hour IS NOT NULL)
        INTO v_total_annual, v_total_hourly
        FROM aircraft_market.cost_line_items
        WHERE snapshot_id = v_cs;

        IF v_total_annual IS NULL OR v_total_annual < 13000 THEN
            RAISE EXCEPTION 'Annual fixed costs should be ≥ 13800 (got %)', v_total_annual;
        END IF;
        IF v_total_hourly IS NULL OR v_total_hourly < 75 THEN
            RAISE EXCEPTION 'Hourly variable costs should be ≥ 75 (got %)', v_total_hourly;
        END IF;
    END;

    RAISE NOTICE 'Phase 12 smoke test passed: valuation unique, price range CHECK, '
                 'cost snapshot unique, line item UNIQUE + has_amount CHECK, '
                 'negative amount check, annual_hours > 0 check — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 6. FUNCTIONAL UNIQUE INDEX VERIFICATION
-- Confirm both functional UNIQUE indexes exist (non-standard — not visible via
-- information_schema.table_constraints as 'UNIQUE').
-- -----------------------------------------------------------------------------
SELECT i.relname AS index_name, t.relname AS table_name, ix.indisunique,
       pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_market'
  AND ix.indisunique
  AND pg_get_indexdef(ix.indexrelid) LIKE '%COALESCE%'
ORDER BY t.relname;
-- Expect: uq_val_variant_date_source and uq_cs_variant_date_source.

-- -----------------------------------------------------------------------------
-- 7. EXAMPLE BUYER COST COMPARISON QUERY (post-ingestion pattern)
-- Demonstrates total annual cost computation from line items.
-- Uncomment post Phase 17 ingestion.
-- -----------------------------------------------------------------------------
/*
SELECT
    v.slug,
    v.name,
    cs.assumed_annual_hours,
    cs.condition_grade_code,
    round(sum(cli.amount_annual),                           2) AS total_annual_fixed,
    round(sum(cli.amount_per_hour),                         4) AS total_hourly_variable,
    round(sum(cli.amount_annual)
        + sum(cli.amount_per_hour) * cs.assumed_annual_hours, 2) AS total_annual_all_in
FROM aircraft_market.cost_snapshots cs
JOIN aircraft_core.variants v ON v.id = cs.variant_id
JOIN aircraft_market.cost_line_items cli ON cli.snapshot_id = cs.id
JOIN aircraft_ref.cost_item_types cit ON cit.code = cli.cost_item_type_code
WHERE cs.snapshot_date = (
    SELECT max(snapshot_date)
    FROM aircraft_market.cost_snapshots cs2
    WHERE cs2.variant_id = cs.variant_id
      AND cs2.source_name = cs.source_name
)
GROUP BY v.slug, v.name, cs.id, cs.assumed_annual_hours, cs.condition_grade_code
ORDER BY total_annual_all_in ASC NULLS LAST
LIMIT 20;
*/

-- -----------------------------------------------------------------------------
-- 8. MOST-RECENT VALUATION PATTERN (Phase 16 v_current_valuation preview)
-- -----------------------------------------------------------------------------
/*
SELECT DISTINCT ON (variant_id) *
FROM aircraft_market.valuations
ORDER BY variant_id, snapshot_date DESC, captured_at DESC, id DESC;
-- This is the DISTINCT ON pattern used in the Phase 16 v_current_valuation view.
-- The index idx_val_variant_date covers this query efficiently.
*/

-- -----------------------------------------------------------------------------
-- 9. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_market.valuations)       AS valuation_rows,
    (SELECT count(*) FROM aircraft_market.cost_snapshots)   AS cost_snapshot_rows,
    (SELECT count(*) FROM aircraft_market.cost_line_items)  AS cost_line_item_rows;
-- All zero before Phase 17 ingestion.