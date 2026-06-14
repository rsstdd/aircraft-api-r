-- ============================================================================
-- FILE: 012_advanced_analytics_cache.sql
-- DESCRIPTION: Implements custom analytical functions and materialized cache summaries.
-- ============================================================================

BEGIN;

CREATE SCHEMA aircraft_analytics;
SET search_path TO aircraft_analytics, public, aircraft_core, aircraft_specs, aircraft_perf, aircraft_market;

-- ----------------------------------------------------------------------------
-- 1. ADVANCED CALCULATION FUNCTIONS (Encapsulated Domain Logic)
-- ----------------------------------------------------------------------------

-- A. Calculate Cabin Payload Capacity with Maximum Usable Fuel loaded
CREATE OR REPLACE FUNCTION fn_calculate_full_fuel_payload(p_config_id BIGINT)
RETURNS NUMERIC AS $$
DECLARE
v_mtow NUMERIC;
    v_oem NUMERIC;
    v_fuel_wt NUMERIC;
    v_result NUMERIC;
BEGIN
SELECT
    max_takeoff_weight_lbs,
    COALESCE(operating_empty_weight_lbs, basic_empty_weight_lbs),
    COALESCE(usable_fuel_weight_lbs, 0.00)
INTO v_mtow, v_oem, v_fuel_wt
FROM public.unified_aircraft_registry
WHERE configuration_id = p_config_id;

IF v_mtow IS NULL OR v_oem IS NULL THEN
        RETURN NULL;
END IF;

    v_result := v_mtow - (v_oem + v_fuel_wt);
RETURN v_result;
END;
$$ LANGUAGE plpgsql STABLE;

-- B. Calculate Estimated Trip Fuel and Financial Variable Cost for a given distance
CREATE OR REPLACE FUNCTION fn_estimate_trip_financials(
    p_config_id BIGINT,
    p_distance_nm INT,
    p_fuel_price_per_gal NUMERIC DEFAULT 6.50
)
RETURNS TABLE (
    estimated_hours NUMERIC,
    estimated_fuel_gallons NUMERIC,
    total_variable_trip_cost NUMERIC
) AS $$
DECLARE
v_cruise_spd NUMERIC;
    v_fuel_flow NUMERIC;
    v_maint_hourly NUMERIC;
BEGIN
SELECT
    COALESCE(economy_cruise_speed_ktas, max_cruise_speed_ktas),
    COALESCE(mkt.estimated_fuel_cost_per_hour / NULLIF(p_fuel_price_per_gal, 0), crz.economy_cruise_fuel_flow_gph, crz.max_cruise_fuel_flow_gph),
    (mkt.total_variable_cost_per_hour - mkt.estimated_fuel_cost_per_hour)
INTO v_cruise_spd, v_fuel_flow, v_maint_hourly
FROM aircraft_core.aircraft_configurations cfg
         LEFT JOIN aircraft_perf.cruise_envelopes crz ON cfg.id = crz.configuration_id
         LEFT JOIN aircraft_market.operating_cost_estimates mkt ON cfg.id = mkt.configuration_id
WHERE cfg.id = p_config_id;

-- Avoid division by zero if performance profile data is missing
IF v_cruise_spd IS NULL OR v_cruise_spd = 0 OR v_fuel_flow IS NULL THEN
        RETURN NEXT;
END IF;

    estimated_hours := ROUND((p_distance_nm::NUMERIC / v_cruise_spd), 2);
    estimated_fuel_gallons := ROUND((estimated_hours * v_fuel_flow), 1);
    total_variable_trip_cost := ROUND((estimated_fuel_gallons * p_fuel_price_per_gal) + (estimated_hours * COALESCE(v_maint_hourly, 0)), 2);

RETURN NEXT;
END;
$$ LANGUAGE plpgsql STABLE;

-- ----------------------------------------------------------------------------
-- 2. MATERIALIZED SUMMARY CACHE (Aggregating cross-table metrics for dashboards)
-- ----------------------------------------------------------------------------
CREATE MATERIALIZED VIEW mv_fleet_market_summary AS
SELECT
    role.name AS market_role_segment,
    prop.code AS propulsion_framework,
    COUNT(DISTINCT cfg.id) AS active_configurations_count,

    -- Statistical Averages across segments
    ROUND(AVG(w_lim.max_takeoff_weight_lbs), 1) AS avg_mtow_lbs,
    ROUND(AVG(crz.max_range_nm), 0) AS avg_max_range_nm,
    ROUND(AVG(mkt.total_variable_cost_per_hour), 2) AS avg_variable_operating_cost_hourly,
    ROUND(AVG(val.estimated_retail_value), 2) AS avg_current_market_value

FROM aircraft_core.aircraft_configurations cfg
         JOIN aircraft_core.aircraft_variants vrt ON cfg.variant_id = vrt.id
         JOIN aircraft_core.aircraft_models mdl ON vrt.model_id = mdl.id
         JOIN aircraft_core.aircraft_families fam ON mdl.family_id = fam.id
         JOIN aircraft_ref.propulsion_categories prop ON prop.code = (
    SELECT propulsion_category_code FROM aircraft_prop.engine_models em
                                             JOIN aircraft_prop.propulsion_installations pi ON em.id = pi.engine_model_id
    WHERE pi.configuration_id = cfg.id LIMIT 1
    )
    JOIN aircraft_core.aircraft_role_mappings rm ON rm.variant_id = vrt.id AND rm.priority_ranking = 1
    JOIN aircraft_ref.aircraft_roles role ON rm.role_id = role.id
    LEFT JOIN aircraft_specs.weight_limits w_lim ON cfg.id = w_lim.configuration_id
    LEFT JOIN aircraft_perf.cruise_envelopes crz ON cfg.id = crz.configuration_id
    LEFT JOIN aircraft_market.operating_cost_estimates mkt ON cfg.id = mkt.configuration_id
    LEFT JOIN (
    -- Fetch only the latest market valuation entry per configuration
    SELECT DISTINCT ON (configuration_id) configuration_id, estimated_retail_value
    FROM aircraft_market.historical_valuations
    ORDER BY configuration_id, valuation_date DESC
    ) val ON cfg.id = val.configuration_id

GROUP BY role.name, prop.code
WITH DATA;

-- Enforce rapid indexing onto the materialized summary cache table
CREATE UNIQUE INDEX idx_mv_fleet_market_summary ON mv_fleet_market_summary(market_role_segment, propulsion_framework);

COMMIT;