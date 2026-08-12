-- =============================================================================
-- File: database/validation/phase15_16_comparison_readmodels_validation.sql
-- Phases 15 and 16 — validation for aircraft_compare tables, mission profile
-- seeds, views, and materialized views.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- PART A: PHASE 15 VALIDATION
-- -----------------------------------------------------------------------------

-- A1. TABLE EXISTENCE (4 tables)
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_compare'
ORDER BY table_name;
-- Expect: criterion_scores, mission_criteria, mission_profiles, variant_suitability

-- A2. SEED DATA COUNTS
SELECT
    (SELECT count(*) FROM aircraft_compare.mission_profiles)  AS profiles_total,
    (SELECT count(*) FROM aircraft_compare.mission_criteria)  AS criteria_rows,
    (SELECT count(*) FROM aircraft_compare.mission_profiles
     WHERE applies_to_military)                               AS military_profiles,
    (SELECT count(*) FROM aircraft_compare.mission_profiles
     WHERE applies_to_civilian)                               AS civilian_profiles;
-- Expect: 15 profiles, >30 criteria rows, 3 military, ≥12 civilian.

-- A3. WEIGHT VALIDATION (via Phase 16 view — run after Phase 16 applied)
/*
SELECT slug, criterion_count, weight_sum, weights_sum_to_one
FROM aircraft_read.v_weight_criteria_validation
ORDER BY weights_sum_to_one ASC, slug;
-- Expect: TRUE for the 5 fully-configured profiles (IFR, VFR, STOL, Business, Training).
--         stub profiles show weight_sum = 1.000 (single criterion weight = 1.000).
*/

-- A4. MISSION CRITERIA STRUCTURE FOR KEY PROFILES
SELECT mp.slug, mc.criterion_type_code, mc.weight, mc.is_required,
       mc.scoring_lower_bound, mc.scoring_upper_bound
FROM aircraft_compare.mission_criteria mc
JOIN aircraft_compare.mission_profiles mp ON mp.id = mc.mission_profile_id
WHERE mp.slug IN ('ifr-crosscountry','backcountry-stol','flight-training')
ORDER BY mp.slug, mc.weight DESC;
-- Expect: 6 rows for ifr-crosscountry, 5 for backcountry-stol, 5 for flight-training.

-- A5. is_required criteria present in STOL profile
SELECT mc.criterion_type_code, mc.is_required, mc.scoring_lower_bound, mc.scoring_upper_bound
FROM aircraft_compare.mission_criteria mc
JOIN aircraft_compare.mission_profiles mp ON mp.id = mc.mission_profile_id
WHERE mp.slug = 'backcountry-stol' AND mc.is_required;
-- Expect: 2 required criteria (CRITERION_RUNWAY_TAKEOFF and CRITERION_RUNWAY_LANDING).

-- A6. COMPREHENSIVE PHASE 15 SMOKE TEST
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_mp   BIGINT; v_vs  BIGINT;
BEGIN
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('cmp15-smoke-fam', 'Cmp15 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'cmp15-smoke-mod', 'Cmp15 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'cmp15-smoke-var', 'Cmp15 Variant') RETURNING id INTO v_var;

    SELECT id INTO v_mp FROM aircraft_compare.mission_profiles WHERE slug = 'ifr-crosscountry';

    INSERT INTO aircraft_compare.variant_suitability
        (variant_id, mission_profile_id, overall_score, is_disqualified,
         scored_criteria_count, total_criteria_count)
    VALUES (v_var, v_mp, 0.725, FALSE, 5, 6)
    RETURNING id INTO v_vs;

    -- Duplicate (variant, profile) → rejected
    BEGIN
        INSERT INTO aircraft_compare.variant_suitability
            (variant_id, mission_profile_id, overall_score, is_disqualified)
        VALUES (v_var, v_mp, 0.500, FALSE);
        RAISE EXCEPTION 'UNIQUE(variant_id, mission_profile_id) should reject duplicate';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Score out of range → rejected
    BEGIN
        INSERT INTO aircraft_compare.variant_suitability
            (variant_id, mission_profile_id, overall_score, is_disqualified)
        VALUES (v_var, (SELECT id FROM aircraft_compare.mission_profiles WHERE slug='flight-training'), 1.5, FALSE);
        RAISE EXCEPTION 'chk_vs_score should reject score > 1';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Invalid criterion weight → rejected
    BEGIN
        INSERT INTO aircraft_compare.mission_criteria
            (mission_profile_id, criterion_type_code, weight)
        VALUES (v_mp, 'CRITERION_WINGSPAN', 0.0);
        RAISE EXCEPTION 'chk_mc_weight should reject weight = 0';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 15 smoke test passed.';
    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- PART B: PHASE 16 VALIDATION
