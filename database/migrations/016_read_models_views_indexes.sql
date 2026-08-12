-- =============================================================================
-- File: database/migrations/016_read_models_views_indexes.sql
-- Phase 16 — aircraft_read: read models for the product front-end.
--
-- Changes from original design (post-evaluation fixes):
--   • mv_ownership_cost_summary: removed references to cit.is_fuel. The
--     is_fuel column was removed from aircraft_ref.cost_item_types in the
--     002 fix (it was unused). Fuel cost is now identified by matching
--     cost_item_type_code = 'FUEL_COST_PER_HOUR' directly.
--   • mv_variant_search: fixed the 'adsb' LATERAL subquery which was
--     identical to the 'ifr' subquery (both used approval_type_code = 'IFR').
--     ADS-B is now sourced from aircraft_systems.variant_equipment matched on
--     equipment name (ILIKE '%ADS-B%'), which captures the seeded ADS-B catalog
--     rows. ADS-B is equipment (Phase 10), not a certification approval.
--   • v_weight_criteria_validation: uses mp.title, the actual column name on
--     aircraft_compare.mission_profiles (there is no profile_name column).
-- =============================================================================

BEGIN;

-- =============================================================================
-- SUPPORTING INDEXES (Phase 16 additions)
-- =============================================================================

CREATE INDEX IF NOT EXISTS idx_pm_variant_canonical_all
    ON aircraft_specs.performance_metrics (variant_id, metric_type_code)
    WHERE is_canonical;

CREATE INDEX IF NOT EXISTS idx_wm_variant_standard
    ON aircraft_specs.weight_metrics (variant_id, metric_type_code)
    WHERE configuration IS NULL;

CREATE INDEX IF NOT EXISTS idx_vs_rank
    ON aircraft_compare.variant_suitability
        (mission_profile_id, overall_score DESC NULLS LAST, variant_id);

-- =============================================================================
-- VIEW: aircraft_read.v_current_valuation
-- =============================================================================

CREATE OR REPLACE VIEW aircraft_read.v_current_valuation AS
SELECT DISTINCT ON (variant_id)
    id,
    variant_id,
    snapshot_date,
    source_name,
    papi_price_estimate,
    for_sale_count,
    currency_code,
    region_code,
    condition_grade_code,
    confidence,
    captured_at
FROM aircraft_market.valuations
ORDER BY variant_id, snapshot_date DESC, captured_at DESC, id DESC;

COMMENT ON VIEW aircraft_read.v_current_valuation IS
    'Most recent market valuation snapshot per variant. '
    'Uses DISTINCT ON with the Phase 12 idx_val_variant_date covering index.';

-- =============================================================================
-- VIEW: aircraft_read.v_hangar_fit
-- =============================================================================

CREATE OR REPLACE VIEW aircraft_read.v_hangar_fit AS
SELECT
    v.id                                            AS variant_id,
    v.slug,
    v.name                                          AS variant_name,
    o.name                                          AS manufacturer_name,
    ws.canonical_value                              AS wingspan_ft,
    ws_fold.canonical_value                         AS wingspan_folded_ft,
    COALESCE(ws_fold.canonical_value,
             ws.canonical_value)                    AS effective_wingspan_ft,
    ht.canonical_value                              AS height_ft,
    ln.canonical_value                              AS length_ft,
    -- Common reference sizes
    ws.canonical_value <= 36  AS fits_t_hangar_36ft,
    ws.canonical_value <= 40  AS fits_t_hangar_40ft,
    ws.canonical_value <= 50  AS fits_t_hangar_50ft,
    COALESCE(ws_fold.canonical_value,
             ws.canonical_value) <= 40              AS fits_40ft_with_fold,
    ht.canonical_value <= 14  AS clears_14ft_door
FROM aircraft_core.variants v
JOIN aircraft_core.models    m  ON m.id = v.model_id
JOIN aircraft_core.families  f  ON f.id = m.family_id
LEFT JOIN aircraft_org.organizations o ON o.id = f.manufacturer_org_id
LEFT JOIN aircraft_specs.dimension_metrics ws
    ON ws.variant_id = v.id AND ws.metric_type_code = 'DIM_WINGSPAN'
    AND ws.configuration IS NULL
LEFT JOIN aircraft_specs.dimension_metrics ws_fold
    ON ws_fold.variant_id = v.id AND ws_fold.metric_type_code = 'DIM_WINGSPAN'
    AND ws_fold.configuration = 'WINGS_FOLDED'
LEFT JOIN aircraft_specs.dimension_metrics ht
    ON ht.variant_id = v.id AND ht.metric_type_code = 'DIM_HEIGHT'
    AND ht.configuration IS NULL
