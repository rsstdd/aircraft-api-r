-- =============================================================================
-- File: database/migrations/005_certification_operating_approvals.sql
-- Phase 5 — aircraft_cert: type certificates, variant certification links,
-- operating approvals, pilot requirements, and safety metrics.
--
-- All tables in this phase FK to aircraft_core.variants.id.
-- aircraft_ref.operating_approval_types is a new lookup table added to the
-- aircraft_ref schema here (not in Phase 2) because it is first consumed in
-- this phase; all lookup tables canonically live in aircraft_ref.
--
-- Spec coverage (requirement 11):
--   certification authority       → type_certificates.authority_code
--   type certificate              → type_certificates
--   airworthiness category        → type_certificates.airworthiness_category_code
--   certification basis           → type_certificates.certification_basis
--   pilot certificate requirement → pilot_requirements.min_certificate_code
--   ratings required              → pilot_requirements.*
--   type rating requirement       → pilot_requirements.type_rating_required
--   minimum crew                  → pilot_requirements.min_crew
--   operating limitations         → variant_operating_approvals (fact rows)
--   icing / night / IFR / aerobatic → operating_approval_types seed values
--   accident-rate metrics         → safety_metrics
-- =============================================================================

BEGIN;


-- =============================================================================
-- aircraft_cert.type_certificates
-- Formal type certificate records issued by aviation regulatory authorities.
-- One TC record covers the design approval for a model series; individual
-- variants link to one or more TCs via variant_type_certs (M:N).
-- =============================================================================

CREATE TABLE aircraft_cert.type_certificates (
    id                          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- TC number as published by the authority (e.g., "3A4", "EASA.A.064",
    -- "IM-E4" for an engine TC).
    tc_number                   TEXT NOT NULL,
    authority_code              aircraft_ref.lookup_code NOT NULL
                                    REFERENCES aircraft_ref.certification_authorities(code),
    -- Organization that holds the TC (may differ from original designer
    -- after acquisition; e.g., Textron Aviation now holds Cessna TCs).
    tc_holder_org_id            BIGINT
                                    REFERENCES aircraft_org.organizations(id) ON DELETE SET NULL,
    airworthiness_category_code aircraft_ref.lookup_code
                                    REFERENCES aircraft_ref.airworthiness_categories(code),
    -- Regulatory standard the design is certified to (free-text verbatim
    -- from TCDS, e.g., "14 CFR Part 23, Amendments 23-1 through 23-48").
    certification_basis         TEXT,
    -- Date the original TC was issued.
    issued_date                 DATE,
    -- Date of the most recent TCDS amendment.
    amended_date                DATE,
    -- Official TCDS document URL (FAA TCDS are publicly hosted on faa.gov).
    tcds_url                    TEXT,
    notes                       TEXT,
    extra_attributes            JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tc_number, authority_code)
);
COMMENT ON TABLE aircraft_cert.type_certificates IS
    'Type Certificate records from aviation regulatory authorities. '
    'One TC may cover many model variants (1:N); variants link via '
    'variant_type_certs (M:N). The TC holder may differ from the original '
    'designer after acquisitions.';
COMMENT ON COLUMN aircraft_cert.type_certificates.tc_number IS
    'TC identifier as published by the authority: '
    'FAA format "3A4"; EASA format "EASA.A.064"; '
    'Transport Canada format "A-82". Combined with authority_code for uniqueness.';
COMMENT ON COLUMN aircraft_cert.type_certificates.certification_basis IS
    'Regulatory standard verbatim from the TCDS '
    '(e.g., "14 CFR Part 23, effective Feb 1 1965, Amendments 23-1 through 23-48"). '
    'Stored as free-text to preserve the authoritative wording.';
COMMENT ON COLUMN aircraft_cert.type_certificates.tcds_url IS
    'URL to the official Type Certificate Data Sheet document. '
    'For FAA: https://rgl.faa.gov/Regulatory_and_Guidance_Library/rgMakeModel.nsf/...';

