-- =============================================================================
-- File: database/validation/002_core_reference_tables_validation.sql
-- Phase 2 — validation queries for all aircraft_ref lookup tables.
-- Run after all three Phase 2 scripts have been applied:
--   002_core_reference_tables.sql
--   seeds/001_reference_units.sql
--   seeds/002_lookup_seed_data.sql
-- Assertions raise an error immediately when seed invariants are violated.
-- =============================================================================

-- -----------------------------------------------------------------------------
-- 1. EXPECTED TABLE ROW COUNTS
-- Every listed table must contain the exact canonical seed count.
-- -----------------------------------------------------------------------------
DO $validation$
DECLARE
    item RECORD;
    actual_count BIGINT;
BEGIN
    FOR item IN
        SELECT *
        FROM (VALUES
            ('unit_categories', 15),
            ('measurement_units', 38),
            ('aircraft_roles', 57),
            ('service_statuses', 7),
            ('variant_types', 9),
            ('landing_gear_types', 12),
            ('propulsion_categories', 11),
            ('fuel_types', 10),
            ('performance_metric_types', 36),
            ('weight_metric_types', 17),
            ('dimension_metric_types', 17),
            ('certification_authorities', 8),
            ('airworthiness_categories', 9),
            ('pilot_certificate_types', 8),
            ('operating_approval_types', 14),
            ('military_mission_types', 12),
            ('weapon_categories', 9),
            ('hardpoint_position_types', 7),
            ('stores_types', 12),
            ('currencies', 6),
            ('cost_item_types', 21),
            ('aircraft_condition_grades', 5),
            ('ad_types', 5),
            ('sb_compliance_statuses', 6),
            ('availability_grades', 5),
            ('source_types', 8),
            ('source_reliability_grades', 5),
            ('curation_flag_statuses', 5),
            ('curation_entity_types', 11),
            ('assertion_statuses', 5),
            ('mission_profile_types', 15),
            ('comparison_criterion_types', 12),
            ('organization_types', 10),
            ('org_relationship_types', 7),
            ('systems_categories', 15),
            ('equipment_provision_types', 7)
        ) AS expected(table_name, expected_count)
    LOOP
        EXECUTE format('SELECT count(*) FROM aircraft_ref.%I', item.table_name)
            INTO actual_count;
        IF actual_count <> item.expected_count THEN
            RAISE EXCEPTION 'aircraft_ref.% has % rows; expected %',
                item.table_name, actual_count, item.expected_count;
        END IF;
    END LOOP;
END
$validation$;
-- Counts are exact; this block does not rely on PostgreSQL statistics estimates.

-- -----------------------------------------------------------------------------
-- 2. CANONICAL ROW COMPLETENESS AND SEMANTIC NULLS
-- Keep this allowlist synchronized with database/seeds/001_reference_units.sql,
-- database/seeds/002_lookup_seed_data.sql, and the disposable PostgreSQL gate
-- in crates/aircraft_testsupport/tests/seed_data.rs.
-- -----------------------------------------------------------------------------
DO $validation$
DECLARE
    lookup_table TEXT;
    invalid_count BIGINT;
