-- =============================================================================
-- File: database/migrations/011_military_sensors_stores_loadouts.sql
-- Phase 11 — aircraft_military: public, unclassified reference data for
-- hardpoints, compatible stores, weapons/stores catalog, representative
-- loadout examples, mission capabilities, and military sensor links.
--
-- *** SCOPE CONSTRAINT ***
-- This schema stores PUBLIC, UNCLASSIFIED, encyclopedia-style reference data
-- ONLY. It explicitly excludes:
--   - Classified performance data (CEP, seeker range, fuzing parameters)
--   - Operational employment guidance or targeting recommendations
--   - Mission-planning or tactical data of any kind
--   - Classified system specifications
-- Sources: Jane's All the World's Aircraft, official manufacturer brochures,
-- publicly released government fact sheets, and Wikipedia (cross-checked).
-- All data here is equivalent to what a reference encyclopedia would publish.
--
-- Four layers of data are clearly separated:
--   1. hardpoints          — structural capability (what stations exist)
--   2. hardpoint_compatibilities — what store TYPES are certified for each station
--   3. weapons_catalog     — public reference catalogue of weapons and stores
--   4. representative_loadouts + loadout_items — example configurations from
--                           public sources (airshow, press, manufacturer brochures)
--
-- Spec coverage (requirement 8):
--   hardpoints, internal bays, station limits → hardpoints
--   max external stores, compatible stores    → hardpoint_compatibilities
--   representative weapons, sensor pods       → weapons_catalog
--   external fuel tanks                       → weapons_catalog (EXT_FUEL_TANK type)
--   radar, EO/IR, EW, datalink               → variant_sensors → equipment_catalog
--   representative loadout examples           → representative_loadouts + loadout_items
--   mission capabilities                      → variant_mission_capabilities
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_military.hardpoints
-- Individual weapons stations or hardpoints on a variant.
-- station_number: manufacturer/military designator (e.g., 'STA 1', 'BL219').
-- position_type_code: broad positional category (WING_INBOARD, etc.)
-- max_load_lbs: maximum allowable store weight at this station.
-- is_wet: TRUE if the station has fuel plumbing for external tanks.
-- UNIQUE (variant_id, station_number) — one station label per variant.
-- =============================================================================

CREATE TABLE aircraft_military.hardpoints (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id          BIGINT NOT NULL
                            REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    station_number      TEXT NOT NULL,  -- e.g., 'STA 1', 'Station 3', 'BL219', 'L-1'
    station_label       TEXT,           -- human-readable label, e.g., 'Left Wing Inboard'
    position_type_code  aircraft_ref.lookup_code NOT NULL
                            REFERENCES aircraft_ref.hardpoint_position_types(code),
    -- Maximum certified store weight in lbs at this station.
    max_load_lbs        NUMERIC,
    -- Maximum ejector/pylon capacity (structural limit, may differ from certified).
    ejector_capacity_lbs NUMERIC,
    -- TRUE if station has fuel system plumbing for external tanks.
    is_wet              BOOLEAN NOT NULL DEFAULT FALSE,
    -- TRUE for internal weapons bay stations.
    is_internal_bay     BOOLEAN NOT NULL DEFAULT FALSE,
    notes               TEXT,
    UNIQUE (variant_id, station_number),
    CONSTRAINT chk_hp_loads CHECK (
        (max_load_lbs       IS NULL OR max_load_lbs       >= 0)
        AND (ejector_capacity_lbs IS NULL OR ejector_capacity_lbs >= 0)
    )
);

COMMENT ON TABLE aircraft_military.hardpoints IS
    'Individual weapons stations on a military variant. '
    'Represents structural capability only (what station exists and its limits). '
    'What can be carried is in hardpoint_compatibilities; '
    'what IS carried in a specific example is in loadout_items.';
COMMENT ON COLUMN aircraft_military.hardpoints.is_wet IS
    'TRUE when the station has fuel system plumbing enabling external fuel tanks. '
    'Dry stations can carry weapons and sensor pods but not fuel tanks.';

