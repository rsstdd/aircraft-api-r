-- =============================================================================
-- File: database/migrations/009_propulsion_engines_rotors_stcs.sql
-- Phase 9 — aircraft_power: engine specifications, variant powerplant links,
-- propeller catalog, rotor systems, APU data, and engine conversion STCs.
--
-- Changes from original design (post-evaluation fixes):
--   • engine_variants: added rated_thrust_n NUMERIC column to store thrust in
--     the source unit (Newtons). thrust_lbf_dry stores the canonical LBF value
--     (rated_thrust_n × 0.224809). Both are preserved following the raw/
--     canonical triple pattern used across aircraft_specs. Phase 17 ingestion
--     writes rated_thrust_n from the source string and derives thrust_lbf_dry.
--   • engine_variants: slug generation delegated to Phase 17 ingestion via
--     aircraft_ref.slugify(manufacturer_name_raw || '-' || model_designation).
--     The slug column is NOT NULL UNIQUE so every INSERT must supply one.
--   • variant_powerplants: added updated_at TIMESTAMPTZ and corresponding
--     trigger (the trigger existed in the original but the column did not,
--     causing a runtime error on any UPDATE).
--   • variant_powerplants: added source_document_id BIGINT nullable FK to
--     aircraft_prov.source_documents. Phase 17 promotion writes this to link
--     each powerplant link back to the ingestion source document.
--   • variant_powerplants: added partial UNIQUE index
--     uq_vp_standard_per_variant enforcing at most one is_standard = TRUE row
--     per variant. The evaluation identified that nothing prevented two
--     different "standard" engines on the same variant simultaneously.
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_power.engine_variants
-- =============================================================================

CREATE TABLE aircraft_power.engine_variants (
    id                        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- slug is NOT NULL UNIQUE; Phase 17 ingestion must supply it via
    -- aircraft_ref.slugify(manufacturer_name_raw || '-' || model_designation).
    slug                      aircraft_ref.slug_text NOT NULL UNIQUE,
    manufacturer_org_id       BIGINT
                                  REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    manufacturer_name_raw     TEXT,
    model_designation         TEXT NOT NULL,
    model_family              TEXT,
    name_aliases              TEXT[],
    propulsion_category_code  aircraft_ref.lookup_code
                                  REFERENCES aircraft_ref.propulsion_categories(code),
    fuel_type_code            aircraft_ref.lookup_code
                                  REFERENCES aircraft_ref.fuel_types(code),

    -- ── Power output ─────────────────────────────────────────────────────────
    hp_rated                  NUMERIC,   -- rated continuous shaft horsepower
    hp_takeoff                NUMERIC,   -- takeoff power (may differ from continuous)
    -- Thrust stored in two columns following the raw/canonical triple pattern:
    --   rated_thrust_n    = raw source value (Newtons, as published)
    --   thrust_lbf_dry    = canonical comparison value (LBF = N × 0.224809)
    -- Phase 17 ingestion writes both from the source 'thrust' field.
    rated_thrust_n            NUMERIC,   -- raw thrust in Newtons (source unit)
    thrust_lbf_dry            NUMERIC,   -- canonical dry thrust in LBF
    thrust_lbf_wet            NUMERIC,   -- wet (afterburner) thrust in LBF
    has_afterburner           BOOLEAN    NOT NULL DEFAULT FALSE,

    -- ── Engine characteristics ────────────────────────────────────────────────
    has_fadec                 BOOLEAN    NOT NULL DEFAULT FALSE,
    is_turbocharged           BOOLEAN    NOT NULL DEFAULT FALSE,
    is_supercharged           BOOLEAN    NOT NULL DEFAULT FALSE,
    is_geared                 BOOLEAN    NOT NULL DEFAULT FALSE,
    is_fuel_injected          BOOLEAN    NOT NULL DEFAULT FALSE,
    displacement_cubic_in     NUMERIC,
    cylinder_count            SMALLINT,
    engine_weight_lbs         NUMERIC,
    specific_fuel_consumption NUMERIC,
    sfc_unit                  TEXT,

    -- ── Overhaul / life limits ────────────────────────────────────────────────
    tbo_hours                 INTEGER,   -- Time Between Overhaul in flight hours
    -- tbo_years: calendar TBO limit. Populated from 'years_before_overhaul' in
    -- PlanePHD engine JSON (e.g., "12" → 12). Many GA engines have both an
    -- hour-based and calendar-based TBO, whichever comes first.
    tbo_years                 INTEGER,

    description               TEXT,
    extra_attributes          JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at                TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at                TIMESTAMPTZ  NOT NULL DEFAULT now(),

    CONSTRAINT chk_ev_hp_nonneg CHECK (
        (hp_rated    IS NULL OR hp_rated    >= 0) AND
        (hp_takeoff  IS NULL OR hp_takeoff  >= 0)
    ),
    CONSTRAINT chk_ev_thrust_nonneg CHECK (
        (rated_thrust_n  IS NULL OR rated_thrust_n  >= 0) AND
        (thrust_lbf_dry  IS NULL OR thrust_lbf_dry  >= 0) AND
        (thrust_lbf_wet  IS NULL OR thrust_lbf_wet  >= 0)
    ),
    CONSTRAINT chk_ev_tbo CHECK (
        (tbo_hours IS NULL OR tbo_hours > 0) AND
        (tbo_years IS NULL OR tbo_years > 0)
    ),
    CONSTRAINT chk_ev_sfc_unit CHECK (
        sfc_unit IS NULL OR
        sfc_unit IN ('LB_PER_HP_HR', 'LB_PER_LBF_HR', 'G_PER_KN_HR')
    ),
    CONSTRAINT chk_ev_afterburner CHECK (
        thrust_lbf_wet IS NULL OR has_afterburner
    )
);

