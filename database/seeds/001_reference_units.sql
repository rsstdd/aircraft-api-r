-- =============================================================================
-- File: database/seeds/001_reference_units.sql
-- Phase 2 — seed data for aircraft_ref.unit_categories and
-- aircraft_ref.measurement_units. MUST be applied BEFORE
-- seeds/002_lookup_seed_data.sql because other
-- lookup tables (performance_metric_types, weight_metric_types,
-- dimension_metric_types, propulsion_categories) carry FKs into
-- measurement_units.
--
-- Unit-conversion conventions:
--   canonical_unit_code IS NULL  → this row is the canonical unit
--   canonical_factor            → raw_value × factor = canonical value
--
-- Temperature (DEG_C, DEG_F) intentionally have no canonical_factor because
-- absolute temperature conversion requires an additive offset (F→C: subtract
-- 32 then × 5/9), not a simple multiplicative factor. Both are treated as
-- their own canonical for their respective source strings; callers must handle
-- conversion explicitly. Temperature differences (ISA deviations) are linear
-- but that use-case is stored as source values without canonical conversion.
--
-- PPH (pounds per hour, fuel flow) has an approximate canonical_factor of
-- 0.1661129 (1/6.02 lbs per gallon, using avgas 100LL density).
-- The approximation introduces ~0.3% error for avgas. Jet-A users should
-- apply fuel_types.density_lbs_per_gal for accurate conversion.
-- =============================================================================

BEGIN;

-- unit_categories (15 rows)
-- -----------------------------------------------------------------------------
INSERT INTO aircraft_ref.unit_categories
(code, label, description, sort_order)
VALUES
    ('SPEED',          'Speed',
     'Airspeed and ground speed.',                                              10),
    ('MACH_NUMBER',    'Mach Number',
     'Dimensionless speed relative to local speed of sound.',                  15),
    ('ALTITUDE',       'Altitude / Elevation',
     'Height above a reference datum; also used for short distances.',          20),
    ('RANGE',          'Range',
     'Navigation distance (great-circle horizontal).',                          30),
    ('RUNWAY_DISTANCE','Runway Distance',
     'Takeoff and landing performance distances.',                              35),
    ('WEIGHT',         'Weight / Mass',
     'Structural weights, payload, and fuel mass.',                             40),
    ('VOLUME',         'Volume',
     'Fuel capacity and cargo volume.',                                         50),
    ('FLOW_RATE',      'Flow Rate',
     'Fuel and fluid consumption rates.',                                       55),
    ('POWER',          'Power',
     'Engine shaft power output.',                                              60),
    ('THRUST',         'Thrust',
     'Jet and rocket propulsive force.',                                        65),
    ('CLIMB_RATE',     'Climb Rate',
     'Vertical speed (positive = climbing).',                                   70),
    ('TIME_DURATION',  'Time / Duration',
     'Endurance, maintenance intervals, calendar periods.',                     75),
    ('AREA',           'Area',
     'Wing reference area and planform area.',                                  80),
    ('PRESSURE',       'Pressure',
     'Cabin differential, tyre, and manifold pressure.',                        90),
    ('TEMPERATURE',    'Temperature',
     'Ambient, operating, and certification temperature limits. '
         'No canonical cross-unit factor (offset conversion); '
         'both DEG_C and DEG_F are treated as independent canonicals.',            95)
    ON CONFLICT (code) DO NOTHING;

-- -----------------------------------------------------------------------------
-- measurement_units (~40 rows)
-- Canonical units (canonical_unit_code IS NULL) inserted first for clarity.
-- DEFERRABLE FK means order within the transaction is unconstrained.
-- -----------------------------------------------------------------------------

INSERT INTO aircraft_ref.measurement_units
(code, label, symbol, unit_category_code,
 canonical_unit_code, canonical_factor,
 si_factor, si_base_unit_symbol,
 source_string_patterns, sort_order)
VALUES

-- ── SPEED (canonical: KNOTS) ─────────────────────────────────────────────────
('KNOTS',  'Knots',                          'kt',    'SPEED',
 NULL,     NULL,           0.5144444,  'm/s',
 ARRAY['knots','kt','kts'],                              10),

('KIAS',   'Knots Indicated Airspeed',       'KIAS',  'SPEED',
 'KNOTS',  1.0,            0.5144444,  'm/s',
 ARRAY['kias'],                                          11),

('KTAS',   'Knots True Airspeed',            'KTAS',  'SPEED',
 'KNOTS',  1.0,            0.5144444,  'm/s',
 ARRAY['ktas'],                                          12),

