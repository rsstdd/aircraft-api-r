-- =============================================================================
-- File: database/migrations/019_weight_metrics_curation_gate.sql
-- Phase 19: Gate weight metrics on curation, as performance metrics already are.
--
-- aircraft_specs.performance_metrics carries is_canonical, guarded by the
-- uq_perf_canonical partial UNIQUE index, and aircraft_read.mv_variant_search
-- filters on it. aircraft_specs.weight_metrics had no such column, and the
-- matview's weight aggregate filtered only on `configuration IS NULL`. Empty
-- weight, MTOW, and usable fuel capacity therefore reached the read model the
-- moment they were ingested, with no curation step -- while every other
-- ingested measurement correctly waited for one.
--
-- Migration 007 allowed only one row per (variant, metric_type, configuration).
-- That prevented a second source's conflicting value from being stored at all.
-- This migration replaces that uniqueness rule with the canonical partial
-- UNIQUE index: any number of source values may remain pending, while at most
-- one value per variant and metric is published.
--
-- Existing rows with configuration IS NULL are backfilled TRUE because those
-- are the only rows the pre-migration read model served. Configured siblings
-- stay pending, which keeps the new canonical UNIQUE index upgrade-safe.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

ALTER TABLE aircraft_specs.weight_metrics
    ADD COLUMN IF NOT EXISTS is_canonical BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN aircraft_specs.weight_metrics.is_canonical IS
    'TRUE = this row is served through aircraft_read.mv_variant_search. '
    'The Rust ingestion adapter writes FALSE and leaves promotion to curation; '
    'the legacy SQL loader canonicalizes its own output. At most one TRUE per '
    '(variant, metric_type), enforced by uq_wm_canonical.';

-- Preserve exactly what the read model currently serves. Migration 016 filters
-- weight metrics on configuration IS NULL.
UPDATE aircraft_specs.weight_metrics
SET    is_canonical = TRUE
WHERE  is_canonical = FALSE
  AND  configuration IS NULL;

-- The old unique index must disappear atomically with its non-unique
-- replacement and the new canonical constraint, so CONCURRENTLY is unavailable.
-- squawk-ignore require-concurrent-index-deletion
DROP INDEX aircraft_specs.uq_weight_metrics_config;

-- Multiple sources may report different values under the same configuration.
-- Keep the former lookup shape without using it to discard evidence.
-- squawk-ignore require-concurrent-index-creation
CREATE INDEX idx_weight_metrics_config
    ON aircraft_specs.weight_metrics
        (variant_id, metric_type_code, COALESCE(configuration, ''));

-- Mirrors uq_perf_canonical (migration 008): one canonical row per
-- (variant, metric_type), so two configurations cannot both be served.
-- This index must commit atomically with the curation gate and read-model
-- rebuild below; a concurrent index cannot run in this transaction.
-- squawk-ignore require-concurrent-index-creation
CREATE UNIQUE INDEX IF NOT EXISTS uq_wm_canonical
    ON aircraft_specs.weight_metrics (variant_id, metric_type_code)
    WHERE is_canonical;

COMMENT ON COLUMN aircraft_specs.performance_metrics.is_canonical IS
    'TRUE = this row is the designated cross-fleet comparison value '
    'for this (variant, metric_type) pair. At most one TRUE per pair '
    '(enforced by uq_perf_canonical partial UNIQUE index). '
    'The Rust ingestion adapter writes every value is_canonical = FALSE; '
    'curation must explicitly promote one. The legacy SQL loader still '
    'canonicalizes the first value it sees.';

-- -----------------------------------------------------------------------------
-- Rebuild mv_variant_search so its weight aggregate respects the new gate.
--
-- The definition is read back from the catalog rather than restated, because
-- migration 017 already rebuilt this matview in place and a literal copy here
-- would silently diverge from whatever 017 produced. Index definitions are
-- captured before the DROP and replayed, since DROP MATERIALIZED VIEW takes the
-- indexes with it.
-- -----------------------------------------------------------------------------
DO $migration$
DECLARE
    view_definition   TEXT;
    index_definitions TEXT[];
    statement         TEXT;
BEGIN
    SELECT pg_get_viewdef('aircraft_read.mv_variant_search'::regclass, TRUE)
    INTO view_definition;
    view_definition := regexp_replace(view_definition, ';[[:space:]]*$', '');

    IF position('weight_metrics.is_canonical' IN view_definition) > 0 THEN
        RAISE NOTICE 'mv_variant_search already gates weight metrics; leaving it alone';
        RETURN;
    END IF;

    IF position('weight_metrics.configuration IS NULL' IN view_definition) = 0 THEN
        RAISE EXCEPTION
            'mv_variant_search does not contain the expected weight predicate; '
            'refusing to rebuild it into an ungated view';
    END IF;

    view_definition := replace(
        view_definition,
        'weight_metrics.configuration IS NULL',
        'weight_metrics.configuration IS NULL AND weight_metrics.is_canonical'
    );

    SELECT array_agg(indexdef)
    INTO index_definitions
    FROM pg_indexes
    WHERE schemaname = 'aircraft_read'
      AND tablename = 'mv_variant_search';

    DROP MATERIALIZED VIEW aircraft_read.mv_variant_search;
    EXECUTE
        'CREATE MATERIALIZED VIEW aircraft_read.mv_variant_search AS '
        || view_definition
        || ' WITH NO DATA';

    FOREACH statement IN ARRAY COALESCE(index_definitions, ARRAY[]::TEXT[]) LOOP
        EXECUTE statement;
    END LOOP;
END
$migration$;

COMMIT;
