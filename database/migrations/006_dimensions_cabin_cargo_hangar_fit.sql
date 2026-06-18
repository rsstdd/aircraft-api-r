-- =============================================================================
-- File: database/migrations/006_dimensions_cabin_cargo_hangar_fit.sql
-- Phase 6 — aircraft_specs: dimensional data, cabin configuration, and
-- cargo hold specifications.
--
-- Design pattern shared by Phases 6, 7, and 8:
--   metric-fact table — one row per (variant, metric_type, configuration)
--   raw_value + raw_unit_code  → preserves source fidelity
--   canonical_value            → computed via aircraft_ref.to_canonical()
--                                for cross-fleet numeric comparison
--
-- aircraft_ref.to_canonical() is introduced here as a cross-cutting helper;
-- it is re-used identically by Phases 7 and 8.
--
-- Spec coverage (requirement 5):
--   wingspan, length, height, wing area, aspect ratio  → dimension_metrics
--   cabin dimensions                                   → dimension_metrics + cabin_specs
--   baggage volume, cargo volume                       → dimension_metrics + cargo_holds
--   cargo door dimensions                              → cargo_holds
--   rotor diameter, propeller diameter                 → dimension_metrics
--   folded dimensions (wing fold)                      → dimension_metrics (configuration='WINGS_FOLDED')
--   wheelbase, track width                             → dimension_metrics
--   hangar-fit constraints                             → derived from dimension_metrics in Phase 16 view
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_ref.to_canonical(raw_value, unit_code)
-- Cross-cutting helper: converts a raw source value + unit code to the
-- canonical value using the conversion factors seeded in measurement_units.
-- Returns NULL for NULL input.
-- Returns raw_value unchanged when the unit is already canonical
-- (canonical_unit_code IS NULL on the measurement_units row).
-- Used by Phase 17 ingestion and by curation tooling to populate
-- canonical_value columns in Phases 6, 7, and 8.
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_ref.to_canonical(
    p_raw_value  NUMERIC,
    p_unit_code  aircraft_ref.lookup_code
)
RETURNS NUMERIC
LANGUAGE sql
STABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT CASE
        WHEN mu.canonical_unit_code IS NULL THEN p_raw_value   -- unit is already canonical
        ELSE p_raw_value * mu.canonical_factor
    END
    FROM aircraft_ref.measurement_units mu
    WHERE mu.code = p_unit_code;
$$;

COMMENT ON FUNCTION aircraft_ref.to_canonical(NUMERIC, aircraft_ref.lookup_code) IS
    'Convert a raw source value to the canonical unit for its measurement category. '
    'Returns NULL for NULL input (RETURNS NULL ON NULL INPUT). '
    'Returns raw_value unchanged when the unit is already canonical '
    '(measurement_units.canonical_unit_code IS NULL). '
    'Returns NULL when p_unit_code does not match any measurement_units row. '
    'Usage: aircraft_ref.to_canonical(39.2, ''FT'') → 39.2 (FT is canonical for ALTITUDE). '
    '       aircraft_ref.to_canonical(11.9, ''METERS'') → 39.04 (METERS → FT).';

-- =============================================================================
-- aircraft_specs.dimension_metrics
-- Metric-fact table: one row per (variant, metric_type, configuration).
-- Covers all 17 DIM_* types seeded in aircraft_ref.dimension_metric_types
-- (Phase 2): wingspan, length, height, wing area, aspect ratio,
-- rotor/prop diameter, cabin dimensions, baggage/cargo volumes,
-- cargo door dimensions, wheelbase, track width, and folded wingspan.
--
-- The UNIQUE functional index allows multiple rows for the same metric
-- when the measurement differs by configuration (e.g., DIM_WINGSPAN at
-- 'WINGS_EXTENDED' vs 'WINGS_FOLDED'). NULL configuration = standard.
-- =============================================================================

