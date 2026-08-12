-- =============================================================================
-- File: database/migrations/012_ownership_cost_valuation_market.sql
-- Phase 12 — aircraft_market: market valuations, ownership cost snapshots
-- with assumption sets, and itemised cost line items.
--
-- Design improvement over reference schema:
--   Old: single valuation_snapshots table mixing market data + cost estimates,
--        with fixed named cost columns + additional_costs JSONB overflow.
--   New: two separated concerns:
--     valuations   — market price and availability time-series
--     cost_snapshots + cost_line_items — cost estimates with full assumption
--         sets and one normalised row per cost_item_type (no JSONB overflow).
--
-- aircraft_ref.cost_item_types contains 18 component rows and 3 aggregate
-- total rows. Components map to cost_line_items; aggregate totals map to
-- cost_snapshot_totals. Phase 17 ingestion reads PlanePHD ownership_costs
-- JSONB and routes each recognized key to the appropriate table.
--
-- Spec coverage (requirement 9):
--   acquisition price / listing prices → valuations
--   for_sale_count / market availability→ valuations.for_sale_count
--   currency, region                   → currency_code / region_code on both tables
--   model year / condition / hours     → cost_snapshots assumption columns
--   all 18 named ownership cost types  → cost_line_items.cost_item_type_code
--   time-series snapshots              → snapshot_date on both tables
--   confidence scores                  → confidence on both tables
--   regional variation                 → region_code FK → aircraft_geo.regions
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_market.valuations
-- Market price and availability snapshots per variant.
-- One row per (variant, snapshot_date, source) using a functional UNIQUE index.
-- papi_price_estimate: source system's estimated typical market value
--   (PlanePHD PAPI = Price Aircraft Price Index; other sources use their own
--   estimating methodology — column name preserved for ETL compatibility).
-- listing_price_*: actual active listing statistics from the marketplace.
-- for_sale_count: number of active listings at snapshot time.
-- Assumptions (condition_grade_code, assumed_year, assumed_airframe_hours)
-- contextualise what aircraft configuration the price estimate applies to.
-- =============================================================================

CREATE TABLE aircraft_market.valuations (
    id                      BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id              BIGINT NOT NULL
                                REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    snapshot_date           DATE        NOT NULL DEFAULT CURRENT_DATE,
    source_name             TEXT,     -- 'PlanePHD', 'Controller.com', 'manual_entry', etc.
    source_url              TEXT,

    -- ── Market data ──────────────────────────────────────────────────────────
    -- Estimated typical market value (source methodology varies by system).
    papi_price_estimate     NUMERIC(14,2),
    listing_price_low       NUMERIC(14,2),   -- lowest active listing price observed
    listing_price_high      NUMERIC(14,2),   -- highest active listing price observed
    listing_price_median    NUMERIC(14,2),   -- median active listing price
    for_sale_count          INTEGER,          -- active listings at snapshot time

    -- ── Currency and region ────────────────────────────────────────────────────
    currency_code           VARCHAR(3) NOT NULL DEFAULT 'USD'
                                REFERENCES aircraft_ref.currencies(code),
    -- NULL = global / not region-specific.
    region_code             aircraft_ref.lookup_code
                                REFERENCES aircraft_geo.regions(code),

    -- ── Assumptions contextualising this valuation ────────────────────────────
    condition_grade_code    aircraft_ref.lookup_code
                                REFERENCES aircraft_ref.aircraft_condition_grades(code),
    -- Model year assumption for this price estimate.
    assumed_year            aircraft_ref.year_value,
    -- Assumed total airframe hours for this estimate (e.g., 1 500 hrs).
    assumed_airframe_hours  INTEGER,

    confidence              aircraft_ref.confidence_score,
    notes                   TEXT,
    -- Timestamp when the snapshot was recorded (differs from snapshot_date
    -- which reflects the reporting period, not the capture time).
    captured_at             TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT chk_val_prices CHECK (
        (papi_price_estimate IS NULL OR papi_price_estimate >= 0)
        AND (listing_price_low  IS NULL OR listing_price_low  >= 0)
        AND (listing_price_high IS NULL OR listing_price_high >= 0)
        AND (listing_price_median IS NULL OR listing_price_median >= 0)
        AND (for_sale_count IS NULL OR for_sale_count >= 0)
        AND (assumed_airframe_hours IS NULL OR assumed_airframe_hours >= 0)
    ),
    CONSTRAINT chk_val_price_range CHECK (
        listing_price_low IS NULL
        OR listing_price_high IS NULL
        OR listing_price_low <= listing_price_high
    )
);

