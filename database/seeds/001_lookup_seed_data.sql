-- =============================================================================
-- File: database/seeds/001_lookup_seed_data.sql
-- Phase 2 — seed rows for all aircraft_ref lookup tables EXCEPT
-- unit_categories and measurement_units (see seeds/002_reference_units.sql).
--
-- Dependency order within this file:
--   Group 2  (taxonomy)         → no intra-file FKs
--   Group 3  (physical)         → propulsion_categories.primary_power_unit
--                                  FK to measurement_units (already committed)
--   Group 4  (metric types)     → canonical_unit_code FK to measurement_units
--   Group 5  (certification)    → airworthiness_categories and
--                                  pilot_certificate_types FK to
--                                  certification_authorities (same file)
--   Group 6  (military)         → stores_types FK to weapon_categories
--   Group 7  (market)           → no intra-file FKs
--   Group 8  (maintenance)      → no intra-file FKs
--   Group 9  (provenance)       → no intra-file FKs
--   Group 10 (comparison)       → comparison_criterion_types FKs to
--                                  performance/weight/dimension_metric_types
--                                  (same file, inserted above)
--   Group 11 (organization)     → no intra-file FKs
--   Group 12 (systems)          → no intra-file FKs
--
-- All cross-table FKs in this file are DEFERRABLE INITIALLY DEFERRED;
-- the constraint batch-validates at COMMIT, by which time all referenced
-- rows exist within this transaction or from prior committed scripts.
-- =============================================================================

BEGIN;

