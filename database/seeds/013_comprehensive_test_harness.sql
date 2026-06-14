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
        INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Antonov Heavy Transport', 'antonov-heavy-transport') RETURNING id INTO v_fam;
END IF;
INSERT INTO aircraft_models (family_id, designation, slug) VALUES (v_fam, 'An-225', 'an-225') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Strategic Fleet Unit 1', 'mriya-hull', 'STUDY', 1988, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Max Cargo Bay Layout', 'max-cargo-bay', 0, 0, 6, 'RETRACTABLE_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative)
VALUES (v_vrt, TRUE, TRUE, 36000.00, 2.50, -1.00);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, type_rating_designator)
VALUES (v_vrt, 6, 'AIRLINE_TRANSPORT', TRUE, 'AN-225');

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 290.00, 275.60, 59.30, 9741.00);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, max_takeoff_weight_lbs, max_zero_fuel_weight_lbs)
VALUES (v_cfg, 628300.00, 1410958.00, 1102300.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 98450.00, 98000.00, 450.00, 6.70);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 108.00, 122.00, 458.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 11500.00, 9800.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 432.00, 8300, 36000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_static_thrust_lbf)
VALUES ((SELECT id FROM organizations WHERE slug = 'ivchenko-progress'), 'Progress D-18T', 'progress-d-18t', 'TURBOFAN') RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count)
VALUES (v_cfg, v_eng, 6);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 18500.00, 1200.00, 1500.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'cargo-transport'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 5: CIRRUS SR22T GTS G6
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'cirrus-aircraft');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT; v_avx BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Cirrus SR22', 'cirrus-sr22') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, marketing_name, slug) VALUES (v_fam, 'SR22T', 'SR22T GTS', 'sr22t') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Generation 6 Carbon Edition', 'g6-carbon', 'ACTIVE_PRODUCTION', 2017, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Luxury Premium Leather 5-Seat', 'premium-5-seat', 4, 4, 1, 'FIXED_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative, has_ballistic_parachute)
VALUES (v_vrt, TRUE, TRUE, 25000.00, 3.80, -1.52, TRUE);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, requires_high_performance_endorsement)
VALUES (v_vrt, 1, 'PRIVATE', FALSE, TRUE);

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 38.30, 26.00, 8.90, 144.80);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, operating_empty_weight_lbs, max_takeoff_weight_lbs)
VALUES (v_cfg, 2350.00, 2440.00, 3600.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 95.00, 92.00, 3.00, 6.01);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vy_best_rate_climb_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 60.00, 69.00, 103.00, 205.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 1517.00, 1178.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_cruise_fuel_flow_gph, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 213.00, 18.2, 1020, 25000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_horsepower)
VALUES ((SELECT id FROM organizations WHERE slug = 'continental-aerospace'), 'TSIO-550-K', 'tsio-550-k', 'PISTON', 315.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count, propeller_model, propeller_blades_count, is_constant_speed_propeller)
VALUES (v_cfg, v_eng, 1, 'Hartzell Lightweight Composite', 3, TRUE);

INSERT INTO aircraft_avionics.avionics_suites (manufacturer_id, name, slug)
VALUES ((SELECT id FROM organizations WHERE slug = 'garmin'), 'Cirrus Perspective+ by Garmin', 'perspective-plus') RETURNING id INTO v_avx;

INSERT INTO aircraft_avionics.avionics_installations (configuration_id, avionics_suite_id, is_glass_cockpit, has_autopilot, autopilot_model)
VALUES (v_cfg, v_avx, TRUE, TRUE, 'Garmin GFC 700 Digital');

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 115.00, 38.00, 30.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'general-aviation'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 6: BEECHCRAFT SUPER KING AIR 350i
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'textron-aviation-inc');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT; v_avx BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Beechcraft King Air', 'beechcraft-king-air') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, marketing_name, slug) VALUES (v_fam, 'Super King Air 350', 'King Air 350i', 'king-air-350i') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Standard Production 350i', '350i-base', 'ACTIVE_PRODUCTION', 2010, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Double-Club Executive Interior', 'double-club-exec', 9, 11, 2, 'RETRACTABLE_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, has_rvsm_approval, has_rvsm_compliant_avionics, max_operating_altitude_ft)
VALUES (v_vrt, TRUE, TRUE, TRUE, TRUE, 35000.00);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, type_rating_designator)
VALUES (v_vrt, 2, 'COMMERCIAL', TRUE, 'BE-300');

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 57.90, 46.70, 14.30, 310.00);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, max_takeoff_weight_lbs, max_landing_weight_lbs, max_zero_fuel_weight_lbs)
VALUES (v_cfg, 9955.00, 15000.00, 15000.00, 12500.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 544.00, 539.00, 5.00, 6.70);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 78.00, 84.00, 263.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 3300.00, 2692.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_cruise_fuel_flow_gph, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 312.00, 116.0, 1806, 35000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_shaft_horsepower)
VALUES ((SELECT id FROM organizations WHERE slug = 'pratt-whitney'), 'PT6A-60A', 'pt6a-60a', 'TURBOPROP', 1050.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count, propeller_model, propeller_blades_count, is_constant_speed_propeller, is_featherable_propeller)
VALUES (v_cfg, v_eng, 2, 'Hartzell 4-Blade Constant Speed', 4, TRUE, TRUE);