BEGIN
    FOREACH lookup_table IN ARRAY ARRAY[
        'unit_categories', 'measurement_units', 'aircraft_roles',
        'service_statuses', 'variant_types', 'landing_gear_types',
        'propulsion_categories', 'fuel_types', 'performance_metric_types',
        'weight_metric_types', 'dimension_metric_types',
        'certification_authorities', 'airworthiness_categories',
        'pilot_certificate_types', 'operating_approval_types',
        'military_mission_types', 'weapon_categories',
        'hardpoint_position_types', 'stores_types', 'currencies',
        'cost_item_types', 'aircraft_condition_grades', 'ad_types',
        'sb_compliance_statuses', 'availability_grades', 'source_types',
        'source_reliability_grades', 'curation_flag_statuses',
        'curation_entity_types', 'assertion_statuses', 'mission_profile_types',
        'comparison_criterion_types', 'organization_types',
        'org_relationship_types', 'systems_categories',
        'equipment_provision_types'
    ]
    LOOP
        EXECUTE format(
            'SELECT count(*) FROM aircraft_ref.%I '
            'WHERE label IS NULL OR btrim(label) = ''''',
            lookup_table
        ) INTO invalid_count;
        IF invalid_count <> 0 THEN
            RAISE EXCEPTION 'aircraft_ref.% has % blank labels',
                lookup_table, invalid_count;
        END IF;

        IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_schema = 'aircraft_ref'
              AND columns.table_name = lookup_table
              AND column_name = 'is_active'
        ) THEN
            EXECUTE format(
                'SELECT count(*) FROM aircraft_ref.%I WHERE NOT is_active',
                lookup_table
            ) INTO invalid_count;
            IF invalid_count <> 0 THEN
                RAISE EXCEPTION 'aircraft_ref.% has % inactive canonical rows',
                    lookup_table, invalid_count;
            END IF;
        END IF;
    END LOOP;

    FOREACH lookup_table IN ARRAY ARRAY[
        'unit_categories', 'aircraft_roles', 'service_statuses', 'variant_types',
        'landing_gear_types', 'propulsion_categories', 'fuel_types',
        'performance_metric_types', 'weight_metric_types',
        'dimension_metric_types', 'airworthiness_categories',
        'pilot_certificate_types', 'operating_approval_types',
        'military_mission_types', 'weapon_categories',
        'hardpoint_position_types', 'stores_types', 'cost_item_types',
        'aircraft_condition_grades', 'ad_types', 'sb_compliance_statuses',
        'availability_grades', 'source_types', 'source_reliability_grades',
        'curation_flag_statuses', 'curation_entity_types',
        'assertion_statuses', 'mission_profile_types',
        'comparison_criterion_types', 'organization_types',
        'org_relationship_types', 'systems_categories',
        'equipment_provision_types'
    ]
    LOOP
        EXECUTE format(
            'SELECT count(*) FROM aircraft_ref.%I '
            'WHERE description IS NULL OR btrim(description) = ''''',
            lookup_table
        ) INTO invalid_count;
        IF invalid_count <> 0 THEN
            RAISE EXCEPTION 'aircraft_ref.% has % blank descriptions',
                lookup_table, invalid_count;
        END IF;

        -- A table whose every description is its own label plus one shared
        -- suffix was generated, not authored. `label || ' aircraft role.'`
        -- clears the blank check above while carrying nothing the label does
        -- not already carry, and the API publishes it verbatim through
        -- GET /v1/reference/{catalog}. Requiring every row to match AND the
        -- remainders to be identical is what keeps authored prose clear of
        -- this: several real descriptions do open with their label
        -- (RETRACTABLE_TRICYCLE, EASA_CS25), but no table shares one suffix
        -- across all of them. HAVING without GROUP BY yields no row when the
        -- condition is false, leaving invalid_count NULL.
        EXECUTE format(
            'SELECT count(*) FROM aircraft_ref.%I '
            'HAVING count(*) > 2 '
            '   AND bool_and(left(description, char_length(label)) = label) '
            '   AND count(DISTINCT substr(description, char_length(label) + 1)) = 1',
            lookup_table
        ) INTO invalid_count;
        IF invalid_count IS NOT NULL THEN
            RAISE EXCEPTION
                'aircraft_ref.% descriptions are derived from label, not authored',
                lookup_table;
        END IF;
    END LOOP;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.measurement_units
        WHERE symbol IS NULL OR btrim(symbol) = ''
           OR source_string_patterns IS NULL
           OR cardinality(source_string_patterns) = 0
           OR EXISTS (
               SELECT 1 FROM unnest(source_string_patterns) AS pattern
               WHERE btrim(pattern) = ''
           )
    ) THEN
        RAISE EXCEPTION 'measurement units require symbols and non-blank source patterns';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.measurement_units
        WHERE (canonical_unit_code IS NULL) <> (canonical_factor IS NULL)
    ) THEN
        RAISE EXCEPTION 'measurement unit canonical keys and factors must be null together';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.measurement_units
        WHERE (si_factor IS NULL OR si_base_unit_symbol IS NULL)
              <> (code IN ('PPH', 'DEG_F'))
    ) THEN
        RAISE EXCEPTION 'only context-dependent PPH and affine DEG_F may omit SI conversion';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.propulsion_categories
        WHERE (primary_power_unit IS NULL) <> (code = 'NONE_GLIDER')
    ) THEN
        RAISE EXCEPTION 'only unpowered propulsion may omit primary_power_unit';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.fuel_types
        WHERE (density_lbs_per_gal IS NULL) <> (code IN ('ELECTRIC', 'HYDROGEN'))
    ) THEN
        RAISE EXCEPTION 'only non-liquid or state-dependent carriers may omit fuel density';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.weight_metric_types
        WHERE (canonical_unit_code IS NULL)
              <> (code IN ('LOAD_FACTOR_POS', 'LOAD_FACTOR_NEG'))
    ) OR EXISTS (
        SELECT 1 FROM aircraft_ref.dimension_metric_types
        WHERE (canonical_unit_code IS NULL) <> (code = 'DIM_ASPECT_RATIO')
    ) OR EXISTS (
        SELECT 1 FROM aircraft_ref.performance_metric_types
        WHERE canonical_unit_code IS NULL
    ) THEN
        RAISE EXCEPTION 'only dimensionless metrics may omit canonical units';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.performance_metric_types
        WHERE (is_higher_better IS NULL) <> (code IN (
            'SPEED_VX', 'SPEED_VY', 'SPEED_VA', 'SPEED_VYSE',
            'SPEED_VAPP', 'SPEED_ROTATE'))
    ) OR EXISTS (
        SELECT 1 FROM aircraft_ref.comparison_criterion_types
        WHERE (is_higher_better IS NULL) <> (code = 'CRITERION_WINGSPAN')
    ) THEN
        RAISE EXCEPTION 'comparison direction may be null only for documented context-dependent metrics';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.airworthiness_categories
        WHERE (authority_code IS NULL) <> (code = 'MILITARY_SPEC')
    ) THEN
        RAISE EXCEPTION 'only MILITARY_SPEC may omit a civil authority';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM aircraft_ref.comparison_criterion_types
        WHERE ((performance_metric_code IS NULL)::INTEGER
             + (weight_metric_code IS NULL)::INTEGER
             + (dimension_metric_code IS NULL)::INTEGER = 3)
              <> (code IN (
                  'CRITERION_FUEL_EFFICIENCY', 'CRITERION_PAX_SEATS',
                  'CRITERION_PRICE', 'CRITERION_HOURLY_COST'))
    ) THEN
        RAISE EXCEPTION 'only computed comparison criteria may omit all metric foreign keys';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.certification_authorities
        WHERE full_name IS NULL OR btrim(full_name) = ''
           OR website_url IS NULL OR website_url !~ '^https://'
           OR country_codes IS NULL OR cardinality(country_codes) = 0
           OR EXISTS (
               SELECT 1 FROM unnest(country_codes) AS country_code
               WHERE country_code !~ '^[A-Z]{3}$'
           )
    ) THEN
        RAISE EXCEPTION 'certification authorities require names, HTTPS URLs, and ISO alpha-3 jurisdictions';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ref.curation_entity_types
        WHERE schema_name IS NULL OR btrim(schema_name) = ''
           OR table_name IS NULL OR btrim(table_name) = ''
    ) THEN
        RAISE EXCEPTION 'curation entity types require owning schema and table names';
    END IF;