('MPH',    'Miles Per Hour',                 'mph',   'SPEED',
 'KNOTS',  0.8689762,      0.4470400,  'm/s',
 ARRAY['mph','miles per hour'],                          20),

('KMH',    'Kilometres Per Hour',            'km/h',  'SPEED',
 'KNOTS',  0.5399568,      0.2777778,  'm/s',
 ARRAY['km/h','kmh','kph'],                              30),

-- ── MACH NUMBER (canonical: MACH) ────────────────────────────────────────────
('MACH',   'Mach Number',                    'M',     'MACH_NUMBER',
 NULL,     NULL,           NULL,       NULL,
 ARRAY['mach'],                                          10),

-- ── ALTITUDE / SHORT DISTANCE (canonical: FT) ────────────────────────────────
-- FT is also used for RUNWAY_DISTANCE metrics; performance_metric_types
-- points canonical_unit_code to 'FT' for all distance-class metrics.
('FT',     'Feet',                           'ft',    'ALTITUDE',
 NULL,     NULL,           0.3048000,  'm',
 ARRAY['ft','feet'],                                     10),

('METERS', 'Metres (altitude)',              'm',     'ALTITUDE',
 'FT',     3.2808399,      1.0,        'm',
 ARRAY['meters','metres'],                               20),

-- ── RANGE (canonical: NM) ────────────────────────────────────────────────────
('NM',     'Nautical Miles',                 'nm',    'RANGE',
 NULL,     NULL,           1852.0,     'm',
 ARRAY['nm','nmi'],                                      10),

('KM',     'Kilometres (range)',             'km',    'RANGE',
 'NM',     0.5399568,      1000.0,     'm',
 ARRAY['km','kilometers','kilometres'],                   20),

('MI',     'Statute Miles (range)',          'mi',    'RANGE',
 'NM',     0.8689762,      1609.344,   'm',
 ARRAY['mi','miles','statute miles'],                     30),

-- ── WEIGHT (canonical: LBS) ──────────────────────────────────────────────────
('LBS',    'Pounds',                         'lbs',   'WEIGHT',
 NULL,     NULL,           0.4535924,  'kg',
 ARRAY['lbs','lb','pounds'],                             10),

('KG',     'Kilograms',                      'kg',    'WEIGHT',
 'LBS',    2.2046226,      1.0,        'kg',
 ARRAY['kg','kgs','kilograms'],                          20),

-- ── VOLUME (canonical: US_GAL) ───────────────────────────────────────────────
('US_GAL', 'US Gallons',                     'gal',   'VOLUME',
 NULL,     NULL,           3.7854118,  'L',
 ARRAY['gal','gallons','us gal','usg'],                  10),

('LITERS', 'Litres',                         'L',     'VOLUME',
 'US_GAL', 0.2641721,      1.0,        'L',
 ARRAY['l','liters','litres'],                           20),

('IMP_GAL','Imperial Gallons',               'imp gal','VOLUME',
 'US_GAL', 1.2009499,      4.5460900,  'L',
 ARRAY['imp gal','imperial gallons'],                    30),

('CU_FT',  'Cubic Feet (volume)',            'cu ft', 'VOLUME',
 'US_GAL', 7.4805195,      28.316847,  'L',
 ARRAY['cu ft','cubic feet','ft³'],                      40),

-- ── FLOW RATE (canonical: GPH) ───────────────────────────────────────────────
('GPH',    'US Gallons Per Hour',            'GPH',   'FLOW_RATE',
 NULL,     NULL,           3.7854118,  'L/hr',
 ARRAY['gph'],                                           10),

('LPH',    'Litres Per Hour',                'LPH',   'FLOW_RATE',
 'GPH',    0.2641721,      1.0,        'L/hr',
 ARRAY['lph'],                                           20),

-- PPH: approximate using avgas 100LL density ≈ 6.02 lbs/gal → 1/6.02 ≈ 0.1661
-- For Jet-A (≈6.7 lbs/gal) use fuel_types.density_lbs_per_gal instead.
('PPH',    'Pounds Per Hour (fuel flow)',     'PPH',   'FLOW_RATE',
 'GPH',    0.1661129,      NULL,       NULL,
 ARRAY['pph'],                                           30),

-- ── POWER (canonical: HP) ────────────────────────────────────────────────────
('HP',     'Horsepower',                     'hp',    'POWER',
 NULL,     NULL,           745.69987,  'W',
 ARRAY['hp'],                                            10),

