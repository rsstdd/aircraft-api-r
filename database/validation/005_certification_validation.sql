-- ============================================================================
-- FILE: 005_certification_validation.sql
-- DESCRIPTION: Validates regulatory constraint boundaries and logic checks.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_cert, aircraft_core, aircraft_org, public;

DO $$
DECLARE
v_variant_id BIGINT;
    v_authority_id BIGINT;
BEGIN
    -- Extract mock or bootstrap identifiers
SELECT id INTO v_variant_id FROM aircraft_core.aircraft_variants LIMIT 1;
SELECT id INTO v_authority_id FROM aircraft_org.organizations WHERE org_type_code = 'REGULATOR' LIMIT 1;

IF v_variant_id IS NOT NULL AND v_authority_id IS NOT NULL THEN
        -- Verify load factor validation rules stop invalid flight envelopes
BEGIN
INSERT INTO operating_approvals (
    variant_id, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative
) VALUES (
             v_variant_id, 3.80, 1.20 -- Flips negative load factor to positive, should fail
         );
RAISE EXCEPTION 'Constraint Failure: Positive structural values allowed in negative G-limit fields.';
EXCEPTION WHEN check_violation THEN
            -- Caught expected error condition; pass
END;

        -- Verify type certificate storage integrity
INSERT INTO type_certificates (variant_id, authority_id, certificate_number, airworthiness_category)
VALUES (v_variant_id, v_authority_id, 'TEST-TC-123', 'NORMAL');
END IF;
END $$;

ROLLBACK;