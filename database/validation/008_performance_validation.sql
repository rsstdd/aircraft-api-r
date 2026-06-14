-- ============================================================================
-- FILE: 008_performance_validation.sql
-- DESCRIPTION: Tests aerodynamic data boundaries and speed safety profiles.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_perf, aircraft_core, public;

DO $$
DECLARE
v_config_id BIGINT;
BEGIN
SELECT id INTO v_config_id FROM aircraft_core.aircraft_configurations LIMIT 1;

IF v_config_id IS NOT NULL THEN
        -- Verify constraint blocks invalid structural stall progressions
BEGIN
INSERT INTO speed_limits (
    configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas
) VALUES (
             v_config_id, 55.00, 48.00, 160.00 -- Flaps down (55) faster than clean stall (48); must fail
         );
RAISE EXCEPTION 'Constraint Failure: Invalid aerodynamics profile allowed without catch.';
EXCEPTION WHEN check_violation THEN
            -- Caught expected validation error; pass
END;

        -- Verify field math catches impossible runway properties
BEGIN
INSERT INTO field_performance (
    configuration_id, takeoff_ground_roll_ft, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft
) VALUES (
             v_config_id, 1200.00, 950.00, 1000.00 -- Obstacle distance (950) shorter than roll (1200); must fail
         );
RAISE EXCEPTION 'Constraint Failure: Runways properties allowed to violate physical bounds.';
EXCEPTION WHEN check_violation THEN
            -- Caught expected validation error; pass
END;
END IF;
END $$;

ROLLBACK;