-- ============================================================================
-- FILE: 011_consolidated_verification_view.sql
-- DESCRIPTION: Stitches the normalized architecture into a backward-compatible flat view.
-- ============================================================================

BEGIN;

SET search_path TO public, aircraft_core, aircraft_org, aircraft_ref, aircraft_cert, aircraft_specs, aircraft_perf, aircraft_market;

DROP VIEW IF EXISTS unified_aircraft_registry;

CREATE VIEW unified_aircraft_registry AS
SELECT
    -- 1. Identity Hierarchy Contexts
    cfg.id AS configuration_id,
    oem.name AS oem_name,
    oem.slug AS oem_slug,
    fam.name AS family_name,
    fam.slug AS family_slug,
    mdl.designation AS model_designation,
    mdl.marketing_name AS model_marketing_name,
    vrt.variant_suffix,
    vrt.is_military,
    vrt.introduction_year,
    cfg.name AS configuration_name,
    cfg.slug AS configuration_slug,

    -- 2. Base Regulatory & Crewing Mandates
    p_req.minimum_crew_count,
    p_req.required_license_level,
    p_req.is_type_rating_required,
    p_req.type_rating_designator,
    p_req.requires_high_performance_endorsement,
    p_req.requires_complex_endorsement,
    p_req.requires_tailwheel_endorsement,
    ops_app.has_fiki_approval,
    ops_app.has_ifr_approval,
    ops_app.max_operating_altitude_ft,

    -- 3. External & Internal Volumetric Geometries
    ext_dim.wingspan_ft,
    ext_dim.length_ft,
    ext_dim.height_ft,
    ext_dim.wing_area_sqft,
    cab_dim.width_inches AS cabin_width_inches,
    cab_dim.height_inches AS cabin_height_inches,
    cab_dim.volume_cuft AS cabin_volume_cubic_feet,
    hgr.tail_height_ft,
    hgr.has_folding_wings,
    hgr.wingspan_folded_ft,

    -- 4. Weight Profile Boundaries (LBS)
    w_lim.basic_empty_weight_lbs,
    w_lim.operating_empty_weight_lbs,
    w_lim.max_takeoff_weight_lbs,
    w_lim.max_landing_weight_lbs,
    w_lim.max_zero_fuel_weight_lbs,
    w_lim.max_payload_lbs,
    fuel.total_capacity_gal AS fuel_total_capacity_gal,
    fuel.usable_capacity_gal AS fuel_usable_capacity_gal,
    fuel.usable_fuel_weight_lbs,

    -- 5. Aerodynamic Performance & Field Requirements
    spd.vs0_stall_flaps_down_ktas,
    spd.vs1_stall_clean_ktas,
    spd.vne_never_exceed_ktas,
    fld.takeoff_ground_roll_ft,
    fld.takeoff_total_clear_50ft_obstacle_ft,
    fld.landing_ground_roll_ft,
    fld.landing_total_clear_50ft_obstacle_ft,
    crz.max_cruise_speed_ktas,
    crz.max_range_nm,
    crz.service_ceiling_ft,

    -- 6. Financial Operating Cost Metrics
    mkt.currency_code AS operating_cost_currency,
    mkt.estimated_fuel_cost_per_hour,
    mkt.total_variable_cost_per_hour

FROM aircraft_core.aircraft_configurations cfg
         JOIN aircraft_core.aircraft_variants vrt ON cfg.variant_id = vrt.id
         JOIN aircraft_core.aircraft_models mdl ON vrt.model_id = mdl.id
         JOIN aircraft_core.aircraft_families fam ON mdl.family_id = fam.id
         JOIN aircraft_org.organizations oem ON fam.manufacturer_id = oem.id

-- Left joins insulate the structural flat view from missing secondary specification matrices
         LEFT JOIN aircraft_cert.operating_approvals ops_app ON vrt.id = ops_app.variant_id
         LEFT JOIN aircraft_cert.pilot_requirements p_req ON vrt.id = p_req.variant_id
         LEFT JOIN aircraft_specs.external_dimensions ext_dim ON cfg.id = ext_dim.configuration_id
         LEFT JOIN aircraft_specs.cabin_dimensions cab_dim ON cfg.id = cab_dim.configuration_id
         LEFT JOIN aircraft_specs.hangar_fit_dimensions hgr ON cfg.id = hgr.configuration_id
         LEFT JOIN aircraft_specs.weight_limits w_lim ON cfg.id = w_lim.configuration_id
         LEFT JOIN aircraft_specs.fuel_mass_capacities fuel ON cfg.id = fuel.configuration_id
         LEFT JOIN aircraft_perf.speed_limits spd ON cfg.id = spd.configuration_id
         LEFT JOIN aircraft_perf.field_performance fld ON cfg.id = fld.configuration_id
         LEFT JOIN aircraft_perf.cruise_envelopes crz ON cfg.id = crz.configuration_id
         LEFT JOIN aircraft_market.operating_cost_estimates mkt ON cfg.id = mkt.configuration_id;

COMMIT;