-- -----------------------------------------------------------------------------

-- B1. VIEW EXISTENCE
SELECT table_schema AS schema_name, table_name AS relation_name, 'VIEW' AS relation_type
FROM information_schema.views
WHERE table_schema = 'aircraft_read'
UNION ALL
SELECT schemaname, matviewname, 'MATERIALIZED VIEW'
FROM pg_matviews
WHERE schemaname = 'aircraft_read'
ORDER BY relation_name;

-- B2. MATVIEW COLUMN COVERAGE
-- Both materialized views are created WITH NO DATA. The first refresh must be
-- non-concurrent; subsequent application refreshes may use the default.
SELECT aircraft_read.refresh_search_matviews(FALSE);

SELECT a.attname AS column_name, pg_catalog.format_type(a.atttypid, a.atttypmod) AS data_type
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = 'aircraft_read' AND c.relname = 'mv_variant_search'
  AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum;
-- Expect: ~45 columns including search_tsv, all boolean approval flags,
--         performance metrics, weight metrics, price, manufacturer info.

-- B3. MATVIEW INDEXES
SELECT i.relname AS index_name, am.amname AS type,
       CASE WHEN pg_get_indexdef(ix.indexrelid) LIKE '%WHERE%' THEN 'partial' ELSE 'full' END
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
JOIN pg_am am ON am.oid = i.relam
WHERE n.nspname = 'aircraft_read'
ORDER BY t.relname, i.relname;
-- Expect: GIN indexes (search_tsv, family_name_trgm, variant_name_trgm),
--         UNIQUE on variant_id for both matviews,
--         B-tree range indexes on speed/range/ceiling/mtow/price/pax.

-- B4. v_current_valuation STRUCTURE
SELECT column_name FROM information_schema.columns
WHERE table_schema = 'aircraft_read' AND table_name = 'v_current_valuation'
ORDER BY ordinal_position;
-- Expect: ~16 columns matching aircraft_market.valuations.

-- B5. REFRESH FUNCTION EXISTS
SELECT routine_name, routine_type
FROM information_schema.routines
WHERE routine_schema = 'aircraft_read'
  AND routine_name   = 'refresh_search_matviews';
-- Expect: 1 row (FUNCTION).

-- B6. WEIGHT VALIDATION VIEW (requires Phase 15 seed to be loaded)
SELECT slug, weight_sum, weights_sum_to_one
FROM aircraft_read.v_weight_criteria_validation
ORDER BY weights_sum_to_one, slug;
-- Expect: all rows show weight_sum = 1.000, weights_sum_to_one = TRUE.

-- B7. SUPPORTING INDEXES ADDED IN PHASE 16
SELECT i.relname AS index_name, n.nspname AS schema_name, t.relname AS table_name
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE i.relname IN (
    'idx_pm_variant_canonical_all',
    'idx_wm_variant_standard',
    'idx_vs_rank'
);
-- Expect: 3 rows.

-- B8. HANGAR FIT VIEW SANITY
SELECT count(*) AS variants_with_wingspan
FROM aircraft_read.v_hangar_fit
WHERE wingspan_ft IS NOT NULL;
-- Expect: 0 (no data yet); non-zero after Phase 17 ingestion.

-- B9. SUMMARY
SELECT
    (SELECT count(*) FROM aircraft_compare.mission_profiles)    AS profiles,
    (SELECT count(*) FROM aircraft_compare.mission_criteria)    AS criteria,
    (SELECT count(*) FROM aircraft_compare.variant_suitability) AS suitability_scores,
    -- Matview row counts (0 until refresh_search_matviews() is called)
    (SELECT count(*) FROM aircraft_read.mv_variant_search)      AS search_matview_rows,
    (SELECT count(*) FROM aircraft_read.mv_ownership_cost_summary) AS cost_matview_rows;