-- =============================================================================
-- aircraft_cert.variant_type_certs
-- M:N junction: a variant may be covered by multiple TCs (FAA + EASA + TCCA),
-- and a single TC often covers an entire model series.
-- tc_model_name preserves the exact designation string from the TC
-- (may differ from the canonical variant slug/name).
-- =============================================================================

CREATE TABLE aircraft_cert.variant_type_certs (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id    BIGINT NOT NULL
                      REFERENCES aircraft_core.variants(id)        ON DELETE CASCADE,
    tc_id         BIGINT NOT NULL
                      REFERENCES aircraft_cert.type_certificates(id) ON DELETE RESTRICT,
    -- Model name as it appears on the type certificate
    -- (e.g., "Model 172S" may be listed as "172S" on the TCDS).
    tc_model_name TEXT,
    -- Date this variant was added to the TC (initial or amendment date).
    approved_date DATE,
    notes         TEXT,
    UNIQUE (variant_id, tc_id)
);
COMMENT ON TABLE aircraft_cert.variant_type_certs IS
    'M:N junction linking aircraft variants to their type certificates. '
    'A variant may hold multiple TCs (different authorities). '
    'tc_model_name preserves the exact designation used in the TCDS, '
    'which may differ from the canonical variant name.';

-- =============================================================================
-- aircraft_cert.variant_operating_approvals
-- Fact table: one row per (variant, approval_type) pair.
-- is_approved: TRUE = approved; FALSE = explicitly not approved;
--              NULL = not determined from available sources.
-- conditions: free-text limitations or qualifications on the approval
--   (e.g., "STC SA02386NY required", "Day VMC only", "Below FL280").
-- confidence: source reliability score for this assertion (0-1).
-- =============================================================================

CREATE TABLE aircraft_cert.variant_operating_approvals (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id          BIGINT NOT NULL
                            REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    approval_type_code  aircraft_ref.lookup_code NOT NULL
                            REFERENCES aircraft_ref.operating_approval_types(code),
    -- Three-state: TRUE = approved, FALSE = not approved, NULL = unknown.
    is_approved         BOOLEAN,
    -- Conditions, limitations, or STC references qualifying this approval.
    conditions          TEXT,
    confidence          aircraft_ref.confidence_score,
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (variant_id, approval_type_code)
);
COMMENT ON TABLE aircraft_cert.variant_operating_approvals IS
    'Per-variant operating approval facts: IFR, FIKI, aerobatic, CAT II ILS, etc. '
    'is_approved is three-state (TRUE/FALSE/NULL) to distinguish '
    '"explicitly approved", "explicitly not approved", and "not yet curated". '
    'conditions captures limitations or STC requirements qualifying the approval.';
COMMENT ON COLUMN aircraft_cert.variant_operating_approvals.is_approved IS
    'TRUE = explicitly approved; FALSE = explicitly not approved; '
    'NULL = status not established from available sources (curation required).';
COMMENT ON COLUMN aircraft_cert.variant_operating_approvals.confidence IS
    'Source reliability for this approval assertion (aircraft_ref.confidence_score '
    'domain: 0.00–1.00). Populated by Phase 17 ingestion from source reliability '
    'grade; updated during curation.';

-- =============================================================================
-- aircraft_cert.pilot_requirements
-- 1:1 extension of aircraft_core.variants (enforced by UNIQUE on variant_id).
-- Captures the minimum pilot certification, ratings, and endorsements
-- required to act as PIC of this variant.
--
-- FAA-specific endorsement columns (requires_complex, requires_high_perf,
-- requires_tailwheel) are broadly applicable as de facto categories
-- internationally but are formally defined under US FARs.
-- These columns are NULL for variants where FAA endorsements are not relevant.
-- =============================================================================

