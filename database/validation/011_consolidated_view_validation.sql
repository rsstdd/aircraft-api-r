-- ============================================================================
-- FILE: 011_consolidated_view_validation.sql
-- DESCRIPTION: Confirms view generation stability and flat schema access bounds.
-- ============================================================================

BEGIN;

SET search_path TO public, public;

DO $$
DECLARE
v_column_count INT;
BEGIN
    -- Verify the system catalog registers all target tables inside the stitched view
SELECT COUNT(*) INTO v_column_count
FROM information_schema.columns
WHERE table_name = 'unified_aircraft_registry';

IF v_column_count < 40 THEN
        RAISE EXCEPTION 'View Validation Failure: Flat matrix dropped columns during table integration.';
END IF;
END $$;

ROLLBACK;