COMMENT ON TABLE aircraft_market.valuations IS
    'Market price and availability snapshots. Time-series: one row per '
    '(variant, snapshot_date, source). '
    'papi_price_estimate is the source system''s estimated typical value '
    '(PlanePHD uses their PAPI index; other sources use their own methodology). '
    'listing_price_* are actual active marketplace listing statistics. '
    'A Phase 16 view (v_current_valuation) replicates the reference schema ''s '
    'DISTINCT ON pattern to return the most recent row per variant.';
COMMENT ON COLUMN aircraft_market.valuations.papi_price_estimate IS
    'Source system''s estimated typical market value. Column name retained for '
    'PlanePHD ETL compatibility (PAPI = Price Aircraft Price Index). '
    'Treat as "estimated_market_value" when sourced from non-PlanePHD systems.';
COMMENT ON COLUMN aircraft_market.valuations.for_sale_count IS
    'Number of active listings observed at snapshot_date. '
    'NULL = not reported by source. '
    'Used for market liquidity comparison: few listings = illiquid market.';

-- =============================================================================
-- aircraft_market.cost_snapshots
-- Ownership cost estimate for a variant under a stated assumption set.
-- Time-series: one row per (variant, snapshot_date, source).
-- Cost line items live in cost_line_items (FK snapshot_id).
-- Assumption columns contextualise the estimate:
--   - condition grade (what state of aircraft was assumed)
--   - hours (airframe, engine, prop — time-in-service affects maintenance costs)
--   - utilisation (annual hours flown — drives variable cost impact)
--   - fuel price (drives fuel cost line item)
-- Phase 16 aggregation views compute:
--   total_annual_fixed   = SUM(amount_annual)  WHERE is_fixed
--   total_hourly_variable= SUM(amount_per_hour) WHERE NOT is_fixed
--   total_annual_cost    = fixed + (hourly × assumed_annual_hours)
-- =============================================================================

CREATE TABLE aircraft_market.cost_snapshots (
    id                          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id                  BIGINT NOT NULL
                                    REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    snapshot_date               DATE        NOT NULL DEFAULT CURRENT_DATE,
    source_name                 TEXT,    -- 'PlanePHD', 'AOPA', 'manual_entry', etc.

    -- ── Assumptions ───────────────────────────────────────────────────────────
    condition_grade_code        aircraft_ref.lookup_code
                                    REFERENCES aircraft_ref.aircraft_condition_grades(code),
    -- Total airframe time assumed (affects annual inspection scope).
    assumed_airframe_hours      INTEGER,
    -- Engine time since major overhaul (SMOH) — drives reserve accrual rate.
    assumed_engine_hours_smoh   INTEGER,
    -- Propeller time since overhaul (SMOH).
    assumed_prop_hours_smoh     INTEGER,
    -- Annual flight hours assumed for this owner profile (key utilisation lever).
    assumed_annual_hours        NUMERIC,
    -- Fuel price per US gallon (drives fuel cost line item calculation).
    assumed_fuel_price_per_gal  NUMERIC,
    -- Assumed fuel burn rate in GPH (cross-check against specs; may be source-provided).
    assumed_fuel_burn_gph       NUMERIC,

    -- ── Currency and region ────────────────────────────────────────────────────
    currency_code               VARCHAR(3) NOT NULL DEFAULT 'USD'
                                    REFERENCES aircraft_ref.currencies(code),
    region_code                 aircraft_ref.lookup_code
                                    REFERENCES aircraft_geo.regions(code),

    confidence                  aircraft_ref.confidence_score,
    notes                       TEXT,
    -- Overflow for source cost keys that are unmapped, non-numeric, or
    -- unparseable. Written by Phase 17b ingestion (902) as {key: raw_value}
    -- and surfaced for curation. Follows the extra_attributes idiom used
    -- elsewhere in the schema.
    extra_attributes            JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT chk_cs_hours CHECK (
        (assumed_airframe_hours      IS NULL OR assumed_airframe_hours      >= 0)
        AND (assumed_engine_hours_smoh IS NULL OR assumed_engine_hours_smoh >= 0)
        AND (assumed_prop_hours_smoh  IS NULL OR assumed_prop_hours_smoh    >= 0)
        AND (assumed_annual_hours     IS NULL OR assumed_annual_hours        > 0)
    ),
    CONSTRAINT chk_cs_fuel CHECK (
        (assumed_fuel_price_per_gal IS NULL OR assumed_fuel_price_per_gal >= 0)
        AND (assumed_fuel_burn_gph  IS NULL OR assumed_fuel_burn_gph       >= 0)
    )
);