COMMENT ON TABLE aircraft_power.engine_variants IS
    'Engine specification catalog: one row per distinct engine model/variant. '
    'name_aliases supports Phase 17 ingestion alias resolution. '
    'manufacturer_name_raw is a fallback when the manufacturer is not yet '
    'in aircraft_org.organizations.';
COMMENT ON COLUMN aircraft_power.engine_variants.rated_thrust_n IS
    'Raw thrust value in Newtons, exactly as published in the source. '
    'Preserved for source fidelity following the raw/canonical pattern. '
    'Canonical comparison value is thrust_lbf_dry (N × 0.224809).';
COMMENT ON COLUMN aircraft_power.engine_variants.thrust_lbf_dry IS
    'Dry (unaugmented) thrust in LBF: the canonical comparison unit for jets. '
    'Derived from rated_thrust_n × 0.224809 during Phase 17 ingestion. '
    'NULL for piston/turboprop engines (use hp_rated for those).';
COMMENT ON COLUMN aircraft_power.engine_variants.tbo_hours IS
    'Manufacturer Time Between Overhaul in flight hours. '
    'Per FAR 91, TBO is a recommendation for Part 91 operations; '
    'Part 135 operators must comply.';
COMMENT ON COLUMN aircraft_power.engine_variants.tbo_years IS
    'Calendar TBO limit in years. Populated from the PlanePHD '
    'engine.years_before_overhaul field. Many GA engines expire at '
    'whichever limit (hours or calendar) is reached first.';

-- =============================================================================
-- aircraft_power.propeller_specs
-- =============================================================================

