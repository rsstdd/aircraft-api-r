-- =============================================================================
-- File: database/migrations/001_extensions_schemas_domains_triggers.sql
-- Phase 1: extensions, schema namespaces, cross-cutting domains, shared
-- trigger/helper functions. No data-bearing tables are created in this file.
-- =============================================================================

BEGIN;

-- -----------------------------------------------------------------------------
-- Extensions
-- -----------------------------------------------------------------------------
-- pg_trgm is installed into 'public' deliberately: its operator classes
-- (gin_trgm_ops) are then resolvable via the default search_path from every
-- namespace without per-session search_path manipulation. This is the
-- conventional placement and the only extension this project requires.
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

-- -----------------------------------------------------------------------------
-- Schema namespaces
-- -----------------------------------------------------------------------------
CREATE SCHEMA IF NOT EXISTS aircraft_ref;
COMMENT ON SCHEMA aircraft_ref IS
  'Lookup/reference tables: units, currencies, categories, statuses, and other extensible enumerations.';

CREATE SCHEMA IF NOT EXISTS aircraft_geo;
COMMENT ON SCHEMA aircraft_geo IS
  'Countries, regions, and geography-lite reference data for origin and operator faceting.';

CREATE SCHEMA IF NOT EXISTS aircraft_org;
COMMENT ON SCHEMA aircraft_org IS
  'Manufacturers, operators, organizations, licenses, and ownership relationships.';

CREATE SCHEMA IF NOT EXISTS aircraft_core;
COMMENT ON SCHEMA aircraft_core IS
  'Aircraft families, models, variants, aliases, roles, and lifecycle data: the identity backbone.';

CREATE SCHEMA IF NOT EXISTS aircraft_cert;
COMMENT ON SCHEMA aircraft_cert IS
  'Certification, airworthiness, operating approvals, and pilot requirements.';

CREATE SCHEMA IF NOT EXISTS aircraft_specs;
COMMENT ON SCHEMA aircraft_specs IS
  'Dimensions, weights, performance, loading, cabin, cargo, and hangar-fit data.';

CREATE SCHEMA IF NOT EXISTS aircraft_power;
COMMENT ON SCHEMA aircraft_power IS
  'Engines, propulsion, propellers, rotors, APU, fuel, and STC/conversion data.';

CREATE SCHEMA IF NOT EXISTS aircraft_systems;
COMMENT ON SCHEMA aircraft_systems IS
  'Avionics, equipment, systems, options, and retrofit data.';

CREATE SCHEMA IF NOT EXISTS aircraft_military;
COMMENT ON SCHEMA aircraft_military IS
  'Public, unclassified military sensors, weapons, stores, hardpoints, and representative loadout reference data. Encyclopedia-style comparison only.';

CREATE SCHEMA IF NOT EXISTS aircraft_market;
COMMENT ON SCHEMA aircraft_market IS
  'Ownership cost, valuation, market snapshots, and buyer-research data.';

CREATE SCHEMA IF NOT EXISTS aircraft_maint;
COMMENT ON SCHEMA aircraft_maint IS
  'Maintenance, reliability, ADs, service bulletins, lifecycle, and supportability.';

CREATE SCHEMA IF NOT EXISTS aircraft_prov;
COMMENT ON SCHEMA aircraft_prov IS
  'Source catalog, raw documents, field-level source assertions, provenance, curation, conflicts, and audit history.';

CREATE SCHEMA IF NOT EXISTS aircraft_compare;
COMMENT ON SCHEMA aircraft_compare IS
  'Mission profiles, comparison criteria, scoring, and suitability data.';

CREATE SCHEMA IF NOT EXISTS aircraft_ingest;
COMMENT ON SCHEMA aircraft_ingest IS
  'Staging tables, import runs, and seed-data import helpers. Transient/ETL-facing.';

CREATE SCHEMA IF NOT EXISTS aircraft_read;
COMMENT ON SCHEMA aircraft_read IS
  'Denormalized read models, views, materialized views, and search surfaces.';