COMMENT ON TABLE aircraft_market.cost_snapshots IS
    'Ownership cost estimate for a variant under a stated assumption set. '
    'Assumptions (condition, hours, utilisation, fuel price) are embedded '
    'to contextualise the line items. Changing one assumption should yield a '
    'new snapshot row, not an UPDATE. '
    'Phase 16 views aggregate cost_line_items to produce total annual and '
    'per-hour cost figures. '
    'Maps to PlanePHD ''s ownership_costs JSONB via Phase 17 ingestion.';
COMMENT ON COLUMN aircraft_market.cost_snapshots.assumed_annual_hours IS
    'Annual flight hours assumed for this owner profile. '
    'Critical: it converts per-hour variable costs to annual equivalents. '
    'Example: 100 hr/year ≈ private owner; 400 hr/year ≈ charter.';
COMMENT ON COLUMN aircraft_market.cost_snapshots.assumed_fuel_burn_gph IS
    'Assumed fuel consumption rate in US GPH for cost calculation. '
    'May differ from aircraft_specs.performance_metrics (FUEL_BURN_CRUISE) '
    'if the source used a different cruise power setting in their cost model. '
    'Preserve source value; cross-reference against spec data in curation.';

-- =============================================================================
-- aircraft_market.cost_line_items
-- Individual ownership cost items within a cost snapshot.
-- One row per (snapshot, cost_item_type) using UNIQUE constraint.
-- amount_annual: annual fixed cost (is_fixed = TRUE items).
-- amount_per_hour: per-flight-hour variable cost (is_fixed = FALSE items).
-- A line item may have BOTH set (e.g., engine reserve has both an annual
-- minimum and a per-hour accrual). At least one must be non-NULL.
-- =============================================================================

CREATE TABLE aircraft_market.cost_line_items (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    snapshot_id          BIGINT NOT NULL
                             REFERENCES aircraft_market.cost_snapshots(id) ON DELETE CASCADE,
    cost_item_type_code  aircraft_ref.lookup_code NOT NULL
                             REFERENCES aircraft_ref.cost_item_types(code),
    -- Annual cost for this item (typically used for is_fixed items).
    amount_annual        NUMERIC(12,2),
    -- Per-flight-hour cost (typically used for is_fixed = FALSE items).
    amount_per_hour      NUMERIC(10,4),
    -- Currency may differ per line item (e.g., hangar in local currency).
    currency_code        VARCHAR(3) NOT NULL DEFAULT 'USD'
                             REFERENCES aircraft_ref.currencies(code),
    notes                TEXT,

    UNIQUE (snapshot_id, cost_item_type_code),

    CONSTRAINT chk_cli_has_amount CHECK (
        amount_annual IS NOT NULL OR amount_per_hour IS NOT NULL
    ),
    CONSTRAINT chk_cli_amounts_nonneg CHECK (
        (amount_annual    IS NULL OR amount_annual    >= 0)
        AND (amount_per_hour IS NULL OR amount_per_hour >= 0)
    )
);