-- =============================================================================
-- GROUP 2: AIRCRAFT TAXONOMY
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.aircraft_roles (54 rows)
-- role_group is a non-FK informational grouping for display clustering.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.aircraft_roles (code, label, role_group, sort_order) VALUES

                                                                                  -- Civilian: Commercial air transport
                                                                                  ('AIRLINER_NARROWBODY',        'Narrowbody Airliner',              'CIVILIAN_COMMERCIAL', 10),
                                                                                  ('AIRLINER_WIDEBODY',          'Widebody Airliner',                'CIVILIAN_COMMERCIAL', 11),
                                                                                  ('AIRLINER_REGIONAL',          'Regional Jet',                     'CIVILIAN_COMMERCIAL', 12),
                                                                                  ('AIRLINER_COMMUTER',          'Commuter / Turboprop Airliner',    'CIVILIAN_COMMERCIAL', 13),
                                                                                  ('CARGO_FREIGHTER',            'Dedicated Cargo Freighter',        'CIVILIAN_COMMERCIAL', 14),
                                                                                  ('CARGO_COMBI',                'Combi (Pax + Cargo)',              'CIVILIAN_COMMERCIAL', 15),

                                                                                  -- Civilian: Business aviation
                                                                                  ('BUSINESS_JET_HEAVY',         'Heavy Business Jet',               'CIVILIAN_BUSINESS',   20),
                                                                                  ('BUSINESS_JET_MIDSIZE',       'Midsize Business Jet',             'CIVILIAN_BUSINESS',   21),
                                                                                  ('BUSINESS_JET_LIGHT',         'Light Business Jet',               'CIVILIAN_BUSINESS',   22),
                                                                                  ('BUSINESS_JET_VLJ',           'Very Light Jet (VLJ)',             'CIVILIAN_BUSINESS',   23),
                                                                                  ('TURBOPROP_EXECUTIVE',        'Executive Turboprop',              'CIVILIAN_BUSINESS',   24),

                                                                                  -- Civilian: General aviation
                                                                                  ('GENERAL_AVIATION_TOURING',   'GA Touring',                       'CIVILIAN_GA',         30),
                                                                                  ('GENERAL_AVIATION_TRAINING',  'Flight Training',                  'CIVILIAN_GA',         31),
                                                                                  ('LIGHT_SPORT',                'Light Sport Aircraft (LSA)',       'CIVILIAN_GA',         32),
                                                                                  ('ULTRALIGHT',                 'Ultralight / Microlight',          'CIVILIAN_GA',         33),
                                                                                  ('HOMEBUILT_EXPERIMENTAL',     'Homebuilt / Experimental',         'CIVILIAN_GA',         34),
                                                                                  ('AEROBATIC',                  'Aerobatic',                        'CIVILIAN_GA',         35),
                                                                                  ('GLIDER_MOTORGLIDER',         'Glider / Motorglider',             'CIVILIAN_GA',         36),

                                                                                  -- Civilian: Special mission (fixed-wing)
                                                                                  ('AGRICULTURAL_SPRAY',         'Agricultural / Aerial Application','CIVILIAN_SPECIAL',    40),
                                                                                  ('MEDEVAC_AIR_AMBULANCE',      'Medical / Air Ambulance',          'CIVILIAN_SPECIAL',    41),
                                                                                  ('SEARCH_AND_RESCUE_CIVIL',    'Search and Rescue (Civil)',        'CIVILIAN_SPECIAL',    42),
                                                                                  ('AERIAL_SURVEY',              'Aerial Survey / Remote Sensing',   'CIVILIAN_SPECIAL',    43),
                                                                                  ('FIREFIGHTING_AIR_TANKER',    'Firefighting / Air Tanker',        'CIVILIAN_SPECIAL',    44),
                                                                                  ('LAW_ENFORCEMENT',            'Law Enforcement / Police',         'CIVILIAN_SPECIAL',    45),
                                                                                  ('FLOAT_SEAPLANE',             'Float-plane / Seaplane',           'CIVILIAN_SPECIAL',    46),
                                                                                  ('FLYING_BOAT',                'Flying Boat',                      'CIVILIAN_SPECIAL',    47),

                                                                                  -- Rotary wing: Civilian
                                                                                  ('ROTORCRAFT_HELICOPTER_CIVIL','Civil Helicopter',                 'ROTARY_CIVIL',        50),
                                                                                  ('ROTORCRAFT_AUTOGYRO',        'Autogyro / Gyrocopter',           'ROTARY_CIVIL',        51),
                                                                                  ('TILTROTOR_CIVIL',            'Civil Tiltrotor',                  'ROTARY_CIVIL',        52),

                                                                                  -- UAV: Civilian
                                                                                  ('UAV_CIVIL_FIXED_WING',       'Civil UAV / RPAS (Fixed Wing)',    'UAV_CIVIL',           55),
                                                                                  ('UAV_CIVIL_ROTARY',           'Civil UAV / RPAS (Rotary Wing)',   'UAV_CIVIL',           56),

                                                                                  -- Military: Fixed-wing combat
                                                                                  ('MILITARY_FIGHTER_AIR_SUP',   'Fighter — Air Superiority',       'MILITARY_FIXED_WING', 60),
                                                                                  ('MILITARY_FIGHTER_MULTIROLE', 'Fighter — Multirole',             'MILITARY_FIXED_WING', 61),
                                                                                  ('MILITARY_FIGHTER_INTERCEPT', 'Fighter — Interceptor',            'MILITARY_FIXED_WING', 62),
                                                                                  ('MILITARY_ATTACK_CAS',        'Attack — Close Air Support',      'MILITARY_FIXED_WING', 63),
                                                                                  ('MILITARY_ATTACK_STRIKE',     'Attack — Strike / Deep Strike',   'MILITARY_FIXED_WING', 64),
                                                                                  ('MILITARY_BOMBER_STRATEGIC',  'Bomber — Strategic',               'MILITARY_FIXED_WING', 65),
                                                                                  ('MILITARY_BOMBER_TACTICAL',   'Bomber — Tactical / Dive',        'MILITARY_FIXED_WING', 66),

                                                                                  -- Military: Transport and support
                                                                                  ('MILITARY_TRANSPORT_STRATEGIC','Transport — Strategic Lift',      'MILITARY_FIXED_WING', 70),
                                                                                  ('MILITARY_TRANSPORT_TACTICAL', 'Transport — Tactical / Assault',  'MILITARY_FIXED_WING', 71),
                                                                                  ('MILITARY_TRANSPORT_UTILITY',  'Transport — Utility / Light Lift','MILITARY_FIXED_WING', 72),
                                                                                  ('MILITARY_TANKER_REFUELER',   'Aerial Refuelling Tanker',        'MILITARY_FIXED_WING', 73),
                                                                                  ('MILITARY_AEW_AWACS',         'AEW / AWACS / C2 Platform',       'MILITARY_FIXED_WING', 74),
                                                                                  ('MILITARY_ELECTRONIC_WARFARE','Electronic Warfare / Attack',      'MILITARY_FIXED_WING', 75),
                                                                                  ('MILITARY_MARITIME_PATROL',   'Maritime Patrol / MPA',           'MILITARY_FIXED_WING', 76),
                                                                                  ('MILITARY_ANTISUBMARINE',     'Anti-Submarine Warfare (ASW)',     'MILITARY_FIXED_WING', 77),
                                                                                  ('MILITARY_RECONNAISSANCE',    'Reconnaissance / ISR',             'MILITARY_FIXED_WING', 78),

                                                                                  -- Military: Training
                                                                                  ('MILITARY_TRAINER_BASIC',     'Trainer — Basic / Primary',       'MILITARY_FIXED_WING', 80),
                                                                                  ('MILITARY_TRAINER_ADVANCED',  'Trainer — Advanced',               'MILITARY_FIXED_WING', 81),
                                                                                  ('MILITARY_TRAINER_COMBAT',    'Trainer — Lead-In Fighter (LIFT)', 'MILITARY_FIXED_WING', 82),
                                                                                  ('MILITARY_SPECIAL_OPS',       'Special Operations Aviation',      'MILITARY_FIXED_WING', 83),

                                                                                  -- Military: Rotary wing
                                                                                  ('MILITARY_HELICOPTER_ATTACK', 'Attack Helicopter',                'MILITARY_ROTARY',     85),
                                                                                  ('MILITARY_HELICOPTER_TRANSPORT','Transport Helicopter',           'MILITARY_ROTARY',     86),
                                                                                  ('MILITARY_HELICOPTER_ASW',    'ASW / Naval Helicopter',           'MILITARY_ROTARY',     87),
                                                                                  ('MILITARY_HELICOPTER_SAR',    'Combat SAR / Rescue Helicopter',   'MILITARY_ROTARY',     88),

                                                                                  -- Military: UAV
                                                                                  ('MILITARY_UAV_SURVEILLANCE',  'Military UAV — Surveillance/ISR', 'MILITARY_UAV',        90),
                                                                                  ('MILITARY_UAV_STRIKE',        'Military UAV — Strike / UCAV',    'MILITARY_UAV',        91)

    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.service_statuses (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.service_statuses (code, label, description, sort_order) VALUES
                                                                                     ('IN_PRODUCTION',    'In Production',
                                                                                      'Currently manufactured and delivered to customers.',                        10),
                                                                                     ('LIMITED_PRODUCTION','Limited Production',
                                                                                      'Production ongoing but limited in volume or available only to order.',      20),
                                                                                     ('DISCONTINUED',     'Discontinued',
                                                                                      'Production has permanently ended; type still actively operated.',           30),
                                                                                     ('RETIRED',          'Retired',
                                                                                      'No longer in regular service; museum or static examples only.',             40),
                                                                                     ('EXPERIMENTAL',     'Experimental',
                                                                                      'Prototype, X-plane, or test article; not type-certificated for service.',   50),
                                                                                     ('ON_ORDER',         'On Order / In Development',
                                                                                      'In development with orders placed; not yet in service.',                    60),
                                                                                     ('UNKNOWN',          'Unknown',
                                                                                      'Production / service status not established from available sources.',       99)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.variant_types (9 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.variant_types (code, label, description, sort_order) VALUES
                                                                                  ('PRODUCTION_STANDARD', 'Standard Production',
                                                                                   'Primary commercially offered production configuration.',                    10),
                                                                                  ('PROTOTYPE',           'Prototype',
                                                                                   'Pre-production development aircraft.',                                      20),
                                                                                  ('PRE_PRODUCTION',      'Pre-Production',
                                                                                   'Early series build; not to full production standard.',                      25),
                                                                                  ('EXPORT',              'Export Variant',
                                                                                   'Modified configuration for export customers.',                              30),
                                                                                  ('MILITARY_CONVERSION', 'Military Conversion',
                                                                                   'Civilian design adapted for military use.',                                 40),
                                                                                  ('CIVIL_CONVERSION',    'Civil Conversion',
                                                                                   'Military design adapted for civilian use.',                                 41),
                                                                                  ('STC_CONVERSION',      'STC / Aftermarket Conversion',
                                                                                   'Major conversion under a Supplemental Type Certificate.',                   50),
                                                                                  ('SPECIAL_MISSION',     'Special Mission',
                                                                                   'Factory-modified for surveillance, medevac, or other special purposes.',    60),
                                                                                  ('CLASSIC_SERIES',      'Classic / Legacy Series',
                                                                                   'Historical production series representing an earlier generation.',          70)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 3: PHYSICAL PROPERTY LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.landing_gear_types (10 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.landing_gear_types (code, label, description, sort_order) VALUES
                                                                                       ('FIXED_TRICYCLE',       'Fixed Tricycle',
                                                                                        'Fixed nosewheel tricycle undercarriage.',                                   10),
                                                                                       ('RETRACTABLE_TRICYCLE', 'Retractable Tricycle',
                                                                                        'Retractable nosewheel tricycle undercarriage.',                             20),
                                                                                       ('FIXED_TAILWHEEL',      'Fixed Tailwheel (Conventional)',
                                                                                        'Fixed conventional / taildragger undercarriage.',                           30),
                                                                                       ('RETRACTABLE_TAILWHEEL','Retractable Tailwheel',
                                                                                        'Retractable conventional / taildragger undercarriage.',                     40),
                                                                                       ('AMPHIBIOUS',           'Amphibious',
                                                                                        'Retractable gear plus hull or floats; land and water operations.',          50),
                                                                                       ('FLOATS',               'Floats',
                                                                                        'Pontoon floats; water operations only.',                                    60),
                                                                                       ('SKIS',                 'Skis',
                                                                                        'Ski undercarriage for snow and ice surfaces.',                              70),
                                                                                       ('SKIDS',                'Skids',
                                                                                        'Skid undercarriage (helicopter, VTOL).',                                    80),
                                                                                       ('TAILSKID',             'Tailskid',
                                                                                        'Fixed or retractable tailskid; no main tailwheel.',                        85),
                                                                                       ('NONE_GLIDER',          'None (Glider / Sailplane)',
                                                                                        'Mono-wheel or no gear; glider and motorglider convention.',                 90)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.propulsion_categories (11 rows)
-- primary_power_unit FK to measurement_units (already committed via
-- seeds/002_reference_units.sql); constraint is DEFERRABLE.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.propulsion_categories
(code, label, description, is_jet, is_rotating, primary_power_unit, sort_order)
VALUES
    ('PISTON_RECIPROCATING', 'Piston / Reciprocating',
     'Conventional 4-stroke piston engines.',                FALSE, TRUE,  'HP',   10),
    ('PISTON_ROTARY',        'Rotary (Wankel)',
     'Rotary / Wankel piston engines.',                      FALSE, TRUE,  'HP',   11),
    ('TURBOPROP',            'Turboprop',
     'Turbine engine driving a propeller via reduction gearbox.', FALSE, TRUE, 'SHP', 20),
    ('TURBOJET',             'Turbojet',
     'Pure turbojet; no bypass flow.',                        TRUE,  FALSE, 'LBF',  30),
    ('TURBOFAN_LOW_BPR',     'Low-Bypass Turbofan',
     'Turbofan with bypass ratio < 4:1.',                    TRUE,  FALSE, 'LBF',  31),
    ('TURBOFAN_HIGH_BPR',    'High-Bypass Turbofan',
     'Turbofan with bypass ratio ≥ 4:1.',                    TRUE,  FALSE, 'LBF',  32),
    ('TURBOSHAFT',           'Turboshaft',
     'Turbine outputting shaft power; primary rotorcraft propulsion.', FALSE, TRUE, 'SHP', 40),
    ('ELECTRIC',             'Electric Motor',
     'Battery or fuel-cell electric propulsion.',             FALSE, TRUE,  'KW',   50),
    ('HYBRID_ELECTRIC',      'Hybrid Electric',
     'Combined conventional and electric propulsion.',        FALSE, TRUE,  'HP',   51),
    ('ROCKET',               'Rocket Motor',
     'Rocket propulsion (X-planes, some experimental aircraft).', TRUE, FALSE, 'LBF', 60),
    ('NONE_GLIDER',          'None (Unpowered Glider)',
     'Unpowered glider or sailplane.',                        FALSE, FALSE, NULL,   99)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.fuel_types (10 rows)
-- density_lbs_per_gal at standard conditions; NULL for non-liquid fuels.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.fuel_types
(code, label, description, density_lbs_per_gal, sort_order)
VALUES
    ('AVGAS_100LL', '100LL Avgas',
     'Aviation gasoline, low-lead, 100 octane. Most common piston fuel.',        6.02, 10),
    ('AVGAS_100',   '100/130 Avgas',
     'Aviation gasoline, 100/130 octane (legacy grade).',                        6.02, 11),
    ('JET_A',       'Jet-A',
     'Kerosene-based turbine fuel; freeze point –40 °C. Common outside Russia.', 6.70, 20),
    ('JET_A1',      'Jet-A1',
     'Jet-A with lower freeze point (–47 °C); standard outside North America.',  6.70, 21),
    ('JET_B',       'Jet-B / JP-4',
     'Wide-cut gasoline-kerosene blend; used in cold-weather environments.',      6.50, 22),
    ('JP_8',        'JP-8',
     'Military turbine fuel (NATO F-34); Jet-A1 with additives.',                6.70, 23),
    ('DIESEL_ASTM', 'Diesel / ASTM D975',
     'Automotive or ASTM D975 diesel for certified diesel-cycle piston engines.', 7.05, 30),
    ('MOGAS',       'Mogas (Automotive Gasoline)',
     'Automotive unleaded gasoline under STC-approved installations.',            6.15, 40),
    ('ELECTRIC',    'Electric (Battery / Fuel Cell)',
     'Energy stored in batteries or generated by fuel cell; no liquid fuel.',     NULL, 50),
    ('HYDROGEN',    'Hydrogen (LH2 / GH2)',
     'Cryogenic liquid or compressed gaseous hydrogen.',                          NULL, 60)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 4: METRIC TYPE LOOKUPS
-- canonical_unit_code FKs reference measurement_units (committed earlier).
-- All FKs are DEFERRABLE INITIALLY DEFERRED; validated at COMMIT of this block.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.performance_metric_types (25 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.performance_metric_types
(code, label, canonical_unit_code,
 is_higher_better, is_speed, is_distance, is_rate, sort_order)
VALUES
    -- Speed
    ('SPEED_VNE',          'Never-Exceed Speed (Vne)',          'KNOTS', FALSE, TRUE,  FALSE, FALSE, 10),
    ('SPEED_MAX',          'Maximum Speed (Vmax)',               'KNOTS', TRUE,  TRUE,  FALSE, FALSE, 11),
    ('SPEED_CRUISE_HIGH',  'High-Speed Cruise',                  'KNOTS', TRUE,  TRUE,  FALSE, FALSE, 12),
    ('SPEED_CRUISE_BEST',  'Best Cruise Speed',                  'KNOTS', TRUE,  TRUE,  FALSE, FALSE, 13),
    ('SPEED_CRUISE_LRC',   'Long-Range Cruise Speed',            'KNOTS', TRUE,  TRUE,  FALSE, FALSE, 14),
    ('SPEED_CRUISE_ECON',  'Economy Cruise Speed',               'KNOTS', TRUE,  TRUE,  FALSE, FALSE, 15),
    ('SPEED_MMO',          'Maximum Operating Mach (Mmo)',       'MACH',  FALSE, TRUE,  FALSE, FALSE, 16),
    ('SPEED_MACH_CRUISE',  'Typical Cruise Mach Number',         'MACH',  TRUE,  TRUE,  FALSE, FALSE, 17),
    ('SPEED_STALL_CLEAN',  'Stall Speed — Clean (VS1)',          'KNOTS', FALSE, TRUE,  FALSE, FALSE, 20),
    ('SPEED_STALL_LANDING','Stall Speed — Landing Config (VS0)', 'KNOTS', FALSE, TRUE,  FALSE, FALSE, 21),
    -- Climb
    ('CLIMB_RATE_SL',      'Rate of Climb — Sea Level',          'FPM',   TRUE,  FALSE, FALSE, TRUE,  30),
    ('CLIMB_RATE_OEI',     'Rate of Climb — One Engine Inop.',   'FPM',   TRUE,  FALSE, FALSE, TRUE,  31),
    -- Ceilings
    ('CEILING_SERVICE',    'Service Ceiling',                    'FT',    TRUE,  FALSE, FALSE, FALSE, 40),
    ('CEILING_ABSOLUTE',   'Absolute Ceiling',                   'FT',    TRUE,  FALSE, FALSE, FALSE, 41),
    ('CEILING_OEI',        'Ceiling — One Engine Inoperative',   'FT',    TRUE,  FALSE, FALSE, FALSE, 42),
    -- Range and endurance
    ('RANGE_NORMAL',       'Range — Normal Cruise',              'NM',    TRUE,  FALSE, TRUE,  FALSE, 50),
    ('RANGE_MAX_FUEL',     'Range — Maximum Fuel',               'NM',    TRUE,  FALSE, TRUE,  FALSE, 51),
    ('RANGE_FERRY',        'Ferry Range',                        'NM',    TRUE,  FALSE, TRUE,  FALSE, 52),
    ('RANGE_COMBAT_RADIUS','Combat Radius',                      'NM',    TRUE,  FALSE, TRUE,  FALSE, 53),
    ('ENDURANCE_HRS',      'Maximum Endurance',                  'HRS',   TRUE,  FALSE, FALSE, FALSE, 54),
    -- Runway distances (canonical unit FT, same code as altitude FT)
    ('DIST_TO_GROUND_ROLL','Takeoff Ground Roll',                'FT',    FALSE, FALSE, TRUE,  FALSE, 60),
    ('DIST_TO_50FT',       'Takeoff Distance over 50 ft',        'FT',    FALSE, FALSE, TRUE,  FALSE, 61),
    ('DIST_LDG_GROUND_ROLL','Landing Ground Roll',               'FT',    FALSE, FALSE, TRUE,  FALSE, 62),
    ('DIST_LDG_50FT',      'Landing Distance over 50 ft',        'FT',    FALSE, FALSE, TRUE,  FALSE, 63),
    -- Fuel consumption
    ('FUEL_BURN_CRUISE',   'Fuel Burn — Best Cruise Setting',    'GPH',   FALSE, FALSE, FALSE, TRUE,  70)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.weight_metric_types (17 rows)
-- LOAD_FACTOR_POS / NEG are dimensionless; canonical_unit_code = NULL.
-- WING_LOADING / POWER_LOADING are derived ratios stored as LBS for sorting.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.weight_metric_types
(code, label, description, canonical_unit_code, sort_order)
VALUES
    ('WEIGHT_EMPTY',          'Basic Empty Weight',
     'Standard empty weight per manufacturer published data.',             'LBS',    10),
    ('WEIGHT_OEW',            'Operating Empty Weight (OEW)',
     'BEW plus standard equipment, unusable fuel, and full oil.',          'LBS',    11),
    ('WEIGHT_MTOW',           'Max Takeoff Weight (MTOW)',
     'Maximum certificated gross takeoff weight.',                         'LBS',    20),
    ('WEIGHT_MLW',            'Max Landing Weight (MLW)',
     'Maximum certificated landing weight.',                               'LBS',    21),
    ('WEIGHT_MZFW',           'Max Zero-Fuel Weight (MZFW)',
     'MTOW minus minimum required fuel; limits structural bending.',       'LBS',    22),
    ('WEIGHT_MRW',            'Max Ramp Weight (MRW)',
     'Maximum allowable weight before taxi; includes taxi fuel.',          'LBS',    23),
    ('WEIGHT_USEFUL_LOAD',    'Useful Load',
     'MTOW minus BEW; pilot + pax + baggage + fuel.',                      'LBS',    30),
    ('WEIGHT_PAYLOAD',        'Payload',
     'Useful load minus full fuel; revenue or passenger weight.',           'LBS',    31),
    ('WEIGHT_PAYLOAD_FULL_FUEL','Payload with Full Fuel',
     'Remaining payload at maximum usable fuel load.',                     'LBS',    32),
    ('WEIGHT_BAGGAGE_MAX',    'Max Baggage Weight',
     'Maximum certificated baggage compartment weight.',                   'LBS',    33),
    ('FUEL_CAPACITY_TOTAL',   'Total Fuel Capacity',
     'Total tank volume including unusable fuel.',                         'US_GAL', 40),
    ('FUEL_CAPACITY_USABLE',  'Usable Fuel Capacity',
     'Fuel volume available for flight; excludes unusable.',               'US_GAL', 41),
    ('FUEL_WEIGHT_MAX',       'Max Usable Fuel Weight',
     'Weight of maximum usable fuel at standard density.',                 'LBS',    42),
    ('WING_LOADING',          'Wing Loading (MTOW / Wing Area)',
     'MTOW divided by wing reference area; lbs/sq ft.',                   'LBS',    50),
    ('POWER_LOADING',         'Power Loading (MTOW / Total HP)',
     'MTOW divided by total installed power; lbs/hp.',                    'LBS',    51),
    ('LOAD_FACTOR_POS',       'Positive Load Factor Limit',
     'Maximum positive g-loading certificated limit.',                     NULL,     60),
    ('LOAD_FACTOR_NEG',       'Negative Load Factor Limit',
     'Maximum negative g-loading certificated limit.',                     NULL,     61)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.dimension_metric_types (17 rows)
-- DIM_ASPECT_RATIO is dimensionless (no canonical unit).
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.dimension_metric_types
(code, label, description, canonical_unit_code, sort_order)
VALUES
    ('DIM_WINGSPAN',         'Wingspan (tip to tip)',
     'Overall wing span from tip to tip.',                                 'FT',    10),
    ('DIM_WINGSPAN_FOLDED',  'Wingspan — Folded',
     'Wing span in folded configuration (carrier aircraft).',              'FT',    11),
    ('DIM_LENGTH',           'Overall Length',
     'Overall fuselage length nose to tail.',                              'FT',    12),
    ('DIM_HEIGHT',           'Overall Height (to tail)',
     'Ground-to-tail height on standard gear.',                            'FT',    13),
    ('DIM_WING_AREA',        'Wing Reference Area',
     'Gross planform reference wing area.',                                'SQ_FT', 20),
    ('DIM_ASPECT_RATIO',     'Wing Aspect Ratio',
     'Span² divided by reference area; dimensionless.',                    NULL,    21),
    ('DIM_ROTOR_DIAMETER',   'Main Rotor Diameter',
     'Main rotor tip-to-tip diameter (helicopter).',                       'FT',    25),
    ('DIM_PROP_DIAMETER',    'Propeller Diameter',
     'Propeller disc diameter.',                                           'FT',    26),
    ('DIM_CABIN_LENGTH',     'Cabin Interior Length',
     'Interior pressurised or habitable cabin length.',                    'FT',    30),
    ('DIM_CABIN_WIDTH',      'Cabin Interior Width (max)',
     'Maximum interior width of the cabin.',                               'FT',    31),
    ('DIM_CABIN_HEIGHT',     'Cabin Interior Height (max)',
     'Maximum interior standing height of the cabin.',                     'FT',    32),
    ('DIM_BAGGAGE_VOLUME',   'Baggage Compartment Volume',
     'Total accessible baggage compartment volume.',                       'CU_FT', 40),
    ('DIM_CARGO_VOLUME',     'Cargo Hold Volume',
     'Total volumetric capacity of cargo hold (freighters).',              'CU_FT', 41),
    ('DIM_CARGO_DOOR_WIDTH', 'Cargo Door Width',
     'Clear opening width of the main cargo door.',                        'FT',    42),
    ('DIM_CARGO_DOOR_HEIGHT','Cargo Door Height',
     'Clear opening height of the main cargo door.',                       'FT',    43),
    ('DIM_WHEELBASE',        'Wheelbase',
     'Longitudinal distance from nose gear to main gear.',                 'FT',    50),
    ('DIM_TRACK_WIDTH',      'Main Gear Track Width',
     'Lateral distance between main gear contact points.',                 'FT',    51)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 5: CERTIFICATION LOOKUPS
-- airworthiness_categories and pilot_certificate_types FK to
-- certification_authorities inserted first in this group.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.certification_authorities (8 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.certification_authorities
(code, label, full_name, country_codes, website_url, sort_order)
VALUES
    ('FAA',    'FAA',
     'Federal Aviation Administration',
     ARRAY['USA'], 'https://www.faa.gov',         10),
    ('EASA',   'EASA',
     'European Union Aviation Safety Agency',
     ARRAY['EUE'], 'https://www.easa.europa.eu',  11),
    ('TCCA',   'TCCA',
     'Transport Canada Civil Aviation',
     ARRAY['CAN'], 'https://tc.canada.ca',         12),
    ('CASA',   'CASA',
     'Civil Aviation Safety Authority (Australia)',
     ARRAY['AUS'], 'https://www.casa.gov.au',      13),
    ('CAA_UK', 'CAA UK',
     'Civil Aviation Authority (United Kingdom)',
     ARRAY['GBR'], 'https://www.caa.co.uk',        14),
    ('CAAC',   'CAAC',
     'Civil Aviation Administration of China',
     ARRAY['CHN'], 'https://www.caac.gov.cn',      15),
    ('DGCA',   'DGCA',
     'Directorate General of Civil Aviation (India)',
     ARRAY['IND'], 'https://dgca.gov.in',          16),
    ('ANAC',   'ANAC',
     'Agência Nacional de Aviação Civil (Brazil)',
     ARRAY['BRA'], 'https://www.anac.gov.br',      17)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.airworthiness_categories (9 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.airworthiness_categories
(code, label, description, authority_code, sort_order)
VALUES
    ('FAA_NORMAL',       'FAA Normal Category',
     'FAR Part 23 Normal; positive load factor +3.8g.',               'FAA',  10),
    ('FAA_UTILITY',      'FAA Utility Category',
     'FAR Part 23 Utility; limited aerobatics (spins, chandelles).',  'FAA',  11),
    ('FAA_ACROBATIC',    'FAA Acrobatic Category',
     'FAR Part 23 Acrobatic; full aerobatic operations approved.',     'FAA',  12),
    ('FAA_TRANSPORT',    'FAA Transport Category',
     'FAR Part 25 Transport; large / air-carrier aircraft.',           'FAA',  13),
    ('FAA_LSA',          'FAA Light Sport',
     'ASTM consensus standard; MTOW ≤ 1 320 lbs (600 kg).',           'FAA',  14),
    ('FAA_EXPERIMENTAL', 'FAA Experimental',
     'Experimental airworthiness; not certificated for public transport.','FAA',15),
    ('EASA_CS23_NORMAL', 'EASA CS-23 Normal',
     'EASA CS-23 Amendment 5 Normal category.',                        'EASA', 20),
    ('EASA_CS25',        'EASA CS-25 Transport',
     'EASA CS-25 Transport category (large aircraft).',                'EASA', 21),
    ('MILITARY_SPEC',    'Military Specification',
     'Airworthiness governed by applicable MIL-SPEC.',                 NULL,   30)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.pilot_certificate_types (8 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.pilot_certificate_types
(code, label, description, authority_code, sort_order)
VALUES
    ('FAA_STUDENT',      'Student Pilot',
     'FAA student pilot certificate; supervised solo operations only.', 'FAA',  10),
    ('FAA_SPORT',        'Sport Pilot',
     'FAA sport pilot; limited to LSA in VMC.',                        'FAA',  11),
    ('FAA_RECREATIONAL', 'Recreational Pilot',
     'FAA recreational pilot certificate.',                            'FAA',  12),
    ('FAA_PRIVATE',      'Private Pilot (PPL)',
     'FAA private pilot licence; VFR and IFR with rating.',            'FAA',  13),
    ('FAA_COMMERCIAL',   'Commercial Pilot (CPL)',
     'FAA commercial pilot certificate; compensation and hire.',        'FAA',  14),
    ('FAA_ATP',          'Airline Transport Pilot (ATP)',
     'FAA ATP; required as PIC on air-carrier turbine aircraft.',       'FAA',  15),
    ('FAA_TYPE_RATING',  'Type Rating',
     'Aircraft-specific type rating required on some complex types.',   'FAA',  16),
    ('EASA_PPL',         'EASA PPL',
     'EASA Part-FCL private pilot licence.',                           'EASA', 20)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 6: MILITARY LOOKUPS
-- stores_types.weapon_category_code FKs weapon_categories inserted above.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.military_mission_types (12 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.military_mission_types (code, label, sort_order) VALUES
                                                                              ('AIR_SUPERIORITY',   'Air Superiority',                  10),
                                                                              ('CLOSE_AIR_SUPPORT', 'Close Air Support (CAS)',           20),
                                                                              ('DEEP_STRIKE',       'Deep Strike / Interdiction',        30),
                                                                              ('STRATEGIC_BOMBING', 'Strategic Bombing',                 40),
                                                                              ('MARITIME_PATROL',   'Maritime Patrol / ASW',            50),
                                                                              ('AIRBORNE_ISR',      'Airborne ISR / Reconnaissance',    60),
                                                                              ('AEW_C2',            'Airborne Early Warning / C2',      70),
                                                                              ('ELECTRONIC_WARFARE','Electronic Attack / Warfare',      80),
                                                                              ('AERIAL_REFUELLING', 'Aerial Refuelling',                90),
                                                                              ('STRATEGIC_AIRLIFT', 'Strategic Airlift',               100),
                                                                              ('TACTICAL_AIRLIFT',  'Tactical Airlift',                110),
                                                                              ('SPECIAL_OPERATIONS','Special Operations Aviation',      120)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.weapon_categories (9 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.weapon_categories (code, label, sort_order) VALUES
                                                                         ('AIR_TO_AIR',     'Air-to-Air Munitions',         10),
                                                                         ('AIR_TO_GROUND',  'Air-to-Ground Munitions',       20),
                                                                         ('ANTI_SHIP',      'Anti-Ship Munitions',           30),
                                                                         ('ANTI_SUBMARINE', 'Anti-Submarine Weapons',        40),
                                                                         ('GUNS_CANNON',    'Guns and Cannon',               50),
                                                                         ('EXTERNAL_STORES','External Stores (Non-Weapon)',  60),
                                                                         ('SENSOR_POD',     'Sensor / Surveillance Pod',     70),
                                                                         ('ELECTRONIC',     'Electronic Warfare Pod',        80),
                                                                         ('SPECIAL_WEAPON', 'Special Weapon (Nuclear/CBRN)', 90)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.hardpoint_position_types (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.hardpoint_position_types (code, label, sort_order) VALUES
                                                                                ('WING_INBOARD',        'Wing — Inboard',        10),
                                                                                ('WING_MIDBOARD',       'Wing — Mid-board',       20),
                                                                                ('WING_OUTBOARD',       'Wing — Outboard',        30),
                                                                                ('WINGTIP',             'Wingtip',                40),
                                                                                ('FUSELAGE_CENTERLINE', 'Fuselage Centreline',    50),
                                                                                ('FUSELAGE_CONFORMAL',  'Fuselage — Conformal',   60),
                                                                                ('INTERNAL_BAY',        'Internal Weapons Bay',   70)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.stores_types (12 rows)
-- weapon_category_code FK to weapon_categories (inserted above).
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.stores_types
(code, label, weapon_category_code, sort_order)
VALUES
    ('AAM_SHORT_RANGE',  'Short-Range AAM',           'AIR_TO_AIR',     10),
    ('AAM_MEDIUM_RANGE', 'Medium-Range AAM (BVR)',     'AIR_TO_AIR',     11),
    ('AAM_LONG_RANGE',   'Long-Range AAM',             'AIR_TO_AIR',     12),
    ('AGM_LASER',        'Laser-Guided AGM',           'AIR_TO_GROUND',  20),
    ('LGB',              'Laser-Guided Bomb (LGB)',    'AIR_TO_GROUND',  21),
    ('JDAM',             'GPS-Guided Bomb (JDAM)',     'AIR_TO_GROUND',  22),
    ('UNGUIDED_BOMB',    'Unguided / Iron Bomb',       'AIR_TO_GROUND',  23),
    ('ROCKET_POD',       'Unguided Rocket Pod',        'AIR_TO_GROUND',  24),
    ('GUN_POD',          'Gun / Cannon Pod',           'GUNS_CANNON',    30),
    ('EXT_FUEL_TANK',    'External Fuel Tank (Drop)',  'EXTERNAL_STORES',40),
    ('RECCE_POD',        'Reconnaissance Pod',         'SENSOR_POD',     50),
    ('ECM_POD',          'Electronic Warfare Pod',     'ELECTRONIC',     60)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 7: MARKET / COST LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.currencies (6 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.currencies
(code, label, symbol, decimal_places)
VALUES
    ('USD', 'US Dollar',       '$',   2),
    ('EUR', 'Euro',            '€',   2),
    ('GBP', 'British Pound',   '£',   2),
    ('CAD', 'Canadian Dollar', 'CA$', 2),
    ('AUD', 'Australian Dollar','A$', 2),
    ('CHF', 'Swiss Franc',     'CHF', 2)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.cost_item_types (18 rows)
-- is_fixed TRUE = annual fixed cost; FALSE = per flight-hour variable cost.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.cost_item_types
(code, label, description, is_fixed, is_fuel, sort_order)
VALUES
    -- Fixed annual costs
    ('ANNUAL_INSPECTION',  'Annual Inspection',
     'Regulatory annual airworthiness inspection.',                  TRUE,  FALSE, 10),
    ('INSURANCE',          'Insurance',
     'Hull and liability insurance premium.',                        TRUE,  FALSE, 11),
    ('HANGAR_STORAGE',     'Hangar / Tiedown / Storage',
     'Monthly or annual aircraft storage fee.',                      TRUE,  FALSE, 12),
    ('DEPRECIATION',       'Depreciation',
     'Annual market-value reduction of the airframe.',               TRUE,  FALSE, 13),
    ('WEATHER_SERVICE',    'Weather / Data Services',
     'Weather briefing subscriptions and navigation database fees.',  TRUE,  FALSE, 14),
    ('PILOT_TRAINING',     'Pilot Training / Currency',
     'Recurrent training, simulator, and check-ride costs.',         TRUE,  FALSE, 15),
    ('REFURBISHING',       'Refurbishing / Modernisation',
     'Interior, paint, and avionics update reserves.',               TRUE,  FALSE, 16),
    ('REGISTRATION_TAXES', 'Registration / Taxes',
     'Annual aircraft registration and excise taxes.',               TRUE,  FALSE, 17),
    ('FINANCING',          'Financing Costs',
     'Loan interest or equivalent finance charges.',                 TRUE,  FALSE, 18),
    -- Per-hour variable costs
    ('FUEL',               'Fuel',
     'Direct fuel cost per flight hour.',                            FALSE, TRUE,  30),
    ('OIL',                'Oil',
     'Engine oil consumption per flight hour.',                      FALSE, FALSE, 31),
    ('HOURLY_MAINTENANCE', 'Scheduled Maintenance',
     'Line maintenance and labour per flight hour.',                 FALSE, FALSE, 32),
    ('UNSCHEDULED_MAINT',  'Unscheduled Maintenance',
     'Reserve for unscheduled repairs and AOG situations.',          FALSE, FALSE, 33),
    ('ENGINE_RESERVE',     'Engine Overhaul Reserve',
     'Per-hour accrual toward TBO overhaul cost.',                   FALSE, FALSE, 34),
    ('PROP_RESERVE',       'Propeller Reserve',
     'Per-hour accrual toward propeller overhaul.',                  FALSE, FALSE, 35),
    ('AVIONICS_RESERVE',   'Avionics Reserve',
     'Per-hour accrual for avionics maintenance and upgrades.',      FALSE, FALSE, 36),
    ('LANDING_FEES',       'Landing / Navigation Fees',
     'Airport landing fees averaged per flight hour.',               FALSE, FALSE, 37),
    ('MISC_VARIABLE',      'Miscellaneous Variable',
     'Catering, ground handling, parking, and sundry costs.',        FALSE, FALSE, 38)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.aircraft_condition_grades (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.aircraft_condition_grades
(code, label, description, numeric_score, sort_order)
VALUES
    ('EXCELLENT', 'Excellent',
     'Like-new or recently refurbished; all systems fully serviceable.',   5, 10),
    ('GOOD',      'Good',
     'Well-maintained; minor cosmetic wear only.',                         4, 20),
    ('FAIR',      'Fair',
     'Serviceable with some deferred maintenance or cosmetic wear.',       3, 30),
    ('POOR',      'Poor',
     'Operational but significant maintenance required.',                  2, 40),
    ('SALVAGE',   'Salvage',
     'Not airworthy; parts-only or rebuild project.',                      1, 50)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 8: MAINTENANCE LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.ad_types (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.ad_types (code, label, description, sort_order) VALUES
                                                                             ('RECURRING',            'Recurring',
                                                                              'Must be repeated at defined calendar or hourly intervals.',          10),
                                                                             ('ONE_TIME',             'One-Time',
                                                                              'Single compliance action; no repetition required.',                  20),
                                                                             ('OPTIONAL_TERMINATING', 'Optional Terminating Action',
                                                                              'One-time action that terminates a recurring compliance requirement.', 30),
                                                                             ('ALERT_SB',             'Alert Service Bulletin',
                                                                              'Urgent manufacturer safety communication; may precede formal AD.',   40),
                                                                             ('EMERGENCY',            'Emergency AD',
                                                                              'Immediate action required before next flight.',                      50)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.sb_compliance_statuses (6 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.sb_compliance_statuses (code, label, sort_order) VALUES
                                                                              ('MANDATORY',      'Mandatory',       10),
                                                                              ('RECOMMENDED',    'Recommended',     20),
                                                                              ('OPTIONAL',       'Optional',        30),
                                                                              ('COMPLIED',       'Complied With',   40),
                                                                              ('SUPERSEDED',     'Superseded',      50),
                                                                              ('NOT_APPLICABLE', 'Not Applicable',  60)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.availability_grades (5 rows)
-- Shared by parts_availability and maintenance_network assessments.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.availability_grades
(code, label, description, numeric_score, sort_order)
VALUES
    ('EXCELLENT', 'Excellent',
     'Widely available worldwide; no sourcing concerns.',                  5, 10),
    ('GOOD',      'Good',
     'Readily available; minor lead time or regional variation.',          4, 20),
    ('FAIR',      'Fair',
     'Available with meaningful lead time or a limited supplier base.',    3, 30),
    ('POOR',      'Poor',
     'Difficult to source; significant lead time or cost premium.',        2, 40),
    ('CRITICAL',  'Critical / Scarce',
     'Obsolete or near-obsolete; stockpile-only supply.',                  1, 50)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 9: PROVENANCE / CURATION LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.source_types (8 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.source_types (code, label, description, sort_order) VALUES
                                                                                 ('SCRAPED_WEB',        'Scraped Web Page',
                                                                                  'Data obtained by automated web scraping.',                              10),
                                                                                 ('MANUFACTURER_SPEC',  'Manufacturer Spec Sheet',
                                                                                  'Official manufacturer specifications document or datasheet.',           20),
                                                                                 ('TYPE_CERTIFICATE',   'Type Certificate Data Sheet (TCDS)',
                                                                                  'FAA / EASA TCDS or equivalent official TC data sheet.',                 30),
                                                                                 ('POH_AFM',            'POH / AFM',
                                                                                  'Pilot Operating Handbook or Airplane Flight Manual.',                   40),
                                                                                 ('JANES',              "Jane's All the World's Aircraft",
                                                                                  "IHS Markit / Jane's reference publication.",                            50),
                                                                                 ('IMPORTED_DATASET',   'Imported Dataset',
                                                                                  'Third-party dataset batch import.',                                     60),
                                                                                 ('MANUAL_ENTRY',       'Manual Entry',
                                                                                  'Human-curated entry by the editorial team.',                            70),
                                                                                 ('CALCULATED',         'Calculated / Derived',
                                                                                  'Value derived from other source values by formula.',                    80)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.source_reliability_grades (5 rows)
-- numeric_score / 5 ≈ default confidence_score for assertions from this source.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.source_reliability_grades
(code, label, description, numeric_score, sort_order)
VALUES
    ('AUTHORITATIVE', 'Authoritative',
     'Type certificate, POH/AFM, or regulatory filing.',                    5, 10),
    ('HIGH',          'High',
     "Official manufacturer publication or Jane's-class reference.",         4, 20),
    ('MEDIUM',        'Medium',
     'Reputable third-party publication or well-curated dataset.',           3, 30),
    ('LOW',           'Low',
     'Secondary source with limited verifiability.',                         2, 40),
    ('UNVERIFIED',    'Unverified',
     'Raw scraped or crowd-sourced; no independent verification.',           1, 50)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.curation_flag_statuses (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.curation_flag_statuses
(code, label, description, is_terminal, sort_order)
VALUES
    ('OPEN',         'Open',
     'Issue identified; not yet reviewed by a curator.',           FALSE, 10),
    ('UNDER_REVIEW', 'Under Review',
     'Assigned to curator; investigation in progress.',            FALSE, 20),
    ('RESOLVED',     'Resolved',
     'Issue resolved; canonical value confirmed or updated.',      TRUE,  30),
    ('DISMISSED',    'Dismissed',
     'Investigated and found not significant; no action taken.',   TRUE,  40),
    ('DEFERRED',     'Deferred',
     'Acknowledged but deferred to a later curation cycle.',       FALSE, 50)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.curation_entity_types (11 rows)
-- schema_name / table_name document the real PostgreSQL table for each type.
-- These will match the tables created in later phases.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.curation_entity_types
(code, label, schema_name, table_name, sort_order)
VALUES
    ('AIRCRAFT_FAMILY',    'Aircraft Family',
     'aircraft_core', 'families',             10),
    ('AIRCRAFT_MODEL',     'Aircraft Model',
     'aircraft_core', 'models',               11),
    ('AIRCRAFT_VARIANT',   'Aircraft Variant',
     'aircraft_core', 'variants',             12),
    ('ENGINE_SPEC',        'Engine Specification',
     'aircraft_power','engine_variants',       20),
    ('PERFORMANCE_METRIC', 'Performance Metric Value',
     'aircraft_specs','performance_metrics',   30),
    ('WEIGHT_METRIC',      'Weight Metric Value',
     'aircraft_specs','weight_metrics',        31),
    ('DIMENSION_METRIC',   'Dimension Metric Value',
     'aircraft_specs','dimension_metrics',     32),
    ('COST_SNAPSHOT',      'Ownership Cost Snapshot',
     'aircraft_market','cost_snapshots',       40),
    ('VALUATION',          'Market Valuation',
     'aircraft_market','valuations',           41),
    ('SOURCE_DOCUMENT',    'Source Document',
     'aircraft_prov', 'source_documents',      50),
    ('MANUFACTURER',       'Manufacturer / Organisation',
     'aircraft_org',  'organizations',         60)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.assertion_statuses (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.assertion_statuses
(code, label, description, sort_order)
VALUES
    ('PENDING',    'Pending',
     'Ingested but not yet reviewed by a curator.',                    10),
    ('ACCEPTED',   'Accepted',
     'Accepted as the canonical or best-available value.',             20),
    ('REJECTED',   'Rejected',
     'Rejected; superseded by a better source or identified as error.', 30),
    ('CONFLICT',   'In Conflict',
     'Conflicts with an assertion from another source.',               40),
    ('SUPERSEDED', 'Superseded',
     'A newer assertion from the same or better source now exists.',   50)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 10: COMPARISON / MISSION LOOKUPS
-- comparison_criterion_types FKs to performance/weight/dimension_metric_types
-- inserted earlier in this file; validated at COMMIT (DEFERRABLE).
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.mission_profile_types (15 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.mission_profile_types (code, label, sort_order) VALUES
                                                                             ('PERSONAL_VFR_TOURING',      'Personal VFR Touring',              10),
                                                                             ('IFR_CROSSCOUNTRY',          'IFR Cross-Country Travel',          20),
                                                                             ('BUSINESS_TRAVEL',           'Business Aviation Travel',          30),
                                                                             ('FLIGHT_TRAINING',           'Flight Training',                   40),
                                                                             ('BACKCOUNTRY_STOL',          'Backcountry / STOL Operations',     50),
                                                                             ('FLOATPLANE_OPERATIONS',     'Float-plane / Amphibious Ops',      55),
                                                                             ('CARGO_FREIGHT',             'Cargo / Freight Transport',         60),
                                                                             ('MEDEVAC_SAR',               'Medevac / Search and Rescue',       70),
                                                                             ('PATROL_SURVEILLANCE',       'Patrol / Surveillance',             80),
                                                                             ('HIGH_ALTITUDE_OPS',         'High-Altitude Operations',          85),
                                                                             ('AEROBATICS',                'Aerobatics / Air Show',             90),
                                                                             ('MILITARY_CLOSE_AIR_SUPPORT','Military: Close Air Support',      100),
                                                                             ('MILITARY_TRANSPORT_AIRLIFT','Military: Transport / Airlift',    110),
                                                                             ('MILITARY_MARITIME_PATROL',  'Military: Maritime Patrol',        120),
                                                                             ('UNMANNED_SPECIAL_MISSION',  'Unmanned / Special Mission',       130)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.comparison_criterion_types (12 rows)
-- Metric FKs wired here; all three FK columns are DEFERRABLE.
-- CRITERION_FUEL_EFFICIENCY, CRITERION_PAX_SEATS, CRITERION_PRICE, and
-- CRITERION_HOURLY_COST are computed / non-metric criteria (all metric FKs NULL).
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.comparison_criterion_types
(code, label,
 performance_metric_code, weight_metric_code, dimension_metric_code,
 is_higher_better, sort_order)
VALUES
    ('CRITERION_CRUISE_SPEED',   'Cruise Speed',
     'SPEED_CRUISE_BEST', NULL,             NULL,          TRUE,  10),
    ('CRITERION_RANGE',          'Range',
     'RANGE_NORMAL',      NULL,             NULL,          TRUE,  20),
    ('CRITERION_CEILING',        'Service Ceiling',
     'CEILING_SERVICE',   NULL,             NULL,          TRUE,  30),
    ('CRITERION_CLIMB_RATE',     'Rate of Climb',
     'CLIMB_RATE_SL',     NULL,             NULL,          TRUE,  40),
    -- Fuel efficiency is nm/gal; computed in Phase 15 views, no single metric
    ('CRITERION_FUEL_EFFICIENCY','Fuel Efficiency (nm/gal)',
     NULL,                NULL,             NULL,          TRUE,  50),
    ('CRITERION_PAYLOAD',        'Payload Capacity',
     NULL,                'WEIGHT_PAYLOAD', NULL,          TRUE,  60),
    -- Passenger seats come from aircraft_core.variants attribute, not a metric
    ('CRITERION_PAX_SEATS',      'Passenger Seats',
     NULL,                NULL,             NULL,          TRUE,  70),
    ('CRITERION_RUNWAY_TAKEOFF', 'Takeoff Distance',
     'DIST_TO_50FT',      NULL,             NULL,          FALSE, 80),
    ('CRITERION_RUNWAY_LANDING', 'Landing Distance',
     'DIST_LDG_50FT',     NULL,             NULL,          FALSE, 90),
    -- Price and hourly cost come from aircraft_market, not metric tables
    ('CRITERION_PRICE',          'Acquisition Price',
     NULL,                NULL,             NULL,          FALSE, 100),
    ('CRITERION_HOURLY_COST',    'Total Hourly Operating Cost',
     NULL,                NULL,             NULL,          FALSE, 110),
    ('CRITERION_WINGSPAN',       'Wingspan',
     NULL,                NULL,             'DIM_WINGSPAN', NULL, 120)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 11: ORGANIZATION LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.organization_types (10 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.organization_types (code, label, sort_order) VALUES
                                                                          ('MANUFACTURER',            'Aircraft Manufacturer',         10),
                                                                          ('DESIGN_BUREAU',           'Design Bureau',                 11),
                                                                          ('LICENSE_MANUFACTURER',    'Licensed Manufacturer',         12),
                                                                          ('OPERATOR_MILITARY',       'Military Operator',             20),
                                                                          ('OPERATOR_COMMERCIAL',     'Commercial Airline / Air Taxi', 21),
                                                                          ('OPERATOR_GOVERNMENT',     'Government / State Operator',   22),
                                                                          ('OPERATOR_PRIVATE',        'Private Aviation Operator',     23),
                                                                          ('CERTIFICATION_AUTHORITY', 'Certification Authority',       30),
                                                                          ('MAINTENANCE_PROVIDER',    'MRO / Maintenance Provider',    40),
                                                                          ('INDUSTRY_ASSOCIATION',    'Industry Association',          50)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.org_relationship_types (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.org_relationship_types
(code, label, description, sort_order)
VALUES
    ('SUBSIDIARY',       'Subsidiary',
     'Child company wholly or majority owned by a parent.',             10),
    ('SUCCESSOR_ENTITY', 'Successor Entity',
     'Historical succession after renaming or acquisition.',            20),
    ('JOINT_VENTURE',    'Joint Venture',
     'Co-production or co-development arrangement.',                    30),
    ('LICENSE_AGREEMENT','License Agreement',
     'Licensed production of a type granted to another company.',       40),
    ('DESIGN_AUTHORITY', 'Design Authority',
     'Holds the type certificate and primary design responsibility.',    50),
    ('MAJOR_SUBCONTRACT','Major Subcontractor',
     'Supplies critical structural or system components.',              60),
    ('CONSORTIUM_MEMBER','Consortium Member',
     'Co-development and co-production consortium membership.',         70)
    ON CONFLICT (code) DO NOTHING;

-- =============================================================================
-- GROUP 12: AVIONICS / SYSTEMS LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.systems_categories (15 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.systems_categories (code, label, sort_order) VALUES
                                                                          ('NAVIGATION',           'Navigation Systems',                  10),
                                                                          ('COMMUNICATION',        'Communication Systems',               20),
                                                                          ('AUTOPILOT_FMS',        'Autopilot / FMS',                    30),
                                                                          ('FLIGHT_INSTRUMENTS',   'Flight Instruments / EFIS',          40),
                                                                          ('ENGINE_MONITORING',    'Engine / Systems Monitoring',        50),
                                                                          ('TERRAIN_AWARENESS',    'Terrain Awareness (TAWS / GPWS)',    60),
                                                                          ('TRAFFIC_AWARENESS',    'Traffic / ADS-B',                    70),
                                                                          ('WEATHER',              'Weather Detection / Datalink',       80),
                                                                          ('ICE_PROTECTION',       'Ice Protection (FIKI / De-Ice)',     90),
                                                                          ('PRESSURIZATION',       'Pressurization / Oxygen',           100),
                                                                          ('SURVEILLANCE_RECON',   'Surveillance / Recon (Military)',   110),
                                                                          ('EW_SYSTEMS',           'Electronic Warfare (Military)',      120),
                                                                          ('DATALINK_TACTICAL',    'Tactical Datalink (Military)',       130),
                                                                          ('EMERGENCY_SAFETY',     'Emergency / Safety Systems',        140),
                                                                          ('LIGHTING',             'Interior / Exterior Lighting',      150)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- aircraft_ref.equipment_provision_types (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.equipment_provision_types
(code, label, description, sort_order)
VALUES
    ('STANDARD',        'Standard Equipment',
     'Included in the base aircraft at no extra charge.',              10),
    ('OPTIONAL_FACTORY','Factory Option',
     'Available as a factory-installed option at additional cost.',    20),
    ('OPTIONAL_DEALER', 'Dealer Option',
     'Available as a dealer-installed option.',                        30),
    ('RETROFIT_STC',    'STC Retrofit',
     'Available via FAA / EASA Supplemental Type Certificate.',        40),
    ('RETROFIT_337',    'Field Approval (FAA Form 337)',
     'Field-approved installation under FAA Form 337.',                50),
    ('NOT_AVAILABLE',   'Not Available',
     'Cannot be installed or approved on this aircraft type.',         60),
    ('REMOVED',         'Removed',
     'Was offered but production or STC has been discontinued.',       70)
    ON CONFLICT (code) DO NOTHING;

COMMIT;