LEFT JOIN aircraft_specs.dimension_metrics ln
    ON ln.variant_id = v.id AND ln.metric_type_code = 'DIM_LENGTH'
    AND ln.configuration IS NULL;

COMMENT ON VIEW aircraft_read.v_hangar_fit IS
    'Hangar clearance analysis from dimension_metrics. '
    'Standard T-hangar reference: 40 ft wide, 14 ft door height. '
    'effective_wingspan_ft uses folded wingspan when available.';

-- =============================================================================
-- VIEW: aircraft_read.v_weight_criteria_validation
-- Uses mp.title (the actual mission_profiles column).
-- =============================================================================

CREATE OR REPLACE VIEW aircraft_read.v_weight_criteria_validation AS
SELECT
    mp.slug,
    mp.title,
    count(mc.id)                         AS criterion_count,
    round(sum(mc.weight), 4)             AS weight_sum,
    abs(sum(mc.weight) - 1.0) < 0.0001  AS weights_sum_to_one
FROM aircraft_compare.mission_profiles mp
LEFT JOIN aircraft_compare.mission_criteria mc ON mc.mission_profile_id = mp.id
WHERE mp.is_active
GROUP BY mp.id, mp.slug, mp.title
ORDER BY mp.sort_order;

COMMENT ON VIEW aircraft_read.v_weight_criteria_validation IS
    'QA view: mission_criteria weights must sum to 1.0000 per active profile. '
    'weights_sum_to_one should be TRUE for all rows. '
    'Run after any change to mission_criteria.';

-- =============================================================================
-- MATERIALIZED VIEW: aircraft_read.mv_ownership_cost_summary
-- FIX: removed cit.is_fuel references; fuel identified by cost_item_type_code
-- =============================================================================

CREATE MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary AS
WITH latest_snapshot AS (
    SELECT DISTINCT ON (variant_id) *
    FROM aircraft_market.cost_snapshots
    ORDER BY variant_id, snapshot_date DESC, id DESC
)
SELECT
    ls.variant_id,
    ls.id                                                               AS snapshot_id,
    ls.snapshot_date,
    ls.source_name,
    ls.assumed_annual_hours,
    ls.assumed_fuel_price_per_gal,
    ls.currency_code,
    -- Source-provided totals from cost_snapshot_totals (preferred for display)
    cst.total_annual_usd                                                AS source_total_annual_usd,
    cst.total_fixed_usd                                                 AS source_total_fixed_usd,
    cst.total_variable_usd                                              AS source_total_variable_usd,
    cst.assumed_hours                                                   AS source_assumed_hours,
    -- Computed totals from line items
    -- FIX: is_fuel removed; use explicit cost_item_type_code for fuel
    round(SUM(cli.amount_annual)
        FILTER (WHERE cit.is_fixed = TRUE), 2)                         AS total_annual_fixed_usd,
    round(SUM(cli.amount_per_hour)
        FILTER (WHERE cit.is_fixed = FALSE), 4)                        AS total_hourly_variable_usd,
    round(SUM(cli.amount_per_hour)
        FILTER (WHERE cli.cost_item_type_code = 'FUEL_COST_PER_HOUR'), 4)
                                                                        AS hourly_fuel_cost_usd,
    round(SUM(cli.amount_per_hour)
        FILTER (WHERE cit.is_fixed = FALSE
                  AND cli.cost_item_type_code <> 'FUEL_COST_PER_HOUR'), 4)
                                                                        AS hourly_maintenance_reserve_usd,
    -- Fully computed annual total (use when source total unavailable)
    round(
        COALESCE(SUM(cli.amount_annual) FILTER (WHERE cit.is_fixed = TRUE), 0)
        + COALESCE(
            SUM(cli.amount_annual)   FILTER (WHERE cit.is_fixed = FALSE),
            SUM(cli.amount_per_hour) FILTER (WHERE cit.is_fixed = FALSE)
                * COALESCE(ls.assumed_annual_hours, cst.assumed_hours, 100)
        , 0),
        2
    )                                                                   AS computed_total_annual_usd,
    -- Hourly cost: prefer source-derived, fall back to computed
    COALESCE(
        cst.total_variable_usd
            / NULLIF(COALESCE(cst.assumed_hours, ls.assumed_annual_hours), 0),
        SUM(cli.amount_per_hour) FILTER (WHERE cit.is_fixed = FALSE)
    )                                                                   AS cost_per_hour_usd
