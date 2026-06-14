-- =============================================================================
-- File: database/validation/phase3_geo_org_validation.sql
-- Phase 3 — validation queries for aircraft_geo and aircraft_org tables.
-- All queries are SELECT-only.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. ROW COUNTS
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_geo.countries)            AS countries_total,
    (SELECT count(*) FROM aircraft_geo.countries WHERE is_active)           AS countries_active,
    (SELECT count(*) FROM aircraft_geo.countries WHERE NOT is_active)       AS countries_historical,
    (SELECT count(*) FROM aircraft_geo.regions)              AS regions_total,
    (SELECT count(*) FROM aircraft_geo.country_regions)      AS country_region_mappings,
    (SELECT count(*) FROM aircraft_org.organizations)        AS organizations_total,
    (SELECT count(*) FROM aircraft_org.organizations WHERE is_active)       AS orgs_active,
    (SELECT count(*) FROM aircraft_org.organizations WHERE NOT is_active)   AS orgs_historical,
    (SELECT count(*) FROM aircraft_org.org_relationships)    AS org_relationships_total;
-- Expected: 81 total countries; 77 active; 4 historical;
--           11 regions; ~120 country_region mappings;
--           27 orgs total; ~18 active; ~9 historical; 6 relationships.

-- -----------------------------------------------------------------------------
-- 2. COUNTRIES: check all continents are populated
-- -----------------------------------------------------------------------------
SELECT continent, count(*) AS country_count, bool_and(is_active) AS all_active
FROM aircraft_geo.countries
GROUP BY continent
ORDER BY country_count DESC;
-- Expect: EUROPE has most rows (34: 30 active + 4 historical);
--         all historical rows in the EUROPE continent.

-- -----------------------------------------------------------------------------
-- 3. HISTORICAL STATES: all 4 present and inactive
-- -----------------------------------------------------------------------------
SELECT code, alpha2, name, is_active
FROM aircraft_geo.countries
WHERE NOT is_active
ORDER BY code;
-- Expect: CSK, DDR, SUN, YUG — all is_active = FALSE.

-- -----------------------------------------------------------------------------
-- 4. REGIONS: all four region_types represented
-- -----------------------------------------------------------------------------
SELECT region_type, count(*) AS region_count,
       string_agg(code, ', ' ORDER BY sort_order) AS codes
FROM aircraft_geo.regions
GROUP BY region_type
ORDER BY region_type;
-- Expect: AVIATION (2), GEOGRAPHIC (6), MILITARY (1), POLITICAL (2).

-- -----------------------------------------------------------------------------
-- 5. COUNTRY_REGIONS: every country mapped to at least one geographic region
-- -----------------------------------------------------------------------------
SELECT c.code, c.name, c.continent,
       count(cr.region_code) AS region_count,
       string_agg(cr.region_code, ', ' ORDER BY cr.region_code) AS regions
FROM aircraft_geo.countries c
         LEFT JOIN aircraft_geo.country_regions cr ON cr.country_code = c.code
GROUP BY c.code, c.name, c.continent
HAVING count(cr.region_code) = 0
ORDER BY c.code;
-- Expect: zero rows (every country should be in at least one region).

-- -----------------------------------------------------------------------------
-- 6. COUNTRY_REGIONS: coverage of key political/aviation groups
-- -----------------------------------------------------------------------------
SELECT r.code AS region_code, r.label, r.region_type, count(cr.country_code) AS members
FROM aircraft_geo.regions r
         LEFT JOIN aircraft_geo.country_regions cr ON cr.region_code = r.code
GROUP BY r.code, r.label, r.region_type
ORDER BY r.sort_order;
-- Expect: NATO ≥ 28 members, EASA_STATES ≥ 27, EU ≥ 25.

-- -----------------------------------------------------------------------------
-- 7. ORGANIZATIONS: FK integrity — all country_codes reference valid countries
-- -----------------------------------------------------------------------------
SELECT o.slug, o.name, o.country_code,
       CASE WHEN c.code IS NOT NULL THEN 'OK'
            WHEN o.country_code IS NULL THEN 'NULL (international)'
            ELSE 'BROKEN FK ← investigate'
           END AS country_status
FROM aircraft_org.organizations o
         LEFT JOIN aircraft_geo.countries c ON c.code = o.country_code
ORDER BY o.slug;
-- Expect: zero rows with 'BROKEN FK'. NULL is acceptable for international bodies.

