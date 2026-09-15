-- =============================================================================
-- File: database/migrations/027_ownership_cost_summary_fuel_code.sql
-- Phase 27: Make the ownership-cost read model name a cost code that exists.
--
-- aircraft_read.mv_ownership_cost_summary filtered its fuel column on
-- 'FUEL_COST_PER_HOUR'. aircraft_ref.cost_item_types has no such code -- the
-- vocabulary spells it FUEL (database/seeds/002_lookup_seed_data.sql) -- so
-- hourly_fuel_cost_usd was structurally NULL for every row, 0 of 737 populated
-- against 2,524 FUEL line items. Its sibling
-- hourly_maintenance_reserve_usd excluded the same absent code, so the
-- exclusion never excluded anything and every fuel figure was reported as a
-- maintenance reserve. Migration 016 carries the marker that introduced it:
-- "FIX: is_fuel removed; use explicit cost_item_type_code for fuel".
--
-- 016 is hashed in database/migrations.lock.json and immutable, and a
-- materialized view has no CREATE OR REPLACE, so both views are dropped and
-- recreated. mv_variant_search is recreated verbatim: it only reads variant_id,
-- computed_total_annual_usd and cost_per_hour_usd from the summary, none of
-- which this changes, and it is rebuilt solely because it depends on the view
-- being replaced.
--
-- Both definitions and all 23 indexes were taken from pg_get_viewdef and
-- pg_indexes on an installed database rather than retyped, so the only
-- difference from 016 is the one string this migration exists to correct.
-- Neither view carries a grant today, so nothing is re-granted; add them to
-- database/roles/app_grants.sql when a runtime statement first reads one.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '15min';

DROP MATERIALIZED VIEW aircraft_read.mv_variant_search;
DROP MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary;

CREATE MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary AS
WITH latest_snapshot AS (
         SELECT DISTINCT ON (cost_snapshots.variant_id) cost_snapshots.id,
            cost_snapshots.variant_id,
            cost_snapshots.snapshot_date,
            cost_snapshots.source_name,
            cost_snapshots.condition_grade_code,
            cost_snapshots.assumed_airframe_hours,
            cost_snapshots.assumed_engine_hours_smoh,
            cost_snapshots.assumed_prop_hours_smoh,
            cost_snapshots.assumed_annual_hours,
            cost_snapshots.assumed_fuel_price_per_gal,
            cost_snapshots.assumed_fuel_burn_gph,
            cost_snapshots.currency_code,
            cost_snapshots.region_code,
            cost_snapshots.confidence,
            cost_snapshots.notes,
            cost_snapshots.extra_attributes,
            cost_snapshots.created_at
           FROM aircraft_market.cost_snapshots
          WHERE cost_snapshots.is_canonical
          ORDER BY cost_snapshots.variant_id, cost_snapshots.snapshot_date DESC, cost_snapshots.id DESC
        )
 SELECT ls.variant_id,
    ls.id AS snapshot_id,
    ls.snapshot_date,
    ls.source_name,
    ls.assumed_annual_hours,
    ls.assumed_fuel_price_per_gal,
    ls.currency_code,
    cst.total_annual_usd AS source_total_annual_usd,
    cst.total_fixed_usd AS source_total_fixed_usd,
    cst.total_variable_usd AS source_total_variable_usd,
    cst.assumed_hours AS source_assumed_hours,
    round(sum(cli.amount_annual) FILTER (WHERE cit.is_fixed = true), 2) AS total_annual_fixed_usd,
    round(sum(cli.amount_per_hour) FILTER (WHERE cit.is_fixed = false), 4) AS total_hourly_variable_usd,
    round(sum(cli.amount_per_hour) FILTER (WHERE cli.cost_item_type_code::text = 'FUEL'::text), 4) AS hourly_fuel_cost_usd,
    round(sum(cli.amount_per_hour) FILTER (WHERE cit.is_fixed = false AND cli.cost_item_type_code::text <> 'FUEL'::text), 4) AS hourly_maintenance_reserve_usd,
    round(COALESCE(sum(cli.amount_annual) FILTER (WHERE cit.is_fixed = true), 0::numeric) + COALESCE(sum(cli.amount_annual) FILTER (WHERE cit.is_fixed = false), sum(cli.amount_per_hour) FILTER (WHERE cit.is_fixed = false) * COALESCE(ls.assumed_annual_hours, cst.assumed_hours, 100::numeric), 0::numeric), 2) AS computed_total_annual_usd,
    COALESCE(cst.total_variable_usd / NULLIF(COALESCE(cst.assumed_hours, ls.assumed_annual_hours), 0::numeric), sum(cli.amount_per_hour) FILTER (WHERE cit.is_fixed = false)) AS cost_per_hour_usd
   FROM latest_snapshot ls
     LEFT JOIN aircraft_market.cost_snapshot_totals cst ON cst.snapshot_id = ls.id
     LEFT JOIN aircraft_market.cost_line_items cli ON cli.snapshot_id = ls.id
     LEFT JOIN aircraft_ref.cost_item_types cit ON cit.code::text = cli.cost_item_type_code::text
  GROUP BY ls.id, ls.variant_id, ls.snapshot_date, ls.source_name, ls.assumed_annual_hours, ls.assumed_fuel_price_per_gal, ls.currency_code, cst.total_annual_usd, cst.total_fixed_usd, cst.total_variable_usd, cst.assumed_hours
