-- =============================================================================
-- Phase 1 validation: schema, extension, domain, and function existence.
-- =============================================================================

-- 1. All 16 namespaces present
SELECT nspname
FROM pg_namespace
WHERE nspname LIKE 'aircraft_%'
ORDER BY 1;
-- expect 16 rows: aircraft_auth, aircraft_compare, aircraft_core, aircraft_cert,
-- aircraft_geo, aircraft_ingest, aircraft_maint, aircraft_market,
-- aircraft_military, aircraft_org, aircraft_power, aircraft_prov,
-- aircraft_read, aircraft_ref, aircraft_specs, aircraft_systems
--
-- aircraft_auth arrives with migration 025. These statements report rather than
-- assert -- this file raises nothing -- so a stale roll-call here fails no gate
-- and has to be maintained by hand.

-- 2. pg_trgm installed in public
SELECT extname, extnamespace::regnamespace AS schema
FROM pg_extension
WHERE extname = 'pg_trgm';
-- expect 1 row: pg_trgm | public

-- 3. Domains present in aircraft_ref
SELECT typname
FROM pg_type
WHERE typnamespace = 'aircraft_ref'::regnamespace
  AND typtype = 'd'
ORDER BY 1;
-- expect: confidence_score, lookup_code, nonneg_numeric, slug_text, year_value

-- 4. Helper functions present
SELECT proname
FROM pg_proc
WHERE pronamespace = 'aircraft_ref'::regnamespace
ORDER BY 1;
-- expect: normalize_lookup_code, set_updated_at, slugify

-- 5. Functional smoke test
SELECT aircraft_ref.slugify('North American')            AS slug_example,
       aircraft_ref.normalize_lookup_code('Light Sport') AS code_example;
-- expect: 'north-american' | 'LIGHT_SPORT'