END
$validation$;

-- -----------------------------------------------------------------------------
-- 3. SOURCE-BACKED ANCHORS
-- Unit values are rounded to NUMERIC(18,10) from NIST SP 811. Currency codes,
-- names, and minor units follow ISO 4217; symbols are repository display policy.
-- EASA jurisdictions follow its current member-state register. FAA descriptions
-- reflect MOSAIC and the FAA runway-visual-range category definitions.
-- -----------------------------------------------------------------------------
DO $validation$
DECLARE
    invalid_count BIGINT;
    easa_countries TEXT[];
BEGIN
    SELECT count(*) INTO invalid_count
    FROM (VALUES
        ('KNOTS', 0.5144444444::NUMERIC), ('KIAS', 0.5144444444),
        ('KTAS', 0.5144444444), ('MPH', 0.4470400000),
        ('KMH', 0.2777777778), ('MACH', 1.0000000000),
        ('FT', 0.3048000000), ('METERS', 1.0000000000),
        ('NM', 1852.0000000000), ('KM', 1000.0000000000),
        ('MI', 1609.3440000000), ('LBS', 0.4535923700),
        ('KG', 1.0000000000), ('US_GAL', 3.7854117840),
        ('LITERS', 1.0000000000), ('IMP_GAL', 4.5460900000),
        ('CU_FT', 28.3168465920), ('GPH', 3.7854117840),
        ('LPH', 1.0000000000), ('HP', 745.6998715823),
        ('SHP', 745.6998715823), ('ESHP', 745.6998715823),
        ('KW', 1000.0000000000), ('LBF', 4.4482216153),
        ('NEWTONS', 1.0000000000), ('KN', 1000.0000000000),
        ('FPM', 0.0050800000), ('MPS', 1.0000000000),
        ('HRS', 3600.0000000000), ('MINUTES', 60.0000000000),
        ('SQ_FT', 0.0929030400), ('SQ_M', 1.0000000000),
        ('PSI', 6894.7572931684), ('INHG', 3386.3890000000),
        ('HPA', 100.0000000000), ('DEG_C', 1.0000000000)
    ) AS expected(code, si_factor)
    LEFT JOIN aircraft_ref.measurement_units AS actual USING (code)
    WHERE actual.si_factor IS DISTINCT FROM expected.si_factor;
    IF invalid_count <> 0 THEN
        RAISE EXCEPTION '% measurement-unit SI factors differ from NIST-rounded values',
            invalid_count;
    END IF;

    SELECT count(*) INTO invalid_count
    FROM (VALUES
        ('KIAS', 1.0000000000::NUMERIC), ('KTAS', 1.0000000000),
        ('MPH', 0.8689762419), ('KMH', 0.5399568035),
        ('METERS', 3.2808398950), ('KM', 0.5399568035),
        ('MI', 0.8689762419), ('KG', 2.2046226218),
        ('LITERS', 0.2641720524), ('IMP_GAL', 1.2009499255),
        ('CU_FT', 7.4805194805), ('LPH', 0.2641720524),
        ('SHP', 1.0000000000), ('ESHP', 1.0000000000),
        ('KW', 1.3410220896), ('NEWTONS', 0.2248089431),
        ('KN', 224.8089430997), ('MPS', 196.8503937008),
        ('MINUTES', 0.0166666667), ('SQ_M', 10.7639104167),
        ('INHG', 0.4911541522), ('HPA', 0.0145037738)
    ) AS expected(code, canonical_factor)
    LEFT JOIN aircraft_ref.measurement_units AS actual USING (code)
    WHERE actual.canonical_factor IS DISTINCT FROM expected.canonical_factor;
    IF invalid_count <> 0 THEN
        RAISE EXCEPTION '% canonical conversion factors differ from NIST-rounded values',
            invalid_count;
    END IF;

    IF EXISTS (
        (SELECT code, label, decimal_places FROM aircraft_ref.currencies
         EXCEPT VALUES
            ('USD'::VARCHAR(3), 'US Dollar'::TEXT, 2::SMALLINT),
            ('EUR', 'Euro', 2), ('GBP', 'British Pound', 2),
            ('CAD', 'Canadian Dollar', 2), ('AUD', 'Australian Dollar', 2),
            ('CHF', 'Swiss Franc', 2))
        UNION ALL
        (VALUES
            ('USD'::VARCHAR(3), 'US Dollar'::TEXT, 2::SMALLINT),
            ('EUR', 'Euro', 2), ('GBP', 'British Pound', 2),
            ('CAD', 'Canadian Dollar', 2), ('AUD', 'Australian Dollar', 2),
            ('CHF', 'Swiss Franc', 2)
         EXCEPT SELECT code, label, decimal_places FROM aircraft_ref.currencies)
    ) THEN
        RAISE EXCEPTION 'currency codes, names, or minor units differ from ISO 4217';
    END IF;

    SELECT ARRAY(SELECT unnest(country_codes) ORDER BY 1)
    INTO easa_countries
    FROM aircraft_ref.certification_authorities
    WHERE code = 'EASA';
    IF easa_countries IS DISTINCT FROM ARRAY[
        'AUT','BEL','BGR','CHE','CYP','CZE','DEU','DNK','ESP','EST','FIN','FRA','GRC','HRV',
        'HUN','IRL','ISL','ITA','LIE','LTU','LUX','LVA','MLT','NLD','NOR','POL','PRT','ROU',
        'SVK','SVN','SWE'
    ] THEN
        RAISE EXCEPTION 'EASA jurisdiction list does not match the member-state register';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM aircraft_ref.airworthiness_categories
        WHERE code = 'FAA_LSA'
          AND description ILIKE '%performance-based%'
          AND description LIKE '%July 24, 2026%'
          AND description ILIKE '%legacy%'
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_ref.pilot_certificate_types
        WHERE code = 'FAA_SPORT'
          AND description NOT ILIKE '%LSA in VMC%'
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_ref.operating_approval_types
        WHERE code = 'CAT_III_ILS' AND is_positive
          AND description ILIKE '%below 100 ft%'
          AND description ILIKE '%below 1,200 ft%'
    ) OR NOT EXISTS (
        SELECT 1 FROM aircraft_ref.operating_approval_types
        WHERE code = 'STEEP_APPROACH'
          AND label = 'Steep Approach (above 3°)'
          AND description LIKE '%5.5°%'
    ) THEN
        RAISE EXCEPTION 'FAA-backed light-sport, approach, or sport-pilot anchors are stale';
    END IF;