FROM latest_snapshot ls
LEFT JOIN aircraft_market.cost_snapshot_totals cst ON cst.snapshot_id = ls.id
LEFT JOIN aircraft_market.cost_line_items cli ON cli.snapshot_id = ls.id
LEFT JOIN aircraft_ref.cost_item_types cit ON cit.code = cli.cost_item_type_code
GROUP BY
    ls.id, ls.variant_id, ls.snapshot_date, ls.source_name,
    ls.assumed_annual_hours, ls.assumed_fuel_price_per_gal, ls.currency_code,
    cst.total_annual_usd, cst.total_fixed_usd, cst.total_variable_usd, cst.assumed_hours
WITH NO DATA;

CREATE UNIQUE INDEX uq_ocs_variant
    ON aircraft_read.mv_ownership_cost_summary (variant_id);
CREATE INDEX idx_ocs_computed_total
    ON aircraft_read.mv_ownership_cost_summary (computed_total_annual_usd)
    WHERE computed_total_annual_usd IS NOT NULL;
CREATE INDEX idx_ocs_hourly
    ON aircraft_read.mv_ownership_cost_summary (cost_per_hour_usd)
    WHERE cost_per_hour_usd IS NOT NULL;

COMMENT ON MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary IS
    'Pre-aggregated ownership cost per variant. source_total_annual_usd '
    '(from cost_snapshot_totals) is preferred for display. '
    'computed_total_annual_usd is independently derived from cost_line_items '
    'for cross-validation. Never SUM cost_line_items directly for totals.';

-- =============================================================================
-- MATERIALIZED VIEW: aircraft_read.mv_variant_search
-- FIX: adsb lateral now queries aircraft_systems.variant_equipment instead
--      of duplicating the IFR operating approval lateral.
--      ADS-B is equipment (Phase 10), not a certification approval (Phase 5).
-- FIX: column references corrected to match base tables (organizations.name,
--      countries.name / countries.alpha2).
-- =============================================================================

CREATE MATERIALIZED VIEW aircraft_read.mv_variant_search AS
SELECT
    v.id                                AS variant_id,
    v.slug,
    v.name                              AS variant_name,
    v.popular_name,
    v.service_status_code,
    v.production_start_year,
    v.production_end_year,
    v.is_in_production,
    v.passenger_capacity,
    v.crew_count,
    v.engine_count                      AS declared_engine_count,
    v.propulsion_category_code,
    v.landing_gear_type_code,
    v.country_of_origin_code,

    -- Model and family
    m.id                                AS model_id,
    m.name                              AS model_name,
    f.id                                AS family_id,
    f.name                              AS family_name,

    -- Primary manufacturer
    pm_sub.org_id                       AS primary_manufacturer_id,
    pm_sub.org_name                     AS primary_manufacturer_name,
    pm_sub.org_slug                     AS primary_manufacturer_slug,

    -- Primary role
    pr_sub.role_code                    AS primary_role_code,

    -- Country
    c.name                              AS country_name,
    c.alpha2                            AS country_alpha2,

    -- Performance (canonical; single scan via FILTER)
    perf.cruise_speed_kias,
    perf.range_nm,
    perf.service_ceiling_ft,
    perf.rate_of_climb_fpm,
    perf.stall_speed_kias,
    perf.takeoff_50ft_ft,
    perf.landing_50ft_ft,

    -- Weights
    wt.gross_weight_lb,
    wt.empty_weight_lb,
    wt.fuel_capacity_gal,

    -- Primary powerplant
    eng.engine_count                    AS powerplant_count,
    eng.hp_rated                        AS engine_hp_rated,
    eng.thrust_lbf_dry                  AS engine_thrust_lbf,
    eng.model_designation               AS engine_model,

    -- Market
    val.papi_price_estimate             AS papi_price_usd,
    val.for_sale_count,

    -- Cost summary
    ocs.computed_total_annual_usd       AS total_annual_cost_usd,
    ocs.cost_per_hour_usd,

    -- Operating approvals
    ifr.is_approved                     AS is_ifr_approved,
    fiki.is_approved                    AS is_fiki_approved,
    press.is_approved                   AS is_pressurized,
    aero.is_approved                    AS is_aerobatic_approved,
    -- FIX: ADS-B sourced from equipment catalog (Phase 10), not cert approvals
    adsb.has_ads_b_out,

    -- Full-text search vector
    to_tsvector('english',
        coalesce(f.name,             '') || ' ' ||
        coalesce(m.name,             '') || ' ' ||
        coalesce(v.name,             '') || ' ' ||
        coalesce(v.popular_name,     '') || ' ' ||
        coalesce(pm_sub.org_name,    '') || ' ' ||
        coalesce(c.name,             '') || ' ' ||
        coalesce(v.description,      '')
    )                                   AS search_tsv

