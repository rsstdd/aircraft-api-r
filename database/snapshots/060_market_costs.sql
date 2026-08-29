-- Ownership cost line items and their aggregate totals.
--
-- Fixed costs must land in amount_annual and variable costs in amount_per_hour;
-- a loader that routes a per-hour cost to the annual column shows up here.
SELECT
    v.slug AS variant_slug,
    cli.cost_item_type_code,
    cli.amount_annual,
    cli.amount_per_hour,
    cli.currency_code
FROM aircraft_market.cost_line_items cli
JOIN aircraft_market.cost_snapshots cs ON cs.id = cli.snapshot_id
JOIN aircraft_core.variants          v ON v.id  = cs.variant_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5;
