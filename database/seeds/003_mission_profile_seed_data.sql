-- =============================================================================
-- File: database/seeds/003_mission_profile_seed_data.sql
-- Phase 15 — seed data for aircraft_compare.mission_profiles and
-- aircraft_compare.mission_criteria.
--
-- 15 mission_profiles rows (one per aircraft_ref.mission_profile_types seed).
-- Full mission_criteria with weights for 5 representative profiles:
--   PERSONAL_VFR_TOURING, IFR_CROSSCOUNTRY, BACKCOUNTRY_STOL,
--   BUSINESS_TRAVEL, FLIGHT_TRAINING.
-- Stub criteria (weight only, no bounds) for the remaining 10 profiles.
--
-- Criterion weights SUM to 1.000 per profile.
-- Scoring bounds (scoring_lower_bound / scoring_upper_bound) are in the
-- canonical units for that criterion's metric:
--   speeds → KNOTS, range → NM, ceiling/runway → FT, payload → LBS,
--   price/cost → USD (ordering varies by direction of is_higher_better).
-- =============================================================================

BEGIN;

-- =============================================================================
-- PART 1: mission_profiles (15 rows)
-- =============================================================================

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
   'Takeoff/landing distance is the primary discriminator.',
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
   'Operations at density altitudes above 10,000 ft PA. '
   'Service ceiling, turbocharged or turbo-normalised engine required.',
   300, 2, 12000, TRUE, FALSE, 85),

  ('AEROBATICS',
   'aerobatics', 'Aerobatics / Air Show',
   'Intentional aerobatic manoeuvres and air show display flying. '
   'Structural load factor approval, roll rate, and power loading are key.',
   100, 1, 4000, TRUE, FALSE, 90),

  ('MILITARY_CLOSE_AIR_SUPPORT',
   'military-cas', 'Military: Close Air Support',
   'Public reference comparison for CAS-capable aircraft. '
   'External stores capacity, survivability features, speed, and range. '
   'Encyclopedia comparison only; no operational/tactical content.',
   400, 1, 20000, FALSE, TRUE, 100),

  ('MILITARY_TRANSPORT_AIRLIFT',
   'military-transport', 'Military: Transport / Airlift',
   'Strategic and tactical airlift comparison. '
   'Payload, range, short-field capability (tactical), and cargo door.',
   2000, 0, 28000, FALSE, TRUE, 110),

  ('MILITARY_MARITIME_PATROL',
   'military-maritime-patrol', 'Military: Maritime Patrol',
   'Maritime patrol and anti-submarine warfare comparison. '
   'Endurance, mission radius, and sensor carriage are primary.',
   1000, 4, 15000, FALSE, TRUE, 120),

  ('UNMANNED_SPECIAL_MISSION',
   'unmanned-special-mission', 'Unmanned / Special Mission',
   'Comparison of UAV and special-mission platforms. '
   'Endurance, payload, ceiling, and datalink range are key.',
   800, 0, 30000, TRUE, TRUE, 130)

ON CONFLICT (profile_type_code) DO NOTHING;

-- =============================================================================
-- PART 2: mission_criteria — 5 fully-configured profiles
-- Criterion codes reference aircraft_ref.comparison_criterion_types.
-- Bounds are in canonical units (KNOTS, NM, FT, LBS, USD).
-- =============================================================================

-- Helper: look up mission_profile id by slug
-- All INSERTs use subselects for portability (no hardcoded IDs).

-- ----------------------------------------------------------------------------
-- PERSONAL_VFR_TOURING criteria (weights sum to 1.000)
-- ----------------------------------------------------------------------------
INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code,
     weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
SELECT mp.id, crit.code, crit.w, crit.req, crit.lb, crit.ub, crit.n
FROM aircraft_compare.mission_profiles mp
CROSS JOIN (VALUES
    ('CRITERION_PRICE',         0.300::numeric(4,3), FALSE,  20000::numeric,  200000::numeric, 'Lower is better; typical GA singles $40k-$180k.'),
    ('CRITERION_HOURLY_COST',   0.200, FALSE,  30,       150,    'Lower is better; typical $50-$120/hr.'),
    ('CRITERION_CRUISE_SPEED',  0.200, FALSE,  80,       150,    'Knots; 80 minimum, 150+ ideal.'),
    ('CRITERION_RANGE',         0.150, FALSE, 200,       800,    'NM; 200 minimum, 800+ ideal.'),
    ('CRITERION_PAX_SEATS',     0.100, FALSE,   1,         4,    'Seats including pilot; 1 min, 4+ ideal.'),
    ('CRITERION_FUEL_EFFICIENCY',0.050,FALSE,   NULL,     NULL,  'NM/gal; computed criterion, no hard bounds.')
) crit(code, w, req, lb, ub, n)
WHERE mp.slug = 'personal-vfr-touring'
ON CONFLICT (mission_profile_id, criterion_type_code) DO NOTHING;

-- ----------------------------------------------------------------------------
-- IFR_CROSSCOUNTRY criteria (weights sum to 1.000)
-- ----------------------------------------------------------------------------
INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code,
     weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
SELECT mp.id, crit.code, crit.w, crit.req, crit.lb, crit.ub, crit.n
FROM aircraft_compare.mission_profiles mp
CROSS JOIN (VALUES
    ('CRITERION_RANGE',          0.300::numeric(4,3), TRUE,  400::numeric, 1500::numeric, 'NM; 400 required minimum, 1500+ ideal.'),
    ('CRITERION_CRUISE_SPEED',   0.200, TRUE,  100,  220,   'Knots; 100 required minimum, 220+ ideal.'),
    ('CRITERION_CEILING',        0.150, TRUE, 10000, 25000, 'FT; 10,000 required, 25,000+ ideal for IFR.'),
    ('CRITERION_FUEL_EFFICIENCY',0.150, FALSE,  NULL, NULL, 'NM/gal; no hard bounds.'),
    ('CRITERION_PAX_SEATS',      0.100, FALSE,    1,    4,  'Seats.'),
    ('CRITERION_PRICE',          0.100, FALSE, NULL,  NULL, 'USD; informational weighting only.')
) crit(code, w, req, lb, ub, n)
WHERE mp.slug = 'ifr-crosscountry'
ON CONFLICT (mission_profile_id, criterion_type_code) DO NOTHING;