FROM aircraft_core.variants v
JOIN aircraft_core.models    m  ON m.id = v.model_id
JOIN aircraft_core.families  f  ON f.id = m.family_id
LEFT JOIN aircraft_geo.countries c ON c.code = v.country_of_origin_code

LEFT JOIN LATERAL (
    SELECT o.id AS org_id, o.name AS org_name, o.slug AS org_slug
    FROM aircraft_core.variant_manufacturers vm
    JOIN aircraft_org.organizations o ON o.id = vm.org_id
    WHERE vm.variant_id = v.id AND vm.is_primary
    LIMIT 1
) pm_sub ON TRUE

LEFT JOIN LATERAL (
    SELECT role_code
    FROM aircraft_core.variant_roles
    WHERE variant_id = v.id AND is_primary
    LIMIT 1
) pr_sub ON TRUE

LEFT JOIN LATERAL (
    SELECT
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'CRUISE_SPEED'
                                       AND is_canonical)    AS cruise_speed_kias,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'RANGE'
                                       AND is_canonical)    AS range_nm,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'SERVICE_CEILING'
                                       AND is_canonical)    AS service_ceiling_ft,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'RATE_OF_CLIMB'
                                       AND is_canonical)    AS rate_of_climb_fpm,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'STALL_SPEED_CLEAN'
                                       AND is_canonical)    AS stall_speed_kias,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'TAKEOFF_DISTANCE_50FT'
                                       AND is_canonical)    AS takeoff_50ft_ft,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'LANDING_DISTANCE_50FT'
                                       AND is_canonical)    AS landing_50ft_ft
    FROM aircraft_specs.performance_metrics
    WHERE variant_id = v.id
) perf ON TRUE

LEFT JOIN LATERAL (
    SELECT
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'WEIGHT_GROSS_MTOW') AS gross_weight_lb,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'WEIGHT_EMPTY')      AS empty_weight_lb,
        MAX(canonical_value) FILTER (WHERE metric_type_code = 'FUEL_CAPACITY')     AS fuel_capacity_gal
    FROM aircraft_specs.weight_metrics
    WHERE variant_id = v.id AND configuration IS NULL
) wt ON TRUE

LEFT JOIN LATERAL (
    SELECT vp.engine_count, ev.hp_rated, ev.thrust_lbf_dry, ev.model_designation
    FROM aircraft_power.variant_powerplants vp
    JOIN aircraft_power.engine_variants ev ON ev.id = vp.engine_variant_id
    WHERE vp.variant_id = v.id AND vp.is_primary
    LIMIT 1
) eng ON TRUE

LEFT JOIN LATERAL (
    SELECT papi_price_estimate, for_sale_count
    FROM aircraft_market.valuations
    WHERE variant_id = v.id
    ORDER BY snapshot_date DESC, captured_at DESC, id DESC
    LIMIT 1
) val ON TRUE

LEFT JOIN aircraft_read.mv_ownership_cost_summary ocs ON ocs.variant_id = v.id

-- Certification approvals
LEFT JOIN LATERAL (
    SELECT is_approved FROM aircraft_cert.variant_operating_approvals
    WHERE variant_id = v.id AND approval_type_code = 'IFR'
    LIMIT 1
) ifr ON TRUE

LEFT JOIN LATERAL (
    SELECT is_approved FROM aircraft_cert.variant_operating_approvals
    WHERE variant_id = v.id AND approval_type_code = 'KNOWN_ICING_FIKI'
    LIMIT 1
) fiki ON TRUE

LEFT JOIN LATERAL (
    SELECT is_approved FROM aircraft_cert.variant_operating_approvals
    WHERE variant_id = v.id AND approval_type_code = 'PRESSURIZED'
    LIMIT 1
) press ON TRUE

LEFT JOIN LATERAL (
    SELECT is_approved FROM aircraft_cert.variant_operating_approvals
    WHERE variant_id = v.id AND approval_type_code = 'AEROBATIC'
    LIMIT 1
) aero ON TRUE

-- FIX: ADS-B from equipment, not approvals (was a duplicate of the IFR lateral).
-- Match on equipment name (captures both 'Garmin GTX 345R ADS-B Transponder'
-- and the 'ADS-B Out (Generic)' placeholder); the previously-referenced slug
-- 'ads-b-out-transponder' does not exist in equipment_catalog.
LEFT JOIN LATERAL (
    SELECT TRUE AS has_ads_b_out
    FROM aircraft_systems.variant_equipment ve
    JOIN aircraft_systems.equipment_catalog ec ON ec.id = ve.equipment_id
    WHERE ve.variant_id = v.id
      AND ec.name ILIKE '%ADS-B%'
    LIMIT 1
) adsb ON TRUE

