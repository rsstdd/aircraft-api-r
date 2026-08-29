-- The source and the documents captured from it.
--
-- raw_json is excluded: the Rust adapter preserves one immutable document per
-- logical run while the legacy loader deduplicates, so the payloads are compared
-- through the assertions derived from them instead.
SELECT
    s.slug AS source_slug,
    s.name AS source_name,
    s.source_type_code,
    s.reliability_grade_code,
    s.base_url,
    sd.source_url,
    sd.source_path,
    sd.source_system_key,
    sd.processing_status,
    v.slug AS variant_slug
FROM aircraft_prov.source_documents sd
JOIN aircraft_prov.sources    s ON s.id = sd.source_id
LEFT JOIN aircraft_core.variants v ON v.id = sd.variant_id
ORDER BY 1, 2, 3, 4, 5, 6, 7, 8, 9, 10;
