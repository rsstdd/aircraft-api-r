-- Weight measurements, excluding confidence.
SELECT
    v.slug AS variant_slug,
    wm.metric_type_code,
    wm.raw_value,
    wm.raw_unit_code,
    wm.canonical_value,
    wm.configuration
FROM aircraft_specs.weight_metrics wm
JOIN aircraft_core.variants v ON v.id = wm.variant_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5, 6;
