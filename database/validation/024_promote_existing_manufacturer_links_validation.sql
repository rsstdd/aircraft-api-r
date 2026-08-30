-- Companion to database/migrations/024_promote_existing_manufacturer_links.sql.
--
-- Migration 023's own validation counts imported variants with no primary
-- manufacturer link at all. This one names the case 023 could not repair: the
-- correct (variant_id, org_id) link is present but demoted, so 023's INSERT
-- conflicted and DO NOTHING left it that way. After 024 there must be none.
DO $validation$
DECLARE
    demoted_manufacturers BIGINT;
BEGIN
    SELECT count(*)
    INTO demoted_manufacturers
    FROM aircraft_core.variant_manufacturers AS link
    JOIN aircraft_core.variants AS variant ON variant.id = link.variant_id
    JOIN aircraft_core.models AS model ON model.id = variant.model_id
    JOIN aircraft_core.families AS family ON family.id = model.family_id
    WHERE link.org_id = family.manufacturer_org_id
      AND NOT link.is_primary
      AND EXISTS (
          SELECT 1
          FROM aircraft_prov.source_documents AS document
          WHERE document.variant_id = variant.id
            AND document.ingest_run_id IS NOT NULL
      )
      AND NOT EXISTS (
          SELECT 1
          FROM aircraft_core.variant_manufacturers AS standing
          WHERE standing.variant_id = variant.id
            AND standing.is_primary
      );

    IF demoted_manufacturers <> 0 THEN
        RAISE EXCEPTION
            'imported variants left with a demoted manufacturer link: %',
            demoted_manufacturers;
    END IF;
END
$validation$;
