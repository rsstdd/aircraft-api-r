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

-- A2. SEED DATA COMPLETENESS
-- Keep this policy check synchronized with
-- database/seeds/003_mission_profile_seed_data.sql and
-- crates/aircraft_testsupport/tests/seed_data.rs.
DO $validation$
DECLARE
    invalid_count BIGINT;
BEGIN
    IF (SELECT count(*) FROM aircraft_compare.mission_profiles) <> 15 THEN
        RAISE EXCEPTION 'mission profile count must be exactly 15';
    END IF;
    IF (SELECT count(*) FROM aircraft_compare.mission_criteria) <> 88 THEN
        RAISE EXCEPTION 'mission criterion count must be exactly 88';
    END IF;
    IF (SELECT count(*) FROM aircraft_compare.mission_profiles
        WHERE applies_to_military) <> 4 THEN
        RAISE EXCEPTION 'military mission profile count must be exactly 4';
    END IF;
    IF (SELECT count(*) FROM aircraft_compare.mission_profiles
        WHERE applies_to_civilian) <> 12 THEN
        RAISE EXCEPTION 'civilian mission profile count must be exactly 12';
    END IF;

    SELECT count(*) INTO invalid_count
    FROM aircraft_compare.mission_profiles
    WHERE btrim(slug) = '' OR btrim(title) = ''
       OR description IS NULL OR btrim(description) = ''
       OR typical_range_nm IS NULL OR typical_pax_count IS NULL
       OR typical_altitude_ft IS NULL OR NOT is_active;
    IF invalid_count <> 0 THEN
        RAISE EXCEPTION '% mission profiles have incomplete canonical data', invalid_count;
    END IF;

    SELECT count(*) INTO invalid_count
    FROM aircraft_compare.mission_criteria
    WHERE scoring_lower_bound IS NULL OR scoring_upper_bound IS NULL
       OR scoring_lower_bound >= scoring_upper_bound
       OR notes IS NULL OR btrim(notes) = '' OR notes ILIKE '%stub%';
    IF invalid_count <> 0 THEN
        RAISE EXCEPTION '% mission criteria have invalid bounds or notes', invalid_count;
    END IF;

    SELECT count(*) INTO invalid_count
    FROM (
        SELECT profile.id
        FROM aircraft_compare.mission_profiles AS profile
        JOIN aircraft_compare.mission_criteria AS criterion
          ON criterion.mission_profile_id = profile.id
        GROUP BY profile.id
        HAVING sum(criterion.weight) <> 1.000
           OR count(*) NOT IN (5, 6)
           OR (profile.profile_type_code NOT IN ('BACKCOUNTRY_STOL', 'FLIGHT_TRAINING')
               AND count(*) <> 6)
    ) AS invalid_profiles;
    IF invalid_count <> 0 THEN
        RAISE EXCEPTION '% mission profiles have invalid criterion counts or weights',
            invalid_count;
    END IF;

    IF EXISTS (
        (SELECT profile.profile_type_code, criterion.criterion_type_code,
                criterion.weight, criterion.is_required,
                criterion.scoring_lower_bound, criterion.scoring_upper_bound
         FROM aircraft_compare.mission_criteria AS criterion
         JOIN aircraft_compare.mission_profiles AS profile
           ON profile.id = criterion.mission_profile_id
         WHERE profile.profile_type_code IN (
             'FLOATPLANE_OPERATIONS', 'CARGO_FREIGHT', 'MEDEVAC_SAR',
             'PATROL_SURVEILLANCE', 'HIGH_ALTITUDE_OPS', 'AEROBATICS',
             'MILITARY_CLOSE_AIR_SUPPORT', 'MILITARY_TRANSPORT_AIRLIFT',
             'MILITARY_MARITIME_PATROL', 'UNMANNED_SPECIAL_MISSION')
         EXCEPT VALUES
           ('FLOATPLANE_OPERATIONS'::aircraft_ref.lookup_code, 'CRITERION_CRUISE_SPEED'::aircraft_ref.lookup_code, 0.100::NUMERIC, FALSE, 70::NUMERIC, 180::NUMERIC),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_RANGE', 0.250, FALSE, 150, 800),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_PAYLOAD', 0.200, FALSE, 300, 2000),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_RUNWAY_TAKEOFF', 0.200, FALSE, 500, 3000),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_RUNWAY_LANDING', 0.150, FALSE, 500, 3000),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_HOURLY_COST', 0.100, FALSE, 50, 500),
           ('CARGO_FREIGHT', 'CRITERION_CRUISE_SPEED', 0.050, FALSE, 100, 450),
           ('CARGO_FREIGHT', 'CRITERION_RANGE', 0.250, TRUE, 500, 4000),
           ('CARGO_FREIGHT', 'CRITERION_PAYLOAD', 0.350, TRUE, 1000, 50000),
           ('CARGO_FREIGHT', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 1500, 8000),
           ('CARGO_FREIGHT', 'CRITERION_RUNWAY_LANDING', 0.100, FALSE, 1500, 8000),
           ('CARGO_FREIGHT', 'CRITERION_HOURLY_COST', 0.150, FALSE, 200, 10000),
           ('MEDEVAC_SAR', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 100, 300),
           ('MEDEVAC_SAR', 'CRITERION_RANGE', 0.200, TRUE, 300, 1500),
           ('MEDEVAC_SAR', 'CRITERION_CEILING', 0.100, FALSE, 10000, 30000),
           ('MEDEVAC_SAR', 'CRITERION_PAYLOAD', 0.150, FALSE, 500, 3000),
           ('MEDEVAC_SAR', 'CRITERION_RUNWAY_TAKEOFF', 0.200, TRUE, 500, 3000),
           ('MEDEVAC_SAR', 'CRITERION_RUNWAY_LANDING', 0.200, TRUE, 500, 3000),
           ('PATROL_SURVEILLANCE', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 80, 350),
           ('PATROL_SURVEILLANCE', 'CRITERION_RANGE', 0.300, TRUE, 500, 4000),
           ('PATROL_SURVEILLANCE', 'CRITERION_CEILING', 0.150, FALSE, 10000, 40000),
           ('PATROL_SURVEILLANCE', 'CRITERION_FUEL_EFFICIENCY', 0.200, FALSE, 2, 20),
           ('PATROL_SURVEILLANCE', 'CRITERION_PAYLOAD', 0.150, FALSE, 500, 10000),
           ('PATROL_SURVEILLANCE', 'CRITERION_HOURLY_COST', 0.100, FALSE, 100, 5000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 100, 300),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_RANGE', 0.150, FALSE, 300, 2000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_CEILING', 0.400, TRUE, 12000, 40000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_CLIMB_RATE', 0.200, FALSE, 500, 3000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_PAYLOAD', 0.100, FALSE, 300, 3000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_RUNWAY_TAKEOFF', 0.050, FALSE, 1000, 5000),
           ('AEROBATICS', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 100, 300),
           ('AEROBATICS', 'CRITERION_RANGE', 0.100, FALSE, 100, 800),
           ('AEROBATICS', 'CRITERION_CLIMB_RATE', 0.300, FALSE, 1000, 5000),
           ('AEROBATICS', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 500, 3000),
           ('AEROBATICS', 'CRITERION_PRICE', 0.200, FALSE, 50000, 1000000),
           ('AEROBATICS', 'CRITERION_HOURLY_COST', 0.150, FALSE, 100, 1000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 250, 700),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_RANGE', 0.200, FALSE, 300, 2000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_CLIMB_RATE', 0.150, FALSE, 3000, 15000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_PAYLOAD', 0.300, FALSE, 2000, 20000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 1500, 6000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_HOURLY_COST', 0.100, FALSE, 1000, 30000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 200, 550),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RANGE', 0.250, TRUE, 1000, 6000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_CEILING', 0.050, FALSE, 20000, 45000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_PAYLOAD', 0.350, TRUE, 10000, 200000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RUNWAY_TAKEOFF', 0.150, FALSE, 2000, 8000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RUNWAY_LANDING', 0.100, FALSE, 2000, 8000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 180, 500),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_RANGE', 0.350, TRUE, 1000, 6000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_CEILING', 0.100, FALSE, 15000, 45000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_FUEL_EFFICIENCY', 0.150, FALSE, 0.5, 10),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_PAYLOAD', 0.200, FALSE, 2000, 30000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_HOURLY_COST', 0.100, FALSE, 1000, 30000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 80, 400),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_RANGE', 0.300, TRUE, 500, 6000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CEILING', 0.250, TRUE, 20000, 60000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CLIMB_RATE', 0.050, FALSE, 500, 5000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_PAYLOAD', 0.200, FALSE, 100, 5000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_HOURLY_COST', 0.100, FALSE, 50, 5000))
        UNION ALL
        (VALUES
           ('FLOATPLANE_OPERATIONS'::aircraft_ref.lookup_code, 'CRITERION_CRUISE_SPEED'::aircraft_ref.lookup_code, 0.100::NUMERIC, FALSE, 70::NUMERIC, 180::NUMERIC),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_RANGE', 0.250, FALSE, 150, 800),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_PAYLOAD', 0.200, FALSE, 300, 2000),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_RUNWAY_TAKEOFF', 0.200, FALSE, 500, 3000),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_RUNWAY_LANDING', 0.150, FALSE, 500, 3000),
           ('FLOATPLANE_OPERATIONS', 'CRITERION_HOURLY_COST', 0.100, FALSE, 50, 500),
           ('CARGO_FREIGHT', 'CRITERION_CRUISE_SPEED', 0.050, FALSE, 100, 450),
           ('CARGO_FREIGHT', 'CRITERION_RANGE', 0.250, TRUE, 500, 4000),
           ('CARGO_FREIGHT', 'CRITERION_PAYLOAD', 0.350, TRUE, 1000, 50000),
           ('CARGO_FREIGHT', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 1500, 8000),
           ('CARGO_FREIGHT', 'CRITERION_RUNWAY_LANDING', 0.100, FALSE, 1500, 8000),
           ('CARGO_FREIGHT', 'CRITERION_HOURLY_COST', 0.150, FALSE, 200, 10000),
           ('MEDEVAC_SAR', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 100, 300),
           ('MEDEVAC_SAR', 'CRITERION_RANGE', 0.200, TRUE, 300, 1500),
           ('MEDEVAC_SAR', 'CRITERION_CEILING', 0.100, FALSE, 10000, 30000),
           ('MEDEVAC_SAR', 'CRITERION_PAYLOAD', 0.150, FALSE, 500, 3000),
           ('MEDEVAC_SAR', 'CRITERION_RUNWAY_TAKEOFF', 0.200, TRUE, 500, 3000),
           ('MEDEVAC_SAR', 'CRITERION_RUNWAY_LANDING', 0.200, TRUE, 500, 3000),
           ('PATROL_SURVEILLANCE', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 80, 350),
           ('PATROL_SURVEILLANCE', 'CRITERION_RANGE', 0.300, TRUE, 500, 4000),
           ('PATROL_SURVEILLANCE', 'CRITERION_CEILING', 0.150, FALSE, 10000, 40000),
           ('PATROL_SURVEILLANCE', 'CRITERION_FUEL_EFFICIENCY', 0.200, FALSE, 2, 20),
           ('PATROL_SURVEILLANCE', 'CRITERION_PAYLOAD', 0.150, FALSE, 500, 10000),
           ('PATROL_SURVEILLANCE', 'CRITERION_HOURLY_COST', 0.100, FALSE, 100, 5000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 100, 300),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_RANGE', 0.150, FALSE, 300, 2000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_CEILING', 0.400, TRUE, 12000, 40000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_CLIMB_RATE', 0.200, FALSE, 500, 3000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_PAYLOAD', 0.100, FALSE, 300, 3000),
           ('HIGH_ALTITUDE_OPS', 'CRITERION_RUNWAY_TAKEOFF', 0.050, FALSE, 1000, 5000),
           ('AEROBATICS', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 100, 300),
           ('AEROBATICS', 'CRITERION_RANGE', 0.100, FALSE, 100, 800),
           ('AEROBATICS', 'CRITERION_CLIMB_RATE', 0.300, FALSE, 1000, 5000),
           ('AEROBATICS', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 500, 3000),
           ('AEROBATICS', 'CRITERION_PRICE', 0.200, FALSE, 50000, 1000000),
           ('AEROBATICS', 'CRITERION_HOURLY_COST', 0.150, FALSE, 100, 1000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_CRUISE_SPEED', 0.150, FALSE, 250, 700),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_RANGE', 0.200, FALSE, 300, 2000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_CLIMB_RATE', 0.150, FALSE, 3000, 15000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_PAYLOAD', 0.300, FALSE, 2000, 20000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_RUNWAY_TAKEOFF', 0.100, FALSE, 1500, 6000),
           ('MILITARY_CLOSE_AIR_SUPPORT', 'CRITERION_HOURLY_COST', 0.100, FALSE, 1000, 30000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 200, 550),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RANGE', 0.250, TRUE, 1000, 6000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_CEILING', 0.050, FALSE, 20000, 45000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_PAYLOAD', 0.350, TRUE, 10000, 200000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RUNWAY_TAKEOFF', 0.150, FALSE, 2000, 8000),
           ('MILITARY_TRANSPORT_AIRLIFT', 'CRITERION_RUNWAY_LANDING', 0.100, FALSE, 2000, 8000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 180, 500),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_RANGE', 0.350, TRUE, 1000, 6000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_CEILING', 0.100, FALSE, 15000, 45000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_FUEL_EFFICIENCY', 0.150, FALSE, 0.5, 10),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_PAYLOAD', 0.200, FALSE, 2000, 30000),
           ('MILITARY_MARITIME_PATROL', 'CRITERION_HOURLY_COST', 0.100, FALSE, 1000, 30000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CRUISE_SPEED', 0.100, FALSE, 80, 400),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_RANGE', 0.300, TRUE, 500, 6000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CEILING', 0.250, TRUE, 20000, 60000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_CLIMB_RATE', 0.050, FALSE, 500, 5000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_PAYLOAD', 0.200, FALSE, 100, 5000),
           ('UNMANNED_SPECIAL_MISSION', 'CRITERION_HOURLY_COST', 0.100, FALSE, 50, 5000)
         EXCEPT
         SELECT profile.profile_type_code, criterion.criterion_type_code,
                criterion.weight, criterion.is_required,
                criterion.scoring_lower_bound, criterion.scoring_upper_bound
         FROM aircraft_compare.mission_criteria AS criterion
         JOIN aircraft_compare.mission_profiles AS profile
           ON profile.id = criterion.mission_profile_id
         WHERE profile.profile_type_code IN (
             'FLOATPLANE_OPERATIONS', 'CARGO_FREIGHT', 'MEDEVAC_SAR',
             'PATROL_SURVEILLANCE', 'HIGH_ALTITUDE_OPS', 'AEROBATICS',
             'MILITARY_CLOSE_AIR_SUPPORT', 'MILITARY_TRANSPORT_AIRLIFT',
             'MILITARY_MARITIME_PATROL', 'UNMANNED_SPECIAL_MISSION'))
    ) THEN
        RAISE EXCEPTION 'six-criterion v1 mission policy differs from the repository baseline';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM aircraft_compare.mission_criteria AS criterion
        JOIN aircraft_compare.mission_profiles AS profile
          ON profile.id = criterion.mission_profile_id
        WHERE profile.profile_type_code = 'PERSONAL_VFR_TOURING'
          AND criterion.criterion_type_code = 'CRITERION_FUEL_EFFICIENCY'
          AND criterion.scoring_lower_bound = 5 AND criterion.scoring_upper_bound = 25
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_compare.mission_criteria AS criterion
        JOIN aircraft_compare.mission_profiles AS profile
          ON profile.id = criterion.mission_profile_id
        WHERE profile.profile_type_code = 'IFR_CROSSCOUNTRY'
          AND criterion.criterion_type_code = 'CRITERION_FUEL_EFFICIENCY'
          AND criterion.scoring_lower_bound = 4 AND criterion.scoring_upper_bound = 20
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_compare.mission_criteria AS criterion
        JOIN aircraft_compare.mission_profiles AS profile
          ON profile.id = criterion.mission_profile_id
        WHERE profile.profile_type_code = 'IFR_CROSSCOUNTRY'
          AND criterion.criterion_type_code = 'CRITERION_PRICE'
          AND criterion.scoring_lower_bound = 50000
          AND criterion.scoring_upper_bound = 1000000
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_compare.mission_criteria AS criterion
        JOIN aircraft_compare.mission_profiles AS profile
          ON profile.id = criterion.mission_profile_id
        WHERE profile.profile_type_code = 'BACKCOUNTRY_STOL'
          AND criterion.criterion_type_code = 'CRITERION_PRICE'
          AND criterion.scoring_lower_bound = 30000
          AND criterion.scoring_upper_bound = 500000
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_compare.mission_criteria AS criterion
        JOIN aircraft_compare.mission_profiles AS profile
          ON profile.id = criterion.mission_profile_id
        WHERE profile.profile_type_code = 'BUSINESS_TRAVEL'
          AND criterion.criterion_type_code = 'CRITERION_HOURLY_COST'
          AND criterion.scoring_lower_bound = 100
          AND criterion.scoring_upper_bound = 2500
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_compare.mission_criteria AS criterion
        JOIN aircraft_compare.mission_profiles AS profile
          ON profile.id = criterion.mission_profile_id
        WHERE profile.profile_type_code = 'FLIGHT_TRAINING'
          AND criterion.criterion_type_code = 'CRITERION_HOURLY_COST'
          AND criterion.scoring_lower_bound = 30
          AND criterion.scoring_upper_bound = 250
    ) THEN
        RAISE EXCEPTION 'configured-profile scoring bounds differ from v1 policy';
    END IF;
END
$validation$;

-- A3. WEIGHT VALIDATION (via Phase 16 view — run after Phase 16 applied)
/*
SELECT slug, criterion_count, weight_sum, weights_sum_to_one
FROM aircraft_read.v_weight_criteria_validation
ORDER BY weights_sum_to_one ASC, slug;
-- Expect: TRUE for every profile.
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

DO $validation$
BEGIN
    IF EXISTS (
        SELECT 1 FROM aircraft_read.v_weight_criteria_validation
        WHERE NOT weights_sum_to_one OR weight_sum <> 1.000
    ) THEN
        RAISE EXCEPTION 'mission criterion weights must sum exactly to 1.000';
    END IF;
END
$validation$;

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
