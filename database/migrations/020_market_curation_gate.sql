-- =============================================================================
-- File: database/migrations/020_market_curation_gate.sql
-- Phase 20: Gate market data and link measurements to exact assertions.
--
-- Migrations 008 and 019 put every performance and weight measurement behind an
-- is_canonical flag, so nothing ingestion writes reaches the read model until a
-- curator accepts it. Market data was left out. Verified on a live database:
-- after importing tests/fixtures/planephd_edge_cases.json,
-- aircraft_read.mv_variant_search served
--
--     papi_price_usd = 1150000.00, for_sale_count = 7,
--     total_annual_cost_usd and cost_per_hour_usd for all three variants
--
-- while cruise_speed_kias and gross_weight_lb were correctly NULL. An uncurated
-- scraped price estimate was published the moment it was ingested.
--
-- The gate is at the snapshot level, not the line-item level. A cost snapshot is
-- a coherent unit -- a set of costs captured together under stated assumptions --
-- and mv_ownership_cost_summary's computed_total_annual_usd wraps its sums in
-- COALESCE(..., 0). Filtering individual line items would therefore publish a
-- confident "$0.00 annual cost" for an uncurated aircraft, which is worse than
-- withholding it. Filtering the snapshot removes the row entirely and the
-- LEFT JOIN in mv_variant_search yields NULL.
--
-- Existing rows are backfilled only when the pre-migration read models selected
-- them as current. Historical time-series siblings remain stored but pending.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

-- A curation decision must identify the exact stored measurement it backs.
-- Variant and metric type alone are insufficient because several sources may
-- report competing values for the same field.
ALTER TABLE aircraft_specs.performance_metrics
    ADD COLUMN source_assertion_id BIGINT;
ALTER TABLE aircraft_specs.weight_metrics
    ADD COLUMN source_assertion_id BIGINT;

ALTER TABLE aircraft_specs.performance_metrics
    ADD CONSTRAINT fk_pm_source_assertion
    FOREIGN KEY (source_assertion_id)
    REFERENCES aircraft_prov.source_assertions(id) NOT VALID;
ALTER TABLE aircraft_specs.weight_metrics
    ADD CONSTRAINT fk_wm_source_assertion
    FOREIGN KEY (source_assertion_id)
    REFERENCES aircraft_prov.source_assertions(id) NOT VALID;

COMMENT ON COLUMN aircraft_specs.performance_metrics.source_assertion_id IS
    'Exact pending source assertion that produced this measurement. Curation '
    'changes is_canonical only on the row linked to its decision.';
COMMENT ON COLUMN aircraft_specs.weight_metrics.source_assertion_id IS
    'Exact pending source assertion that produced this measurement. Curation '
    'changes is_canonical only on the row linked to its decision.';

-- One assertion represents one stored measurement. Historical rows remain
-- nullable because migration-time inference could associate the wrong source.
-- squawk-ignore require-concurrent-index-creation
CREATE UNIQUE INDEX uq_pm_source_assertion
    ON aircraft_specs.performance_metrics (source_assertion_id)
    WHERE source_assertion_id IS NOT NULL;
-- squawk-ignore require-concurrent-index-creation
CREATE UNIQUE INDEX uq_wm_source_assertion
    ON aircraft_specs.weight_metrics (source_assertion_id)
    WHERE source_assertion_id IS NOT NULL;

COMMENT ON COLUMN aircraft_prov.source_assertions.is_accepted IS
    'TRUE = this assertion is the designated canonical value for this field. '
    'At most one TRUE per (entity_type_code, entity_id, field_name), enforced '
    'by the partial UNIQUE index uq_assertion_accepted. '
    'The Rust ingestion adapter writes every assertion PENDING with '
    'is_accepted = FALSE; curation must explicitly accept one. The legacy SQL '
    'loader still accepts the first assertion per field.';

ALTER TABLE aircraft_market.valuations
    ADD COLUMN IF NOT EXISTS is_canonical BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE aircraft_market.cost_snapshots
    ADD COLUMN IF NOT EXISTS is_canonical BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN aircraft_market.valuations.is_canonical IS
    'TRUE = this valuation is served through the read model. Ingestion writes '
    'FALSE and leaves promotion to curation, which accepts the VALUATION '
    'assertions backing it. At most one TRUE per variant (uq_val_canonical).';
COMMENT ON COLUMN aircraft_market.cost_snapshots.is_canonical IS
    'TRUE = this snapshot and its line items and totals are served through the '
    'read model. Published as a unit: the snapshot''s totals are only meaningful '
    'together. At most one TRUE per variant (uq_cs_canonical).';

-- Preserve exactly the one valuation and cost snapshot per variant that the
-- pre-migration read models selected. Older time-series rows stay non-canonical.
WITH current_valuations AS (
    SELECT DISTINCT ON (variant_id) id
    FROM aircraft_market.valuations
    ORDER BY variant_id, snapshot_date DESC, captured_at DESC, id DESC
)
UPDATE aircraft_market.valuations AS valuations
SET    is_canonical = TRUE
FROM   current_valuations
WHERE  valuations.id = current_valuations.id;

