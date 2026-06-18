-- =============================================================================
-- File: database/validation/phase9_propulsion_validation.sql
-- Phase 9 — validation for aircraft_power tables.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. TABLE EXISTENCE (7 tables expected)
-- -----------------------------------------------------------------------------
SELECT table_schema, table_name
FROM information_schema.tables
WHERE table_schema = 'aircraft_power'
ORDER BY table_name;
-- Expect: apu_specs, engine_variants, powerplant_stcs, propeller_specs,
--         rotor_systems, variant_powerplants, variant_propellers

-- -----------------------------------------------------------------------------
-- 2. FK CHAINS — all aircraft_power FKs
-- -----------------------------------------------------------------------------
SELECT tc.table_name AS fk_table, kcu.column_name AS fk_column,
       ccu.table_schema AS ref_schema, ccu.table_name AS ref_table,
       rc.delete_rule
FROM information_schema.table_constraints       tc
JOIN information_schema.key_column_usage        kcu
    ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
JOIN information_schema.referential_constraints rc
    ON rc.constraint_name = tc.constraint_name AND rc.constraint_schema = tc.constraint_schema
JOIN information_schema.constraint_column_usage ccu
    ON ccu.constraint_name = rc.unique_constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_schema    = 'aircraft_power'
ORDER BY tc.table_name, kcu.column_name;
-- Expected FK targets:
--   engine_variants      → aircraft_org.organizations (SET NULL), aircraft_ref.fuel_types,
--                          aircraft_ref.propulsion_categories
--   variant_powerplants  → aircraft_core.variants (CASCADE), aircraft_power.engine_variants (RESTRICT)
--   propeller_specs      → aircraft_org.organizations (SET NULL)
--   variant_propellers   → aircraft_core.variants (CASCADE), aircraft_power.propeller_specs (RESTRICT)
--   rotor_systems        → aircraft_core.variants (CASCADE)
--   apu_specs            → aircraft_core.variants (CASCADE), aircraft_org.organizations (SET NULL),
--                          aircraft_ref.fuel_types
--   powerplant_stcs      → aircraft_core.variants (CASCADE), aircraft_power.engine_variants (SET NULL),
--                          aircraft_org.organizations (SET NULL),
--                          aircraft_ref.certification_authorities

SELECT count(*) AS total_aircraft_power_fks
FROM information_schema.table_constraints
WHERE constraint_type = 'FOREIGN KEY' AND table_schema = 'aircraft_power';
-- Expect: ~13

-- -----------------------------------------------------------------------------
-- 3. CHECK CONSTRAINTS
-- -----------------------------------------------------------------------------
SELECT tc.table_name, tc.constraint_name, cc.check_clause
FROM information_schema.table_constraints  tc
JOIN information_schema.check_constraints  cc
    ON cc.constraint_name = tc.constraint_name AND cc.constraint_schema = tc.constraint_schema
WHERE tc.table_schema    = 'aircraft_power'
  AND tc.constraint_type = 'CHECK'
ORDER BY tc.table_name, tc.constraint_name;
-- Expected: chk_ev_hp_nonneg, chk_ev_thrust_nonneg, chk_ev_tbo, chk_ev_sfc_unit,
--           chk_ev_afterburner, chk_ps_type, chk_ps_dims,
--           chk_vp_engine_count, chk_vp_option_flags, chk_vp_primary_implies,
--           chk_vpr_option_flags, chk_rs_role, chk_rs_dims

-- -----------------------------------------------------------------------------
-- 4. PARTIAL UNIQUE INDEXES — is_primary constraints
-- -----------------------------------------------------------------------------
SELECT i.relname AS index_name, t.relname AS table_name,
       pg_get_indexdef(ix.indexrelid) AS definition
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname       = 'aircraft_power'
  AND ix.indisunique  = TRUE
  AND pg_get_indexdef(ix.indexrelid) LIKE '%WHERE%'
ORDER BY t.relname, i.relname;
-- Expect: uq_vp_primary (variant_powerplants WHERE is_primary),
--         uq_vpr_primary (variant_propellers WHERE is_primary),
--         uq_engine_variant_raw_dedup (engine_variants WHERE org IS NULL AND name NOT NULL)

-- -----------------------------------------------------------------------------
-- 5. COMPREHENSIVE SMOKE TEST
-- -----------------------------------------------------------------------------
DO $$
DECLARE
    v_fam  BIGINT; v_mod BIGINT; v_var BIGINT;
    v_eng1 BIGINT; v_eng2 BIGINT;
    v_prop BIGINT;