INSERT INTO aircraft_avionics.avionics_suites (manufacturer_id, name, slug)
VALUES ((SELECT id FROM organizations WHERE slug = 'airbus-se'), 'Rockwell Collins Pro Line 21', 'pro-line-21') RETURNING id INTO v_avx; -- Re-use mapped context cleanly

INSERT INTO aircraft_avionics.avionics_installations (configuration_id, avionics_suite_id, is_glass_cockpit, has_autopilot)
VALUES (v_cfg, v_avx, TRUE, TRUE);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 754.00, 145.00, 190.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'airliner'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 7: BOEING 737-800
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'boeing');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Boeing 737', 'boeing-737') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, marketing_name, slug) VALUES (v_fam, '737-800', '737 Next Generation', '737-800') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Blended Winglets Model', '737-800-winglets', 'ACTIVE_PRODUCTION', 1998, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Standard Two-Class Commercial Fleet Layout', 'two-class-civil', 162, 189, 2, 'RETRACTABLE_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, has_rvsm_approval, has_rvsm_compliant_avionics, max_operating_altitude_ft)
VALUES (v_vrt, TRUE, TRUE, TRUE, TRUE, 41000.00);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, type_rating_designator)
VALUES (v_vrt, 2, 'AIRLINE_TRANSPORT', TRUE, 'B737');

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 117.40, 129.50, 41.20, 1344.00);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, operating_empty_weight_lbs, max_takeoff_weight_lbs, max_landing_weight_lbs, max_zero_fuel_weight_lbs)
VALUES (v_cfg, 91300.00, 91500.00, 174200.00, 146300.00, 138300.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 6875.00, 6810.00, 65.00, 6.70);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 121.00, 135.00, 515.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 7900.00, 5350.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 461.00, 2935, 41000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_static_thrust_lbf)
VALUES ((SELECT id FROM organizations WHERE slug = 'cfm-international'), 'CFM56-7B27', 'cfm56-7b27', 'TURBOFAN', 27300.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count)
VALUES (v_cfg, v_eng, 2);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 2850.00, 450.00, 620.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'airliner'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 8: LOCKHEED MARTIN F-16C BLOCK 50
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'lockheed-martin');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Lockheed F-16 Fighting Falcon', 'f-16') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, slug) VALUES (v_fam, 'F-16C', 'f-16c') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Block 50/52 Multi-Role Fighter', 'block-50', 'ACTIVE_PRODUCTION', 1991, TRUE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Single Pilot Air Superiority Loadout', 'air-superiority', 0, 0, 1, 'RETRACTABLE_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative)
VALUES (v_vrt, TRUE, TRUE, 50000.00, 9.00, -3.50);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required)
VALUES (v_vrt, 1, 'MILITARY_ONLY', FALSE);

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 32.80, 49.30, 16.00, 300.00);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, max_takeoff_weight_lbs, max_landing_weight_lbs)
VALUES (v_cfg, 18900.00, 42300.00, 30000.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 1075.00, 1050.00, 25.00, 6.70);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 115.00, 125.00, 1147.00); -- Mach 2.0 Class Target Metric Bounds

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 2500.00, 3000.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 485.00, 2280, 50000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_static_thrust_lbf)
VALUES ((SELECT id FROM organizations WHERE slug = 'ge-aerospace'), 'General Electric F110-GE-129', 'f110-ge-129', 'TURBOFAN', 29500.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count)
VALUES (v_cfg, v_eng, 1);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 4500.00, 1200.00, 1800.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'military-fighter'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 9: ROBINSON R44 RAVEN II
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'robinson-helicopter');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Robinson R44', 'robinson-r44') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, marketing_name, slug) VALUES (v_fam, 'R44 II', 'Raven II', 'r44-raven-ii') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Fuel Injected Standard Airframe', 'raven-ii-base', 'ACTIVE_PRODUCTION', 2002, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'Utility 4-Seat Skids Layout', 'utility-skids', 3, 3, 1, 'SKIDS') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, max_operating_altitude_ft, limit_load_factor_flaps_up_positive, limit_load_factor_flaps_up_negative)
VALUES (v_vrt, FALSE, FALSE, 14000.00, 2.80, -0.50);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required)
VALUES (v_vrt, 1, 'PRIVATE', FALSE);

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, main_rotor_diameter_ft)
VALUES (v_cfg, 33.00, 38.20, 10.70, 33.00); -- Wingspan mirrors active rotor diameter boundaries cleanly

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, max_takeoff_weight_lbs)
VALUES (v_cfg, 1506.00, 2500.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 49.30, 46.50, 2.80, 6.01);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 0.00, 0.00, 130.00); -- Rotary wing displays 0 stall bounds due to structural hover envelope mechanics

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_cruise_fuel_flow_gph, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 109.00, 15.0, 300, 14000.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_horsepower)
VALUES ((SELECT id FROM organizations WHERE slug = 'lycoming-engines'), 'Lycoming IO-540-AE1A5', 'io-540-ae1a5', 'PISTON', 245.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count)
VALUES (v_cfg, v_eng, 1);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 90.00, 45.00, 40.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'helicopter'), 1);
END $$;

