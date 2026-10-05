DO $validation$
DECLARE
    missing_engine_counts BIGINT;
    missing_manufacturers BIGINT;
    unpublished_manufacturers BIGINT;
BEGIN
    -- Compares the projection against the authority rather than asserting the
    -- projection is merely not NULL. The weaker form could not see ten ingested
    -- variants whose powerplant claimed one engine while the variant column
    -- said two, or four for a DC-8-62: both were non-NULL, so both passed.
    -- Since migration 028 the authority may itself be NULL, and IS DISTINCT
    -- FROM treats two NULLs as agreement -- a source that states no count
    -- leaves both sides silent, which is the honest answer and not a defect.
    SELECT count(*)
    INTO missing_engine_counts
    FROM aircraft_core.variants AS variant
    JOIN aircraft_power.variant_powerplants AS powerplant
        ON powerplant.variant_id = variant.id AND powerplant.is_primary
    WHERE variant.engine_count IS DISTINCT FROM powerplant.engine_count
      AND EXISTS (
          SELECT 1
          FROM aircraft_prov.source_documents AS document
          WHERE document.variant_id = variant.id
            AND document.ingest_run_id IS NOT NULL
      );

    SELECT count(*)
    INTO missing_manufacturers
    FROM aircraft_core.variants AS variant
    JOIN aircraft_core.models AS model ON model.id = variant.model_id
    JOIN aircraft_core.families AS family ON family.id = model.family_id
    WHERE family.manufacturer_org_id IS NOT NULL
      AND EXISTS (
          SELECT 1
          FROM aircraft_prov.source_documents AS document
          WHERE document.variant_id = variant.id
            AND document.ingest_run_id IS NOT NULL
      )
      AND NOT EXISTS (
          SELECT 1
          FROM aircraft_core.variant_manufacturers AS manufacturer
          WHERE manufacturer.variant_id = variant.id
            AND manufacturer.is_primary
      );

    SELECT count(*)
    INTO unpublished_manufacturers
    FROM aircraft_read.mv_variant_search AS search
    WHERE search.primary_manufacturer_id IS NULL
      AND EXISTS (
          SELECT 1
          FROM aircraft_prov.source_documents AS document
          WHERE document.variant_id = search.variant_id
            AND document.ingest_run_id IS NOT NULL
      );

    IF missing_engine_counts <> 0
       OR missing_manufacturers <> 0
       OR unpublished_manufacturers <> 0 THEN
        RAISE EXCEPTION
            'ingestion identity projections incomplete: engine counts disagreeing with their powerplant %, manufacturer links %, read-model manufacturers %',
            missing_engine_counts,
            missing_manufacturers,
            unpublished_manufacturers;
    END IF;
END
$validation$;
