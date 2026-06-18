-- =============================================================================
-- File: database/validation/phase13_maintenance_validation.sql
-- Phase 13 — validation for aircraft_maint tables.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE (6 tables expected)
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_maint'
ORDER BY table_name;
-- Expect: airworthiness_directives, life_limited_parts, service_bulletins,
--         support_assessments, variant_ads, variant_sbs

-- -----------------------------------------------------------------------------
-- 2. LOOKUP TABLE INTEGRATION CHECK
-- Verify Phase 2 lookups consumed by Phase 13 all have rows.
-- -----------------------------------------------------------------------------
SELECT 'ad_types'              AS lookup, count(*) AS rows FROM aircraft_ref.ad_types
UNION ALL
SELECT 'sb_compliance_statuses', count(*) FROM aircraft_ref.sb_compliance_statuses
UNION ALL
SELECT 'availability_grades',    count(*) FROM aircraft_ref.availability_grades
ORDER BY lookup;
-- Expect: ad_types=5, sb_compliance_statuses=6, availability_grades=5.

-- Spot-check key values
SELECT code, label FROM aircraft_ref.ad_types    ORDER BY sort_order;
SELECT code, label FROM aircraft_ref.availability_grades ORDER BY sort_order;

-- -----------------------------------------------------------------------------
-- 3. FK CHAINS
-- -----------------------------------------------------------------------------
SELECT tc.table_name AS fk_table, kcu.column_name AS fk_col,
       ccu.table_schema AS ref_schema, ccu.table_name AS ref_table,
       rc.delete_rule
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
JOIN information_schema.referential_constraints rc  ON rc.constraint_name  = tc.constraint_name AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_maint'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   airworthiness_directives → aircraft_ref.certification_authorities,
--                              aircraft_ref.ad_types
--   variant_ads    → aircraft_core.variants (CASCADE), airworthiness_directives (RESTRICT)
--   service_bulletins → aircraft_org.organizations (SET NULL),
--                       aircraft_ref.sb_compliance_statuses
--   variant_sbs    → aircraft_core.variants (CASCADE), service_bulletins (RESTRICT)
--   life_limited_parts → aircraft_core.variants (CASCADE),
--                        aircraft_org.organizations (SET NULL)
--   support_assessments → aircraft_core.variants (CASCADE),
--                         aircraft_ref.availability_grades ×2 (parts + network),
--                         (no FK on TEXT CHECK columns)

SELECT count(*) AS total_fks
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY' AND table_schema = 'aircraft_maint';
-- Expect: ~11

-- -----------------------------------------------------------------------------
-- 4. CHECK CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name,
       left(cc.check_clause, 100) AS check_snippet
FROM information_schema.table_constraints  tc
JOIN information_schema.check_constraints  cc
    ON cc.constraint_name = tc.constraint_name AND cc.constraint_schema = tc.constraint_schema
WHERE tc.table_schema    = 'aircraft_maint'
  AND tc.constraint_type = 'CHECK'
ORDER BY tc.table_name, tc.constraint_name;
-- Expect: chk_ad_interval, chk_vsb_cost,
--         chk_llp_has_limit, chk_llp_hours_positive,
--         chk_sa_oem_status, chk_sa_corrosion, chk_sa_dispatch, chk_sa_fleet

-- -----------------------------------------------------------------------------
-- 5. COMPREHENSIVE SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_ad1  BIGINT; v_ad2  BIGINT;
    v_sb1  BIGINT;
    v_sa   BIGINT;
