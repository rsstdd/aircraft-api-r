-- =============================================================================
-- File: database/migrations/030_wikidata_model_first_flight.sql
-- Phase 30: First-flight years, for the models whose designation matches a
--           Wikidata aircraft exactly.
--
-- Migration 029 filled country of origin by joining at the manufacturer, which
-- works because a manufacturer's aircraft agree about their country. First
-- flight admits no such aggregate: it is a property of one specific type, so
-- the only honest join is a match on the designation itself.
--
-- Exact match only, and deliberately nothing looser. Wikidata's "Cessna 172
-- Skyhawk" first flew in 1955 and this catalogue's "172S Skyhawk SP" in 1998,
-- so a prefix or fuzzy match would write the first number onto the second
-- model and be wrong by forty-three years. Thirty of the 688 models under the
-- manufacturers 029 qualified match exactly; the remaining 658 keep a NULL,
-- which is the correct value for a year nobody has established.
--
-- A first flight cannot postdate the production it precedes, and that invariant
-- is enforced below rather than assumed. It is not decoration: it rejected the
-- 737-100 and 737-600, whose PlanePHD production ranges begin in 1965 and 1995
-- against Wikidata first flights of 1967 and 1998. Whichever side is wrong,
-- writing the pair would publish a contradiction.
--
-- aircraft_read.mv_variant_search does not read this column, so nothing is
-- refreshed here.
--
-- Wikidata is CC0; the source row and its licence note come from migration 029.
-- Retrieved 2026-09-13. scripts/wikidata-first-flight.sh re-derives the table.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

CREATE TEMP TABLE wikidata_first_flight(
    model_slug TEXT PRIMARY KEY,
    first_flight_year BIGINT NOT NULL
) ON COMMIT DROP;

INSERT INTO wikidata_first_flight(model_slug, first_flight_year)
VALUES
    ('beechcraft-4000-2008-2012', 2005),
    ('beechcraft-king-air-100-1969-1983', 1969),
    ('boeing-737-800-1998-present', 1997),
    ('boeing-767-300-1986-2000', 1986),
    ('cessna-140-1946-1948', 1945),
    ('cessna-150-1959-1960', 1957),
    ('cessna-162-skycatcher-2009-2013', 2006),
    ('cessna-170-1948-1948', 1948),
    ('cessna-172m-skyhawk-1973-1975', 1973),
    ('cessna-172m-skyhawk-1976-1976', 1973),
    ('cessna-177-cardinal-1968-1968', 1966),
    ('cessna-195-1947-1947', 1945),
    ('cessna-207-1969-1976', 1968),
    ('cessna-400-2004-2010', 2000),
    ('cessna-414-1970-1977', 1968),
    ('cessna-425-conquest-i-1983-1986', 1978),
    ('cessna-441-conquest-ii-1977-1987', 1974),
    ('cessna-citation-longitude-2017-2019', 2016),
    ('cessna-t303-crusader-1982-1984', 1978),
    ('embraer-lineage-1000-2009-2013', 2007),
    ('embraer-phenom-100-2008-2013', 2007),
    ('gulfstream-g200-1999-2011', 1997),
    ('gulfstream-g280-2012-present', 2009),
    ('gulfstream-g500-2003-2012', 1995),
    ('gulfstream-g550-2003-present', 2002),
    ('piaggio-aero-industries-p-180-avanti-1990-2013', 1986),
    ('piper-j-3-cub-1938-1947', 1938),
    ('piper-pa-16-clipper-1949-1949', 1947),
    ('piper-pa-30-twin-comanche-1963-1965', 1962),
    ('piper-pa-34-seneca-1971-1974', 1967);

-- The invariant restated as a constraint against the catalogue, not trusted
-- from the table above. A row that contradicts the production it precedes stops
-- the migration rather than being written and explained later.
DO $invariant$
DECLARE
    offending TEXT;
BEGIN
    SELECT model.slug
      INTO offending
      FROM wikidata_first_flight AS wikidata
      JOIN aircraft_core.models AS model ON model.slug = wikidata.model_slug
      JOIN aircraft_core.variants AS variant ON variant.model_id = model.id
     WHERE variant.production_start_year IS NOT NULL
       AND wikidata.first_flight_year > variant.production_start_year
     LIMIT 1;

    IF offending IS NOT NULL THEN
        RAISE EXCEPTION
            'first flight for % postdates the production start this catalogue records',
            offending;
    END IF;
END
$invariant$;

INSERT INTO aircraft_prov.source_documents(
    source_id, source_system_key, source_url, retrieved_at, raw_json,
    ingest_batch_label, parser_version, processing_status, notes)
SELECT
    source.id,
    'wikidata:first-flight:' || wikidata.model_slug,
    'https://query.wikidata.org/sparql',
    now(),
    jsonb_build_object(
        'model_slug', wikidata.model_slug,
        'first_flight_year', wikidata.first_flight_year,
        'rule', 'exact designation match against a Wikidata aircraft carrying P606'),
    'wikidata-first-flight-2026-09-13',
    'wikidata-designation-match/1.0.0',
    'PROCESSED',
    'Matched on the designation alone; no prefix or fuzzy matching was used.'
FROM wikidata_first_flight AS wikidata
JOIN aircraft_prov.sources AS source ON source.slug = 'wikidata'
WHERE EXISTS (
    SELECT 1 FROM aircraft_core.models AS model WHERE model.slug = wikidata.model_slug
);

INSERT INTO aircraft_prov.source_assertions(
    source_document_id, entity_type_code, entity_id, field_name,
    raw_value, asserted_value, asserted_numeric, status_code, is_accepted,
    confidence, notes)
SELECT
    document.id,
    'AIRCRAFT_MODEL',
    model.id,
    'first_flight_year',
    wikidata.first_flight_year::text,
    wikidata.first_flight_year::text,
    wikidata.first_flight_year,
    'ACCEPTED',
    TRUE,
    0.70,
    'Exact designation match, and the year precedes the recorded production start.'
FROM wikidata_first_flight AS wikidata
JOIN aircraft_core.models AS model ON model.slug = wikidata.model_slug
JOIN aircraft_prov.source_documents AS document
  ON document.source_system_key = 'wikidata:first-flight:' || wikidata.model_slug
ON CONFLICT DO NOTHING;

-- A non-NULL value is left alone for the reason 029 leaves country alone: a
-- curator who established a date from a type certificate outranks a designation
-- match.
UPDATE aircraft_core.models AS model
SET first_flight_year = wikidata.first_flight_year,
    updated_at = clock_timestamp()
FROM wikidata_first_flight AS wikidata
WHERE model.slug = wikidata.model_slug
  AND model.first_flight_year IS NULL;

DO $published$
DECLARE
    matched BIGINT;
    unset BIGINT;
BEGIN
    SELECT count(*) INTO matched
      FROM aircraft_core.models AS model
      JOIN wikidata_first_flight AS wikidata ON wikidata.model_slug = model.slug;

    SELECT count(*) INTO unset
      FROM aircraft_core.models AS model
      JOIN wikidata_first_flight AS wikidata ON wikidata.model_slug = model.slug
     WHERE model.first_flight_year IS NULL;

    -- Zero matches is valid: a database holding none of these models, which is
    -- every database the automated suites build. A model this migration matched
    -- and then left unset is not.
    IF unset <> 0 THEN
        RAISE EXCEPTION
            '% models matched a wikidata first flight and were left without one', unset;
    END IF;

    RAISE NOTICE 'wikidata first flight: % models', matched;
END
$published$;

COMMIT;
