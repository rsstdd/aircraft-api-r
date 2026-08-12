-- Reconcile local databases created before aircraft_schema_migrations existed.
--
-- This file is intentionally run only by the local Compose recipe. Production
-- databases must be baselined deliberately rather than adopted automatically.
-- The original migration files are transactional, so a legacy phase can only
-- be adopted when every object from that phase is present.
\set ON_ERROR_STOP on

BEGIN;

SELECT pg_advisory_xact_lock(
    hashtextextended(current_database() || ':aircraft-install', 0)
);

CREATE TABLE IF NOT EXISTS public.aircraft_schema_migrations (
    version TEXT PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DO $legacy_reconciliation$
DECLARE
    phase_1_aircraft_objects INTEGER;
    phase_1_verified_objects INTEGER;
    phase_2_tables INTEGER;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '001'
    ) THEN
        SELECT count(*)
        INTO phase_1_aircraft_objects
        FROM unnest(ARRAY[
            'aircraft_ref', 'aircraft_geo', 'aircraft_org', 'aircraft_core',
            'aircraft_cert', 'aircraft_specs', 'aircraft_power',
            'aircraft_systems', 'aircraft_military', 'aircraft_market',
            'aircraft_maint', 'aircraft_prov', 'aircraft_compare',
            'aircraft_ingest', 'aircraft_read'
        ]) AS expected(schema_name)
        WHERE to_regnamespace(schema_name) IS NOT NULL;

        phase_1_aircraft_objects := phase_1_aircraft_objects
            + CASE WHEN to_regtype('aircraft_ref.nonneg_numeric') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regtype('aircraft_ref.confidence_score') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regtype('aircraft_ref.year_value') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regtype('aircraft_ref.slug_text') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regtype('aircraft_ref.lookup_code') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regprocedure('aircraft_ref.set_updated_at()') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regprocedure('aircraft_ref.slugify(text)') IS NOT NULL THEN 1 ELSE 0 END
            + CASE WHEN to_regprocedure('aircraft_ref.normalize_lookup_code(text)') IS NOT NULL THEN 1 ELSE 0 END;

        IF phase_1_aircraft_objects > 0 THEN
            SELECT phase_1_aircraft_objects
                + CASE WHEN EXISTS (
                    SELECT 1
                    FROM pg_extension extension
                    JOIN pg_namespace namespace
                      ON namespace.oid = extension.extnamespace
                    WHERE extension.extname = 'pg_trgm'
                      AND namespace.nspname = 'public'
                ) THEN 1 ELSE 0 END
            INTO phase_1_verified_objects;

            IF phase_1_verified_objects <> 24 THEN
                RAISE EXCEPTION USING
                    MESSAGE = format(
                        'Cannot adopt legacy migration 001: found %s of 24 required objects',
                        phase_1_verified_objects
                    ),
                    HINT = 'Restore the missing Phase 1 objects or rebuild only a disposable local database with just db-rebuild.';
            END IF;

            CREATE OR REPLACE FUNCTION aircraft_ref.text_array_to_string(
                input TEXT[], delimiter TEXT
            )
            RETURNS TEXT
            LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $function$
                SELECT pg_catalog.array_to_string(input, delimiter);
            $function$;

            COMMENT ON FUNCTION aircraft_ref.text_array_to_string(TEXT[], TEXT) IS
                'Immutable, text-array-specific wrapper used by generated search-vector columns. PostgreSQL marks the polymorphic array_to_string function STABLE even though text-array rendering is deterministic.';

            INSERT INTO public.aircraft_schema_migrations(version) VALUES ('001');
            RAISE NOTICE 'Adopted verified legacy migration 001';
        END IF;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '002'
    ) THEN
        SELECT count(*)
        INTO phase_2_tables
        FROM unnest(ARRAY[
            'unit_categories', 'measurement_units', 'aircraft_roles',
            'service_statuses', 'variant_types', 'landing_gear_types',
            'propulsion_categories', 'fuel_types', 'dimension_metric_types',
            'weight_metric_types', 'performance_metric_types',
            'certification_authorities', 'airworthiness_categories',
            'pilot_certificate_types', 'operating_approval_types',
            'military_mission_types', 'weapon_categories',
            'hardpoint_position_types', 'stores_types', 'currencies',
            'cost_item_types', 'aircraft_condition_grades', 'ad_types',
            'sb_compliance_statuses', 'availability_grades', 'source_types',
            'source_reliability_grades', 'curation_flag_statuses',
            'curation_entity_types', 'assertion_statuses',
            'mission_profile_types', 'comparison_criterion_types',
            'organization_types', 'org_relationship_types',
            'systems_categories', 'equipment_provision_types'
        ]) AS expected(table_name)
        WHERE to_regclass(format('aircraft_ref.%I', table_name)) IS NOT NULL;

        IF phase_2_tables = 36 THEN
            IF NOT EXISTS (
                SELECT 1 FROM public.aircraft_schema_migrations WHERE version = '001'
            ) THEN
                RAISE EXCEPTION 'Cannot adopt legacy migration 002 before migration 001';
            END IF;

            INSERT INTO public.aircraft_schema_migrations(version) VALUES ('002');
            RAISE NOTICE 'Adopted verified legacy migration 002';
        ELSIF phase_2_tables > 0 THEN
            RAISE EXCEPTION USING
                MESSAGE = format(
                    'Cannot adopt legacy migration 002: found %s of 36 required tables',
                    phase_2_tables
                ),
                HINT = 'Restore the missing Phase 2 tables or rebuild only a disposable local database with just db-rebuild.';
        END IF;
    END IF;
END
$legacy_reconciliation$;

COMMIT;
