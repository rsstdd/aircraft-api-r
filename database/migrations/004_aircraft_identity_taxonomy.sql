-- =============================================================================
-- File: database/migrations/004_aircraft_identity_taxonomy.sql
-- Phase 4 — aircraft_core: the identity backbone of the entire database.
--
-- Hierarchy:  families (1) ──── (N) models (1) ──── (N) variants
--
-- aircraft_core.variants is the ATOMIC UNIT to which all domain-specific
-- tables (specs, power, systems, military, market, maintenance, provenance)
-- attach via FK.  Every table in Phases 5–14 references variants.id.
--
-- Junction tables in this phase:
--   variant_aliases       — alternate designations (NATO names, mil codes)
--   variant_roles         — M:N with aircraft_ref.aircraft_roles
--   variant_manufacturers — M:N with aircraft_org.organizations
--   variant_operators     — M:N with countries + optional operator org
--
-- No aircraft data rows are seeded here; data arrives via Phase 17 ingestion.
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_core.families
-- Broadest identity grouping: "Cessna 172", "F-16 Fighting Falcon",
-- "Boeing 737". One family can span many model generations.
-- manufacturer_org_id captures the original/primary designer; the full M:N
-- production history is in variant_manufacturers.
-- =============================================================================

CREATE TABLE aircraft_core.families (
                                        id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                        slug                  aircraft_ref.slug_text NOT NULL UNIQUE,
                                        name                  TEXT NOT NULL,     -- "Cessna 172", "P-51 Mustang"
                                        common_name           TEXT,              -- shorter informal name if different
    -- Array of alternate names, former names, or local-language designations.
    -- GIN-indexed for alias resolution during ingestion.
                                        name_aliases          TEXT[],
    -- Original designer / primary manufacturer organization.
    -- Variant-level manufacture (joint ventures, license production)
    -- is tracked in variant_manufacturers.
                                        manufacturer_org_id   BIGINT
                                                                                     REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
                                        country_of_origin_code VARCHAR(3)
                                            REFERENCES aircraft_geo.countries(code)  ON DELETE RESTRICT,
                                        first_flight_year     aircraft_ref.year_value,
                                        description           TEXT,
    -- Generated tsvector for full-text search on family name + aliases.
                                        name_tsv              tsvector GENERATED ALWAYS AS (
                                            to_tsvector('english',
                                                        coalesce(name, '') || ' ' ||
                                                        coalesce(common_name, '') || ' ' ||
                                                        array_to_string(coalesce(name_aliases, ARRAY[]::text[]), ' ')
                                            )
                                            ) STORED,
                                        extra_attributes      JSONB        NOT NULL DEFAULT '{}'::jsonb,
                                        created_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),
                                        updated_at            TIMESTAMPTZ  NOT NULL DEFAULT now()
);
COMMENT ON TABLE aircraft_core.families IS
    'Broadest aircraft identity grouping (e.g., "Cessna 172", "F-16 Fighting Falcon"). '
    'One family contains one or more models. manufacturer_org_id captures the original designer; '
    'all production parties are tracked in variant_manufacturers.';
COMMENT ON COLUMN aircraft_core.families.name_aliases IS
    'Alternative, former, or local-language family names. GIN-indexed for alias '
    'resolution during Phase 17 ingestion (manufacturer string matching).';
COMMENT ON COLUMN aircraft_core.families.name_tsv IS
    'Generated stored tsvector over name + common_name + name_aliases. '
    'Backed by a GIN index for full-text family search in Phase 16.';

-- =============================================================================
-- aircraft_core.models
-- A specific model designation within a family (e.g., "Cessna 172S",
-- "F-16C Block 52", "Boeing 737-800"). Each model belongs to exactly one
-- family and may have many production variants.
-- =============================================================================