CREATE TABLE aircraft_cert.pilot_requirements (
    id                      BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id              BIGINT NOT NULL UNIQUE
                                REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    -- Minimum pilot certificate required to act as PIC.
    min_certificate_code    aircraft_ref.lookup_code
                                REFERENCES aircraft_ref.pilot_certificate_types(code),
    -- TRUE = a specific aircraft type rating is required.
    type_rating_required    BOOLEAN,
    -- ICAO aircraft type designator used for the type rating
    -- (e.g., "B738", "A320", "C208"). NULL if type_rating_required = FALSE.
    type_rating_code        TEXT,
    -- Minimum certificated flight crew (1 = single-pilot, 2 = two-crew).
    min_crew                SMALLINT,
    -- FAA complex aircraft endorsement (retractable gear + const-speed prop + flaps).
    requires_complex        BOOLEAN,
    -- FAA high-performance endorsement (>200 HP at takeoff).
    requires_high_perf      BOOLEAN,
    -- FAA tailwheel endorsement (conventional gear).
    requires_tailwheel      BOOLEAN,
    -- Instrument rating required even for VFR-only aircraft
    -- (e.g., aircraft with partial-panel IFR avionics).
    requires_instrument     BOOLEAN,
    notes                   TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_pr_min_crew CHECK (min_crew IS NULL OR min_crew >= 1),
    CONSTRAINT chk_pr_type_rating CHECK (
        -- type_rating_code may only be set when type_rating_required = TRUE
        type_rating_code IS NULL OR type_rating_required = TRUE
    )
);
COMMENT ON TABLE aircraft_cert.pilot_requirements IS
    '1:1 extension of aircraft_core.variants capturing pilot certification '
    'requirements: minimum certificate level, type rating, minimum crew, '
    'and endorsements. UNIQUE on variant_id enforces the 1:1 relationship. '
    'Endorsement columns (requires_complex, requires_high_perf, requires_tailwheel) '
    'are defined under US FARs but used as reference categories internationally.';
COMMENT ON COLUMN aircraft_cert.pilot_requirements.type_rating_code IS
    'ICAO aircraft type designator for the required type rating '
    '(e.g., "B738" = Boeing 737-800, "A320" = Airbus A320 family). '
    'NULL when type_rating_required = FALSE or = NULL.';
COMMENT ON COLUMN aircraft_cert.pilot_requirements.min_crew IS
    'Minimum certificated flight crew for this variant: '
    '1 = single-pilot approved; 2 = two-crew (PIC + SIC required); '
    'NULL = not established from available sources.';

-- =============================================================================
-- aircraft_cert.safety_metrics
-- Public accident-rate and reliability statistics for a variant.
-- Stores one row per (variant, metric_type, data_source) triple.
-- metric_type is free-text to accommodate varying source terminology
-- (NTSB, EASA, ASI, IAOPA, AOPA all publish different rate definitions).
-- Confidence and source notes support the provenance model introduced in
-- Phase 14; this table is a forward-compatible staging point.
-- =============================================================================

CREATE TABLE aircraft_cert.safety_metrics (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id       BIGINT NOT NULL
                         REFERENCES aircraft_core.variants(id) ON DELETE CASCADE,
    -- Free-text metric name matching the source terminology:
    -- e.g., "fatal_accidents_per_million_flight_hours",
    --       "hull_loss_rate_per_100k_departures",
    --       "total_accidents_per_100k_hours"
    metric_type      TEXT NOT NULL,
    rate_value       NUMERIC,
    -- Unit / denominator description (e.g., "per million flight hours")
    rate_unit        TEXT,
    -- Calendar period the statistic covers.
    period_start_year aircraft_ref.year_value,
    period_end_year   aircraft_ref.year_value,
    -- Name of the statistical body or publication.
    data_source      TEXT,
    -- URL to the originating report (if publicly available).
    source_url       TEXT,
    confidence       aircraft_ref.confidence_score,
    notes            TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_sm_period CHECK (
        period_end_year IS NULL
        OR period_start_year IS NULL
        OR period_end_year >= period_start_year
    ),
    CONSTRAINT chk_sm_rate_nonneg CHECK (
        rate_value IS NULL OR rate_value >= 0
    )
);
COMMENT ON TABLE aircraft_cert.safety_metrics IS
    'Public accident-rate and safety statistics per variant. '
    'metric_type is free-text to accommodate different source terminologies '
    '(NTSB, EASA, ASI). data_source and source_url enable citation. '
    'Once aircraft_prov (Phase 14) is operational, assertions here can be '
    'linked to source_documents for full provenance tracking.';
