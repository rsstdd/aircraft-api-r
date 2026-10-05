-- =============================================================================
-- File: database/seeds/003_mission_profile_seed_data.sql
-- Phase 15 -- canonical aircraft_compare mission policy.
--
-- The five original profiles retain their configured criteria and weights;
-- previously missing bounds are now explicit. The other ten profiles use the
-- six-criterion v1 repository policy recorded below. Bounds use the canonical
-- criterion units: KNOTS, NM, FT, LBS, USD, or USD/hr.
--
-- Reapplication compares the desired matrix before writing it. Cached scores
-- are deleted only for profiles whose criteria policy changed. Completeness,
-- convergence, identity stability, and selective cache invalidation are gated
-- by crates/aircraft_testsupport/tests/seed_data.rs.
-- =============================================================================

BEGIN;

INSERT INTO aircraft_compare.mission_profiles
    (profile_type_code, slug, title, description,
     typical_range_nm, typical_pax_count, typical_altitude_ft,
     applies_to_civilian, applies_to_military, sort_order)
VALUES
  ('PERSONAL_VFR_TOURING',
   'personal-vfr-touring', 'Personal VFR Touring',
   'Weekend and holiday VFR cross-country travel; cost-sensitive private owner. '
   'Prioritises acquisition cost, operating economy, and basic cross-country range.',
   400, 2, 6500, TRUE, FALSE, 10),
  ('IFR_CROSSCOUNTRY',
   'ifr-crosscountry', 'IFR Cross-Country Travel',
   'Single-pilot IFR operations for business or personal travel. '
   'Range, cruise speed, certified IFR avionics, and service ceiling are primary drivers.',
   800, 2, 10000, TRUE, FALSE, 20),
  ('BUSINESS_TRAVEL',
   'business-travel', 'Business Aviation Travel',
   'Multi-passenger business travel: speed, comfort, and cabin capacity '
   'balanced against operating cost per seat-mile.',
   1200, 4, 25000, TRUE, FALSE, 30),
  ('FLIGHT_TRAINING',
   'flight-training', 'Flight Training',
   'Primary and instrument flight training. Docile handling, '
   'low hourly operating cost, and short-field capability are key.',
   150, 2, 4000, TRUE, FALSE, 40),
  ('BACKCOUNTRY_STOL',
   'backcountry-stol', 'Backcountry / STOL Operations',
   'Short-field and off-airport operations on grass, gravel, and dirt strips. '
   'Takeoff and landing distance are the primary discriminators.',
   300, 2, 5000, TRUE, FALSE, 50),
  ('FLOATPLANE_OPERATIONS',
   'floatplane-operations', 'Float-plane / Amphibious Ops',
   'Water-based operations from lakes, rivers, and coastal inlets.',
   300, 3, 4000, TRUE, FALSE, 55),
  ('CARGO_FREIGHT',
   'cargo-freight', 'Cargo / Freight Transport',
   'Point-to-point cargo and freight operations. '
   'Payload capacity, range, and cargo door access are primary.',
   600, 0, 8000, TRUE, FALSE, 60),
  ('MEDEVAC_SAR',
   'medevac-sar', 'Medevac / Search and Rescue',
   'Medical evacuation and search-and-rescue operations. '
   'Short-field capability, payload, and range are all important.',
   400, 2, 6000, TRUE, FALSE, 70),
  ('PATROL_SURVEILLANCE',
   'patrol-surveillance', 'Patrol / Surveillance',
   'Aerial patrol, survey, and border surveillance. '
   'Endurance and low-speed flying are primary drivers.',
   500, 2, 8000, TRUE, FALSE, 80),
  ('HIGH_ALTITUDE_OPS',
   'high-altitude-ops', 'High-Altitude Operations',
   'Operations at density altitudes above 10,000 ft pressure altitude. '
   'Service ceiling and high-altitude propulsion performance are primary.',
   300, 2, 12000, TRUE, FALSE, 85),
  ('AEROBATICS',
   'aerobatics', 'Aerobatics / Air Show',
   'Intentional aerobatic manoeuvres and air-show display flying. '
   'Climb performance, operating cost, and acquisition price are compared.',
   100, 1, 4000, TRUE, FALSE, 90),
  ('MILITARY_CLOSE_AIR_SUPPORT',
   'military-cas', 'Military: Close Air Support',
   'Public reference comparison for close-air-support aircraft. '
   'Encyclopedia comparison only; no operational or tactical content.',
   400, 1, 20000, FALSE, TRUE, 100),
  ('MILITARY_TRANSPORT_AIRLIFT',
   'military-transport', 'Military: Transport / Airlift',
   'Public reference comparison for strategic and tactical airlift aircraft.',
   2000, 0, 28000, FALSE, TRUE, 110),
  ('MILITARY_MARITIME_PATROL',
   'military-maritime-patrol', 'Military: Maritime Patrol',
   'Public reference comparison for maritime patrol and anti-submarine aircraft.',
   1000, 4, 15000, FALSE, TRUE, 120),
  ('UNMANNED_SPECIAL_MISSION',
   'unmanned-special-mission', 'Unmanned / Special Mission',
   'Comparison of uncrewed and special-mission aircraft using public reference data.',
   800, 0, 30000, TRUE, TRUE, 130)
