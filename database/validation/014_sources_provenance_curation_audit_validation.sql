-- =============================================================================
-- File: database/validation/014_sources_provenance_curation_audit_validation.sql
-- Phase 14 — validation for aircraft_prov tables.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE AND SEED DATA EXISTENCE
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_prov'
ORDER BY table_name;
-- Expect: audit_log, curation_flags, source_assertions, source_documents, sources

SELECT slug, name, source_type_code, reliability_grade_code,
       default_confidence, is_active
FROM aircraft_prov.sources
ORDER BY slug;
-- Expect: 3 rows: faa-tcds (1.00), manual-editorial (0.80), planephd (0.20).

-- Verify confidence values reflect reliability grade intent.
SELECT s.slug, s.default_confidence, srg.numeric_score,
       round(srg.numeric_score::numeric / 5, 2) AS derived_confidence
FROM aircraft_prov.sources s
JOIN aircraft_ref.source_reliability_grades srg ON srg.code = s.reliability_grade_code
ORDER BY srg.numeric_score DESC;
-- PlanePHD: numeric_score=1, derived=0.20 ✓
-- manual-editorial: numeric_score=4, derived=0.80 ✓
-- faa-tcds: numeric_score=5, derived=1.00 ✓

-- -----------------------------------------------------------------------------
-- 2. FK CHAINS
-- -----------------------------------------------------------------------------
SELECT tc.table_name AS fk_table, kcu.column_name AS fk_col,
       ccu.table_schema AS ref_schema, ccu.table_name AS ref_table,
       rc.delete_rule
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
JOIN information_schema.referential_constraints rc  ON rc.constraint_name  = tc.constraint_name AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_prov'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   sources           → aircraft_ref.source_types, source_reliability_grades
--   source_documents  → aircraft_prov.sources (RESTRICT), aircraft_core.variants (SET NULL)
--   source_assertions → aircraft_prov.source_documents (CASCADE),
--                       aircraft_ref.curation_entity_types, aircraft_ref.assertion_statuses
--   curation_flags    → aircraft_ref.curation_entity_types, curation_flag_statuses,
--                       aircraft_prov.source_assertions (SET NULL)
--   audit_log         → aircraft_ref.curation_entity_types,
--                       aircraft_prov.source_assertions (SET NULL)

SELECT count(*) AS total_fks
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY' AND table_schema = 'aircraft_prov';
-- Expect: ~12

-- -----------------------------------------------------------------------------
-- 3. CHECK CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name
FROM information_schema.table_constraints tc
WHERE tc.table_schema    = 'aircraft_prov'
  AND tc.constraint_type = 'CHECK'
ORDER BY tc.table_name, tc.constraint_name;
-- Expect: chk_sdo_status, chk_cf_priority, chk_cf_resolution, chk_al_change_source

-- -----------------------------------------------------------------------------
-- 4. COMPREHENSIVE SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_src  BIGINT; v_doc BIGINT;
    v_sa1  BIGINT; v_sa2 BIGINT;
    v_cf   BIGINT;
    v_et_variant TEXT := 'AIRCRAFT_VARIANT';
