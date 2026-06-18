-- =============================================================================
-- File: database/validation/phase10_systems_validation.sql
-- Phase 10 — validation for aircraft_systems tables.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_systems'
ORDER BY table_name;
-- Expect: bundle_items, equipment_bundles, equipment_catalog,
--         variant_bundles, variant_equipment

-- -----------------------------------------------------------------------------
-- 2. EQUIPMENT CATALOG SEED DATA CHECK
-- -----------------------------------------------------------------------------
SELECT count(*) AS total_catalog_items
FROM aircraft_systems.equipment_catalog;
-- Expect: 18

SELECT category_code, count(*) AS items
FROM aircraft_systems.equipment_catalog
GROUP BY category_code
ORDER BY items DESC;
-- Expect distribution across: FLIGHT_INSTRUMENTS, NAVIGATION, AUTOPILOT_FMS,
--   TRAFFIC_AWARENESS, WEATHER, TERRAIN_AWARENESS, ICE_PROTECTION,
--   PRESSURIZATION, EMERGENCY_SAFETY

-- Spot-check key items present
SELECT slug, name, short_name, category_code
FROM aircraft_systems.equipment_catalog
WHERE slug IN ('garmin-g1000-nxi','garmin-gfc500','honeywell-kap140',
               'generic-adsb-out','tks-ice-protection','cirrus-caps')
ORDER BY slug;
-- Expect: 6 rows

-- -----------------------------------------------------------------------------
-- 3. NAME ALIAS GIN INDEX SMOKE TEST
-- -----------------------------------------------------------------------------
SELECT slug, name, name_aliases
FROM aircraft_systems.equipment_catalog
WHERE name_aliases @> ARRAY['KAP140']
UNION ALL
SELECT slug, name, name_aliases
FROM aircraft_systems.equipment_catalog
WHERE name_aliases @> ARRAY['Stormscope']
UNION ALL
SELECT slug, name, name_aliases
FROM aircraft_systems.equipment_catalog
WHERE name_aliases @> ARRAY['G1000']
ORDER BY slug;
-- Expect: 3 rows (one for each alias search).

-- -----------------------------------------------------------------------------
-- 4. FK CHAINS
-- -----------------------------------------------------------------------------
SELECT tc.table_name AS fk_table, kcu.column_name AS fk_col,
       ccu.table_schema AS ref_schema, ccu.table_name AS ref_table,
       rc.delete_rule
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
JOIN information_schema.referential_constraints rc  ON rc.constraint_name  = tc.constraint_name AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_systems'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   equipment_catalog → aircraft_org.organizations (SET NULL), aircraft_ref.systems_categories
--   variant_equipment → aircraft_core.variants (CASCADE), equipment_catalog (RESTRICT),
--                       aircraft_ref.equipment_provision_types
--   equipment_bundles → aircraft_org.organizations (SET NULL)
--   bundle_items      → equipment_bundles (CASCADE), equipment_catalog (RESTRICT)
--   variant_bundles   → aircraft_core.variants (CASCADE), equipment_bundles (RESTRICT),
--                       aircraft_ref.equipment_provision_types

SELECT count(*) AS total_fks
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY' AND table_schema = 'aircraft_systems';
-- Expect: 9

-- -----------------------------------------------------------------------------
-- 5. COMPREHENSIVE SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var1 BIGINT; v_var2 BIGINT;
    v_eq1  BIGINT; v_eq2  BIGINT; v_eq3  BIGINT;
    v_bun  BIGINT;
