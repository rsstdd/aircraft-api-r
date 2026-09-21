-- Companion to database/migrations/030_wikidata_model_first_flight.sql.
--
-- Read-only: db-prod-validate runs this directory against production.
--
-- Row counts are not asserted, for the reason migration 029's companion gives:
-- a database holding none of these models is valid and is what every automated
-- suite builds. What is asserted is the invariant that decided which matches
-- were safe to write, applied to whatever the catalogue holds rather than to
-- the thirty rows the migration carried -- so a first flight arriving later by
-- any other route is held to the same rule.

DO $validation$
DECLARE
    contradicting BIGINT;
    undocumented BIGINT;
BEGIN
    -- A type cannot have flown after the aircraft built from it went on sale.
    -- This is what rejected the 737-100 and 737-600 during the migration.
    SELECT count(*)
    INTO contradicting
    FROM aircraft_core.models AS model
    JOIN aircraft_core.variants AS variant ON variant.model_id = model.id
    WHERE model.first_flight_year IS NOT NULL
      AND variant.production_start_year IS NOT NULL
      AND model.first_flight_year > variant.production_start_year;

    IF contradicting <> 0 THEN
        RAISE EXCEPTION
            '% models record a first flight later than a production start on their own variants',
            contradicting;
    END IF;

    -- Every first flight this source published must still have its assertion.
    -- Scoped to models the wikidata source actually asserted about, so a date a
    -- curator enters by hand is not required to carry one.
    SELECT count(*)
    INTO undocumented
    FROM aircraft_prov.source_assertions AS assertion
    JOIN aircraft_prov.source_documents AS document ON document.id = assertion.source_document_id
    JOIN aircraft_prov.sources AS source ON source.id = document.source_id
    JOIN aircraft_core.models AS model ON model.id = assertion.entity_id
    WHERE source.slug = 'wikidata'
      AND assertion.entity_type_code = 'AIRCRAFT_MODEL'
      AND assertion.field_name = 'first_flight_year'
      AND assertion.is_accepted
      AND model.first_flight_year IS DISTINCT FROM assertion.asserted_numeric::smallint;

    IF undocumented <> 0 THEN
        RAISE EXCEPTION
            '% models disagree with the accepted first-flight assertion behind them',
            undocumented;
    END IF;
END
$validation$;
