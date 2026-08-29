-- Manufacturer, family, model, and variant identity as the loaders resolved it.
SELECT
    o.slug   AS organization_slug,
    o.name   AS organization_name,
    f.slug   AS family_slug,
    m.slug   AS model_slug,
    v.slug   AS variant_slug,
    v.name   AS variant_name,
    v.ingest_key,
    v.source_path
FROM aircraft_core.variants v
JOIN aircraft_core.models       m ON m.id = v.model_id
JOIN aircraft_core.families     f ON f.id = m.family_id
JOIN aircraft_org.organizations o ON o.id = f.manufacturer_org_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5, 6, 7, 8;
