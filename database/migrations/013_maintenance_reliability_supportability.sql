-- ============================================================================
-- FILE: 013_comprehensive_test_harness.sql
-- DESCRIPTION: High-fidelity transactional population of 10 validation airframes.
-- ============================================================================

BEGIN;

-- Establish target namespaces for clean insertion resolution
SET search_path TO aircraft_core, aircraft_org, aircraft_ref, aircraft_cert, aircraft_specs, aircraft_perf, aircraft_prop, aircraft_avionics, aircraft_market, aircraft_geo, public;

-- ----------------------------------------------------------------------------
-- STEP 1: EXTEND CORPORATE LOGISTICS DIRECTORY
-- ----------------------------------------------------------------------------
INSERT INTO aircraft_org.organizations (name, slug, org_type_code, headquarters_country_code) VALUES
                                                                                                  ('Cirrus Aircraft Corporation', 'cirrus-aircraft', 'OEM', 'USA'),
                                                                                                  ('The Boeing Company', 'boeing', 'OEM', 'USA'),
                                                                                                  ('Lockheed Martin Corporation', 'lockheed-martin', 'OEM', 'USA'),
                                                                                                  ('Robinson Helicopter Company', 'robinson-helicopter', 'OEM', 'USA'),
                                                                                                  ('Airbus SE', 'airbus-se', 'OEM', 'FRA'),
                                                                                                  ('Continental Aerospace Technologies', 'continental-aerospace', 'OEM', 'USA'),
                                                                                                  ('Pratt & Whitney', 'pratt-whitney', 'OEM', 'USA'),
                                                                                                  ('CFM International', 'cfm-international', 'OEM', 'FRA'),
                                                                                                  ('Garmin International, Inc.', 'garmin', 'OEM', 'USA'),
                                                                                                  ('General Electric Aerospace', 'ge-aerospace', 'OEM', 'USA'),
                                                                                                  ('Ivchenko-Progress', 'ivchenko-progress', 'OEM', 'UKR'),
                                                                                                  ('Lycoming Engines', 'lycoming', 'OEM', 'USA');

INSERT INTO aircraft_org.organization_aliases (organization_id, alias_name, slug, is_primary_trade_name) VALUES
                                                                                                             ((SELECT id FROM aircraft_org.organizations WHERE slug = 'cirrus-aircraft'), 'CIRRUS', 'cirrus', TRUE),
                                                                                                             ((SELECT id FROM aircraft_org.organizations WHERE slug = 'boeing'), 'BOEING', 'boeing', TRUE),
                                                                                                             ((SELECT id FROM aircraft_org.organizations WHERE slug = 'lockheed-martin'), 'LOCKHEED MARTIN', 'lockheed-martin', TRUE),
                                                                                                             ((SELECT id FROM aircraft_org.organizations WHERE slug = 'robinson-helicopter'), 'ROBINSON', 'robinson', TRUE),
                                                                                                             ((SELECT id FROM aircraft_org.organizations WHERE slug = 'airbus-se'), 'AIRBUS', 'airbus', TRUE);

-- ----------------------------------------------------------------------------
-- STEP 2: TRANSACTIONAL INJECTION PIPELINE FOR THE 10 VALIDATION FLEET ENTRIES
-- ----------------------------------------------------------------------------

-- ============================================================================
-- AIRFRAME 1: CESSNA 172S SKYHAWK SP
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'textron-aviation-inc');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT; v_avx BIGINT;
BEGIN
SELECT id INTO v_fam FROM aircraft_families WHERE slug = 'cessna-172' AND manufacturer_id = v_oem;
SELECT id INTO v_mdl FROM aircraft_models WHERE slug = '172-skyhawk-series' AND family_id = v_fam;
SELECT id INTO v_vrt FROM aircraft_variants WHERE slug = '172s' AND model_id = v_mdl;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Garmin G1000 Factory Standard', 'g1000-factory-standard', 3, 3, 1, 'FIXED_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative)
VALUES (v_vrt, FALSE, TRUE, 14000.00, 3.80, -1.52);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required)
VALUES (v_vrt, 1, 'PRIVATE', FALSE);

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_raw, wingspan_ft, length_raw, length_ft, height_raw, height_ft, wing_area_sqft, aspect_ratio)
VALUES (v_cfg, '36 ft 1 in', 36.08, '27 ft 2 in', 27.17, '8 ft 11 in', 8.92, 174.00, 7.48);