-- =============================================================================
-- aircraft_military.weapons_catalog
-- Public encyclopedia reference for weapons, stores, sensor pods, and
-- external fuel tanks. Physical properties only — no performance data.
-- stores_type_code: links to aircraft_ref.stores_types (which carries
--   weapon_category_code as a parent FK for category-level grouping).
-- =============================================================================

CREATE TABLE aircraft_military.weapons_catalog (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug                  aircraft_ref.slug_text NOT NULL UNIQUE,
    name                  TEXT NOT NULL,         -- "AIM-9X Sidewinder"
    common_name           TEXT,                  -- "Sidewinder"
    designation           TEXT,                  -- "AIM-9X", "GBU-12", "SUU-23"
    stores_type_code      aircraft_ref.lookup_code
                              REFERENCES aircraft_ref.stores_types(code),
    manufacturer_org_id   BIGINT
                              REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    manufacturer_name_raw TEXT,
    country_of_origin_code VARCHAR(3)
                              REFERENCES aircraft_geo.countries(code),
    -- Physical reference data (public unclassified)
    weight_lbs            NUMERIC,               -- total weight including warhead/fuel
    length_in             NUMERIC,               -- overall length in inches
    diameter_in           NUMERIC,               -- body diameter in inches
    -- Encyclopedia reference only; no performance parameters.
    description           TEXT,
    -- Link to public reference source (Wikipedia, manufacturer public page, etc.)
    reference_url         TEXT,
    is_active             BOOLEAN NOT NULL DEFAULT TRUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_wc_dims CHECK (
        (weight_lbs  IS NULL OR weight_lbs  >= 0)
        AND (length_in  IS NULL OR length_in  > 0)
        AND (diameter_in IS NULL OR diameter_in > 0)
    )
);

COMMENT ON TABLE aircraft_military.weapons_catalog IS
    'Public encyclopedia reference for weapons, stores, sensor pods, and '
    'external fuel tanks. Physical dimensions and mass only — '
    'no classified performance parameters (CEP, seeker range, fuzing details). '
    'Equivalent to what Jane''s or a public manufacturer brochure would publish.';
COMMENT ON COLUMN aircraft_military.weapons_catalog.description IS
    'Brief public description of the store. Must not contain classified '
    'performance data, targeting parameters, or employment guidance.';

-- =============================================================================
-- aircraft_military.hardpoint_compatibilities
-- Which store TYPES (broad categories) are documented as compatible with
-- each hardpoint, based on public sources (TCDS, manufacturer data sheets,
-- operator handbooks where publicly available).
-- Note: compatibility at stores_type level (e.g., LGB, AAM_SHORT_RANGE),
-- not individual weapon level. This reflects public certification data.
-- =============================================================================

CREATE TABLE aircraft_military.hardpoint_compatibilities (
    hardpoint_id      BIGINT NOT NULL
                          REFERENCES aircraft_military.hardpoints(id)   ON DELETE CASCADE,
    stores_type_code  aircraft_ref.lookup_code NOT NULL
                          REFERENCES aircraft_ref.stores_types(code),
    -- Maximum weight of this stores type at this hardpoint (may be < max_load_lbs).
    max_stores_weight_lbs NUMERIC,
    notes             TEXT,
    PRIMARY KEY (hardpoint_id, stores_type_code)
);

COMMENT ON TABLE aircraft_military.hardpoint_compatibilities IS
    'Public documented store TYPE compatibility per hardpoint. '
    'Records what categories (LGB, AAM_SHORT_RANGE, EXT_FUEL_TANK) are '
    'cleared for each station, based on public sources. '
    'For specific weapon compatibility, use representative_loadouts.';

-- =============================================================================
-- aircraft_military.representative_loadouts
-- Named reference loadout configurations from public sources.
-- These are EXAMPLES published in reference materials — not prescriptive
-- tactical configurations. source_notes MUST cite the public source.
-- =============================================================================

