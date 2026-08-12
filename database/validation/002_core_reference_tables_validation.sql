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
            ('landing_gear_types', 10),
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
-- 2. MEASUREMENT UNITS: canonical consistency
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
-- 3. METRIC TYPE FK INTEGRITY
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
-- 4. PROPULSION CATEGORIES: primary_power_unit FK check
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
-- 5. COMPARISON CRITERION TYPES: mutual-exclusivity constraint
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
-- 6. CURATION FLAG STATUSES: terminal-state coverage
-- -----------------------------------------------------------------------------
SELECT code, label, is_terminal
FROM aircraft_ref.curation_flag_statuses
ORDER BY sort_order;
-- Expect: RESOLVED and DISMISSED are is_terminal = TRUE; others FALSE.

-- -----------------------------------------------------------------------------
-- 7. SPOT-CHECKS: key seed rows present
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
-- Expect: 26 military FW roles, 7 civilian GA roles, 9 fixed costs,
--         1 fuel cost, 1 authoritative grade, KNOTS, LBS, LBF.
-- -----------------------------------------------------------------------------
-- 8. UNIT CONVERSION SMOKE TEST
-- Verify key canonical_factors are plausible (not NULL or negative).
-- -----------------------------------------------------------------------------
SELECT code, unit_category_code, canonical_unit_code, canonical_factor
FROM aircraft_ref.measurement_units
WHERE canonical_unit_code IS NOT NULL
  AND (canonical_factor IS NULL OR canonical_factor <= 0)
ORDER BY code;
-- Expect: zero rows (every non-canonical unit has a positive conversion factor).

-- -----------------------------------------------------------------------------
-- 9. DOMAIN CHECK: all lookup_code columns conform to the domain pattern
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
-- 10. SUMMARY COUNTS by group
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