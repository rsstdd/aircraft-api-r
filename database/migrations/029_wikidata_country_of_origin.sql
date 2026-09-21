-- =============================================================================
-- File: database/migrations/029_wikidata_country_of_origin.sql
-- Phase 29: Country of origin, from Wikidata, for the manufacturers whose
--           aircraft agree about it.
--
-- aircraft_core.variants.country_of_origin_code and the matching column on
-- families have been NULL on every row since the schema was written. PlanePHD
-- states no country, so the catalogue's own source cannot fill them and the
-- filter `aircraft_app::catalog`'s VariantFilter declares returns nothing.
--
-- Wikidata can, but not per aircraft: it models notable types where this
-- catalogue models year-range variants, so aircraft-to-aircraft matching finds
-- almost nothing. The join that works is at the manufacturer. For each
-- manufacturer this catalogue holds, its Wikidata aircraft were counted by
-- `?ac wdt:P176 <manufacturer> . ?ac wdt:P495 ?c . ?c wdt:P298 ?iso`, and the
-- dominant country assigned only where it covers **at least 90% of at least 10**
-- aircraft.
--
-- The threshold is the whole control and is not decoration. Relaxing it to
-- 80% of 5 admits Learjet -> CAN on 5 of 6, which is Bombardier's ownership
-- rather than where the aircraft were designed; at 90/10 Learjet is refused, and
-- so are Dassault (85%), Bombardier (88%) and every other genuinely mixed
-- manufacturer. A manufacturer resolved to the wrong Wikidata entity returns no
-- aircraft at all and is refused the same way, which is why the rule needs no
-- separate identity check.
--
-- What this records is therefore a manufacturer-level aggregate, not a
-- per-aircraft fact, and the assertions below say so in their notes. Aircraft
-- built under licence elsewhere are the known limitation: Reims-built Cessnas
-- carry USA here. This catalogue holds no Reims designation -- F150, F172 and
-- FR172 return no rows -- so none is currently misstated, and a curator
-- correcting one is why the variant update leaves a non-NULL value alone.
--
-- Wikidata is CC0. Retrieved 2026-09-13.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

INSERT INTO aircraft_prov.sources(
    name, slug, source_type_code, reliability_grade_code, base_url,
    default_confidence, license_notes, notes)
VALUES (
    'Wikidata', 'wikidata', 'IMPORTED_DATASET', 'MEDIUM',
    'https://query.wikidata.org/sparql', 0.70,
    'CC0 1.0 Universal public domain dedication '
    '(https://www.wikidata.org/wiki/Wikidata:Licensing). No attribution is '
    'required and redistribution is unrestricted, which is why values derived '
    'from it may be published where PlanePHD-derived values may not.',
    'Manufacturer-level aggregates only. Statements here are derived from the '
    'countries a manufacturer''s Wikidata aircraft agree on, not read from an '
    'individual aircraft.')
ON CONFLICT (slug) DO UPDATE SET
    source_type_code = EXCLUDED.source_type_code,
    reliability_grade_code = EXCLUDED.reliability_grade_code,
    license_notes = EXCLUDED.license_notes;

-- TEXT and BIGINT rather than mirroring the catalogue's VARCHAR(3)/SMALLINT:
-- the foreign key on aircraft_core is what constrains the country code, and a
-- scratch table that drops at COMMIT gains nothing from repeating it.
CREATE TEMP TABLE wikidata_country(
    family_slug TEXT PRIMARY KEY,
    country_code TEXT NOT NULL,
    agreeing BIGINT NOT NULL,
    sampled BIGINT NOT NULL
) ON COMMIT DROP;

INSERT INTO wikidata_country(family_slug, country_code, agreeing, sampled)
VALUES
    ('cessna-family', 'USA', 58, 60),
    ('beechcraft-family', 'USA', 96, 96),
    ('piper-family', 'USA', 22, 22),
    ('boeing-family', 'USA', 166, 168),
    ('gulfstream-family', 'USA', 10, 11),
    ('hawker-family', 'GBR', 11, 11),
    ('mitsubishi-family', 'JPN', 104, 114),
    ('embraer-family', 'BRA', 18, 20),
    ('fairchild-family', 'USA', 13, 13),
    ('mcdonnell-family', 'USA', 21, 21),
    ('piaggio-aero-industries-family', 'ITA', 12, 12),
    ('chance-vought-family', 'USA', 27, 27),
    ('lockheed-family', 'USA', 109, 111);

-- The rule, restated as a constraint rather than trusted. A row that does not
-- meet the threshold it claims to have met stops the migration.
DO $threshold$
DECLARE
    offending TEXT;
BEGIN
    SELECT family_slug INTO offending
      FROM wikidata_country
     WHERE sampled < 10 OR agreeing::numeric / sampled < 0.90
     LIMIT 1;
    IF offending IS NOT NULL THEN
        RAISE EXCEPTION
            'wikidata country row for % does not meet the 90%%-of-10 threshold this migration documents',
            offending;
    END IF;
END
$threshold$;

