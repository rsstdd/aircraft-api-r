-- =============================================================================
-- File: database/migrations/013_maintenance_reliability_supportability.sql
-- Phase 13 — aircraft_maint: ADs, service bulletins, life-limited parts,
-- and overall supportability assessments.
--
-- Design principle: ADs and SBs are modelled as catalog-level records
-- linked to variants via M:N junctions. One AD often applies to multiple
-- variants (e.g., all Cessna 172 variants fitted with Lycoming O-320).
-- The M:N model preserves this reality while keeping the AD record
-- deduplicated. Support assessments are 1:1 per variant.
--
-- Phase 2 lookups consumed here:
--   aircraft_ref.ad_types              (5 rows: RECURRING, ONE_TIME, …)
--   aircraft_ref.sb_compliance_statuses(6 rows: MANDATORY, RECOMMENDED, …)
--   aircraft_ref.availability_grades   (5 rows: EXCELLENT → CRITICAL)
--
-- Spec coverage (requirement 10):
--   airworthiness directives           → airworthiness_directives + variant_ads
--   service bulletins                  → service_bulletins + variant_sbs
--   life-limited parts, overhaul ivls  → life_limited_parts
--   parts availability                 → support_assessments.parts_availability_grade_code
--   maintenance network                → support_assessments.maintenance_network_grade_code
--   common failure points              → support_assessments.common_issues_notes
--   corrosion risk                     → support_assessments.corrosion_risk_level
--   dispatch reliability               → support_assessments.dispatch_reliability_pct
--   fleet size                         → support_assessments.fleet_size_estimate
--   OEM support status                 → support_assessments.oem_support_status
--   mod/STC ecosystem                  → support_assessments.mod_stc_ecosystem_notes
--   owner community                    → support_assessments.owner_community_notes
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_maint.airworthiness_directives
-- AD catalog: one row per unique AD issued by a regulatory authority.
-- ADs target type certificates and may apply to multiple variants;
-- variant applicability is captured in variant_ads (M:N).
-- UNIQUE (ad_number, authority_code) prevents duplicate AD entries.
-- compliance_interval_hours: hours between inspections (RECURRING ADs).
-- compliance_interval_months: calendar interval (months between inspections).
-- =============================================================================

CREATE TABLE aircraft_maint.airworthiness_directives (
    id                          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ad_number                   TEXT NOT NULL,    -- e.g., '2023-19-04', 'AD 2022-0196R1'
    authority_code              aircraft_ref.lookup_code NOT NULL
                                    REFERENCES aircraft_ref.certification_authorities(code),
    ad_type_code                aircraft_ref.lookup_code
                                    REFERENCES aircraft_ref.ad_types(code),
    subject                     TEXT NOT NULL,    -- brief subject from the AD title
    description                 TEXT,             -- fuller public description
    effective_date              DATE,
    -- For RECURRING ADs: compliance interval.
    compliance_interval_hours   NUMERIC,          -- e.g., 100 hours
    compliance_interval_months  SMALLINT,         -- e.g., 24 months
    -- For ONE_TIME ADs: initial compliance deadline.
    initial_compliance_date     DATE,
    -- Supersession chain: if this AD is replaced by a newer AD.
    superseded_by_ad_number     TEXT,             -- AD number that supersedes this one
    -- Link to official regulatory document.
    reference_url               TEXT,
    -- FALSE when this AD has been fully superseded and is no longer enforceable.
    is_active                   BOOLEAN NOT NULL DEFAULT TRUE,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (ad_number, authority_code),
    CONSTRAINT chk_ad_interval CHECK (
        (compliance_interval_hours  IS NULL OR compliance_interval_hours  > 0)
        AND (compliance_interval_months IS NULL OR compliance_interval_months > 0)
    )
);