CREATE TABLE aircraft_power.propeller_specs (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug                 aircraft_ref.slug_text NOT NULL UNIQUE,
    manufacturer_org_id  BIGINT
                             REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    manufacturer_name_raw TEXT,
    model_designation    TEXT NOT NULL,
    name_aliases         TEXT[],
    blade_count          SMALLINT,
    diameter_in          NUMERIC,
    diameter_ft          NUMERIC,
    prop_type            TEXT,
    material             TEXT,
    tbo_hours            INTEGER,
    notes                TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_ps_type CHECK (
        prop_type IS NULL OR prop_type IN (
            'FIXED_PITCH',
            'GROUND_ADJUSTABLE',
            'CONSTANT_SPEED',
            'CONSTANT_SPEED_FEATHERING',
            'CONSTANT_SPEED_REVERSING'
        )
    ),
    CONSTRAINT chk_ps_dims CHECK (
        (diameter_in IS NULL OR diameter_in > 0) AND
        (diameter_ft IS NULL OR diameter_ft > 0) AND
        (blade_count IS NULL OR blade_count >= 2) AND
        (tbo_hours   IS NULL OR tbo_hours   > 0)
    )
);

COMMENT ON TABLE aircraft_power.propeller_specs IS
    'Propeller model catalog. diameter_ft is the canonical comparison unit '
    'computed from diameter_in / 12 during ingestion.';

-- =============================================================================
-- aircraft_power.variant_powerplants
--
-- Changes from original:
--   • updated_at TIMESTAMPTZ added (trigger existed but column was missing)
--   • source_document_id BIGINT added (Phase 17 needs it; original lacked it)
--   • uq_vp_standard_per_variant partial UNIQUE index added (prevents two
--     is_standard = TRUE rows for the same variant simultaneously)
-- =============================================================================

CREATE TABLE aircraft_power.variant_powerplants (
    id                 BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id         BIGINT NOT NULL
                           REFERENCES aircraft_core.variants(id)        ON DELETE CASCADE,
    engine_variant_id  BIGINT NOT NULL
                           REFERENCES aircraft_power.engine_variants(id) ON DELETE RESTRICT,
    engine_count       SMALLINT NOT NULL DEFAULT 1,
    is_standard        BOOLEAN  NOT NULL DEFAULT FALSE,
    is_optional        BOOLEAN  NOT NULL DEFAULT FALSE,
    is_primary         BOOLEAN  NOT NULL DEFAULT FALSE,
    install_position   TEXT,
    notes              TEXT,
    -- Provenance: which source document established this powerplant link.
    -- NOTE: The FK to aircraft_prov.source_documents is added in Phase 14
    -- (ALTER TABLE ... ADD CONSTRAINT fk_vp_source_document) because that table
    -- does not exist yet at Phase 9 migration time. Declaring the FK inline here
    -- would fail with "relation aircraft_prov.source_documents does not exist".
    source_document_id BIGINT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (variant_id, engine_variant_id),
    CONSTRAINT chk_vp_engine_count CHECK (engine_count >= 1),
    CONSTRAINT chk_vp_option_flags CHECK (is_standard OR is_optional),
    CONSTRAINT chk_vp_primary_implies CHECK (
        NOT is_primary OR (is_standard OR is_optional)
    )
);

COMMENT ON TABLE aircraft_power.variant_powerplants IS
    'M:N junction between aircraft variants and engine variants. '
    'Supports multiple engine options per variant and multiple installed '
    'engines per option (engine_count). '
    'is_standard / is_optional classify the option type; '
    'is_primary designates the single engine used for comparison metrics. '
    'source_document_id links back to the Phase 14 provenance document.';
COMMENT ON COLUMN aircraft_power.variant_powerplants.engine_count IS
    'Number of installed engines of this engine_variant type. '
    'For a twin with identical engines: engine_count = 2. '
    'For a tandem helicopter with different power sections: two rows, each '
    'engine_count = 1.';
COMMENT ON COLUMN aircraft_power.variant_powerplants.source_document_id IS
    'FK to aircraft_prov.source_documents. Records which source established '
    'this powerplant link. SET NULL on source document deletion.';

-- =============================================================================
-- aircraft_power.variant_propellers
-- =============================================================================