BEGIN
    -- Hierarchy
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('prov14-smoke-fam', 'Prov14 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'prov14-smoke-mod', 'Prov14 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'prov14-smoke-var', 'Prov14 Variant') RETURNING id INTO v_var;

    -- Retrieve PlanePHD source
    SELECT id INTO v_src FROM aircraft_prov.sources WHERE slug = 'planephd';

    -- Source document
    INSERT INTO aircraft_prov.source_documents
        (source_id, source_url, source_path, source_system_key,
         ingest_batch_label, parser_version, variant_id,
         raw_json, processing_status)
    VALUES
        (v_src, 'https://planephd.com/wizard/details/123456',
         '/wizard/details/123456', '123456',
         'planephd_bulk_2024_01', 'planephd_etl_v2.1.0', v_var,
         '{"manufacturer_name":"Test Co","aircraft_name":"Smoke 1","papi_price_estimate":"$185,000"}'::jsonb,
         'PROCESSED')
    RETURNING id INTO v_doc;

    -- Dedup: same source_system_key from same source → rejected
    BEGIN
        INSERT INTO aircraft_prov.source_documents
            (source_id, source_url, source_system_key, processing_status)
        VALUES (v_src, 'https://planephd.com/wizard/details/123456', '123456', 'PENDING');
        RAISE EXCEPTION 'uq_sd_source_key should reject duplicate source_system_key';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Same key from DIFFERENT source → allowed
    DECLARE v_src2 BIGINT;
    BEGIN
        SELECT id INTO v_src2 FROM aircraft_prov.sources WHERE slug = 'manual-editorial';
        INSERT INTO aircraft_prov.source_documents
            (source_id, source_url, source_system_key, processing_status)
        VALUES (v_src2, 'https://example.com/123456', '123456', 'PENDING');
        -- This should succeed (different source_id, same key)
    END;

    -- Source assertion 1: PlanePHD asserts cruise speed (PENDING, first value)
    INSERT INTO aircraft_prov.source_assertions
        (source_document_id, entity_type_code, entity_id, field_name,
         raw_value, raw_unit, asserted_numeric,
         status_code, is_accepted, confidence)
    VALUES
        (v_doc, v_et_variant, v_var, 'description',
         'Prov14 Variant is a test aircraft.',
         NULL, NULL,
         'ACCEPTED', TRUE, 0.20)
    RETURNING id INTO v_sa1;

    -- Assertion 2: different source asserts different description (PENDING, not accepted)
    INSERT INTO aircraft_prov.source_assertions
        (source_document_id, entity_type_code, entity_id, field_name,
         raw_value, asserted_numeric,
         status_code, is_accepted, confidence)
    VALUES
        (v_doc, v_et_variant, v_var, 'description',
         'Alternative description from second source.',
         NULL,
         'PENDING', FALSE, 0.40)
    RETURNING id INTO v_sa2;

    -- Attempting to accept BOTH → rejected (partial UNIQUE uq_assertion_accepted)
    BEGIN
        UPDATE aircraft_prov.source_assertions
        SET is_accepted = TRUE, status_code = 'ACCEPTED'
        WHERE id = v_sa2;
        RAISE EXCEPTION 'uq_assertion_accepted should reject second accepted assertion for same field';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Accepting sa2 is only allowed after revoking sa1's acceptance
    UPDATE aircraft_prov.source_assertions SET is_accepted = FALSE, status_code = 'SUPERSEDED'
    WHERE id = v_sa1;
    -- Now sa2 can be accepted
    UPDATE aircraft_prov.source_assertions
    SET is_accepted = TRUE, status_code = 'ACCEPTED'
    WHERE id = v_sa2;
    -- Revert for clean state
    UPDATE aircraft_prov.source_assertions SET is_accepted = FALSE, status_code = 'PENDING'
    WHERE id = v_sa2;
    UPDATE aircraft_prov.source_assertions SET is_accepted = TRUE, status_code = 'ACCEPTED'
    WHERE id = v_sa1;

    -- Curation flag: conflicting values detected
    INSERT INTO aircraft_prov.curation_flags
        (entity_type_code, entity_id, field_name,
         issue_type, issue_description,
         status_code, priority, source_assertion_id)
    VALUES
        (v_et_variant, v_var, 'description',
         'CONFLICTING_SOURCES',
         'PlanePHD and manual entry report different descriptions.',
         'OPEN', 2, v_sa2)
    RETURNING id INTO v_cf;

    -- Invalid priority → rejected
    BEGIN
        INSERT INTO aircraft_prov.curation_flags
            (entity_type_code, entity_id, issue_type, status_code, priority)
        VALUES (v_et_variant, v_var, 'TEST', 'OPEN', 6);
        RAISE EXCEPTION 'chk_cf_priority should reject priority > 5';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- chk_cf_resolution: resolved_at on a non-terminal status → rejected
    BEGIN
        UPDATE aircraft_prov.curation_flags
        SET resolved_at = now()
        WHERE id = v_cf;   -- status is still OPEN (non-terminal)
        RAISE EXCEPTION 'chk_cf_resolution should reject resolved_at with non-terminal status';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Resolving correctly: set terminal status first
    UPDATE aircraft_prov.curation_flags
    SET status_code = 'RESOLVED', resolved_at = now(), resolved_by = 'test_curator',
        resolution_notes = 'PlanePHD description accepted; manual entry archived.'
    WHERE id = v_cf;

    -- Audit log entry
    INSERT INTO aircraft_prov.audit_log
        (entity_type_code, entity_id, field_name,
         old_value, new_value,
         change_source, changed_by, change_reason,
         source_assertion_id)
    VALUES
        (v_et_variant, v_var, 'description',
         NULL,
         'Prov14 Variant is a test aircraft.',
         'INGESTION', 'planephd_etl_v2.1.0',
         'Initial ingestion from PlanePHD document 123456.',
         v_sa1);

    -- Invalid change_source → rejected
    BEGIN
        INSERT INTO aircraft_prov.audit_log
            (entity_type_code, entity_id, field_name, change_source, changed_by)
        VALUES (v_et_variant, v_var, 'name', 'MANUAL_TWEAK', 'user');
        RAISE EXCEPTION 'chk_al_change_source should reject unknown source';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Invalid processing_status → rejected
    BEGIN
        INSERT INTO aircraft_prov.source_documents
            (source_id, source_url, processing_status)
        VALUES (v_src, 'https://example.com/bad', 'UNKNOWN');
        RAISE EXCEPTION 'chk_sdo_status should reject unknown processing_status';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 14 smoke test passed: '
                 'source dedup (uq_sd_source_key), '
                 'assertion accepted uniqueness (uq_assertion_accepted), '
                 'flag priority and resolution CHECKs, '
                 'audit log change_source CHECK, '
                 'document status CHECK — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 5. KEY INDEXES PRESENT
-- -----------------------------------------------------------------------------
SELECT i.relname AS index_name, t.relname AS table_name,
       ix.indisunique,
       CASE WHEN pg_get_indexdef(ix.indexrelid) LIKE '%WHERE%' THEN 'partial' ELSE 'full' END AS scope,
       left(pg_get_indexdef(ix.indexrelid), 80) AS definition_snippet
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_prov'
ORDER BY t.relname, i.relname;
-- Verify: uq_assertion_accepted (partial UNIQUE), uq_sd_source_key (partial UNIQUE),
--         idx_sdo_raw_json (GIN), idx_cf_open_priority (partial), etc.

-- -----------------------------------------------------------------------------
-- 6. CONFLICT DETECTION QUERY PATTERN
-- Shows assertions where multiple sources disagree on the same field.
-- (post-ingestion; demonstrates the core provenance use case)
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
SELECT
    cet.table_name   AS entity_table,
    sa.entity_id,
    sa.field_name,
    count(*)         AS assertion_count,
    count(*) FILTER (WHERE sa.is_accepted) AS accepted_count,
    count(*) FILTER (WHERE sa.status_code = 'PENDING') AS pending_count,
    array_agg(DISTINCT sa.asserted_value ORDER BY sa.asserted_value) AS distinct_values
FROM aircraft_prov.source_assertions sa
JOIN aircraft_ref.curation_entity_types cet ON cet.code = sa.entity_type_code
GROUP BY cet.table_name, sa.entity_id, sa.field_name
HAVING count(*) > 1
   AND count(DISTINCT sa.asserted_value) > 1
ORDER BY assertion_count DESC, cet.table_name, sa.entity_id
LIMIT 25;
-- Returns fields where multiple sources report different values → curate these.
*/

-- -----------------------------------------------------------------------------
-- 7. STALE DATA DETECTION QUERY PATTERN
-- Identifies source documents not refreshed within the source refresh interval.
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
SELECT
    s.name AS source_name, s.refresh_interval_days,
    sd.source_url,
    sd.retrieved_at,
    (now() - sd.retrieved_at)::interval AS age,
    sd.variant_id
FROM aircraft_prov.source_documents sd
JOIN aircraft_prov.sources s ON s.id = sd.source_id
WHERE s.refresh_interval_days IS NOT NULL
  AND sd.retrieved_at < (now() - (s.refresh_interval_days || ' days')::interval)
  AND sd.processing_status = 'PROCESSED'
ORDER BY sd.retrieved_at ASC
LIMIT 50;
-- Returns documents overdue for re-retrieval per the source refresh schedule.
*/

-- -----------------------------------------------------------------------------
-- 8. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_prov.sources)           AS sources_seeded,
    (SELECT count(*) FROM aircraft_prov.source_documents)  AS source_documents,
    (SELECT count(*) FROM aircraft_prov.source_assertions) AS source_assertions,
    (SELECT count(*) FROM aircraft_prov.curation_flags)    AS curation_flags,
    (SELECT count(*) FROM aircraft_prov.audit_log)         AS audit_log_entries;
-- Expect: 3 sources seeded; 0 for all others (data arrives Phase 17).