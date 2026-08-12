-- =============================================================================
-- File: database/migrations/007_weight_balance_payload_loading.sql
-- Phase 7 — aircraft_specs: weight and loading metrics, CG envelope data,
-- and payload-range trade-off curves.
--
-- Changes from original design (post-evaluation fixes):
--   • cg_envelopes: added validate_cg_envelope_points() trigger function and
--     BEFORE INSERT OR UPDATE trigger on cg_envelopes. The trigger validates
--     that fwd_limit_points and aft_limit_points, when non-NULL, are JSONB
--     arrays where every element has numeric 'w' and 'cg' keys, and that
--     the points are sorted in ascending order by 'w'. Without this, a
--     malformed JSONB array would insert silently and corrupt CG display
--     output without any error at write time.
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_specs.weight_metrics
-- =============================================================================

CREATE TABLE aircraft_specs.weight_metrics (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id       BIGINT NOT NULL
                         REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    metric_type_code aircraft_ref.lookup_code NOT NULL
                         REFERENCES aircraft_ref.weight_metric_types(code),
    raw_value        NUMERIC,
    raw_unit_code    aircraft_ref.lookup_code
                         REFERENCES aircraft_ref.measurement_units(code),
    canonical_value  NUMERIC,
    configuration    TEXT,
    is_estimated     BOOLEAN              NOT NULL DEFAULT FALSE,
    confidence       aircraft_ref.confidence_score,
    source_notes     TEXT,
    created_at       TIMESTAMPTZ          NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ          NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_specs.weight_metrics IS
    'Metric-fact table for all aircraft weight, loading, and balance data. '
    'One row per (variant, metric_type, configuration). '
    'Canonical unit: LBS for mass, US_GAL for fuel volume, '
    'no unit for dimensionless metrics (load factors, wing/power loading). '
    'Does not carry a non-negative CHECK because LOAD_FACTOR_NEG is legitimately negative.';
COMMENT ON COLUMN aircraft_specs.weight_metrics.canonical_value IS
    'Value in the metric-type canonical unit. For mass: LBS. For fuel: US_GAL. '
    'For dimensionless metrics (LOAD_FACTOR_*, WING_LOADING): equals raw_value. '
    'Populated by Phase 17 ingestion.';
COMMENT ON COLUMN aircraft_specs.weight_metrics.raw_unit_code IS
    'Source unit code. NULL for dimensionless metrics. '
    'Phase 17 sets canonical_value = raw_value when raw_unit_code IS NULL.';

-- =============================================================================
-- aircraft_specs.cg_envelopes
-- =============================================================================

CREATE TABLE aircraft_specs.cg_envelopes (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id        BIGINT NOT NULL
                          REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    config_label      TEXT,
    datum_description TEXT,
    cg_unit           TEXT NOT NULL DEFAULT 'INCHES_AFT_DATUM',
    -- JSONB arrays of {w: weight_lbs, cg: cg_value} objects.
    -- Structure validated by trg_validate_cg_envelope_points trigger below.
    fwd_limit_points  JSONB,
    aft_limit_points  JSONB,
    min_weight_lbs    NUMERIC,
    max_weight_lbs    NUMERIC,
    most_fwd_cg       NUMERIC,
    most_aft_cg       NUMERIC,
    is_primary        BOOLEAN    NOT NULL DEFAULT FALSE,
    confidence        aircraft_ref.confidence_score,
    notes             TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_cge_cg_unit CHECK (
        cg_unit IN (
            'INCHES_AFT_DATUM',
            'PERCENT_MAC',
            'MILLIMETERS_AFT_DATUM'
        )
    ),
    CONSTRAINT chk_cge_weights CHECK (
        max_weight_lbs IS NULL OR
        min_weight_lbs IS NULL OR
        max_weight_lbs >= min_weight_lbs
    )
);

COMMENT ON TABLE aircraft_specs.cg_envelopes IS
    'CG envelope data per variant and operating category. '
    'fwd_limit_points / aft_limit_points are JSONB arrays of '
    '{w: weight_lbs, cg: cg_value} pairs defining the approved boundary. '
    'Structure is validated at write time by trg_validate_cg_envelope_points. '
    'JSONB is used because point count varies (3-15) and the entire envelope '
    'is always read/written together. '
    'Pre-computed scalar extremes (most_fwd_cg, most_aft_cg) support quick '
    'validation queries without scanning the JSONB arrays.';
COMMENT ON COLUMN aircraft_specs.cg_envelopes.fwd_limit_points IS
    'Forward CG limit boundary. JSON array: [{"w":<lbs>,"cg":<value>}, ...]. '
    'Points sorted by ascending w. cg value in cg_unit. '
    'Validated by trigger: every element must have numeric w and cg keys, '
    'and points must be sorted ascending by w.';
COMMENT ON COLUMN aircraft_specs.cg_envelopes.aft_limit_points IS
    'Aft CG limit boundary. Same validated structure as fwd_limit_points.';

-- =============================================================================
-- CG envelope JSONB validation trigger
-- Validates fwd_limit_points and aft_limit_points before every INSERT/UPDATE.
-- Checks:
--   1. Value is a JSON array (not an object, scalar, or null — null is allowed
--      to mean "not yet populated").
--   2. Every element has a 'w' key with a numeric value >= 0.
--   3. Every element has a 'cg' key with a numeric value.
--   4. Points are sorted in strictly ascending order by w (required for linear
--      interpolation to work correctly).
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_specs.validate_cg_envelope_points()
RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_col       TEXT;
    v_points    JSONB;
    v_elem      JSONB;
    v_prev_w    NUMERIC;
    v_curr_w    NUMERIC;
    v_i         INT;
    v_len       INT;
BEGIN
    -- Validate both columns using a loop to avoid code duplication
    FOR v_col IN SELECT unnest(ARRAY['fwd_limit_points', 'aft_limit_points'])
    LOOP
        v_points := CASE v_col
            WHEN 'fwd_limit_points' THEN NEW.fwd_limit_points
            WHEN 'aft_limit_points' THEN NEW.aft_limit_points
        END;

        -- NULL is permitted (not yet populated)
        IF v_points IS NULL THEN CONTINUE; END IF;

        -- Must be a JSON array
        IF jsonb_typeof(v_points) <> 'array' THEN
            RAISE EXCEPTION
                'cg_envelopes.% must be a JSON array, got %',
                v_col, jsonb_typeof(v_points);
        END IF;

        v_len := jsonb_array_length(v_points);

        -- Empty array is valid (zero points = no data yet)
        IF v_len = 0 THEN CONTINUE; END IF;

        -- Minimum meaningful envelope requires at least 2 points
        IF v_len < 2 THEN
            RAISE EXCEPTION
                'cg_envelopes.% must have at least 2 points to define a boundary, got %',
                v_col, v_len;
        END IF;

        v_prev_w := NULL;

        FOR v_i IN 0 .. v_len - 1
        LOOP
            v_elem := v_points -> v_i;

            -- Each element must be an object
            IF jsonb_typeof(v_elem) <> 'object' THEN
                RAISE EXCEPTION
                    'cg_envelopes.%[%] must be a JSON object, got %',
                    v_col, v_i, jsonb_typeof(v_elem);
            END IF;

            -- Must have 'w' key with a numeric value
            IF v_elem -> 'w' IS NULL OR jsonb_typeof(v_elem -> 'w') <> 'number' THEN
                RAISE EXCEPTION
                    'cg_envelopes.%[%] missing or non-numeric "w" key. '
                    'Expected: {"w": <weight_lbs>, "cg": <cg_value>}',
                    v_col, v_i;
            END IF;

            -- Must have 'cg' key with a numeric value
            IF v_elem -> 'cg' IS NULL OR jsonb_typeof(v_elem -> 'cg') <> 'number' THEN
                RAISE EXCEPTION
                    'cg_envelopes.%[%] missing or non-numeric "cg" key. '
                    'Expected: {"w": <weight_lbs>, "cg": <cg_value>}',
                    v_col, v_i;
            END IF;

            v_curr_w := (v_elem ->> 'w')::NUMERIC;

            -- Weight must be non-negative
            IF v_curr_w < 0 THEN
                RAISE EXCEPTION
                    'cg_envelopes.%[%] has negative weight w=%. '
                    'CG envelope weights must be >= 0.',
                    v_col, v_i, v_curr_w;
            END IF;

            -- Points must be strictly ascending by w
            IF v_prev_w IS NOT NULL AND v_curr_w <= v_prev_w THEN
                RAISE EXCEPTION
                    'cg_envelopes.% points must be sorted ascending by w. '
                    'Point [%] has w=% which is not greater than previous w=%.',
                    v_col, v_i, v_curr_w, v_prev_w;
            END IF;

            v_prev_w := v_curr_w;
        END LOOP;

    END LOOP;

    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION aircraft_specs.validate_cg_envelope_points() IS
    'BEFORE INSERT OR UPDATE trigger on cg_envelopes. '
    'Validates that fwd_limit_points and aft_limit_points are either NULL or '
    'well-formed JSON arrays of {w: numeric, cg: numeric} objects with '
    'at least 2 points sorted in strictly ascending order by w. '
    'Raises EXCEPTION on any structural violation, preventing malformed data '
    'from entering the table silently.';

CREATE TRIGGER trg_validate_cg_envelope_points
    BEFORE INSERT OR UPDATE ON aircraft_specs.cg_envelopes
    FOR EACH ROW EXECUTE FUNCTION aircraft_specs.validate_cg_envelope_points();

-- =============================================================================
-- aircraft_specs.payload_range_points
-- =============================================================================

CREATE TABLE aircraft_specs.payload_range_points (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id       BIGINT NOT NULL
                         REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    curve_label      TEXT,
    sequence_order   SMALLINT NOT NULL,
    payload_lbs      NUMERIC,
    range_nm         NUMERIC,
    fuel_onboard_lbs NUMERIC,
    speed_ktas       NUMERIC,
    altitude_ft      NUMERIC,
    conditions_text  TEXT,
    confidence       aircraft_ref.confidence_score,
    notes            TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_prp_payload_nonneg CHECK (payload_lbs      IS NULL OR payload_lbs      >= 0),
    CONSTRAINT chk_prp_range_nonneg   CHECK (range_nm          IS NULL OR range_nm          >= 0),
    CONSTRAINT chk_prp_fuel_nonneg    CHECK (fuel_onboard_lbs  IS NULL OR fuel_onboard_lbs  >= 0),
    CONSTRAINT chk_prp_sequence       CHECK (sequence_order > 0)
);

COMMENT ON TABLE aircraft_specs.payload_range_points IS
    'Discrete points on payload-range trade-off curves. '
    'sequence_order = 1: maximum payload, minimum range. '
    'Last point: maximum range, minimum or zero payload. '
    'NULL curve_label = standard/baseline conditions.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_weight_metrics_updated
    BEFORE UPDATE ON aircraft_specs.weight_metrics
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_specs.weight_metrics ────────────────────────────────────────────

CREATE UNIQUE INDEX uq_weight_metrics_config
    ON aircraft_specs.weight_metrics
        (variant_id, metric_type_code, COALESCE(configuration, ''));

CREATE INDEX idx_wm_type_canonical
    ON aircraft_specs.weight_metrics (metric_type_code, canonical_value)
    WHERE canonical_value IS NOT NULL;

CREATE INDEX idx_wm_variant
    ON aircraft_specs.weight_metrics (variant_id);

-- ── aircraft_specs.cg_envelopes ──────────────────────────────────────────────

CREATE INDEX idx_cge_variant
    ON aircraft_specs.cg_envelopes (variant_id);

CREATE UNIQUE INDEX uq_cge_primary
    ON aircraft_specs.cg_envelopes (variant_id)
    WHERE is_primary;

CREATE INDEX idx_cge_fwd_points
    ON aircraft_specs.cg_envelopes USING gin (fwd_limit_points jsonb_path_ops)
    WHERE fwd_limit_points IS NOT NULL;

CREATE INDEX idx_cge_aft_points
    ON aircraft_specs.cg_envelopes USING gin (aft_limit_points jsonb_path_ops)
    WHERE aft_limit_points IS NOT NULL;

-- ── aircraft_specs.payload_range_points ──────────────────────────────────────

CREATE UNIQUE INDEX uq_payload_range_point
    ON aircraft_specs.payload_range_points
        (variant_id, COALESCE(curve_label, ''), sequence_order);

CREATE INDEX idx_prp_variant_curve
    ON aircraft_specs.payload_range_points (variant_id, curve_label, sequence_order);

COMMIT;