WITH NO DATA;

CREATE INDEX idx_ocs_computed_total ON aircraft_read.mv_ownership_cost_summary USING btree (computed_total_annual_usd) WHERE (computed_total_annual_usd IS NOT NULL);
CREATE INDEX idx_ocs_hourly ON aircraft_read.mv_ownership_cost_summary USING btree (cost_per_hour_usd) WHERE (cost_per_hour_usd IS NOT NULL);
CREATE UNIQUE INDEX uq_ocs_variant ON aircraft_read.mv_ownership_cost_summary USING btree (variant_id);

COMMENT ON MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary IS
    'Pre-aggregated ownership cost per variant. source_total_annual_usd '
    '(from cost_snapshot_totals) is preferred for display. '
    'computed_total_annual_usd is independently derived from cost_line_items '
    'for cross-validation. Never SUM cost_line_items directly for totals.';

CREATE MATERIALIZED VIEW aircraft_read.mv_variant_search AS
SELECT v.id AS variant_id,
    v.slug,
    v.name AS variant_name,
    v.popular_name,
    v.service_status_code,
    v.production_start_year,
    v.production_end_year,
    v.is_in_production,
    v.passenger_capacity,
    v.crew_count,
    v.engine_count AS declared_engine_count,
    v.propulsion_category_code,
    v.landing_gear_type_code,
    v.country_of_origin_code,
    m.id AS model_id,
    m.name AS model_name,
    f.id AS family_id,
    f.name AS family_name,
    pm_sub.org_id AS primary_manufacturer_id,
    pm_sub.org_name AS primary_manufacturer_name,
    pm_sub.org_slug AS primary_manufacturer_slug,
    pr_sub.role_code AS primary_role_code,
    c.name AS country_name,
    c.alpha2 AS country_alpha2,
    perf.cruise_speed_kias,
    perf.range_nm,
    perf.service_ceiling_ft,
    perf.rate_of_climb_fpm,
    perf.stall_speed_kias,
    perf.takeoff_50ft_ft,
    perf.landing_50ft_ft,
    wt.gross_weight_lb,
    wt.empty_weight_lb,
    wt.fuel_capacity_gal,
    eng.engine_count AS powerplant_count,
    eng.hp_rated AS engine_hp_rated,
    eng.thrust_lbf_dry AS engine_thrust_lbf,
    eng.model_designation AS engine_model,
    val.papi_price_estimate AS papi_price_usd,
    val.for_sale_count,
    ocs.computed_total_annual_usd AS total_annual_cost_usd,
    ocs.cost_per_hour_usd,
    ifr.is_approved AS is_ifr_approved,
    fiki.is_approved AS is_fiki_approved,
    press.is_approved AS is_pressurized,
    aero.is_approved AS is_aerobatic_approved,
    adsb.has_ads_b_out,
    to_tsvector('english'::regconfig, (((((((((((COALESCE(f.name, ''::text) || ' '::text) || COALESCE(m.name, ''::text)) || ' '::text) || COALESCE(v.name, ''::text)) || ' '::text) || COALESCE(v.popular_name, ''::text)) || ' '::text) || COALESCE(pm_sub.org_name, ''::text)) || ' '::text) || COALESCE(c.name, ''::text)) || ' '::text) || COALESCE(v.description, ''::text)) AS search_tsv
   FROM aircraft_core.variants v
     JOIN aircraft_core.models m ON m.id = v.model_id
     JOIN aircraft_core.families f ON f.id = m.family_id
     LEFT JOIN aircraft_geo.countries c ON c.code::text = v.country_of_origin_code::text
     LEFT JOIN LATERAL ( SELECT o.id AS org_id,
            o.name AS org_name,
            o.slug AS org_slug
           FROM aircraft_core.variant_manufacturers vm
             JOIN aircraft_org.organizations o ON o.id = vm.org_id
          WHERE vm.variant_id = v.id AND vm.is_primary
         LIMIT 1) pm_sub ON true
     LEFT JOIN LATERAL ( SELECT variant_roles.role_code
           FROM aircraft_core.variant_roles
          WHERE variant_roles.variant_id = v.id AND variant_roles.is_primary
         LIMIT 1) pr_sub ON true
     LEFT JOIN LATERAL ( SELECT max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'SPEED_CRUISE_BEST'::text AND performance_metrics.is_canonical) AS cruise_speed_kias,
            max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'RANGE_NORMAL'::text AND performance_metrics.is_canonical) AS range_nm,
            max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'CEILING_SERVICE'::text AND performance_metrics.is_canonical) AS service_ceiling_ft,
            max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'CLIMB_RATE_SL'::text AND performance_metrics.is_canonical) AS rate_of_climb_fpm,
            max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'SPEED_STALL_CLEAN'::text AND performance_metrics.is_canonical) AS stall_speed_kias,
            max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'DIST_TO_50FT'::text AND performance_metrics.is_canonical) AS takeoff_50ft_ft,
            max(performance_metrics.canonical_value) FILTER (WHERE performance_metrics.metric_type_code::text = 'DIST_LDG_50FT'::text AND performance_metrics.is_canonical) AS landing_50ft_ft
           FROM aircraft_specs.performance_metrics
          WHERE performance_metrics.variant_id = v.id) perf ON true
     LEFT JOIN LATERAL ( SELECT max(weight_metrics.canonical_value) FILTER (WHERE weight_metrics.metric_type_code::text = 'WEIGHT_MTOW'::text) AS gross_weight_lb,
            max(weight_metrics.canonical_value) FILTER (WHERE weight_metrics.metric_type_code::text = 'WEIGHT_EMPTY'::text) AS empty_weight_lb,
            max(weight_metrics.canonical_value) FILTER (WHERE weight_metrics.metric_type_code::text = 'FUEL_CAPACITY_USABLE'::text) AS fuel_capacity_gal
           FROM aircraft_specs.weight_metrics
          WHERE weight_metrics.variant_id = v.id AND weight_metrics.configuration IS NULL AND weight_metrics.is_canonical) wt ON true
     LEFT JOIN LATERAL ( SELECT vp.engine_count,
            ev.hp_rated,
            ev.thrust_lbf_dry,
            ev.model_designation
           FROM aircraft_power.variant_powerplants vp
             JOIN aircraft_power.engine_variants ev ON ev.id = vp.engine_variant_id
          WHERE vp.variant_id = v.id AND vp.is_primary
         LIMIT 1) eng ON true
     LEFT JOIN LATERAL ( SELECT valuations.papi_price_estimate,
            valuations.for_sale_count
           FROM aircraft_market.valuations
          WHERE valuations.variant_id = v.id AND valuations.is_canonical
          ORDER BY valuations.snapshot_date DESC, valuations.captured_at DESC, valuations.id DESC
         LIMIT 1) val ON true
     LEFT JOIN aircraft_read.mv_ownership_cost_summary ocs ON ocs.variant_id = v.id
     LEFT JOIN LATERAL ( SELECT variant_operating_approvals.is_approved
           FROM aircraft_cert.variant_operating_approvals
          WHERE variant_operating_approvals.variant_id = v.id AND variant_operating_approvals.approval_type_code::text = 'IFR'::text
         LIMIT 1) ifr ON true
     LEFT JOIN LATERAL ( SELECT variant_operating_approvals.is_approved
           FROM aircraft_cert.variant_operating_approvals
          WHERE variant_operating_approvals.variant_id = v.id AND variant_operating_approvals.approval_type_code::text = 'KNOWN_ICING_FIKI'::text
         LIMIT 1) fiki ON true
     LEFT JOIN LATERAL ( SELECT variant_operating_approvals.is_approved
           FROM aircraft_cert.variant_operating_approvals
          WHERE variant_operating_approvals.variant_id = v.id AND variant_operating_approvals.approval_type_code::text = 'PRESSURIZED'::text
         LIMIT 1) press ON true
     LEFT JOIN LATERAL ( SELECT variant_operating_approvals.is_approved
           FROM aircraft_cert.variant_operating_approvals
          WHERE variant_operating_approvals.variant_id = v.id AND variant_operating_approvals.approval_type_code::text = 'AEROBATIC'::text
         LIMIT 1) aero ON true
     LEFT JOIN LATERAL ( SELECT true AS has_ads_b_out
           FROM aircraft_systems.variant_equipment ve
             JOIN aircraft_systems.equipment_catalog ec ON ec.id = ve.equipment_id
          WHERE ve.variant_id = v.id AND ec.name ~~* '%ADS-B%'::text
         LIMIT 1) adsb ON true
