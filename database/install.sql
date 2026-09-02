-- Canonical dependency-aware installer for psql.
-- Run with: psql -X -v ON_ERROR_STOP=1 "$DATABASE_URL" -f database/install.sql
\set ON_ERROR_STOP on

-- Serialize installers for this database. The advisory lock is held by the
-- psql session across the per-migration transactions and is released
-- automatically if the session terminates because a migration fails.
SELECT pg_advisory_lock(hashtextextended(current_database() || ':aircraft-install', 0));

CREATE TABLE IF NOT EXISTS public.aircraft_schema_migrations (
    version TEXT PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
COMMENT ON TABLE public.aircraft_schema_migrations IS
    'Records successfully applied Aircraft Management Engine SQL migrations.';

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '001'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 001: 001_extensions_schemas_domains_triggers.sql'
\ir migrations/001_extensions_schemas_domains_triggers.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('001');
\else
\echo 'Skipping applied migration 001'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '002'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 002: 002_core_reference_tables.sql'
\ir migrations/002_core_reference_tables.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('002');
\else
\echo 'Skipping applied migration 002'
\endif

-- Later schema phases reference these canonical lookup rows.
\ir seeds/001_reference_units.sql
\ir seeds/002_lookup_seed_data.sql

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '003'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 003: 003_geography_operators_organizations.sql'
\ir migrations/003_geography_operators_organizations.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('003');
\else
\echo 'Skipping applied migration 003'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '004'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 004: 004_aircraft_identity_taxonomy.sql'
\ir migrations/004_aircraft_identity_taxonomy.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('004');
\else
\echo 'Skipping applied migration 004'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '005'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 005: 005_certification_operating_approvals.sql'
\ir migrations/005_certification_operating_approvals.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('005');
\else
\echo 'Skipping applied migration 005'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '006'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 006: 006_dimensions_cabin_cargo_hangar_fit.sql'
\ir migrations/006_dimensions_cabin_cargo_hangar_fit.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('006');
\else
\echo 'Skipping applied migration 006'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '007'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 007: 007_weight_balance_payload_loading.sql'
\ir migrations/007_weight_balance_payload_loading.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('007');
\else
\echo 'Skipping applied migration 007'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '008'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 008: 008_performance_metrics_conditions.sql'
\ir migrations/008_performance_metrics_conditions.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('008');
\else
\echo 'Skipping applied migration 008'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '009'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 009: 009_propulsion_engines_rotors_stcs.sql'
\ir migrations/009_propulsion_engines_rotors_stcs.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('009');
\else
\echo 'Skipping applied migration 009'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '010'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 010: 010_avionics_equipment_systems.sql'
\ir migrations/010_avionics_equipment_systems.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('010');
\else
\echo 'Skipping applied migration 010'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '011'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 011: 011_military_sensors_stores_loadouts.sql'
\ir migrations/011_military_sensors_stores_loadouts.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('011');
\else
\echo 'Skipping applied migration 011'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '012'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 012: 012_ownership_cost_valuation_market.sql'
\ir migrations/012_ownership_cost_valuation_market.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('012');
\else
\echo 'Skipping applied migration 012'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '013'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 013: 013_maintenance_reliability_supportability.sql'
\ir migrations/013_maintenance_reliability_supportability.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('013');
\else
\echo 'Skipping applied migration 013'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '014'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 014: 014_sources_provenance_curation_audit.sql'
\ir migrations/014_sources_provenance_curation_audit.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('014');
\else
\echo 'Skipping applied migration 014'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '015'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 015: 015_mission_profiles_comparison_scoring.sql'
\ir migrations/015_mission_profiles_comparison_scoring.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('015');
\else
\echo 'Skipping applied migration 015'
\endif

\ir seeds/003_mission_profile_seed_data.sql

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '016'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 016: 016_read_models_views_indexes.sql'
\ir migrations/016_read_models_views_indexes.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('016');
\else
\echo 'Skipping applied migration 016'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '017'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 017: 017_rust_ingestion_adapter.sql'
\ir migrations/017_rust_ingestion_adapter.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('017');
\else
\echo 'Skipping applied migration 017'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '018'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 018: 018_staged_aircraft_variant_fk.sql'
\ir migrations/018_staged_aircraft_variant_fk.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('018');
\else
\echo 'Skipping applied migration 018'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '019'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 019: 019_weight_metrics_curation_gate.sql'
\ir migrations/019_weight_metrics_curation_gate.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('019');
\else
\echo 'Skipping applied migration 019'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '020'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 020: 020_market_curation_gate.sql'
\ir migrations/020_market_curation_gate.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('020');
\else
\echo 'Skipping applied migration 020'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '021'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 021: 021_validate_measurement_assertion_foreign_keys.sql'
\ir migrations/021_validate_measurement_assertion_foreign_keys.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('021');
\else
\echo 'Skipping applied migration 021'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '022'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 022: 022_read_model_refresh_requests.sql'
\ir migrations/022_read_model_refresh_requests.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('022');
\else
\echo 'Skipping applied migration 022'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '023'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 023: 023_backfill_ingestion_identity_projections.sql'
\ir migrations/023_backfill_ingestion_identity_projections.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('023');
\else
\echo 'Skipping applied migration 023'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '024'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 024: 024_promote_existing_manufacturer_links.sql'
\ir migrations/024_promote_existing_manufacturer_links.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('024');
\else
\echo 'Skipping applied migration 024'
\endif

SELECT NOT EXISTS (
    SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '025'
) AS apply_migration \gset
\if :apply_migration
\echo 'Applying migration 025: 025_authentication_schema.sql'
\ir migrations/025_authentication_schema.sql
INSERT INTO public.aircraft_schema_migrations(version) VALUES ('025');
\else
\echo 'Skipping applied migration 025'
\endif

-- The authentication scope vocabulary, seeded after the migration that creates
-- the table it fills.
\ir seeds/004_authentication_seed_data.sql


\echo 'Database schema and canonical seed installation complete.'
SELECT pg_advisory_unlock(hashtextextended(current_database() || ':aircraft-install', 0));