COMMENT ON TABLE aircraft_market.cost_line_items IS
    'Individual ownership cost items within a cost_snapshot. '
    'One row per (snapshot, cost_item_type). '
    'amount_annual: annual total (insurance, hangar, inspection, depreciation). '
    'amount_per_hour: per-flight-hour rate (fuel, maintenance, reserves). '
    'Both may be set (some items have fixed minimums + variable components). '
    'At least one must be non-NULL (chk_cli_has_amount). '
    'Phase 16 aggregation: '
    '  total_annual_fixed  = SUM(amount_annual) JOIN is_fixed = TRUE '
    '  total_hourly_var    = SUM(amount_per_hour) JOIN is_fixed = FALSE '
    '  total_annual @ N hr = fixed + (hourly × N)';
COMMENT ON COLUMN aircraft_market.cost_line_items.amount_per_hour IS
    'Per-flight-hour operating cost in currency_code. '
    'NUMERIC(10,4): four decimal places for low-cost fractional hourly items '
    '(e.g., oil consumption $0.0083/hr). '
    'Phase 17 ingestion maps PlanePHD ownership_costs keys to these rows '
    'via aircraft_ref.cost_item_types lookup.';

-- =============================================================================
-- aircraft_market.cost_snapshot_totals
-- Pre-computed aggregate totals provided directly by the source
-- (the three is_aggregate cost types: TOTAL_COST_ANNUAL, TOTAL_FIXED_COST,
-- TOTAL_VARIABLE_COST). Kept SEPARATE from cost_line_items so that summing
-- cost_line_items never double-counts these source-provided totals
-- (see aircraft_ref.cost_item_types.is_aggregate and the docs in Phase 2).
-- 1:1 with cost_snapshots (UNIQUE on snapshot_id).
-- Consumed by aircraft_read.mv_ownership_cost_summary (Phase 16).
-- =============================================================================

CREATE TABLE aircraft_market.cost_snapshot_totals (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    snapshot_id         BIGINT NOT NULL UNIQUE
                            REFERENCES aircraft_market.cost_snapshots(id) ON DELETE CASCADE,
    -- Source-provided pre-computed totals (USD canonical).
    total_annual_usd    NUMERIC(14,2),
    total_fixed_usd     NUMERIC(14,2),
    total_variable_usd  NUMERIC(14,2),
    -- Annual flight-hours assumption the source used to derive these totals.
    -- May differ from cost_snapshots.assumed_annual_hours when the source
    -- published totals under its own utilisation assumption. Read by Phase 16
    -- mv_ownership_cost_summary.
    assumed_hours       NUMERIC,
    -- Currency of the totals. Named source_currency (not currency_code) to match
    -- the Phase 17b ingestion contract (902 writes source_currency).
    source_currency     VARCHAR(3) NOT NULL DEFAULT 'USD'
                            REFERENCES aircraft_ref.currencies(code),
    -- Raw PlanePHD ownership_costs key the variable total was captured from
    -- (e.g. 'total_variable_cost_per_hour_...'); written by 902 for provenance.
    captured_from_key   TEXT,
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_cst_nonneg CHECK (
        (total_annual_usd   IS NULL OR total_annual_usd   >= 0)
        AND (total_fixed_usd    IS NULL OR total_fixed_usd    >= 0)
        AND (total_variable_usd IS NULL OR total_variable_usd >= 0)
        AND (assumed_hours      IS NULL OR assumed_hours       > 0)
    )
);

COMMENT ON TABLE aircraft_market.cost_snapshot_totals IS
    'Source-provided pre-computed cost totals, one row per cost_snapshot. '
    'Holds the three is_aggregate cost types (TOTAL_COST_ANNUAL, TOTAL_FIXED_COST, '
    'TOTAL_VARIABLE_COST) so they are never mixed into cost_line_items and never '
    'double-counted in SUM() aggregations. Preferred for display when present; '
    'mv_ownership_cost_summary cross-validates against computed line-item totals.';