-- ----------------------------------------------------------------------------
-- BACKCOUNTRY_STOL criteria (weights sum to 1.000)
-- ----------------------------------------------------------------------------
INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code,
     weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
SELECT mp.id, crit.code, crit.w, crit.req, crit.lb, crit.ub, crit.n
FROM aircraft_compare.mission_profiles mp
CROSS JOIN (VALUES
    ('CRITERION_RUNWAY_TAKEOFF', 0.350::numeric(4,3), TRUE,  500::numeric, 2500::numeric,  'FT; lower is better. <=500 = ideal (score 1), 2500+ = unsuitable (score 0).'),
    ('CRITERION_RUNWAY_LANDING', 0.350, TRUE,  500, 2500,   'FT; lower is better (<=500 ideal, 2500+ unsuitable).'),
    ('CRITERION_PAYLOAD',        0.200, FALSE,  300, 1500,  'LBS useful payload.'),
    ('CRITERION_CRUISE_SPEED',   0.070, FALSE,   60,  130,  'Knots; not primary driver.'),
    ('CRITERION_PRICE',          0.030, FALSE, NULL, NULL,  'USD; minor factor.')
) crit(code, w, req, lb, ub, n)
WHERE mp.slug = 'backcountry-stol'
ON CONFLICT (mission_profile_id, criterion_type_code) DO NOTHING;

-- ----------------------------------------------------------------------------
-- BUSINESS_TRAVEL criteria (weights sum to 1.000)
-- ----------------------------------------------------------------------------
INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code,
     weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
SELECT mp.id, crit.code, crit.w, crit.req, crit.lb, crit.ub, crit.n
FROM aircraft_compare.mission_profiles mp
CROSS JOIN (VALUES
    ('CRITERION_CRUISE_SPEED',   0.250::numeric(4,3), FALSE, 150::numeric, 400::numeric,  'Knots; 150 minimum viable, 400+ ideal.'),
    ('CRITERION_RANGE',          0.250, FALSE,  500, 2500,  'NM; 500 minimum, 2500+ ideal.'),
    ('CRITERION_PAX_SEATS',      0.200, FALSE,    2,    8,  'Seats; 2 minimum, 8+ ideal.'),
    ('CRITERION_HOURLY_COST',    0.150, FALSE, NULL, NULL,  'USD/hr; lower is better.'),
    ('CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 2000, 5000,  'FT; lower is better (runway access; 2000 ideal, 5000+ poor).'),
    ('CRITERION_CEILING',        0.050, FALSE, 20000, 45000,'FT; 20,000+ desirable.')
) crit(code, w, req, lb, ub, n)
WHERE mp.slug = 'business-travel'
ON CONFLICT (mission_profile_id, criterion_type_code) DO NOTHING;

-- ----------------------------------------------------------------------------
-- FLIGHT_TRAINING criteria (weights sum to 1.000)
-- ----------------------------------------------------------------------------
INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code,
     weight, is_required,
     scoring_lower_bound, scoring_upper_bound, notes)
SELECT mp.id, crit.code, crit.w, crit.req, crit.lb, crit.ub, crit.n
FROM aircraft_compare.mission_profiles mp
CROSS JOIN (VALUES
    ('CRITERION_PRICE',          0.300::numeric(4,3), FALSE, 10000::numeric, 120000::numeric, 'USD; lower is better; accessible training aircraft.'),
    ('CRITERION_HOURLY_COST',    0.300, FALSE, NULL, NULL,  'USD/hr; lower is better for utilisation.'),
    ('CRITERION_RUNWAY_LANDING', 0.200, FALSE, 800,  3000, 'FT; shorter landing roll = more forgiving (800 ideal, 3000+ poor).'),
    ('CRITERION_CRUISE_SPEED',   0.100, FALSE,   60,  130,  'Knots; slow enough for training.'),
    ('CRITERION_PAX_SEATS',      0.100, FALSE,    1,    2,  'Seats; side-by-side or tandem 2-seat preferred.')
) crit(code, w, req, lb, ub, n)
WHERE mp.slug = 'flight-training'
ON CONFLICT (mission_profile_id, criterion_type_code) DO NOTHING;

-- =============================================================================
-- PART 3: Stub criteria for remaining 10 profiles
-- Minimal (single criterion) stubs so the profiles have at least one criterion
-- and the scoring engine can function. Curators expand these.
-- =============================================================================

INSERT INTO aircraft_compare.mission_criteria
    (mission_profile_id, criterion_type_code, weight, is_required, notes)
SELECT mp.id, crit.code, crit.w, FALSE, crit.n
FROM aircraft_compare.mission_profiles mp
CROSS JOIN (VALUES ('CRITERION_RANGE', 1.000::numeric(4,3), 'Stub: primary criterion. Expand with full criteria set.')) crit(code, w, n)
WHERE mp.slug IN (
    'floatplane-operations', 'cargo-freight', 'medevac-sar',
    'patrol-surveillance', 'high-altitude-ops', 'aerobatics',
    'military-cas', 'military-transport',
    'military-maritime-patrol', 'unmanned-special-mission'
)
ON CONFLICT (mission_profile_id, criterion_type_code) DO NOTHING;

COMMIT;