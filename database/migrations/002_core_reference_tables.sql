-- =============================================================================
-- File: database/migrations/002_core_reference_tables.sql
-- Phase 2 — aircraft_ref lookup/reference tables
-- No seed rows are inserted here. See:
--   seeds/001_reference_units.sql  → unit_categories + measurement_units
--   seeds/002_lookup_seed_data.sql → all other lookup tables
--
-- =============================================================================

BEGIN;

-- =============================================================================
-- GROUP 1: UNIT INFRASTRUCTURE
-- All physical measurements reference these two tables.
-- measurement_units carries a self-referential FK (canonical_unit_code)
-- declared DEFERRABLE INITIALLY DEFERRED to allow single-pass seed INSERTs.
-- =============================================================================

CREATE TABLE aircraft_ref.unit_categories
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0
);
COMMENT ON TABLE aircraft_ref.unit_categories IS
    'Physical-quantity category groupings for measurement_units '
    '(e.g., SPEED, WEIGHT, THRUST). One row per quantity dimension.';

CREATE TABLE aircraft_ref.measurement_units
(
    code                   aircraft_ref.lookup_code PRIMARY KEY,
    label                  TEXT                     NOT NULL,
    symbol                 TEXT,
    unit_category_code     aircraft_ref.lookup_code NOT NULL
        REFERENCES aircraft_ref.unit_categories (code),
    -- NULL means this row IS the canonical unit for its category
    canonical_unit_code    aircraft_ref.lookup_code,
    -- raw_value × canonical_factor = value in canonical_unit_code
    canonical_factor       NUMERIC(18, 10),
    -- raw_value × si_factor = value in SI base unit (informational)
    si_factor              NUMERIC(18, 10),
    si_base_unit_symbol    TEXT,
    -- lowercase strings from raw seed data that identify this unit;
    -- used by Phase 17 ingestion parser to map source strings to codes
    source_string_patterns TEXT[],
    sort_order             SMALLINT                 NOT NULL DEFAULT 0,
    is_active              BOOLEAN                  NOT NULL DEFAULT TRUE,
    CONSTRAINT chk_mu_canonical_pair CHECK (
        (canonical_unit_code IS NULL AND canonical_factor IS NULL)
            OR (canonical_unit_code IS NOT NULL AND canonical_factor IS NOT NULL)
        )
);
COMMENT ON TABLE aircraft_ref.measurement_units IS
    'Unit registry. Rows with canonical_unit_code IS NULL are themselves the '
    'canonical unit for their unit_category. Others store: '
    'raw_value × canonical_factor = canonical_value. '
    'source_string_patterns aids the Phase 17 ingestion unit resolver.';
COMMENT ON COLUMN aircraft_ref.measurement_units.canonical_factor IS
    'Multiply any raw source value in this unit by this factor to obtain the '
    'value expressed in canonical_unit_code. NULL for canonical units themselves.';
COMMENT ON COLUMN aircraft_ref.measurement_units.source_string_patterns IS
    'Case-insensitive strings from raw data that map to this unit '
    '(e.g., ''{kias, kts, knots}'' for the KNOTS row). Used by Phase 17.';

-- Self-referential FK; DEFERRABLE so canonical and non-canonical units
-- can be INSERTed in a single transaction without ordering constraints.
ALTER TABLE aircraft_ref.measurement_units
    ADD CONSTRAINT fk_mu_canonical
        FOREIGN KEY (canonical_unit_code)
            REFERENCES aircraft_ref.measurement_units (code)
            DEFERRABLE INITIALLY DEFERRED;

-- =============================================================================
-- GROUP 2: AIRCRAFT TAXONOMY LOOKUPS
-- Consumed by Phase 4 (aircraft_core: families, models, variants, roles).
-- =============================================================================