-- =============================================================================
-- Aggregate-guard trigger on cost_line_items
-- aircraft_ref.cost_item_types.is_aggregate rows must NEVER be inserted into
-- cost_line_items (they belong in cost_snapshot_totals). A CHECK constraint
-- cannot reference another table, so this is enforced with a trigger.
-- This is the chk_cli_no_aggregate guarantee documented in Phase 2.
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_market.reject_aggregate_line_item()
RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_is_aggregate BOOLEAN;
BEGIN
    SELECT is_aggregate INTO v_is_aggregate
    FROM aircraft_ref.cost_item_types
    WHERE code = NEW.cost_item_type_code;

    IF v_is_aggregate THEN
        RAISE EXCEPTION
            'cost_line_items may not contain aggregate cost type %; '
            'route pre-computed totals to aircraft_market.cost_snapshot_totals.',
            NEW.cost_item_type_code;
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION aircraft_market.reject_aggregate_line_item() IS
    'BEFORE INSERT/UPDATE trigger on cost_line_items. Rejects any row whose '
    'cost_item_type is is_aggregate = TRUE (these belong in cost_snapshot_totals). '
    'Implements the chk_cli_no_aggregate guarantee documented in Phase 2.';

CREATE TRIGGER trg_cli_reject_aggregate
    BEFORE INSERT OR UPDATE ON aircraft_market.cost_line_items
    FOR EACH ROW EXECUTE FUNCTION aircraft_market.reject_aggregate_line_item();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_market.valuations ────────────────────────────────────────────────

-- One snapshot per (variant, date, source); NULL source treated as empty string.
CREATE UNIQUE INDEX uq_val_variant_date_source
    ON aircraft_market.valuations
        (variant_id, snapshot_date, COALESCE(source_name, ''));

-- Most-recent valuation retrieval (DISTINCT ON pattern used in Phase 16 view).
CREATE INDEX idx_val_variant_date
    ON aircraft_market.valuations (variant_id, snapshot_date DESC, captured_at DESC, id DESC);

-- Price range filter — buyer search "show me aircraft under $200k".
CREATE INDEX idx_val_price
    ON aircraft_market.valuations (papi_price_estimate)
    WHERE papi_price_estimate IS NOT NULL;

-- Market availability filter.
CREATE INDEX idx_val_for_sale
    ON aircraft_market.valuations (for_sale_count)
    WHERE for_sale_count IS NOT NULL;

-- Region-based market queries.
CREATE INDEX idx_val_region
    ON aircraft_market.valuations (region_code)
    WHERE region_code IS NOT NULL;

-- ── aircraft_market.cost_snapshots ────────────────────────────────────────────

-- One snapshot per (variant, date, source).
CREATE UNIQUE INDEX uq_cs_variant_date_source
    ON aircraft_market.cost_snapshots
        (variant_id, snapshot_date, COALESCE(source_name, ''));

-- Most-recent cost snapshot per variant.
CREATE INDEX idx_cs_variant_date
    ON aircraft_market.cost_snapshots (variant_id, snapshot_date DESC);

-- Filter by condition grade — "show cost estimates for GOOD condition aircraft".
CREATE INDEX idx_cs_condition
    ON aircraft_market.cost_snapshots (condition_grade_code)
    WHERE condition_grade_code IS NOT NULL;

-- Annual hours filter — "show estimates for 100 hr/year owners".
CREATE INDEX idx_cs_annual_hours
    ON aircraft_market.cost_snapshots (assumed_annual_hours)
    WHERE assumed_annual_hours IS NOT NULL;

-- ── aircraft_market.cost_line_items ───────────────────────────────────────────

-- All snapshots containing a specific cost item type — comparison.
CREATE INDEX idx_cli_type
    ON aircraft_market.cost_line_items (cost_item_type_code);

-- All line items for one snapshot (detail page retrieval).
CREATE INDEX idx_cli_snapshot
    ON aircraft_market.cost_line_items (snapshot_id);

-- Fuel cost comparison across variants.
CREATE INDEX idx_cli_fuel_items
    ON aircraft_market.cost_line_items (amount_per_hour)
    WHERE cost_item_type_code = 'FUEL' AND amount_per_hour IS NOT NULL;

COMMIT;