ON CONFLICT (profile_type_code) DO UPDATE SET
    slug = EXCLUDED.slug,
    title = EXCLUDED.title,
    description = EXCLUDED.description,
    typical_range_nm = EXCLUDED.typical_range_nm,
    typical_pax_count = EXCLUDED.typical_pax_count,
    typical_altitude_ft = EXCLUDED.typical_altitude_ft,
    applies_to_civilian = EXCLUDED.applies_to_civilian,
    applies_to_military = EXCLUDED.applies_to_military,
    is_active = EXCLUDED.is_active,
    sort_order = EXCLUDED.sort_order;

CREATE TEMP TABLE desired_mission_criteria
(
    profile_type_code   aircraft_ref.lookup_code NOT NULL,
    criterion_type_code aircraft_ref.lookup_code NOT NULL,
    weight              NUMERIC(4,3) NOT NULL,
    is_required         BOOLEAN NOT NULL,
    scoring_lower_bound NUMERIC NOT NULL,
    scoring_upper_bound NUMERIC NOT NULL,
    notes               TEXT NOT NULL,
    PRIMARY KEY (profile_type_code, criterion_type_code)
) ON COMMIT DROP;

INSERT INTO desired_mission_criteria
    (profile_type_code, criterion_type_code, weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
VALUES
  ('PERSONAL_VFR_TOURING', 'CRITERION_PRICE', 0.300, FALSE, 20000, 200000,
   'USD; 20,000 preferred, 200,000 upper scoring bound.'),
  ('PERSONAL_VFR_TOURING', 'CRITERION_HOURLY_COST', 0.200, FALSE, 30, 150,
   'USD/hr; 30 preferred, 150 upper scoring bound.'),
  ('PERSONAL_VFR_TOURING', 'CRITERION_CRUISE_SPEED', 0.200, FALSE, 80, 150,
   'KNOTS; 80 minimum, 150 ideal.'),
  ('PERSONAL_VFR_TOURING', 'CRITERION_RANGE', 0.150, FALSE, 200, 800,
   'NM; 200 minimum, 800 ideal.'),
  ('PERSONAL_VFR_TOURING', 'CRITERION_PAX_SEATS', 0.100, FALSE, 1, 4,
   'Seats including pilot; 1 minimum, 4 ideal.'),
  ('PERSONAL_VFR_TOURING', 'CRITERION_FUEL_EFFICIENCY', 0.050, FALSE, 5, 25,
   'NM/gal; 5 minimum, 25 ideal.'),
  ('IFR_CROSSCOUNTRY', 'CRITERION_RANGE', 0.300, TRUE, 400, 1500,
   'NM; 400 required minimum, 1,500 ideal.'),
  ('IFR_CROSSCOUNTRY', 'CRITERION_CRUISE_SPEED', 0.200, TRUE, 100, 220,
   'KNOTS; 100 required minimum, 220 ideal.'),
  ('IFR_CROSSCOUNTRY', 'CRITERION_CEILING', 0.150, TRUE, 10000, 25000,
   'FT; 10,000 required minimum, 25,000 ideal.'),
  ('IFR_CROSSCOUNTRY', 'CRITERION_FUEL_EFFICIENCY', 0.150, FALSE, 4, 20,
   'NM/gal; 4 minimum, 20 ideal.'),
  ('IFR_CROSSCOUNTRY', 'CRITERION_PAX_SEATS', 0.100, FALSE, 1, 4,
   'Seats including pilot; 1 minimum, 4 ideal.'),
  ('IFR_CROSSCOUNTRY', 'CRITERION_PRICE', 0.100, FALSE, 50000, 1000000,
   'USD; 50,000 preferred, 1,000,000 upper scoring bound.'),
  ('BACKCOUNTRY_STOL', 'CRITERION_RUNWAY_TAKEOFF', 0.350, TRUE, 500, 2500,
   'FT; 500 preferred, 2,500 upper scoring bound.'),
  ('BACKCOUNTRY_STOL', 'CRITERION_RUNWAY_LANDING', 0.350, TRUE, 500, 2500,
   'FT; 500 preferred, 2,500 upper scoring bound.'),
  ('BACKCOUNTRY_STOL', 'CRITERION_PAYLOAD', 0.200, FALSE, 300, 1500,
   'LBS; 300 minimum, 1,500 ideal.'),
  ('BACKCOUNTRY_STOL', 'CRITERION_CRUISE_SPEED', 0.070, FALSE, 60, 130,
   'KNOTS; 60 minimum, 130 ideal.'),
  ('BACKCOUNTRY_STOL', 'CRITERION_PRICE', 0.030, FALSE, 30000, 500000,
   'USD; 30,000 preferred, 500,000 upper scoring bound.'),
  ('BUSINESS_TRAVEL', 'CRITERION_CRUISE_SPEED', 0.250, FALSE, 150, 400,
   'KNOTS; 150 minimum, 400 ideal.'),
  ('BUSINESS_TRAVEL', 'CRITERION_RANGE', 0.250, FALSE, 500, 2500,
   'NM; 500 minimum, 2,500 ideal.'),
  ('BUSINESS_TRAVEL', 'CRITERION_PAX_SEATS', 0.200, FALSE, 2, 8,
   'Seats including pilot; 2 minimum, 8 ideal.'),
  ('BUSINESS_TRAVEL', 'CRITERION_HOURLY_COST', 0.150, FALSE, 100, 2500,
   'USD/hr; 100 preferred, 2,500 upper scoring bound.'),
  ('BUSINESS_TRAVEL', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 2000, 5000,
   'FT; 2,000 preferred, 5,000 upper scoring bound.'),
  ('BUSINESS_TRAVEL', 'CRITERION_CEILING', 0.050, FALSE, 20000, 45000,
   'FT; 20,000 minimum, 45,000 ideal.'),
  ('FLIGHT_TRAINING', 'CRITERION_PRICE', 0.300, FALSE, 10000, 120000,
   'USD; 10,000 preferred, 120,000 upper scoring bound.'),
  ('FLIGHT_TRAINING', 'CRITERION_HOURLY_COST', 0.300, FALSE, 30, 250,
   'USD/hr; 30 preferred, 250 upper scoring bound.'),
  ('FLIGHT_TRAINING', 'CRITERION_RUNWAY_LANDING', 0.200, FALSE, 800, 3000,
   'FT; 800 preferred, 3,000 upper scoring bound.'),
  ('FLIGHT_TRAINING', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 60, 130,
   'KNOTS; 60 minimum, 130 ideal.'),
  ('FLIGHT_TRAINING', 'CRITERION_PAX_SEATS', 0.100, FALSE, 1, 2,
   'Seats including pilot; 1 minimum, 2 ideal.'),
  ('FLOATPLANE_OPERATIONS', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 70, 180,
   'KNOTS; 70 minimum, 180 ideal.'),
  ('FLOATPLANE_OPERATIONS', 'CRITERION_RANGE', 0.250, FALSE, 150, 800,
   'NM; 150 minimum, 800 ideal.'),
  ('FLOATPLANE_OPERATIONS', 'CRITERION_PAYLOAD', 0.200, FALSE, 300, 2000,
   'LBS; 300 minimum, 2,000 ideal.'),
  ('FLOATPLANE_OPERATIONS', 'CRITERION_RUNWAY_TAKEOFF', 0.200, FALSE, 500, 3000,
   'FT; 500 preferred, 3,000 upper scoring bound.'),
  ('FLOATPLANE_OPERATIONS', 'CRITERION_RUNWAY_LANDING', 0.150, FALSE, 500, 3000,
   'FT; 500 preferred, 3,000 upper scoring bound.'),
  ('FLOATPLANE_OPERATIONS', 'CRITERION_HOURLY_COST', 0.100, FALSE, 50, 500,
   'USD/hr; 50 preferred, 500 upper scoring bound.'),
  ('CARGO_FREIGHT', 'CRITERION_CRUISE_SPEED', 0.050, FALSE, 100, 450,
   'KNOTS; 100 minimum, 450 ideal.'),
  ('CARGO_FREIGHT', 'CRITERION_RANGE', 0.250, TRUE, 500, 4000,
   'NM; 500 required minimum, 4,000 ideal.'),
  ('CARGO_FREIGHT', 'CRITERION_PAYLOAD', 0.350, TRUE, 1000, 50000,
   'LBS; 1,000 required minimum, 50,000 ideal.'),
  ('CARGO_FREIGHT', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 1500, 8000,
   'FT; 1,500 preferred, 8,000 upper scoring bound.'),
  ('CARGO_FREIGHT', 'CRITERION_RUNWAY_LANDING', 0.100, FALSE, 1500, 8000,
   'FT; 1,500 preferred, 8,000 upper scoring bound.'),
  ('CARGO_FREIGHT', 'CRITERION_HOURLY_COST', 0.150, FALSE, 200, 10000,
   'USD/hr; 200 preferred, 10,000 upper scoring bound.'),
  ('MEDEVAC_SAR', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 100, 300,
   'KNOTS; 100 minimum, 300 ideal.'),
  ('MEDEVAC_SAR', 'CRITERION_RANGE', 0.200, TRUE, 300, 1500,
   'NM; 300 required minimum, 1,500 ideal.'),
  ('MEDEVAC_SAR', 'CRITERION_CEILING', 0.100, FALSE, 10000, 30000,
   'FT; 10,000 minimum, 30,000 ideal.'),
  ('MEDEVAC_SAR', 'CRITERION_PAYLOAD', 0.150, FALSE, 500, 3000,
   'LBS; 500 minimum, 3,000 ideal.'),
  ('MEDEVAC_SAR', 'CRITERION_RUNWAY_TAKEOFF', 0.200, TRUE, 500, 3000,
   'FT; 500 preferred, 3,000 required upper scoring bound.'),
  ('MEDEVAC_SAR', 'CRITERION_RUNWAY_LANDING', 0.200, TRUE, 500, 3000,
   'FT; 500 preferred, 3,000 required upper scoring bound.'),
  ('PATROL_SURVEILLANCE', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 80, 350,
   'KNOTS; 80 minimum, 350 ideal.'),
  ('PATROL_SURVEILLANCE', 'CRITERION_RANGE', 0.300, TRUE, 500, 4000,
   'NM; 500 required minimum, 4,000 ideal.'),
  ('PATROL_SURVEILLANCE', 'CRITERION_CEILING', 0.150, FALSE, 10000, 40000,
   'FT; 10,000 minimum, 40,000 ideal.'),
  ('PATROL_SURVEILLANCE', 'CRITERION_FUEL_EFFICIENCY', 0.200, FALSE, 2, 20,
   'NM/gal; 2 minimum, 20 ideal.'),
  ('PATROL_SURVEILLANCE', 'CRITERION_PAYLOAD', 0.150, FALSE, 500, 10000,
   'LBS; 500 minimum, 10,000 ideal.'),
  ('PATROL_SURVEILLANCE', 'CRITERION_HOURLY_COST', 0.100, FALSE, 100, 5000,
   'USD/hr; 100 preferred, 5,000 upper scoring bound.'),
  ('HIGH_ALTITUDE_OPS', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 100, 300,
   'KNOTS; 100 minimum, 300 ideal.'),
  ('HIGH_ALTITUDE_OPS', 'CRITERION_RANGE', 0.150, FALSE, 300, 2000,
   'NM; 300 minimum, 2,000 ideal.'),
  ('HIGH_ALTITUDE_OPS', 'CRITERION_CEILING', 0.400, TRUE, 12000, 40000,
   'FT; 12,000 required minimum, 40,000 ideal.'),
  ('HIGH_ALTITUDE_OPS', 'CRITERION_CLIMB_RATE', 0.200, FALSE, 500, 3000,
   'FPM; 500 minimum, 3,000 ideal.'),
  ('HIGH_ALTITUDE_OPS', 'CRITERION_PAYLOAD', 0.100, FALSE, 300, 3000,
   'LBS; 300 minimum, 3,000 ideal.'),
  ('HIGH_ALTITUDE_OPS', 'CRITERION_RUNWAY_TAKEOFF', 0.050, FALSE, 1000, 5000,
   'FT; 1,000 preferred, 5,000 upper scoring bound.'),
  ('AEROBATICS', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 100, 300,
   'KNOTS; 100 minimum, 300 ideal.'),
  ('AEROBATICS', 'CRITERION_RANGE', 0.100, FALSE, 100, 800,
   'NM; 100 minimum, 800 ideal.'),
  ('AEROBATICS', 'CRITERION_CLIMB_RATE', 0.300, FALSE, 1000, 5000,
   'FPM; 1,000 minimum, 5,000 ideal.'),
  ('AEROBATICS', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 500, 3000,
   'FT; 500 preferred, 3,000 upper scoring bound.'),
  ('AEROBATICS', 'CRITERION_PRICE', 0.200, FALSE, 50000, 1000000,
   'USD; 50,000 preferred, 1,000,000 upper scoring bound.'),
  ('AEROBATICS', 'CRITERION_HOURLY_COST', 0.150, FALSE, 100, 1000,
   'USD/hr; 100 preferred, 1,000 upper scoring bound.'),
  ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 250, 700,
   'KNOTS; 250 minimum, 700 ideal.'),
  ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_RANGE', 0.200, FALSE, 300, 2000,
   'NM; 300 minimum, 2,000 ideal.'),
  ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_CLIMB_RATE', 0.150, FALSE, 3000, 15000,
   'FPM; 3,000 minimum, 15,000 ideal.'),
  ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_PAYLOAD', 0.300, FALSE, 2000, 20000,
   'LBS; 2,000 minimum, 20,000 ideal.'),
  ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 1500, 6000,
   'FT; 1,500 preferred, 6,000 upper scoring bound.'),
  ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_HOURLY_COST', 0.100, FALSE, 1000, 30000,
   'USD/hr; 1,000 preferred, 30,000 upper scoring bound.'),
  ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 200, 550,
   'KNOTS; 200 minimum, 550 ideal.'),
  ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RANGE', 0.250, TRUE, 1000, 6000,
   'NM; 1,000 required minimum, 6,000 ideal.'),
  ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_CEILING', 0.050, FALSE, 20000, 45000,
   'FT; 20,000 minimum, 45,000 ideal.'),
  ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_PAYLOAD', 0.350, TRUE, 10000, 200000,
   'LBS; 10,000 required minimum, 200,000 ideal.'),
  ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RUNWAY_TAKEOFF', 0.150, FALSE, 2000, 8000,
   'FT; 2,000 preferred, 8,000 upper scoring bound.'),
  ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RUNWAY_LANDING', 0.100, FALSE, 2000, 8000,
   'FT; 2,000 preferred, 8,000 upper scoring bound.'),
  ('MILITARY_MARITIME_PATROL', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 180, 500,
   'KNOTS; 180 minimum, 500 ideal.'),
  ('MILITARY_MARITIME_PATROL', 'CRITERION_RANGE', 0.350, TRUE, 1000, 6000,
   'NM; 1,000 required minimum, 6,000 ideal.'),
  ('MILITARY_MARITIME_PATROL', 'CRITERION_CEILING', 0.100, FALSE, 15000, 45000,
   'FT; 15,000 minimum, 45,000 ideal.'),
  ('MILITARY_MARITIME_PATROL', 'CRITERION_FUEL_EFFICIENCY', 0.150, FALSE, 0.5, 10,
   'NM/gal; 0.5 minimum, 10 ideal.'),
  ('MILITARY_MARITIME_PATROL', 'CRITERION_PAYLOAD', 0.200, FALSE, 2000, 30000,
   'LBS; 2,000 minimum, 30,000 ideal.'),
  ('MILITARY_MARITIME_PATROL', 'CRITERION_HOURLY_COST', 0.100, FALSE, 1000, 30000,
   'USD/hr; 1,000 preferred, 30,000 upper scoring bound.'),
  ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 80, 400,
   'KNOTS; 80 minimum, 400 ideal.'),
  ('UNMANNED_SPECIAL_MISSION', 'CRITERION_RANGE', 0.300, TRUE, 500, 6000,
   'NM; 500 required minimum, 6,000 ideal.'),
  ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CEILING', 0.250, TRUE, 20000, 60000,
   'FT; 20,000 required minimum, 60,000 ideal.'),
  ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CLIMB_RATE', 0.050, FALSE, 500, 5000,
   'FPM; 500 minimum, 5,000 ideal.'),
  ('UNMANNED_SPECIAL_MISSION', 'CRITERION_PAYLOAD', 0.200, FALSE, 100, 5000,
   'LBS; 100 minimum, 5,000 ideal.'),
  ('UNMANNED_SPECIAL_MISSION', 'CRITERION_HOURLY_COST', 0.100, FALSE, 50, 5000,
   'USD/hr; 50 preferred, 5,000 upper scoring bound.');