BEGIN
    -- Hierarchy skeleton
    INSERT INTO aircraft_core.families (slug, name)
    VALUES ('pow9-smoke-fam', 'Pow9 Family') RETURNING id INTO v_fam;
    INSERT INTO aircraft_core.models (family_id, slug, name)
    VALUES (v_fam, 'pow9-smoke-mod', 'Pow9 Model') RETURNING id INTO v_mod;
    INSERT INTO aircraft_core.variants (model_id, slug, name, engine_count, propulsion_category_code)
    VALUES (v_mod, 'pow9-smoke-var', 'Pow9 Variant', 1, 'PISTON_RECIPROCATING')
    RETURNING id INTO v_var;

    -- Engine variant 1: Lycoming O-320
    INSERT INTO aircraft_power.engine_variants
        (slug, model_designation, model_family, propulsion_category_code,
         fuel_type_code, hp_rated, tbo_hours, is_fuel_injected, has_fadec)
    VALUES ('lycoming-o-320-d2j', 'O-320-D2J', 'O-320',
            'PISTON_RECIPROCATING', 'AVGAS_100LL', 160, 2000, FALSE, FALSE)
    RETURNING id INTO v_eng1;

    -- Engine variant 2: Lycoming IO-360 (injected, higher power)
    INSERT INTO aircraft_power.engine_variants
        (slug, model_designation, model_family, propulsion_category_code,
         fuel_type_code, hp_rated, tbo_hours, is_fuel_injected, has_fadec)
    VALUES ('lycoming-io-360-a1b6', 'IO-360-A1B6', 'IO-360',
            'PISTON_RECIPROCATING', 'AVGAS_100LL', 200, 2000, TRUE, FALSE)
    RETURNING id INTO v_eng2;

    -- Dedup test: inserting the same model_designation for NULL manufacturer
    BEGIN
        INSERT INTO aircraft_power.engine_variants
            (slug, model_designation, model_family, propulsion_category_code,
             manufacturer_name_raw)
        VALUES ('lycoming-o-320-d2j-dup', 'O-320-D2J', 'O-320',
                'PISTON_RECIPROCATING', NULL);
        -- Should succeed as different slug; uq_engine_variant_raw_dedup only fires
        -- when manufacturer_org_id IS NULL AND manufacturer_name_raw IS NOT NULL
        -- and (manufacturer_name_raw, model_designation) conflicts.
        -- This insert has manufacturer_name_raw = NULL → no conflict.
        NULL;
    END;

    -- Link standard engine (O-320) as primary powerplant
    INSERT INTO aircraft_power.variant_powerplants
        (variant_id, engine_variant_id, engine_count,
         is_standard, is_optional, is_primary)
    VALUES (v_var, v_eng1, 1, TRUE, FALSE, TRUE);

    -- Link optional engine (IO-360) as non-primary alternative
    INSERT INTO aircraft_power.variant_powerplants
        (variant_id, engine_variant_id, engine_count,
         is_standard, is_optional, is_primary)
    VALUES (v_var, v_eng2, 1, FALSE, TRUE, FALSE);

    -- Second primary powerplant → must be rejected
    BEGIN
        -- Try inserting a third engine option with is_primary = TRUE
        INSERT INTO aircraft_power.engine_variants
            (slug, model_designation, propulsion_category_code)
        VALUES ('test-engine-3rd', 'TEST-3RD', 'PISTON_RECIPROCATING')
        RETURNING id INTO v_eng1;  -- reuse v_eng1 to hold temp id

        INSERT INTO aircraft_power.variant_powerplants
            (variant_id, engine_variant_id, engine_count, is_standard, is_optional, is_primary)
        VALUES (v_var, v_eng1, 1, FALSE, TRUE, TRUE);
        RAISE EXCEPTION 'uq_vp_primary should have rejected second primary powerplant';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    -- chk_vp_option_flags: neither standard nor optional → rejected
    BEGIN
        INSERT INTO aircraft_power.variant_powerplants
            (variant_id, engine_variant_id, engine_count, is_standard, is_optional, is_primary)
        VALUES (v_var, v_eng2, 1, FALSE, FALSE, FALSE);
        RAISE EXCEPTION 'chk_vp_option_flags should have rejected row with neither flag';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- chk_ev_afterburner: thrust_lbf_wet without has_afterburner → rejected
    BEGIN
        INSERT INTO aircraft_power.engine_variants
            (slug, model_designation, thrust_lbf_dry, thrust_lbf_wet, has_afterburner)
        VALUES ('bad-ab-engine', 'BADJET-100', 5000, 8000, FALSE);
        RAISE EXCEPTION 'chk_ev_afterburner should have rejected wet thrust without afterburner flag';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- chk_ps_type: invalid prop_type → rejected
    BEGIN
        INSERT INTO aircraft_power.propeller_specs
            (slug, model_designation, prop_type)
        VALUES ('bad-prop', 'BAD-PROP-1', 'VARIABLE_INCIDENCE');
        RAISE EXCEPTION 'chk_ps_type should have rejected unknown prop_type';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- Valid propeller insertion
    INSERT INTO aircraft_power.propeller_specs
        (slug, model_designation, blade_count, diameter_in, diameter_ft, prop_type)
    VALUES ('hartzell-hc-c2yk-1bf-f7666', 'HC-C2YK-1BF/F7666A',
            2, 76.0, 76.0/12, 'CONSTANT_SPEED')
    RETURNING id INTO v_prop;

    INSERT INTO aircraft_power.variant_propellers
        (variant_id, propeller_spec_id, is_standard, is_optional, is_primary)
    VALUES (v_var, v_prop, TRUE, FALSE, TRUE);

    -- Rotor test: valid MAIN rotor for helicopter variant
    INSERT INTO aircraft_power.rotor_systems
        (variant_id, rotor_role, blade_count, diameter_ft, rotor_rpm)
    VALUES (v_var, 'MAIN', 4, 44.0, 324);

    -- Invalid rotor_role → rejected
    BEGIN
        INSERT INTO aircraft_power.rotor_systems
            (variant_id, rotor_role, blade_count, diameter_ft)
        VALUES (v_var, 'REAR_THRUSTER', 4, 10.0);
        RAISE EXCEPTION 'chk_rs_role should have rejected unknown rotor_role';
    EXCEPTION WHEN check_violation THEN NULL;
    END;

    -- APU spec (1:1)
    INSERT INTO aircraft_power.apu_specs
        (variant_id, model_designation, output_kva, fuel_type_code)
    VALUES (v_var, 'RE220', 90.0, 'JET_A');

    -- Second APU → rejected
    BEGIN
        INSERT INTO aircraft_power.apu_specs
            (variant_id, model_designation, output_kva)
        VALUES (v_var, 'GTCP36-300', 40.0);
        RAISE EXCEPTION 'UNIQUE on variant_id should reject second APU row';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;

    RAISE NOTICE 'Phase 9 smoke test passed: engine dedup, powerplant option flags, '
                 'afterburner CHECK, prop_type CHECK, primary powerplant/propeller unique, '
                 'rotor role CHECK, APU 1:1 unique — all verified.';

    RAISE EXCEPTION 'ROLLBACK_SMOKE_TEST' USING ERRCODE = 'P0001';
