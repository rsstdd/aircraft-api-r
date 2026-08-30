-- Open curation work raised by the loader.
SELECT
    cf.entity_type_code,
    COALESCE(flag_variant.slug, valuation_variant.slug, cost_variant.slug) AS variant_slug,
    CASE
        WHEN valuation.snapshot_date = CURRENT_DATE OR cost_snapshot.snapshot_date = CURRENT_DATE
            THEN '<current>'
        ELSE COALESCE(valuation.snapshot_date::text, cost_snapshot.snapshot_date::text)
    END AS snapshot_date,
    COALESCE(valuation.source_name, cost_snapshot.source_name) AS source_name,
    cf.field_name,
    cf.issue_type,
    cf.issue_description,
    cf.priority
FROM aircraft_prov.curation_flags cf
LEFT JOIN aircraft_core.variants flag_variant
    ON cf.entity_type_code = 'AIRCRAFT_VARIANT' AND flag_variant.id = cf.entity_id
LEFT JOIN aircraft_market.valuations valuation
    ON cf.entity_type_code = 'VALUATION' AND valuation.id = cf.entity_id
LEFT JOIN aircraft_core.variants valuation_variant
    ON valuation_variant.id = valuation.variant_id
LEFT JOIN aircraft_market.cost_snapshots cost_snapshot
    ON cf.entity_type_code = 'COST_SNAPSHOT' AND cost_snapshot.id = cf.entity_id
LEFT JOIN aircraft_core.variants cost_variant
    ON cost_variant.id = cost_snapshot.variant_id
ORDER BY 1, 2, 3, 4, 5, 6, 7, 8;
