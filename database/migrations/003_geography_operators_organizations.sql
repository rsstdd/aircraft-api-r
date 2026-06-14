-- =============================================================================
-- File: database/migrations/003_geography_operators_organizations.sql
-- Phase 3 — aircraft_geo (countries, regions, country_regions) and
--           aircraft_org (organizations, org_relationships)
--
-- This file contains both DDL and seed data in a single transaction.
-- Seed order: countries → regions → country_regions → organizations
--             → org_relationships (depends on org IDs via subselects).
--
-- Historical states (Soviet Union, East Germany, Czechoslovakia, Yugoslavia)
-- are included with is_active = FALSE. They are required because many
-- aircraft variants in our seed data carry these as country_of_origin.
-- ISO alpha-3 codes for historical states follow the last-published
-- ISO 3166-1 codes before those states dissolved.
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_geo.countries
-- Master ISO 3166-1 country table. Also holds historical states
-- (is_active = FALSE) needed for origin attribution of legacy aircraft.
-- =============================================================================

CREATE TABLE aircraft_geo.countries (
                                        code          VARCHAR(3)  PRIMARY KEY,  -- ISO 3166-1 alpha-3
                                        alpha2        VARCHAR(2)  UNIQUE,       -- ISO 3166-1 alpha-2
                                        name          TEXT        NOT NULL,     -- common English name
                                        official_name TEXT,                    -- full official English name (nullable)
    -- Informational geographic continent grouping; not a FK.
    -- Values: NORTH_AMERICA, LATIN_AMERICA, EUROPE, MIDDLE_EAST,
    --         ASIA_EAST, ASIA_SOUTH, ASIA_SOUTHEAST, CENTRAL_ASIA,
    --         OCEANIA, AFRICA
                                        continent     TEXT,
                                        is_active     BOOLEAN     NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_geo.countries IS
    'ISO 3166-1 country registry. is_active = FALSE flags historical states '
    '(e.g., Soviet Union, Czechoslovakia) that are still referenced as '
    'aircraft countries of origin. country_regions (M:N) maps countries '
    'to named aviation/political regions for faceted filtering.';
COMMENT ON COLUMN aircraft_geo.countries.continent IS
    'Informational geographic continent grouping for quick regional faceting. '
    'Political and aviation-authority groupings are modelled in country_regions.';
COMMENT ON COLUMN aircraft_geo.countries.is_active IS
    'FALSE for dissolved or historical states still needed for origin '
    'attribution of legacy aircraft (e.g., SUN = Soviet Union).';

-- =============================================================================
-- aircraft_geo.regions
-- Named geographic, political, aviation-authority, and military-alliance
-- groupings. Countries link to regions via the country_regions M:N junction.
-- region_type discriminates grouping intent.
-- =============================================================================

CREATE TABLE aircraft_geo.regions (
                                      code        aircraft_ref.lookup_code PRIMARY KEY,
                                      label       TEXT     NOT NULL,
                                      description TEXT,
    -- 'GEOGRAPHIC': continents / sub-continental zones
    -- 'POLITICAL': political unions and intergovernmental bodies
    -- 'AVIATION': aviation regulatory authority jurisdictions
    -- 'MILITARY': military alliances
                                      region_type TEXT     NOT NULL DEFAULT 'GEOGRAPHIC',
                                      sort_order  SMALLINT NOT NULL DEFAULT 0,
                                      CONSTRAINT chk_regions_type CHECK (
                                          region_type IN ('GEOGRAPHIC', 'POLITICAL', 'AVIATION', 'MILITARY')
                                          )
);
COMMENT ON TABLE aircraft_geo.regions IS
    'Named groupings used for faceted filtering of aircraft by region '
    '(e.g., NATO, EASA_STATES, EUROPE, ASIA_PACIFIC). '
    'A country may belong to multiple regions via country_regions.';
COMMENT ON COLUMN aircraft_geo.regions.region_type IS
    'GEOGRAPHIC = continent/sub-region; POLITICAL = union/alliance; '
    'AVIATION = regulatory jurisdiction; MILITARY = defence alliance. '
    'A TEXT CHECK is used here (not a lookup table) because these four '
    'categories are definitionally complete and non-extensible by design.';

-- =============================================================================
-- aircraft_geo.country_regions
-- M:N junction: a country belongs to one or more regions.
-- Examples: Germany → EUROPE (geographic), EU (political),
--           EASA_STATES (aviation), NATO (military).
-- =============================================================================

CREATE TABLE aircraft_geo.country_regions (
                                              country_code VARCHAR(3)               NOT NULL
                                                  REFERENCES aircraft_geo.countries(code) ON DELETE CASCADE,
                                              region_code  aircraft_ref.lookup_code NOT NULL
                                                  REFERENCES aircraft_geo.regions(code)   ON DELETE CASCADE,
                                              joined_year  aircraft_ref.year_value,  -- year country joined this region/group
                                              notes        TEXT,
                                              PRIMARY KEY (country_code, region_code)
);
COMMENT ON TABLE aircraft_geo.country_regions IS
    'M:N junction between countries and named regions. '
    'A country may belong to multiple regions simultaneously '
    '(geographic, political, aviation authority, military alliance).';

-- =============================================================================
-- aircraft_org.organizations
-- Unified table for manufacturers, operators, design bureaus, regulatory
-- bodies, and other aviation-ecosystem entities. All aircraft, engines,
-- and related entities reference this table rather than a narrow
-- "manufacturers" table.
-- name_aliases is a TEXT[] to capture brand variants, historical names,
-- and local-language names without a separate history table.
-- =============================================================================

CREATE TABLE aircraft_org.organizations (
                                            id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                            slug             aircraft_ref.slug_text NOT NULL UNIQUE,
                                            name             TEXT     NOT NULL,
    -- Shortened or informal reference name (e.g., 'Cessna' for
    -- 'Cessna Aircraft Company'). Used in display and search.
                                            common_name      TEXT,
    -- Alternative names: historical names, brand names, local names.
    -- Array rather than a join table for read simplicity on detail pages.
                                            name_aliases     TEXT[],
                                            org_type_code    aircraft_ref.lookup_code NOT NULL
                                                REFERENCES aircraft_ref.organization_types(code),
    -- Country of primary incorporation or headquarters.
    -- NULL for international / joint bodies (e.g., early Airbus consortium).
                                            country_code     VARCHAR(3)
                                                REFERENCES aircraft_geo.countries(code) ON DELETE RESTRICT,
    -- ICAO manufacturer code (up to 4 uppercase alphanumeric chars).
    -- Unique partial index below allows NULLs.
                                            icao_mfr_code    VARCHAR(4),
                                            iata_code        VARCHAR(3),   -- IATA airline code for operator organizations
                                            founded_year     aircraft_ref.year_value,
                                            dissolved_year   aircraft_ref.year_value,  -- NULL = still operating
                                            is_active        BOOLEAN  NOT NULL DEFAULT TRUE,
                                            website_url      TEXT,
                                            description      TEXT,
    -- Sparse or source-specific attributes that don't warrant columns.
                                            extra_attributes JSONB    NOT NULL DEFAULT '{}'::jsonb,
                                            created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
                                            updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
                                            CONSTRAINT chk_org_years CHECK (
                                                dissolved_year IS NULL
                                                    OR founded_year IS NULL
                                                    OR dissolved_year >= founded_year
                                                ),
                                            CONSTRAINT chk_org_icao_fmt CHECK (
                                                icao_mfr_code IS NULL
                                                    OR icao_mfr_code ~ '^[A-Z0-9]{2,4}$'
)
    );
COMMENT ON TABLE aircraft_org.organizations IS
    'Unified organization table covering manufacturers, design bureaus, '
    'operators, and regulatory bodies. Replaces the narrow "manufacturers" '
    'table in the reference schema. All aircraft manufacturer, operator, '
    'and certification relationships FK here. '
    'Historical / dissolved organizations are retained (is_active = FALSE) '
    'because aircraft variants permanently reference their original manufacturer.';
COMMENT ON COLUMN aircraft_org.organizations.slug IS
    'URL-routing slug derived from the organization name. Stable identifier '
    'for front-end links and API routes.';
COMMENT ON COLUMN aircraft_org.organizations.name_aliases IS
    'Array of alternative, historical, or local-language names. '
    'Used by Phase 17 ingestion to resolve variant source manufacturer strings '
    'to the canonical organization row.';
COMMENT ON COLUMN aircraft_org.organizations.icao_mfr_code IS
    'ICAO aircraft manufacturer designator (2–4 uppercase alphanumeric). '
    'Unique among non-NULL values (enforced by partial index).';
COMMENT ON COLUMN aircraft_org.organizations.dissolved_year IS
    'Year the organization ceased to exist as a legal entity. '
    'NULL = still operating. is_active may be FALSE for dormant entities '
    'that have not formally dissolved.';
COMMENT ON COLUMN aircraft_org.organizations.extra_attributes IS
    'JSONB escape valve for organization-specific data not covered by '
    'dedicated columns (e.g., stock ticker, government registry number, '
    'consortium membership details).';

-- =============================================================================
-- aircraft_org.org_relationships
-- Directed M:N between organizations: from_org → to_org with a typed
-- relationship and optional date range. A UNIQUE partial index prevents
-- duplicate active relationships of the same type between the same pair.
-- =============================================================================

CREATE TABLE aircraft_org.org_relationships (
                                                id                     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                                from_org_id            BIGINT NOT NULL
                                                    REFERENCES aircraft_org.organizations(id) ON DELETE CASCADE,
                                                to_org_id              BIGINT NOT NULL
                                                    REFERENCES aircraft_org.organizations(id) ON DELETE CASCADE,
                                                relationship_type_code aircraft_ref.lookup_code NOT NULL
                                                    REFERENCES aircraft_ref.org_relationship_types(code),
                                                started_year           aircraft_ref.year_value,
                                                ended_year             aircraft_ref.year_value,
                                                notes                  TEXT,
                                                CONSTRAINT chk_org_rel_not_self CHECK (from_org_id <> to_org_id),
                                                CONSTRAINT chk_org_rel_years CHECK (
                                                    ended_year IS NULL
                                                        OR started_year IS NULL
                                                        OR ended_year >= started_year
                                                    )
);
COMMENT ON TABLE aircraft_org.org_relationships IS
    'Directed typed relationships between organizations '
    '(e.g., Cessna SUBSIDIARY_OF Textron Aviation, '
    'Grumman SUCCESSOR_ENTITY_OF Northrop Grumman). '
    'from_org is the subordinate / predecessor / licensor; '
    'to_org is the parent / successor / licensee. '
    'ended_year IS NULL means the relationship is currently active.';
COMMENT ON COLUMN aircraft_org.org_relationships.from_org_id IS
    'The originating / subordinate / predecessor organization in the relationship.';
COMMENT ON COLUMN aircraft_org.org_relationships.to_org_id IS
    'The target / parent / successor organization in the relationship.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_org_updated
    BEFORE UPDATE ON aircraft_org.organizations
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- countries — trigram for name search; covering index for continent facet
CREATE INDEX idx_countries_name_trgm
    ON aircraft_geo.countries USING gin (name gin_trgm_ops);
CREATE INDEX idx_countries_continent
    ON aircraft_geo.countries (continent)
    WHERE is_active;      -- active-country continent facet is the hot path
CREATE INDEX idx_countries_inactive
    ON aircraft_geo.countries (code)
    WHERE NOT is_active;  -- historical states: small, separate scan

-- country_regions — support both directions of the M:N
CREATE INDEX idx_cr_region
    ON aircraft_geo.country_regions (region_code);

-- organizations
CREATE UNIQUE INDEX uq_org_icao_mfr
    ON aircraft_org.organizations (icao_mfr_code)
    WHERE icao_mfr_code IS NOT NULL;

CREATE INDEX idx_org_type
    ON aircraft_org.organizations (org_type_code);
CREATE INDEX idx_org_country
    ON aircraft_org.organizations (country_code)
    WHERE country_code IS NOT NULL;
CREATE INDEX idx_org_active
    ON aircraft_org.organizations (is_active);
CREATE INDEX idx_org_name_trgm
    ON aircraft_org.organizations USING gin (name gin_trgm_ops);
CREATE INDEX idx_org_common_name_trgm
    ON aircraft_org.organizations USING gin (common_name gin_trgm_ops)
    WHERE common_name IS NOT NULL;
-- GIN on name_aliases array for ingestion alias resolution
CREATE INDEX idx_org_aliases
    ON aircraft_org.organizations USING gin (name_aliases)
    WHERE name_aliases IS NOT NULL;

-- org_relationships — both traversal directions
CREATE UNIQUE INDEX uq_org_rel_active
    ON aircraft_org.org_relationships
        (from_org_id, to_org_id, relationship_type_code)
    WHERE ended_year IS NULL;  -- only one active relationship of each type per pair

CREATE INDEX idx_org_rel_from
    ON aircraft_org.org_relationships (from_org_id);
CREATE INDEX idx_org_rel_to
    ON aircraft_org.org_relationships (to_org_id);
CREATE INDEX idx_org_rel_type
    ON aircraft_org.org_relationships (relationship_type_code);

-- =============================================================================
-- SEED DATA — countries (81 rows)
-- =============================================================================

INSERT INTO aircraft_geo.countries (code, alpha2, name, continent, is_active) VALUES

                                                                                  -- ── North America ──────────────────────────────────────────────────────────
                                                                                  ('USA', 'US', 'United States',         'NORTH_AMERICA', TRUE),
                                                                                  ('CAN', 'CA', 'Canada',                'NORTH_AMERICA', TRUE),
                                                                                  ('MEX', 'MX', 'Mexico',               'NORTH_AMERICA', TRUE),
                                                                                  ('CUB', 'CU', 'Cuba',                 'NORTH_AMERICA', TRUE),

                                                                                  -- ── Latin America ──────────────────────────────────────────────────────────
                                                                                  ('BRA', 'BR', 'Brazil',               'LATIN_AMERICA', TRUE),
                                                                                  ('ARG', 'AR', 'Argentina',            'LATIN_AMERICA', TRUE),
                                                                                  ('COL', 'CO', 'Colombia',             'LATIN_AMERICA', TRUE),
                                                                                  ('CHL', 'CL', 'Chile',               'LATIN_AMERICA', TRUE),
                                                                                  ('PER', 'PE', 'Peru',                 'LATIN_AMERICA', TRUE),
                                                                                  ('VEN', 'VE', 'Venezuela',            'LATIN_AMERICA', TRUE),
                                                                                  ('URY', 'UY', 'Uruguay',              'LATIN_AMERICA', TRUE),

                                                                                  -- ── Europe (active) ────────────────────────────────────────────────────────
                                                                                  ('GBR', 'GB', 'United Kingdom',       'EUROPE', TRUE),
                                                                                  ('FRA', 'FR', 'France',               'EUROPE', TRUE),
                                                                                  ('DEU', 'DE', 'Germany',              'EUROPE', TRUE),
                                                                                  ('ITA', 'IT', 'Italy',               'EUROPE', TRUE),
                                                                                  ('ESP', 'ES', 'Spain',               'EUROPE', TRUE),
                                                                                  ('NLD', 'NL', 'Netherlands',          'EUROPE', TRUE),
                                                                                  ('BEL', 'BE', 'Belgium',              'EUROPE', TRUE),
                                                                                  ('SWE', 'SE', 'Sweden',               'EUROPE', TRUE),
                                                                                  ('NOR', 'NO', 'Norway',               'EUROPE', TRUE),
                                                                                  ('DNK', 'DK', 'Denmark',              'EUROPE', TRUE),
                                                                                  ('FIN', 'FI', 'Finland',              'EUROPE', TRUE),
                                                                                  ('POL', 'PL', 'Poland',               'EUROPE', TRUE),
                                                                                  ('CZE', 'CZ', 'Czech Republic',       'EUROPE', TRUE),
                                                                                  ('SVK', 'SK', 'Slovakia',             'EUROPE', TRUE),
                                                                                  ('AUT', 'AT', 'Austria',              'EUROPE', TRUE),
                                                                                  ('CHE', 'CH', 'Switzerland',          'EUROPE', TRUE),
                                                                                  ('PRT', 'PT', 'Portugal',             'EUROPE', TRUE),
                                                                                  ('ROU', 'RO', 'Romania',              'EUROPE', TRUE),
                                                                                  ('HUN', 'HU', 'Hungary',              'EUROPE', TRUE),
                                                                                  ('GRC', 'GR', 'Greece',               'EUROPE', TRUE),
                                                                                  ('TUR', 'TR', 'Turkey',               'EUROPE', TRUE),
                                                                                  ('UKR', 'UA', 'Ukraine',              'EUROPE', TRUE),
                                                                                  ('BGR', 'BG', 'Bulgaria',             'EUROPE', TRUE),
                                                                                  ('SRB', 'RS', 'Serbia',               'EUROPE', TRUE),
                                                                                  ('HRV', 'HR', 'Croatia',              'EUROPE', TRUE),
                                                                                  ('SVN', 'SI', 'Slovenia',             'EUROPE', TRUE),
                                                                                  ('ISL', 'IS', 'Iceland',              'EUROPE', TRUE),
                                                                                  ('IRL', 'IE', 'Ireland',              'EUROPE', TRUE),
                                                                                  ('LTU', 'LT', 'Lithuania',            'EUROPE', TRUE),
                                                                                  ('LVA', 'LV', 'Latvia',               'EUROPE', TRUE),
                                                                                  ('EST', 'EE', 'Estonia',              'EUROPE', TRUE),
                                                                                  ('BLR', 'BY', 'Belarus',              'EUROPE', TRUE),
                                                                                  ('GEO', 'GE', 'Georgia',              'EUROPE', TRUE),

                                                                                  -- ── Europe (historical / dissolved states, is_active = FALSE) ─────────────
                                                                                  -- Soviet Union: ISO alpha-3 'SUN', alpha-2 'SU' (retired 1992).
                                                                                  -- Required: many aircraft in the seed data have "Soviet Union" as origin.
                                                                                  ('SUN', 'SU', 'Soviet Union',          'EUROPE', FALSE),
                                                                                  -- East Germany: dissolved 1990, reunified into DEU.
                                                                                  ('DDR', 'DD', 'East Germany',          'EUROPE', FALSE),
                                                                                  -- Czechoslovakia: split 1993 into CZE and SVK.
                                                                                  ('CSK', 'CS', 'Czechoslovakia',        'EUROPE', FALSE),
                                                                                  -- Yugoslavia: dissolved 1991–2006 into several successor states.
                                                                                  ('YUG', 'YU', 'Yugoslavia',            'EUROPE', FALSE),

                                                                                  -- ── Middle East ────────────────────────────────────────────────────────────
                                                                                  ('ISR', 'IL', 'Israel',               'MIDDLE_EAST', TRUE),
                                                                                  ('SAU', 'SA', 'Saudi Arabia',          'MIDDLE_EAST', TRUE),
                                                                                  ('ARE', 'AE', 'United Arab Emirates',  'MIDDLE_EAST', TRUE),
                                                                                  ('IRN', 'IR', 'Iran',                 'MIDDLE_EAST', TRUE),
                                                                                  ('IRQ', 'IQ', 'Iraq',                 'MIDDLE_EAST', TRUE),
                                                                                  ('EGY', 'EG', 'Egypt',               'MIDDLE_EAST', TRUE),
                                                                                  ('JOR', 'JO', 'Jordan',               'MIDDLE_EAST', TRUE),
                                                                                  ('KWT', 'KW', 'Kuwait',               'MIDDLE_EAST', TRUE),
                                                                                  ('OMN', 'OM', 'Oman',                 'MIDDLE_EAST', TRUE),

                                                                                  -- ── Asia: East ─────────────────────────────────────────────────────────────
                                                                                  ('CHN', 'CN', 'China',               'ASIA_EAST', TRUE),
                                                                                  ('JPN', 'JP', 'Japan',               'ASIA_EAST', TRUE),
                                                                                  ('KOR', 'KR', 'South Korea',          'ASIA_EAST', TRUE),
                                                                                  ('PRK', 'KP', 'North Korea',          'ASIA_EAST', TRUE),
                                                                                  ('TWN', 'TW', 'Taiwan',               'ASIA_EAST', TRUE),

                                                                                  -- ── Asia: South ────────────────────────────────────────────────────────────
                                                                                  ('IND', 'IN', 'India',               'ASIA_SOUTH', TRUE),
                                                                                  ('PAK', 'PK', 'Pakistan',             'ASIA_SOUTH', TRUE),
                                                                                  ('BGD', 'BD', 'Bangladesh',           'ASIA_SOUTH', TRUE),
                                                                                  ('LKA', 'LK', 'Sri Lanka',            'ASIA_SOUTH', TRUE),

                                                                                  -- ── Asia: Southeast ────────────────────────────────────────────────────────
                                                                                  ('SGP', 'SG', 'Singapore',            'ASIA_SOUTHEAST', TRUE),
                                                                                  ('IDN', 'ID', 'Indonesia',            'ASIA_SOUTHEAST', TRUE),
                                                                                  ('THA', 'TH', 'Thailand',             'ASIA_SOUTHEAST', TRUE),
                                                                                  ('MYS', 'MY', 'Malaysia',             'ASIA_SOUTHEAST', TRUE),
                                                                                  ('PHL', 'PH', 'Philippines',          'ASIA_SOUTHEAST', TRUE),
                                                                                  ('VNM', 'VN', 'Vietnam',              'ASIA_SOUTHEAST', TRUE),

                                                                                  -- ── Central Asia (includes South Caucasus for this schema) ─────────────────
                                                                                  ('KAZ', 'KZ', 'Kazakhstan',           'CENTRAL_ASIA', TRUE),
                                                                                  ('UZB', 'UZ', 'Uzbekistan',           'CENTRAL_ASIA', TRUE),
                                                                                  ('AZE', 'AZ', 'Azerbaijan',           'CENTRAL_ASIA', TRUE),

                                                                                  -- ── Oceania ────────────────────────────────────────────────────────────────
                                                                                  ('AUS', 'AU', 'Australia',            'OCEANIA', TRUE),
                                                                                  ('NZL', 'NZ', 'New Zealand',          'OCEANIA', TRUE),

                                                                                  -- ── Africa ─────────────────────────────────────────────────────────────────
                                                                                  ('ZAF', 'ZA', 'South Africa',         'AFRICA', TRUE),
                                                                                  ('KEN', 'KE', 'Kenya',               'AFRICA', TRUE),
                                                                                  ('NGA', 'NG', 'Nigeria',              'AFRICA', TRUE),
                                                                                  ('MAR', 'MA', 'Morocco',              'AFRICA', TRUE),
                                                                                  ('ETH', 'ET', 'Ethiopia',             'AFRICA', TRUE),
                                                                                  ('TUN', 'TN', 'Tunisia',              'AFRICA', TRUE),
                                                                                  ('DZA', 'DZ', 'Algeria',              'AFRICA', TRUE)

    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- SEED DATA — regions (11 rows)
-- =============================================================================

INSERT INTO aircraft_geo.regions (code, label, description, region_type, sort_order) VALUES

                                                                                         -- Geographic
                                                                                         ('NORTH_AMERICA',   'North America',
                                                                                          'Canada, United States, Mexico, Caribbean.',      'GEOGRAPHIC', 10),
                                                                                         ('LATIN_AMERICA',   'Latin America',
                                                                                          'Central and South America.',                     'GEOGRAPHIC', 20),
                                                                                         ('EUROPE',          'Europe',
                                                                                          'Greater Europe including Turkey and Caucasus.',  'GEOGRAPHIC', 30),
                                                                                         ('MIDDLE_EAST',     'Middle East',
                                                                                          'Middle East and North Africa (MENA).',           'GEOGRAPHIC', 40),
                                                                                         ('ASIA_PACIFIC',    'Asia-Pacific',
                                                                                          'East, South, Southeast Asia, and Oceania.',      'GEOGRAPHIC', 50),
                                                                                         ('AFRICA',          'Africa',
                                                                                          'Sub-Saharan Africa.',                            'GEOGRAPHIC', 60),

                                                                                         -- Political
                                                                                         ('EU',              'European Union',
                                                                                          'Current EU member states.',                      'POLITICAL',  70),
                                                                                         ('CIS',             'Commonwealth of Independent States',
                                                                                          'Former Soviet republics (excl. Baltic states).', 'POLITICAL',  80),

                                                                                         -- Aviation authority
                                                                                         ('EASA_STATES',     'EASA Member States',
                                                                                          'Countries under EASA regulatory jurisdiction '
                                                                                              '(EU + Iceland, Norway, Switzerland + UK arrangements).', 'AVIATION', 90),
                                                                                         ('FAA_BILATERAL',   'FAA Bilateral Agreement States',
                                                                                          'Countries with FAA bilateral aviation safety agreements.','AVIATION', 91),

                                                                                         -- Military alliance
                                                                                         ('NATO',            'NATO',
                                                                                          'North Atlantic Treaty Organisation member states.','MILITARY', 100)

    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- SEED DATA — country_regions
-- Organized by region for readability.
-- =============================================================================

-- Geographic: NORTH_AMERICA
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('USA','NORTH_AMERICA'),('CAN','NORTH_AMERICA'),
                                                                         ('MEX','NORTH_AMERICA'),('CUB','NORTH_AMERICA')
    ON CONFLICT DO NOTHING;

-- Geographic: LATIN_AMERICA
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('BRA','LATIN_AMERICA'),('ARG','LATIN_AMERICA'),('COL','LATIN_AMERICA'),
                                                                         ('CHL','LATIN_AMERICA'),('PER','LATIN_AMERICA'),('VEN','LATIN_AMERICA'),
                                                                         ('URY','LATIN_AMERICA')
    ON CONFLICT DO NOTHING;

-- Geographic: EUROPE (active + historical)
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('GBR','EUROPE'),('FRA','EUROPE'),('DEU','EUROPE'),('ITA','EUROPE'),
                                                                         ('ESP','EUROPE'),('NLD','EUROPE'),('BEL','EUROPE'),('SWE','EUROPE'),
                                                                         ('NOR','EUROPE'),('DNK','EUROPE'),('FIN','EUROPE'),('POL','EUROPE'),
                                                                         ('CZE','EUROPE'),('SVK','EUROPE'),('AUT','EUROPE'),('CHE','EUROPE'),
                                                                         ('PRT','EUROPE'),('ROU','EUROPE'),('HUN','EUROPE'),('GRC','EUROPE'),
                                                                         ('TUR','EUROPE'),('UKR','EUROPE'),('BGR','EUROPE'),('SRB','EUROPE'),
                                                                         ('HRV','EUROPE'),('SVN','EUROPE'),('ISL','EUROPE'),('IRL','EUROPE'),
                                                                         ('LTU','EUROPE'),('LVA','EUROPE'),('EST','EUROPE'),('BLR','EUROPE'),
                                                                         ('GEO','EUROPE'),
                                                                         -- Historical European states
                                                                         ('SUN','EUROPE'),('DDR','EUROPE'),('CSK','EUROPE'),('YUG','EUROPE')
    ON CONFLICT DO NOTHING;

-- Geographic: MIDDLE_EAST
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('ISR','MIDDLE_EAST'),('SAU','MIDDLE_EAST'),('ARE','MIDDLE_EAST'),
                                                                         ('IRN','MIDDLE_EAST'),('IRQ','MIDDLE_EAST'),('EGY','MIDDLE_EAST'),
                                                                         ('JOR','MIDDLE_EAST'),('KWT','MIDDLE_EAST'),('OMN','MIDDLE_EAST')
    ON CONFLICT DO NOTHING;

-- Geographic: ASIA_PACIFIC
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('CHN','ASIA_PACIFIC'),('JPN','ASIA_PACIFIC'),('KOR','ASIA_PACIFIC'),
                                                                         ('PRK','ASIA_PACIFIC'),('TWN','ASIA_PACIFIC'),
                                                                         ('IND','ASIA_PACIFIC'),('PAK','ASIA_PACIFIC'),('BGD','ASIA_PACIFIC'),
                                                                         ('LKA','ASIA_PACIFIC'),
                                                                         ('SGP','ASIA_PACIFIC'),('IDN','ASIA_PACIFIC'),('THA','ASIA_PACIFIC'),
                                                                         ('MYS','ASIA_PACIFIC'),('PHL','ASIA_PACIFIC'),('VNM','ASIA_PACIFIC'),
                                                                         ('KAZ','ASIA_PACIFIC'),('UZB','ASIA_PACIFIC'),('AZE','ASIA_PACIFIC'),
                                                                         ('AUS','ASIA_PACIFIC'),('NZL','ASIA_PACIFIC')
    ON CONFLICT DO NOTHING;

-- Geographic: AFRICA
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('ZAF','AFRICA'),('KEN','AFRICA'),('NGA','AFRICA'),
                                                                         ('MAR','AFRICA'),('ETH','AFRICA'),('TUN','AFRICA'),('DZA','AFRICA'),
                                                                         ('EGY','AFRICA')   -- Egypt also mapped to AFRICA (already MIDDLE_EAST above)
    ON CONFLICT DO NOTHING;

-- Political: EU (27 current members as of 2024)
INSERT INTO aircraft_geo.country_regions (country_code, region_code, joined_year) VALUES
                                                                                      ('DEU','EU',1958),('FRA','EU',1958),('ITA','EU',1958),
                                                                                      ('NLD','EU',1958),('BEL','EU',1958),('LUX','EU',1958),  -- LUX not in countries seed; insert ignored
                                                                                      ('DNK','EU',1973),('IRL','EU',1973),
                                                                                      ('GRC','EU',1981),
                                                                                      ('ESP','EU',1986),('PRT','EU',1986),
                                                                                      ('AUT','EU',1995),('FIN','EU',1995),('SWE','EU',1995),
                                                                                      ('CZE','EU',2004),('SVK','EU',2004),('HUN','EU',2004),('POL','EU',2004),
                                                                                      ('SVN','EU',2004),('EST','EU',2004),('LVA','EU',2004),('LTU','EU',2004),
                                                                                      ('BGR','EU',2007),('ROU','EU',2007),
                                                                                      ('HRV','EU',2013)
    ON CONFLICT DO NOTHING;

-- Political: CIS (Commonwealth of Independent States)
INSERT INTO aircraft_geo.country_regions (country_code, region_code, joined_year) VALUES
                                                                                      ('RUS','CIS',1991),  -- RUS not in countries seed; insert ignored
                                                                                      ('UKR','CIS',1991),
                                                                                      ('BLR','CIS',1991),
                                                                                      ('KAZ','CIS',1991),
                                                                                      ('UZB','CIS',1991),
                                                                                      ('AZE','CIS',1991),
                                                                                      ('GEO','CIS',1993)
    ON CONFLICT DO NOTHING;

-- Aviation: EASA_STATES
-- EU27 + Norway, Iceland, Switzerland + UK (bilateral arrangement post-Brexit)
INSERT INTO aircraft_geo.country_regions (country_code, region_code) VALUES
                                                                         ('DEU','EASA_STATES'),('FRA','EASA_STATES'),('ITA','EASA_STATES'),
                                                                         ('ESP','EASA_STATES'),('NLD','EASA_STATES'),('BEL','EASA_STATES'),
                                                                         ('SWE','EASA_STATES'),('DNK','EASA_STATES'),('FIN','EASA_STATES'),
                                                                         ('POL','EASA_STATES'),('CZE','EASA_STATES'),('SVK','EASA_STATES'),
                                                                         ('AUT','EASA_STATES'),('PRT','EASA_STATES'),('ROU','EASA_STATES'),
                                                                         ('HUN','EASA_STATES'),('GRC','EASA_STATES'),('IRL','EASA_STATES'),
                                                                         ('HRV','EASA_STATES'),('SVN','EASA_STATES'),
                                                                         ('BGR','EASA_STATES'),('EST','EASA_STATES'),('LVA','EASA_STATES'),('LTU','EASA_STATES'),
                                                                         ('NOR','EASA_STATES'),('ISL','EASA_STATES'),('CHE','EASA_STATES'),
                                                                         ('GBR','EASA_STATES')   -- UK maintains bilateral arrangements
    ON CONFLICT DO NOTHING;

-- Military: NATO (current 32 members as of 2024 from countries in this seed)
INSERT INTO aircraft_geo.country_regions (country_code, region_code, joined_year) VALUES
                                                                                      ('USA','NATO',1949),('CAN','NATO',1949),('GBR','NATO',1949),
                                                                                      ('FRA','NATO',1949),('BEL','NATO',1949),('NLD','NATO',1949),
                                                                                      ('NOR','NATO',1949),('DNK','NATO',1949),('ISL','NATO',1949),
                                                                                      ('ITA','NATO',1949),('PRT','NATO',1949),
                                                                                      ('GRC','NATO',1952),('TUR','NATO',1952),
                                                                                      ('DEU','NATO',1955),
                                                                                      ('ESP','NATO',1982),
                                                                                      ('HUN','NATO',1999),('POL','NATO',1999),('CZE','NATO',1999),
                                                                                      ('BGR','NATO',2004),('ROU','NATO',2004),('SVK','NATO',2004),
                                                                                      ('SVN','NATO',2004),('EST','NATO',2004),('LVA','NATO',2004),('LTU','NATO',2004),
                                                                                      ('HRV','NATO',2009),
                                                                                      ('FIN','NATO',2023),
                                                                                      ('SWE','NATO',2024)
    ON CONFLICT DO NOTHING;

-- =============================================================================
-- SEED DATA — organizations (27 rows)
-- These are anchor manufacturer records. Phase 17 ingestion will INSERT
-- additional organizations derived from the PlanePHD manufacturer keys,
-- matching against name_aliases before creating new rows.
-- =============================================================================

INSERT INTO aircraft_org.organizations
(slug, name, common_name, name_aliases,
 org_type_code, country_code,
 founded_year, dissolved_year, is_active,
 description)
VALUES

    -- ── United States manufacturers ────────────────────────────────────────────

    ('aeronca-aircraft',
     'Aeronca Aircraft Corporation', 'Aeronca',
     ARRAY['Aeronca','Aeronautical Corporation of America'],
     'MANUFACTURER', 'USA', 1928, 1954, FALSE,
     'Pioneer US light aircraft maker; Champion and Champ series. '
         'Type certificates transferred to Champion Aircraft in 1954.'),

    ('american-champion-aircraft',
     'American Champion Aircraft', 'American Champion',
     ARRAY['American Champion','Champion Aircraft','Bellanca Champion'],
     'MANUFACTURER', 'USA', 1954, NULL, TRUE,
     'Successor to Aeronca; produces the Citabria, Decathlon, and Scout series.'),

    ('cessna-aircraft-company',
     'Cessna Aircraft Company', 'Cessna',
     ARRAY['Cessna','Cessna Aircraft'],
     'MANUFACTURER', 'USA', 1927, NULL, TRUE,
     'Wichita-based manufacturer of the Skyhawk, Skylane, Citation, and Caravan series. '
         'Acquired by Textron in 1992; brand retained under Textron Aviation.'),

    ('beech-aircraft-corporation',
     'Beech Aircraft Corporation', 'Beechcraft',
     ARRAY['Beechcraft','Beech Aircraft','Hawker Beechcraft','Beechcraft Corporation'],
     'MANUFACTURER', 'USA', 1932, 1994, FALSE,
     'Wichita manufacturer of Bonanza, Baron, King Air, and Starship series. '
         'Acquired by Raytheon 1994; later Hawker Beechcraft (2006); '
         'Textron acquired 2014.'),

    ('textron-aviation',
     'Textron Aviation', 'Textron Aviation',
     ARRAY['Textron Aviation'],
     'MANUFACTURER', 'USA', 2014, NULL, TRUE,
     'Textron subsidiary formed 2014, consolidating Cessna and Beechcraft brands.'),

    ('piper-aircraft',
     'Piper Aircraft', 'Piper',
     ARRAY['Piper','Piper Aircraft Corporation','New Piper Aircraft'],
     'MANUFACTURER', 'USA', 1937, NULL, TRUE,
     'Lock Haven / Vero Beach manufacturer of Cherokee, Warrior, Archer, '
         'Seneca, Meridian, and M-class series.'),

    ('cirrus-aircraft',
     'Cirrus Aircraft', 'Cirrus',
     ARRAY['Cirrus','Cirrus Design Corporation'],
     'MANUFACTURER', 'USA', 1984, NULL, TRUE,
     'Duluth/Knoxville manufacturer; SR20, SR22, SR22T, SF50 Vision Jet. '
         'Known for standard-equipment BRS parachute system.'),

    ('mooney-aviation',
     'Mooney Aviation Company', 'Mooney',
     ARRAY['Mooney','Mooney Aircraft Corporation'],
     'MANUFACTURER', 'USA', 1929, NULL, TRUE,
     'Kerrville TX manufacturer; M20 series (Ovation, Acclaim, Acclaim Ultra). '
         'Known for efficient high-performance singles.'),

    ('boeing-company',
     'The Boeing Company', 'Boeing',
     ARRAY['Boeing','Boeing Commercial Airplanes','Boeing Defense'],
     'MANUFACTURER', 'USA', 1916, NULL, TRUE,
     'Major US aerospace manufacturer; commercial jetliners (737, 747, 777, 787), '
         'defense, and space systems.'),

    ('north-american-aviation',
     'North American Aviation', 'North American',
     ARRAY['North American','North American Aviation','NAA'],
     'MANUFACTURER', 'USA', 1934, 1967, FALSE,
     'Designer of P-51 Mustang, B-25 Mitchell, T-6 Texan, F-86 Sabre, F-100 Super Sabre. '
         'Merged into North American Rockwell 1967; later Rockwell International; '
         'Boeing acquired Rockwell aerospace divisions 1996.'),

    ('grumman-aerospace',
     'Grumman Aerospace Corporation', 'Grumman',
     ARRAY['Grumman','Grumman Aircraft Engineering','Grumman Aerospace'],
     'MANUFACTURER', 'USA', 1930, 1994, FALSE,
     'Bethpage NY manufacturer of F6F Hellcat, F-14 Tomcat, E-2 Hawkeye, '
         'EA-6B Prowler, A-6 Intruder. Merged with Northrop 1994.'),

    ('northrop-grumman',
     'Northrop Grumman Corporation', 'Northrop Grumman',
     ARRAY['Northrop Grumman','NGC'],
     'MANUFACTURER', 'USA', 1994, NULL, TRUE,
     'Formed from merger of Northrop Corporation and Grumman 1994. '
         'Produces B-21, B-2, E-2D, RQ-4 Global Hawk, and MQ-8 Fire Scout.'),

    ('lockheed-corporation',
     'Lockheed Corporation', 'Lockheed',
     ARRAY['Lockheed','Lockheed Aircraft Corporation'],
     'MANUFACTURER', 'USA', 1926, 1995, FALSE,
     'Burbank CA manufacturer of P-38 Lightning, F-80 Shooting Star, '
         'C-130 Hercules, SR-71 Blackbird, F-104 Starfighter, U-2. '
         'Merged with Martin Marietta 1995 to form Lockheed Martin.'),

    ('lockheed-martin',
     'Lockheed Martin Corporation', 'Lockheed Martin',
     ARRAY['Lockheed Martin','LMT'],
     'MANUFACTURER', 'USA', 1995, NULL, TRUE,
     'Formed 1995 from Lockheed Corporation and Martin Marietta. '
         'Produces F-22, F-35, C-130J, P-8 Poseidon, and space systems.'),

    ('republic-aviation',
     'Republic Aviation Corporation', 'Republic',
     ARRAY['Republic Aviation','Republic'],
     'MANUFACTURER', 'USA', 1939, 1965, FALSE,
     'Farmingdale NY manufacturer of P-47 Thunderbolt and F-105 Thunderchief. '
         'Merged into Fairchild Hiller 1965.'),

    ('general-dynamics',
     'General Dynamics', 'General Dynamics',
     ARRAY['General Dynamics','GD','Convair'],
     'MANUFACTURER', 'USA', 1952, NULL, TRUE,
     'Manufacturer of F-111 Aardvark and F-16 Fighting Falcon (before F-16 '
         'line sold to Lockheed 1993). Current products include submarines and armored vehicles.'),

    -- ── Diamond Aircraft (Austria) ─────────────────────────────────────────────

    ('diamond-aircraft-industries',
     'Diamond Aircraft Industries', 'Diamond',
     ARRAY['Diamond','Diamond Aircraft'],
     'MANUFACTURER', 'AUT', 1981, NULL, TRUE,
     'Wiener Neustadt manufacturer of DA20, DA40, DA42, DA62, and DA50 series. '
         'Also produces military variants. Acquired by Chinese investor group 2017.'),

    -- ── European manufacturers ─────────────────────────────────────────────────

    ('aero-vodochody',
     'Aero Vodochody', 'Aero',
     ARRAY['Aero','Aero Vodochody','AERO Vodochody','Avia'],
     'MANUFACTURER', 'CZE', 1919, NULL, TRUE,
     'Vodochody, Czech Republic. Produces L-39 Albatros, L-159 ALCA, '
         'and L-39NG advanced jet trainers.'),

    ('airbus-se',
     'Airbus SE', 'Airbus',
     ARRAY['Airbus','Airbus S.A.S.','EADS'],
     'MANUFACTURER', 'NLD', 1970, NULL, TRUE,
     'Pan-European aerospace group (headquarters Leiden, NL; production in FR/DE/ES/UK). '
         'Produces A220, A320, A330, A350, A380, and military transports.'),

    ('dassault-aviation',
     'Dassault Aviation', 'Dassault',
     ARRAY['Dassault','Avions Marcel Dassault','AMD'],
     'MANUFACTURER', 'FRA', 1929, NULL, TRUE,
     'Saint-Cloud manufacturer; Mirage, Rafale fighter families '
         'and Falcon business jet series.'),

    ('focke-wulf',
     'Focke-Wulf Flugzeugbau GmbH', 'Focke-Wulf',
     ARRAY['Focke-Wulf','Focke-Wulf Flugzeugbau'],
     'MANUFACTURER', 'DEU', 1923, 1964, FALSE,
     'Bremen manufacturer; Fw 190 Würger and Ta 152 fighter series.'),

    ('messerschmitt-ag',
     'Messerschmitt AG', 'Messerschmitt',
     ARRAY['Messerschmitt','BFW','Bayerische Flugzeugwerke'],
     'MANUFACTURER', 'DEU', 1938, 1968, FALSE,
     'Augsburg manufacturer; Bf 109, Me 262, Me 163 Komet. '
         'Predecessor Bayerische Flugzeugwerke founded 1916.'),

    -- ── Soviet / Russian design bureaus ────────────────────────────────────────

    ('sukhoi',
     'Sukhoi Design Bureau', 'Sukhoi',
     ARRAY['Sukhoi','OKB Sukhoi','JSC Sukhoi','UAC'],
     'DESIGN_BUREAU', 'RUS', 1939, NULL, TRUE,
     'Moscow design bureau; Su-27, Su-30, Su-35, Su-57 fighter families '
         'and Su-25 Frogfoot ground attack.'),

    ('mikoyan',
     'Mikoyan Design Bureau', 'Mikoyan',
     ARRAY['Mikoyan','MiG','Mikoyan-Gurevich','RSK MiG'],
     'DESIGN_BUREAU', 'RUS', 1939, NULL, TRUE,
     'Moscow design bureau; MiG-15 through MiG-35 series.'),

    -- ── Canada / Brazil ────────────────────────────────────────────────────────

    ('bombardier-aerospace',
     'Bombardier Aerospace', 'Bombardier',
     ARRAY['Bombardier','Canadair','de Havilland Canada','Shorts'],
     'MANUFACTURER', 'CAN', 1942, NULL, TRUE,
     'Montreal-based; Challenger, Global, and Learjet business jets; '
         'CRJ regional jets; Q-Series turboprops.'),

    ('embraer',
     'Embraer S.A.', 'Embraer',
     ARRAY['Embraer','EMBRAER'],
     'MANUFACTURER', 'BRA', 1969, NULL, TRUE,
     'São José dos Campos; ERJ, E-Jet, E2 regional jets; '
         'Phenom, Praetor business jets; KC-390 military transport.')

    ON CONFLICT (slug) DO NOTHING;

-- =============================================================================
-- SEED DATA — org_relationships (6 rows)
-- Uses subselects to avoid hardcoded IDs.
-- =============================================================================

-- Aeronca → SUCCESSOR_ENTITY → American Champion (1954)
INSERT INTO aircraft_org.org_relationships
(from_org_id, to_org_id, relationship_type_code, started_year, notes)
SELECT f.id, t.id, 'SUCCESSOR_ENTITY', 1954,
       'Aeronca type certificates and Champion line transferred to American Champion Aircraft 1954.'
FROM aircraft_org.organizations f
         JOIN aircraft_org.organizations t ON TRUE
WHERE f.slug = 'aeronca-aircraft'
  AND t.slug = 'american-champion-aircraft';

-- North American Aviation → SUCCESSOR_ENTITY → Boeing (via Rockwell; 1996)
INSERT INTO aircraft_org.org_relationships
(from_org_id, to_org_id, relationship_type_code, started_year, notes)
SELECT f.id, t.id, 'SUCCESSOR_ENTITY', 1996,
       'North American merged into Rockwell International 1967; '
           'Boeing acquired Rockwell aerospace divisions 1996.'
FROM aircraft_org.organizations f
         JOIN aircraft_org.organizations t ON TRUE
WHERE f.slug = 'north-american-aviation'
  AND t.slug = 'boeing-company';

-- Grumman → SUCCESSOR_ENTITY → Northrop Grumman (1994)
INSERT INTO aircraft_org.org_relationships
(from_org_id, to_org_id, relationship_type_code, started_year, notes)
SELECT f.id, t.id, 'SUCCESSOR_ENTITY', 1994,
       'Grumman merged with Northrop Corporation to form Northrop Grumman 1994.'
FROM aircraft_org.organizations f
         JOIN aircraft_org.organizations t ON TRUE
WHERE f.slug = 'grumman-aerospace'
  AND t.slug = 'northrop-grumman';

-- Lockheed → SUCCESSOR_ENTITY → Lockheed Martin (1995)
INSERT INTO aircraft_org.org_relationships
(from_org_id, to_org_id, relationship_type_code, started_year, notes)
SELECT f.id, t.id, 'SUCCESSOR_ENTITY', 1995,
       'Lockheed Corporation merged with Martin Marietta to form Lockheed Martin 1995.'
FROM aircraft_org.organizations f
         JOIN aircraft_org.organizations t ON TRUE
WHERE f.slug = 'lockheed-corporation'
  AND t.slug = 'lockheed-martin';

-- Cessna → SUBSIDIARY → Textron Aviation (brand consolidated 2014)
INSERT INTO aircraft_org.org_relationships
(from_org_id, to_org_id, relationship_type_code, started_year, notes)
SELECT f.id, t.id, 'SUBSIDIARY', 2014,
       'Cessna brand consolidated under Textron Aviation 2014; '
           'Cessna Aircraft Company itself acquired by Textron 1992.'
FROM aircraft_org.organizations f
         JOIN aircraft_org.organizations t ON TRUE
WHERE f.slug = 'cessna-aircraft-company'
  AND t.slug = 'textron-aviation';

-- Beechcraft → SUBSIDIARY → Textron Aviation (2014)
INSERT INTO aircraft_org.org_relationships
(from_org_id, to_org_id, relationship_type_code, started_year, notes)
SELECT f.id, t.id, 'SUBSIDIARY', 2014,
       'Beechcraft brand consolidated under Textron Aviation 2014, '
           'following Textron acquisition of Beechcraft Corporation.'
FROM aircraft_org.organizations f
         JOIN aircraft_org.organizations t ON TRUE
WHERE f.slug = 'beech-aircraft-corporation'
  AND t.slug = 'textron-aviation';

COMMIT;