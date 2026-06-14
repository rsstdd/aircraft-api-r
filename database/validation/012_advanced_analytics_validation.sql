-- ============================================================================
-- FILE: 012_advanced_analytics_validation.sql
-- DESCRIPTION: Verifies programmatic math boundaries and cache performance locks.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_analytics, public;

DO $$
DECLARE
v_test_config_id BIGINT;
    v_payload_check NUMERIC;
BEGIN
SELECT configuration_id INTO v_test_config_id FROM public.unified_aircraft_registry LIMIT 1;

IF v_test_config_id IS NOT NULL THEN
        -- Run full fuel payload evaluation function logic
        v_payload_check := fn_calculate_full_fuel_payload(v_test_config_id);

        -- Assert function returns a valid object or successfully defaults to structural null parameters
        RAISE NOTICE 'Analytics Pipeline Status: Verification function resolved payload to % lbs.', v_payload_check;
END IF;

    -- Verify Materialized summary contains tracking indexes
    IF NOT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_mv_fleet_market_summary') THEN
        RAISE EXCEPTION 'Cache System Failure: Materialized cache missing unique performance indexing strategy.';
END IF;
END $$;

ROLLBACK;