CREATE TABLE aircraft_ref.aircraft_roles
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    -- Informational grouping for display/faceting; not a FK.
    -- Values: CIVILIAN_COMMERCIAL | CIVILIAN_BUSINESS | CIVILIAN_GA |
    --         CIVILIAN_SPECIAL | ROTARY_CIVIL | UAV_CIVIL |
    --         MILITARY_FIXED_WING | MILITARY_ROTARY | MILITARY_UAV
    role_group  TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.aircraft_roles IS
    'Enumeration of aircraft roles (e.g., MILITARY_FIGHTER_MULTIROLE, '
    'GENERAL_AVIATION_TOURING). A variant may hold multiple roles via '
    'the aircraft_core.variant_roles M:N junction (Phase 4).';
COMMENT ON COLUMN aircraft_ref.aircraft_roles.role_group IS
    'Non-FK grouping label for display clustering and faceted filtering. '
    'Intentionally denormalized here; a full role hierarchy is out of scope.';

CREATE TABLE aircraft_ref.service_statuses
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.service_statuses IS
    'Production and service lifecycle status of an aircraft variant '
    '(e.g., IN_PRODUCTION, DISCONTINUED, EXPERIMENTAL).';

CREATE TABLE aircraft_ref.variant_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.variant_types IS
    'Classification of a variant within its model lineage '
    '(e.g., PRODUCTION_STANDARD, PROTOTYPE, EXPORT, MILITARY_CONVERSION).';

-- =============================================================================
-- GROUP 3: PHYSICAL PROPERTY LOOKUPS
-- Consumed by Phase 9 (propulsion) and Phase 4 (aircraft identity).
-- propulsion_categories.primary_power_unit FK is DEFERRABLE so the row
-- can be inserted in seeds/002_lookup_seed_data.sql after measurement_units
-- are populated by seeds/001_reference_units.sql.
-- =============================================================================

CREATE TABLE aircraft_ref.landing_gear_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.landing_gear_types IS
    'Landing gear configuration (e.g., FIXED_TRICYCLE, RETRACTABLE_TAILWHEEL, '
    'FLOATS, SKIDS). Replaces the TEXT CHECK constraint in the reference schema.';