-- -----------------------------------------------------------------------------
-- Cross-cutting domains
-- -----------------------------------------------------------------------------

-- Non-negative numeric: weights, speeds, distances, costs, capacities, etc.
-- NULL remains permitted (CHECK on a domain is vacuously true for NULL inputs).
CREATE DOMAIN aircraft_ref.nonneg_numeric AS NUMERIC
    CHECK (VALUE >= 0);
COMMENT ON DOMAIN aircraft_ref.nonneg_numeric IS
  'Non-negative numeric value; NULL permitted. Used for any physical or monetary quantity that cannot be negative.';

-- Provenance confidence score in [0,1].
CREATE DOMAIN aircraft_ref.confidence_score AS NUMERIC(3,2)
    CHECK (VALUE BETWEEN 0 AND 1);
COMMENT ON DOMAIN aircraft_ref.confidence_score IS
  'Provenance confidence score in [0,1], where 1.00 indicates a fully trusted, corroborated value. Used throughout aircraft_prov.';

-- Calendar year for production/service ranges.
CREATE DOMAIN aircraft_ref.year_value AS SMALLINT
    CHECK (VALUE BETWEEN 1900 AND 2100);
COMMENT ON DOMAIN aircraft_ref.year_value IS
  'Calendar year for production/service dates, constrained to a plausible aviation-history range (1900-2100).';

-- URL-routing slug.
CREATE DOMAIN aircraft_ref.slug_text AS TEXT
    CHECK (VALUE ~ '^[a-z0-9]+(-[a-z0-9]+)*$');
COMMENT ON DOMAIN aircraft_ref.slug_text IS
  'Lowercase, hyphen-delimited URL slug (e.g., "north-american"). Used for routing-friendly identifiers on families, models, manufacturers, etc.';

-- Stable machine code for lookup-table rows.
CREATE DOMAIN aircraft_ref.lookup_code AS TEXT
    CHECK (VALUE ~ '^[A-Z][A-Z0-9_]*$');
COMMENT ON DOMAIN aircraft_ref.lookup_code IS
  'Stable, upper-snake-case machine code for lookup-table rows (e.g., "RETRACTABLE_TRICYCLE"). The human-readable label lives in a separate column on the lookup table.';

-- -----------------------------------------------------------------------------
-- Shared trigger and helper functions
-- -----------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION aircraft_ref.set_updated_at()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.updated_at := now();
RETURN NEW;
END;
$$;
COMMENT ON FUNCTION aircraft_ref.set_updated_at() IS
  'Generic BEFORE UPDATE trigger: stamps updated_at = now() on every row update. Attach to any table with an updated_at TIMESTAMPTZ column.';

CREATE OR REPLACE FUNCTION aircraft_ref.slugify(input text)
RETURNS text
LANGUAGE sql IMMUTABLE AS $$
SELECT trim(both '-' FROM regexp_replace(lower(coalesce(input, '')), '[^a-z0-9]+', '-', 'g'));
$$;
COMMENT ON FUNCTION aircraft_ref.slugify(text) IS
  'Converts free text to a lowercase hyphen-delimited slug. For non-empty, alphanumeric-containing input the result satisfies aircraft_ref.slug_text.';

CREATE OR REPLACE FUNCTION aircraft_ref.normalize_lookup_code(input text)
RETURNS text
LANGUAGE sql IMMUTABLE AS $$
SELECT upper(trim(both '_' FROM regexp_replace(trim(coalesce(input, '')), '[^A-Za-z0-9]+', '_', 'g')));
$$;
COMMENT ON FUNCTION aircraft_ref.normalize_lookup_code(text) IS
  'Converts free text (e.g., a source category string such as "Light Sport") to an upper-snake-case candidate for aircraft_ref.lookup_code (e.g., "LIGHT_SPORT"). Caller must ensure the result begins with a letter before use as a code value.';

COMMIT;