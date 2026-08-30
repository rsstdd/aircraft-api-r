-- Market valuations, excluding confidence and the snapshot date.
SELECT
    v.slug AS variant_slug,
    val.source_name,
    val.source_url,
    val.papi_price_estimate,
    val.for_sale_count,
    val.currency_code
FROM aircraft_market.valuations val
JOIN aircraft_core.variants v ON v.id = val.variant_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5, 6;
