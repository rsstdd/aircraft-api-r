-- ============================================================================
-- FILE: 006_dimensions_validation.sql
-- DESCRIPTION: Validates volumetric attributes and physical structure bounds.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_specs, aircraft_core, public;

DO $$
DECLARE
v_config_id BIGINT;
BEGIN
    -- Extract an operational configuration ID
SELECT id INTO v_config_id FROM aircraft_core.aircraft_configurations LIMIT 1;

IF v_config_id IS NOT NULL THEN
        -- Verify that the folding wings validation check works correctly
BEGIN
INSERT INTO hangar_fit_dimensions (
    configuration_id, tail_height_ft, has_folding_wings, wingspan_folded_ft
) VALUES (
             v_config_id, 15.00, TRUE, NULL -- Indicates folding wings but omits size; must fail
         );
RAISE EXCEPTION 'Constraint Failure: Folding wing status accepted without specifying folded dimensions.';
EXCEPTION WHEN check_violation THEN
            -- Caught expected validation error; pass
END;

        -- Verify base insertion metrics pass safely
INSERT INTO external_dimensions (configuration_id, wingspan_raw, wingspan_ft, length_raw, length_ft, height_raw, height_ft)
VALUES (v_config_id, '36 ft 1 in', 36.08, '27 ft 2 in', 27.17, '8 ft 11 in', 8.92);
END IF;
END $$;

ROLLBACK;