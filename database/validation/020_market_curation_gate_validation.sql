-- Validation for migration 020: market data must be gated on curation the same
-- way measurements are.
DO $validation$
DECLARE
    cost_definition   TEXT;
    search_definition TEXT;
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_specs'
          AND table_name = 'performance_metrics'
          AND column_name = 'source_assertion_id'
    ) OR NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_specs'
          AND table_name = 'weight_metrics'
          AND column_name = 'source_assertion_id'
    ) THEN
        RAISE EXCEPTION 'Measurement rows must identify their exact source assertion';
    END IF;

    IF to_regclass('aircraft_specs.uq_pm_source_assertion') IS NULL
       OR to_regclass('aircraft_specs.uq_wm_source_assertion') IS NULL THEN
        RAISE EXCEPTION 'Missing measurement-to-assertion uniqueness indexes';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_pm_source_assertion'
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_wm_source_assertion'
    ) THEN
        RAISE EXCEPTION 'Missing measurement-to-assertion foreign keys';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_market' AND table_name = 'valuations'
          AND column_name = 'is_canonical' AND is_nullable = 'NO'
    ) OR NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_market' AND table_name = 'cost_snapshots'
          AND column_name = 'is_canonical' AND is_nullable = 'NO'
    ) THEN
        RAISE EXCEPTION 'market tables must carry a NOT NULL is_canonical column';
    END IF;

    IF to_regclass('aircraft_market.uq_val_canonical') IS NULL
       OR to_regclass('aircraft_market.uq_cs_canonical') IS NULL THEN
        RAISE EXCEPTION 'Missing market canonical partial UNIQUE indexes';
    END IF;

    -- The read model must not serve an uncurated price or cost. mv_variant_search
    -- reads valuations directly in a LATERAL, so gating the view alone is not
    -- enough; both predicates have to be present.
    SELECT pg_get_viewdef('aircraft_read.mv_ownership_cost_summary'::regclass, TRUE)
    INTO cost_definition;
    SELECT pg_get_viewdef('aircraft_read.mv_variant_search'::regclass, TRUE)
    INTO search_definition;

    IF position('cost_snapshots.is_canonical' IN cost_definition) = 0 THEN
        RAISE EXCEPTION 'mv_ownership_cost_summary must filter cost snapshots on is_canonical';
    END IF;
    IF position('valuations.is_canonical' IN search_definition) = 0 THEN
        RAISE EXCEPTION 'mv_variant_search must filter valuations on is_canonical';
    END IF;

    IF pg_get_viewdef('aircraft_read.v_current_valuation'::regclass, TRUE)
       NOT ILIKE '%is_canonical%' THEN
        RAISE EXCEPTION 'v_current_valuation must filter on is_canonical';
    END IF;

    -- Both matviews are dropped and recreated by this migration; their identity
    -- indexes must survive.
    IF to_regclass('aircraft_read.uq_mvs_variant') IS NULL
       OR to_regclass('aircraft_read.uq_ocs_variant') IS NULL THEN
        RAISE EXCEPTION 'A rebuilt read model is missing its identity index';
    END IF;
END
$validation$;