CREATE TABLE aircraft_military.representative_loadouts (
    id                      BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id              BIGINT NOT NULL
                                REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    name                    TEXT NOT NULL,  -- "Air Superiority (2×AIM-120C + 2×AIM-9X)"
    mission_type_code       aircraft_ref.lookup_code
                                REFERENCES aircraft_ref.military_mission_types(code),
    -- Reference total stores weight (informational, from public source).
    total_stores_weight_lbs NUMERIC,
    -- Approximate fuel state at time of loadout definition.
    fuel_state_pct          NUMERIC,
    -- Description must not contain tactical/operational content.
    description             TEXT,
    -- REQUIRED: citation for this loadout (Jane's issue, press release, etc.)
    source_notes            TEXT,
    confidence              aircraft_ref.confidence_score,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_rl_fuel_pct CHECK (
        fuel_state_pct IS NULL
        OR (fuel_state_pct >= 0 AND fuel_state_pct <= 100)
    ),
    CONSTRAINT chk_rl_weight_nonneg CHECK (
        total_stores_weight_lbs IS NULL OR total_stores_weight_lbs >= 0
    )
);

COMMENT ON TABLE aircraft_military.representative_loadouts IS
    'Named reference loadout configurations from public sources only. '
    'These are examples from Jane''s, manufacturer brochures, or airshow displays — '
    'NOT prescriptive tactical configurations. '
    'source_notes MUST cite the public reference for each loadout row.';
COMMENT ON COLUMN aircraft_military.representative_loadouts.source_notes IS
    'REQUIRED citation: public source for this loadout example '
    '(e.g., "Jane''s All the World''s Aircraft 2023 p.412", '
    '"Lockheed Martin F-35 fact sheet rev 2022", "USAF photo caption 2019").';

-- =============================================================================
-- aircraft_military.loadout_items
-- Individual stores on hardpoints within a representative loadout.
-- weapon_id: specific weapon from weapons_catalog (preferred).
-- stores_type_code: fallback when specific weapon is not catalogued.
-- At least one of weapon_id or stores_type_code must be non-NULL.
-- PRIMARY KEY (loadout_id, hardpoint_id) — one store per station per loadout.
-- quantity > 1 models multiple weapons on a single ejector rack/MER.
-- =============================================================================

CREATE TABLE aircraft_military.loadout_items (
    loadout_id       BIGINT NOT NULL
                         REFERENCES aircraft_military.representative_loadouts(id) ON DELETE CASCADE,
    hardpoint_id     BIGINT NOT NULL
                         REFERENCES aircraft_military.hardpoints(id)             ON DELETE RESTRICT,
    weapon_id        BIGINT
                         REFERENCES aircraft_military.weapons_catalog(id)        ON DELETE SET NULL,
    stores_type_code aircraft_ref.lookup_code
                         REFERENCES aircraft_ref.stores_types(code),
    -- Quantity on this hardpoint (e.g., 3 for a triple ejector rack).
    quantity         SMALLINT NOT NULL DEFAULT 1,
    notes            TEXT,
    PRIMARY KEY (loadout_id, hardpoint_id),
    CONSTRAINT chk_li_has_store CHECK (
        weapon_id IS NOT NULL OR stores_type_code IS NOT NULL
    ),
    CONSTRAINT chk_li_quantity CHECK (quantity >= 1)
);

COMMENT ON TABLE aircraft_military.loadout_items IS
    'Per-hardpoint store assignments within a representative_loadouts record. '
    'weapon_id references a specific weapons_catalog entry; '
    'stores_type_code is a fallback category when the specific weapon is not catalogued. '
    'quantity > 1 models multiple weapons on a single ejector rack (MER/TER).';

-- =============================================================================
-- aircraft_military.variant_mission_capabilities
-- Missions a variant is publicly documented to be capable of.
-- is_primary_mission: the variant''s primary designed role.
-- Partial UNIQUE index enforces at most one primary mission per variant.
-- =============================================================================

CREATE TABLE aircraft_military.variant_mission_capabilities (
    variant_id         BIGINT NOT NULL
                           REFERENCES aircraft_core.variants(id)            ON DELETE CASCADE,
    mission_type_code  aircraft_ref.lookup_code NOT NULL
                           REFERENCES aircraft_ref.military_mission_types(code),
    is_primary_mission BOOLEAN NOT NULL DEFAULT FALSE,
    confidence         aircraft_ref.confidence_score,
    notes              TEXT,
    PRIMARY KEY (variant_id, mission_type_code)
);

COMMENT ON TABLE aircraft_military.variant_mission_capabilities IS
    'Mission types a variant is publicly documented to support. '
    'is_primary_mission marks the dominant designed role. '
    'All entries must be supported by public sources; '
    'use notes to cite the reference.';

-- =============================================================================
-- aircraft_military.variant_sensors
-- Military sensors installed on or available for a variant.
-- Links to aircraft_systems.equipment_catalog for the sensor definition.
-- equipment_id: null if sensor is known by name but not in equipment catalog.
-- sensor_name_raw: fallback name when equipment_id is NULL.
-- is_internal: TRUE for built-in systems; FALSE for podded systems.
-- Separated from aircraft_systems.variant_equipment because military
-- sensor data requires additional military-specific context and namespace
-- separation per the spec's "clearly separate" design requirement.
-- =============================================================================

CREATE TABLE aircraft_military.variant_sensors (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id            BIGINT NOT NULL
                              REFERENCES aircraft_core.variants(id)              ON DELETE CASCADE,
    -- Reference to the civilian/military equipment catalog (cross-schema FK).
    equipment_id          BIGINT
                              REFERENCES aircraft_systems.equipment_catalog(id)  ON DELETE SET NULL,
    -- Free-text fallback when sensor is not in equipment_catalog.
    sensor_name_raw       TEXT,
    provision_type_code   aircraft_ref.lookup_code
                              REFERENCES aircraft_ref.equipment_provision_types(code),
    -- TRUE for built-in systems (internal radar, internal EW suite).
    -- FALSE for podded systems (targeting pod, recon pod, ECM pod).
    is_internal           BOOLEAN NOT NULL DEFAULT TRUE,
    confidence            aircraft_ref.confidence_score,
    notes                 TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_vs_has_sensor CHECK (
        equipment_id IS NOT NULL OR sensor_name_raw IS NOT NULL
    )
);

COMMENT ON TABLE aircraft_military.variant_sensors IS
    'Military sensors per variant: radar, EO/IR, EW, and datalink systems. '
    'Separated from aircraft_systems.variant_equipment for namespace clarity '
    'and additional military-context columns (is_internal). '
    'equipment_id links to aircraft_systems.equipment_catalog for systems '
    'that have catalog entries; sensor_name_raw covers systems not yet catalogued. '
    'All entries are public, unclassified reference data only.';
COMMENT ON COLUMN aircraft_military.variant_sensors.is_internal IS
    'TRUE = built-in system (APG-83 radar, internal EW suite). '
    'FALSE = podded/external system (Sniper ATP, ALQ-184). '
    'Pods may also appear as loadout_items on a hardpoint.';

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_military.hardpoints ─────────────────────────────────────────────
CREATE INDEX idx_hp_variant
    ON aircraft_military.hardpoints (variant_id, position_type_code);
CREATE INDEX idx_hp_position
    ON aircraft_military.hardpoints (position_type_code);
CREATE INDEX idx_hp_wet
    ON aircraft_military.hardpoints (variant_id)
    WHERE is_wet;

-- ── aircraft_military.weapons_catalog ────────────────────────────────────────
CREATE INDEX idx_wc_stores_type
    ON aircraft_military.weapons_catalog (stores_type_code);
CREATE INDEX idx_wc_country
    ON aircraft_military.weapons_catalog (country_of_origin_code);
CREATE INDEX idx_wc_name_trgm
    ON aircraft_military.weapons_catalog USING gin (name gin_trgm_ops);
CREATE INDEX idx_wc_designation_trgm
    ON aircraft_military.weapons_catalog USING gin (designation gin_trgm_ops)
    WHERE designation IS NOT NULL;

-- ── aircraft_military.hardpoint_compatibilities ───────────────────────────────
-- "Which hardpoints can carry a given stores type?"
CREATE INDEX idx_hc_stores_type
    ON aircraft_military.hardpoint_compatibilities (stores_type_code);

-- ── aircraft_military.representative_loadouts ────────────────────────────────
CREATE INDEX idx_rl_variant
    ON aircraft_military.representative_loadouts (variant_id);
CREATE INDEX idx_rl_mission
    ON aircraft_military.representative_loadouts (mission_type_code)
    WHERE mission_type_code IS NOT NULL;

-- ── aircraft_military.loadout_items ──────────────────────────────────────────
CREATE INDEX idx_li_weapon
    ON aircraft_military.loadout_items (weapon_id)
    WHERE weapon_id IS NOT NULL;

-- ── aircraft_military.variant_mission_capabilities ───────────────────────────
-- At most one primary mission per variant.
CREATE UNIQUE INDEX uq_vmc_primary
    ON aircraft_military.variant_mission_capabilities (variant_id)
    WHERE is_primary_mission;
-- "All variants capable of mission X"
CREATE INDEX idx_vmc_mission
    ON aircraft_military.variant_mission_capabilities (mission_type_code);

-- ── aircraft_military.variant_sensors ────────────────────────────────────────
-- Prevent duplicate (variant, equipment) links when equipment_id is known.
CREATE UNIQUE INDEX uq_vs_variant_equipment
    ON aircraft_military.variant_sensors (variant_id, equipment_id)
    WHERE equipment_id IS NOT NULL;
CREATE INDEX idx_vs_equipment
    ON aircraft_military.variant_sensors (equipment_id)
    WHERE equipment_id IS NOT NULL;
CREATE INDEX idx_vs_variant
    ON aircraft_military.variant_sensors (variant_id);

-- =============================================================================
-- SEED DATA — weapons_catalog (17 public reference entries)
-- Sources: Jane's, public manufacturer datasheets, Wikipedia (cross-checked).
-- Physical data only; no classified performance parameters.
-- =============================================================================

INSERT INTO aircraft_military.weapons_catalog
    (slug, name, common_name, designation, stores_type_code,
     manufacturer_name_raw, country_of_origin_code,
     weight_lbs, length_in, diameter_in, description)
VALUES

  -- Air-to-air: short-range
  ('aim-9x-sidewinder',
   'AIM-9X Sidewinder Block II', 'Sidewinder', 'AIM-9X',
   'AAM_SHORT_RANGE', 'Raytheon Technologies', 'USA',
   188, 119.4, 5.0,
   'Imaging infrared-guided short-range air-to-air missile. '
   'Off-boresight capability via JHMCS helmet cueing. Public reference only.'),

  ('r-73-archer',
   'R-73 Archer (AA-11)', 'Archer', 'R-73',
   'AAM_SHORT_RANGE', 'Vympel NPO', 'RUS',
   229, 114.2, 5.7,
   'Soviet/Russian infrared-guided short-range AAM; NATO reporting name Archer. '
   'High-off-boresight seeker with helmet cueing. Public reference only.'),

  ('aim-132-asraam',
   'AIM-132 ASRAAM', 'ASRAAM', 'AIM-132',
   'AAM_SHORT_RANGE', 'MBDA', 'GBR',
   190, 111.0, 6.6,
   'British imaging-infrared SRAAM; low-drag body optimised for high-speed launch. '
   'Public reference only.'),

  -- Air-to-air: medium-range / BVR
  ('aim-120c-amraam',
   'AIM-120C AMRAAM', 'AMRAAM', 'AIM-120C',
   'AAM_MEDIUM_RANGE', 'Raytheon Technologies', 'USA',
   335, 142.0, 7.0,
   'Active radar-guided beyond-visual-range AAM. '
   'C-variant with clipped fins for internal carriage. Public reference only.'),

  ('aim-7m-sparrow',
   'AIM-7M Sparrow', 'Sparrow', 'AIM-7M',
   'AAM_MEDIUM_RANGE', 'Raytheon Technologies', 'USA',
   510, 144.0, 8.0,
   'Semi-active radar-guided medium-range AAM. Legacy system; '
   'widely exported. Public reference only.'),

  -- Air-to-ground
  ('agm-65g-maverick',
   'AGM-65G Maverick', 'Maverick', 'AGM-65G',
   'AGM_LASER', 'Raytheon Technologies', 'USA',
   670, 98.0, 12.0,
   'Imaging infrared guided air-to-ground missile with penetrating warhead. '
   'G-variant optimised for hard targets. Public reference only.'),

  ('agm-88c-harm',
   'AGM-88C HARM', 'HARM', 'AGM-88C',
   'AGM_LASER', 'Raytheon / Texas Instruments', 'USA',
   795, 164.0, 10.0,
   'High-Speed Anti-Radiation Missile; homes on radar emissions. '
   'Primary SEAD weapon. Public reference only.'),

  ('gbu-12-paveway-ii',
   'GBU-12 Paveway II', 'Paveway II', 'GBU-12',
   'LGB', 'Raytheon Technologies', 'USA',
   585, 129.0, 10.75,
   '500 lb Mk 82 bomb with laser seeker / guidance kit. '
   'Most widely used precision bomb in Western inventories. Public reference only.'),

  ('gbu-31-jdam',
   'GBU-31 JDAM (2000 lb)', 'JDAM', 'GBU-31',
   'JDAM', 'Boeing', 'USA',
   2036, 152.7, 19.6,
   'Mk 84 / BLU-109 bomb fitted with GPS/INS guidance tail kit. '
   'All-weather precision strike. Public reference only.'),

  ('mk-82-iron-bomb',
   'Mk 82 500 lb General Purpose Bomb', 'Mk 82', 'Mk 82',
   'UNGUIDED_BOMB', 'Various', 'USA',
   531, 88.0, 10.75,
   'Unguided general-purpose bomb; basis for GBU-12 and GBU-38 JDAM. '
   'Public reference only.'),

  ('agm-158a-jassm',
   'AGM-158A JASSM', 'JASSM', 'AGM-158A',
   'AGM_LASER', 'Lockheed Martin', 'USA',
   2250, 168.0, 27.0,
   'Stealthy low-observable cruise missile for suppression of defended targets. '
   'Public reference data from Lockheed Martin fact sheet.'),

  -- Guns and gun pods
  ('m61a2-vulcan',
   'M61A2 Vulcan 20 mm Cannon', 'Vulcan', 'M61A2',
   'GUN_POD', 'General Dynamics / Nexter', 'USA',
   250, NULL, NULL,
   'Six-barrel electrically-driven Gatling cannon; 20×102 mm ammunition. '
   'Internally mounted on F-16, F/A-18, F-22. Weight is gun only (no ammo). '
   'Public reference only.'),

  -- External tanks
  ('centerline-300gal-tank',
   '300 US Gallon Centreline Drop Tank', '300 Gal Tank', NULL,
   'EXT_FUEL_TANK', 'Various', 'USA',
   182, NULL, NULL,
   'Generic 300 US gallon aluminium drop tank for centreline station. '
   'Approximate empty weight; requires wet-plumbed hardpoint. Public reference.'),

  -- Anti-ship
  ('agm-84d-harpoon',
   'AGM-84D Harpoon', 'Harpoon', 'AGM-84D',
   NULL, 'Boeing', 'USA',   -- stores_type_code NULL: no ANTI_SHIP stores_type seeded in Phase 2.
   1523, 152.0, 13.5,       -- weapon_category is ANTI_SHIP via stores_types.weapon_category_code
   'Active radar-guided anti-ship missile; airlaunched variant. '
   'Public reference only.'),

  -- Sensor / targeting pods
  ('litening-iii-pod',
   'Rafael / Northrop Grumman Litening III Targeting Pod', 'Litening III', NULL,
   'RECCE_POD', 'Rafael / Northrop Grumman', 'ISR',
   440, 77.0, 16.0,
   'Electro-optical / IR targeting pod with laser designator and range-finder. '
   'Podded system for hardpoint mounting. Public reference only.'),

  ('sniper-atp',
   'Lockheed Martin Sniper Advanced Targeting Pod', 'Sniper ATP', 'AN/AAQ-33',
   'RECCE_POD', 'Lockheed Martin', 'USA',
   441, 98.0, 12.0,
   'Advanced targeting pod: dual-mode laser, CCD-TV, FLIR, MWIR. '
   'Podded system for hardpoint. Public reference from LM fact sheet.'),

  -- ECM pod
  ('alq-184-ecm-pod',
   'AN/ALQ-184 Electronic Countermeasures Pod', 'ALQ-184', 'AN/ALQ-184',
   'ECM_POD', 'Raytheon Technologies', 'USA',
   616, 151.0, NULL,
   'Self-protection noise and deception jamming pod for tactical aircraft. '
   'Public reference only.')
ON CONFLICT (slug) DO NOTHING;

COMMIT;