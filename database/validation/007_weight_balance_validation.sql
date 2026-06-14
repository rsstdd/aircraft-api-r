-- ============================================================================
-- FILE: 007_weight_balance_validation.sql
-- DESCRIPTION: Tests weight mathematical logic and fuel density constraints.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_specs, aircraft_core, public;

DO $$
DECLARE
v_config_id BIGINT;
BEGIN
SELECT id INTO v_config_id FROM aircraft_core.aircraft_configurations LIMIT 1;

IF v_config_id IS NOT NULL THEN
        -- Verify logic constraint prevents MTOW lower than empty states
BEGIN
INSERT INTO weight_limits (
    configuration_id, basic_empty_weight_lbs, max_takeoff_weight_lbs
) VALUES (
             v_config_id, 2300.00, 1800.00 -- Violates chk_mtow_gt_empty; must fail
         );
RAISE EXCEPTION 'Constraint Failure: MTOW allowed to be structurally lower than empty weights.';
EXCEPTION WHEN check_violation THEN
            -- Caught expected validation error; pass
END;

        -- Verify math validation rule on fuel totals
BEGIN
INSERT INTO fuel_mass_capacities (
    configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal
) VALUES (
             v_config_id, 56.00, 50.00, 3.00, 6.00 -- Total (56) != Usable (50) + Unusable (3); must fail
         );
RAISE EXCEPTION 'Constraint Failure: Invalid volumetric fuel math bypass detected.';
EXCEPTION WHEN check_violation THEN
            -- Caught expected validation error; pass
END;
END IF;
END $$;

ROLLBACK;