CREATE TABLE aircraft_core.models (
                                      id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                      family_id            BIGINT NOT NULL
                                          REFERENCES aircraft_core.families(id) ON DELETE RESTRICT,
                                      slug                 aircraft_ref.slug_text NOT NULL UNIQUE,
                                      name                 TEXT NOT NULL,      -- "172S", "737-800", "F-16C"
    -- Full model name including family prefix when useful.
                                      display_name         TEXT,              -- "Cessna 172S Skyhawk SP"
                                      name_aliases         TEXT[],
    -- Series / generation within the family (textual label, e.g. "NG", "Classic").
                                      series               TEXT,
    -- Numeric generation counter within the family (1, 2, 3 …).
                                      generation           SMALLINT,
                                      first_flight_year    aircraft_ref.year_value,
                                      certification_year   aircraft_ref.year_value,
                                      description          TEXT,
                                      extra_attributes     JSONB        NOT NULL DEFAULT '{}'::jsonb,
                                      created_at           TIMESTAMPTZ  NOT NULL DEFAULT now(),
                                      updated_at           TIMESTAMPTZ  NOT NULL DEFAULT now(),
                                      CONSTRAINT chk_model_generation CHECK (generation IS NULL OR generation > 0)
);
COMMENT ON TABLE aircraft_core.models IS
    'Specific model designation within a family (e.g., "Cessna 172S", "737-800"). '
    'Models sit between families (broad grouping) and variants (atomic spec unit).';
COMMENT ON COLUMN aircraft_core.models.display_name IS
    'Human-readable full model name for display (e.g., "Cessna 172S Skyhawk SP"). '
    'name holds the bare designation; display_name adds family prefix and popular name.';

-- =============================================================================
-- aircraft_core.variants
-- The ATOMIC UNIT of the aircraft identity hierarchy.
-- All domain tables (specs, power, systems, military, market, maintenance,
-- provenance) reference variants.id.
--
-- Quick-filter columns (passenger_capacity, engine_count, propulsion_category,
-- landing_gear_type) are denormalized here to avoid joins for common facets.
-- The canonical/authoritative values for these fields still live in the
-- aircraft_specs, aircraft_power, and aircraft_cert tables created in later
-- phases; these columns serve the read-model and search use cases.
--
-- ingest_key: opaque deduplication key used by Phase 17 ingestion
--   (e.g., "planephd:123456"). Superseded by aircraft_prov.source_assertions
--   once Phase 14 data is populated; retained here for ingestion-phase
--   compatibility.
-- source_path: URI path from the originating source system (e.g., PlanePHD
--   /wizard/details/…). Staging convenience; canonical URL stored in
--   aircraft_prov.source_documents.source_url after Phase 14.
-- =============================================================================

CREATE TABLE aircraft_core.variants (
                                        id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                        model_id                 BIGINT NOT NULL
                                            REFERENCES aircraft_core.models(id) ON DELETE RESTRICT,
                                        slug                     aircraft_ref.slug_text NOT NULL UNIQUE,
                                        name                     TEXT NOT NULL,   -- variant designation, e.g. "172S-G1000"
                                        popular_name             TEXT,            -- informal name, e.g. "Skyhawk SP"
                                        variant_type_code        aircraft_ref.lookup_code
                                            REFERENCES aircraft_ref.variant_types(code),
                                        service_status_code      aircraft_ref.lookup_code
                                            REFERENCES aircraft_ref.service_statuses(code),

    -- ── Origin ──────────────────────────────────────────────────────────────
                                        country_of_origin_code   VARCHAR(3)
                                            REFERENCES aircraft_geo.countries(code) ON DELETE RESTRICT,

    -- ── Dates ────────────────────────────────────────────────────────────────
                                        first_flight_year        aircraft_ref.year_value,
                                        certification_year       aircraft_ref.year_value,
                                        production_start_year    aircraft_ref.year_value,
                                        production_end_year      aircraft_ref.year_value,   -- NULL = still in production

    -- ── Quick-filter denormalized fields ────────────────────────────────────
    -- Authoritative values live in aircraft_specs / aircraft_cert (Phase 5+).
    -- Populated during ingestion; updated by curation workflow.
                                        passenger_capacity       SMALLINT,          -- total passenger seats
                                        crew_count               SMALLINT,          -- minimum certificated crew
                                        landing_gear_type_code   aircraft_ref.lookup_code
                                            REFERENCES aircraft_ref.landing_gear_types(code),
                                        propulsion_category_code aircraft_ref.lookup_code
                                            REFERENCES aircraft_ref.propulsion_categories(code),
                                        engine_count             SMALLINT,          -- number of engines/motors installed
                                        is_in_production         BOOLEAN,           -- NULL = unknown

    -- ── Ingestion staging fields (see note above) ────────────────────────────
    -- Unique among non-NULL values; partial unique index below.
                                        ingest_key               TEXT,
                                        source_path              TEXT,

                                        description              TEXT,

    -- ── Full-text search ─────────────────────────────────────────────────────
                                        description_tsv          tsvector GENERATED ALWAYS AS (
                                            to_tsvector('english',
                                                        coalesce(name, '')        || ' ' ||
                                                        coalesce(popular_name, '') || ' ' ||
                                                        coalesce(description, '')
                                            )
                                            ) STORED,

                                        extra_attributes         JSONB        NOT NULL DEFAULT '{}'::jsonb,
                                        created_at               TIMESTAMPTZ  NOT NULL DEFAULT now(),
                                        updated_at               TIMESTAMPTZ  NOT NULL DEFAULT now(),

                                        CONSTRAINT chk_variant_production_years CHECK (
                                            production_end_year IS NULL
                                                OR production_start_year IS NULL
                                                OR production_end_year >= production_start_year
                                            ),
                                        CONSTRAINT chk_variant_engine_count CHECK (
                                            engine_count IS NULL OR engine_count > 0
                                            ),
                                        CONSTRAINT chk_variant_passenger CHECK (
                                            passenger_capacity IS NULL OR passenger_capacity >= 0
                                            ),
                                        CONSTRAINT chk_variant_crew CHECK (
                                            crew_count IS NULL OR crew_count >= 0
                                            )
);
COMMENT ON TABLE aircraft_core.variants IS
    'Atomic unit of the aircraft identity hierarchy. '
    'All domain tables (aircraft_specs, aircraft_power, aircraft_systems, '
    'aircraft_military, aircraft_market, aircraft_maint, aircraft_prov) '
    'reference variants.id. '
    'Quick-filter columns (passenger_capacity, engine_count, propulsion_category, '
    'landing_gear_type) are denormalized for faceted search without joins; '
    'authoritative values come from the respective domain phase tables.';
