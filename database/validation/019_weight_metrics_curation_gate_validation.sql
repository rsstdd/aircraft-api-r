-- Validation for migration 019: weight metrics must be gated on curation the
-- same way performance metrics are.
DO $validation$
DECLARE
    view_definition TEXT;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_specs'
          AND table_name = 'weight_metrics'
          AND column_name = 'is_canonical'
          AND is_nullable = 'NO'
    ) THEN
        RAISE EXCEPTION 'weight_metrics.is_canonical must exist and be NOT NULL';
    END IF;

    IF to_regclass('aircraft_specs.uq_wm_canonical') IS NULL THEN
        RAISE EXCEPTION 'Missing uq_wm_canonical partial UNIQUE index';
    END IF;

    IF to_regclass('aircraft_specs.uq_weight_metrics_config') IS NOT NULL
       OR to_regclass('aircraft_specs.idx_weight_metrics_config') IS NULL THEN
        RAISE EXCEPTION
            'Weight configuration lookup must allow competing source values';
    END IF;

    -- The index must be partial on is_canonical; a plain UNIQUE would forbid a
    -- variant from holding more than one pending value per metric.
    IF NOT EXISTS (
        SELECT 1 FROM pg_indexes
        WHERE schemaname = 'aircraft_specs'
          AND indexname = 'uq_wm_canonical'
          AND indexdef ILIKE '%WHERE%is_canonical%'
    ) THEN
        RAISE EXCEPTION 'uq_wm_canonical must be partial on is_canonical';
    END IF;

    -- The whole point of the migration: the read model must not serve an
    -- uncurated weight.
    SELECT pg_get_viewdef('aircraft_read.mv_variant_search'::regclass, TRUE)
    INTO view_definition;
    IF position('weight_metrics.is_canonical' IN view_definition) = 0 THEN
        RAISE EXCEPTION 'mv_variant_search must filter weight metrics on is_canonical';
    END IF;

    -- Migration 017's rebuild and 019's rebuild both drop and recreate the
    -- matview; its identity and search indexes must survive both.
    IF to_regclass('aircraft_read.uq_mvs_variant') IS NULL
       OR to_regclass('aircraft_read.idx_mvs_fts') IS NULL THEN
        RAISE EXCEPTION 'Rebuilt mv_variant_search is missing its identity or search index';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM aircraft_specs.weight_metrics
        WHERE is_canonical
        GROUP BY variant_id, metric_type_code
        HAVING COUNT(*) > 1
    ) THEN
        RAISE EXCEPTION 'A variant has more than one canonical value for a weight metric';
    END IF;
END
$validation$;
