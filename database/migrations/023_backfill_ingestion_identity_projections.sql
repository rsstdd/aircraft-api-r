-- =============================================================================
-- File: database/migrations/023_backfill_ingestion_identity_projections.sql
-- Phase 23: Complete source-backed identity projections for Rust ingestion.
--
-- Rust ingestion already creates the manufacturer organization, attaches it to
-- the family, and records the authoritative engine count on the primary
-- powerplant link. It did not populate the variant-level projections consumed
-- by mv_variant_search, leaving every imported manufacturer and declared engine
-- count NULL despite having an unambiguous source-backed relationship.
--
-- Limit the backfill to variants with Rust ingestion documents. Historical and
-- manually curated variants keep their existing relationship semantics.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

UPDATE aircraft_core.variants AS variant
SET engine_count = powerplant.engine_count,
    updated_at = clock_timestamp()
FROM aircraft_power.variant_powerplants AS powerplant
WHERE powerplant.variant_id = variant.id
  AND powerplant.is_primary
  AND variant.engine_count IS NULL
  AND EXISTS (
      SELECT 1
      FROM aircraft_prov.source_documents AS document
      WHERE document.variant_id = variant.id
        AND document.ingest_run_id IS NOT NULL
  );

INSERT INTO aircraft_core.variant_manufacturers(
    variant_id, org_id, role, is_primary
)
SELECT
    variant.id,
    family.manufacturer_org_id,
    'MANUFACTURER',
    TRUE
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
  )
ON CONFLICT (variant_id, org_id) DO NOTHING;

SELECT aircraft_read.refresh_search_matviews(FALSE);

COMMIT;