COMMENT ON COLUMN aircraft_core.variants.ingest_key IS
    'Opaque ingestion deduplication key, e.g. "planephd:123456". '
    'Populated by Phase 17 ingestion to prevent duplicate variant rows. '
    'Not a semantic business key; superseded by aircraft_prov.source_documents '
    'once Phase 14 is populated.';
COMMENT ON COLUMN aircraft_core.variants.source_path IS
    'URI path from the originating source system used during Phase 17 ingestion '
    '(e.g., PlanePHD /wizard/details/123456). '
    'Canonical source URL lives in aircraft_prov.source_documents.source_url '
    'after Phase 14 population.';
COMMENT ON COLUMN aircraft_core.variants.description_tsv IS
    'Generated stored tsvector over name + popular_name + description. '
    'GIN index enables fast full-text variant search in Phase 16 read models.';
COMMENT ON COLUMN aircraft_core.variants.production_end_year IS
    'NULL indicates the variant is still in production (or status is unknown). '
    'Use is_in_production for an explicit three-state flag.';

-- =============================================================================
-- aircraft_core.variant_aliases
-- Alternate designations for a variant: NATO reporting names (e.g., "Fulcrum"),
-- military serial designations (e.g., "C-17A"), popular names, export names,
-- ICAO type codes, and former/superseded designations.
-- alias_type uses TEXT CHECK (6 stable values) rather than a lookup table
-- because the set is definitionally complete and non-extensible by design.
-- =============================================================================

CREATE TABLE aircraft_core.variant_aliases (
                                               id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                               variant_id   BIGINT NOT NULL
                                                   REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
                                               alias_type   TEXT NOT NULL,
                                               alias        TEXT NOT NULL,
    -- Which country uses this alias. NULL = universally used.
                                               country_code VARCHAR(3)
                                                                   REFERENCES aircraft_geo.countries(code) ON DELETE SET NULL,
                                               notes        TEXT,
                                               CONSTRAINT chk_alias_type CHECK (
                                                   alias_type IN (
                                                                  'NATO_REPORTING_NAME',  -- e.g., "Fulcrum", "Flanker"
                                                                  'MILITARY_DESIGNATION', -- e.g., "C-17A", "KC-135R"
                                                                  'POPULAR_NAME',         -- e.g., "Skyhawk", "Hercules"
                                                                  'EXPORT_NAME',          -- name used specifically in export markets
                                                                  'ICAO_TYPE_CODE',       -- e.g., "C172", "B738", "F15"
                                                                  'FORMER_DESIGNATION'    -- superseded or historical designation
                                                       )
                                                   ),
                                               UNIQUE (variant_id, alias_type, alias)
);
COMMENT ON TABLE aircraft_core.variant_aliases IS
    'Alternate variant designations: NATO reporting names, military codes, '
    'popular names, export designations, ICAO type codes, and former names. '
    'country_code indicates which country uses this alias (NULL = universal). '
    'Trigram-indexed for search; also enables Phase 17 ingestion to resolve '
    'variant name strings to canonical variant rows.';
