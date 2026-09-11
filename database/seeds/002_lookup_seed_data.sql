-- =============================================================================
-- File: database/seeds/002_lookup_seed_data.sql
-- Phase 2 — seed rows for all aircraft_ref lookup tables EXCEPT
-- unit_categories and measurement_units (see seeds/001_reference_units.sql).
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
-- Externally factual anchors and repository-owned descriptions are audited in
-- database/README.md and gated by crates/aircraft_testsupport/tests/seed_data.rs.
-- =============================================================================

BEGIN;

-- =============================================================================
-- GROUP 2: AIRCRAFT TAXONOMY
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.aircraft_roles (57 rows)
-- role_group is a non-FK informational grouping for display clustering.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.aircraft_roles (code, label, description, role_group, sort_order)
VALUES

    -- Civilian: Commercial air transport
    ('AIRLINER_NARROWBODY', 'Narrowbody Airliner',
     'Single-aisle jet transport for short- and medium-haul scheduled service.',
     'CIVILIAN_COMMERCIAL', 10),
    ('AIRLINER_WIDEBODY', 'Widebody Airliner',
     'Twin-aisle jet transport for long-haul and high-density trunk routes.',
     'CIVILIAN_COMMERCIAL', 11),
    ('AIRLINER_REGIONAL', 'Regional Jet',
     'Short-haul jet feeding trunk routes from smaller markets.',
     'CIVILIAN_COMMERCIAL', 12),
    ('AIRLINER_COMMUTER', 'Commuter / Turboprop Airliner',
     'Turboprop scheduled transport on thin or short-field routes where jet '
     'economics do not close.', 'CIVILIAN_COMMERCIAL', 13),
    ('CARGO_FREIGHTER', 'Dedicated Cargo Freighter',
     'Built or converted for freight only: main-deck cargo door, no passenger '
     'cabin.', 'CIVILIAN_COMMERCIAL', 14),
    ('CARGO_COMBI', 'Combi (Pax + Cargo)',
     'Main deck split between palletised freight and a passenger cabin on the '
     'same flight.', 'CIVILIAN_COMMERCIAL', 15),

    -- Civilian: Business aviation
    ('BUSINESS_JET_HEAVY', 'Heavy Business Jet',
     'Long-range corporate jet with a stand-up cabin and intercontinental legs.',
     'CIVILIAN_BUSINESS', 20),
    ('BUSINESS_JET_MIDSIZE', 'Midsize Business Jet',
     'Transcontinental corporate jet trading cabin volume for lower operating '
     'cost than the heavy class.', 'CIVILIAN_BUSINESS', 21),
    ('BUSINESS_JET_LIGHT', 'Light Business Jet',
     'Short- to medium-range corporate jet, commonly single-pilot certificated.',
     'CIVILIAN_BUSINESS', 22),
    ('BUSINESS_JET_VLJ', 'Very Light Jet (VLJ)',
     'Entry-level jet aimed at owner-pilot and air-taxi operation; the lightest '
     'turbofan class in this vocabulary.', 'CIVILIAN_BUSINESS', 23),
    ('TURBOPROP_EXECUTIVE', 'Executive Turboprop',
     'Pressurised turboprop in corporate configuration, trading cruise speed for '
     'field performance and fuel cost.', 'CIVILIAN_BUSINESS', 24),

    -- Civilian: General aviation
    ('GENERAL_AVIATION_TOURING', 'GA Touring',
     'Personal cross-country travel in a piston or light turbine aircraft.',
     'CIVILIAN_GA', 30),
    ('GENERAL_AVIATION_TRAINING', 'Flight Training',
     'Ab-initio and instrument instruction; docile handling and cost per hour '
     'dominate the choice.', 'CIVILIAN_GA', 31),
    ('LIGHT_SPORT', 'Light Sport Aircraft (LSA)',
     'Operated under a light-sport certification standard; the governing weight, '
     'seating, and speed limits live in airworthiness_categories.',
     'CIVILIAN_GA', 32),
    ('ULTRALIGHT', 'Ultralight / Microlight',
     'Flown under an ultralight or microlight exemption rather than a type '
     'certificate.', 'CIVILIAN_GA', 33),
    ('HOMEBUILT_EXPERIMENTAL', 'Homebuilt / Experimental',
     'Amateur-built and operated in the experimental category; published figures '
     'describe individual airframes rather than a uniform type.',
     'CIVILIAN_GA', 34),
    ('AEROBATIC', 'Aerobatic',
     'Stressed for sustained inverted and high-g manoeuvring in competition or '
     'display flying.', 'CIVILIAN_GA', 35),
    ('GLIDER_MOTORGLIDER', 'Glider / Motorglider',
     'Unpowered or self-launching sailplane; range and endurance follow soaring '
     'conditions rather than fuel aboard.', 'CIVILIAN_GA', 36),

    -- Civilian: Special mission (fixed-wing)
    ('AGRICULTURAL_SPRAY', 'Agricultural / Aerial Application',
     'Crop treatment by air; hopper capacity and low-level handling govern the '
     'design.', 'CIVILIAN_SPECIAL', 40),
    ('MEDEVAC_AIR_AMBULANCE', 'Medical / Air Ambulance',
     'Configured for patient transport with a medical interior, litter loading, '
     'and clinical power.', 'CIVILIAN_SPECIAL', 41),
    ('SEARCH_AND_RESCUE_CIVIL', 'Search and Rescue (Civil)',
     'Civil rescue tasking: endurance, search sensors, and low-speed loiter over '
     'the search area.', 'CIVILIAN_SPECIAL', 42),
    ('AERIAL_SURVEY', 'Aerial Survey / Remote Sensing',
     'Photogrammetry, LiDAR, and remote sensing from camera ports in stable '
     'long-endurance cruise.', 'CIVILIAN_SPECIAL', 43),
    ('FIREFIGHTING_AIR_TANKER', 'Firefighting / Air Tanker',
     'Delivers water or retardant onto wildfire; drop volume and turnaround time '
     'are the governing figures.', 'CIVILIAN_SPECIAL', 44),
    ('LAW_ENFORCEMENT', 'Law Enforcement / Police',
     'Police patrol and pursuit support carrying observation sensors and a '
     'ground downlink.', 'CIVILIAN_SPECIAL', 45),
    ('FLOAT_SEAPLANE', 'Float-plane / Seaplane',
     'Landplane on floats operating from water; the floats cost cruise speed and '
     'payload against the wheeled variant.', 'CIVILIAN_SPECIAL', 46),
    ('FLYING_BOAT', 'Flying Boat',
     'Carried on its own hull rather than on floats; amphibious versions add '
     'retractable land gear.', 'CIVILIAN_SPECIAL', 47),

    -- Rotary wing: Civilian
    ('ROTORCRAFT_HELICOPTER_CIVIL', 'Civil Helicopter',
     'Civil rotorcraft for transport, utility lift, or offshore work, buying '
     'hover capability at the cost of cruise efficiency.', 'ROTARY_CIVIL', 50),
    ('ROTORCRAFT_AUTOGYRO', 'Autogyro / Gyrocopter',
     'Unpowered rotor in autorotation with separate thrust: cannot hover, needs '
     'almost no runway.', 'ROTARY_CIVIL', 51),
    ('TILTROTOR_CIVIL', 'Civil Tiltrotor',
     'Tilting proprotors combining helicopter hover with turboprop cruise speed '
     'and altitude.', 'ROTARY_CIVIL', 52),

    -- UAV: Civilian
    ('UAV_CIVIL_FIXED_WING', 'Civil UAV / RPAS (Fixed Wing)',
     'Uncrewed fixed-wing for survey, inspection, or delivery; endurance and '
     'payload replace seating as the size measure.', 'UAV_CIVIL', 55),
    ('UAV_CIVIL_ROTARY', 'Civil UAV / RPAS (Rotary Wing)',
     'Uncrewed rotorcraft or multirotor for hovering inspection and '
     'short-radius work.', 'UAV_CIVIL', 56),

    -- Military: Fixed-wing combat
    ('MILITARY_FIGHTER_AIR_SUP', 'Fighter — Air Superiority',
     'Optimised for air-to-air combat and control of contested airspace.',
     'MILITARY_FIXED_WING', 60),
    ('MILITARY_FIGHTER_MULTIROLE', 'Fighter — Multirole',
     'Carries both air-to-air and air-to-ground tasking in one airframe.',
     'MILITARY_FIXED_WING', 61),
    ('MILITARY_FIGHTER_INTERCEPT', 'Fighter — Interceptor',
     'Climbs and dashes to engage incoming aircraft at range, trading endurance '
     'for acceleration.', 'MILITARY_FIXED_WING', 62),
    ('MILITARY_ATTACK_CAS', 'Attack — Close Air Support',
     'Supports ground forces in contact; loiter, survivability, and low-altitude '
     'precision over speed.', 'MILITARY_FIXED_WING', 63),
    ('MILITARY_ATTACK_STRIKE', 'Attack — Strike / Deep Strike',
     'Attacks fixed and defended targets beyond the forward edge; range and '
     'payload over agility.', 'MILITARY_FIXED_WING', 64),
    ('MILITARY_BOMBER_STRATEGIC', 'Bomber — Strategic',
     'Intercontinental delivery of heavy payloads against strategic targets.',
     'MILITARY_FIXED_WING', 65),
    ('MILITARY_BOMBER_TACTICAL', 'Bomber — Tactical / Dive',
     'Theatre-range bombing including dive attack; largely superseded by '
     'multirole strike in current inventories.', 'MILITARY_FIXED_WING', 66),

    -- Military: Transport and support
    ('MILITARY_TRANSPORT_STRATEGIC', 'Transport — Strategic Lift',
     'Intertheatre airlift of outsize cargo and vehicles from established '
     'runways.', 'MILITARY_FIXED_WING', 70),
    ('MILITARY_TRANSPORT_TACTICAL', 'Transport — Tactical / Assault',
     'Intratheatre lift into short, unpaved, or contested strips, including '
     'airdrop and assault landing.', 'MILITARY_FIXED_WING', 71),
    ('MILITARY_TRANSPORT_UTILITY', 'Transport — Utility / Light Lift',
     'Liaison and light utility movement of personnel and small loads.',
     'MILITARY_FIXED_WING', 72),
    ('MILITARY_TANKER_REFUELER', 'Aerial Refuelling Tanker',
     'Transfers fuel in flight by boom or drogue; offload at radius is the '
     'governing capability figure.', 'MILITARY_FIXED_WING', 73),
    ('MILITARY_AEW_AWACS', 'AEW / AWACS / C2 Platform',
     'Carries the airborne radar picture and battle management crew.',
     'MILITARY_FIXED_WING', 74),
    ('MILITARY_ELECTRONIC_WARFARE', 'Electronic Warfare / Attack',
     'Jamming, deception, and suppression of enemy air defence.',
     'MILITARY_FIXED_WING', 75),
    ('MILITARY_MARITIME_PATROL', 'Maritime Patrol / MPA',
     'Long-endurance overwater surveillance of shipping and economic zones.',
     'MILITARY_FIXED_WING', 76),
    ('MILITARY_ANTISUBMARINE', 'Anti-Submarine Warfare (ASW)',
     'Detects and attacks submarines using sonobuoys, magnetic anomaly '
     'detection, and lightweight torpedoes.', 'MILITARY_FIXED_WING', 77),
    ('MILITARY_RECONNAISSANCE', 'Reconnaissance / ISR',
     'Intelligence, surveillance, and reconnaissance collection; sensors and '
     'endurance rather than weapons.', 'MILITARY_FIXED_WING', 78),

    -- Military: Training
    ('MILITARY_TRAINER_BASIC', 'Trainer — Basic / Primary',
     'Ab-initio and primary military instruction: forgiving handling, low cost '
     'per hour.', 'MILITARY_FIXED_WING', 80),
    ('MILITARY_TRAINER_ADVANCED', 'Trainer — Advanced',
     'Advanced handling, formation, and weapons introduction ahead of type '
     'conversion.', 'MILITARY_FIXED_WING', 81),
    ('MILITARY_TRAINER_COMBAT', 'Trainer — Lead-In Fighter (LIFT)',
     'Lead-in fighter training with representative avionics and, on most types, '
     'a light attack capability.', 'MILITARY_FIXED_WING', 82),
    ('MILITARY_SPECIAL_OPS', 'Special Operations Aviation',
     'Special operations infiltration, resupply, and support flown on '
     'terrain-following and covert profiles.', 'MILITARY_FIXED_WING', 83),

    -- Military: Rotary wing
    ('MILITARY_HELICOPTER_ATTACK', 'Attack Helicopter',
     'Armed rotorcraft for anti-armour and close support, carrying targeting '
     'sensors and weapon hardpoints.', 'MILITARY_ROTARY', 85),
    ('MILITARY_HELICOPTER_TRANSPORT', 'Transport Helicopter',
     'Troop and cargo lift by rotorcraft, including underslung loads.',
     'MILITARY_ROTARY', 86),
    ('MILITARY_HELICOPTER_ASW', 'ASW / Naval Helicopter',
     'Ship-based rotorcraft for anti-submarine and anti-surface work; folding '
     'rotor and deck handling provisions.', 'MILITARY_ROTARY', 87),
    ('MILITARY_HELICOPTER_SAR', 'Combat SAR / Rescue Helicopter',
     'Recovers personnel under threat: hoist, self-protection, and on some types '
     'aerial refuelling.', 'MILITARY_ROTARY', 88),

    -- Military: UAV
    ('MILITARY_UAV_SURVEILLANCE', 'Military UAV — Surveillance/ISR',
     'Uncrewed collection platform trading weapons for endurance and sensor '
     'payload.', 'MILITARY_UAV', 90),
    ('MILITARY_UAV_STRIKE', 'Military UAV — Strike / UCAV',
     'Uncrewed combat air vehicle carrying weapons alongside its sensors.',
     'MILITARY_UAV', 91)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    role_group = EXCLUDED.role_group,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.service_statuses (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.service_statuses (code, label, description, sort_order)
VALUES ('IN_PRODUCTION', 'In Production', 'Currently manufactured and delivered to customers.', 10),
       ('LIMITED_PRODUCTION', 'Limited Production', 'Production ongoing but limited in volume or available only to order.', 20),
       ('DISCONTINUED', 'Discontinued', 'Production has permanently ended; type still actively operated.', 30),
       ('RETIRED', 'Retired', 'No longer in regular service; museum or static examples only.', 40),
       ('EXPERIMENTAL', 'Experimental', 'Prototype, X-plane, or test article; not type-certificated for service.', 50),
       ('ON_ORDER', 'On Order / In Development', 'In development with orders placed; not yet in service.', 60),
       ('UNKNOWN', 'Unknown', 'Production / service status not established from available sources.', 99)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.variant_types (9 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.variant_types (code, label, description, sort_order)
VALUES ('PRODUCTION_STANDARD', 'Standard Production',
        'Primary commercially offered production configuration.', 10),
       ('PROTOTYPE', 'Prototype',
        'Pre-production development aircraft.', 20),
       ('PRE_PRODUCTION', 'Pre-Production',
        'Early series build; not to full production standard.', 25),
       ('EXPORT', 'Export Variant',
        'Modified configuration for export customers.', 30),
       ('MILITARY_CONVERSION', 'Military Conversion',
        'Civilian design adapted for military use.', 40),
       ('CIVIL_CONVERSION', 'Civil Conversion',
        'Military design adapted for civilian use.', 41),
       ('STC_CONVERSION', 'STC / Aftermarket Conversion',
        'Major conversion under a Supplemental Type Certificate.', 50),
       ('SPECIAL_MISSION', 'Special Mission',
        'Factory-modified for surveillance, medevac, or other special purposes.', 60),
       ('CLASSIC_SERIES', 'Classic / Legacy Series',
        'Historical production series representing an earlier generation.', 70)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 3: PHYSICAL PROPERTY LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.landing_gear_types (10 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.landing_gear_types (code, label, description, sort_order)
VALUES ('FIXED_TRICYCLE', 'Fixed Tricycle',
        'Fixed nosewheel tricycle undercarriage.', 10),
       ('RETRACTABLE_TRICYCLE', 'Retractable Tricycle',
        'Retractable nosewheel tricycle undercarriage.', 20),
       ('FIXED_TAILWHEEL', 'Fixed Tailwheel (Conventional)',
        'Fixed conventional / taildragger undercarriage.', 30),
       ('RETRACTABLE_TAILWHEEL', 'Retractable Tailwheel',
        'Retractable conventional / taildragger undercarriage.', 40),
       -- Retraction is known, configuration is not. Sources routinely state
       -- "fixed landing gear" or "retractable landing gear" without saying
       -- tricycle or tailwheel; without these two codes such a variant has to be
       -- left NULL, which loses the half of the fact the source does give and
       -- makes the column unfilterable. A curator narrows these to the specific
       -- pair once the configuration is established.
       ('FIXED_UNSPECIFIED', 'Fixed (Configuration Unspecified)',
        'Non-retracting undercarriage whose tricycle or tailwheel configuration '
        'the source does not state.', 45),
       ('RETRACTABLE_UNSPECIFIED', 'Retractable (Configuration Unspecified)',
        'Retracting undercarriage whose tricycle or tailwheel configuration the '
        'source does not state.', 48),
       ('AMPHIBIOUS', 'Amphibious',
        'Retractable gear plus hull or floats; land and water operations.', 50),
       ('FLOATS', 'Floats',
        'Pontoon floats; water operations only.', 60),
       ('SKIS', 'Skis',
        'Ski undercarriage for snow and ice surfaces.', 70),
       ('SKIDS', 'Skids',
        'Skid undercarriage (helicopter, VTOL).', 80),
       ('TAILSKID', 'Tailskid',
        'Fixed or retractable tailskid; no main tailwheel.', 85),
       ('NONE_GLIDER', 'None (Glider / Sailplane)',
        'Mono-wheel or no gear; glider and motorglider convention.', 90)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.propulsion_categories (11 rows)
-- primary_power_unit FK to measurement_units (already committed via
-- seeds/001_reference_units.sql); constraint is DEFERRABLE.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.propulsion_categories
(code, label, description, is_jet, is_rotating, primary_power_unit, sort_order)
VALUES ('PISTON_RECIPROCATING', 'Piston / Reciprocating',
        'Conventional 4-stroke piston engines.', FALSE, TRUE, 'HP', 10),
       ('PISTON_ROTARY', 'Rotary (Wankel)',
        'Rotary / Wankel piston engines.', FALSE, TRUE, 'HP', 11),
       ('TURBOPROP', 'Turboprop',
        'Turbine engine driving a propeller via reduction gearbox.', FALSE, TRUE, 'SHP', 20),
       ('TURBOJET', 'Turbojet',
        'Pure turbojet; no bypass flow.', TRUE, FALSE, 'LBF', 30),
       ('TURBOFAN_LOW_BPR', 'Low-Bypass Turbofan',
        'Turbofan with bypass ratio < 4:1.', TRUE, FALSE, 'LBF', 31),
       ('TURBOFAN_HIGH_BPR', 'High-Bypass Turbofan',
        'Turbofan with bypass ratio ≥ 4:1.', TRUE, FALSE, 'LBF', 32),
       ('TURBOSHAFT', 'Turboshaft',
        'Turbine outputting shaft power; primary rotorcraft propulsion.', FALSE, TRUE, 'SHP', 40),
       ('ELECTRIC', 'Electric Motor',
        'Battery or fuel-cell electric propulsion.', FALSE, TRUE, 'KW', 50),
       ('HYBRID_ELECTRIC', 'Hybrid Electric',
        'Combined conventional and electric propulsion.', FALSE, TRUE, 'HP', 51),
       ('ROCKET', 'Rocket Motor',
        'Rocket propulsion (X-planes, some experimental aircraft).', TRUE, FALSE, 'LBF', 60),
       ('NONE_GLIDER', 'None (Unpowered Glider)',
        'Unpowered glider or sailplane.', FALSE, FALSE, NULL, 99)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    is_jet = EXCLUDED.is_jet,
    is_rotating = EXCLUDED.is_rotating,
    primary_power_unit = EXCLUDED.primary_power_unit,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.fuel_types (10 rows)
-- density_lbs_per_gal at standard conditions; NULL for non-liquid fuels.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.fuel_types
    (code, label, description, density_lbs_per_gal, sort_order)
VALUES ('AVGAS_100LL', '100LL Avgas',
        'Aviation gasoline, low-lead, 100 octane. Most common piston fuel.', 6.02, 10),
       ('AVGAS_100', '100/130 Avgas',
        'Aviation gasoline, 100/130 octane (legacy grade).', 6.02, 11),
       ('JET_A', 'Jet-A',
        'Kerosene-based turbine fuel; freeze point –40 °C. Common outside Russia.', 6.70, 20),
       ('JET_A1', 'Jet-A1',
        'Jet-A with lower freeze point (–47 °C); standard outside North America.', 6.70, 21),
       ('JET_B', 'Jet-B / JP-4',
        'Wide-cut gasoline-kerosene blend; used in cold-weather environments.', 6.50, 22),
       ('JP_8', 'JP-8',
        'Military turbine fuel (NATO F-34); Jet-A1 with additives.', 6.70, 23),
       ('DIESEL_ASTM', 'Diesel / ASTM D975',
        'Automotive or ASTM D975 diesel for certified diesel-cycle piston engines.', 7.05, 30),
       ('MOGAS', 'Mogas (Automotive Gasoline)',
        'Automotive unleaded gasoline under STC-approved installations.', 6.15, 40),
       ('ELECTRIC', 'Electric (Battery / Fuel Cell)',
        'Energy stored in batteries or generated by fuel cell; no liquid fuel.', NULL, 50),
       ('HYDROGEN', 'Hydrogen (LH2 / GH2)',
        'Cryogenic liquid or compressed gaseous hydrogen; density depends on physical state.', NULL, 60)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    density_lbs_per_gal = EXCLUDED.density_lbs_per_gal,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 4: METRIC TYPE LOOKUPS
-- canonical_unit_code FKs reference measurement_units (committed earlier).
-- All FKs are DEFERRABLE INITIALLY DEFERRED; validated at COMMIT of this block.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.performance_metric_types (25 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.performance_metric_types
(code, label, description, canonical_unit_code,
 is_higher_better, is_speed, is_distance, is_rate, sort_order)
VALUES
    -- Speed
    ('SPEED_VNE', 'Never-Exceed Speed (Vne)',
     'Red-line indicated airspeed the airframe must never exceed. A structural '
     'limit, not an achievable cruise.',
     'KNOTS', FALSE, TRUE, FALSE, FALSE, 10),
    ('SPEED_MAX', 'Maximum Speed (Vmax)',
     'Highest speed attainable in level flight at the rated power setting.',
     'KNOTS', TRUE, TRUE, FALSE, FALSE, 11),
    ('SPEED_CRUISE_HIGH', 'High-Speed Cruise',
     'Cruise at a high power setting, favouring block time over fuel burn.',
     'KNOTS', TRUE, TRUE, FALSE, FALSE, 12),
    ('SPEED_CRUISE_BEST', 'Best Cruise Speed',
     'The manufacturer normal cruise figure, and the one most often quoted '
     'without its power setting attached.',
     'KNOTS', TRUE, TRUE, FALSE, FALSE, 13),
    ('SPEED_CRUISE_LRC', 'Long-Range Cruise Speed',
     'Setting that maximises distance flown per unit of fuel.',
     'KNOTS', TRUE, TRUE, FALSE, FALSE, 14),
    ('SPEED_CRUISE_ECON', 'Economy Cruise Speed',
     'Reduced power cruise chosen to lower cost per hour rather than to extend '
     'range.', 'KNOTS', TRUE, TRUE, FALSE, FALSE, 15),
    ('SPEED_MMO', 'Maximum Operating Mach (Mmo)',
     'Certificated Mach limit for normal operation; the compressibility '
     'counterpart to a red-line airspeed.',
     'MACH', FALSE, TRUE, FALSE, FALSE, 16),
    ('SPEED_MACH_CRUISE', 'Typical Cruise Mach Number',
     'Representative cruise Mach at normal cruise altitude and weight.',
     'MACH', TRUE, TRUE, FALSE, FALSE, 17),
    ('SPEED_STALL_CLEAN', 'Stall Speed — Clean (VS1)',
     'Stall speed with flaps and gear retracted.',
     'KNOTS', FALSE, TRUE, FALSE, FALSE, 20),
    ('SPEED_STALL_LANDING', 'Stall Speed — Landing Config (VS0)',
     'Stall speed in the landing configuration; sets approach speed and much of '
     'the runway requirement.', 'KNOTS', FALSE, TRUE, FALSE, FALSE, 21),
    -- Climb
    ('CLIMB_RATE_SL', 'Rate of Climb — Sea Level',
     'Best rate of climb at sea level with all engines operating.',
     'FPM', TRUE, FALSE, FALSE, TRUE, 30),
    ('CLIMB_RATE_OEI', 'Rate of Climb — One Engine Inop.',
     'Climb rate with one engine failed: a terrain-clearance figure that exists '
     'only for multi-engine types.', 'FPM', TRUE, FALSE, FALSE, TRUE, 31),
    -- Ceilings
    ('CEILING_SERVICE', 'Service Ceiling',
     'Altitude at which the best rate of climb decays to a small published '
     'residual. Sources differ on the residual used, so compare this figure only '
     'within one source.', 'FT', TRUE, FALSE, FALSE, FALSE, 40),
    ('CEILING_ABSOLUTE', 'Absolute Ceiling',
     'Altitude at which rate of climb reaches zero and the aircraft can climb no '
     'further.', 'FT', TRUE, FALSE, FALSE, FALSE, 41),
    ('CEILING_OEI', 'Ceiling — One Engine Inoperative',
     'Highest altitude a multi-engine type can hold with one engine failed.',
     'FT', TRUE, FALSE, FALSE, FALSE, 42),
    -- Range and endurance
    ('RANGE_NORMAL', 'Range — Normal Cruise',
     'Still-air distance at normal cruise on standard fuel with a typical '
     'payload aboard.', 'NM', TRUE, FALSE, TRUE, FALSE, 50),
    ('RANGE_MAX_FUEL', 'Range — Maximum Fuel',
     'Distance with tanks filled to capacity and payload cut back as far as the '
     'weight limits demand.', 'NM', TRUE, FALSE, TRUE, FALSE, 51),
    ('RANGE_FERRY', 'Ferry Range',
     'Delivery distance with no payload and, where fitted, auxiliary tanks.',
     'NM', TRUE, FALSE, TRUE, FALSE, 52),
    ('RANGE_COMBAT_RADIUS', 'Combat Radius',
     'Distance out to the target and back on one tanking, including reserves and '
     'time on station. Not half of ferry range, and not comparable to it.',
     'NM', TRUE, FALSE, TRUE, FALSE, 53),
    ('ENDURANCE_HRS', 'Maximum Endurance',
     'Longest time airborne on internal fuel, flown for loiter rather than for '
     'distance covered.', 'HRS', TRUE, FALSE, FALSE, FALSE, 54),
    -- Runway distances (canonical unit FT, same code as altitude FT)
    ('DIST_TO_GROUND_ROLL', 'Takeoff Ground Roll',
     'Brake release to liftoff, measured with the wheels still on the surface.',
     'FT', FALSE, FALSE, TRUE, FALSE, 60),
    ('DIST_TO_50FT', 'Takeoff Distance over 50 ft',
     'Brake release to clearing a 50 ft obstacle. This is the figure runway '
     'analysis uses, not the ground roll.', 'FT', FALSE, FALSE, TRUE, FALSE, 61),
    ('DIST_LDG_GROUND_ROLL', 'Landing Ground Roll',
     'Touchdown to full stop, excluding the airborne approach segment.',
     'FT', FALSE, FALSE, TRUE, FALSE, 62),
    ('DIST_LDG_50FT', 'Landing Distance over 50 ft',
     'From crossing a 50 ft obstacle to full stop.',
     'FT', FALSE, FALSE, TRUE, FALSE, 63),
    -- Fuel consumption
    ('FUEL_BURN_CRUISE', 'Fuel Burn — Best Cruise Setting',
     'Fuel consumed per hour at the best cruise setting. Pairs with the matching '
     'cruise speed to give distance flown per unit of fuel.',
     'GPH', FALSE, FALSE, FALSE, TRUE, 70)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    canonical_unit_code = EXCLUDED.canonical_unit_code,
    is_higher_better = EXCLUDED.is_higher_better,
    is_speed = EXCLUDED.is_speed,
    is_distance = EXCLUDED.is_distance,
    is_rate = EXCLUDED.is_rate,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;
-- =============================================================================
-- aircraft_ref.performance_metric_types V-speed rows
-- Kept in the canonical Phase 2 seed so later phases only define schema
-- and do not introduce reference data. The upsert repairs seed-owned drift.
-- =============================================================================

INSERT INTO aircraft_ref.performance_metric_types
    (code, label, description, canonical_unit_code,
     is_higher_better, is_speed, sort_order)
VALUES
    ('SPEED_VX',     'Best Angle of Climb (Vx)',
     'Speed for maximum altitude gain per unit of distance.',
     'KNOTS', NULL,  TRUE,  22),
    ('SPEED_VY',     'Best Rate of Climb (Vy)',
     'Speed for maximum altitude gain per unit of time.',
     'KNOTS', NULL,  TRUE,  23),
    ('SPEED_VA',     'Maneuvering Speed (Va)',
     'Maximum speed at which full deflection of any one control is permitted. '
     'Weight-dependent; store at MTOW and at light weight separately.',
     'KNOTS', NULL,  TRUE,  24),
    ('SPEED_VNO',    'Max Structural Cruising Speed (Vno)',
     'Maximum speed in normal operations (green arc upper limit).',
     'KNOTS', FALSE, TRUE,  25),
    ('SPEED_VFE',    'Max Flaps Extended Speed (Vfe)',
     'Maximum speed with flaps in specified extended position.',
     'KNOTS', FALSE, TRUE,  26),
    ('SPEED_VLE',    'Max Landing Gear Extended Speed (Vle)',
     'Maximum speed with landing gear in extended position.',
     'KNOTS', FALSE, TRUE,  27),
    ('SPEED_VLO',    'Max Landing Gear Operating Speed (Vlo)',
     'Maximum speed for extending or retracting landing gear.',
     'KNOTS', FALSE, TRUE,  28),
    ('SPEED_VMC',    'Min Control Speed, Multi-Engine (Vmc)',
     'Minimum airspeed at which directional control can be maintained '
     'with one engine inoperative at max thrust.',
     'KNOTS', FALSE, TRUE,  29),
    ('SPEED_VYSE',   'Best Rate of Climb, Single Engine (Vyse)',
     'Speed for best rate of climb with one engine inoperative.',
     'KNOTS', NULL,  TRUE,  32),
    ('SPEED_VAPP',   'Reference Approach Speed (Vref / Vapp)',
     'Stabilised approach speed (typically 1.3 × Vs0 or aircraft-specific Vref).',
     'KNOTS', NULL,  TRUE,  35),
    ('SPEED_ROTATE', 'Rotation Speed (Vr)',
     'Speed at which the pilot initiates nose-up rotation during takeoff roll.',
     'KNOTS', NULL,  TRUE,  36)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    canonical_unit_code = EXCLUDED.canonical_unit_code,
    is_higher_better = EXCLUDED.is_higher_better,
    is_speed = EXCLUDED.is_speed,
    is_distance = EXCLUDED.is_distance,
    is_rate = EXCLUDED.is_rate,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.weight_metric_types (17 rows)
-- LOAD_FACTOR_POS / NEG are dimensionless; canonical_unit_code = NULL.
-- WING_LOADING / POWER_LOADING are derived ratios stored as LBS for sorting.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.weight_metric_types
    (code, label, description, canonical_unit_code, sort_order)
VALUES ('WEIGHT_EMPTY', 'Basic Empty Weight',
        'Standard empty weight per manufacturer published data.', 'LBS', 10),
       ('WEIGHT_OEW', 'Operating Empty Weight (OEW)',
        'BEW plus standard equipment, unusable fuel, and full oil.', 'LBS', 11),
       ('WEIGHT_MTOW', 'Max Takeoff Weight (MTOW)',
        'Maximum certificated gross takeoff weight.', 'LBS', 20),
       ('WEIGHT_MLW', 'Max Landing Weight (MLW)',
        'Maximum certificated landing weight.', 'LBS', 21),
       ('WEIGHT_MZFW', 'Max Zero-Fuel Weight (MZFW)',
        'MTOW minus minimum required fuel; limits structural bending.', 'LBS', 22),
       ('WEIGHT_MRW', 'Max Ramp Weight (MRW)',
        'Maximum allowable weight before taxi; includes taxi fuel.', 'LBS', 23),
       ('WEIGHT_USEFUL_LOAD', 'Useful Load',
        'MTOW minus BEW; pilot + pax + baggage + fuel.', 'LBS', 30),
       ('WEIGHT_PAYLOAD', 'Payload',
        'Useful load minus full fuel; revenue or passenger weight.', 'LBS', 31),
       ('WEIGHT_PAYLOAD_FULL_FUEL', 'Payload with Full Fuel',
        'Remaining payload at maximum usable fuel load.', 'LBS', 32),
       ('WEIGHT_BAGGAGE_MAX', 'Max Baggage Weight',
        'Maximum certificated baggage compartment weight.', 'LBS', 33),
       ('FUEL_CAPACITY_TOTAL', 'Total Fuel Capacity',
        'Total tank volume including unusable fuel.', 'US_GAL', 40),
       ('FUEL_CAPACITY_USABLE', 'Usable Fuel Capacity',
        'Fuel volume available for flight; excludes unusable.', 'US_GAL', 41),
       ('FUEL_WEIGHT_MAX', 'Max Usable Fuel Weight',
        'Weight of maximum usable fuel at standard density.', 'LBS', 42),
       ('WING_LOADING', 'Wing Loading (MTOW / Wing Area)',
        'MTOW divided by wing reference area; lbs/sq ft.', 'LBS', 50),
       ('POWER_LOADING', 'Power Loading (MTOW / Total HP)',
        'MTOW divided by total installed power; lbs/hp.', 'LBS', 51),
       ('LOAD_FACTOR_POS', 'Positive Load Factor Limit',
        'Maximum positive g-loading certificated limit.', NULL, 60),
       ('LOAD_FACTOR_NEG', 'Negative Load Factor Limit',
        'Maximum negative g-loading certificated limit.', NULL, 61)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    canonical_unit_code = EXCLUDED.canonical_unit_code,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.dimension_metric_types (17 rows)
-- DIM_ASPECT_RATIO is dimensionless (no canonical unit).
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.dimension_metric_types
    (code, label, description, canonical_unit_code, sort_order)
VALUES ('DIM_WINGSPAN', 'Wingspan (tip to tip)',
        'Overall wing span from tip to tip.', 'FT', 10),
       ('DIM_WINGSPAN_FOLDED', 'Wingspan — Folded',
        'Wing span in folded configuration (carrier aircraft).', 'FT', 11),
       ('DIM_LENGTH', 'Overall Length',
        'Overall fuselage length nose to tail.', 'FT', 12),
       ('DIM_HEIGHT', 'Overall Height (to tail)',
        'Ground-to-tail height on standard gear.', 'FT', 13),
       ('DIM_WING_AREA', 'Wing Reference Area',
        'Gross planform reference wing area.', 'SQ_FT', 20),
       ('DIM_ASPECT_RATIO', 'Wing Aspect Ratio',
        'Span² divided by reference area; dimensionless.', NULL, 21),
       ('DIM_ROTOR_DIAMETER', 'Main Rotor Diameter',
        'Main rotor tip-to-tip diameter (helicopter).', 'FT', 25),
       ('DIM_PROP_DIAMETER', 'Propeller Diameter',
        'Propeller disc diameter.', 'FT', 26),
       ('DIM_CABIN_LENGTH', 'Cabin Interior Length',
        'Interior pressurised or habitable cabin length.', 'FT', 30),
       ('DIM_CABIN_WIDTH', 'Cabin Interior Width (max)',
        'Maximum interior width of the cabin.', 'FT', 31),
       ('DIM_CABIN_HEIGHT', 'Cabin Interior Height (max)',
        'Maximum interior standing height of the cabin.', 'FT', 32),
       ('DIM_BAGGAGE_VOLUME', 'Baggage Compartment Volume',
        'Total accessible baggage compartment volume.', 'CU_FT', 40),
       ('DIM_CARGO_VOLUME', 'Cargo Hold Volume',
        'Total volumetric capacity of cargo hold (freighters).', 'CU_FT', 41),
       ('DIM_CARGO_DOOR_WIDTH', 'Cargo Door Width',
        'Clear opening width of the main cargo door.', 'FT', 42),
       ('DIM_CARGO_DOOR_HEIGHT', 'Cargo Door Height',
        'Clear opening height of the main cargo door.', 'FT', 43),
       ('DIM_WHEELBASE', 'Wheelbase',
        'Longitudinal distance from nose gear to main gear.', 'FT', 50),
       ('DIM_TRACK_WIDTH', 'Main Gear Track Width',
        'Lateral distance between main gear contact points.', 'FT', 51)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    canonical_unit_code = EXCLUDED.canonical_unit_code,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

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
VALUES ('FAA', 'FAA',
        'Federal Aviation Administration',
        ARRAY['USA'], 'https://www.faa.gov', 10),
       ('EASA', 'EASA',
        'European Union Aviation Safety Agency',
        ARRAY['AUT','BEL','BGR','HRV','CYP','CZE','DNK','EST','FIN','FRA','DEU','GRC','HUN',
              'ISL','IRL','ITA','LVA','LIE','LTU','LUX','MLT','NLD','NOR','POL','PRT','ROU',
              'SVK','SVN','ESP','SWE','CHE'],
        'https://www.easa.europa.eu', 11),
       ('TCCA', 'TCCA',
        'Transport Canada Civil Aviation',
        ARRAY['CAN'], 'https://tc.canada.ca', 12),
       ('CASA', 'CASA',
        'Civil Aviation Safety Authority (Australia)',
        ARRAY['AUS'], 'https://www.casa.gov.au', 13),
       ('CAA_UK', 'CAA UK',
        'Civil Aviation Authority (United Kingdom)',
        ARRAY['GBR'], 'https://www.caa.co.uk', 14),
       ('CAAC', 'CAAC',
        'Civil Aviation Administration of China',
        ARRAY['CHN'], 'https://www.caac.gov.cn', 15),
       ('DGCA', 'DGCA',
        'Directorate General of Civil Aviation (India)',
        ARRAY['IND'], 'https://dgca.gov.in', 16),
       ('ANAC', 'ANAC',
        'Agência Nacional de Aviação Civil (Brazil)',
        ARRAY['BRA'], 'https://www.anac.gov.br', 17)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    full_name = EXCLUDED.full_name,
    country_codes = EXCLUDED.country_codes,
    website_url = EXCLUDED.website_url,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.airworthiness_categories (9 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.airworthiness_categories
    (code, label, description, authority_code, sort_order)
VALUES ('FAA_NORMAL', 'FAA Normal Category',
        'FAR Part 23 Normal; positive load factor +3.8g.', 'FAA', 10),
       ('FAA_UTILITY', 'FAA Utility Category',
        'FAR Part 23 Utility; limited aerobatics (spins, chandelles).', 'FAA', 11),
       ('FAA_ACROBATIC', 'FAA Acrobatic Category',
        'FAR Part 23 Acrobatic; full aerobatic operations approved.', 'FAA', 12),
       ('FAA_TRANSPORT', 'FAA Transport Category',
        'FAR Part 25 Transport; large / air-carrier aircraft.', 'FAA', 13),
       ('FAA_LSA', 'FAA Light Sport',
        'Performance-based FAA light-sport category under MOSAIC; aircraft certificated before '
        'July 24, 2026 may remain legacy light-sport aircraft under the prior limits.', 'FAA', 14),
       ('FAA_EXPERIMENTAL', 'FAA Experimental',
        'Experimental airworthiness; not certificated for public transport.', 'FAA', 15),
       ('EASA_CS23_NORMAL', 'EASA CS-23 Normal',
        'EASA CS-23 Amendment 5 Normal category.', 'EASA', 20),
       ('EASA_CS25', 'EASA CS-25 Transport',
        'EASA CS-25 Transport category (large aircraft).', 'EASA', 21),
       ('MILITARY_SPEC', 'Military Specification',
        'Airworthiness governed by applicable military specifications.', NULL, 30)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    authority_code = EXCLUDED.authority_code,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.pilot_certificate_types (8 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.pilot_certificate_types
    (code, label, description, authority_code, sort_order)
VALUES ('FAA_STUDENT', 'Student Pilot',
        'FAA student pilot certificate; supervised solo operations only.', 'FAA', 10),
       ('FAA_SPORT', 'Sport Pilot',
        'FAA sport pilot certificate; privileges depend on aircraft performance, endorsements, '
        'ratings, and operating limitations.', 'FAA', 11),
       ('FAA_RECREATIONAL', 'Recreational Pilot',
        'FAA recreational pilot certificate.', 'FAA', 12),
       ('FAA_PRIVATE', 'Private Pilot (PPL)',
        'FAA private pilot licence; VFR and IFR with rating.', 'FAA', 13),
       ('FAA_COMMERCIAL', 'Commercial Pilot (CPL)',
        'FAA commercial pilot certificate; compensation and hire.', 'FAA', 14),
       ('FAA_ATP', 'Airline Transport Pilot (ATP)',
        'FAA ATP; required as PIC on air-carrier turbine aircraft.', 'FAA', 15),
       ('FAA_TYPE_RATING', 'Type Rating',
        'Aircraft-specific type rating required on some complex types.', 'FAA', 16),
       ('EASA_PPL', 'EASA PPL',
        'EASA Part-FCL private pilot licence.', 'EASA', 20)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    authority_code = EXCLUDED.authority_code,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- SEED DATA — aircraft_ref.operating_approval_types (14 rows)
-- =============================================================================

INSERT INTO aircraft_ref.operating_approval_types
    (code, label, description, is_positive, sort_order)
VALUES
    ('VFR_DAY',        'VFR Day',
     'Visual Flight Rules, daytime operations (baseline).',                 TRUE,   10),
    ('VFR_NIGHT',      'VFR Night',
     'Visual Flight Rules, night operations.',                              TRUE,   20),
    ('IFR',            'Instrument Flight Rules',
     'IFR operations; aircraft and avionics must meet IFR minimums.',       TRUE,   30),
    ('KNOWN_ICING_FIKI','Flight Into Known Icing (FIKI)',
     'Approved for flight in known icing conditions per certification.',     TRUE,   40),
    ('AEROBATIC',      'Aerobatic Operations',
     'Approved for intentional aerobatic maneuvers.',                       TRUE,   50),
    ('PRESSURIZED',    'Pressurized Cabin',
     'Aircraft has a pressurized cabin approved for high-altitude ops.',     TRUE,   60),
    ('CAT_I_ILS',      'Category I ILS Approach',
     'CAT I precision approach: decision height at least 200 ft and runway visual range at least '
     '2,400 ft; specifically authorized operations may use lower visibility minima.', TRUE, 70),
    ('CAT_II_ILS',     'Category II ILS Approach',
     'CAT II precision approach: decision height below 200 ft but at least 100 ft and runway '
     'visual range at least 1,200 ft.', TRUE, 71),
    ('CAT_III_ILS',    'Category III ILS Approach',
     'CAT III precision approach: decision height below 100 ft or none and runway visual range '
     'below 1,200 ft or none, according to the authorized CAT III subtype.', TRUE, 72),
    ('ETOPS_120',      'ETOPS 120 min',
     'Extended range twin-engine ops approved up to 120 min diversion.',    TRUE,   80),
    ('ETOPS_180',      'ETOPS 180 min',
     'Extended range twin-engine ops approved up to 180 min diversion.',    TRUE,   81),
    ('RVSM',           'Reduced Vertical Separation Minimum (RVSM)',
     'Approved for FL290–FL410 in RVSM airspace.',                          TRUE,   90),
    ('STEEP_APPROACH', 'Steep Approach (above 3°)',
     'Approved for approach glidepath steeper than standard 3°, '
     'e.g., London City Airport 5.5°.',                                     TRUE,  100),
    ('AMPHIBIOUS_OPS', 'Amphibious / Water Operations',
     'Approved for takeoff and landing on water surfaces.',                  TRUE,  110)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    is_positive = EXCLUDED.is_positive,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;
-- =============================================================================
-- GROUP 6: MILITARY LOOKUPS
-- stores_types.weapon_category_code FKs weapon_categories inserted above.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.military_mission_types (12 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.military_mission_types (code, label, description, sort_order)
VALUES ('AIR_SUPERIORITY', 'Air Superiority',
        'Establishing and holding control of the air over a contested area.', 10),
       ('CLOSE_AIR_SUPPORT', 'Close Air Support (CAS)',
        'Attack on targets near friendly ground forces, requiring detailed '
        'integration with them.', 20),
       ('DEEP_STRIKE', 'Deep Strike / Interdiction',
        'Attack behind the forward edge to disrupt forces before they reach the '
        'battle.', 30),
       ('STRATEGIC_BOMBING', 'Strategic Bombing',
        'Attack on industrial, infrastructure, or command targets rather than on '
        'fielded forces.', 40),
       ('MARITIME_PATROL', 'Maritime Patrol / ASW',
        'Overwater surveillance of surface and subsurface contacts, including '
        'anti-submarine prosecution.', 50),
       ('AIRBORNE_ISR', 'Airborne ISR / Reconnaissance',
        'Collection of imagery, signals, and electronic intelligence from an '
        'airborne sensor platform.', 60),
       ('AEW_C2', 'Airborne Early Warning / C2',
        'Extending the radar horizon and directing other aircraft from an '
        'airborne command position.', 70),
       ('ELECTRONIC_WARFARE', 'Electronic Attack / Warfare',
        'Degrading enemy sensors and communications by jamming, deception, or '
        'anti-radiation attack.', 80),
       ('AERIAL_REFUELLING', 'Aerial Refuelling',
        'Transferring fuel in flight to extend the radius or endurance of '
        'receiving aircraft.', 90),
       ('STRATEGIC_AIRLIFT', 'Strategic Airlift',
        'Moving forces and materiel between theatres over intercontinental '
        'distance.', 100),
       ('TACTICAL_AIRLIFT', 'Tactical Airlift',
        'Moving forces and supplies within a theatre, often into short or '
        'unprepared strips.', 110),
       ('SPECIAL_OPERATIONS', 'Special Operations Aviation',
        'Infiltration, exfiltration, and support of special operations forces, '
        'usually at night and at low level.', 120)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.weapon_categories (9 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.weapon_categories (code, label, description, sort_order)
VALUES ('AIR_TO_AIR', 'Air-to-Air Munitions',
        'Munitions carried to engage other aircraft.', 10),
       ('AIR_TO_GROUND', 'Air-to-Ground Munitions',
        'Munitions released against targets on land.', 20),
       ('ANTI_SHIP', 'Anti-Ship Munitions',
        'Munitions specialised for surface vessels, typically sea-skimming with '
        'a large warhead.', 30),
       ('ANTI_SUBMARINE', 'Anti-Submarine Weapons',
        'Torpedoes, depth charges, and mines used against submerged targets.', 40),
       ('GUNS_CANNON', 'Guns and Cannon',
        'Internal and podded gun armament. Distinguished from released munitions '
        'by having no separation event.', 50),
       ('EXTERNAL_STORES', 'External Stores (Non-Weapon)',
        'Carried items that are not weapons: fuel tanks, pylon adapters, and '
        'cargo containers.', 60),
       ('SENSOR_POD', 'Sensor / Surveillance Pod',
        'Podded sensors carried on a weapon station in place of a weapon.', 70),
       ('ELECTRONIC', 'Electronic Warfare Pod',
        'Podded jamming and electronic countermeasure equipment.', 80),
       ('SPECIAL_WEAPON', 'Special Weapon (Nuclear/CBRN)',
        'Nuclear and CBRN munitions, recorded as a carriage classification only. '
        'This dataset holds no yield, stockpile, or employment data.', 90)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.hardpoint_position_types (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.hardpoint_position_types (code, label, description, sort_order)
VALUES ('WING_INBOARD', 'Wing — Inboard',
        'Wing station nearest the fuselage, normally rated for the heaviest '
        'single load.', 10),
       ('WING_MIDBOARD', 'Wing — Mid-board',
        'Wing station between the inboard and outboard positions.', 20),
       ('WING_OUTBOARD', 'Wing — Outboard',
        'Outer wing station, limited by wing bending to lighter stores.', 30),
       ('WINGTIP', 'Wingtip',
        'Rail at the wing tip, conventionally carrying a short-range missile or '
        'a countermeasures pod.', 40),
       ('FUSELAGE_CENTERLINE', 'Fuselage Centreline',
        'Single station under the fuselage centreline, commonly used for a drop '
        'tank or a large pod.', 50),
       ('FUSELAGE_CONFORMAL', 'Fuselage — Conformal',
        'Semi-recessed or shoulder carriage hugging the fuselage to reduce the '
        'drag penalty of external stores.', 60),
       ('INTERNAL_BAY', 'Internal Weapons Bay',
        'Enclosed bay carrying stores inside the airframe, preserving low '
        'observability at the cost of usable volume.', 70)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.stores_types (12 rows)
-- weapon_category_code FK to weapon_categories (inserted above).
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.stores_types
    (code, label, description, weapon_category_code, sort_order)
VALUES ('AAM_SHORT_RANGE', 'Short-Range AAM',
        'Within-visual-range air-to-air missile for close manoeuvring combat, '
        'usually infrared guided.', 'AIR_TO_AIR', 10),
       ('AAM_MEDIUM_RANGE', 'Medium-Range AAM (BVR)',
        'Beyond-visual-range air-to-air missile, typically radar guided.',
        'AIR_TO_AIR', 11),
       ('AAM_LONG_RANGE', 'Long-Range AAM',
        'Very-long-range air-to-air missile aimed at high-value support aircraft '
        'rather than at fighters.', 'AIR_TO_AIR', 12),
       ('AGM_LASER', 'Laser-Guided AGM',
        'Powered air-to-ground missile riding a laser designation to the target.',
        'AIR_TO_GROUND', 20),
       ('LGB', 'Laser-Guided Bomb (LGB)',
        'Unpowered bomb with a laser seeker kit; needs the designation held to '
        'impact.', 'AIR_TO_GROUND', 21),
       ('JDAM', 'GPS-Guided Bomb (JDAM)',
        'Unpowered bomb with a satellite and inertial guidance kit; needs no '
        'designation after release.', 'AIR_TO_GROUND', 22),
       ('UNGUIDED_BOMB', 'Unguided / Iron Bomb',
        'Free-fall bomb with no guidance, aimed by the release solution alone.',
        'AIR_TO_GROUND', 23),
       ('ROCKET_POD', 'Unguided Rocket Pod',
        'Pod of unguided rockets for area suppression at short range.',
        'AIR_TO_GROUND', 24),
       ('GUN_POD', 'Gun / Cannon Pod',
        'Externally carried gun with its own ammunition, fitted to aircraft with '
        'no internal cannon.', 'GUNS_CANNON', 30),
       ('EXT_FUEL_TANK', 'External Fuel Tank (Drop)',
        'Jettisonable tank buying radius at the cost of a station and added '
        'drag.', 'EXTERNAL_STORES', 40),
       ('RECCE_POD', 'Reconnaissance Pod',
        'Podded imaging or signals sensors flown for reconnaissance tasking.',
        'SENSOR_POD', 50),
       ('ECM_POD', 'Electronic Warfare Pod',
        'Podded jammer providing self-protection or support jamming.',
        'ELECTRONIC', 60)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    weapon_category_code = EXCLUDED.weapon_category_code,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 7: MARKET / COST LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.currencies (6 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.currencies
    (code, label, symbol, decimal_places)
VALUES ('USD', 'US Dollar', '$', 2),
       ('EUR', 'Euro', '€', 2),
       ('GBP', 'British Pound', '£', 2),
       ('CAD', 'Canadian Dollar', 'CA$', 2),
       ('AUD', 'Australian Dollar', 'A$', 2),
       ('CHF', 'Swiss Franc', 'CHF', 2)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    symbol = EXCLUDED.symbol,
    decimal_places = EXCLUDED.decimal_places,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.cost_item_types (21 rows)
-- is_fixed TRUE = annual fixed cost; is_aggregate TRUE = source-provided total.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.cost_item_types
    (code, label, description, is_fixed, is_aggregate, sort_order)
VALUES
    -- Fixed annual costs
    ('ANNUAL_INSPECTION', 'Annual Inspection',
     'Regulatory annual airworthiness inspection.', TRUE, FALSE, 10),
    ('INSURANCE', 'Insurance',
     'Hull and liability insurance premium.', TRUE, FALSE, 11),
    ('HANGAR_STORAGE', 'Hangar / Tiedown / Storage',
     'Monthly or annual aircraft storage fee.', TRUE, FALSE, 12),
    ('DEPRECIATION', 'Depreciation',
     'Annual market-value reduction of the airframe.', TRUE, FALSE, 13),
    ('WEATHER_SERVICE', 'Weather / Data Services',
     'Weather briefing subscriptions and navigation database fees.', TRUE, FALSE, 14),
    ('PILOT_TRAINING', 'Pilot Training / Currency',
     'Recurrent training, simulator, and check-ride costs.', TRUE, FALSE, 15),
    ('REFURBISHING', 'Refurbishing / Modernisation',
     'Interior, paint, and avionics update reserves.', TRUE, FALSE, 16),
    ('REGISTRATION_TAXES', 'Registration / Taxes',
     'Annual aircraft registration and excise taxes.', TRUE, FALSE, 17),
    ('FINANCING', 'Financing Costs',
     'Loan interest or equivalent finance charges.', TRUE, FALSE, 18),
    -- Per-hour variable costs
    ('FUEL', 'Fuel',
     'Direct fuel cost per flight hour.', FALSE, FALSE, 30),
    ('OIL', 'Oil',
     'Engine oil consumption per flight hour.', FALSE, FALSE, 31),
    ('HOURLY_MAINTENANCE', 'Scheduled Maintenance',
     'Line maintenance and labour per flight hour.', FALSE, FALSE, 32),
    ('UNSCHEDULED_MAINT', 'Unscheduled Maintenance',
     'Reserve for unscheduled repairs and AOG situations.', FALSE, FALSE, 33),
    ('ENGINE_RESERVE', 'Engine Overhaul Reserve',
     'Per-hour accrual toward TBO overhaul cost.', FALSE, FALSE, 34),
    ('PROP_RESERVE', 'Propeller Reserve',
     'Per-hour accrual toward propeller overhaul.', FALSE, FALSE, 35),
    ('AVIONICS_RESERVE', 'Avionics Reserve',
     'Per-hour accrual for avionics maintenance and upgrades.', FALSE, FALSE, 36),
    ('LANDING_FEES', 'Landing / Navigation Fees',
     'Airport landing fees averaged per flight hour.', FALSE, FALSE, 37),
    ('MISC_VARIABLE', 'Miscellaneous Variable',
     'Catering, ground handling, parking, and sundry costs.', FALSE, FALSE, 38),
    -- Source-provided aggregate totals; routed to cost_snapshot_totals.
    ('TOTAL_COST_ANNUAL', 'Total Annual Cost',
     'Source-provided annual ownership cost total.', FALSE, TRUE, 90),
    ('TOTAL_FIXED_COST', 'Total Fixed Cost',
     'Source-provided annual fixed-cost total.', FALSE, TRUE, 91),
    ('TOTAL_VARIABLE_COST', 'Total Variable Cost',
     'Source-provided hourly variable-cost total.', FALSE, TRUE, 92)
    ON CONFLICT (code) DO UPDATE SET
        label = EXCLUDED.label,
        description = EXCLUDED.description,
        is_fixed = EXCLUDED.is_fixed,
        is_aggregate = EXCLUDED.is_aggregate,
        sort_order = EXCLUDED.sort_order,
        is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.aircraft_condition_grades (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.aircraft_condition_grades
    (code, label, description, numeric_score, sort_order)
VALUES ('EXCELLENT', 'Excellent',
        'Like-new or recently refurbished; all systems fully serviceable.', 5, 10),
       ('GOOD', 'Good',
        'Well-maintained; minor cosmetic wear only.', 4, 20),
       ('FAIR', 'Fair',
        'Serviceable with some deferred maintenance or cosmetic wear.', 3, 30),
       ('POOR', 'Poor',
        'Operational but significant maintenance required.', 2, 40),
       ('SALVAGE', 'Salvage',
        'Not airworthy; parts-only or rebuild project.', 1, 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    numeric_score = EXCLUDED.numeric_score,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 8: MAINTENANCE LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.ad_types (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.ad_types (code, label, description, sort_order)
VALUES ('RECURRING', 'Recurring',
        'Must be repeated at defined calendar or hourly intervals.', 10),
       ('ONE_TIME', 'One-Time',
        'Single compliance action; no repetition required.', 20),
       ('OPTIONAL_TERMINATING', 'Optional Terminating Action',
        'One-time action that terminates a recurring compliance requirement.', 30),
       ('ALERT_SB', 'Alert Service Bulletin',
        'Urgent manufacturer safety communication; may precede formal AD.', 40),
       ('EMERGENCY', 'Emergency AD',
        'Immediate action required before next flight.', 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.sb_compliance_statuses (6 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.sb_compliance_statuses (code, label, description, sort_order)
VALUES ('MANDATORY', 'Mandatory',
        'The manufacturer classes the bulletin as required. An authority may '
        'separately compel it by airworthiness directive; that is recorded in '
        'ad_types, not here.', 10),
       ('RECOMMENDED', 'Recommended',
        'The manufacturer advises the work but does not require it for continued '
        'airworthiness.', 20),
       ('OPTIONAL', 'Optional',
        'Offered as an improvement or convenience; declining it carries no '
        'airworthiness consequence.', 30),
       ('COMPLIED', 'Complied With',
        'The work described by the bulletin has been carried out on this '
        'airframe.', 40),
       ('SUPERSEDED', 'Superseded',
        'Replaced by a later bulletin. Comply with the successor rather than with '
        'this one.', 50),
       ('NOT_APPLICABLE', 'Not Applicable',
        'Outside the effectivity of the bulletin: serial range, configuration, or '
        'engine fit does not match.', 60)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.availability_grades (5 rows)
-- Shared by parts_availability and maintenance_network assessments.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.availability_grades
    (code, label, description, numeric_score, sort_order)
VALUES ('EXCELLENT', 'Excellent',
        'Widely available worldwide; no sourcing concerns.', 5, 10),
       ('GOOD', 'Good',
        'Readily available; minor lead time or regional variation.', 4, 20),
       ('FAIR', 'Fair',
        'Available with meaningful lead time or a limited supplier base.', 3, 30),
       ('POOR', 'Poor',
        'Difficult to source; significant lead time or cost premium.', 2, 40),
       ('CRITICAL', 'Critical / Scarce',
        'Obsolete or near-obsolete; stockpile-only supply.', 1, 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    numeric_score = EXCLUDED.numeric_score,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 9: PROVENANCE / CURATION LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.source_types (8 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.source_types (code, label, description, sort_order)
VALUES ('SCRAPED_WEB', 'Scraped Web Page',
        'Data obtained by automated web scraping.', 10),
       ('MANUFACTURER_SPEC', 'Manufacturer Spec Sheet',
        'Official manufacturer specifications document or datasheet.', 20),
       ('TYPE_CERTIFICATE', 'Type Certificate Data Sheet (TCDS)',
        'FAA / EASA TCDS or equivalent official TC data sheet.', 30),
       ('POH_AFM', 'POH / AFM',
        'Pilot Operating Handbook or Airplane Flight Manual.', 40),
       ('JANES', 'Jane''s All the World''s Aircraft',
        'IHS Markit / Jane''s reference publication.', 50),
       ('IMPORTED_DATASET', 'Imported Dataset',
        'Third-party dataset batch import.', 60),
       ('MANUAL_ENTRY', 'Manual Entry',
        'Human-curated entry by the editorial team.', 70),
       ('CALCULATED', 'Calculated / Derived',
        'Value derived from other source values by formula.', 80)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.source_reliability_grades (5 rows)
-- numeric_score / 5 ≈ default confidence_score for assertions from this source.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.source_reliability_grades
    (code, label, description, numeric_score, sort_order)
VALUES ('AUTHORITATIVE', 'Authoritative',
        'Type certificate, POH/AFM, or regulatory filing.', 5, 10),
       ('HIGH', 'High',
        'Official manufacturer publication or Jane''s-class reference.', 4, 20),
       ('MEDIUM', 'Medium',
        'Reputable third-party publication or well-curated dataset.', 3, 30),
       ('LOW', 'Low',
        'Secondary source with limited verifiability.', 2, 40),
       ('UNVERIFIED', 'Unverified',
        'Raw scraped or crowd-sourced; no independent verification.', 1, 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    numeric_score = EXCLUDED.numeric_score,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.curation_flag_statuses (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.curation_flag_statuses
    (code, label, description, is_terminal, sort_order)
VALUES ('OPEN', 'Open',
        'Issue identified; not yet reviewed by a curator.', FALSE, 10),
       ('UNDER_REVIEW', 'Under Review',
        'Assigned to curator; investigation in progress.', FALSE, 20),
       ('RESOLVED', 'Resolved',
        'Issue resolved; canonical value confirmed or updated.', TRUE, 30),
       ('DISMISSED', 'Dismissed',
        'Investigated and found not significant; no action taken.', TRUE, 40),
       ('DEFERRED', 'Deferred',
        'Acknowledged but deferred to a later curation cycle.', FALSE, 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    is_terminal = EXCLUDED.is_terminal,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.curation_entity_types (11 rows)
-- schema_name / table_name document the real PostgreSQL table for each type.
-- These will match the tables created in later phases.
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.curation_entity_types
    (code, label, description, schema_name, table_name, sort_order)
VALUES ('AIRCRAFT_FAMILY', 'Aircraft Family',
        'A design lineage grouping related models under one programme. Curating a '
        'family changes how its models roll up, never their own figures.',
        'aircraft_core', 'families', 10),
       ('AIRCRAFT_MODEL', 'Aircraft Model',
        'A named type within a family. Most published specifications are quoted '
        'at this level even when they describe one variant.',
        'aircraft_core', 'models', 11),
       ('AIRCRAFT_VARIANT', 'Aircraft Variant',
        'A specific configuration of a model, and the level at which performance '
        'and weight figures are actually comparable.',
        'aircraft_core', 'variants', 12),
       ('ENGINE_SPEC', 'Engine Specification',
        'A powerplant as installed on a variant, including its rating. Source '
        'manufacturer names often arrive unmatched and are resolved here.',
        'aircraft_power', 'engine_variants', 20),
       ('PERFORMANCE_METRIC', 'Performance Metric Value',
        'One measured performance value for a variant. Accepting an assertion '
        'against it promotes that value to canonical.',
        'aircraft_specs', 'performance_metrics', 30),
       ('WEIGHT_METRIC', 'Weight Metric Value',
        'One measured weight or capacity value for a variant.',
        'aircraft_specs', 'weight_metrics', 31),
       ('DIMENSION_METRIC', 'Dimension Metric Value',
        'One measured physical dimension for a variant.',
        'aircraft_specs', 'dimension_metrics', 32),
       ('COST_SNAPSHOT', 'Ownership Cost Snapshot',
        'Ownership and operating cost captured at a point in time. Meaningful '
        'only alongside the assumptions recorded with it.',
        'aircraft_market', 'cost_snapshots', 40),
       ('VALUATION', 'Market Valuation',
        'Market value estimated at a point in time. Superseded by a later '
        'estimate rather than corrected in place.',
        'aircraft_market', 'valuations', 41),
       ('SOURCE_DOCUMENT', 'Source Document',
        'The document a set of assertions was drawn from. Curating its '
        'reliability changes what every dependent assertion inherits.',
        'aircraft_prov', 'source_documents', 50),
       ('MANUFACTURER', 'Manufacturer / Organisation',
        'An organisation record. Ingestion leaves an unrecognised manufacturer '
        'name as a raw string for a curator to match here.',
        'aircraft_org', 'organizations', 60)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    schema_name = EXCLUDED.schema_name,
    table_name = EXCLUDED.table_name,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.assertion_statuses (5 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.assertion_statuses
    (code, label, description, sort_order)
VALUES ('PENDING', 'Pending',
        'Ingested but not yet reviewed by a curator.', 10),
       ('ACCEPTED', 'Accepted',
        'Accepted as the canonical or best-available value.', 20),
       ('REJECTED', 'Rejected',
        'Rejected; superseded by a better source or identified as error.', 30),
       ('CONFLICT', 'In Conflict',
        'Conflicts with an assertion from another source.', 40),
       ('SUPERSEDED', 'Superseded',
        'A newer assertion from the same or better source now exists.', 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 10: COMPARISON / MISSION LOOKUPS
-- comparison_criterion_types FKs to performance/weight/dimension_metric_types
-- inserted earlier in this file; validated at COMMIT (DEFERRABLE).
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.mission_profile_types (15 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.mission_profile_types (code, label, description, sort_order)
VALUES ('PERSONAL_VFR_TOURING', 'Personal VFR Touring',
        'Weekend and holiday travel by a private owner in visual conditions, '
        'where cost outweighs performance.', 10),
       ('IFR_CROSSCOUNTRY', 'IFR Cross-Country Travel',
        'Single-pilot instrument travel decided by range, cruise speed, and '
        'certified avionics.', 20),
       ('BUSINESS_TRAVEL', 'Business Aviation Travel',
        'Multi-passenger corporate travel judged on cost per seat-mile against '
        'cabin comfort.', 30),
       ('FLIGHT_TRAINING', 'Flight Training',
        'Instructional flying where docile handling and cost per hour outrank '
        'speed and range.', 40),
       ('BACKCOUNTRY_STOL', 'Backcountry / STOL Operations',
        'Short unimproved strips, where takeoff and landing distance decide the '
        'choice before cruise speed does.', 50),
       ('FLOATPLANE_OPERATIONS', 'Float-plane / Amphibious Ops',
        'Operation from water on floats or a hull, accepting the cruise and '
        'payload penalty that carriage brings.', 55),
       ('CARGO_FREIGHT', 'Cargo / Freight Transport',
        'Freight carriage judged on usable volume, door size, and payload rather '
        'than on seating.', 60),
       ('MEDEVAC_SAR', 'Medevac / Search and Rescue',
        'Patient transport and rescue tasking: cabin access, endurance, and '
        'dispatch reliability.', 70),
       ('PATROL_SURVEILLANCE', 'Patrol / Surveillance',
        'Long loiter over an area with sensors aboard, where endurance outranks '
        'speed.', 80),
       ('HIGH_ALTITUDE_OPS', 'High-Altitude Operations',
        'Operation from high-elevation or hot airfields, where density altitude '
        'erodes climb and runway margin.', 85),
       ('AEROBATICS', 'Aerobatics / Air Show',
        'Competition and display flying, needing a stressed airframe and high '
        'roll rate rather than range.', 90),
       ('MILITARY_CLOSE_AIR_SUPPORT', 'Military: Close Air Support',
        'Supporting troops in contact, weighted toward loiter and survivability '
        'at low level.', 100),
       ('MILITARY_TRANSPORT_AIRLIFT', 'Military: Transport / Airlift',
        'Military lift judged on payload, field length, and airdrop capability.',
        110),
       ('MILITARY_MARITIME_PATROL', 'Military: Maritime Patrol',
        'Overwater patrol and anti-submarine work at long endurance.', 120),
       ('UNMANNED_SPECIAL_MISSION', 'Unmanned / Special Mission',
        'Uncrewed or one-off tasking, where seating and cabin comfort do not '
        'apply as criteria at all.', 130)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.comparison_criterion_types (12 rows)
-- Metric FKs wired here; all three FK columns are DEFERRABLE.
-- CRITERION_FUEL_EFFICIENCY, CRITERION_PAX_SEATS, CRITERION_PRICE, and
-- CRITERION_HOURLY_COST are computed / non-metric criteria (all metric FKs NULL).
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.comparison_criterion_types
(code, label, description,
 performance_metric_code, weight_metric_code, dimension_metric_code,
 is_higher_better, sort_order)
VALUES ('CRITERION_CRUISE_SPEED', 'Cruise Speed',
        'Block speed at normal cruise. Trades against fuel burn and, on most '
        'types, against range.',
        'SPEED_CRUISE_BEST', NULL, NULL, TRUE, 10),
       ('CRITERION_RANGE', 'Range',
        'Still-air distance at normal cruise. Buying range normally costs '
        'payload.',
        'RANGE_NORMAL', NULL, NULL, TRUE, 20),
       ('CRITERION_CEILING', 'Service Ceiling',
        'Usable operating altitude, which governs terrain and weather avoidance '
        'and turbine cruise efficiency.',
        'CEILING_SERVICE', NULL, NULL, TRUE, 30),
       ('CRITERION_CLIMB_RATE', 'Rate of Climb',
        'Sea-level climb performance, standing in for excess power and for '
        'terrain escape after departure.',
        'CLIMB_RATE_SL', NULL, NULL, TRUE, 40),
       -- Fuel efficiency is nm/gal; computed in Phase 15 views, no single metric
       ('CRITERION_FUEL_EFFICIENCY', 'Fuel Efficiency (nm/gal)',
        'Distance flown per unit of fuel. Computed from cruise speed and cruise '
        'burn, so no single metric backs it.',
        NULL, NULL, NULL, TRUE, 50),
       ('CRITERION_PAYLOAD', 'Payload Capacity',
        'Useful load left for people and cargo once fuel is aboard.',
        NULL, 'WEIGHT_PAYLOAD', NULL, TRUE, 60),
       -- Passenger seats come from aircraft_core.variants attribute, not a metric
       ('CRITERION_PAX_SEATS', 'Passenger Seats',
        'Seats installed, read from the variant record rather than from a '
        'measured metric.',
        NULL, NULL, NULL, TRUE, 70),
       ('CRITERION_RUNWAY_TAKEOFF', 'Takeoff Distance',
        'Runway needed to depart over an obstacle. Lower is better, and it gates '
        'which airfields the aircraft can use at all.',
        'DIST_TO_50FT', NULL, NULL, FALSE, 80),
       ('CRITERION_RUNWAY_LANDING', 'Landing Distance',
        'Runway needed to arrive over an obstacle. Lower is better.',
        'DIST_LDG_50FT', NULL, NULL, FALSE, 90),
       -- Price and hourly cost come from aircraft_market, not metric tables
       ('CRITERION_PRICE', 'Acquisition Price',
        'Purchase cost. Lower is better, and it comes from market data rather '
        'than from any metric table.',
        NULL, NULL, NULL, FALSE, 100),
       ('CRITERION_HOURLY_COST', 'Total Hourly Operating Cost',
        'All-in cost per flight hour, valid only under the fuel price and '
        'utilisation assumptions recorded with the snapshot.',
        NULL, NULL, NULL, FALSE, 110),
       ('CRITERION_WINGSPAN', 'Wingspan',
        'Span across the wings. Direction is deliberately unset: more span buys '
        'efficiency, less span buys hangar and ramp access.',
        NULL, NULL, 'DIM_WINGSPAN', NULL, 120)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    performance_metric_code = EXCLUDED.performance_metric_code,
    weight_metric_code = EXCLUDED.weight_metric_code,
    dimension_metric_code = EXCLUDED.dimension_metric_code,
    is_higher_better = EXCLUDED.is_higher_better,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 11: ORGANIZATION LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.organization_types (10 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.organization_types (code, label, description, sort_order)
VALUES ('MANUFACTURER', 'Aircraft Manufacturer',
        'Builds complete aircraft under its own type certificate.', 10),
       ('DESIGN_BUREAU', 'Design Bureau',
        'Designs types that a separate plant builds, so the design name and the '
        'builder name differ on the same airframe.', 11),
       ('LICENSE_MANUFACTURER', 'Licensed Manufacturer',
        'Builds another design under licence. Licence-built airframes can differ '
        'from the original in fit and standard.', 12),
       ('OPERATOR_MILITARY', 'Military Operator',
        'Armed service or defence ministry operating aircraft.', 20),
       ('OPERATOR_COMMERCIAL', 'Commercial Airline / Air Taxi',
        'Airline, charter, or air-taxi operator flying for hire.', 21),
       ('OPERATOR_GOVERNMENT', 'Government / State Operator',
        'State or agency operator outside the armed services: coastguard, police, '
        'survey, and similar.', 22),
       ('OPERATOR_PRIVATE', 'Private Aviation Operator',
        'Corporate or individual owner flying for its own account.', 23),
       ('CERTIFICATION_AUTHORITY', 'Certification Authority',
        'Regulator issuing type certificates and airworthiness approvals. The '
        'authorities themselves are enumerated in certification_authorities.', 30),
       ('MAINTENANCE_PROVIDER', 'MRO / Maintenance Provider',
        'Maintenance, repair, and overhaul organisation.', 40),
       ('INDUSTRY_ASSOCIATION', 'Industry Association',
        'Trade or standards body. Appears in this dataset as a source of '
        'published figures rather than as an operator.', 50)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.org_relationship_types (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.org_relationship_types
    (code, label, description, sort_order)
VALUES ('SUBSIDIARY', 'Subsidiary',
        'Child company wholly or majority owned by a parent.', 10),
       ('SUCCESSOR_ENTITY', 'Successor Entity',
        'Historical succession after renaming or acquisition.', 20),
       ('JOINT_VENTURE', 'Joint Venture',
        'Co-production or co-development arrangement.', 30),
       ('LICENSE_AGREEMENT', 'License Agreement',
        'Licensed production of a type granted to another company.', 40),
       ('DESIGN_AUTHORITY', 'Design Authority',
        'Holds the type certificate and primary design responsibility.', 50),
       ('MAJOR_SUBCONTRACT', 'Major Subcontractor',
        'Supplies critical structural or system components.', 60),
       ('CONSORTIUM_MEMBER', 'Consortium Member',
        'Co-development and co-production consortium membership.', 70)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- =============================================================================
-- GROUP 12: AVIONICS / SYSTEMS LOOKUPS
-- =============================================================================

-- -----------------------------------------------------------------------------
-- aircraft_ref.systems_categories (15 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.systems_categories (code, label, description, sort_order)
VALUES ('NAVIGATION', 'Navigation Systems',
        'Position and course guidance: satellite receivers, ground-based '
        'navigation radios, and area navigation.', 10),
       ('COMMUNICATION', 'Communication Systems',
        'Voice and data radios, including satellite communication where fitted.',
        20),
       ('AUTOPILOT_FMS', 'Autopilot / FMS',
        'Automatic flight control and flight management, from a wing leveller to '
        'a coupled approach.', 30),
       ('FLIGHT_INSTRUMENTS', 'Flight Instruments / EFIS',
        'Primary flight and navigation displays, electronic or mechanical.', 40),
       ('ENGINE_MONITORING', 'Engine / Systems Monitoring',
        'Powerplant and systems indication, including trend recording where '
        'fitted.', 50),
       ('TERRAIN_AWARENESS', 'Terrain Awareness (TAWS / GPWS)',
        'Terrain and obstacle alerting against a stored terrain database.', 60),
       ('TRAFFIC_AWARENESS', 'Traffic / ADS-B',
        'Traffic display and collision alerting, including automatic dependent '
        'surveillance.', 70),
       ('WEATHER', 'Weather Detection / Datalink',
        'Onboard weather radar, lightning detection, and uplinked weather '
        'products.', 80),
       ('ICE_PROTECTION', 'Ice Protection (FIKI / De-Ice)',
        'Airframe and powerplant anti-ice and de-ice provisions. Decides whether '
        'a variant may be dispatched into known icing.', 90),
       ('PRESSURIZATION', 'Pressurization / Oxygen',
        'Cabin pressurisation and supplemental oxygen, which together set the '
        'usable cruise altitude.', 100),
       ('SURVEILLANCE_RECON', 'Surveillance / Recon (Military)',
        'Military imaging and signals collection equipment.', 110),
       ('EW_SYSTEMS', 'Electronic Warfare (Military)',
        'Military threat warning, jamming, and countermeasure dispensing.', 120),
       ('DATALINK_TACTICAL', 'Tactical Datalink (Military)',
        'Military data links sharing a common tactical picture between '
        'platforms.', 130),
       ('EMERGENCY_SAFETY', 'Emergency / Safety Systems',
        'Emergency locator, airframe parachute, fire suppression, and egress '
        'provisions.', 140),
       ('LIGHTING', 'Interior / Exterior Lighting',
        'Position, anti-collision, landing, and cabin lighting.', 150)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

-- -----------------------------------------------------------------------------
-- aircraft_ref.equipment_provision_types (7 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.equipment_provision_types
    (code, label, description, sort_order)
VALUES ('STANDARD', 'Standard Equipment',
        'Included in the base aircraft at no extra charge.', 10),
       ('OPTIONAL_FACTORY', 'Factory Option',
        'Available as a factory-installed option at additional cost.', 20),
       ('OPTIONAL_DEALER', 'Dealer Option',
        'Available as a dealer-installed option.', 30),
       ('RETROFIT_STC', 'STC Retrofit',
        'Available via FAA / EASA Supplemental Type Certificate.', 40),
       ('RETROFIT_337', 'Field Approval (FAA Form 337)',
        'Field-approved installation under FAA Form 337.', 50),
       ('NOT_AVAILABLE', 'Not Available',
        'Cannot be installed or approved on this aircraft type.', 60),
       ('REMOVED', 'Removed',
        'Was offered but production or STC has been discontinued.', 70)
ON CONFLICT (code) DO UPDATE SET
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    sort_order = EXCLUDED.sort_order,
    is_active = EXCLUDED.is_active;

COMMIT;