CREATE TABLE aircraft_specs.dimension_metrics (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id       BIGINT NOT NULL
                         REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    metric_type_code aircraft_ref.lookup_code NOT NULL
                         REFERENCES aircraft_ref.dimension_metric_types(code),

    -- ── Source-fidelity columns ───────────────────────────────────────────────
    -- Preserve the raw value and unit exactly as found in the source.
    raw_value        NUMERIC,
    raw_unit_code    aircraft_ref.lookup_code
                         REFERENCES aircraft_ref.measurement_units(code),

    -- ── Canonical comparison column ───────────────────────────────────────────
    -- Populated by: aircraft_ref.to_canonical(raw_value, raw_unit_code)
    -- Unit is determined by dimension_metric_types.canonical_unit_code.
    -- For most dimension metrics: FT (linear), SQ_FT (area), CU_FT (volume).
    canonical_value  NUMERIC,

    -- ── Configuration discriminator ───────────────────────────────────────────
    -- NULL = standard / default configuration.
    -- Named variants: 'WINGS_FOLDED', 'GEAR_DOWN', 'GEAR_UP',
    --   'WITH_TIP_TANKS', 'FULL_FUEL', 'EMPTY'.
    configuration    TEXT,

    -- ── Metadata ─────────────────────────────────────────────────────────────
    is_estimated     BOOLEAN               NOT NULL DEFAULT FALSE,
    confidence       aircraft_ref.confidence_score,
    source_notes     TEXT,
    created_at       TIMESTAMPTZ           NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ           NOT NULL DEFAULT now(),

    CONSTRAINT chk_dm_canonical_nonneg CHECK (
        canonical_value IS NULL OR canonical_value >= 0
    ),
    CONSTRAINT chk_dm_raw_nonneg CHECK (
        raw_value IS NULL OR raw_value >= 0
    )
);

COMMENT ON TABLE aircraft_specs.dimension_metrics IS
    'Metric-fact table for all aircraft dimensional data. '
    'One row per (variant, metric_type, configuration). '
    'raw_value / raw_unit_code preserve source fidelity; '
    'canonical_value enables cross-fleet numeric comparison. '
    'Covers wingspan, length, height, wing area, rotor/prop diameter, '
    'cabin dimensions, baggage/cargo volumes, cargo door sizes, wheelbase, '
    'track width, and folded dimensions. '
    'Hangar-fit analysis is derived from these rows in a Phase 16 view.';
COMMENT ON COLUMN aircraft_specs.dimension_metrics.canonical_value IS
    'Value in the canonical unit for this metric type '
    '(from aircraft_ref.dimension_metric_types.canonical_unit_code). '
    'For DIM_WINGSPAN: feet. For DIM_BAGGAGE_VOLUME: cubic feet. '
    'Computed by aircraft_ref.to_canonical(raw_value, raw_unit_code) '
    'during Phase 17 ingestion. NULL = not yet computed or not available.';