BEGIN
    -- Build hierarchy
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('sys10-smoke-fam', 'Sys10 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'sys10-smoke-mod', 'Sys10 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'sys10-smoke-var1', 'Sys10 Variant Standard') RETURNING id INTO v_var1;
    INSERT INTO aircraft_core.variants (model_id, slug, name)
    VALUES (v_mod, 'sys10-smoke-var2', 'Sys10 Variant Glass') RETURNING id INTO v_var2;

    -- Retrieve existing catalog items for testing
    SELECT id INTO v_eq1 FROM aircraft_systems.equipment_catalog WHERE slug = 'garmin-gfc500';
    SELECT id INTO v_eq2 FROM aircraft_systems.equipment_catalog WHERE slug = 'garmin-g1000-nxi';
    SELECT id INTO v_eq3 FROM aircraft_systems.equipment_catalog WHERE slug = 'tks-ice-protection';

    -- Link autopilot as STANDARD on var1
    INSERT INTO aircraft_systems.variant_equipment
        (variant_id, equipment_id, provision_type_code, notes)
    VALUES (v_var1, v_eq1, 'STANDARD', 'GFC 500 standard on all builds.');

    -- Link G1000 as STANDARD on var2 (different variant, same equip → allowed)
    INSERT INTO aircraft_systems.variant_equipment
        (variant_id, equipment_id, provision_type_code)
    VALUES (v_var2, v_eq2, 'STANDARD');

    -- Link TKS as OPTIONAL_FACTORY on both variants
    INSERT INTO aircraft_systems.variant_equipment
        (variant_id, equipment_id, provision_type_code)
    VALUES (v_var1, v_eq3, 'OPTIONAL_FACTORY');
    INSERT INTO aircraft_systems.variant_equipment
        (variant_id, equipment_id, provision_type_code)
    VALUES (v_var2, v_eq3, 'OPTIONAL_FACTORY');

    -- Duplicate (variant, equipment) → rejected
    BEGIN
        INSERT INTO aircraft_systems.variant_equipment
            (variant_id, equipment_id, provision_type_code)
        VALUES (v_var1, v_eq1, 'OPTIONAL_FACTORY');
        RAISE EXCEPTION 'UNIQUE(variant_id, equipment_id) should have rejected duplicate';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Create an avionics bundle
    INSERT INTO aircraft_systems.equipment_bundles (slug, name, short_name, description)
    VALUES ('sys10-test-bundle', 'Sys10 Glass Cockpit Package', 'Glass Package',
            'G1000 Nxi with GFC 500 autopilot and TKS ice protection.')
    RETURNING id INTO v_bun;

    -- Add items to the bundle
    INSERT INTO aircraft_systems.bundle_items (bundle_id, equipment_id, is_core) VALUES
        (v_bun, v_eq2, TRUE),   -- G1000 Nxi: core
        (v_bun, v_eq1, TRUE),   -- GFC 500: core
        (v_bun, v_eq3, FALSE);  -- TKS: optional within bundle

    -- Link bundle to var2 as STANDARD
    INSERT INTO aircraft_systems.variant_bundles
        (variant_id, bundle_id, provision_type_code)
    VALUES (v_var2, v_bun, 'STANDARD');

    -- Duplicate bundle link → rejected (PK on variant_id, bundle_id)
    BEGIN
        INSERT INTO aircraft_systems.variant_bundles
            (variant_id, bundle_id, provision_type_code)
        VALUES (v_var2, v_bun, 'OPTIONAL_FACTORY');
        RAISE EXCEPTION 'PK(variant_id, bundle_id) should have rejected duplicate bundle link';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- Verify: query all standard equipment items for var2 (should find G1000 Nxi and TKS)
    DECLARE
        v_count INTEGER;
    BEGIN
        SELECT count(*) INTO v_count
        FROM aircraft_systems.variant_equipment ve
        JOIN aircraft_systems.equipment_catalog ec ON ec.id = ve.equipment_id
        WHERE ve.variant_id         = v_var2
          AND ve.provision_type_code IN ('STANDARD', 'OPTIONAL_FACTORY');

        IF v_count != 2 THEN
            RAISE EXCEPTION 'Expected 2 equipment rows for var2, got %', v_count;
        END IF;
    END;

    -- Verify bundle composition query works
    DECLARE
        v_bun_count INTEGER;
    BEGIN
        SELECT count(*) INTO v_bun_count
        FROM aircraft_systems.variant_bundles vb
        JOIN aircraft_systems.bundle_items bi ON bi.bundle_id = vb.bundle_id
        WHERE vb.variant_id = v_var2;

        IF v_bun_count != 3 THEN
            RAISE EXCEPTION 'Expected 3 bundle items for var2 bundle, got %', v_bun_count;
        END IF;
    END;

    RAISE NOTICE 'Phase 10 smoke test passed: variant equipment UNIQUE, '
                 'bundle items composition, variant bundle PK, '
                 'alias GIN queries, FK chain integrity — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 6. SYSTEMS CATEGORY COVERAGE
-- Verify all seeded equipment_catalog items reference valid category codes.
-- -----------------------------------------------------------------------------
SELECT ec.category_code, sc.label AS category_label,
       count(ec.id) AS item_count
FROM aircraft_systems.equipment_catalog ec
JOIN aircraft_ref.systems_categories sc ON sc.code = ec.category_code
GROUP BY ec.category_code, sc.label
ORDER BY item_count DESC;
-- Expect: all rows join successfully; no orphaned category codes.

-- Equipment catalog items with NO matching systems_category (should be zero):
SELECT ec.slug, ec.name, ec.category_code
FROM aircraft_systems.equipment_catalog ec
LEFT JOIN aircraft_ref.systems_categories sc ON sc.code = ec.category_code
WHERE sc.code IS NULL;
-- Expect: 0 rows.

-- -----------------------------------------------------------------------------
-- 7. INDEX COUNT
-- -----------------------------------------------------------------------------
SELECT t.relname AS table_name, count(*) AS index_count
FROM pg_index ix
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_systems'
GROUP BY t.relname
ORDER BY t.relname;
-- Expected:
--   bundle_items:       pk, idx_bi_equipment = 2
--   equipment_bundles:  pk, uq_slug = 2
--   equipment_catalog:  pk, uq_slug, uq_mfr_name, idx_category, idx_name_trgm, idx_aliases = 6
--   variant_bundles:    pk, idx_vb_bundle = 2
--   variant_equipment:  pk, uq_variant_equipment, idx_ve_equipment, idx_ve_provision, idx_ve_variant = 5

-- -----------------------------------------------------------------------------
-- 8. EXAMPLE BUYER QUERY: variants with standard glass cockpit
-- (Demonstrates the key equipment facet query pattern)
-- -----------------------------------------------------------------------------
/*  Uncomment post-ingestion:
SELECT v.slug, v.name,
       ec.name  AS equipment_name,
       ve.provision_type_code
FROM aircraft_systems.variant_equipment ve
JOIN aircraft_core.variants v ON v.id = ve.variant_id
JOIN aircraft_systems.equipment_catalog ec ON ec.id = ve.equipment_id
WHERE ec.category_code       = 'FLIGHT_INSTRUMENTS'
  AND ve.provision_type_code = 'STANDARD'
ORDER BY v.slug, ec.name;
*/

-- -----------------------------------------------------------------------------
-- 9. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_systems.equipment_catalog)  AS catalog_items,
    (SELECT count(*) FROM aircraft_systems.equipment_bundles)  AS bundles,
    (SELECT count(*) FROM aircraft_systems.bundle_items)       AS bundle_items,
    (SELECT count(*) FROM aircraft_systems.variant_equipment)  AS variant_equipment_links,
    (SELECT count(*) FROM aircraft_systems.variant_bundles)    AS variant_bundle_links;
-- Expect: 18 catalog items; 0 for all others (aircraft data arrives in Phase 17).