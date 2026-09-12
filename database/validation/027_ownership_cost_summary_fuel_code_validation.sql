-- Companion to database/migrations/027_ownership_cost_summary_fuel_code.sql.
--
-- Read-only: db-prod-validate runs this directory against production.
--
-- The check is on the view's *definition*, not on its contents, because
-- contents cannot distinguish the two states this migration exists to separate.
-- A database with no fuel line items has a NULL hourly_fuel_cost_usd whether the
-- filter names FUEL or a code that does not exist, so an assertion over rows
-- would pass on the broken view for every empty or freshly installed database --
-- which is every database the automated suites build.
--
-- Asserting that the definition references no cost code absent from
-- aircraft_ref.cost_item_types is the gate that fails when a read model is
-- written against a vocabulary entry nobody seeded.

DO $validation$
DECLARE
    definition TEXT;
    orphan TEXT;
BEGIN
    IF to_regclass('aircraft_read.mv_ownership_cost_summary') IS NULL THEN
        RAISE EXCEPTION 'aircraft_read.mv_ownership_cost_summary must exist';
    END IF;

    definition := pg_get_viewdef('aircraft_read.mv_ownership_cost_summary'::regclass, TRUE);

    IF definition NOT LIKE '%cost_item_type_code%' THEN
        RAISE EXCEPTION
            'mv_ownership_cost_summary no longer identifies costs by cost_item_type_code; '
            'this check was written against a definition that does and can no longer see '
            'the defect it guards';
    END IF;

    -- Every quoted token the definition compares cost_item_type_code against
    -- must be a seeded code. 'FUEL_COST_PER_HOUR' was not, for the life of
    -- migration 016.
    SELECT candidate INTO orphan
    FROM (
        SELECT (regexp_matches(
                    definition,
                    'cost_item_type_code::text (?:=|<>) ''([A-Z_]+)''::text',
                    'g'))[1] AS candidate
    ) AS referenced
    WHERE NOT EXISTS (
        SELECT 1 FROM aircraft_ref.cost_item_types WHERE code = candidate
    )
    LIMIT 1;

    IF orphan IS NOT NULL THEN
        RAISE EXCEPTION
            'mv_ownership_cost_summary filters on cost code %, which aircraft_ref.cost_item_types '
            'does not contain; the column it computes can only ever be NULL',
            orphan;
    END IF;
END
$validation$;