COMMENT ON COLUMN aircraft_cert.safety_metrics.metric_type IS
    'Source-verbatim metric name (e.g., "fatal_accidents_per_million_flight_hours"). '
    'Not FK-constrained; free-text preserves source terminology variation.';
COMMENT ON COLUMN aircraft_cert.safety_metrics.rate_value IS
    'The numeric rate value. Always non-negative (CHECK). '
    'NULL if the source reports the metric qualitatively.';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_type_certs_updated
    BEFORE UPDATE ON aircraft_cert.type_certificates
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_voa_updated
    BEFORE UPDATE ON aircraft_cert.variant_operating_approvals
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_pilot_req_updated
    BEFORE UPDATE ON aircraft_cert.pilot_requirements
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_safety_metrics_updated
    BEFORE UPDATE ON aircraft_cert.safety_metrics
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_ref.operating_approval_types ───────────────────────────────────
-- PK already indexed; add active filter for quick lookup list
CREATE INDEX idx_oat_active
    ON aircraft_ref.operating_approval_types (sort_order)
    WHERE is_active;

-- ── aircraft_cert.type_certificates ─────────────────────────────────────────
CREATE INDEX idx_tc_authority
    ON aircraft_cert.type_certificates (authority_code);

CREATE INDEX idx_tc_holder
    ON aircraft_cert.type_certificates (tc_holder_org_id)
    WHERE tc_holder_org_id IS NOT NULL;

CREATE INDEX idx_tc_airworthiness
    ON aircraft_cert.type_certificates (airworthiness_category_code)
    WHERE airworthiness_category_code IS NOT NULL;

CREATE INDEX idx_tc_number_trgm
    ON aircraft_cert.type_certificates USING gin (tc_number gin_trgm_ops);

-- ── aircraft_cert.variant_type_certs ────────────────────────────────────────
-- "All variants on this TC" — reverse traversal
CREATE INDEX idx_vtc_tc
    ON aircraft_cert.variant_type_certs (tc_id);

-- "All TCs for this variant" — covered by UNIQUE idx (variant_id, tc_id)

-- ── aircraft_cert.variant_operating_approvals ────────────────────────────────
-- "All variants with FIKI approval" — key buyer-search facet
CREATE INDEX idx_voa_type_approved
    ON aircraft_cert.variant_operating_approvals (approval_type_code, is_approved)
    WHERE is_approved IS NOT NULL;

-- ── aircraft_cert.pilot_requirements ────────────────────────────────────────
-- Type rating filter — important for high-value aircraft search
CREATE INDEX idx_pr_type_rating
    ON aircraft_cert.pilot_requirements (type_rating_required)
    WHERE type_rating_required IS NOT NULL;

-- Certificate level filter — e.g., "aircraft that require ATP"
CREATE INDEX idx_pr_certificate
    ON aircraft_cert.pilot_requirements (min_certificate_code)
    WHERE min_certificate_code IS NOT NULL;

-- Complex / high-perf / tailwheel endorsement filters
CREATE INDEX idx_pr_complex
    ON aircraft_cert.pilot_requirements (requires_complex)
    WHERE requires_complex;
CREATE INDEX idx_pr_high_perf
    ON aircraft_cert.pilot_requirements (requires_high_perf)
    WHERE requires_high_perf;
CREATE INDEX idx_pr_tailwheel
    ON aircraft_cert.pilot_requirements (requires_tailwheel)
    WHERE requires_tailwheel;

-- ── aircraft_cert.safety_metrics ────────────────────────────────────────────
CREATE INDEX idx_sm_variant
    ON aircraft_cert.safety_metrics (variant_id);

CREATE INDEX idx_sm_type
    ON aircraft_cert.safety_metrics (metric_type);


COMMIT;