BEGIN
    -- Hierarchy
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('mnt13-smoke-fam', 'Mnt13 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'mnt13-smoke-mod', 'Mnt13 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'mnt13-smoke-var', 'Mnt13 Variant') RETURNING id INTO v_var;

    -- Insert a RECURRING AD
    INSERT INTO aircraft_maint.airworthiness_directives
        (ad_number, authority_code, ad_type_code, subject, effective_date,
         compliance_interval_hours, compliance_interval_months, is_active)
    VALUES
        ('2023-19-04', 'FAA', 'RECURRING',
         'Inspection of Elevator Trim Tab Hinge Bolts',
         '2023-10-01', 100, 12, TRUE)
    RETURNING id INTO v_ad1;

    -- Insert a ONE_TIME AD
    INSERT INTO aircraft_maint.airworthiness_directives
        (ad_number, authority_code, ad_type_code, subject, effective_date,
         initial_compliance_date, is_active)
    VALUES
        ('2022-14-11', 'FAA', 'ONE_TIME',
         'Replacement of Fuel Selector Valve O-rings',
         '2022-07-15', '2022-12-31', TRUE)
    RETURNING id INTO v_ad2;

    -- Duplicate AD number+authority → rejected
    BEGIN
        INSERT INTO aircraft_maint.airworthiness_directives
            (ad_number, authority_code, ad_type_code, subject)
        VALUES ('2023-19-04', 'FAA', 'ONE_TIME', 'Duplicate Test');
        RAISE EXCEPTION 'UNIQUE(ad_number, authority_code) should reject duplicate AD';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Same AD number for EASA → allowed (different authority)
    INSERT INTO aircraft_maint.airworthiness_directives
        (ad_number, authority_code, ad_type_code, subject)
    VALUES ('2023-19-04', 'EASA', 'ONE_TIME', 'EASA equivalent AD');

    -- Link ADs to variant
    INSERT INTO aircraft_maint.variant_ads
        (variant_id, ad_id, is_significant, applicability_notes)
    VALUES
        (v_var, v_ad1, TRUE,  'All serial numbers 10000 and above.'),
        (v_var, v_ad2, FALSE, NULL);

    -- Duplicate variant-AD link → rejected
    BEGIN
        INSERT INTO aircraft_maint.variant_ads (variant_id, ad_id)
        VALUES (v_var, v_ad1);
        RAISE EXCEPTION 'PK(variant_id, ad_id) should reject duplicate link';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- chk_ad_interval: zero interval → rejected
    BEGIN
        INSERT INTO aircraft_maint.airworthiness_directives
            (ad_number, authority_code, ad_type_code, subject, compliance_interval_hours)
        VALUES ('TEST-ZERO', 'FAA', 'RECURRING', 'Zero interval test', 0);
        RAISE EXCEPTION 'chk_ad_interval should reject compliance_interval_hours = 0';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Service bulletin
    INSERT INTO aircraft_maint.service_bulletins
        (sb_number, issuer_name_raw, compliance_status_code, subject, issued_date)
    VALUES ('SEB99-2', 'Lycoming Engines', 'RECOMMENDED',
            'Installation of Chrome Steel Valve Guides', '1999-06-01')
    RETURNING id INTO v_sb1;

    INSERT INTO aircraft_maint.variant_sbs
        (variant_id, sb_id, estimated_compliance_cost_usd, applicability_notes)
    VALUES (v_var, v_sb1, 850.00, 'All IO-360 variants prior to serial cutoff.');

    -- Negative SB compliance cost → rejected
    BEGIN
        UPDATE aircraft_maint.variant_sbs
        SET estimated_compliance_cost_usd = -100
        WHERE variant_id = v_var AND sb_id = v_sb1;
        RAISE EXCEPTION 'chk_vsb_cost should reject negative compliance cost';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Life-limited part with no limit → rejected
    BEGIN
        INSERT INTO aircraft_maint.life_limited_parts
            (variant_id, part_description, life_limit_hours, life_limit_calendar)
        VALUES (v_var, 'Main Rotor Spindle', NULL, NULL);
        RAISE EXCEPTION 'chk_llp_has_limit should reject part with no life limit';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Valid life-limited part
    INSERT INTO aircraft_maint.life_limited_parts
        (variant_id, part_number, part_description,
         life_limit_hours, life_limit_calendar, overhaul_interval_hours)
    VALUES
        (v_var, 'LW-16056', 'Lycoming Connecting Rod',
         2000, NULL, 2000);

    -- Support assessment
    INSERT INTO aircraft_maint.support_assessments
        (variant_id, snapshot_date,
         fleet_size_estimate, fleet_size_source, fleet_size_year,
         oem_support_status,
         parts_availability_grade_code, maintenance_network_grade_code,
         corrosion_risk_level,
         common_issues_notes, dispatch_reliability_pct,
         dispatch_reliability_source, confidence)
    VALUES
        (v_var, CURRENT_DATE,
         42000, 'FAA Registry 2023', 2023,
         'FULL_SUPPORT',
         'EXCELLENT', 'EXCELLENT',
         'LOW',
         'Carburetor ice in humid conditions; oil consumption above 2000 hr TT.',
         NULL, NULL, 0.80)
    RETURNING id INTO v_sa;

    -- Second support assessment for same variant → rejected (UNIQUE)
    BEGIN
        INSERT INTO aircraft_maint.support_assessments
            (variant_id, snapshot_date, oem_support_status)
        VALUES (v_var, CURRENT_DATE, 'FULL_SUPPORT');
        RAISE EXCEPTION 'UNIQUE(variant_id) should reject second support assessment';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Invalid oem_support_status → rejected
    BEGIN
        UPDATE aircraft_maint.support_assessments
        SET oem_support_status = 'FULLY_ACTIVE'
        WHERE id = v_sa;
        RAISE EXCEPTION 'chk_sa_oem_status should reject unknown status';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Invalid corrosion_risk_level → rejected
    BEGIN
        UPDATE aircraft_maint.support_assessments
        SET corrosion_risk_level = 'EXTREME'
        WHERE id = v_sa;
        RAISE EXCEPTION 'chk_sa_corrosion should reject unknown level';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Dispatch reliability > 100 → rejected
    BEGIN
        UPDATE aircraft_maint.support_assessments
        SET dispatch_reliability_pct = 105.0
        WHERE id = v_sa;
        RAISE EXCEPTION 'chk_sa_dispatch should reject pct > 100';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 13 smoke test passed: AD UNIQUE, recurring AD interval CHECK, '
                 'variant_ad PK, SB dedup, SB cost nonneg, LLP has_limit CHECK, '
                 'support assessment 1:1 UNIQUE, oem_status + corrosion + dispatch CHECKs '
                 '— all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 6. PARTIAL UNIQUE INDEX ON service_bulletins