CREATE TEMP TABLE changed_seeded_mission_profiles ON COMMIT DROP AS
SELECT profile.id
FROM aircraft_compare.mission_profiles AS profile
WHERE EXISTS (
    (SELECT criterion.criterion_type_code, criterion.weight, criterion.is_required,
            criterion.scoring_lower_bound, criterion.scoring_upper_bound, criterion.notes
     FROM aircraft_compare.mission_criteria AS criterion
     WHERE criterion.mission_profile_id = profile.id
     EXCEPT
     SELECT desired.criterion_type_code, desired.weight, desired.is_required,
            desired.scoring_lower_bound, desired.scoring_upper_bound, desired.notes
     FROM desired_mission_criteria AS desired
     WHERE desired.profile_type_code = profile.profile_type_code)
    UNION ALL
    (SELECT desired.criterion_type_code, desired.weight, desired.is_required,
            desired.scoring_lower_bound, desired.scoring_upper_bound, desired.notes
     FROM desired_mission_criteria AS desired
     WHERE desired.profile_type_code = profile.profile_type_code
     EXCEPT
     SELECT criterion.criterion_type_code, criterion.weight, criterion.is_required,
            criterion.scoring_lower_bound, criterion.scoring_upper_bound, criterion.notes
     FROM aircraft_compare.mission_criteria AS criterion
     WHERE criterion.mission_profile_id = profile.id)
);

