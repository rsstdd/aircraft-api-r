-- =============================================================================
-- File: database/validation/011_consolidated_view_validation.sql
-- Phase 11 — validation for aircraft_military tables.
-- All data in these tables is public, unclassified reference data only.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE (7 tables expected)
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_military'
ORDER BY table_name;
-- Expect: hardpoint_compatibilities, hardpoints, loadout_items,
--         representative_loadouts, variant_mission_capabilities,
--         variant_sensors, weapons_catalog

-- -----------------------------------------------------------------------------
-- 2. WEAPONS CATALOG SEED DATA
-- -----------------------------------------------------------------------------
SELECT count(*) AS total_weapons FROM aircraft_military.weapons_catalog;
-- Expect: 17

SELECT st.code AS stores_type, wc.code AS weapon_cat,
       count(w.id) AS weapon_count
FROM aircraft_military.weapons_catalog w
JOIN aircraft_ref.stores_types st ON st.code = w.stores_type_code
JOIN aircraft_ref.weapon_categories wc ON wc.code = st.weapon_category_code
GROUP BY st.code, wc.code
ORDER BY weapon_count DESC;
-- Expect: distribution across AAM_SHORT_RANGE, AAM_MEDIUM_RANGE, AGM_LASER,
--         LGB, JDAM, UNGUIDED_BOMB, GUN_POD, EXT_FUEL_TANK, RECCE_POD, ECM_POD.
-- Note: 1 weapon (Harpoon) has NULL stores_type_code — excluded from this query.

-- Harpoon with NULL stores_type_code:
SELECT slug, name, stores_type_code
FROM aircraft_military.weapons_catalog
WHERE stores_type_code IS NULL;
-- Expect: 1 row (agm-84d-harpoon): stores_type_code NULL (no ANTI_SHIP type seeded).

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
  AND tc.table_schema    = 'aircraft_military'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   hardpoints           → aircraft_core.variants (CASCADE), aircraft_ref.hardpoint_position_types
--   weapons_catalog      → aircraft_org.organizations (SET NULL), aircraft_geo.countries,
--                          aircraft_ref.stores_types
--   hardpoint_compatibilities → hardpoints (CASCADE), aircraft_ref.stores_types
--   representative_loadouts → aircraft_core.variants (CASCADE), aircraft_ref.military_mission_types
--   loadout_items        → representative_loadouts (CASCADE), hardpoints (RESTRICT),
--                          weapons_catalog (SET NULL), aircraft_ref.stores_types
--   variant_mission_capabilities → aircraft_core.variants (CASCADE), aircraft_ref.military_mission_types
--   variant_sensors      → aircraft_core.variants (CASCADE), aircraft_systems.equipment_catalog (SET NULL),
--                          aircraft_ref.equipment_provision_types

-- Cross-schema FK (variant_sensors → aircraft_systems):
SELECT tc.table_name, kcu.column_name, ccu.table_schema AS ref_schema, ccu.table_name AS ref_table
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
JOIN information_schema.referential_constraints rc  ON rc.constraint_name  = tc.constraint_name AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_military'
  AND ccu.table_schema   = 'aircraft_systems';
-- Expect: 1 row — variant_sensors.equipment_id → aircraft_systems.equipment_catalog

-- -----------------------------------------------------------------------------
-- 4. CHECK CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name
FROM information_schema.table_constraints tc
WHERE tc.table_schema    = 'aircraft_military'
  AND tc.constraint_type = 'CHECK'
ORDER BY tc.table_name, tc.constraint_name;
-- Expect: chk_hp_loads, chk_wc_dims, chk_rl_fuel_pct, chk_rl_weight_nonneg,
--         chk_li_has_store, chk_li_quantity, chk_vs_has_sensor

-- -----------------------------------------------------------------------------
-- 5. COMPREHENSIVE SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_hp1  BIGINT; v_hp2 BIGINT;
    v_wep  BIGINT; v_ld  BIGINT;
