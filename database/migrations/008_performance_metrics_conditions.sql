-- =============================================================================
-- File: database/migrations/008_performance_metrics_conditions.sql
-- Phase 8 — aircraft_specs: performance metrics with test conditions,
-- runway limitations.
--
-- Design departure from Phases 6–7:
--   performance_metrics does NOT carry a general UNIQUE constraint per
--   (variant, metric_type). Multiple rows per metric type are expected —
--   one from each source and one for each test condition set.
--   The is_canonical flag (partial UNIQUE index) marks the single row per
--   (variant, metric_type) used for cross-fleet comparison queries.
--
-- Spec coverage (requirement 3):
--   max/cruise/stall/Mach speeds     → performance_metrics + metric types
--   V-speeds (Vx,Vy,Va,Vno,Vfe…)    → metric types seeded in Phase 2
--   climb metrics                    → performance_metrics
--   ceilings, range, endurance       → performance_metrics
--   runway distances                 → performance_metrics + runway_limitations
--   test conditions / atmosphere     → condition_* columns
--   runway surface limitations       → runway_limitations.approved_surfaces
--   hot-and-high, density-altitude   → runway_limitations.*_notes
-- =============================================================================

BEGIN;


-- =============================================================================
-- aircraft_specs.performance_metrics
-- Metric-fact table for all named performance measurements.
-- Multiple rows per (variant, metric_type) are allowed and expected:
--   - same metric reported by different sources with different values
--   - same metric measured at different altitude / weight / power conditions
-- is_canonical = TRUE marks the single row per (variant, metric_type) used
-- for cross-fleet comparison queries and Phase 16 read models.
--
-- Test condition columns capture the measurement assumptions; all are nullable
-- because sources vary in how much condition detail they publish. Free-text
-- conditions_notes captures any remaining context that doesn't fit columns.
--
-- condition_power_setting and condition_surface_type use TEXT CHECK (small,
-- stable, definitionally complete sets). Other condition columns are free
-- NUMERIC or TEXT to accommodate all published source formats.
-- =============================================================================

CREATE TABLE aircraft_specs.performance_metrics (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id       BIGINT NOT NULL
                         REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    metric_type_code aircraft_ref.lookup_code NOT NULL
                         REFERENCES aircraft_ref.performance_metric_types(code),

    -- ── Source-fidelity columns ───────────────────────────────────────────────
    raw_value        NUMERIC,
    raw_unit_code    aircraft_ref.lookup_code
                         REFERENCES aircraft_ref.measurement_units(code),

    -- ── Canonical comparison column ───────────────────────────────────────────
    -- Unit determined by performance_metric_types.canonical_unit_code.
    -- e.g., KNOTS for speeds, FT for ceilings, NM for range, GPH for fuel burn.
    -- Populated by: aircraft_ref.to_canonical(raw_value, raw_unit_code).
    canonical_value  NUMERIC,

    -- ── Aircraft configuration when measured ─────────────────────────────────
    -- Descriptive label for the aircraft state during the test.
    -- Common values: 'CLEAN', 'FLAPS_10', 'FLAPS_APPROACH', 'LANDING_CONFIG',
    --   'GEAR_UP', 'GEAR_DOWN', 'WITH_EXTERNAL_TANKS', 'FERRY_CONFIG'.
    configuration    TEXT,

    -- ── Test condition columns ────────────────────────────────────────────────
    -- Altitude at which the metric was measured or calculated.
    condition_altitude_ft   NUMERIC,
    -- Aircraft gross weight at test conditions (lbs).
    condition_weight_lbs    NUMERIC,
    -- Human-readable weight label (e.g., 'MTOW', 'OEW', 'HALF_FUEL').
    -- Stored separately so 'MTOW' is searchable without knowing the exact lbs.
    condition_weight_label  TEXT,
    -- ISA temperature deviation in °C. 0 = standard ISA; +20 = ISA+20°C.
    condition_isa_dev_c     NUMERIC,
    -- Engine power or thrust setting during the test.
    condition_power_setting TEXT,
    -- Surface type for ground roll / landing distance metrics.
    condition_surface_type  TEXT,
    -- Any additional conditions not captured by the columns above.
    conditions_notes        TEXT,

    -- ── Curation / quality ───────────────────────────────────────────────────
    -- TRUE = this row is the designated value for cross-fleet comparison.
    -- At most one is_canonical = TRUE per (variant, metric_type).
    -- Enforced by partial UNIQUE index uq_perf_canonical.
    is_canonical    BOOLEAN NOT NULL DEFAULT FALSE,
    is_estimated    BOOLEAN NOT NULL DEFAULT FALSE,
    confidence      aircraft_ref.confidence_score,
    source_notes    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT chk_pm_canonical_nonneg CHECK (
        canonical_value IS NULL OR canonical_value >= 0
    ),
    CONSTRAINT chk_pm_power_setting CHECK (
        condition_power_setting IS NULL
        OR condition_power_setting IN (
            'MAX_TAKEOFF',      -- maximum takeoff power/thrust
            'MAX_CONTINUOUS',   -- maximum continuous power
            'MAX_CLIMB',        -- maximum climb power
            '75_PCT',           -- 75% power/thrust
            '65_PCT',           -- 65% power/thrust
            '55_PCT',           -- 55% power/thrust
            'BEST_POWER',       -- best-power fuel mixture
            'BEST_ECONOMY',     -- best-economy (lean of peak)
            'LONG_RANGE_CRUISE',-- long-range cruise power setting
            'IDLE'              -- idle / flight idle
        )
    ),
    CONSTRAINT chk_pm_surface_type CHECK (
        condition_surface_type IS NULL
        OR condition_surface_type IN (
            'PAVED',        -- hard paved surface (asphalt / concrete)
            'GRASS',        -- mowed grass strip
            'GRAVEL',       -- gravel / packed aggregate
            'SOFT',         -- soft or unprepared surface
            'WATER',        -- water surface (seaplane / amphibian)
            'CARRIER_DECK'  -- aircraft carrier flight deck
        )
    )
);