COMMENT ON COLUMN aircraft_core.variant_aliases.alias_type IS
    'TEXT CHECK (6 values: NATO_REPORTING_NAME, MILITARY_DESIGNATION, '
    'POPULAR_NAME, EXPORT_NAME, ICAO_TYPE_CODE, FORMER_DESIGNATION). '
    'Uses CHECK rather than lookup FK because the set is definitionally '
    'stable and non-extensible.';

-- =============================================================================
-- aircraft_core.variant_roles
-- M:N junction: one variant may have multiple roles (e.g., a multirole
-- fighter has MILITARY_FIGHTER_AIR_SUP and MILITARY_ATTACK_STRIKE).
-- Exactly one role per variant may be is_primary = TRUE (partial UNIQUE index).
-- =============================================================================

CREATE TABLE aircraft_core.variant_roles (
                                             variant_id  BIGINT NOT NULL
                                                 REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
                                             role_code   aircraft_ref.lookup_code NOT NULL
                                                 REFERENCES aircraft_ref.aircraft_roles(code) ON DELETE RESTRICT,
                                             is_primary  BOOLEAN  NOT NULL DEFAULT FALSE,
    -- Sort priority for display ordering when multiple roles exist.
                                             sort_order  SMALLINT NOT NULL DEFAULT 0,
                                             PRIMARY KEY (variant_id, role_code)
);
COMMENT ON TABLE aircraft_core.variant_roles IS
    'M:N junction between variants and aircraft roles. '
    'A multirole variant holds several rows; is_primary marks the dominant role. '
    'The partial UNIQUE index uq_variant_primary_role enforces at most one '
    'primary role per variant.';

-- =============================================================================
-- aircraft_core.variant_manufacturers
-- M:N junction: one variant may have multiple manufacturing organizations
-- (designer, primary manufacturer, license producers, major subcontractors).
-- At most one row per variant may be is_primary = TRUE (partial UNIQUE index).
-- production_country_code records where a specific org built the aircraft
-- (relevant for license production where the org is in a different country
-- from the original designer).
-- =============================================================================

CREATE TABLE aircraft_core.variant_manufacturers (
                                                     variant_id              BIGINT NOT NULL
                                                         REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
                                                     org_id                  BIGINT NOT NULL
                                                         REFERENCES aircraft_org.organizations(id) ON DELETE RESTRICT,
    -- Role of this organization in the production of this variant.
    -- TEXT CHECK (5 stable values).
                                                     role                    TEXT NOT NULL DEFAULT 'MANUFACTURER',
                                                     is_primary              BOOLEAN  NOT NULL DEFAULT FALSE,
    -- Country where this org's production line is located.
    -- Typically the org's country_code, but differs for offshore plants.
                                                     production_country_code VARCHAR(3)
                                                         REFERENCES aircraft_geo.countries(code) ON DELETE RESTRICT,
                                                     notes                   TEXT,
                                                     PRIMARY KEY (variant_id, org_id),
                                                     CONSTRAINT chk_vm_role CHECK (
                                                         role IN (
                                                                  'DESIGNER',            -- designed but did not manufacture
                                                                  'MANUFACTURER',        -- primary production
                                                                  'LICENSE_MANUFACTURER',-- produced under license
                                                                  'SUBCONTRACTOR',       -- major structural/system subcontractor
                                                                  'JOINT_PRODUCER'       -- joint-venture co-production partner
                                                             )
                                                         )
);
COMMENT ON TABLE aircraft_core.variant_manufacturers IS
    'M:N junction between variants and manufacturing organizations. '
    'Supports joint ventures, license production, and design/build split. '
    'is_primary marks the organization credited as the primary manufacturer '
    '(used on detail pages, search result snippets, and comparison cards). '
    'production_country_code records the plant location (differs from '
    'org.country_code for foreign-licensed production).';

