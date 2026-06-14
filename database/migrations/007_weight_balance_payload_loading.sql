-- ============================================================================
-- FILE: 007_weight_balance_payload_loading.sql
-- DESCRIPTION: Establishes aircraft weight limits, fuel capacities, and CG envelopes.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_specs, aircraft_core, public;

-- ----------------------------------------------------------------------------
-- 1. STRUCTURAL WEIGHT LIMITS (Canonical standard: Pounds / LBS)
-- ----------------------------------------------------------------------------
CREATE TABLE weight_limits (
                               configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Empty Weight States
                               basic_empty_weight_lbs public.aircraft_mass_lbs,       -- BEW: Airframe, engines, unusable fuel, full oil
                               operating_empty_weight_lbs public.aircraft_mass_lbs,   -- OEW: BEW + crew, crew baggage, operator fluids

    -- Maximum Structural Limits
                               max_ramp_weight_lbs public.aircraft_mass_lbs,          -- Maximum weight for ground taxi
                               max_takeoff_weight_lbs public.aircraft_mass_lbs NOT NULL, -- MTOW: Maximum weight authorized for takeoff release
                               max_landing_weight_lbs public.aircraft_mass_lbs,       -- MLW: Maximum structural landing impact mass
                               max_zero_fuel_weight_lbs public.aircraft_mass_lbs,     -- MZFW: Maximum weight prior to usable fuel loading

    -- Calculated Carrying Envelopes
                               max_payload_lbs public.aircraft_mass_lbs,              -- MZFW - OEW
                               max_baggage_capacity_lbs public.aircraft_mass_lbs,

    -- Design Metrics
                               max_wing_loading_lbs_sqft NUMERIC(6, 2),               -- MTOW divided by wing area

                               created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                               updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                               CONSTRAINT chk_mtow_gt_empty CHECK (max_takeoff_weight_lbs > COALESCE(operating_empty_weight_lbs, basic_empty_weight_lbs, 0.00)),
                               CONSTRAINT chk_ramp_gte_mtow CHECK (max_ramp_weight_lbs >= max_takeoff_weight_lbs),
                               CONSTRAINT chk_mtow_gte_mlw CHECK (max_takeoff_weight_lbs >= COALESCE(max_landing_weight_lbs, 0.00)),
                               CONSTRAINT chk_mtow_gte_mzfw CHECK (max_takeoff_weight_lbs >= COALESCE(max_zero_fuel_weight_lbs, 0.00))
);

-- ----------------------------------------------------------------------------
-- 2. FUEL VOLUMETRIC AND MASS ALLOCATIONS
-- ----------------------------------------------------------------------------
CREATE TABLE fuel_mass_capacities (
                                      configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Volumetric Primitives
                                      total_capacity_gal public.aircraft_volume_gal NOT NULL,
                                      usable_capacity_gal public.aircraft_volume_gal NOT NULL,
                                      unusable_capacity_gal public.aircraft_volume_gal NOT NULL,

    -- Derived Weights based on fuel grade density at standard temperature
    -- Gasoline (Avgas): ~6.0 lbs/gal, Jet-A/JP-8: ~6.7 lbs/gal
                                      fuel_density_lbs_gal NUMERIC(4, 2) NOT NULL DEFAULT 6.70,
                                      usable_fuel_weight_lbs public.aircraft_mass_lbs GENERATED ALWAYS AS (usable_capacity_gal * fuel_density_lbs_gal) STORED,

                                      created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                      updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                      CONSTRAINT chk_fuel_capacities CHECK (total_capacity_gal = usable_capacity_gal + unusable_capacity_gal),
                                      CONSTRAINT chk_usable_capacity CHECK (usable_capacity_gal > 0.00),
                                      CONSTRAINT chk_fuel_density CHECK (fuel_density_lbs_gal >= 5.00 AND fuel_density_lbs_gal <= 7.50)
);

-- ----------------------------------------------------------------------------
-- 3. CENTER OF GRAVITY (CG) LIMIT ENVELOPE STRUCTS
-- ----------------------------------------------------------------------------
CREATE TABLE cg_envelope_points (
                                    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                    configuration_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- CG Bounds coordinate system relative to reference datum
                                    aircraft_weight_lbs public.aircraft_mass_lbs NOT NULL,
                                    forward_limit_inches_aft_datum NUMERIC(6, 2) NOT NULL,
                                    aft_limit_inches_aft_datum NUMERIC(6, 2) NOT NULL,

    -- Optional lateral/vertical constraints for advanced stability profiling
                                    lateral_limit_inches_left NUMERIC(5, 2),
                                    lateral_limit_inches_right NUMERIC(5, 2),

                                    created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                    CONSTRAINT chk_cg_bounds CHECK (aft_limit_inches_aft_datum >= forward_limit_inches_aft_datum),
                                    CONSTRAINT chk_lateral_bounds CHECK (lateral_limit_inches_right >= COALESCE(lateral_limit_inches_left, 0.00))
);

-- Index optimizations for sorting weights and capacities
CREATE INDEX idx_weights_mtow ON weight_limits(max_takeoff_weight_lbs);
CREATE INDEX idx_fuel_usable ON fuel_mass_capacities(usable_capacity_gal);
CREATE INDEX idx_cg_config ON cg_envelope_points(configuration_id, aircraft_weight_lbs);

-- Triggers for record updates
CREATE TRIGGER trg_weight_limits_updated BEFORE UPDATE ON weight_limits FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_fuel_mass_capacities_updated BEFORE UPDATE ON fuel_mass_capacities FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();

COMMIT;