WITH NO DATA;

CREATE INDEX idx_mvs_adsb ON aircraft_read.mv_variant_search USING btree (variant_id) WHERE has_ads_b_out;
CREATE INDEX idx_mvs_ceiling ON aircraft_read.mv_variant_search USING btree (service_ceiling_ft) WHERE (service_ceiling_ft IS NOT NULL);
CREATE INDEX idx_mvs_country ON aircraft_read.mv_variant_search USING btree (country_of_origin_code);
CREATE INDEX idx_mvs_cruise ON aircraft_read.mv_variant_search USING btree (cruise_speed_kias) WHERE (cruise_speed_kias IS NOT NULL);
CREATE INDEX idx_mvs_family_trgm ON aircraft_read.mv_variant_search USING gin (family_name gin_trgm_ops);
CREATE INDEX idx_mvs_fiki ON aircraft_read.mv_variant_search USING btree (variant_id) WHERE is_fiki_approved;
CREATE INDEX idx_mvs_fts ON aircraft_read.mv_variant_search USING gin (search_tsv);
CREATE INDEX idx_mvs_gear ON aircraft_read.mv_variant_search USING btree (landing_gear_type_code);
CREATE INDEX idx_mvs_gross_weight ON aircraft_read.mv_variant_search USING btree (gross_weight_lb) WHERE (gross_weight_lb IS NOT NULL);
CREATE INDEX idx_mvs_ifr ON aircraft_read.mv_variant_search USING btree (variant_id) WHERE is_ifr_approved;
CREATE INDEX idx_mvs_pax ON aircraft_read.mv_variant_search USING btree (passenger_capacity) WHERE (passenger_capacity IS NOT NULL);
CREATE INDEX idx_mvs_pressurized ON aircraft_read.mv_variant_search USING btree (variant_id) WHERE is_pressurized;
CREATE INDEX idx_mvs_price ON aircraft_read.mv_variant_search USING btree (papi_price_usd) WHERE (papi_price_usd IS NOT NULL);
CREATE INDEX idx_mvs_propulsion ON aircraft_read.mv_variant_search USING btree (propulsion_category_code);
CREATE INDEX idx_mvs_range ON aircraft_read.mv_variant_search USING btree (range_nm) WHERE (range_nm IS NOT NULL);
CREATE INDEX idx_mvs_role ON aircraft_read.mv_variant_search USING btree (primary_role_code);
CREATE INDEX idx_mvs_status ON aircraft_read.mv_variant_search USING btree (service_status_code);
CREATE INDEX idx_mvs_total_cost ON aircraft_read.mv_variant_search USING btree (total_annual_cost_usd) WHERE (total_annual_cost_usd IS NOT NULL);
CREATE INDEX idx_mvs_variant_name_trgm ON aircraft_read.mv_variant_search USING gin (variant_name gin_trgm_ops);
CREATE UNIQUE INDEX uq_mvs_variant ON aircraft_read.mv_variant_search USING btree (variant_id);

COMMENT ON MATERIALIZED VIEW aircraft_read.mv_variant_search IS
    'Primary search and faceted filter surface. '
    'Populated with NO DATA; call refresh_search_matviews() after ingestion. '
    'ADS-B sourced from aircraft_systems.variant_equipment (correct); '
    'operating approvals (IFR, FIKI, etc.) from aircraft_cert.';

-- 016 created both views WITH NO DATA and the install flow populates them before
-- anything reads them; dropping and recreating puts them back in the
-- unpopulated state, which is an error rather than an empty result for every
-- later reader -- validation 023 among them. Refreshed here, in the dependency
-- order the function already encodes, so the migration leaves them exactly as
-- populated as it found them. FALSE is non-concurrent: CONCURRENTLY cannot
-- populate a view that has never held data.
SELECT aircraft_read.refresh_search_matviews(FALSE);

COMMIT;
