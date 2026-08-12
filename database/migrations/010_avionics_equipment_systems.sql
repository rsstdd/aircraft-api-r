-- =============================================================================
-- File: database/migrations/010_avionics_equipment_systems.sql
-- Phase 10 — aircraft_systems: avionics and equipment item catalog,
-- variant equipment links, and named avionics suite / bundle definitions.
--
-- Design overview:
--   equipment_catalog   — named equipment items (Garmin G1000 Nxi, KAP140, etc.)
--   variant_equipment   — M:N: which equipment is installed on which variant
--   equipment_bundles   — named avionics suites / packages (e.g., "G1000 Nxi Suite")
--   bundle_items        — which equipment items compose a bundle
--   variant_bundles     — M:N: which bundles are available on which variants
--
-- Relationship with aircraft_cert:
--   Operating APPROVALS (IFR, FIKI, RVSM) live in aircraft_cert.
--   EQUIPMENT enabling those approvals lives here.
--   The same aircraft_cert.variant_operating_approvals row may be supported by
--   one or more equipment_catalog items in aircraft_systems.variant_equipment.
--
-- Spec coverage (requirement 7):
--   autopilot, flight directors    → equipment_catalog (AUTOPILOT_FMS category)
--   IFR / glass cockpit / GNSS     → equipment_catalog (FLIGHT_INSTRUMENTS / NAVIGATION)
--   ADS-B, TCAS, terrain awareness → equipment_catalog (TRAFFIC_AWARENESS / TERRAIN_AWARENESS)
--   weather radar / datalink       → equipment_catalog (WEATHER)
--   FIKI / de-ice / anti-ice       → equipment_catalog (ICE_PROTECTION)
--   pressurization / oxygen        → equipment_catalog (PRESSURIZATION)
--   avionics suites                → equipment_bundles + variant_bundles
--   optional equipment / retrofits → variant_equipment.provision_type_code
--   STC-based equipment            → variant_equipment.stc_number
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_systems.equipment_catalog
-- Named equipment / avionics item registry.
-- One row per distinct equipment model (e.g., "Garmin GFC 500 Autopilot").
-- manufacturer_name_raw: fallback when manufacturer not yet in organizations.
-- name_aliases TEXT[]: alternative names for Phase 17 ingestion matching.
-- Dedup UNIQUE on (manufacturer_org_id, name) — one row per item per maker.
-- =============================================================================