BEGIN
    -- Hierarchy
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('mil11-smoke-fam', 'Mil11 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'mil11-smoke-mod', 'Mil11 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name, propulsion_category_code)
    VALUES (v_mod, 'mil11-smoke-var', 'Mil11 Fighter', 'TURBOFAN_LOW_BPR')
    RETURNING id INTO v_var;

    -- Create hardpoints
    INSERT INTO aircraft_military.hardpoints
        (variant_id, station_number, station_label, position_type_code,
         max_load_lbs, is_wet, is_internal_bay)
    VALUES (v_var, 'STA 5', 'Fuselage Centreline', 'FUSELAGE_CENTERLINE', 2500, TRUE, FALSE)
    RETURNING id INTO v_hp1;

    INSERT INTO aircraft_military.hardpoints
        (variant_id, station_number, station_label, position_type_code,
         max_load_lbs, is_wet)
    VALUES (v_var, 'STA 2', 'Left Wing Inboard', 'WING_INBOARD', 3500, FALSE)
    RETURNING id INTO v_hp2;

    -- Duplicate station_number → rejected
    BEGIN
        INSERT INTO aircraft_military.hardpoints
            (variant_id, station_number, position_type_code)
        VALUES (v_var, 'STA 5', 'FUSELAGE_CENTERLINE');
        RAISE EXCEPTION 'UNIQUE(variant_id, station_number) should reject duplicate station';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Hardpoint compatibility
    INSERT INTO aircraft_military.hardpoint_compatibilities
        (hardpoint_id, stores_type_code, max_stores_weight_lbs)
    VALUES
        (v_hp1, 'EXT_FUEL_TANK', 2000),
        (v_hp1, 'JDAM',          2000),
        (v_hp2, 'LGB',           2000),
        (v_hp2, 'AAM_SHORT_RANGE', 500);

    -- Duplicate compatibility → rejected (PK)
    BEGIN
        INSERT INTO aircraft_military.hardpoint_compatibilities
            (hardpoint_id, stores_type_code)
        VALUES (v_hp1, 'JDAM');
        RAISE EXCEPTION 'PK(hardpoint_id, stores_type_code) should reject duplicate compatibility';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Mission capability
    INSERT INTO aircraft_military.variant_mission_capabilities
        (variant_id, mission_type_code, is_primary_mission, confidence, notes)
    VALUES
        (v_var, 'AIR_SUPERIORITY', TRUE,  0.90, 'Primary design role per manufacturer data.'),
        (v_var, 'CLOSE_AIR_SUPPORT', FALSE, 0.75, 'Secondary CAS capability per Jane''s 2023.');

    -- Second primary mission → rejected (partial UNIQUE)
    BEGIN
        INSERT INTO aircraft_military.variant_mission_capabilities
            (variant_id, mission_type_code, is_primary_mission)
        VALUES (v_var, 'ELECTRONIC_WARFARE', TRUE);
        RAISE EXCEPTION 'uq_vmc_primary should reject second primary_mission';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Representative loadout
    INSERT INTO aircraft_military.representative_loadouts
        (variant_id, name, mission_type_code, total_stores_weight_lbs,
         source_notes, confidence)
    VALUES
        (v_var, 'Air Superiority (2×AIM-120C + 2×AIM-9X)', 'AIR_SUPERIORITY',
         1046, 'Jane''s All the World''s Aircraft 2023, p.412', 0.85)
    RETURNING id INTO v_ld;

    -- Retrieve an AIM-9X weapon
    SELECT id INTO v_wep FROM aircraft_military.weapons_catalog WHERE slug = 'aim-9x-sidewinder';

    -- Loadout items: a generic tank uses the stores-type fallback, while
    -- the wing station references a catalog weapon.
    INSERT INTO aircraft_military.loadout_items
        (loadout_id, hardpoint_id, weapon_id, stores_type_code, quantity)
    VALUES
        (v_ld, v_hp1, NULL, 'EXT_FUEL_TANK', 1),
        (v_ld, v_hp2, v_wep, NULL, 1);

    -- Duplicate (loadout, hardpoint) → rejected (PK)
    BEGIN
        INSERT INTO aircraft_military.loadout_items
            (loadout_id, hardpoint_id, stores_type_code, quantity)
        VALUES (v_ld, v_hp1, 'JDAM', 1);
        RAISE EXCEPTION 'PK(loadout_id, hardpoint_id) should reject second item on same hardpoint';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- chk_li_has_store: clearing both references must be rejected.
    BEGIN
        UPDATE aircraft_military.loadout_items
        SET weapon_id = NULL, stores_type_code = NULL
        WHERE loadout_id = v_ld AND hardpoint_id = v_hp2;
        RAISE EXCEPTION 'chk_li_has_store should reject a row with no weapon or stores_type';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Variant sensor
    DECLARE v_eq_id BIGINT;
    BEGIN
        SELECT id INTO v_eq_id
        FROM aircraft_systems.equipment_catalog
        WHERE slug = 'garmin-gwx75';

        INSERT INTO aircraft_military.variant_sensors
            (variant_id, equipment_id, is_internal, provision_type_code, confidence, notes)
        VALUES (v_var, v_eq_id, FALSE, 'STANDARD', 0.75, 'Podded weather radar; public reference.');
    END;

    -- chk_vs_has_sensor: no equipment_id AND no sensor_name_raw → rejected
    BEGIN
        INSERT INTO aircraft_military.variant_sensors
            (variant_id, equipment_id, sensor_name_raw, is_internal)
        VALUES (v_var, NULL, NULL, TRUE);
        RAISE EXCEPTION 'chk_vs_has_sensor should reject row with no equipment or name';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Negative hardpoint load → rejected
    BEGIN
        INSERT INTO aircraft_military.hardpoints
            (variant_id, station_number, position_type_code, max_load_lbs)
        VALUES (v_var, 'STA_BAD', 'WINGTIP', -100);
        RAISE EXCEPTION 'chk_hp_loads should reject negative max_load_lbs';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 11 smoke test passed: hardpoint UNIQUE, station compat PK, '
                 'mission primary unique, loadout item PK + has_store CHECK, '
                 'sensor has_sensor CHECK, load nonneg CHECK — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 6. WEAPONS CATALOG BY COUNTRY OF ORIGIN
