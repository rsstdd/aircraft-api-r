-- Validation for migration 018: the staged-to-canonical link must be enforced.
DO $validation$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_staged_aircraft_variant'
          AND conrelid = 'aircraft_ingest.staged_aircraft'::regclass
          AND contype = 'f'
          AND confrelid = 'aircraft_core.variants'::regclass
    ) THEN
        RAISE EXCEPTION 'staged_aircraft.variant_id must reference aircraft_core.variants';
    END IF;

    -- SET NULL, not CASCADE: deleting a variant must not destroy the evidence
    -- that it was ingested.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_staged_aircraft_variant'
          AND conrelid = 'aircraft_ingest.staged_aircraft'::regclass
          AND confdeltype = 'n'
    ) THEN
        RAISE EXCEPTION 'fk_staged_aircraft_variant must use ON DELETE SET NULL';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ingest.staged_aircraft sa
        WHERE sa.variant_id IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM aircraft_core.variants v WHERE v.id = sa.variant_id)
    ) THEN
        RAISE EXCEPTION 'staged_aircraft rows point at variants that do not exist';
    END IF;
END
$validation$;