DELETE FROM aircraft_compare.variant_suitability AS suitability
USING changed_seeded_mission_profiles AS changed
WHERE suitability.mission_profile_id = changed.id;

DELETE FROM aircraft_compare.mission_criteria AS criterion
USING aircraft_compare.mission_profiles AS profile
WHERE criterion.mission_profile_id = profile.id
  AND EXISTS (
      SELECT 1 FROM desired_mission_criteria AS desired
      WHERE desired.profile_type_code = profile.profile_type_code
  )
  AND NOT EXISTS (
      SELECT 1 FROM desired_mission_criteria AS desired
      WHERE desired.profile_type_code = profile.profile_type_code
        AND desired.criterion_type_code = criterion.criterion_type_code
  );

INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code, weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
SELECT profile.id, desired.criterion_type_code, desired.weight, desired.is_required,
       desired.scoring_lower_bound, desired.scoring_upper_bound, desired.notes
FROM desired_mission_criteria AS desired
JOIN aircraft_compare.mission_profiles AS profile
  ON profile.profile_type_code = desired.profile_type_code
ON CONFLICT (mission_profile_id, criterion_type_code) DO UPDATE SET
    weight = EXCLUDED.weight,
    is_required = EXCLUDED.is_required,
    scoring_lower_bound = EXCLUDED.scoring_lower_bound,
    scoring_upper_bound = EXCLUDED.scoring_upper_bound,
    notes = EXCLUDED.notes;

COMMIT;
