-- ============================================================================
-- FILE: 006_dimensions_cabin_cargo_hangar_fit.sql
-- DESCRIPTION: Establishes external geometry, internal cabin, and hangar fit specifications.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_specs, aircraft_core, public;

-- ----------------------------------------------------------------------------
-- 1. EXTERNAL AIRFRAME GEOMETRY (Canonical standard: Feet / Square Feet)
-- ----------------------------------------------------------------------------
CREATE TABLE external_dimensions (
                                     configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Wingspan Parameters
                                     wingspan_raw TEXT,
                                     wingspan_ft public.aircraft_dim_feet NOT NULL,

    -- Length Parameters
                                     length_raw TEXT,
                                     length_ft public.aircraft_dim_feet NOT NULL,

    -- Height Parameters
                                     height_raw TEXT,
                                     height_ft public.aircraft_dim_feet NOT NULL,

    -- Wing Surface Properties
                                     wing_area_sqft NUMERIC(7, 2),
                                     aspect_ratio NUMERIC(4, 2),

    -- Rotorcraft Specifics
                                     main_rotor_diameter_ft public.aircraft_dim_feet,
                                     tail_rotor_diameter_ft public.aircraft_dim_feet,

                                     created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                     updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                     CONSTRAINT chk_wing_area CHECK (wing_area_sqft > 0.00),
                                     CONSTRAINT chk_aspect_ratio CHECK (aspect_ratio > 0.00)
);

-- ----------------------------------------------------------------------------
-- 2. INTERNAL CABIN SPACE ALLOCATIONS (Canonical standard: Feet / Cubic Feet)
-- ----------------------------------------------------------------------------
CREATE TABLE cabin_dimensions (
                                  configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Internal Envelope Measurements
                                  length_ft public.aircraft_dim_feet,
                                  width_inches NUMERIC(5, 2),
                                  height_inches NUMERIC(5, 2),
                                  volume_cuft NUMERIC(7, 2),

    -- Cabin Environment Parameters
                                  is_pressurized BOOLEAN NOT NULL DEFAULT FALSE,
                                  has_flat_floor BOOLEAN NOT NULL DEFAULT FALSE,
                                  seating_rows_count SMALLINT,

                                  created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                  updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                  CONSTRAINT chk_cabin_width CHECK (width_inches > 0.00),
                                  CONSTRAINT chk_cabin_height CHECK (height_inches > 0.00),
                                  CONSTRAINT chk_cabin_volume CHECK (volume_cuft > 0.00)
);

-- ----------------------------------------------------------------------------
-- 3. CARGO COMPARTMENTS AND APERTURE ENVELOPES
-- ----------------------------------------------------------------------------
CREATE TABLE cargo_compartments (
                                    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                    configuration_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,
                                    compartment_name TEXT NOT NULL,      -- 'Aft Baggage Bay', 'Main Deck Forward', 'Nose Locker'

    -- Capacity Envelopes
                                    volume_cuft NUMERIC(6, 2),
                                    max_weight_lbs public.aircraft_mass_lbs,

    -- Entry Door Aperture Primitives
                                    door_height_inches NUMERIC(5, 2),
                                    door_width_inches NUMERIC(5, 2),

                                    created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                    updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                    CONSTRAINT uq_compartment_identity UNIQUE (configuration_id, compartment_name)
);

-- ----------------------------------------------------------------------------
-- 4. HANGAR FIT AND GROUND MANEUVER CONSTRAINTS
-- ----------------------------------------------------------------------------
CREATE TABLE hangar_fit_dimensions (
                                       configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Storage Envelopes
                                       tail_height_ft public.aircraft_dim_feet NOT NULL,
                                       wheelbase_ft public.aircraft_dim_feet, -- Distance between main gear and nose/tail gear
                                       wheel_track_ft public.aircraft_dim_feet, -- Distance between left and right main gear wheels

    -- Foldable Airframe Adaptations (e.g., F/A-18, folding wingtip commercial airliners)
                                       has_folding_wings BOOLEAN NOT NULL DEFAULT FALSE,
                                       wingspan_folded_ft public.aircraft_dim_feet,

    -- Minimum Clearance Envelope Requirement
                                       min_hangar_door_width_clearance_ft public.aircraft_dim_feet,

                                       created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                       updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                       CONSTRAINT chk_folded_wingspan CHECK (
                                           has_folding_wings = FALSE
                                               OR (wingspan_folded_ft IS NOT NULL AND wingspan_folded_ft > 0.00)
                                           )
);

-- Index optimizations for physical footprint parameters
CREATE INDEX idx_ext_wingspan ON external_dimensions(wingspan_ft);
CREATE INDEX idx_ext_length ON external_dimensions(length_ft);
CREATE INDEX idx_cabin_volume ON cabin_dimensions(volume_cuft);
CREATE INDEX idx_cargo_weight ON cargo_compartments(max_weight_lbs);

-- Triggers for record updates
CREATE TRIGGER trg_external_dimensions_updated BEFORE UPDATE ON external_dimensions FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_cabin_dimensions_updated BEFORE UPDATE ON cabin_dimensions FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_cargo_compartments_updated BEFORE UPDATE ON cargo_compartments FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_hangar_fit_dimensions_updated BEFORE UPDATE ON hangar_fit_dimensions FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();

COMMIT;