END
$validation$;

-- -----------------------------------------------------------------------------
-- 4. MEASUREMENT UNITS: canonical consistency
-- Every canonical unit must have both FK columns NULL.
-- Every non-canonical unit must have both set.
-- -----------------------------------------------------------------------------
SELECT code,
       unit_category_code,
       canonical_unit_code,
       canonical_factor,
       CASE
           WHEN canonical_unit_code IS NULL AND canonical_factor IS NULL THEN 'CANONICAL'
           WHEN canonical_unit_code IS NOT NULL AND canonical_factor IS NOT NULL THEN 'NON_CANONICAL'
           ELSE 'INCONSISTENT ← BUG'
           END AS canonical_status
FROM aircraft_ref.measurement_units
ORDER BY unit_category_code, sort_order;
-- Expect: zero rows with canonical_status = 'INCONSISTENT'

-- Identify canonical units by category. RUNWAY_DISTANCE intentionally reuses FT,
-- which is categorized as ALTITUDE because a unit code has one physical category.
SELECT uc.code AS category, mu.code AS canonical_unit
FROM aircraft_ref.unit_categories uc
LEFT JOIN aircraft_ref.measurement_units mu
  ON mu.unit_category_code = uc.code
 AND mu.canonical_unit_code IS NULL
ORDER BY uc.sort_order;
-- -----------------------------------------------------------------------------
-- 5. METRIC TYPE FK INTEGRITY
-- Ensure all canonical_unit_codes on metric type tables point to real units.
-- -----------------------------------------------------------------------------
SELECT 'performance_metric_types' AS tbl,
       pmt.code,
       pmt.canonical_unit_code,
       CASE
           WHEN mu.code IS NOT NULL OR pmt.canonical_unit_code IS NULL
               THEN 'OK'
           ELSE 'BROKEN FK' END   AS status