CREATE TABLE aircraft_systems.equipment_catalog (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug                  aircraft_ref.slug_text NOT NULL UNIQUE,
    name                  TEXT NOT NULL,     -- full product name e.g. "Garmin GFC 500 Autopilot"
    short_name            TEXT,              -- abbreviated e.g. "GFC 500"
    name_aliases          TEXT[],            -- alternative names; aids ingestion matching
    manufacturer_org_id   BIGINT
                              REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    manufacturer_name_raw TEXT,             -- fallback when not in organizations
    category_code         aircraft_ref.lookup_code NOT NULL
                              REFERENCES aircraft_ref.systems_categories(code),
    -- Brief technical description; fuller detail in extra_attributes JSONB.
    description           TEXT,
    -- Sparse or source-specific attributes not warranting dedicated columns.
    extra_attributes      JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ  NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_systems.equipment_catalog IS
    'Named avionics and equipment item registry. '
    'One row per distinct product model (e.g., "Garmin GFC 500 Autopilot"). '
    'Generic capability items (e.g., "Two-Axis Autopilot") may have '
    'NULL manufacturer_org_id and serve as category-level proxies when '
    'a specific product brand is not identified in the source.';
COMMENT ON COLUMN aircraft_systems.equipment_catalog.name_aliases IS
    'Alternative names and ingestion-matching aliases for this equipment item '
    '(e.g., ["KAP 140","King KAP-140","Bendix KAP140"] for one autopilot model). '
    'GIN-indexed for WHERE name_aliases @> ARRAY[''KAP140''] lookups.';

-- =============================================================================
-- aircraft_systems.variant_equipment
-- M:N junction: which equipment items are installed or available on a variant.
-- provision_type_code distinguishes STANDARD, OPTIONAL_FACTORY, RETROFIT_STC, etc.
-- stc_number: for STC-based retrofits, the specific STC approval number.
-- One row per (variant, equipment item): a variant either has a specific
-- equipment item under a given provision type, or it doesn't.
-- =============================================================================

CREATE TABLE aircraft_systems.variant_equipment (
    id                    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id            BIGINT NOT NULL
                              REFERENCES aircraft_core.variants(id)              ON DELETE CASCADE,
    equipment_id          BIGINT NOT NULL
                              REFERENCES aircraft_systems.equipment_catalog(id)  ON DELETE RESTRICT,
    provision_type_code   aircraft_ref.lookup_code NOT NULL
                              REFERENCES aircraft_ref.equipment_provision_types(code),
    -- STC approval number for retrofit / STC-based equipment.
    stc_number            TEXT,
    confidence            aircraft_ref.confidence_score,
    notes                 TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (variant_id, equipment_id)
);

COMMENT ON TABLE aircraft_systems.variant_equipment IS
    'M:N junction between aircraft variants and equipment catalog items. '
    'provision_type_code classifies the availability: STANDARD (installed as delivered), '
    'OPTIONAL_FACTORY (factory option), RETROFIT_STC (STC modification), etc. '
    'One row per (variant, equipment) pair; if the same equipment is available '
    'both as standard and via STC on the same variant sub-series, model the '
    'sub-series as distinct variants.';
COMMENT ON COLUMN aircraft_systems.variant_equipment.stc_number IS
    'STC approval number for retrofit or STC-based equipment installations. '
    'e.g., "SA02386NY" for a specific ADS-B upgrade STC. '
    'NULL for STANDARD and OPTIONAL_FACTORY provision types.';

-- =============================================================================
-- aircraft_systems.equipment_bundles
-- Named avionics suites or equipment packages offered as a complete unit.
-- Examples: "Garmin G1000 Nxi Suite", "Aspen Connected Panel Package",
--   "Piper Avionics by Garmin", "King Air 250 Fusion Avionics".
-- Bundles are composed of individual equipment_catalog items via bundle_items.
-- =============================================================================

CREATE TABLE aircraft_systems.equipment_bundles (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug           aircraft_ref.slug_text NOT NULL UNIQUE,
    name           TEXT NOT NULL,   -- "Garmin G1000 Nxi Integrated Flight Deck"
    short_name     TEXT,            -- "G1000 Nxi"
    -- Organization that offers / defines this suite (may be avionics OEM or airframer).
    offeror_org_id BIGINT
                       REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    description    TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_systems.equipment_bundles IS
    'Named avionics suites and equipment packages. '
    'A bundle groups related equipment_catalog items under a branded name '
    '(e.g., "Garmin G1000 Nxi" = PFD + MFD + GFC700 + GTX35R + GRS77 + GDU470 + ...). '
    'Bundles are linked to variants via variant_bundles; '
    'individual items within a bundle are listed in bundle_items.';

-- =============================================================================
-- aircraft_systems.bundle_items
-- Equipment items that compose a named bundle.
-- is_core: TRUE = always included; FALSE = available as an add-on within the bundle.
-- =============================================================================

CREATE TABLE aircraft_systems.bundle_items (
    bundle_id    BIGINT NOT NULL
                     REFERENCES aircraft_systems.equipment_bundles(id)   ON DELETE CASCADE,
    equipment_id BIGINT NOT NULL
                     REFERENCES aircraft_systems.equipment_catalog(id)   ON DELETE RESTRICT,
    -- TRUE = always part of this bundle; FALSE = optional addition within the bundle.
    is_core      BOOLEAN NOT NULL DEFAULT TRUE,
    notes        TEXT,
    PRIMARY KEY (bundle_id, equipment_id)
);

COMMENT ON TABLE aircraft_systems.bundle_items IS
    'Composition of equipment_bundles: which equipment_catalog items are included. '
    'is_core = TRUE for mandatory bundle components; FALSE for optional add-ons '
    'within the bundle structure (e.g., a suite offered with or without weather radar).';

-- =============================================================================
-- aircraft_systems.variant_bundles
-- M:N junction: which named avionics suites are available on which variants.
-- provision_type_code mirrors variant_equipment (STANDARD, OPTIONAL_FACTORY, etc.).
-- =============================================================================

CREATE TABLE aircraft_systems.variant_bundles (
    variant_id          BIGINT NOT NULL
                            REFERENCES aircraft_core.variants(id)              ON DELETE CASCADE,
    bundle_id           BIGINT NOT NULL
                            REFERENCES aircraft_systems.equipment_bundles(id)  ON DELETE RESTRICT,
    provision_type_code aircraft_ref.lookup_code NOT NULL
                            REFERENCES aircraft_ref.equipment_provision_types(code),
    notes               TEXT,
    PRIMARY KEY (variant_id, bundle_id)
);

COMMENT ON TABLE aircraft_systems.variant_bundles IS
    'M:N junction between aircraft variants and named avionics suites. '
    'A variant may offer multiple bundles (e.g., a base suite as STANDARD '
    'and an upgraded suite as OPTIONAL_FACTORY). '
    'Reading this table alongside bundle_items gives the full avionics picture '
    'for a variant without listing every individual equipment item.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_ec_updated
    BEFORE UPDATE ON aircraft_systems.equipment_catalog
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_systems.equipment_catalog ───────────────────────────────────────

-- Ingestion dedup: one item per (manufacturer, product name).
CREATE UNIQUE INDEX uq_ec_manufacturer_name
    ON aircraft_systems.equipment_catalog (manufacturer_org_id, name)
    WHERE manufacturer_org_id IS NOT NULL;

-- Category filter — "show all autopilot options"
CREATE INDEX idx_ec_category
    ON aircraft_systems.equipment_catalog (category_code);

-- Name search: trigram for autocomplete
CREATE INDEX idx_ec_name_trgm
    ON aircraft_systems.equipment_catalog USING gin (name gin_trgm_ops);

-- Alias-based ingestion matching
CREATE INDEX idx_ec_aliases
    ON aircraft_systems.equipment_catalog USING gin (name_aliases)
    WHERE name_aliases IS NOT NULL;

-- ── aircraft_systems.variant_equipment ────────────────────────────────────────

-- "All variants with equipment item X" — key comparison query
CREATE INDEX idx_ve_equipment
    ON aircraft_systems.variant_equipment (equipment_id);

-- Filter by provision type — "all variants with STANDARD autopilot"
-- (category join needed for specific category; idx_ve_equipment covers the eq_id path)
CREATE INDEX idx_ve_provision
    ON aircraft_systems.variant_equipment (provision_type_code);

-- All equipment for one variant (detail page)
CREATE INDEX idx_ve_variant
    ON aircraft_systems.variant_equipment (variant_id);

-- ── aircraft_systems.bundle_items ─────────────────────────────────────────────

CREATE INDEX idx_bi_equipment
    ON aircraft_systems.bundle_items (equipment_id);

-- ── aircraft_systems.variant_bundles ─────────────────────────────────────────

CREATE INDEX idx_vb_bundle
    ON aircraft_systems.variant_bundles (bundle_id);

-- =============================================================================
-- SEED DATA — equipment_catalog (18 starter rows)
-- Covers the most commonly referenced GA avionics and systems.
-- manufacturer_org_id: NULL for generic/multi-brand items.
-- These rows are matched by name_aliases during Phase 17 ingestion.
-- Additional rows are added by ingestion and curation.
-- =============================================================================

INSERT INTO aircraft_systems.equipment_catalog
    (slug, name, short_name, name_aliases, category_code, description)
VALUES

  -- FLIGHT_INSTRUMENTS / Glass Cockpit
  ('garmin-g1000-nxi',
   'Garmin G1000 Nxi Integrated Flight Deck', 'G1000 Nxi',
   ARRAY['G1000 NXi','G1000','Garmin G1000'],
   'FLIGHT_INSTRUMENTS',
   'Factory-integrated glass cockpit featuring dual GDU 1060 touchscreen displays, '
   'GFC 700 autopilot, GRS 79 AHRS, and GTX 345R ADS-B transponder.'),

  ('garmin-g3x-touch',
   'Garmin G3X Touch EFIS', 'G3X Touch',
   ARRAY['G3X','G3X Touch','Garmin G3X'],
   'FLIGHT_INSTRUMENTS',
   'Integrated EFIS for light GA and experimental aircraft; '
   '10.6-inch touchscreen PFD/MFD.'),

  ('aspen-efd1000',
   'Aspen Avionics EFD1000 Pro', 'EFD1000',
   ARRAY['EFD1000','Aspen EFD','Aspen EFD1000 Pro PFD'],
   'FLIGHT_INSTRUMENTS',
   'Drop-in primary flight display retrofit for round-dial cockpits.'),

  -- NAVIGATION
  ('garmin-gns430w',
   'Garmin GNS 430W WAAS GPS/NAV/COMM', 'GNS 430W',
   ARRAY['GNS430W','GNS 430W','Garmin 430W','430W'],
   'NAVIGATION',
   'IFR-certified WAAS GPS navigator with VHF nav and comm radio.'),

  ('garmin-gns530w',
   'Garmin GNS 530W WAAS GPS/NAV/COMM', 'GNS 530W',
   ARRAY['GNS530W','GNS 530W','Garmin 530W','530W'],
   'NAVIGATION',
   'IFR-certified WAAS GPS/nav/comm with moving-map MFD page.'),

  ('garmin-gtx750xi',
   'Garmin GTN 750Xi Touch GPS/NAV/COMM', 'GTN 750Xi',
   ARRAY['GTN750Xi','GTN 750Xi','Garmin GTN 750Xi'],
   'NAVIGATION',
   'Touchscreen WAAS GPS with integrated nav, comm, and XM weather.'),

  -- AUTOPILOT_FMS
  ('garmin-gfc500',
   'Garmin GFC 500 Autopilot', 'GFC 500',
   ARRAY['GFC500','GFC-500','Garmin GFC 500'],
   'AUTOPILOT_FMS',
   'Two-axis digital autopilot with optional GPSS and altitude pre-select.'),

  ('garmin-gfc700',
   'Garmin GFC 700 Autopilot', 'GFC 700',
   ARRAY['GFC700','GFC-700','Garmin GFC 700'],
   'AUTOPILOT_FMS',
   'Three-axis fully integrated digital flight control system for the G1000 suite.'),

  ('honeywell-kap140',
   'Honeywell / King KAP 140 Autopilot', 'KAP 140',
   ARRAY['KAP140','KAP 140','King KAP140','Bendix King KAP-140'],
   'AUTOPILOT_FMS',
   'Two-axis autopilot with altitude pre-select; widely installed in GA fleet '
   'from 1997 onward.'),

  -- TRAFFIC_AWARENESS
  ('garmin-gtx345r',
   'Garmin GTX 345R ADS-B Transponder', 'GTX 345R',
   ARRAY['GTX345R','GTX 345R','Garmin GTX 345'],
   'TRAFFIC_AWARENESS',
   'Dual-link ADS-B transponder: 1090ES out + 978 UAT in; provides '
   'traffic and FIS-B weather.'),

  ('generic-adsb-out',
   'ADS-B Out (Generic)', 'ADS-B Out',
   ARRAY['ADS-B Out','ADS-B'],
   'TRAFFIC_AWARENESS',
   'Generic placeholder for ADS-B Out capability when specific unit is not identified.'),

  -- WEATHER
  ('garmin-gwx75',
   'Garmin GWX 75 Weather Radar', 'GWX 75',
   ARRAY['GWX75','GWX 75','Garmin Weather Radar'],
   'WEATHER',
   'Colour weather radar for twin and turbine aircraft; 2D and 3D scan modes.'),

  ('avidyne-strikefinder',
   'Avidyne WX-500 Stormscope', 'Stormscope',
   ARRAY['Stormscope','WX-500','WX500','WX 500'],
   'WEATHER',
   'Passive lightning detection system; maps electrical discharge activity.'),

  -- TERRAIN_AWARENESS
  ('garmin-gts820-taws',
   'Garmin Enhanced GPWS / TAWS-B', 'TAWS-B',
   ARRAY['TAWS-B','EGPWS','GPWS','Garmin TAWS'],
   'TERRAIN_AWARENESS',
   'Class B Terrain Awareness and Warning System (FAR 135/91 compliant).'),

  -- ICE_PROTECTION
  ('tks-ice-protection',
   'TKS Liquid Ice Protection System', 'TKS',
   ARRAY['TKS','TKS Ice Protection','Known Ice / TKS'],
   'ICE_PROTECTION',
   'Titanium-mesh weeping-wing de-ice / anti-ice system; fluid distributed '
   'over leading edges.'),

  ('pneumatic-de-ice-boots',
   'Pneumatic De-Ice Boots', 'De-Ice Boots',
   ARRAY['De-ice boots','Pneumatic boots','Wing boots'],
   'ICE_PROTECTION',
   'Rubber inflation boots on wing and tail leading edges for structural ice removal.'),

  -- PRESSURIZATION
  ('generic-pressurization',
   'Pressurized Cabin System', 'Pressurized',
   ARRAY['Pressurized cabin','Pressurization system'],
   'PRESSURIZATION',
   'Generic pressurized cabin capability marker when specific system data '
   'is not available from source.'),

  -- EMERGENCY_SAFETY
  ('cirrus-caps',
   'Cirrus Airframe Parachute System (CAPS)', 'CAPS',
   ARRAY['CAPS','Airframe Parachute','BRS parachute'],
   'EMERGENCY_SAFETY',
   'Ballistic Recovery System (BRS) whole-airframe parachute. '
   'Standard on all Cirrus SR and SF50 aircraft.')

ON CONFLICT (slug) DO NOTHING;

COMMIT;