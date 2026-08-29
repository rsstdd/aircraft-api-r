-- =============================================================================
-- File: database/migrations/018_staged_aircraft_variant_fk.sql
-- Phase 18: Referential integrity for the staged-to-canonical link.
--
-- Migration 017 added aircraft_ingest.staged_aircraft.variant_id to record which
-- canonical variant a staged record was promoted into, but declared it as a bare
-- BIGINT. It is the only cross-schema link introduced by 017 without a foreign
-- key, so a deleted variant leaves the staging row pointing at nothing and the
-- promotion audit trail silently becomes wrong.
--
-- ON DELETE SET NULL rather than CASCADE: deleting a canonical variant must not
-- destroy the evidence that it was once ingested. The staging row survives with
-- its raw JSON, and only the now-meaningless link is cleared.
-- =============================================================================

BEGIN;

-- Clear any link that a pre-018 database accumulated to a variant that no longer
-- exists, so the constraint can be added without a validation failure.
UPDATE aircraft_ingest.staged_aircraft sa
SET    variant_id = NULL
WHERE  sa.variant_id IS NOT NULL
  AND  NOT EXISTS (
           SELECT 1 FROM aircraft_core.variants v WHERE v.id = sa.variant_id
       );

ALTER TABLE aircraft_ingest.staged_aircraft
    DROP CONSTRAINT IF EXISTS fk_staged_aircraft_variant;
ALTER TABLE aircraft_ingest.staged_aircraft
    ADD CONSTRAINT fk_staged_aircraft_variant
        FOREIGN KEY (variant_id)
        REFERENCES aircraft_core.variants(id)
        ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_staged_aircraft_variant
    ON aircraft_ingest.staged_aircraft (variant_id)
    WHERE variant_id IS NOT NULL;

COMMENT ON COLUMN aircraft_ingest.staged_aircraft.variant_id IS
    'Canonical variant this staged record was promoted into. NULL while pending, '
    'and cleared if that variant is later deleted; the staged evidence remains.';

COMMIT;
