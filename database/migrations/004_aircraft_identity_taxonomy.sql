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
-- Changes from original design (post-evaluation fixes):
--   • variant_roles, variant_manufacturers, variant_operators: added updated_at
--     TIMESTAMPTZ column and corresponding set_updated_at() triggers.
--     Junction tables are updated during curation (role changes, manufacturer
--     corrections, operator retirements) and need audit timestamps like any
--     other table that carries mutable state.
--
-- No aircraft data rows are seeded here; data arrives via Phase 17 ingestion.
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_core.families
-- =============================================================================

CREATE TABLE aircraft_core.families (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug                  aircraft_ref.slug_text NOT NULL UNIQUE,
    name                  TEXT NOT NULL,
    common_name           TEXT,
    name_aliases          TEXT[],
    manufacturer_org_id   BIGINT
                              REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    country_of_origin_code VARCHAR(3)
                              REFERENCES aircraft_geo.countries(code)   ON DELETE RESTRICT,
    first_flight_year     aircraft_ref.year_value,
    description           TEXT,
    name_tsv              tsvector GENERATED ALWAYS AS (
        to_tsvector('english',
            coalesce(name, '')        || ' ' ||
            coalesce(common_name, '') || ' ' ||
            aircraft_ref.text_array_to_string(coalesce(name_aliases, ARRAY[]::text[]), ' ')
        )
    ) STORED,
    extra_attributes      JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_core.families IS
    'Broadest aircraft identity grouping (e.g., "Cessna 172", "F-16 Fighting Falcon"). '
    'One family contains one or more models. manufacturer_org_id captures the original '
    'designer; all production parties are tracked in variant_manufacturers.';
COMMENT ON COLUMN aircraft_core.families.name_aliases IS
    'Alternative, former, or local-language family names. GIN-indexed for alias '
    'resolution during Phase 17 ingestion (manufacturer string matching).';
COMMENT ON COLUMN aircraft_core.families.name_tsv IS
    'Generated stored tsvector over name + common_name + name_aliases. '
    'Backed by a GIN index for full-text family search in Phase 16.';

-- =============================================================================
-- aircraft_core.models
-- =============================================================================

CREATE TABLE aircraft_core.models (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    family_id            BIGINT NOT NULL
                             REFERENCES aircraft_core.families(id) ON DELETE RESTRICT,
    slug                 aircraft_ref.slug_text NOT NULL UNIQUE,
    name                 TEXT NOT NULL,
    display_name         TEXT,
    name_aliases         TEXT[],
    series               TEXT,
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
-- =============================================================================

CREATE TABLE aircraft_core.variants (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    model_id                 BIGINT NOT NULL
                                 REFERENCES aircraft_core.models(id) ON DELETE RESTRICT,
    slug                     aircraft_ref.slug_text NOT NULL UNIQUE,
    name                     TEXT NOT NULL,
    popular_name             TEXT,
    variant_type_code        aircraft_ref.lookup_code
                                 REFERENCES aircraft_ref.variant_types(code),
    service_status_code      aircraft_ref.lookup_code
                                 REFERENCES aircraft_ref.service_statuses(code),
    country_of_origin_code   VARCHAR(3)
                                 REFERENCES aircraft_geo.countries(code) ON DELETE RESTRICT,
    first_flight_year        aircraft_ref.year_value,
    certification_year       aircraft_ref.year_value,
    production_start_year    aircraft_ref.year_value,
    production_end_year      aircraft_ref.year_value,
    passenger_capacity       SMALLINT,
    crew_count               SMALLINT,
    landing_gear_type_code   aircraft_ref.lookup_code
                                 REFERENCES aircraft_ref.landing_gear_types(code),
    propulsion_category_code aircraft_ref.lookup_code
                                 REFERENCES aircraft_ref.propulsion_categories(code),
    engine_count             SMALLINT,
    is_in_production         BOOLEAN,
    -- Ingestion staging fields
    ingest_key               TEXT,
    source_path              TEXT,
    description              TEXT,
    description_tsv          tsvector GENERATED ALWAYS AS (
        to_tsvector('english',
            coalesce(name, '')         || ' ' ||
            coalesce(popular_name, '')  || ' ' ||
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
    'Opaque ingestion deduplication key, e.g. "AERONCA::11AC Chief". '
    'Populated by Phase 17 ingestion to prevent duplicate variant rows. '
    'Not a semantic business key; superseded by aircraft_prov.source_documents '
    'once Phase 14 is populated.';
COMMENT ON COLUMN aircraft_core.variants.source_path IS
    'URI path from the originating source system used during Phase 17 ingestion. '
    'Canonical source URL lives in aircraft_prov.source_documents.source_url.';
COMMENT ON COLUMN aircraft_core.variants.production_end_year IS
    'NULL indicates the variant is still in production (or status is unknown). '
    'Use is_in_production for an explicit three-state flag.';

-- =============================================================================
-- aircraft_core.variant_aliases
-- =============================================================================

CREATE TABLE aircraft_core.variant_aliases (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id   BIGINT NOT NULL
                     REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    alias_type   TEXT NOT NULL,
    alias        TEXT NOT NULL,
    country_code VARCHAR(3)
                     REFERENCES aircraft_geo.countries(code) ON DELETE SET NULL,
    notes        TEXT,
    CONSTRAINT chk_alias_type CHECK (
        alias_type IN (
            'NATO_REPORTING_NAME',
            'MILITARY_DESIGNATION',
            'POPULAR_NAME',
            'EXPORT_NAME',
            'ICAO_TYPE_CODE',
            'FORMER_DESIGNATION'
        )
    ),
    UNIQUE (variant_id, alias_type, alias)
);

COMMENT ON TABLE aircraft_core.variant_aliases IS
    'Alternate variant designations: NATO reporting names, military codes, '
    'popular names, export designations, ICAO type codes, and former names. '
    'country_code indicates which country uses this alias (NULL = universal).';
COMMENT ON COLUMN aircraft_core.variant_aliases.alias_type IS
    'TEXT CHECK (6 values). Uses CHECK rather than lookup FK because the set is '
    'definitionally stable and non-extensible.';

-- =============================================================================
-- aircraft_core.variant_roles
-- updated_at added: roles are updated during curation (e.g., reclassifying a
-- multirole aircraft, correcting an ingestion-assigned role).
-- =============================================================================

CREATE TABLE aircraft_core.variant_roles (
    variant_id  BIGINT NOT NULL
                    REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    role_code   aircraft_ref.lookup_code NOT NULL
                    REFERENCES aircraft_ref.aircraft_roles(code) ON DELETE RESTRICT,
    is_primary  BOOLEAN  NOT NULL DEFAULT FALSE,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    -- Timestamps: role assignments change during curation
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (variant_id, role_code)
);

COMMENT ON TABLE aircraft_core.variant_roles IS
    'M:N junction between variants and aircraft roles. '
    'A multirole variant holds several rows; is_primary marks the dominant role. '
    'The partial UNIQUE index uq_variant_primary_role enforces at most one '
    'primary role per variant.';

-- =============================================================================
-- aircraft_core.variant_manufacturers
-- updated_at added: manufacturer links are corrected during curation (e.g.,
-- matching a stub org to a canonical organization, fixing is_primary flag).
-- =============================================================================

CREATE TABLE aircraft_core.variant_manufacturers (
    variant_id              BIGINT NOT NULL
                                REFERENCES aircraft_core.variants(id)       ON DELETE CASCADE,
    org_id                  BIGINT NOT NULL
                                REFERENCES aircraft_org.organizations(id)   ON DELETE RESTRICT,
    role                    TEXT NOT NULL DEFAULT 'MANUFACTURER',
    is_primary              BOOLEAN  NOT NULL DEFAULT FALSE,
    production_country_code VARCHAR(3)
                                REFERENCES aircraft_geo.countries(code)     ON DELETE RESTRICT,
    notes                   TEXT,
    -- Timestamps: manufacturer links are corrected during curation
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (variant_id, org_id),
    CONSTRAINT chk_vm_role CHECK (
        role IN (
            'DESIGNER',
            'MANUFACTURER',
            'LICENSE_MANUFACTURER',
            'SUBCONTRACTOR',
            'JOINT_PRODUCER'
        )
    )
);

COMMENT ON TABLE aircraft_core.variant_manufacturers IS
    'M:N junction between variants and manufacturing organizations. '
    'Supports joint ventures, license production, and design/build split. '
    'is_primary marks the organization credited as the primary manufacturer. '
    'production_country_code records the plant location.';

-- =============================================================================
-- aircraft_core.variant_operators
-- updated_at added: operator records change when fleets are retired, transferred,
-- or when is_current status is updated.
-- =============================================================================

CREATE TABLE aircraft_core.variant_operators (
    id                 BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id         BIGINT NOT NULL
                           REFERENCES aircraft_core.variants(id)         ON DELETE CASCADE,
    country_code       VARCHAR(3) NOT NULL
                           REFERENCES aircraft_geo.countries(code)        ON DELETE RESTRICT,
    operator_org_id    BIGINT
                           REFERENCES aircraft_org.organizations(id)      ON DELETE SET NULL,
    service_entry_year aircraft_ref.year_value,
    retirement_year    aircraft_ref.year_value,
    is_current         BOOLEAN  NOT NULL DEFAULT TRUE,
    quantity_approx    INTEGER,
    notes              TEXT,
    -- Timestamps: retirement status and fleet size updated during curation
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
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
    'is_current = FALSE for retired/historical operators.';
COMMENT ON COLUMN aircraft_core.variant_operators.operator_org_id IS
    'FK to aircraft_org.organizations. NULL when country-level attribution is '
    'sufficient. The partial UNIQUE index uses COALESCE(operator_org_id, -1) to '
    'treat all NULL-org rows for a country as a single representative record.';
COMMENT ON COLUMN aircraft_core.variant_operators.quantity_approx IS
    'Approximate fleet size (peak or current). Encyclopedia reference only.';

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

-- Junction table triggers (new — fixes missing audit trail on mutable rows)
CREATE TRIGGER trg_variant_roles_updated
    BEFORE UPDATE ON aircraft_core.variant_roles
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_variant_manufacturers_updated
    BEFORE UPDATE ON aircraft_core.variant_manufacturers
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_variant_operators_updated
    BEFORE UPDATE ON aircraft_core.variant_operators
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_core.families ───────────────────────────────────────────────────

CREATE INDEX idx_families_name_trgm
    ON aircraft_core.families USING gin (name gin_trgm_ops);

CREATE INDEX idx_families_fts
    ON aircraft_core.families USING gin (name_tsv);

CREATE INDEX idx_families_aliases
    ON aircraft_core.families USING gin (name_aliases)
    WHERE name_aliases IS NOT NULL;

CREATE INDEX idx_families_manufacturer
    ON aircraft_core.families (manufacturer_org_id)
    WHERE manufacturer_org_id IS NOT NULL;

CREATE INDEX idx_families_country
    ON aircraft_core.families (country_of_origin_code)
    WHERE country_of_origin_code IS NOT NULL;

-- ── aircraft_core.models ─────────────────────────────────────────────────────

CREATE INDEX idx_models_family
    ON aircraft_core.models (family_id);

CREATE INDEX idx_models_name_trgm
    ON aircraft_core.models USING gin (name gin_trgm_ops);

-- ── aircraft_core.variants ───────────────────────────────────────────────────

CREATE INDEX idx_variants_model
    ON aircraft_core.variants (model_id);

CREATE INDEX idx_variants_fts
    ON aircraft_core.variants USING gin (description_tsv);

CREATE INDEX idx_variants_name_trgm
    ON aircraft_core.variants USING gin (name gin_trgm_ops);

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

CREATE INDEX idx_variants_production_years
    ON aircraft_core.variants (production_start_year, production_end_year);

CREATE UNIQUE INDEX uq_variants_ingest_key
    ON aircraft_core.variants (ingest_key)
    WHERE ingest_key IS NOT NULL;

CREATE INDEX idx_variants_extra
    ON aircraft_core.variants USING gin (extra_attributes jsonb_path_ops);

-- ── aircraft_core.variant_aliases ────────────────────────────────────────────

CREATE INDEX idx_aliases_alias_trgm
    ON aircraft_core.variant_aliases USING gin (alias gin_trgm_ops);

CREATE INDEX idx_aliases_type
    ON aircraft_core.variant_aliases (alias_type, alias);

CREATE INDEX idx_aliases_variant
    ON aircraft_core.variant_aliases (variant_id);

-- ── aircraft_core.variant_roles ──────────────────────────────────────────────

CREATE INDEX idx_vroles_role
    ON aircraft_core.variant_roles (role_code);

CREATE UNIQUE INDEX uq_variant_primary_role
    ON aircraft_core.variant_roles (variant_id)
    WHERE is_primary;

-- ── aircraft_core.variant_manufacturers ──────────────────────────────────────

CREATE INDEX idx_vmfr_org
    ON aircraft_core.variant_manufacturers (org_id);

CREATE UNIQUE INDEX uq_variant_primary_mfr
    ON aircraft_core.variant_manufacturers (variant_id)
    WHERE is_primary;

-- ── aircraft_core.variant_operators ──────────────────────────────────────────

CREATE INDEX idx_vop_variant_current
    ON aircraft_core.variant_operators (variant_id, is_current);

CREATE INDEX idx_vop_country
    ON aircraft_core.variant_operators (country_code);

CREATE INDEX idx_vop_org
    ON aircraft_core.variant_operators (operator_org_id)
    WHERE operator_org_id IS NOT NULL;

-- COALESCE(operator_org_id, -1) prevents two NULL-org rows for the same
-- (variant, country) pair from both passing a standard UNIQUE index.
CREATE UNIQUE INDEX uq_variant_operator_current
    ON aircraft_core.variant_operators
        (variant_id, country_code, COALESCE(operator_org_id, -1))
    WHERE is_current;

COMMIT;