-- ============================================================================
-- FILE: 009_propulsion_avionics_systems.sql
-- DESCRIPTION: Establishes propulsion architectures and avionics system catalogs.
-- ============================================================================

BEGIN;

-- ----------------------------------------------------------------------------
-- 1. PROPULSION CATALOG AND ENGINE CONFIGURATIONS
-- ----------------------------------------------------------------------------
CREATE SCHEMA aircraft_prop;
SET search_path TO aircraft_prop, aircraft_core, aircraft_ref, aircraft_org, public;

CREATE TABLE engine_models (
                               id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                               manufacturer_id BIGINT NOT NULL REFERENCES aircraft_org.organizations(id) ON DELETE RESTRICT,
                               name TEXT NOT NULL,                  -- 'IO-360-M1A', 'PT6A-67P', 'CFM56-7B27'
                               slug public.aircraft_slug NOT NULL UNIQUE,
                               propulsion_category_code TEXT NOT NULL REFERENCES aircraft_ref.propulsion_categories(code) ON DELETE RESTRICT,

    -- Power Matrix Limits (Populated contextually per engine type)
                               rated_horsepower NUMERIC(6, 2),      -- Piston engines (HP)
                               rated_shaft_horsepower NUMERIC(6, 2),-- Turboprops / Turboshafts (SHP)
                               rated_static_thrust_lbf public.aircraft_mass_lbs, -- Turbofans / Turbojets (LBF)

    -- Physical footprint attributes
                               dry_weight_lbs public.aircraft_mass_lbs,
                               time_between_overhauls_hours INT,    -- TBO baseline
                               created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                               updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP()
);

CREATE TABLE propulsion_installations (
                                          id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                          configuration_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,
                                          engine_model_id BIGINT NOT NULL REFERENCES engine_models(id) ON DELETE RESTRICT,
                                          engine_count SMALLINT NOT NULL DEFAULT 1,

    -- Installation Traits
                                          is_supplemental_apu_boost BOOLEAN NOT NULL DEFAULT FALSE,
                                          propeller_model TEXT,                -- 'Hartzell HC-C2YR-1B'
                                          propeller_blades_count SMALLINT,
                                          is_constant_speed_propeller BOOLEAN NOT NULL DEFAULT FALSE,
                                          is_featherable_propeller BOOLEAN NOT NULL DEFAULT FALSE,

                                          created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                          CONSTRAINT uq_config_engine UNIQUE (configuration_id, engine_model_id),
                                          CONSTRAINT chk_engine_count CHECK (engine_count > 0)
);

CREATE INDEX idx_engines_cat ON engine_models(propulsion_category_code);
CREATE INDEX idx_prop_config ON propulsion_installations(configuration_id);

-- ----------------------------------------------------------------------------
-- 2. AVIONICS EQUIPMENT AND FLIGHT DECKS
-- ----------------------------------------------------------------------------
CREATE SCHEMA aircraft_avionics;
SET search_path TO aircraft_avionics, aircraft_core, aircraft_org, public;

CREATE TABLE avionics_suites (
                                 id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                 manufacturer_id BIGINT NOT NULL REFERENCES aircraft_org.organizations(id) ON DELETE RESTRICT,
                                 name TEXT NOT NULL,                  -- 'G1000 NXi', 'Pro Line 21', 'Primus Epic'
                                 slug public.aircraft_slug NOT NULL UNIQUE,
                                 description TEXT,
                                 created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP()
);

CREATE TABLE avionics_installations (
                                        configuration_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,
                                        avionics_suite_id BIGINT NOT NULL REFERENCES avionics_suites(id) ON DELETE RESTRICT,
                                        is_glass_cockpit BOOLEAN NOT NULL DEFAULT FALSE,
                                        has_autopilot BOOLEAN NOT NULL DEFAULT FALSE,
                                        autopilot_model TEXT,                -- 'Garmin GFC 700'
                                        has_adsb_out BOOLEAN NOT NULL DEFAULT TRUE,
                                        has_tcas BOOLEAN NOT NULL DEFAULT FALSE,
                                        PRIMARY KEY (configuration_id, avionics_suite_id)
);

CREATE INDEX idx_avionics_config ON avionics_installations(configuration_id);

-- Triggers for record updates
CREATE TRIGGER trg_engine_models_updated BEFORE UPDATE ON engine_models FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();

COMMIT;