CREATE TABLE aircraft_power.variant_propellers (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id          BIGINT NOT NULL
                            REFERENCES aircraft_core.variants(id)          ON DELETE CASCADE,
    propeller_spec_id   BIGINT NOT NULL
                            REFERENCES aircraft_power.propeller_specs(id)  ON DELETE RESTRICT,
    is_standard         BOOLEAN NOT NULL DEFAULT FALSE,
    is_optional         BOOLEAN NOT NULL DEFAULT FALSE,
    is_primary          BOOLEAN NOT NULL DEFAULT FALSE,
    notes               TEXT,
    UNIQUE (variant_id, propeller_spec_id),
    CONSTRAINT chk_vpr_option_flags CHECK (is_standard OR is_optional)
);

COMMENT ON TABLE aircraft_power.variant_propellers IS
    'M:N junction between variants and propeller specifications. '
    'Allows multiple propeller options (standard / optional / STC). '
    'One is_primary propeller per variant (partial UNIQUE index).';

-- =============================================================================
-- aircraft_power.rotor_systems
-- =============================================================================

CREATE TABLE aircraft_power.rotor_systems (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id     BIGINT NOT NULL
                       REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    rotor_role     TEXT NOT NULL,
    blade_count    SMALLINT,
    diameter_ft    NUMERIC,
    rotor_rpm      NUMERIC,
    disc_area_sqft NUMERIC,
    notes          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_rs_role CHECK (
        rotor_role IN (
            'MAIN', 'TAIL', 'TANDEM_FORWARD', 'TANDEM_AFT', 'COAXIAL', 'PROPROTOR'
        )
    ),
    CONSTRAINT chk_rs_dims CHECK (
        (diameter_ft IS NULL OR diameter_ft > 0) AND
        (blade_count IS NULL OR blade_count >= 2) AND
        (rotor_rpm   IS NULL OR rotor_rpm   > 0)
    )
);

COMMENT ON TABLE aircraft_power.rotor_systems IS
    'Per-rotor specifications for rotary-wing aircraft. '
    'A conventional helicopter has two rows: MAIN and TAIL.';

-- =============================================================================
-- aircraft_power.apu_specs
-- =============================================================================