FROM aircraft_ref.performance_metric_types pmt
         LEFT JOIN aircraft_ref.measurement_units mu ON mu.code = pmt.canonical_unit_code
UNION ALL
SELECT 'weight_metric_types',
       wmt.code,
       wmt.canonical_unit_code,
       CASE
           WHEN mu.code IS NOT NULL OR wmt.canonical_unit_code IS NULL
               THEN 'OK'
           ELSE 'BROKEN FK' END
FROM aircraft_ref.weight_metric_types wmt
         LEFT JOIN aircraft_ref.measurement_units mu ON mu.code = wmt.canonical_unit_code
UNION ALL
SELECT 'dimension_metric_types',
       dmt.code,
       dmt.canonical_unit_code,
       CASE
           WHEN mu.code IS NOT NULL OR dmt.canonical_unit_code IS NULL
               THEN 'OK'
           ELSE 'BROKEN FK' END
FROM aircraft_ref.dimension_metric_types dmt
         LEFT JOIN aircraft_ref.measurement_units mu ON mu.code = dmt.canonical_unit_code
ORDER BY 1, 2;
-- Expect: zero rows with status = 'BROKEN FK'

-- -----------------------------------------------------------------------------
-- 6. PROPULSION CATEGORIES: primary_power_unit FK check
-- -----------------------------------------------------------------------------
SELECT pc.code,
       pc.primary_power_unit,
       CASE
           WHEN mu.code IS NOT NULL OR pc.primary_power_unit IS NULL
               THEN 'OK'
           ELSE 'BROKEN FK' END AS status