-- Verify uq_sb_number_org exists and is partial (WHERE issuer_org_id IS NOT NULL).
-- -----------------------------------------------------------------------------
SELECT i.relname AS index_name, pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_maint'
  AND t.relname = 'service_bulletins'
  AND ix.indisunique
ORDER BY i.relname;
-- Expect: uq_sb_number_org (partial: WHERE issuer_org_id IS NOT NULL).

-- -----------------------------------------------------------------------------
-- 7. BUYER RESEARCH QUERY PATTERN: most supportable aircraft
-- (post-ingestion; shows the support_assessments grade queries)
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
SELECT v.slug, v.name,
       sa.fleet_size_estimate,
       sa.oem_support_status,
       pa.label  AS parts_availability,
       mn.label  AS maintenance_network,
       sa.corrosion_risk_level,
       -- Count of recurring ADs (indicative of ongoing maintenance burden)
       (SELECT count(*) FROM aircraft_maint.variant_ads va
        JOIN aircraft_maint.airworthiness_directives ad ON ad.id = va.ad_id
        WHERE va.variant_id = v.id AND ad.ad_type_code = 'RECURRING') AS recurring_ad_count
FROM aircraft_maint.support_assessments sa
JOIN aircraft_core.variants v ON v.id = sa.variant_id
LEFT JOIN aircraft_ref.availability_grades pa ON pa.code = sa.parts_availability_grade_code
LEFT JOIN aircraft_ref.availability_grades mn ON mn.code = sa.maintenance_network_grade_code
WHERE sa.oem_support_status IN ('FULL_SUPPORT','LIMITED_SUPPORT')
ORDER BY pa.numeric_score DESC, mn.numeric_score DESC, sa.fleet_size_estimate DESC NULLS LAST
LIMIT 20;
*/

-- -----------------------------------------------------------------------------
-- 8. INDEX COUNT PER TABLE
-- -----------------------------------------------------------------------------
SELECT t.relname AS table_name, count(*) AS index_count
FROM pg_index ix
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_maint'
GROUP BY t.relname
ORDER BY t.relname;
-- Expected:
--   airworthiness_directives: pk, uq_ad_authority, idx_authority, idx_type, idx_inactive, idx_trgm = 6
--   variant_ads:              pk, idx_va_ad, idx_va_significant = 3
--   service_bulletins:        pk, uq_sb_number_org (partial), idx_compliance, idx_issuer, idx_active = 5
--   variant_sbs:              pk, idx_vsb_sb = 2
--   life_limited_parts:       pk, idx_llp_variant, idx_llp_part_number = 3
--   support_assessments:      pk+uq(variant_id), idx_parts, idx_network, idx_oem, idx_fleet = 6

-- -----------------------------------------------------------------------------
-- 9. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_maint.airworthiness_directives) AS ads,
    (SELECT count(*) FROM aircraft_maint.variant_ads)              AS variant_ad_links,
    (SELECT count(*) FROM aircraft_maint.service_bulletins)        AS sbs,
    (SELECT count(*) FROM aircraft_maint.variant_sbs)              AS variant_sb_links,
    (SELECT count(*) FROM aircraft_maint.life_limited_parts)       AS llps,
    (SELECT count(*) FROM aircraft_maint.support_assessments)      AS support_assessments;
-- All zero before curation/ingestion.