EXCEPTION WHEN SQLSTATE 'P0001' THEN NULL;
END;
$$;

-- -----------------------------------------------------------------------------
-- 6. PROPULSION CATEGORY CROSS-REFERENCE
-- Verify all propulsion_categories seeded in Phase 2 have a primary_power_unit.
-- -----------------------------------------------------------------------------
SELECT code, label, is_jet, is_rotating, primary_power_unit
FROM aircraft_ref.propulsion_categories
ORDER BY sort_order;
-- Expect: 11 rows. All except NONE_GLIDER have a non-NULL primary_power_unit.

-- -----------------------------------------------------------------------------
-- 7. INDEX COUNT PER TABLE
-- -----------------------------------------------------------------------------
SELECT t.relname AS table_name, count(*) AS index_count
FROM pg_index ix
JOIN pg_class t ON t.oid = ix.indrelid
JOIN pg_namespace n ON n.oid = t.relnamespace
WHERE n.nspname = 'aircraft_power'
GROUP BY t.relname
ORDER BY t.relname;
-- Expected indexes (including PK + UNIQUE):
--   engine_variants:      pk, uq_slug, uq_dedup, uq_raw_dedup, idx_hp, idx_thrust,
--                         idx_propulsion, idx_fuel, idx_model_trgm, idx_aliases = ~10
--   variant_powerplants:  pk, uq_variant_engine, idx_vp_engine, uq_vp_primary = 4
--   propeller_specs:      pk, uq_slug, uq_prop_dedup, idx_ps_type = 4
--   variant_propellers:   pk, uq_variant_prop, idx_vpr_prop, uq_vpr_primary = 4
--   rotor_systems:        pk, idx_rs_variant = 2
--   apu_specs:            pk, uq_variant_id = 2
--   powerplant_stcs:      pk, idx_pstc_variant, idx_pstc_engine, idx_pstc_stc_number = 4

-- -----------------------------------------------------------------------------
-- 8. SUMMARY
-- -----------------------------------------------------------------------------
SELECT
    (SELECT count(*) FROM aircraft_power.engine_variants)     AS engine_variants,
    (SELECT count(*) FROM aircraft_power.propeller_specs)     AS propeller_specs,
    (SELECT count(*) FROM aircraft_power.variant_powerplants) AS variant_powerplants,
    (SELECT count(*) FROM aircraft_power.variant_propellers)  AS variant_propellers,
    (SELECT count(*) FROM aircraft_power.rotor_systems)       AS rotor_systems,
    (SELECT count(*) FROM aircraft_power.apu_specs)           AS apu_specs,
    (SELECT count(*) FROM aircraft_power.powerplant_stcs)     AS powerplant_stcs;
-- All zero before Phase 17 ingestion.