FROM aircraft_ref.propulsion_categories pc
         LEFT JOIN aircraft_ref.measurement_units mu ON mu.code = pc.primary_power_unit
ORDER BY pc.sort_order;
-- Expect: zero rows with status = 'BROKEN FK'

-- -----------------------------------------------------------------------------
-- 7. COMPARISON CRITERION TYPES: mutual-exclusivity constraint
-- chk_criterion_single_domain enforces this; verify no violations exist.
-- -----------------------------------------------------------------------------
SELECT code,
       performance_metric_code,
       weight_metric_code,
       dimension_metric_code,
       (CASE WHEN performance_metric_code IS NOT NULL THEN 1 ELSE 0 END
           + CASE WHEN weight_metric_code IS NOT NULL THEN 1 ELSE 0 END
           + CASE WHEN dimension_metric_code IS NOT NULL THEN 1 ELSE 0 END) AS metric_link_count
FROM aircraft_ref.comparison_criterion_types
ORDER BY sort_order;
-- Expect: metric_link_count is 0 or 1 for every row.

-- -----------------------------------------------------------------------------
-- 8. CURATION FLAG STATUSES: terminal-state coverage
-- -----------------------------------------------------------------------------
SELECT code, label, is_terminal
FROM aircraft_ref.curation_flag_statuses
ORDER BY sort_order;
-- Expect: RESOLVED and DISMISSED are is_terminal = TRUE; others FALSE.

