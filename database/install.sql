-- -- # Phase 1 — infrastructure
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/001_extensions_schemas_domains_triggers.sql

-- # Phase 2 — reference/lookup tables + seed data
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/002_core_reference_tables.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f seeds/002_lookup_seed_data.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f seeds/001_reference_units.sql

-- # Phase 3 — geography, organizations
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/003_geography_operators_organizations.sql

-- # Phase 4 — aircraft identity/taxonomy
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/004_aircraft_identity_taxonomy.sql

-- # Phase 5 — certification/airworthiness
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/005_certification_operating_approvals.sql

-- # Phase 6 — dimensions/cabin/cargo
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/006_dimensions_cabin_cargo_hangar_fit.sql

-- # Phase 7 — weight/balance/payload
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/007_weight_balance_payload_loading.sql

-- # Phase 8 — performance metrics
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/008_performance_metrics_conditions.sql

-- # Phase 9 — propulsion/engines
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/009_propulsion_engines_rotors_stcs.sql

-- # Phase 10 — avionics/systems
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/010_avionics_equipment_systems.sql

-- # Phase 11 — military reference data
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/011_military_sensors_stores_loadouts.sql

-- # Phase 12 — ownership cost/valuation
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/012_ownership_cost_valuation_market.sql

-- # Phase 13 — maintenance/reliability
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/013_maintenance_reliability_supportability.sql

-- # Phase 14 — provenance/curation
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/014_sources_provenance_curation_audit.sql

-- # Phase 15 — mission profiles/comparison
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/015_mission_profiles_comparison_scoring.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f seeds/003_mission_profile_seed_data.sql

-- # Phase 16 — read models/views/indexes
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/016_read_models_views_indexes.sql

-- # Phase 17 — JSON seed staging + ingestion
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/901_seed_data_staging.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -v seed_json_path="'/absolute/path/to/aircraft_seed.json'" -f migrations/902_server_side_json_ingestion.sql

-- # Phase 18 — example queries (delivered as docs + executable smoke tests)
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f validation/002_comparison_query_smoke_tests.sql

-- # Phase 19 — validation
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f validation/001_integrity_checks.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f validation/003_seed_ingestion_validation.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f migrations/903_post_bootstrap_validation.sql