CREATE TABLE aircraft_power.apu_specs (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id           BIGINT NOT NULL UNIQUE
                             REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    manufacturer_org_id  BIGINT
                             REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    manufacturer_name_raw TEXT,
    model_designation    TEXT,
    output_kva           NUMERIC,
    bleed_air_output_ppm NUMERIC,
    fuel_type_code       aircraft_ref.lookup_code
                             REFERENCES aircraft_ref.fuel_types(code),
    notes                TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_power.apu_specs IS
    'Auxiliary Power Unit specifications. 1:1 with variants (UNIQUE on variant_id). '
    'Absence of a row means the variant has no APU.';

-- =============================================================================
-- aircraft_power.powerplant_stcs
-- =============================================================================

CREATE TABLE aircraft_power.powerplant_stcs (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id               BIGINT NOT NULL
                                 REFERENCES aircraft_core.variants(id)        ON DELETE CASCADE,
    replacement_engine_id    BIGINT
                                 REFERENCES aircraft_power.engine_variants(id) ON DELETE SET NULL,
    replacement_engine_raw   TEXT,
    replaced_engine_name_raw TEXT,
    stc_number               TEXT,
    stc_holder_org_id        BIGINT
                                 REFERENCES aircraft_org.organizations(id)     ON DELETE SET NULL,
    stc_holder_name_raw      TEXT,
    authority_code           aircraft_ref.lookup_code
                                 REFERENCES aircraft_ref.certification_authorities(code),
    approval_date            DATE,
    stc_url                  TEXT,
    is_active                BOOLEAN NOT NULL DEFAULT TRUE,
    notes                    TEXT,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_power.powerplant_stcs IS
    'Supplemental Type Certificate records for engine conversions and replacements. '
    'replacement_engine_raw / stc_holder_name_raw are free-text fallbacks '
    'when the entity is not yet in our organization/engine catalogs.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_ev_updated
    BEFORE UPDATE ON aircraft_power.engine_variants
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- This trigger now works correctly: updated_at column exists on the table.
CREATE TRIGGER trg_vp_updated
    BEFORE UPDATE ON aircraft_power.variant_powerplants
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_power.engine_variants ───────────────────────────────────────────

CREATE UNIQUE INDEX uq_engine_variant_dedup
    ON aircraft_power.engine_variants (manufacturer_org_id, model_designation);

CREATE UNIQUE INDEX uq_engine_variant_raw_dedup
    ON aircraft_power.engine_variants (manufacturer_name_raw, model_designation)
    WHERE manufacturer_org_id IS NULL AND manufacturer_name_raw IS NOT NULL;

CREATE INDEX idx_ev_hp
    ON aircraft_power.engine_variants (hp_rated)
    WHERE hp_rated IS NOT NULL;

CREATE INDEX idx_ev_thrust_lbf
    ON aircraft_power.engine_variants (thrust_lbf_dry)
    WHERE thrust_lbf_dry IS NOT NULL;

CREATE INDEX idx_ev_thrust_n
    ON aircraft_power.engine_variants (rated_thrust_n)
    WHERE rated_thrust_n IS NOT NULL;

CREATE INDEX idx_ev_propulsion
    ON aircraft_power.engine_variants (propulsion_category_code);

CREATE INDEX idx_ev_fuel
    ON aircraft_power.engine_variants (fuel_type_code);

CREATE INDEX idx_ev_model_trgm
    ON aircraft_power.engine_variants USING gin (model_designation gin_trgm_ops);

CREATE INDEX idx_ev_aliases
    ON aircraft_power.engine_variants USING gin (name_aliases)
    WHERE name_aliases IS NOT NULL;

-- ── aircraft_power.variant_powerplants ───────────────────────────────────────

CREATE INDEX idx_vp_engine
    ON aircraft_power.variant_powerplants (engine_variant_id);

-- At most one primary powerplant per variant.
CREATE UNIQUE INDEX uq_vp_primary
    ON aircraft_power.variant_powerplants (variant_id)
    WHERE is_primary;

-- At most one STANDARD engine per variant.
-- Prevents two different engine models from simultaneously being marked as the
-- factory-standard installation on the same variant.
CREATE UNIQUE INDEX uq_vp_standard_per_variant
    ON aircraft_power.variant_powerplants (variant_id)
    WHERE is_standard;

CREATE INDEX idx_vp_source_document
    ON aircraft_power.variant_powerplants (source_document_id)
    WHERE source_document_id IS NOT NULL;

-- ── aircraft_power.propeller_specs ───────────────────────────────────────────

CREATE UNIQUE INDEX uq_prop_dedup
    ON aircraft_power.propeller_specs (manufacturer_org_id, model_designation);

CREATE INDEX idx_ps_type
    ON aircraft_power.propeller_specs (prop_type)
    WHERE prop_type IS NOT NULL;

-- ── aircraft_power.variant_propellers ────────────────────────────────────────

CREATE INDEX idx_vpr_prop
    ON aircraft_power.variant_propellers (propeller_spec_id);

CREATE UNIQUE INDEX uq_vpr_primary
    ON aircraft_power.variant_propellers (variant_id)
    WHERE is_primary;

-- ── aircraft_power.rotor_systems ─────────────────────────────────────────────

CREATE INDEX idx_rs_variant
    ON aircraft_power.rotor_systems (variant_id, rotor_role);

-- ── aircraft_power.powerplant_stcs ───────────────────────────────────────────

CREATE INDEX idx_pstc_variant
    ON aircraft_power.powerplant_stcs (variant_id);

CREATE INDEX idx_pstc_engine
    ON aircraft_power.powerplant_stcs (replacement_engine_id)
    WHERE replacement_engine_id IS NOT NULL;

CREATE INDEX idx_pstc_stc_number
    ON aircraft_power.powerplant_stcs (stc_number)
    WHERE stc_number IS NOT NULL;

COMMIT;