-- =============================================================================
-- aircraft_core.variant_operators
-- M:N junction: which countries / organizations operate this variant.
-- country_code is the operator nation; operator_org_id optionally identifies
-- the specific air arm or airline within that country.
-- is_current = TRUE for current operational use; FALSE for retired/historical.
-- Partial UNIQUE index prevents duplicate current (variant, country, org) rows.
-- =============================================================================

CREATE TABLE aircraft_core.variant_operators (
                                                 id                 BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                                 variant_id         BIGINT NOT NULL
                                                     REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
                                                 country_code       VARCHAR(3) NOT NULL
                                                     REFERENCES aircraft_geo.countries(code) ON DELETE RESTRICT,
    -- Specific operating organization (air force, airline, etc.).
    -- NULL when country-level attribution is sufficient.
                                                 operator_org_id    BIGINT
                                                     REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
                                                 service_entry_year aircraft_ref.year_value,
                                                 retirement_year    aircraft_ref.year_value,   -- NULL = still in service
                                                 is_current         BOOLEAN  NOT NULL DEFAULT TRUE,
    -- Approximate fleet size at peak or current (for encyclopedia comparison).
                                                 quantity_approx    INTEGER,
                                                 notes              TEXT,
                                                 CONSTRAINT chk_vo_years CHECK (
                                                     retirement_year IS NULL
                                                         OR service_entry_year IS NULL
                                                         OR retirement_year >= service_entry_year
                                                     ),
                                                 CONSTRAINT chk_vo_quantity CHECK (
                                                     quantity_approx IS NULL OR quantity_approx > 0
                                                     )
);
COMMENT ON TABLE aircraft_core.variant_operators IS
    'M:N junction tracking which countries and organizations operate each variant. '
    'country_code = operating nation; operator_org_id = specific air arm or airline. '
    'Replaces aircraft_operators (country-only M:N) from the reference schema '
    'with org-level granularity and temporal data. '
    'is_current = FALSE for retired/historical operators.';
COMMENT ON COLUMN aircraft_core.variant_operators.operator_org_id IS
    'FK to aircraft_org.organizations for the specific operator (e.g., '
    '"United States Air Force"). NULL when country-level is sufficient. '
    'The partial UNIQUE index uses COALESCE(operator_org_id, -1) to treat '
    'all NULL-org rows for a country as a single representative record.';
COMMENT ON COLUMN aircraft_core.variant_operators.quantity_approx IS
    'Approximate fleet size (peak or current). Encyclopedia reference only; '
    'exact operational counts are sensitive and not stored here.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_families_updated
    BEFORE UPDATE ON aircraft_core.families
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_models_updated
    BEFORE UPDATE ON aircraft_core.models
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_variants_updated
    BEFORE UPDATE ON aircraft_core.variants
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_core.families ───────────────────────────────────────────────────

-- Trigram search on family name
CREATE INDEX idx_families_name_trgm
    ON aircraft_core.families USING gin (name gin_trgm_ops);

-- Full-text search (generated column)
CREATE INDEX idx_families_fts
    ON aircraft_core.families USING gin (name_tsv);

-- GIN on name_aliases array for ingestion alias resolution
CREATE INDEX idx_families_aliases
    ON aircraft_core.families USING gin (name_aliases)
    WHERE name_aliases IS NOT NULL;

-- Manufacturer and country facets
CREATE INDEX idx_families_manufacturer
    ON aircraft_core.families (manufacturer_org_id)
    WHERE manufacturer_org_id IS NOT NULL;

CREATE INDEX idx_families_country
    ON aircraft_core.families (country_of_origin_code)
    WHERE country_of_origin_code IS NOT NULL;

-- ── aircraft_core.models ─────────────────────────────────────────────────────

-- Parent FK — most common query: "all models in a family"
CREATE INDEX idx_models_family
    ON aircraft_core.models (family_id);

-- Trigram search
CREATE INDEX idx_models_name_trgm
    ON aircraft_core.models USING gin (name gin_trgm_ops);

-- ── aircraft_core.variants ───────────────────────────────────────────────────

-- Parent FK — "all variants of a model"
CREATE INDEX idx_variants_model
    ON aircraft_core.variants (model_id);