-- Provenance first: the evidence exists whether or not the value is published,
-- and it records the aggregate that produced it rather than just its result.
INSERT INTO aircraft_prov.source_documents(
    source_id, source_system_key, source_url, retrieved_at, raw_json,
    ingest_batch_label, parser_version, processing_status, notes)
SELECT
    source.id,
    'wikidata:country-of-origin:' || wikidata_country.family_slug,
    'https://query.wikidata.org/sparql',
    now(),
    jsonb_build_object(
        'family_slug', wikidata_country.family_slug,
        'country_of_origin_code', wikidata_country.country_code,
        'aircraft_agreeing', wikidata_country.agreeing,
        'aircraft_sampled', wikidata_country.sampled,
        'rule', 'dominant country over >=90% of >=10 aircraft reached by P176/P495/P298'),
    'wikidata-country-2026-09-13',
    'wikidata-manufacturer-aggregate/1.0.0',
    'PROCESSED',
    'Manufacturer-level aggregate, not a per-aircraft statement.'
FROM wikidata_country
JOIN aircraft_prov.sources AS source ON source.slug = 'wikidata'
WHERE EXISTS (
    SELECT 1 FROM aircraft_core.families AS family
     WHERE family.slug = wikidata_country.family_slug
);

INSERT INTO aircraft_prov.source_assertions(
    source_document_id, entity_type_code, entity_id, field_name,
    raw_value, asserted_value, status_code, is_accepted, confidence, notes)
SELECT
    document.id,
    'AIRCRAFT_FAMILY',
    family.id,
    'country_of_origin_code',
    wikidata_country.agreeing || '/' || wikidata_country.sampled || ' aircraft',
    wikidata_country.country_code,
    'ACCEPTED',
    TRUE,
    0.70,
    'Accepted by the threshold this migration documents, not by a curator '
    'reading an individual aircraft.'
FROM wikidata_country
JOIN aircraft_core.families AS family ON family.slug = wikidata_country.family_slug
JOIN aircraft_prov.source_documents AS document
  ON document.source_system_key = 'wikidata:country-of-origin:' || wikidata_country.family_slug
ON CONFLICT DO NOTHING;

UPDATE aircraft_core.families AS family
SET country_of_origin_code = wikidata_country.country_code,
    updated_at = clock_timestamp()
FROM wikidata_country
WHERE family.slug = wikidata_country.family_slug
  AND family.country_of_origin_code IS NULL;

-- Fanned out to the variants because aircraft_read.mv_variant_search reads the
-- variant column, so a family-level value alone would never reach the search
-- surface or the catalogue filter this exists to make answerable. A non-NULL
-- value is left alone: a curator correcting a licence-built airframe must not be
-- overwritten by the manufacturer's aggregate.
UPDATE aircraft_core.variants AS variant
SET country_of_origin_code = wikidata_country.country_code,
    updated_at = clock_timestamp()
FROM aircraft_core.models AS model
JOIN aircraft_core.families AS family ON family.id = model.family_id
JOIN wikidata_country ON wikidata_country.family_slug = family.slug
WHERE variant.model_id = model.id
  AND variant.country_of_origin_code IS NULL;

DO $published$
DECLARE
    families_set BIGINT;
    variants_set BIGINT;
    unset BIGINT;
BEGIN
    SELECT count(*) INTO families_set
      FROM aircraft_core.families AS family
      JOIN wikidata_country ON wikidata_country.family_slug = family.slug
     WHERE family.country_of_origin_code = wikidata_country.country_code;

    SELECT count(*) INTO variants_set
      FROM aircraft_core.variants AS variant
      JOIN aircraft_core.models AS model ON model.id = variant.model_id
      JOIN aircraft_core.families AS family ON family.id = model.family_id
      JOIN wikidata_country ON wikidata_country.family_slug = family.slug
     WHERE variant.country_of_origin_code = wikidata_country.country_code;

    -- A zero count is not a defect and cannot be treated as one: a database
    -- that has imported none of these thirteen manufacturers -- every database
    -- the automated suites build -- legitimately matches nothing, and that is
    -- indistinguishable from a changed slug shape. What is a defect is a family
    -- this migration *did* match and then failed to set, which is the only
    -- failure the statement above can actually produce.
    SELECT count(*)
    INTO unset
      FROM aircraft_core.families AS family
      JOIN wikidata_country ON wikidata_country.family_slug = family.slug
     WHERE family.country_of_origin_code IS NULL;

    IF unset <> 0 THEN
        RAISE EXCEPTION
            '% families matched a wikidata country row and were left without a country',
            unset;
    END IF;

    RAISE NOTICE 'wikidata country of origin: % families, % variants',
        families_set, variants_set;
END
$published$;

-- aircraft_read.mv_variant_search reads variant.country_of_origin_code, and
-- nothing else will rebuild it: the refresh queue is driven by curation
-- decisions, and writing the column directly enqueues no request. Migration 023
-- refreshes after its backfill for the same reason. Without this the values
-- exist and the search surface the filter actually reads still answers nothing.
SELECT aircraft_read.refresh_search_matviews(FALSE);

COMMIT;
