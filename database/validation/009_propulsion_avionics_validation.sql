-- ============================================================================
-- FILE: 009_propulsion_avionics_validation.sql
-- DESCRIPTION: Validates propulsion relationships and system installations.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_prop, aircraft_avionics, aircraft_core, aircraft_org, public;

DO $$
DECLARE
v_oem_id BIGINT;
    v_config_id BIGINT;
    v_engine_id BIGINT;
    v_suite_id BIGINT;
BEGIN
    -- Extract reference identifiers
SELECT id INTO v_oem_id FROM aircraft_org.organizations LIMIT 1;
SELECT id INTO v_config_id FROM aircraft_core.aircraft_configurations LIMIT 1;

IF v_oem_id IS NOT NULL AND v_config_id IS NOT NULL THEN
        -- Verify engine cataloging structures
        INSERT INTO engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_horsepower)
        VALUES (v_oem_id, 'Test-Engine-360', 'test-engine-360', 'PISTON', 180.00)
        RETURNING id INTO v_engine_id;

        -- Verify layout bindings map accurately
INSERT INTO propulsion_installations (configuration_id, engine_model_id, engine_count, is_constant_speed_propeller)
VALUES (v_config_id, v_engine_id, 1, FALSE);

-- Verify avionics registration tracks cleanly
INSERT INTO avionics_suites (manufacturer_id, name, slug)
VALUES (v_oem_id, 'Test Avionics Deck', 'test-avionics-deck')
    RETURNING id INTO v_suite_id;

INSERT INTO avionics_installations (configuration_id, avionics_suite_id, is_glass_cockpit, has_autopilot)
VALUES (v_config_id, v_suite_id, TRUE, TRUE);
END IF;
END $$;

ROLLBACK;