-- ============================================================================
-- AIRFRAME 10: AIRBUS A321NEO
-- ============================================================================
DO $$
DECLARE
v_oem BIGINT := (SELECT id FROM aircraft_org.organizations WHERE slug = 'airbus-se');
    v_fam BIGINT; v_mdl BIGINT; v_vrt BIGINT; v_cfg BIGINT; v_eng BIGINT;
BEGIN
INSERT INTO aircraft_families (manufacturer_id, name, slug) VALUES (v_oem, 'Airbus A320neo Family', 'a320neo-family') RETURNING id INTO v_fam;
INSERT INTO aircraft_models (family_id, designation, marketing_name, slug) VALUES (v_fam, 'A321neo', 'A321neo Cabin-Flex', 'a321neo') RETURNING id INTO v_mdl;
INSERT INTO aircraft_variants (model_id, variant_suffix, slug, lifecycle_status_code, introduction_year, is_military)
VALUES (v_mdl, 'Pratt & Whitney GTF Variant', 'a321neo-pw', 'ACTIVE_PRODUCTION', 2017, FALSE) RETURNING id INTO v_vrt;

INSERT INTO aircraft_configurations (variant_id, name, slug, passenger_capacity_standard, passenger_capacity_max, pilot_capacity_required, landing_gear_code)
VALUES (v_vrt, 'High Density Single Class Charter Layout', 'high-density-charter', 220, 244, 2, 'RETRACTABLE_TRICYCLE') RETURNING id INTO v_cfg;

INSERT INTO aircraft_cert.operating_approvals (variant_id, has_fiki_approval, has_ifr_approval, has_rvsm_approval, has_rvsm_compliant_avionics, max_operating_altitude_ft)
VALUES (v_vrt, TRUE, TRUE, TRUE, TRUE, 39800.00);

INSERT INTO aircraft_cert.pilot_requirements (variant_id, minimum_crew_count, required_license_level, is_type_rating_required, type_rating_designator)
VALUES (v_vrt, 2, 'AIRLINE_TRANSPORT', TRUE, 'A320');

INSERT INTO aircraft_specs.external_dimensions (configuration_id, wingspan_ft, length_ft, height_ft, wing_area_sqft)
VALUES (v_cfg, 117.40, 146.00, 38.60, 1317.00);

INSERT INTO aircraft_specs.weight_limits (configuration_id, basic_empty_weight_lbs, operating_empty_weight_lbs, max_takeoff_weight_lbs, max_landing_weight_lbs, max_zero_fuel_weight_lbs)
VALUES (v_cfg, 106700.00, 110400.00, 213800.00, 174600.00, 166000.00);

INSERT INTO aircraft_specs.fuel_mass_capacities (configuration_id, total_capacity_gal, usable_capacity_gal, unusable_capacity_gal, fuel_density_lbs_gal)
VALUES (v_cfg, 7060.00, 7000.00, 60.00, 6.70);

INSERT INTO aircraft_perf.speed_limits (configuration_id, vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas, vne_never_exceed_ktas)
VALUES (v_cfg, 123.00, 138.00, 510.00);

INSERT INTO aircraft_perf.field_performance (configuration_id, takeoff_total_clear_50ft_obstacle_ft, landing_total_clear_50ft_obstacle_ft)
VALUES (v_cfg, 6520.00, 4820.00);

INSERT INTO aircraft_perf.cruise_envelopes (configuration_id, max_cruise_speed_ktas, max_range_nm, service_ceiling_ft)
VALUES (v_cfg, 467.00, 4000, 39800.00);

INSERT INTO aircraft_prop.engine_models (manufacturer_id, name, slug, propulsion_category_code, rated_static_thrust_lbf)
VALUES ((SELECT id FROM organizations WHERE slug = 'pratt-whitney'), 'PW1133G-JM', 'pw1133g-jm', 'TURBOFAN', 33000.00) RETURNING id INTO v_eng;

INSERT INTO aircraft_prop.propulsion_installations (configuration_id, engine_model_id, engine_count)
VALUES (v_cfg, v_eng, 2);

INSERT INTO aircraft_market.operating_cost_estimates (configuration_id, currency_code, estimated_fuel_cost_per_hour, estimated_maintenance_labor_per_hour, estimated_parts_engine_reserve_per_hour)
VALUES (v_cfg, 'USD', 2410.00, 410.00, 580.00);

INSERT INTO aircraft_core.aircraft_role_mappings (variant_id, role_id, priority_ranking)
VALUES (v_vrt, (SELECT id FROM aircraft_ref.aircraft_roles WHERE slug = 'airliner'), 1);
END $$;

COMMIT;