COMMENT ON TABLE aircraft_specs.performance_metrics IS
    'Metric-fact table for aircraft performance data with full test-condition context. '
    'Multiple rows per (variant, metric_type) are normal: '
    'different sources report different values; the same metric is valid at '
    'different altitudes, weights, and power settings. '
    'is_canonical = TRUE marks the single designated comparison value per '
    '(variant, metric_type), enforced by the partial UNIQUE index. '
    'Phase 16 comparison queries filter WHERE is_canonical for efficiency.';
COMMENT ON COLUMN aircraft_specs.performance_metrics.is_canonical IS
    'TRUE = this row is the designated cross-fleet comparison value '
    'for this (variant, metric_type) pair. At most one TRUE per pair '
    '(enforced by uq_perf_canonical partial UNIQUE index). '
    'Phase 17 ingestion sets is_canonical = TRUE for the first value; '
    'curators resolve conflicts from additional sources.';
COMMENT ON COLUMN aircraft_specs.performance_metrics.condition_power_setting IS
    'Engine power / thrust setting at test conditions. TEXT CHECK '
    '(10 stable values). NULL when power setting is not published by the source.';
COMMENT ON COLUMN aircraft_specs.performance_metrics.condition_surface_type IS
    'Runway or water surface type for takeoff/landing distance metrics. '
    'NULL for airborne metrics (speeds, ceilings, range). '
    'TEXT CHECK (6 stable surface type values).';
COMMENT ON COLUMN aircraft_specs.performance_metrics.configuration IS
    'Descriptive aircraft configuration at time of measurement. '
    'Common values: ''CLEAN'', ''FLAPS_APPROACH'', ''LANDING_CONFIG'', ''GEAR_DOWN''. '
    'For V-speeds, identifies the specific flap/gear state used in the test.';

-- =============================================================================
-- aircraft_specs.runway_limitations
-- 1:1 extension of aircraft_core.variants (UNIQUE on variant_id).
-- Captures approved surface types, crosswind limits, and the qualitative
-- hot-and-high / density-altitude / unpaved-surface notes from the POH.
-- These are narrative/configuration constraints that don't fit the
-- performance_metrics numeric fact model.
-- approved_surfaces TEXT[] enables: WHERE approved_surfaces @> ARRAY['GRASS']
-- =============================================================================

