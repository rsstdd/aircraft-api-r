-- =============================================================================
-- File: database/migrations/015_mission_profiles_comparison_scoring.sql
-- Phase 15 — aircraft_compare: mission profile definitions, scoring criteria,
-- pre-computed variant suitability scores, and individual criterion breakdowns.
--
-- Architecture:
--   aircraft_ref.mission_profile_types (Phase 2 lookup) — the type enum
--   aircraft_compare.mission_profiles  — extended definition + scoring params
--   aircraft_compare.mission_criteria  — weighted criteria per profile
--   aircraft_compare.variant_suitability — pre-computed aggregate score
--   aircraft_compare.criterion_scores    — per-criterion breakdown
--
-- Scoring model (linear interpolation):
--   For is_higher_better = TRUE (range, speed, ceiling):
--     canonical_value < scoring_lower_bound  → raw_score = 0.0
--     canonical_value > scoring_upper_bound  → raw_score = 1.0
--     between bounds                         → linear interpolation
--   For is_higher_better = FALSE (runway dist, cost):
--     canonical_value > scoring_upper_bound  → raw_score = 0.0
--     canonical_value < scoring_lower_bound  → raw_score = 1.0
--     between bounds                         → linear interpolation (inverted)
--   weighted_score = raw_score × mission_criteria.weight
--   overall_score  = SUM(weighted_score) across all criteria
--   is_disqualified = TRUE if any is_required criterion has raw_score = 0
--
-- Criterion weight constraint: weights should sum to 1.000 per mission profile.
-- Enforced by application logic and a Phase 16 validation view, not DDL.
--
-- Spec coverage (requirement 2):
--   mission profiles (15 types)           → mission_profiles + seeds
--   weighted scoring dimensions            → mission_criteria.weight
--   required constraints                   → mission_criteria.is_required
--   preferred constraints                  → mission_criteria.scoring_* bounds
--   aircraft mission suitability           → variant_suitability
--   derived scoring outputs                → criterion_scores
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_compare.mission_profiles
-- Extended definition for each mission profile type.
-- profile_type_code: FK to the Phase 2 lookup for type identity.
-- scoring_lower_bound / scoring_upper_bound: typical parameters for scoring
--   functions when the criterion type does not specify per-criterion bounds.
-- =============================================================================