COMMENT ON TABLE aircraft_maint.airworthiness_directives IS
    'Airworthiness Directive catalog. One row per AD issued by a regulatory authority. '
    'ADs may apply to many variants; variant applicability is in variant_ads (M:N). '
    'UNIQUE (ad_number, authority_code) deduplicates across issuing bodies. '
    'is_active = FALSE when the AD has been fully superseded.';
COMMENT ON COLUMN aircraft_maint.airworthiness_directives.compliance_interval_hours IS
    'Repeat inspection interval in flight hours (RECURRING ADs). '
    'NULL for ONE_TIME ADs where compliance is a single event.';
COMMENT ON COLUMN aircraft_maint.airworthiness_directives.superseded_by_ad_number IS
    'AD number of the replacement (newer revision). '
    'Free text since the replacement may not yet be in the database.';

-- =============================================================================
-- aircraft_maint.variant_ads
-- M:N junction: which ADs apply to which variants.
-- applicability_notes: free-text noting any sub-model or serial restrictions
--   from the AD applicability section (e.g., "S/N 12000-15000 only").
-- is_significant: curator flag for ADs considered notable in buyer research
--   (major structural concern, grounding-type compliance requirement).
-- =============================================================================

CREATE TABLE aircraft_maint.variant_ads (
    variant_id           BIGINT NOT NULL
                             REFERENCES aircraft_core.variants(id)                      ON DELETE CASCADE,
    ad_id                BIGINT NOT NULL
                             REFERENCES aircraft_maint.airworthiness_directives(id)    ON DELETE RESTRICT,
    applicability_notes  TEXT,   -- serial/sub-model restrictions from AD text
    -- TRUE for ADs curators consider notably significant for buyer research.
    is_significant       BOOLEAN NOT NULL DEFAULT FALSE,
    notes                TEXT,
    PRIMARY KEY (variant_id, ad_id)
);

COMMENT ON TABLE aircraft_maint.variant_ads IS
    'M:N junction between aircraft variants and airworthiness directives. '
    'applicability_notes records serial number or sub-model restrictions. '
    'is_significant is a curator flag for ADs worth highlighting in buyer '
    'research (e.g., major structural ADs, frequent recurring inspections).';

-- =============================================================================
-- aircraft_maint.service_bulletins
-- Service bulletin catalog: manufacturer recommendations/advisories.
-- One row per unique SB (sb_number + issuer).
-- compliance_status_code captures the overall classification of this SB
--   (MANDATORY = effectively regulatory even if issued by manufacturer;
--    RECOMMENDED = strongly advised; OPTIONAL = elective improvement).
-- =============================================================================

