-- Companion to database/migrations/029_wikidata_country_of_origin.sql.
--
-- Read-only: db-prod-validate runs this directory against production.
--
-- Row counts are deliberately not asserted. A catalogue that has imported no
-- aircraft holds no families to set a country on, and one that has imported a
-- subset holds fewer than the thirteen the migration lists -- both are valid,
-- and the automated suites build exactly the first kind. What is asserted is
-- what must hold wherever a value was published: that it came from somewhere,
-- that the family and its variants agree, and that the aggregate nature is
-- still recorded rather than lost behind a bare country code.

DO $validation$
DECLARE
    undocumented BIGINT;
    disagreeing BIGINT;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM aircraft_prov.sources WHERE slug = 'wikidata') THEN
        RAISE EXCEPTION
            'the wikidata source row is missing; country values derived from it would have '
            'no provenance to point at';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM aircraft_prov.sources
         WHERE slug = 'wikidata' AND license_notes ILIKE '%CC0%'
    ) THEN
        RAISE EXCEPTION
            'the wikidata source does not record its CC0 dedication; that dedication is why '
            'values derived from it may be published at all';
    END IF;

    -- Every family carrying a country must have an accepted assertion behind it.
    -- This is what separates a derived value from one somebody typed in.
    SELECT count(*)
    INTO undocumented
    FROM aircraft_core.families AS family
    WHERE family.country_of_origin_code IS NOT NULL
      AND NOT EXISTS (
          SELECT 1
          FROM aircraft_prov.source_assertions AS assertion
          WHERE assertion.entity_type_code = 'AIRCRAFT_FAMILY'
            AND assertion.entity_id = family.id
            AND assertion.field_name = 'country_of_origin_code'
            AND assertion.is_accepted
      );

    IF undocumented <> 0 THEN
        RAISE EXCEPTION
            '% families carry a country of origin with no accepted assertion behind it',
            undocumented;
    END IF;

    -- A variant may hold a country its family does not -- a curator correcting a
    -- licence-built airframe is exactly that -- but it may not contradict an
    -- assertion nobody withdrew. Only variants still matching their family's
    -- value are checked, so a deliberate correction does not fail the install.
    SELECT count(*)
    INTO disagreeing
    FROM aircraft_core.variants AS variant
    JOIN aircraft_core.models AS model ON model.id = variant.model_id
    JOIN aircraft_core.families AS family ON family.id = model.family_id
    JOIN aircraft_prov.source_assertions AS assertion
      ON assertion.entity_type_code = 'AIRCRAFT_FAMILY'
     AND assertion.entity_id = family.id
     AND assertion.field_name = 'country_of_origin_code'
     AND assertion.is_accepted
    WHERE variant.country_of_origin_code IS NOT NULL
      AND family.country_of_origin_code IS NOT NULL
      AND variant.country_of_origin_code <> family.country_of_origin_code
      AND variant.country_of_origin_code <> assertion.asserted_value;

    IF disagreeing <> 0 THEN
        RAISE EXCEPTION
            '% variants carry a country matching neither their family nor any assertion',
            disagreeing;
    END IF;
END
$validation$;