CREATE TABLE aircraft_compare.mission_profiles (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    profile_type_code   aircraft_ref.lookup_code NOT NULL UNIQUE
                            REFERENCES aircraft_ref.mission_profile_types(code),
    slug                aircraft_ref.slug_text NOT NULL UNIQUE,
    title               TEXT NOT NULL,
    description         TEXT,
    -- Indicative mission parameters (used for informational display).
    typical_range_nm    NUMERIC,     -- representative one-way mission distance
    typical_pax_count   SMALLINT,    -- typical passenger count for this profile
    typical_altitude_ft NUMERIC,     -- representative cruise altitude
    -- Scope flags
    applies_to_civilian BOOLEAN NOT NULL DEFAULT TRUE,
    applies_to_military BOOLEAN NOT NULL DEFAULT FALSE,
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    sort_order          SMALLINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_compare.mission_profiles IS
    'Extended mission profile definitions. One row per aircraft_ref.mission_profile_types entry. '
    'Defines typical mission parameters and scope flags used in the comparison engine. '
    'Scoring criteria and weights are in mission_criteria (1:N).';

-- =============================================================================
-- aircraft_compare.mission_criteria
-- Weighted scoring criteria for each mission profile.
-- Each row defines: which criterion type, its weight, whether meeting the
-- minimum is required for non-disqualification, and scoring bounds.
-- UNIQUE (mission_profile_id, criterion_type_code): one weight per criterion.
-- =============================================================================

CREATE TABLE aircraft_compare.mission_criteria (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    mission_profile_id    BIGINT NOT NULL
                              REFERENCES aircraft_compare.mission_profiles(id) ON DELETE CASCADE,
    criterion_type_code   aircraft_ref.lookup_code NOT NULL
                              REFERENCES aircraft_ref.comparison_criterion_types(code),
    -- Weight of this criterion within the profile (0 < weight ≤ 1).
    -- Weights should sum to 1.000 per profile (enforced by application/validation view).
    weight                NUMERIC(4,3) NOT NULL,
    -- is_required: if TRUE and the variant cannot meet the minimum acceptable value,
    -- the variant is disqualified from the profile (overall_score = 0, is_disqualified = TRUE).
    is_required           BOOLEAN NOT NULL DEFAULT FALSE,
    -- Scoring bounds for the linear interpolation scoring function.
    -- For is_higher_better = TRUE: below lower_bound → score 0; above upper_bound → score 1.
    -- For is_higher_better = FALSE: above upper_bound → score 0; below lower_bound → score 1.
    -- NULL means the criterion contributes to scoring only qualitatively or
    -- the bounds are set at a global level by the scoring engine.
    scoring_lower_bound   NUMERIC,
    scoring_upper_bound   NUMERIC,
    notes                 TEXT,
    UNIQUE (mission_profile_id, criterion_type_code),
    CONSTRAINT chk_mc_weight CHECK (weight > 0 AND weight <= 1)
);

COMMENT ON TABLE aircraft_compare.mission_criteria IS
    'Weighted scoring criteria per mission profile. '
    'weight: contribution of this criterion to the overall profile score (0 < w ≤ 1). '
    'Weights should sum to 1.000 per mission_profile_id (validated in Phase 16 view). '
    'scoring_lower_bound / scoring_upper_bound define the linear scoring function endpoints. '
    'is_required = TRUE means a variant scoring 0 on this criterion is disqualified.';
COMMENT ON COLUMN aircraft_compare.mission_criteria.scoring_lower_bound IS
    'For is_higher_better = TRUE: canonical values below this yield raw_score = 0.0. '
    'For is_higher_better = FALSE: canonical values above this yield raw_score = 0.0. '
    'NULL = no explicit lower bound (criterion scored without disqualification threshold).';

-- =============================================================================
-- aircraft_compare.variant_suitability
-- Pre-computed aggregate suitability score per (variant, mission_profile).
-- UNIQUE (variant_id, mission_profile_id): one score per variant per profile.
-- Refreshed by a batch process or triggered update when canonical metrics change.
-- Scoring is performed by an application-layer or SQL function; this table
-- is the output cache for fast comparison queries.
-- =============================================================================

CREATE TABLE aircraft_compare.variant_suitability (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id               BIGINT NOT NULL
                                 REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    mission_profile_id       BIGINT NOT NULL
                                 REFERENCES aircraft_compare.mission_profiles(id) ON DELETE CASCADE,
    -- Weighted aggregate suitability score (0.000 = unsuitable, 1.000 = ideal fit).
    overall_score            NUMERIC(5,3),
    -- TRUE if any is_required criterion scored 0 for this variant.
    is_disqualified          BOOLEAN NOT NULL DEFAULT FALSE,
    disqualification_reason  TEXT,
    -- Criteria coverage metadata
    scored_criteria_count    SMALLINT,       -- number of criteria with data
    total_criteria_count     SMALLINT,       -- total criteria in the profile
    required_criteria_met    SMALLINT,       -- of is_required criteria, how many scored > 0
    total_required_criteria  SMALLINT,
    -- Staleness tracking: when were the underlying metrics last updated?
    computed_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (variant_id, mission_profile_id),
    CONSTRAINT chk_vs_score CHECK (
        overall_score IS NULL
        OR (overall_score >= 0 AND overall_score <= 1)
    )
);

COMMENT ON TABLE aircraft_compare.variant_suitability IS
    'Pre-computed mission suitability scores. One row per (variant, mission_profile). '
    'overall_score: weighted sum of criterion_scores.weighted_score (0–1). '
    'is_disqualified: TRUE when any is_required criterion has raw_score = 0. '
    'computed_at: used for staleness detection — scores older than the last '
    'performance_metrics update should be recomputed. '
    'Phase 16 includes a refresh function for batch recomputation.';

-- =============================================================================
-- aircraft_compare.criterion_scores
-- Per-criterion breakdown for each variant_suitability record.
-- Stores both the raw metric value and the computed score components.
-- UNIQUE (variant_suitability_id, criterion_type_code).
-- =============================================================================

CREATE TABLE aircraft_compare.criterion_scores (
    id                     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_suitability_id BIGINT NOT NULL
                               REFERENCES aircraft_compare.variant_suitability(id) ON DELETE CASCADE,
    criterion_type_code    aircraft_ref.lookup_code NOT NULL
                               REFERENCES aircraft_ref.comparison_criterion_types(code),
    -- The variant's actual canonical value for this criterion's metric.
    raw_canonical_value    NUMERIC,
    -- 0.000–1.000 score before applying weight.
    raw_score              NUMERIC(5,3),
    -- raw_score × mission_criteria.weight for this criterion.
    weighted_score         NUMERIC(5,3),
    -- Whether this variant met the minimum acceptable threshold.
    meets_minimum          BOOLEAN,
    notes                  TEXT,
    UNIQUE (variant_suitability_id, criterion_type_code),
    CONSTRAINT chk_cs_scores CHECK (
        (raw_score      IS NULL OR (raw_score      >= 0 AND raw_score      <= 1))
        AND (weighted_score IS NULL OR (weighted_score >= 0 AND weighted_score <= 1))
    )
);

COMMENT ON TABLE aircraft_compare.criterion_scores IS
    'Per-criterion score breakdown within a variant_suitability record. '
    'raw_canonical_value: the variant''s actual metric from aircraft_specs. '
    'raw_score: 0–1 score before weighting (from linear interpolation function). '
    'weighted_score: raw_score × mission_criteria.weight. '
    'Summing weighted_score gives overall_score on variant_suitability.';

-- =============================================================================
-- INDEXES
-- =============================================================================

CREATE INDEX idx_mp_active
    ON aircraft_compare.mission_profiles (sort_order)
    WHERE is_active;

CREATE INDEX idx_mc_profile
    ON aircraft_compare.mission_criteria (mission_profile_id);
CREATE INDEX idx_mc_criterion
    ON aircraft_compare.mission_criteria (criterion_type_code);

CREATE INDEX idx_vs_profile_score
    ON aircraft_compare.variant_suitability (mission_profile_id, overall_score DESC NULLS LAST)
    WHERE NOT is_disqualified AND overall_score IS NOT NULL;

CREATE INDEX idx_vs_variant
    ON aircraft_compare.variant_suitability (variant_id);

CREATE INDEX idx_vs_stale
    ON aircraft_compare.variant_suitability (computed_at ASC)
    WHERE overall_score IS NOT NULL;

CREATE INDEX idx_cs_suitability
    ON aircraft_compare.criterion_scores (variant_suitability_id);

COMMIT;