-- Companion to database/migrations/031_ownership_cost_annual_contribution.sql.
--
-- Read-only in effect: the fixture below is built and rolled back inside an
-- exception block, so db-prod-validate leaves production exactly as it found it.
--
-- Checking the definition would not be enough here. The broken and fixed
-- expressions are both valid SQL over the same columns and differ only in where
-- the COALESCE sits, so a reader cannot tell them apart by shape -- and no
-- imported row reaches the difference, because ingestion writes amount_annual
-- only for is_fixed codes. The case has to be constructed: one variable item
-- quoted annually beside one quoted hourly, which is exactly what a curator
-- entering a mixed snapshot produces and exactly what the old expression got
-- wrong by dropping the hourly one.

DO $validation$
DECLARE
    v_fam BIGINT;
    v_mod BIGINT;
    v_var BIGINT;
    v_snap BIGINT;
    computed NUMERIC;
BEGIN
    BEGIN
        INSERT INTO aircraft_core.families(slug, name)
        VALUES ('cost31-probe-fam', 'Cost31 Probe Family') RETURNING id INTO v_fam;
        INSERT INTO aircraft_core.models(family_id, slug, name)
        VALUES (v_fam, 'cost31-probe-mod', 'Cost31 Probe Model') RETURNING id INTO v_mod;
        INSERT INTO aircraft_core.variants(model_id, slug, name)
        VALUES (v_mod, 'cost31-probe-var', 'Cost31 Probe Variant') RETURNING id INTO v_var;

        -- is_canonical is not decoration here: migration 020's curation gate
        -- put `WHERE cost_snapshots.is_canonical` in the view, so a
        -- non-canonical snapshot produces no row at all and the check below
        -- would read NULL and pass for the wrong reason.
        INSERT INTO aircraft_market.cost_snapshots(
            variant_id, snapshot_date, assumed_annual_hours, currency_code, is_canonical)
        VALUES (v_var, CURRENT_DATE, 100, 'USD', TRUE) RETURNING id INTO v_snap;

        -- One variable item quoted annually, one quoted per hour. The annual one
        -- is what used to suppress the other.
        INSERT INTO aircraft_market.cost_line_items(
            snapshot_id, cost_item_type_code, amount_annual, amount_per_hour, currency_code)
        VALUES (v_snap, 'MISC_VARIABLE', 1000.00, NULL, 'USD'),
               (v_snap, 'FUEL', NULL, 10.0000, 'USD');

        REFRESH MATERIALIZED VIEW aircraft_read.mv_ownership_cost_summary;

        SELECT computed_total_annual_usd
          INTO computed
          FROM aircraft_read.mv_ownership_cost_summary
         WHERE variant_id = v_var;

        -- 1000 annual + 10/hr over 100 hours. The old expression returned 1000,
        -- having taken the annual arm and never evaluated the hourly one.
        IF computed IS NULL THEN
            RAISE EXCEPTION
                'the probe snapshot produced no row in mv_ownership_cost_summary; the view no '
                'longer reads the fixture this check builds and it is proving nothing';
        END IF;

        IF computed <> 2000.00 THEN
            RAISE EXCEPTION
                'computed_total_annual_usd is % for a snapshot mixing annual and hourly '
                'variable items; expected 2000.00. One variable item quoted annually is '
                'suppressing the hourly items on the same snapshot.', computed;
        END IF;

        RAISE EXCEPTION 'ROLLBACK_COST31_PROBE' USING ERRCODE = 'P0001';
    EXCEPTION
        WHEN SQLSTATE 'P0001' THEN
            IF SQLERRM <> 'ROLLBACK_COST31_PROBE' THEN
                RAISE;
            END IF;
    END;
END
$validation$;

-- The probe rolled back, so the view holds whatever the fixture displaced.
-- Put the real data back rather than leaving a truncated read model behind.
SELECT aircraft_read.refresh_search_matviews(FALSE);
