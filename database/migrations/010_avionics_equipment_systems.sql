-- ============================================================================
-- FILE: 010_market_valuation_operating_costs.sql
-- DESCRIPTION: Establishes retail trends, valuation models, and operational costs.
-- ============================================================================

BEGIN;

CREATE SCHEMA aircraft_market;
SET search_path TO aircraft_market, aircraft_core, aircraft_ref, public;

-- ----------------------------------------------------------------------------
-- 1. FACTORY BASE RESALE LISTINGS
-- ----------------------------------------------------------------------------
CREATE TABLE factory_prices (
                                id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                configuration_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,
                                model_year SMALLINT NOT NULL,
                                base_list_price NUMERIC(14, 2) NOT NULL,
                                currency_code CHAR(3) NOT NULL REFERENCES aircraft_ref.currency_definitions(code) ON DELETE RESTRICT,
                                created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                CONSTRAINT uq_config_year_currency UNIQUE (configuration_id, model_year, currency_code),
                                CONSTRAINT chk_price_positive CHECK (base_list_price > 0.00),
                                CONSTRAINT chk_price_year CHECK (model_year > 1900 AND model_year < 2100)
);

-- ----------------------------------------------------------------------------
-- 2. HISTORIC MARKET VALUATION CURVES
-- ----------------------------------------------------------------------------
CREATE TABLE historical_valuations (
                                       id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                       configuration_id BIGINT NOT NULL REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,
                                       valuation_date DATE NOT NULL DEFAULT CURRENT_DATE,
                                       market_condition_tier TEXT NOT NULL DEFAULT 'AVERAGE', -- 'EXCELLENT', 'AVERAGE', 'SALVAGE'
                                       estimated_retail_value NUMERIC(14, 2) NOT NULL,
                                       currency_code CHAR(3) NOT NULL REFERENCES aircraft_ref.currency_definitions(code) ON DELETE RESTRICT,
                                       source_dataset_reference TEXT,                        -- 'Vref Mid-Year Update', 'Bluebook'
                                       created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                       CONSTRAINT chk_valuation_positive CHECK (estimated_retail_value > 0.00),
                                       CONSTRAINT chk_market_tier CHECK (market_condition_tier IN ('NEWHANGER', 'EXCELLENT', 'AVERAGE', 'FAIR', 'SALVAGE'))
);

CREATE INDEX idx_market_val_lookup ON historical_valuations(configuration_id, valuation_date);

-- ----------------------------------------------------------------------------
-- 3. ESTIMATED HOURLY OPERATING COSTS (Direct vs Indirect Variables)
-- ----------------------------------------------------------------------------
CREATE TABLE operating_cost_estimates (
                                          configuration_id BIGINT PRIMARY KEY REFERENCES aircraft_core.aircraft_configurations(id) ON DELETE CASCADE,
                                          currency_code CHAR(3) NOT NULL REFERENCES aircraft_ref.currency_definitions(code) ON DELETE RESTRICT,

    -- Variable Costs per Block Flight Hour
                                          estimated_fuel_cost_per_hour NUMERIC(8, 2) NOT NULL DEFAULT 0.00,
                                          estimated_maintenance_labor_per_hour NUMERIC(8, 2) NOT NULL DEFAULT 0.00,
                                          estimated_parts_engine_reserve_per_hour NUMERIC(8, 2) NOT NULL DEFAULT 0.00,

    -- Fixed Annualized Overhead Mapped to Hourly Baselines (Assumes standard 100hr/yr utilization)
                                          estimated_insurance_annual NUMERIC(10, 2) NOT NULL DEFAULT 0.00,
                                          estimated_hangar_storage_annual NUMERIC(10, 2) NOT NULL DEFAULT 0.00,

    -- Programmatic Generated Columns for Combined Fleet Ledger Auditing
                                          total_variable_cost_per_hour NUMERIC(10, 2) GENERATED ALWAYS AS (
                                              estimated_fuel_cost_per_hour + estimated_maintenance_labor_per_hour + estimated_parts_engine_reserve_per_hour
                                              ) STORED,

                                          created_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),
                                          updated_at public.aircraft_timestamp NOT NULL DEFAULT CLOCK_TIMESTAMP(),

                                          CONSTRAINT chk_costs_positive CHECK (
                                              estimated_fuel_cost_per_hour >= 0.00
                                                  AND estimated_maintenance_labor_per_hour >= 0.00
                                                  AND estimated_parts_engine_reserve_per_hour >= 0.00
                                              )
);

-- Triggers for record updates
CREATE TRIGGER trg_operating_costs_updated BEFORE UPDATE ON operating_cost_estimates FOR EACH ROW EXECUTE FUNCTION public.fn_update_timestamp_column();

COMMIT;