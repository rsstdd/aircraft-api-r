-- ============================================================================
-- FILE: 008_performance_metrics.sql
-- DESCRIPTION: Establishes speed profiles, airfield runway needs, and cruise curves.
-- ============================================================================

BEGIN;

SET search_path TO aircraft_perf, aircraft_core, public;

-- ----------------------------------------------------------------------------
-- 1. STRUCTURAL AIRSPEED V-SPEED PRIMITIVES (Canonical standard: KTAS)
-- ----------------------------------------------------------------------------
CREATE TABLE speed_limits (
                              configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Stall Airspeeds
                              vs0_stall_flaps_down_ktas public.aircraft_speed_knots NOT NULL, -- Stall speed in landing config
                              vs1_stall_clean_ktas public.aircraft_speed_knots NOT NULL,      -- Stall speed specified clean

    -- Maneuvering and Operational Safety Speeds
                              vx_best_angle_climb_ktas public.aircraft_speed_knots,
                              vy_best_rate_climb_ktas public.aircraft_speed_knots,
                              va_maneuvering_speed_ktas public.aircraft_speed_knots,
                              vfe_max_flaps_extended_ktas public.aircraft_speed_knots,
                              vlo_max_gear_operating_ktas public.aircraft_speed_knots,
                              vle_max_gear_extended_ktas public.aircraft_speed_knots,

    -- Structural Structural Redlines
                              vno_max_structural_cruise_ktas public.aircraft_speed_knots,
                              vne_never_exceed_ktas public.aircraft_speed_knots NOT NULL,

    -- Multi-Engine Structural Safeguards
                              vmca_minimum_control_air_ktas public.aircraft_speed_knots,
                              vsse_intentional_one_engine_inop_ktas public.aircraft_speed_knots,

                              created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                              updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                              CONSTRAINT chk_stalls CHECK (vs1_stall_clean_ktas >= vs0_stall_flaps_down_ktas),
                              CONSTRAINT chk_never_exceed CHECK (vne_never_exceed_ktas > vs1_stall_clean_ktas),
                              CONSTRAINT chk_cruise_redline CHECK (vne_never_exceed_ktas >= COALESCE(vno_max_structural_cruise_ktas, 0.00))
);

-- ----------------------------------------------------------------------------
-- 2. RUNWAY FIELD PERFORMANCE SPECIFICATIONS (Canonical standard: Feet)
-- ----------------------------------------------------------------------------
CREATE TABLE field_performance (
                                   configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Takeoff Field Requirements (Calculated at Max Takeoff Weight, Sea Level, ISA)
                                   takeoff_ground_roll_ft public.aircraft_dim_feet,
                                   takeoff_total_clear_50ft_obstacle_ft public.aircraft_dim_feet NOT NULL,
                                   v1_takeoff_decision_speed_ktas public.aircraft_speed_knots,
                                   vr_rotation_speed_ktas public.aircraft_speed_knots,
                                   v2_takeoff_safety_speed_ktas public.aircraft_speed_knots,

    -- Landing Field Requirements (Calculated at Max Landing Weight, Sea Level, ISA)
                                   landing_ground_roll_ft public.aircraft_dim_feet,
                                   landing_total_clear_50ft_obstacle_ft public.aircraft_dim_feet NOT NULL,
                                   vref_landing_approach_speed_ktas public.aircraft_speed_knots,

                                   created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                   updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                   CONSTRAINT chk_to_obstacle CHECK (takeoff_total_clear_50ft_obstacle_ft >= COALESCE(takeoff_ground_roll_ft, 0.00)),
                                   CONSTRAINT chk_ld_obstacle CHECK (landing_total_clear_50ft_obstacle_ft >= COALESCE(landing_ground_roll_ft, 0.00))
);

-- ----------------------------------------------------------------------------
-- 3. CRUISE FLIGHT ENVELOPE PROFILE MODELS
-- ----------------------------------------------------------------------------
CREATE TABLE cruise_envelopes (
                                  configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Maximum Performance Settings
                                  max_cruise_speed_ktas public.aircraft_speed_knots,
                                  max_cruise_fuel_flow_gph NUMERIC(5, 1),             -- Gallons per hour burn rate

    -- Long-Range Economy Performance Benchmarks
                                  economy_cruise_speed_ktas public.aircraft_speed_knots,
                                  economy_cruise_fuel_flow_gph NUMERIC(5, 1),
                                  economy_cruise_altitude_ft public.aircraft_dim_feet,

    -- Fleet Logistics Parameters
                                  max_range_nm INT NOT NULL,                          -- Maximum range with reserves
                                  ferry_range_nm INT,                                 -- Range stripped of payload with max auxiliary fuel
                                  service_ceiling_ft public.aircraft_dim_feet NOT NULL,
                                  combustor_time_to_ceiling_minutes SMALLINT,

                                  created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                  updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                  CONSTRAINT chk_range_positive CHECK (max_range_nm > 0),
                                  CONSTRAINT chk_ferry_range CHECK (COALESCE(ferry_range_nm, max_range_nm) >= max_range_nm)
);

-- ----------------------------------------------------------------------------
-- 4. CLIMB AND DESCENT PERFORMANCE PRIMITIVES
-- ----------------------------------------------------------------------------
CREATE TABLE climb_descent_profiles (
                                        configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,

    -- Initial Climb Profiles (Sea Level, MTOW, Flaps Up)
                                        max_rate_of_climb_fpm INT,                         -- Feet per minute initial climb
                                        single_engine_climb_rate_fpm INT,                  -- Critical multi-engine benchmark (Vyse)

    -- Aero Efficiencies
                                        best_glide_ratio_speed_ktas public.aircraft_speed_knots,
                                        best_glide_lift_to_drag_ratio NUMERIC(4, 1),        -- L/D Ratio (e.g., 9.2, 11.5)

                                        created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                        updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP()
);

-- Index optimizations for operational searches
CREATE INDEX idx_speeds_stall ON speed_limits(vs0_stall_flaps_down_ktas, vs1_stall_clean_ktas);
CREATE INDEX idx_field_takeoff ON field_performance(takeoff_total_clear_50ft_obstacle_ft);
CREATE INDEX idx_cruise_range ON cruise_envelopes(max_range_nm);
CREATE INDEX idx_cruise_speed ON cruise_envelopes(max_cruise_speed_ktas);

-- Triggers for record updates
CREATE TRIGGER trg_speed_limits_updated BEFORE UPDATE ON speed_limits FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_field_performance_updated BEFORE UPDATE ON field_performance FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_cruise_envelopes_updated BEFORE UPDATE ON cruise_envelopes FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();
CREATE TRIGGER trg_climb_descent_profiles_updated BEFORE UPDATE ON climb_descent_profiles FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();

COMMIT;