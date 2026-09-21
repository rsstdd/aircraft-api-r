-- =============================================================================
-- File: database/migrations/028_variant_powerplant_engine_count_optional.sql
-- Phase 28: Let a powerplant say it does not know how many engines there are.
--
-- aircraft_power.variant_powerplants.engine_count was NOT NULL, so ingestion
-- bound `engine.engine_count.unwrap_or(1)` and "the source states no count"
-- became indistinguishable from "one engine". The insert's
-- ON CONFLICT DO NOTHING then froze that value, so a later run that did state a
-- count could not correct it.
--
-- Thirteen variants carried the result, all of them detail pages listed under
-- two manufacturers whose second listing was recovered recently. Ten had a
-- powerplant claiming one engine while aircraft_core.variants.engine_count --
-- fed from the same parse on a later run, through a COALESCE that does update --
-- correctly said two, or four for a DC-8-62. Three state no count anywhere and
-- had a fabricated one against an honest NULL on the variant.
--
-- Migration 023 calls the powerplant link "the authoritative engine count" and
-- backfills aircraft_core.variants.engine_count from it. That is the right
-- direction and this migration restores it: the authority may now be NULL, the
-- ten stale rows are corrected from the projection that outran them, and the
-- three unknowns stop claiming a number. Its validation companion is
-- strengthened in the same change to compare the projection against the
-- authority rather than merely asserting the projection is not NULL, which is
-- what let the ten disagreements stand.
--
-- crates/aircraft_db/src/repositories/ingestion_repository.rs `promote_engine`
-- binds the stated count and names this file; change both together.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

ALTER TABLE aircraft_power.variant_powerplants
    ALTER COLUMN engine_count DROP NOT NULL;

-- The default is dropped with the constraint. Leaving DEFAULT 1 on a column
-- whose point is that "nobody counted" must be representable would just move
-- the fabrication: any insert omitting the column would still claim one engine,
-- and the NULL this migration makes possible would never be reached by that
-- path.
ALTER TABLE aircraft_power.variant_powerplants
    ALTER COLUMN engine_count DROP DEFAULT;

COMMENT ON COLUMN aircraft_power.variant_powerplants.engine_count IS
    'Engines of this powerplant fitted to the variant, as the source states it. '
    'NULL means the source does not say, which is not the same as one: '
    'aircraft_core.variants.engine_count projects this column and must agree '
    'with it, including when both are NULL.';

-- The projection is the better reading wherever the two disagree: it comes from
-- the same parse but through an upsert that accepts a later run's answer, so it
-- has seen evidence this column was frozen against.
UPDATE aircraft_power.variant_powerplants AS powerplant
SET engine_count = variant.engine_count
FROM aircraft_core.variants AS variant
WHERE variant.id = powerplant.variant_id
  AND powerplant.is_primary
  AND variant.engine_count IS NOT NULL
  AND powerplant.engine_count IS DISTINCT FROM variant.engine_count
  AND EXISTS (
      SELECT 1
      FROM aircraft_prov.source_documents AS document
      WHERE document.variant_id = variant.id
        AND document.ingest_run_id IS NOT NULL
  );

-- Where neither side knows, the fabricated count is withdrawn rather than
-- copied onto the variant. Scoped to ingested rows so manually curated
-- powerplants keep whatever a curator entered.
UPDATE aircraft_power.variant_powerplants AS powerplant
SET engine_count = NULL
FROM aircraft_core.variants AS variant
WHERE variant.id = powerplant.variant_id
  AND powerplant.is_primary
  AND variant.engine_count IS NULL
  AND powerplant.engine_count IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM aircraft_prov.source_documents AS document
      WHERE document.variant_id = variant.id
        AND document.ingest_run_id IS NOT NULL
  );

DO $projection$
DECLARE
    disagreements BIGINT;
BEGIN
    SELECT count(*)
    INTO disagreements
    FROM aircraft_power.variant_powerplants AS powerplant
    JOIN aircraft_core.variants AS variant ON variant.id = powerplant.variant_id
    WHERE powerplant.is_primary
      AND variant.engine_count IS DISTINCT FROM powerplant.engine_count
      AND EXISTS (
          SELECT 1
          FROM aircraft_prov.source_documents AS document
          WHERE document.variant_id = variant.id
            AND document.ingest_run_id IS NOT NULL
      );

    IF disagreements <> 0 THEN
        RAISE EXCEPTION
            '% ingested variants still disagree with their primary powerplant about the engine count',
            disagreements;
    END IF;
END
$projection$;

COMMIT;
