-- Field-level assertions, excluding status_code, is_accepted, and confidence.
SELECT
    sa.entity_type_code,
    sa.field_name,
    sa.raw_value,
    sa.raw_unit,
    sa.asserted_value,
    sa.asserted_numeric
FROM aircraft_prov.source_assertions sa
ORDER BY 1, 2, 3, 4, 5, 6;
