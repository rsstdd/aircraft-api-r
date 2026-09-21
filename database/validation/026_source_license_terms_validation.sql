-- Companion to database/migrations/026_source_license_terms.sql.
--
-- Read-only: db-prod-validate runs this directory against production.
--
-- The assertion is positive on both halves rather than a NOT NULL check alone.
-- A NULL check passes on an empty string, and it also passes on a note that
-- records neither of the two documents the terms actually come from -- which is
-- the state this migration exists to end. Each half names the document it
-- requires, so a failure says which one went missing.

DO $validation$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM aircraft_prov.sources WHERE slug = 'planephd') THEN
        RAISE EXCEPTION
            'aircraft_prov.sources has no planephd row; migration 014 seeds it '
            'and migration 026 records its licence against that slug';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM aircraft_prov.sources
         WHERE slug = 'planephd'
           AND license_notes ILIKE '%planephd.com/terms%'
    ) THEN
        RAISE EXCEPTION
            'the planephd source does not record its Terms of Service; every '
            'value derived from it is reference-only and license_notes is where '
            'that is written down';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM aircraft_prov.sources
         WHERE slug = 'planephd'
           AND license_notes ILIKE '%ai-train=no%'
    ) THEN
        RAISE EXCEPTION
            'the planephd source does not record its robots.txt content signals; '
            'the reservation of rights they express is part of the licence';
    END IF;
END
$validation$;
