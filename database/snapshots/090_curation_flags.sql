-- Open curation work raised by the loader.
SELECT
    cf.entity_type_code,
    cf.field_name,
    cf.issue_type,
    cf.issue_description,
    cf.priority
FROM aircraft_prov.curation_flags cf
ORDER BY 1, 2, 3, 4, 5;