WITH NO DATA;

-- Required for CONCURRENT refresh
CREATE UNIQUE INDEX uq_mvs_variant
    ON aircraft_read.mv_variant_search (variant_id);

-- Full-text search
CREATE INDEX idx_mvs_fts
    ON aircraft_read.mv_variant_search USING gin (search_tsv);

-- Trigram search
CREATE INDEX idx_mvs_family_trgm
    ON aircraft_read.mv_variant_search USING gin (family_name gin_trgm_ops);
CREATE INDEX idx_mvs_variant_name_trgm
    ON aircraft_read.mv_variant_search USING gin (variant_name gin_trgm_ops);

-- Facet filters
CREATE INDEX idx_mvs_propulsion
    ON aircraft_read.mv_variant_search (propulsion_category_code);
CREATE INDEX idx_mvs_status
    ON aircraft_read.mv_variant_search (service_status_code);
CREATE INDEX idx_mvs_country
    ON aircraft_read.mv_variant_search (country_of_origin_code);
CREATE INDEX idx_mvs_role
    ON aircraft_read.mv_variant_search (primary_role_code);
CREATE INDEX idx_mvs_gear
    ON aircraft_read.mv_variant_search (landing_gear_type_code);

-- Range filters
CREATE INDEX idx_mvs_cruise
    ON aircraft_read.mv_variant_search (cruise_speed_kias)
    WHERE cruise_speed_kias IS NOT NULL;
CREATE INDEX idx_mvs_range
    ON aircraft_read.mv_variant_search (range_nm)
    WHERE range_nm IS NOT NULL;
CREATE INDEX idx_mvs_ceiling
    ON aircraft_read.mv_variant_search (service_ceiling_ft)
    WHERE service_ceiling_ft IS NOT NULL;
CREATE INDEX idx_mvs_gross_weight
    ON aircraft_read.mv_variant_search (gross_weight_lb)
    WHERE gross_weight_lb IS NOT NULL;
CREATE INDEX idx_mvs_price
    ON aircraft_read.mv_variant_search (papi_price_usd)
    WHERE papi_price_usd IS NOT NULL;
CREATE INDEX idx_mvs_pax
    ON aircraft_read.mv_variant_search (passenger_capacity)
    WHERE passenger_capacity IS NOT NULL;
CREATE INDEX idx_mvs_total_cost
    ON aircraft_read.mv_variant_search (total_annual_cost_usd)
    WHERE total_annual_cost_usd IS NOT NULL;

-- Boolean approval partial indexes
CREATE INDEX idx_mvs_ifr
    ON aircraft_read.mv_variant_search (variant_id)
    WHERE is_ifr_approved;
CREATE INDEX idx_mvs_fiki
    ON aircraft_read.mv_variant_search (variant_id)
    WHERE is_fiki_approved;
CREATE INDEX idx_mvs_pressurized
    ON aircraft_read.mv_variant_search (variant_id)
    WHERE is_pressurized;
CREATE INDEX idx_mvs_adsb
    ON aircraft_read.mv_variant_search (variant_id)
    WHERE has_ads_b_out;

COMMENT ON MATERIALIZED VIEW aircraft_read.mv_variant_search IS
    'Primary search and faceted filter surface. '
    'Populated with NO DATA; call refresh_search_matviews() after ingestion. '
    'ADS-B sourced from aircraft_systems.variant_equipment (correct); '
    'operating approvals (IFR, FIKI, etc.) from aircraft_cert.';

-- =============================================================================
-- REFRESH FUNCTION
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_read.refresh_search_matviews(
    p_concurrently BOOLEAN DEFAULT TRUE
)
RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    IF p_concurrently THEN
        REFRESH MATERIALIZED VIEW CONCURRENTLY aircraft_read.mv_ownership_cost_summary;
        REFRESH MATERIALIZED VIEW CONCURRENTLY aircraft_read.mv_variant_search;
    ELSE
        REFRESH MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary;
        REFRESH MATERIALIZED VIEW aircraft_read.mv_variant_search;
    END IF;
END;
$$;

COMMENT ON FUNCTION aircraft_read.refresh_search_matviews(BOOLEAN) IS
    'Refreshes mv_ownership_cost_summary then mv_variant_search. '
    'FALSE = non-concurrent (required for first population). '
    'TRUE (default) = CONCURRENT, does not block reads. '
    'Always refresh ocs first; mv_variant_search JOINs into it.';

COMMIT;