CREATE TABLE aircraft_ref.propulsion_categories
(
    code               aircraft_ref.lookup_code PRIMARY KEY,
    label              TEXT     NOT NULL,
    description        TEXT,
    is_jet             BOOLEAN  NOT NULL DEFAULT FALSE,
    is_rotating        BOOLEAN  NOT NULL DEFAULT FALSE,
    -- Points to HP (piston/turboprop/turboshaft) or LBF (jet/rocket)
    primary_power_unit aircraft_ref.lookup_code
        REFERENCES aircraft_ref.measurement_units (code)
            DEFERRABLE INITIALLY DEFERRED,
    sort_order         SMALLINT NOT NULL DEFAULT 0,
    is_active          BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.propulsion_categories IS
    'Propulsion system type (e.g., PISTON_RECIPROCATING, TURBOFAN_HIGH_BPR). '
    'Replaces TEXT CHECK engine_type_category in the reference schema. '
    'is_jet / is_rotating support faceted filtering. '
    'primary_power_unit guides performance comparison (HP vs LBF).';

CREATE TABLE aircraft_ref.fuel_types
(
    code                aircraft_ref.lookup_code PRIMARY KEY,
    label               TEXT     NOT NULL,
    description         TEXT,
    -- Approximate density at standard conditions; NULL for non-liquid fuels.
    -- Enables PPH ↔ GPH approximate conversion in Phase 17 ingestion.
    density_lbs_per_gal NUMERIC(6, 4),
    sort_order          SMALLINT NOT NULL DEFAULT 0,
    is_active           BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.fuel_types IS
    'Fuel and energy carrier types (e.g., AVGAS_100LL, JET_A, ELECTRIC). '
    'density_lbs_per_gal enables approximate PPH ↔ GPH conversion '
    'where fuel density is known.';

-- =============================================================================
-- GROUP 4: METRIC TYPE LOOKUPS
-- These three tables enumerate named measurement types for the fact tables
-- introduced in Phases 6 (dimensions), 7 (weights), and 8 (performance).
-- canonical_unit_code FKs are DEFERRABLE so these rows can be inserted in
-- seeds/002_lookup_seed_data.sql alongside the metric fact rows.
-- =============================================================================

CREATE TABLE aircraft_ref.dimension_metric_types
(
    code                aircraft_ref.lookup_code PRIMARY KEY,
    label               TEXT     NOT NULL,
    description         TEXT,
    canonical_unit_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.measurement_units (code)
            DEFERRABLE INITIALLY DEFERRED,
    sort_order          SMALLINT NOT NULL DEFAULT 0,
    is_active           BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.dimension_metric_types IS
    'Named dimensional metrics (e.g., DIM_WINGSPAN, DIM_CABIN_LENGTH) '
    'with their canonical measurement unit. Used as FK by '
    'aircraft_specs.dimension_metrics (Phase 6).';

CREATE TABLE aircraft_ref.weight_metric_types
(
    code                aircraft_ref.lookup_code PRIMARY KEY,
    label               TEXT     NOT NULL,
    description         TEXT,
    canonical_unit_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.measurement_units (code)
            DEFERRABLE INITIALLY DEFERRED,
    sort_order          SMALLINT NOT NULL DEFAULT 0,
    is_active           BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.weight_metric_types IS
    'Named weight and loading metrics (e.g., WEIGHT_MTOW, FUEL_CAPACITY_USABLE) '
    'with canonical unit. Used as FK by aircraft_specs.weight_metrics (Phase 7).';

CREATE TABLE aircraft_ref.performance_metric_types
(
    code                aircraft_ref.lookup_code PRIMARY KEY,
    label               TEXT     NOT NULL,
    description         TEXT,
    canonical_unit_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.measurement_units (code)
            DEFERRABLE INITIALLY DEFERRED,
    -- TRUE = higher canonical value is better (range, ceiling, speed);
    -- FALSE = lower is better (runway distance, fuel burn, stall speed);
    -- NULL = context-dependent.
    is_higher_better    BOOLEAN,
    is_speed            BOOLEAN  NOT NULL DEFAULT FALSE,
    is_distance         BOOLEAN  NOT NULL DEFAULT FALSE,
    is_rate             BOOLEAN  NOT NULL DEFAULT FALSE,
    sort_order          SMALLINT NOT NULL DEFAULT 0,
    is_active           BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.performance_metric_types IS
    'Named performance metrics (e.g., SPEED_CRUISE_BEST, CEILING_SERVICE, '
    'RANGE_NORMAL). is_higher_better drives comparison sort direction in '
    'Phase 15 mission-profile scoring. Used as FK by '
    'aircraft_specs.performance_metrics (Phase 8).';

-- =============================================================================
-- GROUP 5: CERTIFICATION LOOKUPS
-- Consumed by Phase 5 (aircraft_cert).
-- authority_code FKs inside this group are DEFERRABLE to allow
-- any insert ordering within a single transaction.
--
-- operating_approval_types was previously defined inline in Phase 5
-- (005_certification_operating_approvals.sql). It has been moved here
-- because it is a pure lookup with no dependency on Phase 5 tables,
-- removing the need for Phase 5 to self-seed its own lookup table.
-- Phase 5's aircraft_cert.variant_operating_approvals now has a clean
-- FK reference to this table without any ordering sensitivity.
-- =============================================================================

CREATE TABLE aircraft_ref.certification_authorities
(
    code          aircraft_ref.lookup_code PRIMARY KEY,
    label         TEXT     NOT NULL,
    full_name     TEXT,
    country_codes TEXT[], -- ISO 3166-1 alpha-3; array for multi-jurisdiction bodies
    website_url   TEXT,
    sort_order    SMALLINT NOT NULL DEFAULT 0,
    is_active     BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.certification_authorities IS
    'Aviation regulatory/certification bodies (FAA, EASA, TCCA, etc.) '
    'with their jurisdictions. country_codes is an array because some bodies '
    '(EASA) cover multiple member states.';

CREATE TABLE aircraft_ref.airworthiness_categories
(
    code           aircraft_ref.lookup_code PRIMARY KEY,
    label          TEXT     NOT NULL,
    description    TEXT,
    authority_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.certification_authorities (code)
            DEFERRABLE INITIALLY DEFERRED,
    sort_order     SMALLINT NOT NULL DEFAULT 0,
    is_active      BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.airworthiness_categories IS
    'Airworthiness/operating categories '
    '(e.g., FAA_NORMAL, FAA_TRANSPORT, EASA_CS23_NORMAL). '
    'Replaces TEXT CHECK in the reference schema.';

CREATE TABLE aircraft_ref.pilot_certificate_types
(
    code           aircraft_ref.lookup_code PRIMARY KEY,
    label          TEXT     NOT NULL,
    description    TEXT,
    authority_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.certification_authorities (code)
            DEFERRABLE INITIALLY DEFERRED,
    sort_order     SMALLINT NOT NULL DEFAULT 0,
    is_active      BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.pilot_certificate_types IS
    'Pilot certificate/licence types required to operate aircraft '
    '(e.g., FAA_PRIVATE, FAA_ATP, FAA_TYPE_RATING).';

CREATE TABLE aircraft_ref.operating_approval_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    -- TRUE = this approval broadens operations (IFR, RVSM, ETOPS);
    -- FALSE = this approval restricts or prohibits an operation (NOT_AEROBATIC).
    is_positive BOOLEAN  NOT NULL DEFAULT TRUE,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.operating_approval_types IS
    'Types of operating approvals that can be asserted per variant '
    '(e.g., IFR, FIKI, AEROBATIC, CAT_II, ETOPS_120, RVSM). '
    'Moved from Phase 5 inline seed to Phase 2 so the lookup exists '
    'before aircraft_cert.variant_operating_approvals (Phase 5) is created. '
    'Seeded in seeds/002_lookup_seed_data.sql.';

-- =============================================================================
-- GROUP 6: MILITARY LOOKUPS
-- Consumed by Phase 11 (aircraft_military). Encyclopedia reference only;
-- no operational/tactical data.
-- =============================================================================

CREATE TABLE aircraft_ref.military_mission_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.military_mission_types IS
    'Public military mission type classifications used for encyclopedia '
    'comparison only (e.g., AIR_SUPERIORITY, CLOSE_AIR_SUPPORT, '
    'MARITIME_PATROL). No operational or tactical content.';

CREATE TABLE aircraft_ref.weapon_categories
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.weapon_categories IS
    'Broad public weapon/store categories for representative loadout reference '
    '(e.g., AIR_TO_AIR, AIR_TO_GROUND, SENSOR_POD). '
    'Parent category for aircraft_ref.stores_types.';

CREATE TABLE aircraft_ref.hardpoint_position_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.hardpoint_position_types IS
    'Physical location of a hardpoint or weapons station '
    '(e.g., WING_INBOARD, FUSELAGE_CENTERLINE, INTERNAL_BAY).';

CREATE TABLE aircraft_ref.stores_types
(
    code                 aircraft_ref.lookup_code PRIMARY KEY,
    label                TEXT     NOT NULL,
    description          TEXT,
    weapon_category_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.weapon_categories (code),
    sort_order           SMALLINT NOT NULL DEFAULT 0,
    is_active            BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.stores_types IS
    'Finer-grained external stores classification '
    '(e.g., AAM_SHORT_RANGE, LGB, EXT_FUEL_TANK). '
    'weapon_category_code groups stores for display/filtering.';

-- =============================================================================
-- GROUP 7: MARKET / COST LOOKUPS
-- Consumed by Phase 12 (aircraft_market).
--
-- cost_item_types change note:
--   is_aggregate BOOLEAN (new) — TRUE for the three pre-computed total rows
--   that PlanePHD provides (TOTAL_COST_ANNUAL, TOTAL_FIXED_COST,
--   TOTAL_VARIABLE_COST). These rows must be routed to
--   aircraft_market.cost_snapshot_totals during ingestion, NOT inserted into
--   aircraft_market.cost_line_items. A CHECK constraint on cost_line_items
--   (fix_001) enforces this separation at the database level.
--
--   is_fuel BOOLEAN (removed) — was never consumed by any Phase 3-17 table or
--   function. Fuel-type classification is handled by aircraft_ref.fuel_types
--   and aircraft_ref.propulsion_categories. Removing it eliminates an
--   unmaintained boolean that would diverge silently from those tables.
-- =============================================================================

CREATE TABLE aircraft_ref.currencies
(
    code           VARCHAR(3) PRIMARY KEY, -- ISO 4217
    label          TEXT     NOT NULL,
    symbol         TEXT     NOT NULL,
    decimal_places SMALLINT NOT NULL DEFAULT 2,
    is_active      BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.currencies IS
    'ISO 4217 currency codes for cost, valuation, and market data. '
    'Uses VARCHAR(3) rather than lookup_code domain to align with '
    'the ISO 4217 standard key format.';

CREATE TABLE aircraft_ref.cost_item_types
(
    code         aircraft_ref.lookup_code PRIMARY KEY,
    label        TEXT     NOT NULL,
    description  TEXT,
    -- TRUE  = annual fixed cost (insurance, storage, inspection).
    -- FALSE = per-hour variable cost (fuel, oil, maintenance reserve).
    is_fixed     BOOLEAN  NOT NULL DEFAULT FALSE,
    -- TRUE for TOTAL_COST_ANNUAL, TOTAL_FIXED_COST, TOTAL_VARIABLE_COST.
    -- Rows with is_aggregate = TRUE are routed to
    -- aircraft_market.cost_snapshot_totals during ingestion and are
    -- PROHIBITED from aircraft_market.cost_line_items (enforced by
    -- chk_cli_no_aggregate on that table). Never include is_aggregate rows
    -- in SUM(cost_line_items.amount_annual) — they are pre-computed totals,
    -- not components.
    is_aggregate BOOLEAN  NOT NULL DEFAULT FALSE,
    sort_order   SMALLINT NOT NULL DEFAULT 0,
    is_active    BOOLEAN  NOT NULL DEFAULT TRUE,
    -- Aggregates should not be classified as fixed or variable.
    CONSTRAINT chk_cit_aggregate_not_typed CHECK (
        NOT (is_aggregate = TRUE AND is_fixed = TRUE)
    )
);
COMMENT ON TABLE aircraft_ref.cost_item_types IS
    'Named ownership cost line items (e.g., FUEL_COST_PER_HOUR, '
    'ANNUAL_INSPECTION, ENGINE_RESERVE). '
    'is_fixed distinguishes annual fixed costs (TRUE) from per-hour variable '
    'costs (FALSE). '
    'is_aggregate = TRUE marks pre-computed totals provided by the source '
    '(TOTAL_COST_ANNUAL, TOTAL_FIXED_COST, TOTAL_VARIABLE_COST) that go to '
    'aircraft_market.cost_snapshot_totals, not cost_line_items. '
    'Never SUM() is_aggregate rows with component rows — they double-count.';

COMMENT ON COLUMN aircraft_ref.cost_item_types.is_aggregate IS
    'TRUE for rows that represent a pre-computed sum of other line items. '
    'The three aggregate codes are: TOTAL_COST_ANNUAL, TOTAL_FIXED_COST, '
    'TOTAL_VARIABLE_COST. These are routed to cost_snapshot_totals during '
    'Phase 17 ingestion. chk_cli_no_aggregate on cost_line_items enforces '
    'the separation at write time.';

CREATE TABLE aircraft_ref.aircraft_condition_grades
(
    code          aircraft_ref.lookup_code PRIMARY KEY,
    label         TEXT     NOT NULL,
    description   TEXT,
    numeric_score SMALLINT, -- 1 = worst, 5 = best; for score-based filtering
    sort_order    SMALLINT NOT NULL DEFAULT 0,
    is_active     BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.aircraft_condition_grades IS
    'Subjective aircraft condition grades used in market valuation assumptions '
    '(e.g., EXCELLENT, GOOD, FAIR, POOR, SALVAGE). '
    'numeric_score enables range-filtering in buyer-research queries.';

-- =============================================================================
-- GROUP 8: MAINTENANCE LOOKUPS
-- Consumed by Phase 13 (aircraft_maint).
-- =============================================================================

CREATE TABLE aircraft_ref.ad_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.ad_types IS
    'Airworthiness Directive types (e.g., RECURRING, ONE_TIME, EMERGENCY).';

CREATE TABLE aircraft_ref.sb_compliance_statuses
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.sb_compliance_statuses IS
    'Compliance status for Service Bulletins '
    '(e.g., MANDATORY, RECOMMENDED, OPTIONAL, SUPERSEDED).';

CREATE TABLE aircraft_ref.availability_grades
(
    code          aircraft_ref.lookup_code PRIMARY KEY,
    label         TEXT     NOT NULL,
    description   TEXT,
    numeric_score SMALLINT, -- 1 = worst/critical, 5 = best/excellent
    sort_order    SMALLINT NOT NULL DEFAULT 0,
    is_active     BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.availability_grades IS
    'Generic availability grade scale (EXCELLENT through CRITICAL). '
    'Used by aircraft_maint.support_assessments (parts_availability_grade_code '
    'and maintenance_network_grade_code) for parts and maintenance-network quality.';

-- =============================================================================
-- GROUP 9: PROVENANCE / CURATION LOOKUPS
-- Consumed by Phase 14 (aircraft_prov).
-- =============================================================================

CREATE TABLE aircraft_ref.source_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.source_types IS
    'Classification of data source origin '
    '(e.g., SCRAPED_WEB, MANUFACTURER_SPEC, TYPE_CERTIFICATE, MANUAL_ENTRY).';

CREATE TABLE aircraft_ref.source_reliability_grades
(
    code          aircraft_ref.lookup_code PRIMARY KEY,
    label         TEXT     NOT NULL,
    description   TEXT,
    numeric_score SMALLINT NOT NULL,
    sort_order    SMALLINT NOT NULL DEFAULT 0,
    is_active     BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.source_reliability_grades IS
    'Source reliability tiers for default confidence baselines '
    '(AUTHORITATIVE=5 through UNVERIFIED=1). '
    'numeric_score / 5 gives the default aircraft_ref.confidence_score.';

CREATE TABLE aircraft_ref.curation_flag_statuses
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    is_terminal BOOLEAN  NOT NULL DEFAULT FALSE,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.curation_flag_statuses IS
    'Lifecycle statuses for curation flags (e.g., OPEN, UNDER_REVIEW, '
    'RESOLVED, DISMISSED). is_terminal marks end states where no further '
    'curator action is expected.';

CREATE TABLE aircraft_ref.curation_entity_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    schema_name TEXT,
    table_name  TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.curation_entity_types IS
    'Entity types that can carry curation flags or source assertions '
    '(e.g., VARIANT, ENGINE, COST_SNAPSHOT). schema_name / table_name '
    'document the real PostgreSQL table for each logical entity type.';

CREATE TABLE aircraft_ref.assertion_statuses
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.assertion_statuses IS
    'Review/acceptance status of individual source assertions '
    '(e.g., PENDING, ACCEPTED, REJECTED, CONFLICT, SUPERSEDED).';

-- =============================================================================
-- GROUP 10: COMPARISON / MISSION LOOKUPS
-- Consumed by Phase 15 (aircraft_compare).
-- =============================================================================

CREATE TABLE aircraft_ref.mission_profile_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.mission_profile_types IS
    'Named mission profiles used by the comparison engine '
    '(e.g., IFR_CROSSCOUNTRY, BACKCOUNTRY_STOL, MILITARY_CLOSE_AIR_SUPPORT). '
    'Each profile carries weighted scoring criteria in Phase 15.';

CREATE TABLE aircraft_ref.comparison_criterion_types
(
    code                    aircraft_ref.lookup_code PRIMARY KEY,
    label                   TEXT     NOT NULL,
    description             TEXT,
    performance_metric_code aircraft_ref.lookup_code
        REFERENCES aircraft_ref.performance_metric_types (code)
            DEFERRABLE INITIALLY DEFERRED,
    weight_metric_code      aircraft_ref.lookup_code
        REFERENCES aircraft_ref.weight_metric_types (code)
            DEFERRABLE INITIALLY DEFERRED,
    dimension_metric_code   aircraft_ref.lookup_code
        REFERENCES aircraft_ref.dimension_metric_types (code)
            DEFERRABLE INITIALLY DEFERRED,
    is_higher_better        BOOLEAN,
    sort_order              SMALLINT NOT NULL DEFAULT 0,
    is_active               BOOLEAN  NOT NULL DEFAULT TRUE,
    CONSTRAINT chk_criterion_single_domain CHECK (
        (CASE WHEN performance_metric_code IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN weight_metric_code IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN dimension_metric_code IS NOT NULL THEN 1 ELSE 0 END) <= 1
        )
);
COMMENT ON TABLE aircraft_ref.comparison_criterion_types IS
    'Named comparison dimensions used in mission-profile scoring '
    '(e.g., CRITERION_RANGE, CRITERION_RUNWAY_TAKEOFF). '
    'At most one metric-type FK is set per criterion. '
    'Composite/computed criteria leave all three metric FKs NULL.';

-- =============================================================================
-- GROUP 11: ORGANIZATION LOOKUPS
-- Consumed by Phase 3 (aircraft_org).
-- =============================================================================

CREATE TABLE aircraft_ref.organization_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.organization_types IS
    'Classification of organizations in the aviation ecosystem '
    '(e.g., MANUFACTURER, DESIGN_BUREAU, OPERATOR_MILITARY, '
    'CERTIFICATION_AUTHORITY).';

CREATE TABLE aircraft_ref.org_relationship_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.org_relationship_types IS
    'Relationship types between organizations '
    '(e.g., SUBSIDIARY, LICENSE_AGREEMENT, SUCCESSOR_ENTITY, '
    'CONSORTIUM_MEMBER).';

-- =============================================================================
-- GROUP 12: AVIONICS / SYSTEMS LOOKUPS
-- Consumed by Phase 10 (aircraft_systems).
-- =============================================================================

CREATE TABLE aircraft_ref.systems_categories
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.systems_categories IS
    'Categories of aircraft avionics and systems '
    '(e.g., NAVIGATION, AUTOPILOT_FMS, ICE_PROTECTION, TERRAIN_AWARENESS). '
    'Used by aircraft_systems.variant_equipment (Phase 10).';

CREATE TABLE aircraft_ref.equipment_provision_types
(
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT     NOT NULL,
    description TEXT,
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    is_active   BOOLEAN  NOT NULL DEFAULT TRUE
);
COMMENT ON TABLE aircraft_ref.equipment_provision_types IS
    'How a specific avionics/equipment item is provided on a variant '
    '(e.g., STANDARD, OPTIONAL_FACTORY, RETROFIT_STC, NOT_AVAILABLE). '
    'Used by aircraft_systems.variant_equipment (Phase 10).';

COMMIT;