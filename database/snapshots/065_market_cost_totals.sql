-- Aggregate cost totals, kept separate from the line items they summarize.
SELECT
    v.slug AS variant_slug,
    cst.total_annual_usd,
    cst.total_fixed_usd,
    cst.total_variable_usd,
    cst.assumed_hours,
    cst.source_currency,
    cst.captured_from_key
FROM aircraft_market.cost_snapshot_totals cst
JOIN aircraft_market.cost_snapshots cs ON cs.id = cst.snapshot_id
JOIN aircraft_core.variants          v ON v.id  = cs.variant_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5, 6, 7;