CREATE TABLE aircraft_maint.service_bulletins (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    sb_number                TEXT NOT NULL,
    issuer_org_id            BIGINT
                                 REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    issuer_name_raw          TEXT,     -- fallback when issuer not in organizations
    compliance_status_code   aircraft_ref.lookup_code
                                 REFERENCES aircraft_ref.sb_compliance_statuses(code),
    subject                  TEXT NOT NULL,
    description              TEXT,
    issued_date              DATE,
    -- Supersession tracking
    supersedes_sb_number     TEXT,     -- this SB supersedes the named earlier SB
    superseded_by_sb_number  TEXT,     -- this SB is superseded by the named newer SB
    reference_url            TEXT,
    is_active                BOOLEAN NOT NULL DEFAULT TRUE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_maint.service_bulletins IS
    'Service Bulletin catalog from aircraft and component manufacturers. '
    'SBs are manufacturer recommendations; compliance_status_code notes '
    'whether a specific SB has been elevated to effectively mandatory '
    '(e.g., when required by a subsequent AD that references the SB). '
    'Variant applicability is in variant_sbs (M:N).';

-- =============================================================================
-- aircraft_maint.variant_sbs
-- M:N junction: which SBs apply to which variants.
-- estimated_compliance_cost_usd: approximate cost to comply, for buyer research.
--   NULL = cost not published or not estimated.
-- =============================================================================

CREATE TABLE aircraft_maint.variant_sbs (
    variant_id                  BIGINT NOT NULL
                                    REFERENCES aircraft_core.variants(id)               ON DELETE CASCADE,
    sb_id                       BIGINT NOT NULL
                                    REFERENCES aircraft_maint.service_bulletins(id)     ON DELETE RESTRICT,
    applicability_notes         TEXT,
    -- Approximate out-of-pocket cost to comply at a certificated repair station.
    estimated_compliance_cost_usd NUMERIC(10,2),
    notes                       TEXT,
    PRIMARY KEY (variant_id, sb_id),
    CONSTRAINT chk_vsb_cost CHECK (
        estimated_compliance_cost_usd IS NULL OR estimated_compliance_cost_usd >= 0
    )
);

COMMENT ON TABLE aircraft_maint.variant_sbs IS
    'M:N junction between aircraft variants and service bulletins. '
    'estimated_compliance_cost_usd supports buyer-research cost modelling '
    'and is populated from service center estimates or published SB cost data.';

-- =============================================================================
-- aircraft_maint.life_limited_parts
-- Parts with certified retirement lives or mandatory overhaul intervals.
-- life_limit_hours: hard life in hours (must be retired at this TT or SMOH).
-- life_limit_calendar: calendar limit (e.g., "20 calendar years").
-- At least one limit must be set (chk_llp_has_limit).
-- manufacturer_org_id: OEM source for this life limit data.
-- =============================================================================

CREATE TABLE aircraft_maint.life_limited_parts (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id               BIGINT NOT NULL
                                 REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    part_number              TEXT,
    part_description         TEXT NOT NULL,  -- e.g., "Tail rotor hub assembly"
    manufacturer_org_id      BIGINT
                                 REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    -- Life limits
    life_limit_hours         NUMERIC,       -- hard retirement life in flight hours
    life_limit_calendar      TEXT,          -- calendar limit description
    -- Overhaul interval (if overhaul extends life rather than retiring the part)
    overhaul_interval_hours  NUMERIC,
    -- Source of the life limit (e.g., "CMM Rev 4, Section 5-10")
    source_reference         TEXT,
    notes                    TEXT,
    CONSTRAINT chk_llp_has_limit CHECK (
        life_limit_hours IS NOT NULL OR life_limit_calendar IS NOT NULL
    ),
    CONSTRAINT chk_llp_hours_positive CHECK (
        (life_limit_hours       IS NULL OR life_limit_hours       > 0)
        AND (overhaul_interval_hours IS NULL OR overhaul_interval_hours > 0)
    )
);

COMMENT ON TABLE aircraft_maint.life_limited_parts IS
    'Parts with certified life limits (hard-time retirement or mandatory overhaul). '
    'Common in certificated helicopters, turbines, and some high-stress GA components. '
    'At least one of life_limit_hours or life_limit_calendar must be set. '
    'life_limited_parts affects buyer research: approaching or exceeded limits '
    'require replacement before sale/operation.';
COMMENT ON COLUMN aircraft_maint.life_limited_parts.overhaul_interval_hours IS
    'Hours between mandatory overhauls if overhaul is permitted instead of retirement. '
    'NULL when the part is hard-time: must be retired at life_limit_hours '
    'regardless of condition.';

-- =============================================================================
-- aircraft_maint.support_assessments
-- Overall supportability profile for a variant.
-- 1:1 with variants (UNIQUE on variant_id).
-- Represents an editorial assessment updated periodically (snapshot_date).
-- Multiple assessment snapshots per variant are NOT supported by this design:
-- each UPDATE overwrites the prior state; use updated_at for change tracking.
-- If historical assessment snapshots are needed, remove UNIQUE and track by date.
-- oem_support_status, corrosion_risk_level: TEXT CHECK (small stable sets).
-- parts_availability_grade_code and maintenance_network_grade_code both FK to
-- aircraft_ref.availability_grades — shared grade scale, separate columns.
-- =============================================================================

CREATE TABLE aircraft_maint.support_assessments (
    id                              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id                      BIGINT NOT NULL UNIQUE
                                        REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    snapshot_date                   DATE NOT NULL DEFAULT CURRENT_DATE,

    -- ── Fleet size ────────────────────────────────────────────────────────────
    fleet_size_estimate             INTEGER,        -- approx units in service
    fleet_size_source               TEXT,           -- 'FAA Registry', 'GAMA', 'Jane''s'
    fleet_size_year                 aircraft_ref.year_value,

    -- ── OEM support status ────────────────────────────────────────────────────
    -- FULL_SUPPORT:      OEM actively supports; new parts available
    -- LIMITED_SUPPORT:   OEM support limited; some parts discontinued
    -- PARTS_ONLY:        OEM supplies spares only; no engineering/tech support
    -- DISCONTINUED:      OEM no longer supports this type
    oem_support_status              TEXT,

    -- ── Availability grades (both FK to aircraft_ref.availability_grades) ─────
    parts_availability_grade_code   aircraft_ref.lookup_code
                                        REFERENCES aircraft_ref.availability_grades(code),
    maintenance_network_grade_code  aircraft_ref.lookup_code
                                        REFERENCES aircraft_ref.availability_grades(code),

    -- ── Corrosion risk ────────────────────────────────────────────────────────
    -- Composite of design (aluminium vs steel vs composite), operating environment,
    -- and known type-specific corrosion issues.
    corrosion_risk_level            TEXT,

    -- ── Narrative assessments ─────────────────────────────────────────────────
    common_issues_notes             TEXT,   -- publicly documented common failure points
    mod_stc_ecosystem_notes         TEXT,   -- available STCs, PMA parts, mod shops
    owner_community_notes           TEXT,   -- type clubs, online communities, support network

    -- ── Dispatch reliability (where publicly available) ───────────────────────
    dispatch_reliability_pct        NUMERIC(5,2),  -- % of flights completed without AOG
    dispatch_reliability_source     TEXT,          -- citation for this figure

    confidence                      aircraft_ref.confidence_score,
    notes                           TEXT,
    created_at                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT chk_sa_oem_status CHECK (
        oem_support_status IS NULL
        OR oem_support_status IN (
            'FULL_SUPPORT', 'LIMITED_SUPPORT', 'PARTS_ONLY', 'DISCONTINUED'
        )
    ),
    CONSTRAINT chk_sa_corrosion CHECK (
        corrosion_risk_level IS NULL
        OR corrosion_risk_level IN ('LOW', 'MODERATE', 'HIGH', 'VERY_HIGH')
    ),
    CONSTRAINT chk_sa_dispatch CHECK (
        dispatch_reliability_pct IS NULL
        OR (dispatch_reliability_pct >= 0 AND dispatch_reliability_pct <= 100)
    ),
    CONSTRAINT chk_sa_fleet CHECK (
        fleet_size_estimate IS NULL OR fleet_size_estimate >= 0
    )
);

COMMENT ON TABLE aircraft_maint.support_assessments IS
    'Supportability profile for a variant (1:1, UNIQUE on variant_id). '
    'Combines fleet size, OEM support status, parts/network grade, corrosion risk, '
    'and narrative assessments into a single curated overview for buyer research. '
    'oem_support_status TEXT CHECK (4 values): FULL_SUPPORT, LIMITED_SUPPORT, '
    'PARTS_ONLY, DISCONTINUED. corrosion_risk_level TEXT CHECK (4 levels). '
    'dispatch_reliability_pct must cite dispatch_reliability_source.';
COMMENT ON COLUMN aircraft_maint.support_assessments.dispatch_reliability_pct IS
    'Percentage of scheduled flights completed without an AOG (Aircraft on Ground) '
    'technical delay. Only available for commercial-operation types with public '
    'statistical reporting. NULL for most GA types. Must cite dispatch_reliability_source.';
COMMENT ON COLUMN aircraft_maint.support_assessments.fleet_size_estimate IS
    'Approximate number of this variant in active service worldwide. '
    'Source typically FAA Registry (US), GAMA statistics, or Jane''s census. '
    'Larger fleets generally correlate with better parts and MRO availability.';

-- =============================================================================
-- TRIGGER
-- =============================================================================

CREATE TRIGGER trg_support_assessments_updated
    BEFORE UPDATE ON aircraft_maint.support_assessments
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_maint.airworthiness_directives ───────────────────────────────────

CREATE INDEX idx_ad_authority
    ON aircraft_maint.airworthiness_directives (authority_code);
CREATE INDEX idx_ad_type
    ON aircraft_maint.airworthiness_directives (ad_type_code);
-- Identify superseded (inactive) ADs.
CREATE INDEX idx_ad_inactive
    ON aircraft_maint.airworthiness_directives (ad_number)
    WHERE NOT is_active;
-- AD number search (partial match for docket-number lookups).
CREATE INDEX idx_ad_number_trgm
    ON aircraft_maint.airworthiness_directives USING gin (ad_number gin_trgm_ops);

-- ── aircraft_maint.variant_ads ────────────────────────────────────────────────

-- "All variants with this AD" — reverse traversal.
CREATE INDEX idx_va_ad
    ON aircraft_maint.variant_ads (ad_id);
-- "Significant ADs for buyer research" — quick filter.
CREATE INDEX idx_va_significant
    ON aircraft_maint.variant_ads (variant_id)
    WHERE is_significant;

-- ── aircraft_maint.service_bulletins ─────────────────────────────────────────

-- Prevent duplicate SBs from the same known issuer.
CREATE UNIQUE INDEX uq_sb_number_org
    ON aircraft_maint.service_bulletins (issuer_org_id, sb_number)
    WHERE issuer_org_id IS NOT NULL;

CREATE INDEX idx_sb_compliance
    ON aircraft_maint.service_bulletins (compliance_status_code);
CREATE INDEX idx_sb_issuer
    ON aircraft_maint.service_bulletins (issuer_org_id)
    WHERE issuer_org_id IS NOT NULL;
CREATE INDEX idx_sb_active
    ON aircraft_maint.service_bulletins (sb_number)
    WHERE is_active;

-- ── aircraft_maint.variant_sbs ────────────────────────────────────────────────

-- "All variants with this SB" — reverse traversal.
CREATE INDEX idx_vsb_sb
    ON aircraft_maint.variant_sbs (sb_id);

-- ── aircraft_maint.life_limited_parts ─────────────────────────────────────────

CREATE INDEX idx_llp_variant
    ON aircraft_maint.life_limited_parts (variant_id);
CREATE INDEX idx_llp_part_number
    ON aircraft_maint.life_limited_parts (part_number)
    WHERE part_number IS NOT NULL;

-- ── aircraft_maint.support_assessments ───────────────────────────────────────

-- UNIQUE on variant_id already creates an index — no separate idx needed.
-- Grade-based buyer filters.
CREATE INDEX idx_sa_parts
    ON aircraft_maint.support_assessments (parts_availability_grade_code)
    WHERE parts_availability_grade_code IS NOT NULL;
CREATE INDEX idx_sa_network
    ON aircraft_maint.support_assessments (maintenance_network_grade_code)
    WHERE maintenance_network_grade_code IS NOT NULL;
CREATE INDEX idx_sa_oem
    ON aircraft_maint.support_assessments (oem_support_status)
    WHERE oem_support_status IS NOT NULL;
-- Fleet size sort: "largest fleet = best parts support".
CREATE INDEX idx_sa_fleet
    ON aircraft_maint.support_assessments (fleet_size_estimate DESC NULLS LAST)
    WHERE fleet_size_estimate IS NOT NULL;

COMMIT;