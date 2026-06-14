-- ============================================================================
-- FILE: 005_certification_operating_approvals.sql
-- DESCRIPTION: Establishes certification, airworthiness, and crew requirement models.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_cert, aircraft_core, aircraft_org, public;

-- ----------------------------------------------------------------------------
-- 1. TYPE CERTIFICATION REGISTRY
-- ----------------------------------------------------------------------------
CREATE TABLE type_certificates (
                                   id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                   variant_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_variants(id) ON DELETE CASCADE,
                                   authority_id BIGINT NOT NULL REFERENCES aircraft_org.organizations(id) ON DELETE RESTRICT,
                                   certificate_number TEXT NOT NULL,    -- '3A12' (FAA Cessna 172), 'A.014' (EASA Airbus A320)
                                   airworthiness_category TEXT NOT NULL, -- 'NORMAL', 'UTILITY', 'ACROBATIC', 'COMMUTER', 'TRANSPORT', 'TRANSPORT_CATEGORY_MILITARY'
                                   certification_basis TEXT,           -- 'FAR Part 23', 'CS-25', 'CAR 3'
                                   issue_date DATE,
                                   created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                   updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                   CONSTRAINT uq_authority_cert UNIQUE (authority_id, certificate_number),
                                   CONSTRAINT chk_airworthiness_category CHECK (airworthiness_category IN (
                                                                                                           'NORMAL', 'UTILITY', 'ACROBATIC', 'COMMUTER', 'TRANSPORT', 'TRANSPORT_CATEGORY_MILITARY', 'EXPERIMENTAL', 'LIGHT_SPORT'
                                       ))
);

-- ----------------------------------------------------------------------------
-- 2. OPERATING APPROVALS & SYSTEM ENVELOPE ENFORCEMENTS
-- ----------------------------------------------------------------------------
CREATE TABLE operating_approvals (
                                     id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                     variant_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_variants(id) ON DELETE CASCADE,
                                     has_fiki_approval BOOLEAN NOT NULL DEFAULT FALSE,       -- Flight Into Known Icing
                                     has_ifr_approval BOOLEAN NOT NULL DEFAULT TRUE,         -- Instrument Flight Rules
                                     has_vfr_night_approval BOOLEAN NOT NULL DEFAULT TRUE,   -- Night VFR Operations
                                     has_rvsm_approval BOOLEAN NOT NULL DEFAULT FALSE,      -- Reduced Vertical Separation Minimum
                                     has_rvsm_compliant_avionics BOOLEAN NOT NULL DEFAULT FALSE,
                                     max_operating_altitude_ft public.aircraft_dim_feet,     -- Service ceiling limitation
                                     max_cabin_altitude_ft public.aircraft_dim_feet,
                                     max_cabin_differential_psi NUMERIC(4, 2),               -- Pressurization envelope strength
                                     limit_load_factor_flaps_up_positive NUMERIC(3, 2),      -- e.g., +3.8 G
                                     limit_load_factor_flaps_up_negative NUMERIC(3, 2),      -- e.g., -1.52 G
                                     has_ballistic_parachute BOOLEAN NOT NULL DEFAULT FALSE, -- e.g., CAPS system on Cirrus platforms
                                     created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                     updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                     CONSTRAINT chk_load_factors CHECK (limit_load_factor_flaps_up_positive > 0.00 AND limit_load_factor_flaps_up_negative < 0.00),
                                     CONSTRAINT chk_pressurization CHECK (max_cabin_differential_psi >= 0.00 AND max_cabin_differential_psi < 15.00)
);

-- ----------------------------------------------------------------------------
-- 3. PILOT CERTIFICATE, RATING, AND TYPE RATING MANDATES
-- ----------------------------------------------------------------------------
CREATE TABLE pilot_requirements (
                                    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                    variant_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_variants(id) ON DELETE CASCADE,
                                    minimum_crew_count SMALLINT NOT NULL DEFAULT 1,
                                    required_license_level TEXT NOT NULL, -- 'SPORT', 'PRIVATE', 'COMMERCIAL', 'AIRLINE_TRANSPORT'
                                    is_type_rating_required BOOLEAN NOT NULL DEFAULT FALSE,
                                    type_rating_designator VARCHAR(10),   -- 'CL-65', 'B-737', 'CE-500'
                                    requires_high_performance_endorsement BOOLEAN NOT NULL DEFAULT FALSE, -- > 200 HP
                                    requires_complex_endorsement BOOLEAN NOT NULL DEFAULT FALSE,          -- Retractable + Flaps + Controllable Prop
                                    requires_tailwheel_endorsement BOOLEAN NOT NULL DEFAULT FALSE,
                                    created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                    updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                    CONSTRAINT chk_crew_count CHECK (minimum_crew_count >= 1),
                                    CONSTRAINT chk_license_level CHECK (required_license_level IN ('SPORT', 'PRIVATE', 'COMMERCIAL', 'AIRLINE_TRANSPORT', 'MILITARY_ONLY'))
);

-- Index optimizations for targeted filtering
CREATE INDEX idx_cert_variant ON type_certificates(variant_id);
CREATE INDEX idx_approvals_variant ON operating_approvals(variant_id);
CREATE INDEX idx_pilot_reqs_variant ON pilot_requirements(variant_id);
CREATE INDEX idx_pilot_type_rating ON pilot_requirements(type_rating_designator) WHERE is_type_rating_required = TRUE;

-- Triggers for record updates
CREATE TRIGGER trg_type_certificates_updated BEFORE UPDATE ON type_certificates FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_operating_approvals_updated BEFORE UPDATE ON operating_approvals FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_pilot_requirements_updated BEFORE UPDATE ON pilot_requirements FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();

COMMIT;