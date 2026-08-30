-- Performance measurements, excluding is_canonical and confidence.
SELECT
    v.slug AS variant_slug,
    pm.metric_type_code,
    pm.raw_value,
    pm.raw_unit_code,
    pm.canonical_value
FROM aircraft_specs.performance_metrics pm
JOIN aircraft_core.variants v ON v.id = pm.variant_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5;
