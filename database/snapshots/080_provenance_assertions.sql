-- Field-level assertions, excluding status_code, is_accepted, and confidence.
SELECT
    sa.entity_type_code,
    COALESCE(assertion_variant.slug, valuation_variant.slug, cost_variant.slug) AS variant_slug,
    CASE
        WHEN valuation.snapshot_date = CURRENT_DATE OR cost_snapshot.snapshot_date = CURRENT_DATE
            THEN '<current>'
        ELSE COALESCE(valuation.snapshot_date::text, cost_snapshot.snapshot_date::text)
    END AS snapshot_date,
    COALESCE(valuation.source_name, cost_snapshot.source_name) AS source_name,
    sa.field_name,
    sa.raw_value,
    sa.raw_unit,
    sa.asserted_value,
    sa.asserted_numeric
FROM aircraft_prov.source_assertions sa
LEFT JOIN aircraft_core.variants assertion_variant
    ON sa.entity_type_code = 'AIRCRAFT_VARIANT' AND assertion_variant.id = sa.entity_id
LEFT JOIN aircraft_market.valuations valuation
    ON sa.entity_type_code = 'VALUATION' AND valuation.id = sa.entity_id
LEFT JOIN aircraft_core.variants valuation_variant
    ON valuation_variant.id = valuation.variant_id
LEFT JOIN aircraft_market.cost_snapshots cost_snapshot
    ON sa.entity_type_code = 'COST_SNAPSHOT' AND cost_snapshot.id = sa.entity_id
LEFT JOIN aircraft_core.variants cost_variant
    ON cost_variant.id = cost_snapshot.variant_id
ORDER BY 1, 2, 3, 4, 5, 6, 7, 8, 9;
