-- ============================================================================
-- FILE: 010_market_valuation_validation.sql
-- DESCRIPTION: Asserts structural pricing consistency and generated cost arithmetic.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_market, aircraft_core, aircraft_ref, public;

DO $$
DECLARE
v_config_id BIGINT;
    v_calculated_hourly NUMERIC;
BEGIN
SELECT id INTO v_config_id FROM aircraft_core.aircraft_configurations LIMIT 1;

IF v_config_id IS NOT NULL THEN
        -- Insert test cost profile
        INSERT INTO operating_cost_estimates (
            configuration_id, currency_code, estimated_fuel_cost_per_hour,
            estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour,
            estimated_insurance_annual, estimated_hangar_storage_annual
        ) VALUES (
            v_config_id, 'USD', 120.00, 45.00, 35.00, 3200.00, 4800.00
        );

        -- Verify generated column successfully isolates stored math boundaries
SELECT total_variable_cost_per_hour INTO v_calculated_hourly
FROM operating_cost_estimates WHERE configuration_id = v_config_id;

IF v_calculated_hourly IS DISTINCT FROM 200.00 THEN
            RAISE EXCEPTION 'Generated Column Math Breakdown: Operational costs mismatched (Expected 200.00, Got %).', v_calculated_hourly;
END IF;
END IF;
END $$;

ROLLBACK;