-- Engine identity and how it is fitted to the variant.
SELECT
    v.slug AS variant_slug,
    ev.slug AS engine_slug,
    ev.manufacturer_name_raw,
    ev.model_designation,
    ev.propulsion_category_code,
    ev.hp_rated,
    ev.thrust_lbf_dry,
    vp.engine_count,
    vp.is_standard,
    vp.is_primary
FROM aircraft_power.variant_powerplants vp
JOIN aircraft_core.variants        v  ON v.id  = vp.variant_id
JOIN aircraft_power.engine_variants ev ON ev.id = vp.engine_variant_id
WHERE v.ingest_key IS NOT NULL
ORDER BY 1, 2, 3, 4, 5, 6, 7, 8, 9, 10;
