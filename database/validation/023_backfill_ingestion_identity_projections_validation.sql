DO $validation$
DECLARE
    missing_engine_counts BIGINT;
    missing_manufacturers BIGINT;
    unpublished_manufacturers BIGINT;
BEGIN
    SELECT count(*)
    INTO missing_engine_counts
    FROM aircraft_core.variants AS variant
    JOIN aircraft_power.variant_powerplants AS powerplant
        ON powerplant.variant_id = variant.id AND powerplant.is_primary
    WHERE variant.engine_count IS NULL
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
            'ingestion identity projections incomplete: engine counts %, manufacturer links %, read-model manufacturers %',
            missing_engine_counts,
            missing_manufacturers,
            unpublished_manufacturers;
    END IF;
END
$validation$;