('SHP',    'Shaft Horsepower (turboprop)',    'shp',   'POWER',
 'HP',     1.0,            745.69987,  'W',
 ARRAY['shp'],                                           11),

('ESHP',   'Equivalent Shaft Horsepower',    'eshp',  'POWER',
 'HP',     1.0,            745.69987,  'W',
 ARRAY['eshp'],                                          12),

('KW',     'Kilowatts',                      'kW',    'POWER',
 'HP',     1.3410197,      1000.0,     'W',
 ARRAY['kw','kilowatts'],                                20),

-- ── THRUST (canonical: LBF) ──────────────────────────────────────────────────
('LBF',    'Pounds-Force (thrust)',           'lbf',   'THRUST',
 NULL,     NULL,           4.4482216,  'N',
 ARRAY['lbf','lb-f','pounds thrust','lbs thrust'],        10),

-- Source pattern 'n' is ambiguous (also used for knots in some texts).
-- Phase 17 ingestion resolves by context (performance.thrust key ⇒ NEWTONS).
('NEWTONS','Newtons',                        'N',     'THRUST',
 'LBF',    0.2248089,      1.0,        'N',
 ARRAY['n','newtons'],                                   20),

('KN',     'Kilonewtons',                    'kN',    'THRUST',
 'LBF',    224.8089431,    1000.0,     'N',
 ARRAY['kn','kilonewtons'],                              30),

-- ── CLIMB RATE (canonical: FPM) ──────────────────────────────────────────────
('FPM',    'Feet Per Minute',                'fpm',   'CLIMB_RATE',
 NULL,     NULL,           0.00508,    'm/s',
 ARRAY['fpm','ft/min'],                                  10),

('MPS',    'Metres Per Second (climb)',       'm/s',   'CLIMB_RATE',
 'FPM',    196.8503937,    1.0,        'm/s',
 ARRAY['m/s'],                                           20),

-- ── TIME / DURATION (canonical: HRS) ─────────────────────────────────────────
('HRS',    'Hours',                          'hrs',   'TIME_DURATION',
 NULL,     NULL,           3600.0,     's',
 ARRAY['hrs','hours','hr','h'],                          10),

('MINUTES','Minutes',                        'min',   'TIME_DURATION',
 'HRS',    0.0166667,      60.0,       's',
 ARRAY['min','minutes'],                                 20),

-- ── AREA (canonical: SQ_FT) ──────────────────────────────────────────────────
('SQ_FT',  'Square Feet',                    'sq ft', 'AREA',
 NULL,     NULL,           0.0929030,  'm²',
 ARRAY['sq ft','sqft','ft²'],                            10),

('SQ_M',   'Square Metres',                  'm²',    'AREA',
 'SQ_FT',  10.7639104,     1.0,        'm²',
 ARRAY['sq m','sqm','m²'],                               20),

-- ── PRESSURE (canonical: PSI) ─────────────────────────────────────────────────
('PSI',    'Pounds Per Square Inch',         'psi',   'PRESSURE',
 NULL,     NULL,           6894.757,   'Pa',
 ARRAY['psi'],                                           10),

('INHG',   'Inches of Mercury',             'inHg',  'PRESSURE',
 'PSI',    0.4911542,      3386.389,   'Pa',
 ARRAY['inhg','in hg','"hg'],                            20),

('HPA',    'Hectopascals / Millibars',       'hPa',   'PRESSURE',
 'PSI',    0.0145038,      100.0,      'Pa',
 ARRAY['hpa','mb','mbar'],                               30),

-- ── TEMPERATURE ──────────────────────────────────────────────────────────────
-- Absolute temperature conversion (°F → °C) requires an additive offset
-- (subtract 32, then × 5/9) so canonical_factor is not applicable.
-- Both DEG_C and DEG_F are canonical for their own source strings.
-- Temperature differences (ISA deviations) are linear but stored as raw
-- values with their source unit; callers convert explicitly if required.
('DEG_C',  'Degrees Celsius',               '°C',    'TEMPERATURE',
 NULL,     NULL,           1.0,        '°C',
 ARRAY['c','celsius','deg c','°c'],                      10),

('DEG_F',  'Degrees Fahrenheit',            '°F',    'TEMPERATURE',
 NULL,     NULL,           NULL,       NULL,
 ARRAY['f','fahrenheit','deg f','°f'],                   20)

    ON CONFLICT (code) DO NOTHING;

COMMIT;