-- -----------------------------------------------------------------------------
-- 9. SPOT-CHECKS: key seed rows present
-- -----------------------------------------------------------------------------
SELECT (SELECT count(*)
        FROM aircraft_ref.aircraft_roles
        WHERE role_group = 'MILITARY_FIXED_WING') AS military_fw_roles,
       (SELECT count(*)
        FROM aircraft_ref.aircraft_roles
        WHERE role_group = 'CIVILIAN_GA')         AS civilian_ga_roles,
       (SELECT count(*)
        FROM aircraft_ref.cost_item_types
        WHERE is_fixed = TRUE)                    AS fixed_cost_items,
       (SELECT count(*)
        FROM aircraft_ref.cost_item_types
        WHERE code = 'FUEL')                    AS fuel_cost_items,
       (SELECT count(*)
        FROM aircraft_ref.source_reliability_grades
        WHERE numeric_score = 5)                  AS authoritative_grades,
       (SELECT code
        FROM aircraft_ref.measurement_units
        WHERE unit_category_code = 'SPEED'
          AND canonical_unit_code IS NULL)        AS speed_canonical_unit,
       (SELECT code
        FROM aircraft_ref.measurement_units
        WHERE unit_category_code = 'WEIGHT'
          AND canonical_unit_code IS NULL)        AS weight_canonical_unit,
       (SELECT code
        FROM aircraft_ref.measurement_units
        WHERE unit_category_code = 'THRUST'
          AND canonical_unit_code IS NULL)        AS thrust_canonical_unit;
-- Expect: 20 military FW roles, 7 civilian GA roles, 9 fixed costs,
--         1 fuel cost, 1 authoritative grade, KNOTS, LBS, LBF.
-- -----------------------------------------------------------------------------
-- 10. UNIT CONVERSION SMOKE TEST
-- Verify key canonical_factors are plausible (not NULL or negative).
-- -----------------------------------------------------------------------------
SELECT code, unit_category_code, canonical_unit_code, canonical_factor
FROM aircraft_ref.measurement_units
WHERE canonical_unit_code IS NOT NULL
  AND (canonical_factor IS NULL OR canonical_factor <= 0)
ORDER BY code;
-- Expect: zero rows (every non-canonical unit has a positive conversion factor).

-- -----------------------------------------------------------------------------
-- 11. DOMAIN CHECK: all lookup_code columns conform to the domain pattern
-- (uppercase letter start, alphanumeric + underscore only).
-- -----------------------------------------------------------------------------
SELECT 'aircraft_roles' AS tbl, code
FROM aircraft_ref.aircraft_roles
WHERE code !~ '^[A-Z][A-Z0-9_]*$'
UNION ALL
SELECT 'service_statuses', code
FROM aircraft_ref.service_statuses
WHERE code !~ '^[A-Z][A-Z0-9_]*$'
UNION ALL
SELECT 'measurement_units', code
FROM aircraft_ref.measurement_units
WHERE code !~ '^[A-Z][A-Z0-9_]*$'
UNION ALL
SELECT 'performance_metric_types', code
FROM aircraft_ref.performance_metric_types
WHERE code !~ '^[A-Z][A-Z0-9_]*$'
UNION ALL
SELECT 'cost_item_types', code
FROM aircraft_ref.cost_item_types
WHERE code !~ '^[A-Z][A-Z0-9_]*$';
-- Expect: zero rows (all codes match the lookup_code domain pattern).

-- -----------------------------------------------------------------------------
-- 12. SUMMARY COUNTS by group
-- -----------------------------------------------------------------------------
SELECT count(*) FILTER (WHERE table_name IN (
    'unit_categories','measurement_units'))                     AS unit_tables, count(*) FILTER (WHERE table_name IN (
    'aircraft_roles','service_statuses','variant_types'))       AS taxonomy_tables, count(*) FILTER (WHERE table_name IN (
    'landing_gear_types','propulsion_categories','fuel_types'))  AS physical_tables, count(*) FILTER (WHERE table_name IN (
    'performance_metric_types','weight_metric_types',
    'dimension_metric_types'))                                  AS metric_type_tables, count(*) AS total_lookup_tables
FROM information_schema.tables
WHERE table_schema = 'aircraft_ref'
  AND table_type = 'BASE TABLE';
-- Expect: 2, 3, 3, 3, 36 (total 36 lookup tables in aircraft_ref).