COMMENT ON COLUMN aircraft_specs.dimension_metrics.configuration IS
    'Optional discriminator for configuration-specific measurements. '
    'NULL = standard/default. Common values: ''WINGS_FOLDED'', ''GEAR_DOWN'', '
    '''WITH_TIP_TANKS''. Drives the functional UNIQUE index that allows '
    'multiple rows per metric type per variant when configurations differ.';
COMMENT ON COLUMN aircraft_specs.dimension_metrics.is_estimated IS
    'TRUE when canonical_value is approximate, extrapolated, or derived '
    'from related data rather than directly measured from a primary source.';

-- =============================================================================
-- aircraft_specs.cabin_specs
-- Structured cabin configuration data per variant.
-- Supports both simple GA aircraft (one row, STANDARD config, 4 seats)
-- and complex airliners (multiple rows for mixed-class configurations).
-- Partial UNIQUE index enforces at most one is_primary_config row per variant.
-- =============================================================================

CREATE TABLE aircraft_specs.cabin_specs (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id            BIGINT NOT NULL
                              REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    -- Label for this configuration (e.g., 'STANDARD', 'HIGH_DENSITY',
    -- 'EXECUTIVE', 'MEDICAL', 'CARGO_CONVERT').
    config_label          TEXT NOT NULL DEFAULT 'STANDARD',

    -- ── Seating ──────────────────────────────────────────────────────────────
    total_seat_count      SMALLINT,   -- all seats including crew jumpseats
    passenger_seat_count  SMALLINT,   -- revenue / usable passenger seats
    -- Seating layout code (e.g., '2+2', '3+3+3', '2+3+2', '1+1').
    seat_configuration    TEXT,
    -- Seat pitch in inches (floor-to-floor row spacing).
    seat_pitch_in         NUMERIC,
    -- Seat width in inches (armrest-to-armrest).
    seat_width_in         NUMERIC,

    -- ── Cabin features ───────────────────────────────────────────────────────
    has_aisle             BOOLEAN,
    aisle_width_in        NUMERIC,
    lavatory_count        SMALLINT,
    galley_count          SMALLINT,
    is_pressurized        BOOLEAN,

    -- ── Identification ────────────────────────────────────────────────────────
    -- TRUE for the most commonly referenced or baseline configuration.
    is_primary_config     BOOLEAN      NOT NULL DEFAULT FALSE,
    confidence            aircraft_ref.confidence_score,
    notes                 TEXT,
    created_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),

    UNIQUE (variant_id, config_label),
    CONSTRAINT chk_cs_seats_nonneg CHECK (
        (total_seat_count     IS NULL OR total_seat_count     >= 0)
        AND (passenger_seat_count IS NULL OR passenger_seat_count >= 0)
        AND (lavatory_count   IS NULL OR lavatory_count   >= 0)
        AND (galley_count     IS NULL OR galley_count     >= 0)
    ),
    CONSTRAINT chk_cs_pitch_positive CHECK (
        seat_pitch_in IS NULL OR seat_pitch_in > 0
    ),
    CONSTRAINT chk_cs_width_positive CHECK (
        seat_width_in IS NULL OR seat_width_in > 0
    )
);

COMMENT ON TABLE aircraft_specs.cabin_specs IS
    'Cabin configuration data per variant. '
    'One row per named configuration (STANDARD, HIGH_DENSITY, EXECUTIVE). '
    'For GA aircraft: typically one STANDARD row with passenger_seat_count 1–6. '
    'For airliners: multiple rows for mixed-class configs '
    '(e.g., BUSINESS+ECONOMY, ALL_ECONOMY). '
    'is_primary_config marks the most commonly referenced layout.';
COMMENT ON COLUMN aircraft_specs.cabin_specs.seat_configuration IS
    'Abreast seating layout code (e.g., "2+2", "3+3+3", "2+3+2"). '
    'Free-text; not FK-constrained as airline configurations vary widely.';
COMMENT ON COLUMN aircraft_specs.cabin_specs.seat_pitch_in IS
    'Seat pitch in inches (row-to-row distance). '
    'Industry standard reference: economy long-haul typically 30–34 in; '
    'business class 60–80 in; GA aircraft not applicable.';

-- =============================================================================
-- aircraft_specs.cargo_holds
-- Structured cargo hold data for each discrete cargo compartment.
-- Covers multi-hold airliners (FWD + AFT + BULK), dedicated freighters
-- (main deck), and rotorcraft (external sling, internal).
-- For GA aircraft, baggage compartment volume is captured in
-- dimension_metrics (DIM_BAGGAGE_VOLUME) rather than here.
-- hold_position uses a TEXT CHECK (8 values, definitionally stable).
-- =============================================================================

CREATE TABLE aircraft_specs.cargo_holds (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id            BIGINT NOT NULL
                              REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    -- Positional label of the hold within the aircraft.
    hold_position         TEXT NOT NULL,
    -- Operator/manufacturer label (e.g., 'FWD Hold 1', 'Bulk Compartment').
    hold_label            TEXT,

    -- ── Volume ───────────────────────────────────────────────────────────────
    volume_raw_value      NUMERIC,
    volume_raw_unit_code  aircraft_ref.lookup_code
                              REFERENCES aircraft_ref.measurement_units(code),
    -- Canonical volume in cubic feet (CU_FT).
    volume_canonical_ft3  NUMERIC,

    -- ── Internal envelope (informational; not always published) ───────────────
    floor_length_ft       NUMERIC,
    max_width_ft          NUMERIC,
    max_height_ft         NUMERIC,

    -- ── Structural limits ────────────────────────────────────────────────────
    -- Maximum payload weight for this specific hold.
    max_payload_lbs       NUMERIC,
    -- Maximum weight per unit area of floor (psf).
    floor_loading_psf     NUMERIC,

    -- ── Access ───────────────────────────────────────────────────────────────
    cargo_door_width_ft   NUMERIC,
    cargo_door_height_ft  NUMERIC,

    -- ── Environment ──────────────────────────────────────────────────────────
    is_pressurized        BOOLEAN,
    is_heated             BOOLEAN,

    confidence            aircraft_ref.confidence_score,
    notes                 TEXT,
    created_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),

    CONSTRAINT chk_ch_position CHECK (
        hold_position IN (
            'FORWARD',      -- forward underfloor hold (narrowbody/widebody)
            'AFT',          -- aft underfloor hold
            'BULK',         -- bulk (loose-load) aft compartment
            'MAIN_DECK',    -- main cabin floor (freighters, combi)
            'NOSE',         -- nose section hold (some older types)
            'UNDERWING',    -- underwing pod (some turboprops)
            'UNDERFLOOR',   -- generic underfloor (when FWD/AFT not distinguished)
            'EXTERNAL_SLING'-- helicopter external sling load position
        )
    ),
    CONSTRAINT chk_ch_volume_nonneg CHECK (
        (volume_raw_value      IS NULL OR volume_raw_value      >= 0)
        AND (volume_canonical_ft3 IS NULL OR volume_canonical_ft3 >= 0)
        AND (floor_length_ft   IS NULL OR floor_length_ft   >  0)
        AND (max_width_ft      IS NULL OR max_width_ft      >  0)
        AND (max_height_ft     IS NULL OR max_height_ft     >  0)
        AND (max_payload_lbs   IS NULL OR max_payload_lbs   >= 0)
        AND (floor_loading_psf IS NULL OR floor_loading_psf >= 0)
    ),
    CONSTRAINT chk_ch_door_dims CHECK (
        (cargo_door_width_ft  IS NULL OR cargo_door_width_ft  > 0)
        AND (cargo_door_height_ft IS NULL OR cargo_door_height_ft > 0)
    )
);

COMMENT ON TABLE aircraft_specs.cargo_holds IS
    'Per-compartment cargo hold specifications. '
    'Multi-hold aircraft (FWD + AFT + BULK) have one row per hold. '
    'Freighters use MAIN_DECK; helicopters use EXTERNAL_SLING. '
    'GA baggage compartment volume is captured in dimension_metrics '
    '(DIM_BAGGAGE_VOLUME) rather than here. '
    'hold_position TEXT CHECK (8 values) is definitionally stable '
    'and non-extensible by design.';
COMMENT ON COLUMN aircraft_specs.cargo_holds.volume_canonical_ft3 IS
    'Hold volume in cubic feet (canonical unit for VOLUME metrics). '
    'Computed by aircraft_ref.to_canonical(volume_raw_value, volume_raw_unit_code). '
    'Enables total-hold-volume comparison across variants.';
COMMENT ON COLUMN aircraft_specs.cargo_holds.floor_loading_psf IS
    'Structural floor loading limit in pounds per square foot. '
    'Critical for freighter comparison; determines compatibility with '
    'standard ULD (Unit Load Device) configurations.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_dimension_metrics_updated
    BEFORE UPDATE ON aircraft_specs.dimension_metrics
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_cabin_specs_updated
    BEFORE UPDATE ON aircraft_specs.cabin_specs
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_specs.dimension_metrics ─────────────────────────────────────────

-- Functional UNIQUE: one row per (variant, metric_type, configuration).
-- COALESCE collapses NULL configuration with empty string so that
-- two NULL-configuration rows for the same (variant, type) are rejected.
CREATE UNIQUE INDEX uq_dimension_metrics_config
    ON aircraft_specs.dimension_metrics
        (variant_id, metric_type_code, COALESCE(configuration, ''));

-- "All wingspan values across all variants" — mission-profile comparison
-- Covering index includes canonical_value to avoid table fetch for sorting.
CREATE INDEX idx_dm_type_canonical
    ON aircraft_specs.dimension_metrics (metric_type_code, canonical_value)
    WHERE canonical_value IS NOT NULL;

-- "All dimensions for one variant" — detail page retrieval
CREATE INDEX idx_dm_variant
    ON aircraft_specs.dimension_metrics (variant_id);

-- ── aircraft_specs.cabin_specs ───────────────────────────────────────────────

-- Exactly one primary configuration per variant
CREATE UNIQUE INDEX uq_cabin_primary_config
    ON aircraft_specs.cabin_specs (variant_id)
    WHERE is_primary_config;

-- Passenger seat count filter — buyer search "I need 4+ seats"
CREATE INDEX idx_cabin_pax_count
    ON aircraft_specs.cabin_specs (passenger_seat_count)
    WHERE passenger_seat_count IS NOT NULL;

-- Pressurization filter
CREATE INDEX idx_cabin_pressurized
    ON aircraft_specs.cabin_specs (is_pressurized)
    WHERE is_pressurized IS NOT NULL;

-- ── aircraft_specs.cargo_holds ───────────────────────────────────────────────

-- All holds for a variant (detail page)
CREATE INDEX idx_ch_variant
    ON aircraft_specs.cargo_holds (variant_id);

-- All main-deck holds (freighter comparison)
CREATE INDEX idx_ch_main_deck
    ON aircraft_specs.cargo_holds (volume_canonical_ft3)
    WHERE hold_position = 'MAIN_DECK'
      AND volume_canonical_ft3 IS NOT NULL;

-- Total cargo volume summing query uses variant_id; covered above

COMMIT;