-- Full-text search on description_tsv (generated column)
CREATE INDEX idx_variants_fts
    ON aircraft_core.variants USING gin (description_tsv);

-- Trigram search on name for autocomplete
CREATE INDEX idx_variants_name_trgm
    ON aircraft_core.variants USING gin (name gin_trgm_ops);

-- Faceted filtering — high-cardinality columns used in buyer search
CREATE INDEX idx_variants_country
    ON aircraft_core.variants (country_of_origin_code)
    WHERE country_of_origin_code IS NOT NULL;

CREATE INDEX idx_variants_service_status
    ON aircraft_core.variants (service_status_code)
    WHERE service_status_code IS NOT NULL;

CREATE INDEX idx_variants_propulsion
    ON aircraft_core.variants (propulsion_category_code)
    WHERE propulsion_category_code IS NOT NULL;

CREATE INDEX idx_variants_gear
    ON aircraft_core.variants (landing_gear_type_code)
    WHERE landing_gear_type_code IS NOT NULL;

CREATE INDEX idx_variants_engine_count
    ON aircraft_core.variants (engine_count)
    WHERE engine_count IS NOT NULL;

CREATE INDEX idx_variants_pax
    ON aircraft_core.variants (passenger_capacity)
    WHERE passenger_capacity IS NOT NULL;

-- Year range filter — covering index for production year range queries
CREATE INDEX idx_variants_production_years
    ON aircraft_core.variants (production_start_year, production_end_year);

-- Ingestion deduplication key (partial: only non-NULL values must be unique)
CREATE UNIQUE INDEX uq_variants_ingest_key
    ON aircraft_core.variants (ingest_key)
    WHERE ingest_key IS NOT NULL;

-- JSONB escape valve
CREATE INDEX idx_variants_extra
    ON aircraft_core.variants USING gin (extra_attributes jsonb_path_ops);

-- ── aircraft_core.variant_aliases ────────────────────────────────────────────

-- Trigram search on alias text (autocomplete, fuzzy name search)
CREATE INDEX idx_aliases_alias_trgm
    ON aircraft_core.variant_aliases USING gin (alias gin_trgm_ops);

-- Lookup by type (e.g., "all ICAO_TYPE_CODE aliases")
CREATE INDEX idx_aliases_type
    ON aircraft_core.variant_aliases (alias_type, alias);

-- Lookup aliases for a variant
CREATE INDEX idx_aliases_variant
    ON aircraft_core.variant_aliases (variant_id);

-- ── aircraft_core.variant_roles ──────────────────────────────────────────────

-- "All variants with this role" — common mission-profile filter
CREATE INDEX idx_vroles_role
    ON aircraft_core.variant_roles (role_code);

-- At most ONE primary role per variant
CREATE UNIQUE INDEX uq_variant_primary_role
    ON aircraft_core.variant_roles (variant_id)
    WHERE is_primary;

-- ── aircraft_core.variant_manufacturers ──────────────────────────────────────

-- "All variants produced by organization X"
CREATE INDEX idx_vmfr_org
    ON aircraft_core.variant_manufacturers (org_id);

-- At most ONE primary manufacturer per variant
CREATE UNIQUE INDEX uq_variant_primary_mfr
    ON aircraft_core.variant_manufacturers (variant_id)
    WHERE is_primary;

-- ── aircraft_core.variant_operators ──────────────────────────────────────────

-- "All countries operating variant X" / "All current operators"
CREATE INDEX idx_vop_variant_current
    ON aircraft_core.variant_operators (variant_id, is_current);

-- "All variants operated by country Y"
CREATE INDEX idx_vop_country
    ON aircraft_core.variant_operators (country_code);

-- "All variants operated by organization Z"
CREATE INDEX idx_vop_org
    ON aircraft_core.variant_operators (operator_org_id)
    WHERE operator_org_id IS NOT NULL;

-- Prevent duplicate CURRENT (variant, country, org) rows.
-- COALESCE(operator_org_id, -1) folds all NULL-org rows into a
-- single representative slot per (variant, country).
CREATE UNIQUE INDEX uq_variant_operator_current
    ON aircraft_core.variant_operators
        (variant_id, country_code, COALESCE(operator_org_id, -1))
    WHERE is_current;

COMMIT;