WITH current_cost_snapshots AS (
    SELECT DISTINCT ON (variant_id) id
    FROM aircraft_market.cost_snapshots
    ORDER BY variant_id, snapshot_date DESC, id DESC
)
UPDATE aircraft_market.cost_snapshots AS snapshots
SET    is_canonical = TRUE
FROM   current_cost_snapshots
WHERE  snapshots.id = current_cost_snapshots.id;

-- Mirrors uq_perf_canonical (008) and uq_wm_canonical (019): one served row per
-- variant, so two snapshots cannot both claim to be the current picture.
-- These must commit atomically with the view rebuilds below; a concurrent index
-- cannot run inside this transaction.
-- squawk-ignore require-concurrent-index-creation
CREATE UNIQUE INDEX IF NOT EXISTS uq_val_canonical
    ON aircraft_market.valuations (variant_id)
    WHERE is_canonical;
-- squawk-ignore require-concurrent-index-creation
CREATE UNIQUE INDEX IF NOT EXISTS uq_cs_canonical
    ON aircraft_market.cost_snapshots (variant_id)
    WHERE is_canonical;

-- -----------------------------------------------------------------------------
-- v_current_valuation: pick the current *curated* valuation.
-- -----------------------------------------------------------------------------
CREATE OR REPLACE VIEW aircraft_read.v_current_valuation AS
SELECT DISTINCT ON (variant_id)
    id,
    variant_id,
    snapshot_date,
    source_name,
    papi_price_estimate,
    for_sale_count,
    currency_code,
    region_code,
    condition_grade_code,
    confidence,
    captured_at
FROM aircraft_market.valuations
WHERE is_canonical
ORDER BY variant_id, snapshot_date DESC, captured_at DESC, id DESC;

COMMENT ON VIEW aircraft_read.v_current_valuation IS
    'Most recent curated market valuation per variant. Uncurated valuations are '
    'stored and auditable but never served.';

-- -----------------------------------------------------------------------------
-- Read models: gate the cost summary on curated snapshots, and the search
-- matview's valuation lookup on curated valuations.
--
-- mv_variant_search reads aircraft_market.valuations directly in a LATERAL, so
-- fixing v_current_valuation alone would not affect it. It also depends on
-- mv_ownership_cost_summary, so the dependent must be dropped first and rebuilt
-- last. Both definitions are read back from the catalog rather than restated, so
-- they cannot silently diverge from what migrations 016-019 produced, and both
-- index sets are captured before the drops and replayed after.
-- -----------------------------------------------------------------------------
DO $migration$
DECLARE
    cost_definition   TEXT;
    search_definition TEXT;
    cost_indexes      TEXT[];
    search_indexes    TEXT[];
    statement         TEXT;
BEGIN
    SELECT pg_get_viewdef('aircraft_read.mv_ownership_cost_summary'::regclass, TRUE)
    INTO cost_definition;
    SELECT pg_get_viewdef('aircraft_read.mv_variant_search'::regclass, TRUE)
    INTO search_definition;
    cost_definition := regexp_replace(cost_definition, ';[[:space:]]*$', '');
    search_definition := regexp_replace(search_definition, ';[[:space:]]*$', '');

    IF position('cost_snapshots.is_canonical' IN cost_definition) > 0
       AND position('valuations.is_canonical' IN search_definition) > 0 THEN
        RAISE NOTICE 'read models already gate market data; leaving them alone';
        RETURN;
    END IF;

    IF position('FROM aircraft_market.cost_snapshots' IN cost_definition) = 0
       OR position('WHERE valuations.variant_id = v.id' IN search_definition) = 0 THEN
        RAISE EXCEPTION
            'read models do not contain the expected market predicates; '
            'refusing to rebuild them into ungated views';
    END IF;

    cost_definition := replace(
        cost_definition,
        'FROM aircraft_market.cost_snapshots',
        'FROM aircraft_market.cost_snapshots WHERE cost_snapshots.is_canonical'
    );
    search_definition := replace(
        search_definition,
        'WHERE valuations.variant_id = v.id',
        'WHERE valuations.variant_id = v.id AND valuations.is_canonical'
    );

    SELECT array_agg(indexdef) INTO cost_indexes
    FROM pg_indexes
    WHERE schemaname = 'aircraft_read' AND tablename = 'mv_ownership_cost_summary';
    SELECT array_agg(indexdef) INTO search_indexes
    FROM pg_indexes
    WHERE schemaname = 'aircraft_read' AND tablename = 'mv_variant_search';

    -- The dependent first, or the drop below fails.
    DROP MATERIALIZED VIEW aircraft_read.mv_variant_search;
    DROP MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary;

    EXECUTE
        'CREATE MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary AS '
        || cost_definition || ' WITH NO DATA';
    FOREACH statement IN ARRAY COALESCE(cost_indexes, ARRAY[]::TEXT[]) LOOP
        EXECUTE statement;
    END LOOP;

    EXECUTE
        'CREATE MATERIALIZED VIEW aircraft_read.mv_variant_search AS '
        || search_definition || ' WITH NO DATA';
    FOREACH statement IN ARRAY COALESCE(search_indexes, ARRAY[]::TEXT[]) LOOP
        EXECUTE statement;
    END LOOP;
END
$migration$;

COMMIT;