INSERT INTO aircraft_specs.cabin_dimensions (configuration_id, length_ft, width_inches, height_inches, volume_cuft, is_pressurized, has_flat_floor)
VALUES (v_cfg, 11.80, 39.50, 48.00, 122.00, FALSE, FALSE);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, operating_empty_weight_lbs, max_takeoff_weight_lbs, max_landing_weight_lbs, max_zero_fuel_weight_lbs)
VALUES (v_cfg, 1665.00, 1715.00, 2550.00, 2550.00, 2300.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 56.00, 53.00, 3.00, 6.01); -- AvGas Density Baseline

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vx_best_angle_climb_ktas, vy_best_rate_climb_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 40.00, 48.00, 62.00, 74.00, 163.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_ground_roll_ft, takeoff_total_clear_50ft_obstacle_ft, landing_ground_roll_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 960.00, 1630.00, 575.00, 1335.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_cruise_fuel_flow_gph, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 124.00, 10.5, 640, 14000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_horsepower, time_between_overhauls_hours)
VALUES ((SELECT id FROM organizations WHERE slug = 'lycoming-engines'), 'IO-360-L2A', 'io-360-l2a', 'PISTON', 180.00, 2000) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count, propeller_model, propeller_blades_count, is_constant_speed_propeller)
VALUES (v_cfg, v_eng, 1, 'McCauley 1A170E/JHA7660', 2, FALSE);

INSERT INTO aircraft_avionics.avionics_suites (manufacturer_id, name, slug)
VALUES ((SELECT id FROM organizations WHERE slug = 'garmin'), 'Garmin G1000 Legacy', 'garmin-g1000-legacy') RETURNING id INTO v_avx;

INSERT INTO aircraft_avionics.avionics_installations (configuration_id, avionics_suite_id, is_glass_cockpit, has_autopilot, autopilot_model)
VALUES (v_cfg, v_avx, TRUE, TRUE, 'BendixKing KAP 140');

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 63.00, 22.00, 18.00);
END $$;

-- ============================================================================
-- AIRFRAME 2: AERONCA 11AC CHIEF
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'aeronca-aircraft-corporation');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Aeronca Chief', 'aeronca-chief') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, marketing_name, slug) VALUES (v_fam, '11AC', 'Chief', '11ac') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Base Model', 'base', 'DISCONTINUED', 1946, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Standard Vintage Mechanical', 'standard-vintage', 1, 1, 1, 'FIXED_TAILWHEEL') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative)
VALUES (v_vrt, FALSE, FALSE, 10800.00, 4.40, -1.76);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, requires_tailwheel_endorsement)
VALUES (v_vrt, 1, 'SPORT', FALSE, TRUE);

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_raw, wingspan_ft, length_raw, length_ft, height_raw, height_ft, wing_area_sqft)
VALUES (v_cfg, '36 ft 0 in', 36.00, '20 ft 10 in', 20.83, '7 ft 0 in', 7.00, 175.00);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, operating_empty_weight_lbs, max_takeoff_weight_lbs)
VALUES (v_cfg, 725.00, 750.00, 1250.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 15.00, 14.00, 1.00, 6.01);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 33.00, 33.00, 112.00); -- Clean stall equal because it lacks structural flap deployment assemblies

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 850.00, 900.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_cruise_fuel_flow_gph, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 83.00, 4.5, 280, 10800.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_horsepower)
VALUES ((SELECT id FROM organizations WHERE slug = 'continental-aerospace'), 'Continental A-65', 'continental-a-65', 'PISTON', 65.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count, propeller_model, propeller_blades_count, is_constant_speed_propeller)
VALUES (v_cfg, v_eng, 1, 'Sensenich Fixed Wood', 2, FALSE);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 27.00, 15.00, 10.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'light-sport'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 3: AERO VODOCHODY L-39C ALBATROS
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'aero-vodochody-aerospace');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
SELECT id INTO v_fam FROM aircraft_families WHERE slug = 'l-39 Albatross' AND manufacturer_id = v_oem;
IF v_fam IS NULL THEN
        INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'L-39 Albatros', 'l-39-albatros') RETURNING id INTO v_fam;
END IF;
INSERT INTO aircraft_models (family_id, designation, slug) VALUES (v_fam, 'L-39C', 'l-39c') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Standard Soviet Trainer Config', 'soviet-trainer', 'DISCONTINUED', 1971, TRUE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Dual Pilot Tactical Cockpit', 'dual-pilot-tactical', 1, 1, 1, 'RETRACTABLE_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative)
VALUES (v_vrt, TRUE, TRUE, 36100.00, 8.00, -4.00);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, type_rating_designator)
VALUES (v_vrt, 1, 'MILITARY_ONLY', TRUE, 'L-39');

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 31.02, 39.75, 15.62, 202.40);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, max_takeoff_weight_lbs, max_landing_weight_lbs)
VALUES (v_cfg, 7837.00, 10362.00, 9480.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 332.00, 324.00, 8.00, 6.74); -- Jet-A Fuel Density Matrix Target

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 89.00, 97.00, 491.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 2625.00, 2460.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 405.00, 593, 36100.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_static_thrust_lbf)
VALUES ((SELECT id FROM organizations WHERE slug = 'ivchenko-progress'), 'Lotarev AI-25TL', 'lotarev-ai-25tl', 'TURBOJET') RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count)
VALUES (v_cfg, v_eng, 1);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 950.00, 180.00, 220.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'trainer'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 4: ANTONOV AN-225 MRIYA
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'antonov-astc');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
SELECT id INTO v_fam FROM aircraft_families WHERE slug = 'antonov' AND manufacturer_id = v_oem;
IF v_fam IS NULL THEN
        INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (