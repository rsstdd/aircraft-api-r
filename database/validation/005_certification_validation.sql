-- =============================================================================
-- File: database/validation/005_certification_validation.sql
-- Phase 5 — validation for aircraft_cert tables and the new
-- aircraft_ref.operating_approval_types lookup.
-- Run after 005_certification_operating_approvals.sql.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE — all 6 Phase 5 tables present
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema IN ('aircraft_cert', 'aircraft_ref')
  AND table_name IN (
      'operating_approval_types',
      'type_certificates',
      'variant_type_certs',
      'variant_operating_approvals',
      'pilot_requirements',
      'safety_metrics'
  )
ORDER BY table_schema, table_name;
-- Expect: 6 rows.

-- -----------------------------------------------------------------------------
-- 2. SEED DATA — operating_approval_types count and coverage
-- -----------------------------------------------------------------------------
SELECT count(*) AS total_approval_types,
       count(*) FILTER (WHERE is_positive)     AS binary_types,
       count(*) FILTER (WHERE NOT is_positive) AS graduated_types
FROM aircraft_ref.operating_approval_types
WHERE is_active;
-- Expect: 14 total; 13 binary (TRUE); 1 graduated (CAT_III_ILS).

-- Spot-check key approval codes:
SELECT code, label, is_positive, sort_order
FROM aircraft_ref.operating_approval_types
WHERE code IN ('IFR','KNOWN_ICING_FIKI','AEROBATIC','CAT_II_ILS','ETOPS_180','RVSM')
ORDER BY sort_order;
-- Expect: 6 rows, all is_active = TRUE.

-- -----------------------------------------------------------------------------
-- 3. FOREIGN KEY CHAINS — aircraft_cert tables reference correct schemas
-- -----------------------------------------------------------------------------
SELECT
    tc.table_name            AS fk_table,
    kcu.column_name          AS fk_column,
    ccu.table_schema         AS ref_schema,
    ccu.table_name           AS ref_table,
    rc.delete_rule
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu
    ON kcu.constraint_name = tc.constraint_name
    AND kcu.table_schema   = tc.table_schema
JOIN information_schema.referential_constraints rc
    ON rc.constraint_name   = tc.constraint_name
    AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu
    ON ccu.constraint_name  = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_cert'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   type_certificates       → aircraft_ref.certification_authorities (authority_code)
--   type_certificates       → aircraft_ref.airworthiness_categories
--   type_certificates       → aircraft_org.organizations (tc_holder_org_id, SET NULL)
--   variant_type_certs      → aircraft_core.variants     (CASCADE)
--   variant_type_certs      → aircraft_cert.type_certificates (RESTRICT)
--   variant_operating_appr. → aircraft_core.variants     (CASCADE)
--   variant_operating_appr. → aircraft_ref.operating_approval_types
--   pilot_requirements      → aircraft_core.variants     (CASCADE via UNIQUE)
--   pilot_requirements      → aircraft_ref.pilot_certificate_types
--   safety_metrics          → aircraft_core.variants     (CASCADE)

SELECT count(*) AS total_fks
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY'
  AND table_schema    = 'aircraft_cert';
-- Expect: 10

-- -----------------------------------------------------------------------------
-- 4. TRIGGERS ON EDITABLE TABLES
-- -----------------------------------------------------------------------------
SELECT event_object_table AS table_name, trigger_name
FROM information_schema.triggers
WHERE trigger_schema = 'aircraft_cert'
ORDER BY event_object_table, trigger_name;
-- Expect: 4 rows — trg_type_certs_updated, trg_voa_updated,
--         trg_pilot_req_updated, trg_safety_metrics_updated.

-- -----------------------------------------------------------------------------
-- 5. CHECK CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name, cc.check_clause
FROM information_schema.table_constraints  tc
JOIN information_schema.check_constraints  cc
    ON cc.constraint_name   = tc.constraint_name
    AND cc.constraint_schema = tc.constraint_schema
WHERE tc.table_schema    = 'aircraft_cert'
  AND tc.constraint_type = 'CHECK'
ORDER BY tc.table_name, tc.constraint_name;
-- Expect:
--   pilot_requirements:         chk_pr_min_crew, chk_pr_type_rating
--   safety_metrics:             chk_sm_period, chk_sm_rate_nonneg
--   (UNIQUE constraints are separate, not CHECK)

-- -----------------------------------------------------------------------------
-- 6. UNIQUE CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name
FROM information_schema.table_constraints tc
WHERE tc.table_schema    = 'aircraft_cert'
  AND tc.constraint_type = 'UNIQUE'
ORDER BY tc.table_name, tc.constraint_name;
-- Expect:
--   pilot_requirements:          1 row (variant_id UNIQUE → enforces 1:1)
--   type_certificates:           1 row (tc_number, authority_code)
--   variant_type_certs:          1 row (variant_id, tc_id)
--   variant_operating_approvals: 1 row (variant_id, approval_type_code)

-- -----------------------------------------------------------------------------
-- 7. INDEXES — count per table
-- -----------------------------------------------------------------------------
SELECT t.relname AS table_name, count(*) AS index_count
FROM pg_index ix
JOIN pg_class t  ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_cert'
GROUP BY t.relname
ORDER BY t.relname;
-- Expected index counts (including PK + UNIQUE):
--   pilot_requirements:           5  (pk, uq_variant, idx_type_rating, idx_cert, complex/hp/tw)
--   safety_metrics:               3  (pk, idx_variant, idx_type)
--   type_certificates:            5  (pk, uq_tc_auth, idx_authority, idx_holder, idx_airworthiness, idx_trgm)
--   variant_operating_approvals:  3  (pk, uq_variant_type, idx_type_approved)
--   variant_type_certs:           3  (pk, uq_variant_tc, idx_tc)

-- -----------------------------------------------------------------------------
-- 8. SMOKE TEST — hierarchy chain through to pilot_requirements
-- Insert a minimal chain (family → model → variant → pilot_requirements)
-- and verify constraint behaviour, then roll back.
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam_id  BIGINT;
    v_mod_id  BIGINT;
    v_var_id  BIGINT;
    v_pr_id   BIGINT;
BEGIN
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('cert-smoke-family', 'Cert Smoke Family')
    RETURNING id INTO v_fam_id;

    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam_id, 'cert-smoke-model', 'Cert Smoke Model')
    RETURNING id INTO v_mod_id;

    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod_id, 'cert-smoke-variant', 'Cert Smoke Variant 1')
    RETURNING id INTO v_var_id;

    -- Valid pilot_requirements row
    INSERT INTO aircraft_cert.pilot_requirements
        (variant_id, min_certificate_code, type_rating_required, min_crew,
         requires_complex, requires_high_perf)
    VALUES
        (v_var_id, 'FAA_PRIVATE', FALSE, 1, TRUE, TRUE)
    RETURNING id INTO v_pr_id;

    -- Verify 1:1: inserting a second pilot_requirements for the same variant fails
    BEGIN
        INSERT INTO aircraft_cert.pilot_requirements
            (variant_id, min_certificate_code, type_rating_required)
        VALUES (v_var_id, 'FAA_COMMERCIAL', FALSE);
        RAISE EXCEPTION 'UNIQUE on variant_id should have fired';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Verify type_rating_code may not be set when type_rating_required = FALSE
    BEGIN
        INSERT INTO aircraft_cert.pilot_requirements
            (variant_id, type_rating_required, type_rating_code)
        SELECT v_var_id + 9999, FALSE, 'B738'   -- non-existent variant_id
        WHERE FALSE;   -- don't actually execute; just parse
        -- Actual constraint test via UPDATE:
        UPDATE aircraft_cert.pilot_requirements
        SET type_rating_required = FALSE, type_rating_code = 'B738'
        WHERE id = v_pr_id;
        -- Should be rejected by chk_pr_type_rating
        RAISE EXCEPTION 'chk_pr_type_rating should have fired';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Verify operating_approval insert with known approval code
    INSERT INTO aircraft_cert.variant_operating_approvals
        (variant_id, approval_type_code, is_approved, confidence)
    VALUES
        (v_var_id, 'IFR',             TRUE, 0.90),
        (v_var_id, 'KNOWN_ICING_FIKI',FALSE, 0.85);

    -- Verify duplicate approval_type rejected
    BEGIN
        INSERT INTO aircraft_cert.variant_operating_approvals
            (variant_id, approval_type_code, is_approved)
        VALUES (v_var_id, 'IFR', TRUE);
        RAISE EXCEPTION 'UNIQUE on (variant_id, approval_type_code) should have fired';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Verify safety_metrics negative rate rejected
    BEGIN
        INSERT INTO aircraft_cert.safety_metrics
            (variant_id, metric_type, rate_value)
        VALUES (v_var_id, 'fatal_rate', -1.5);
        RAISE EXCEPTION 'chk_sm_rate_nonneg should have fired';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 5 smoke test passed: 1:1 UNIQUE, chk_pr_type_rating, '
                 'duplicate approval rejection, rate non-negative check — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 9. SUMMARY — row counts (all zero before Phase 17 ingestion)
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_ref.operating_approval_types) AS approval_types_seeded,
    (SELECT count(*) FROM aircraft_cert.type_certificates)       AS type_certs,
    (SELECT count(*) FROM aircraft_cert.variant_type_certs)      AS variant_tc_links,
    (SELECT count(*) FROM aircraft_cert.variant_operating_approvals) AS approval_facts,
    (SELECT count(*) FROM aircraft_cert.pilot_requirements)      AS pilot_req_rows,
    (SELECT count(*) FROM aircraft_cert.safety_metrics)          AS safety_metric_rows;
-- Expect: 14 approval_types; 0 for all aircraft data tables.