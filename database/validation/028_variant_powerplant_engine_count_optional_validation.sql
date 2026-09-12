-- Companion to database/migrations/028_variant_powerplant_engine_count_optional.sql.
--
-- Read-only: db-prod-validate runs this directory against production.
--
-- The projection invariant itself lives in
-- validation/023_backfill_ingestion_identity_projections_validation.sql, which
-- migration 028 strengthened. What is checked here is the thing 028 changed and
-- 023 cannot see: that "the source does not say" is still representable. A
-- NOT NULL constraint returning to this column would force ingestion back to
-- fabricating a count, and every projection check would go on passing while the
-- data quietly claimed one engine for aircraft nobody counted.

DO $validation$
DECLARE
    is_nullable TEXT;
BEGIN
    SELECT c.is_nullable
    INTO is_nullable
    FROM information_schema.columns AS c
    WHERE c.table_schema = 'aircraft_power'
      AND c.table_name = 'variant_powerplants'
      AND c.column_name = 'engine_count';

    IF is_nullable IS NULL THEN
        RAISE EXCEPTION
            'aircraft_power.variant_powerplants.engine_count is missing; the projection '
            'aircraft_core.variants.engine_count mirrors has no authority to mirror';
    END IF;

    IF is_nullable <> 'YES' THEN
        RAISE EXCEPTION
            'aircraft_power.variant_powerplants.engine_count is NOT NULL again; ingestion '
            'would have to invent a count for every source that states none, and the '
            'projection check in validation 023 would pass on the invention';
    END IF;
END
$validation$;