CREATE TABLE aircraft_specs.runway_limitations (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id            BIGINT NOT NULL UNIQUE
                              REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    -- Array of approved surface types. Values mirror condition_surface_type
    -- on performance_metrics: 'PAVED','GRASS','GRAVEL','SOFT','WATER','CARRIER_DECK'.
    -- NULL = not yet curated.
    approved_surfaces     TEXT[],
    -- Maximum demonstrated crosswind component in knots (from POH Section 5).
    max_crosswind_ktas    NUMERIC,
    -- Minimum runway length (ft) for operations at MTOW, sea level, ISA.
    min_runway_length_ft  NUMERIC,
    -- Free-text notes from the POH on hot-and-high performance degradation.
    hot_high_notes        TEXT,
    -- Free-text density altitude performance notes.
    density_alt_notes     TEXT,
    -- Soft-field, grass, or unpaved surface operational notes.
    unpaved_surface_notes TEXT,
    confidence            aircraft_ref.confidence_score,
    notes                 TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_rl_crosswind_nonneg CHECK (
        max_crosswind_ktas IS NULL OR max_crosswind_ktas >= 0
    ),
    CONSTRAINT chk_rl_runway_length CHECK (
        min_runway_length_ft IS NULL OR min_runway_length_ft > 0
    )
);

COMMENT ON TABLE aircraft_specs.runway_limitations IS
    '1:1 extension of aircraft_core.variants for runway and surface limitations. '
    'approved_surfaces (TEXT array) lists surface types certified for operations; '
    'GIN-indexed for efficient "find all aircraft approved for grass" queries. '
    'Qualitative POH notes (hot-high, density altitude, soft-field) are stored '
    'as free-text because they do not reduce to a single numeric comparand.';
COMMENT ON COLUMN aircraft_specs.runway_limitations.approved_surfaces IS
    'Array of approved surface type codes. Valid values mirror '
    'performance_metrics.condition_surface_type: '
    '''PAVED'',''GRASS'',''GRAVEL'',''SOFT'',''WATER'',''CARRIER_DECK''. '
    'Query: WHERE approved_surfaces @> ARRAY[''GRASS''] '
    'finds variants approved for grass-strip operations.';
COMMENT ON COLUMN aircraft_specs.runway_limitations.max_crosswind_ktas IS
    'Maximum demonstrated crosswind component in knots from the POH. '
    '"Demonstrated" is a flight-test figure, not a certificated limit, '
    'unless the POH states otherwise. Source context is in notes.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_perf_metrics_updated
    BEFORE UPDATE ON aircraft_specs.performance_metrics
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_runway_lim_updated
    BEFORE UPDATE ON aircraft_specs.runway_limitations
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_specs.performance_metrics ───────────────────────────────────────

-- PRIMARY COMPARISON INDEX
-- Enforces at most one canonical value per (variant, metric_type).
-- Phase 16 comparison views filter exclusively on this index.
CREATE UNIQUE INDEX uq_perf_canonical
    ON aircraft_specs.performance_metrics (variant_id, metric_type_code)
    WHERE is_canonical;

-- CROSS-FLEET SORT INDEX
-- Covers the most frequent comparison query:
--   "sort all variants by metric X, return canonical value"
-- Covering index (metric_type, canonical_value) avoids table-heap access.
CREATE INDEX idx_pm_type_canonical
    ON aircraft_specs.performance_metrics (metric_type_code, canonical_value)
    WHERE is_canonical AND canonical_value IS NOT NULL;

-- VARIANT DETAIL PAGE
-- All performance metrics for one variant (includes non-canonical rows for
-- display of multiple source values and conditions context).
CREATE INDEX idx_pm_variant
    ON aircraft_specs.performance_metrics (variant_id);

-- SPECIFIC METRIC FOR ONE VARIANT
-- Get all recorded values (all sources, all conditions) for one metric on one variant.
CREATE INDEX idx_pm_variant_type
    ON aircraft_specs.performance_metrics (variant_id, metric_type_code);

-- ALTITUDE CONDITION FILTER
-- "Show only metrics measured at or above 10,000 ft" (high-altitude aircraft research).
CREATE INDEX idx_pm_altitude
    ON aircraft_specs.performance_metrics (condition_altitude_ft)
    WHERE condition_altitude_ft IS NOT NULL;

-- ── aircraft_specs.runway_limitations ────────────────────────────────────────

-- GIN on approved_surfaces for containment queries (@>)
CREATE INDEX idx_rl_surfaces
    ON aircraft_specs.runway_limitations USING gin (approved_surfaces)
    WHERE approved_surfaces IS NOT NULL;

-- Crosswind filter — buyer search "I need > 20 kt demonstrated crosswind"
CREATE INDEX idx_rl_crosswind
    ON aircraft_specs.runway_limitations (max_crosswind_ktas)
    WHERE max_crosswind_ktas IS NOT NULL;

COMMIT;