-- -----------------------------------------------------------------------------
-- 8. ORGANIZATIONS: org_type distribution
-- -----------------------------------------------------------------------------
SELECT org_type_code, count(*) AS org_count,
       bool_or(is_active) AS has_active,
       bool_or(NOT is_active) AS has_historical
FROM aircraft_org.organizations
GROUP BY org_type_code
ORDER BY org_count DESC;
-- Expect: MANUFACTURER is the largest group; DESIGN_BUREAU for Soviet/Russian.

-- -----------------------------------------------------------------------------
-- 9. ORGANIZATIONS: dissolved logic consistency
-- All dissolved_year IS NOT NULL → is_active should be FALSE.
-- (Application-level invariant; validated here.)
-- -----------------------------------------------------------------------------
SELECT slug, name, founded_year, dissolved_year, is_active
FROM aircraft_org.organizations
WHERE dissolved_year IS NOT NULL
  AND is_active = TRUE;
-- Expect: zero rows (dissolved organizations should be inactive).
-- Note: this is not enforced by DDL CHECK; any rows here indicate data entry
-- inconsistency and should be corrected.

-- -----------------------------------------------------------------------------
-- 10. ORG_RELATIONSHIPS: no self-referential rows; all FKs valid
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_org.org_relationships
     WHERE from_org_id = to_org_id)              AS self_ref_count,   -- expect 0
    (SELECT count(*) FROM aircraft_org.org_relationships r
                              LEFT JOIN aircraft_org.organizations f ON f.id = r.from_org_id
     WHERE f.id IS NULL)                          AS broken_from_fk,  -- expect 0
    (SELECT count(*) FROM aircraft_org.org_relationships r
                              LEFT JOIN aircraft_org.organizations t ON t.id = r.to_org_id
     WHERE t.id IS NULL)                          AS broken_to_fk;    -- expect 0

-- -----------------------------------------------------------------------------
-- 11. ORG_RELATIONSHIPS: succession chains — verify all 6 seeded
-- -----------------------------------------------------------------------------
SELECT
    f.slug AS from_slug,
    r.relationship_type_code,
    t.slug AS to_slug,
    r.started_year
FROM aircraft_org.org_relationships r
         JOIN aircraft_org.organizations f ON f.id = r.from_org_id
         JOIN aircraft_org.organizations t ON t.id = r.to_org_id
ORDER BY r.started_year, f.slug;
-- Expect 6 rows: aeronca→american-champion (1954), north-american→boeing (1996),
-- grumman→northrop-grumman (1994), lockheed-corp→lockheed-martin (1995),
-- cessna→textron-aviation (2014), beech→textron-aviation (2014).

-- -----------------------------------------------------------------------------
-- 12. SPOT-CHECK: key organizations resolvable by common name alias
-- Phase 17 ingestion uses name_aliases to match manufacturer strings.
-- Verify the GIN index and array-contains operator work correctly.
-- -----------------------------------------------------------------------------
SELECT slug, name, common_name
FROM aircraft_org.organizations
WHERE name_aliases @> ARRAY['Cessna']
UNION ALL
SELECT slug, name, common_name
FROM aircraft_org.organizations
WHERE name_aliases @> ARRAY['MiG']
UNION ALL
SELECT slug, name, common_name
FROM aircraft_org.organizations
WHERE name_aliases @> ARRAY['Aeronca']
ORDER BY slug;
-- Expect 3 rows: cessna-aircraft-company, mikoyan, aeronca-aircraft.

-- -----------------------------------------------------------------------------
-- 13. TRIGRAM SEARCH: verify gin_trgm_ops indexes work
-- -----------------------------------------------------------------------------
SELECT code, name, continent
FROM aircraft_geo.countries
WHERE name ILIKE '%united%'
ORDER BY name;
-- Expect: United Arab Emirates, United Kingdom, United States.

SELECT slug, name, common_name
FROM aircraft_org.organizations
WHERE name ILIKE '%grumman%'
ORDER BY name;
-- Expect: grumman-aerospace and northrop-grumman.

-- -----------------------------------------------------------------------------
-- 14. SUMMARY: tables and index counts for aircraft_geo and aircraft_org
-- -----------------------------------------------------------------------------
SELECT n.nspname AS schema_name,
       count(*) FILTER (WHERE c.relkind = 'r') AS table_count,
    count(*) FILTER (WHERE c.relkind = 'i') AS index_count
FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname IN ('aircraft_geo', 'aircraft_org')
GROUP BY n.nspname
ORDER BY n.nspname;
-- Expect: aircraft_geo: 3 tables, aircraft_org: 2 tables;
--         combined index count ≥ 15.