-- -----------------------------------------------------------------------------
SELECT c.name AS country, count(w.id) AS weapon_count
FROM aircraft_military.weapons_catalog w
JOIN aircraft_geo.countries c ON c.code = w.country_of_origin_code
GROUP BY c.name
ORDER BY weapon_count DESC;
-- Expect: USA (majority), GBR (1: ASRAAM), RUS (1: R-73), ISR (1: Litening)

-- -----------------------------------------------------------------------------
-- 7. INDEX COUNT PER TABLE
-- -----------------------------------------------------------------------------
SELECT t.relname AS table_name, count(*) AS index_count
FROM pg_index ix
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_military'
GROUP BY t.relname
ORDER BY t.relname;

-- -----------------------------------------------------------------------------
-- 8. SCOPE ENFORCEMENT QUERY
-- Verify weapons_catalog has no performance data columns (classified proxy check).
-- All numeric columns should be physical only: weight, length, diameter.
-- -----------------------------------------------------------------------------
SELECT column_name, data_type
FROM information_schema.columns
WHERE table_schema = 'aircraft_military'
  AND table_name   = 'weapons_catalog'
ORDER BY ordinal_position;
-- Verify: NO columns named cep, miss_distance, pk, seeker_range, fuze_mode,
--         yield, warhead_type_classified, etc.

-- -----------------------------------------------------------------------------
-- 9. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_military.weapons_catalog)             AS weapons_seeded,
    (SELECT count(*) FROM aircraft_military.hardpoints)                  AS hardpoints,
    (SELECT count(*) FROM aircraft_military.hardpoint_compatibilities)   AS hp_compat_rows,
    (SELECT count(*) FROM aircraft_military.representative_loadouts)     AS loadouts,
    (SELECT count(*) FROM aircraft_military.loadout_items)               AS loadout_items,
    (SELECT count(*) FROM aircraft_military.variant_mission_capabilities) AS mission_caps,
    (SELECT count(*) FROM aircraft_military.variant_sensors)             AS sensor_links;
-- Expect: 17 weapons seeded; 0 for all aircraft-linked tables (data arrives Phase 17).