-- =============================================================================
-- File: database/staging/902_server_side_json_ingestion.sql
-- Phase 17b: Parsing helper functions + two-pass promotion pipeline.
--
-- USAGE:
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
--        -v seed_json_path=/workspace/database/staging/aircraft_seed.json \
--        -f database/staging/902_server_side_json_ingestion.sql
--
-- Changes from original design (post-evaluation fixes):
--   STEP 2 (family upsert):
--     • Column name fixed: 'name' not 'family_name' (matches 004 DDL).
--
--   STEP 3 (model upsert):
--     • Column name fixed: 'name' not 'model_name' (matches 004 DDL).
--
--   STEP 4 (variant insert):
--     • Column name fixed: 'name' not 'variant_name' (matches 004 DDL).
--     • passenger_capacity: COALESCE guards against NULL + NULL arithmetic
--       when either count extraction returns NULL.
--
--   STEP 8 (engine promotion):
--     • Column name fixed: 'hp_rated' not 'rated_power_hp'.
--     • Thrust: rated_thrust_n stores raw Newtons; thrust_lbf_dry stores
--       canonical LBF (N x 0.224809). Matches the two-column design in 009.
--     • slug added to engine INSERT (NOT NULL UNIQUE in 009 DDL).
--     • tbo_years promoted from engine_json->>'years_before_overhaul'.
--     • source_document_id added to variant_powerplants INSERT (new column in 009).
--
--   STEP 9 (valuation):
--     • for_sale_count: parse_sentinel() called before ::INT cast to safely
--       handle non-numeric strings ('N/A', blank) without a runtime error.
--
--   STEP 10 (ownership costs):
--     • map_cost_key() now returns TABLE (fix_002); call uses SELECT INTO.
--     • Aggregate codes (TOTAL_COST_ANNUAL, TOTAL_FIXED_COST, TOTAL_VARIABLE_COST)
--       routed to aircraft_market.cost_snapshot_totals, not cost_line_items.
--     • pilot_salary / known non-numeric fields (is_numeric=FALSE): value stored
--       in extra_attributes and a curation flag raised. Previously parse_money()
--       returned NULL with no flag, causing silent data loss.
-- =============================================================================

BEGIN;

-- =============================================================================
-- SECTION 1: PARSING HELPER FUNCTIONS
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_ingest.parse_sentinel(raw TEXT)
RETURNS TEXT
LANGUAGE sql IMMUTABLE RETURNS NULL ON NULL INPUT AS $$
    SELECT CASE
        WHEN trim(raw) = ''                         THEN NULL
        WHEN trim(raw) ILIKE 'none%'                THEN NULL
        WHEN trim(raw) IN ('N/A', '-', '--', 'n/a') THEN NULL
        ELSE trim(raw)
    END;
$$;
COMMENT ON FUNCTION aircraft_ingest.parse_sentinel(TEXT) IS
    'Returns NULL for blank, ''None <unit>'', or common placeholder strings. '
    'All other parsers call this first.';

CREATE OR REPLACE FUNCTION aircraft_ingest.parse_numeric(raw TEXT)
RETURNS NUMERIC
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    cleaned TEXT;
    result  NUMERIC;
BEGIN
    cleaned := aircraft_ingest.parse_sentinel(raw);
    IF cleaned IS NULL THEN RETURN NULL; END IF;
    cleaned := regexp_replace(cleaned, '^\$', '');
    cleaned := replace(cleaned, ',', '');
    cleaned := (regexp_match(cleaned, '^-?[0-9]+(\.[0-9]+)?'))[1];
    IF cleaned IS NULL THEN RETURN NULL; END IF;
    BEGIN
        result := cleaned::NUMERIC;
    EXCEPTION WHEN OTHERS THEN
        RETURN NULL;
    END;
    RETURN result;
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.parse_numeric(TEXT) IS
    'Strips currency symbols, thousand-separator commas, and trailing unit '
    'suffixes, then returns the leading numeric value. Returns NULL on failure.';

CREATE OR REPLACE FUNCTION aircraft_ingest.parse_unit(raw TEXT)
RETURNS TEXT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    cleaned TEXT;
    token   TEXT;
BEGIN
    cleaned := aircraft_ingest.parse_sentinel(raw);
    IF cleaned IS NULL THEN RETURN NULL; END IF;
    cleaned := regexp_replace(cleaned, '^\$', '');
    cleaned := replace(cleaned, ',', '');
    token := (regexp_match(cleaned, '[A-Za-z][A-Za-z]*\s*$'))[1];
    IF token IS NULL THEN RETURN NULL; END IF;
    RETURN upper(trim(token));
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.parse_unit(TEXT) IS
    'Returns the uppercase trailing alphabetic unit token from a raw value string.';

CREATE OR REPLACE FUNCTION aircraft_ingest.parse_engine_count(raw TEXT)
RETURNS SMALLINT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    cleaned TEXT;
    match   TEXT[];
BEGIN
    cleaned := aircraft_ingest.parse_sentinel(raw);
    IF cleaned IS NULL THEN RETURN NULL; END IF;
    match := regexp_match(cleaned, '^([0-9]+)\s+[xX]\s+');
    IF match IS NOT NULL THEN RETURN match[1]::SMALLINT; END IF;
    RETURN 1;
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.parse_engine_count(TEXT) IS
    'Extracts count from ''1 x 65 HP'' format. Returns 1 for single-engine strings.';

CREATE OR REPLACE FUNCTION aircraft_ingest.parse_money(raw TEXT)
RETURNS NUMERIC
LANGUAGE sql IMMUTABLE AS $$
    SELECT aircraft_ingest.parse_numeric(raw);
$$;
COMMENT ON FUNCTION aircraft_ingest.parse_money(TEXT) IS
    'Extracts a NUMERIC dollar amount. Thin alias for parse_numeric.';

CREATE OR REPLACE FUNCTION aircraft_ingest.parse_image_dimension(raw TEXT, axis TEXT)
RETURNS SMALLINT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    parts TEXT[];
BEGIN
    IF raw IS NULL OR raw = '' THEN RETURN NULL; END IF;
    parts := string_to_array(lower(raw), 'x');
    IF array_length(parts, 1) <> 2 THEN RETURN NULL; END IF;
    BEGIN
        IF    upper(axis) = 'W' THEN RETURN trim(parts[1])::SMALLINT;
        ELSIF upper(axis) = 'H' THEN RETURN trim(parts[2])::SMALLINT;
        ELSE RETURN NULL;
        END IF;
    EXCEPTION WHEN OTHERS THEN RETURN NULL;
    END;
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.parse_image_dimension(TEXT, TEXT) IS
    'Splits a WxH pixel string and returns the W or H component as SMALLINT.';

CREATE OR REPLACE FUNCTION aircraft_ingest.resolve_href(href TEXT, base_url TEXT)
RETURNS TEXT
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    IF href IS NULL OR trim(href) = '' THEN RETURN NULL; END IF;
    IF href ILIKE 'http%' THEN RETURN href; END IF;
    RETURN rtrim(base_url, '/') || '/' || ltrim(href, '/');
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.resolve_href(TEXT, TEXT) IS
    'Converts a relative href to an absolute URL by prepending base_url.';

CREATE OR REPLACE FUNCTION aircraft_ingest.map_unit_code(raw_unit TEXT)
RETURNS TEXT
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    IF raw_unit IS NULL THEN RETURN NULL; END IF;
    CASE upper(trim(raw_unit))
        WHEN 'KIAS' THEN RETURN 'KIAS';
        WHEN 'KCAS' THEN RETURN 'KNOTS';  -- KCAS not seeded; fold to canonical KNOTS
        WHEN 'KTAS' THEN RETURN 'KTAS';
        WHEN 'NM'   THEN RETURN 'NM';
        WHEN 'FT'   THEN RETURN 'FT';
        WHEN 'FPM'  THEN RETURN 'FPM';
        WHEN 'GPH'  THEN RETURN 'GPH';
        WHEN 'LBS'  THEN RETURN 'LBS';
        WHEN 'LB'   THEN RETURN 'LBS';
        WHEN 'KG'   THEN RETURN 'KG';
        WHEN 'GAL'  THEN RETURN 'US_GAL';
        WHEN 'HP'   THEN RETURN 'HP';
        WHEN 'KW'   THEN RETURN 'KW';
        WHEN 'N'    THEN RETURN 'NEWTONS';
        WHEN 'LBF'  THEN RETURN 'LBF';
        WHEN 'HRS'  THEN RETURN 'HRS';
        WHEN 'HR'   THEN RETURN 'HRS';
        WHEN 'PPH'  THEN RETURN 'PPH';
        ELSE RETURN NULL;
    END CASE;
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.map_unit_code(TEXT) IS
    'Maps raw PlanePHD unit strings to canonical measurement_units codes.';

CREATE OR REPLACE FUNCTION aircraft_ingest.extract_passenger_count(description TEXT)
RETURNS SMALLINT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE match TEXT[];
BEGIN
    IF description IS NULL THEN RETURN NULL; END IF;
    match := regexp_match(description, 'seats up to ([0-9]+) passenger', 'i');
    IF match IS NULL THEN RETURN NULL; END IF;
    RETURN match[1]::SMALLINT;
END;
$$;

CREATE OR REPLACE FUNCTION aircraft_ingest.extract_pilot_count(description TEXT)
RETURNS SMALLINT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE match TEXT[];
BEGIN
    IF description IS NULL THEN RETURN NULL; END IF;
    match := regexp_match(description, 'plus ([0-9]+) pilot', 'i');
    IF match IS NULL THEN RETURN NULL; END IF;
    RETURN match[1]::SMALLINT;
END;
$$;

-- ---------------------------------------------------------------------------
-- map_cost_key: maps a raw PlanePHD ownership_costs key to a seeded
-- aircraft_ref.cost_item_types code via case-insensitive substring matching
-- (PlanePHD keys embed values, e.g. 'fuel_cost_per_hour_3_5_gallons_hr_5_40_gal',
-- so exact lookup is not possible).
-- Returns one row:
--   mapped_code  : cost_item_types.code, or NULL when unrecognised
--   is_numeric   : FALSE for known free-text fields (e.g. pilot_salary) so the
--                  caller routes the value to extra_attributes instead of
--                  parsing it; TRUE otherwise
--   is_aggregate : TRUE for the three source-provided totals, which the caller
--                  routes to cost_snapshot_totals (never cost_line_items)
-- Unrecognised keys return (NULL, NULL, NULL); the caller stores them in
-- extra_attributes and raises a curation flag.
-- NOTE: substring patterns cover the documented PlanePHD key vocabulary; extend
-- as new keys are observed. Conservative by design — unknown keys are preserved
-- in extra_attributes rather than silently dropped or mis-mapped.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION aircraft_ingest.map_cost_key(p_key TEXT)
RETURNS TABLE (mapped_code TEXT, is_numeric BOOLEAN, is_aggregate BOOLEAN)
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    k TEXT := lower(coalesce(p_key, ''));
BEGIN
    -- Source-provided aggregate totals (routed to cost_snapshot_totals)
    IF k LIKE '%total%' AND k LIKE '%fixed%' THEN
        RETURN QUERY SELECT 'TOTAL_FIXED_COST'::TEXT, TRUE, TRUE; RETURN;
    ELSIF k LIKE '%total%' AND k LIKE '%variable%' THEN
        RETURN QUERY SELECT 'TOTAL_VARIABLE_COST'::TEXT, TRUE, TRUE; RETURN;
    ELSIF k LIKE '%total%' AND (k LIKE '%annual%' OR k LIKE '%cost_per_year%' OR k LIKE '%yearly%') THEN
        RETURN QUERY SELECT 'TOTAL_COST_ANNUAL'::TEXT, TRUE, TRUE; RETURN;
    END IF;

    -- Known free-text (non-numeric) field: PlanePHD 'pilot_salary' is a label
    -- such as "Pilot training", not a dollar amount.
    IF k LIKE '%pilot_salary%' OR k LIKE '%pilot salary%' THEN
        RETURN QUERY SELECT 'PILOT_TRAINING'::TEXT, FALSE, FALSE; RETURN;
    END IF;

    -- Component cost lines (numeric). Order: specific before generic.
    RETURN QUERY SELECT
        CASE
            WHEN k LIKE '%inspection%'                                     THEN 'ANNUAL_INSPECTION'
            WHEN k LIKE '%insurance%'                                      THEN 'INSURANCE'
            WHEN k LIKE '%hangar%' OR k LIKE '%tie%down%' OR k LIKE '%tiedown%'
                 OR k LIKE '%storage%'                                     THEN 'HANGAR_STORAGE'
            WHEN k LIKE '%depreciation%'                                   THEN 'DEPRECIATION'
            WHEN k LIKE '%weather%' OR k LIKE '%database%'
                 OR k LIKE '%subscription%' OR k LIKE '%chart%'            THEN 'WEATHER_SERVICE'
            WHEN k LIKE '%training%' OR k LIKE '%currency%'
                 OR k LIKE '%checkride%' OR k LIKE '%recurrent%'           THEN 'PILOT_TRAINING'
            WHEN k LIKE '%refurbish%' OR k LIKE '%paint%'
                 OR k LIKE '%interior%' OR k LIKE '%modern%'               THEN 'REFURBISHING'
            WHEN k LIKE '%registration%' OR k LIKE '%tax%'
                 OR k LIKE '%excise%'                                      THEN 'REGISTRATION_TAXES'
            WHEN k LIKE '%financ%' OR k LIKE '%loan%' OR k LIKE '%interest%' THEN 'FINANCING'
            WHEN k LIKE '%fuel%'                                           THEN 'FUEL'
            WHEN k LIKE '%oil%'                                            THEN 'OIL'
            WHEN k LIKE '%engine%' AND (k LIKE '%reserve%' OR k LIKE '%overhaul%'
                 OR k LIKE '%fund%' OR k LIKE '%tbo%')                     THEN 'ENGINE_RESERVE'
            WHEN (k LIKE '%prop%' OR k LIKE '%propeller%')
                 AND (k LIKE '%reserve%' OR k LIKE '%overhaul%' OR k LIKE '%fund%') THEN 'PROP_RESERVE'
            WHEN k LIKE '%avionics%'                                       THEN 'AVIONICS_RESERVE'
            WHEN k LIKE '%landing%' OR k LIKE '%nav%fee%'
                 OR k LIKE '%navigation%fee%'                              THEN 'LANDING_FEES'
            WHEN k LIKE '%unscheduled%'                                    THEN 'UNSCHEDULED_MAINT'
            WHEN k LIKE '%mainten%' OR k LIKE '%maint%'                    THEN 'HOURLY_MAINTENANCE'
            ELSE NULL
        END::TEXT,
        TRUE,    -- component lines are numeric dollar amounts
        FALSE;   -- not aggregate
END;
$$;
COMMENT ON FUNCTION aircraft_ingest.map_cost_key(TEXT) IS
    'Maps a raw PlanePHD ownership_costs key to a seeded cost_item_types code '
    'by substring matching. Returns (mapped_code, is_numeric, is_aggregate). '
    'Aggregates route to cost_snapshot_totals; non-numeric/unknown keys route '
    'to cost_snapshots.extra_attributes with a curation flag.';

-- =============================================================================
-- SECTION 2: STAGING LOAD FUNCTION
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_ingest.load_seed_json(json_file_path TEXT)
RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_run_id      BIGINT;
    v_run_label   TEXT;
    v_raw_json    JSONB;
    v_mfr_name    TEXT;
    v_acft_name   TEXT;
    v_record      JSONB;
    v_staged_id   BIGINT;
    v_img         JSONB;
    v_img_pos     INT;
    v_total_mfr   INT := 0;
    v_total_acft  INT := 0;
    v_staged_acft INT := 0;
    v_base_url    TEXT := 'https://planephd.com';
BEGIN
    BEGIN
        v_raw_json := pg_read_file(json_file_path)::JSONB;
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'Failed to read seed JSON at "%": %', json_file_path, SQLERRM;
    END;

    -- A content-derived label makes repeated ingestion of identical JSON
    -- deterministic while allowing changed content at the same path to form a
    -- distinct run.
    v_run_label := 'planephd_' || md5(v_raw_json::TEXT);

    INSERT INTO aircraft_ingest.ingest_runs
        (run_label, source_name, source_base_url, json_file_path)
    VALUES
        (v_run_label, 'PlanePHD', v_base_url, json_file_path)
    ON CONFLICT (run_label) DO UPDATE
        SET json_file_path = EXCLUDED.json_file_path, started_at = now()
    RETURNING id INTO v_run_id;

    FOR v_mfr_name, v_record IN
        SELECT key, value FROM jsonb_each(v_raw_json)
    LOOP
        v_total_mfr := v_total_mfr + 1;

        FOR v_acft_name, v_record IN
            SELECT key, value FROM jsonb_each(v_record)
        LOOP
            v_total_acft := v_total_acft + 1;

            INSERT INTO aircraft_ingest.staged_aircraft (
                ingest_run_id, manufacturer_name_raw, aircraft_name_raw,
                source_link, page_url, title, description,
                papi_price_estimate_raw, for_sale_count_raw,
                start_year, end_year, in_production,
                performance_json, weights_json, ownership_costs_json, engine_json
            ) VALUES (
                v_run_id, v_mfr_name, v_acft_name,
                v_record->>'source_link',
                v_record->>'page_url',
                v_record->>'title',
                v_record->>'description',
                CASE WHEN (v_record->'papi_price_estimate') = 'null'::jsonb
                     THEN NULL ELSE v_record->>'papi_price_estimate' END,
                v_record->>'for_sale_count',
                (v_record->>'start_year')::SMALLINT,
                (v_record->>'end_year')::SMALLINT,
                (v_record->>'in_production')::BOOLEAN,
                CASE WHEN jsonb_typeof(v_record->'performance') = 'object'
                     THEN v_record->'performance' ELSE '{}'::JSONB END,
                CASE WHEN jsonb_typeof(v_record->'weights') = 'object'
                     THEN v_record->'weights' ELSE '{}'::JSONB END,
                CASE WHEN jsonb_typeof(v_record->'ownership_costs') = 'object'
                     THEN v_record->'ownership_costs' ELSE '{}'::JSONB END,
                CASE WHEN jsonb_typeof(v_record->'engine') = 'object'
                     THEN v_record->'engine' ELSE '{}'::JSONB END
            )
            ON CONFLICT (ingest_run_id, manufacturer_name_raw, aircraft_name_raw)
                DO NOTHING
            RETURNING id INTO v_staged_id;

            IF v_staged_id IS NULL THEN
                SELECT id INTO v_staged_id
                FROM aircraft_ingest.staged_aircraft
                WHERE ingest_run_id         = v_run_id
                  AND manufacturer_name_raw = v_mfr_name
                  AND aircraft_name_raw     = v_acft_name;
            ELSE
                v_staged_acft := v_staged_acft + 1;
            END IF;

            v_img_pos := 0;
            FOR v_img IN
                SELECT value FROM jsonb_array_elements(
                    CASE WHEN jsonb_typeof(v_record->'images') = 'array'
                         THEN v_record->'images' ELSE '[]'::JSONB END
                )
            LOOP
                INSERT INTO aircraft_ingest.staged_images (
                    staged_aircraft_id, array_position,
                    href_raw, href_resolved, title, holder,
                    dimensions_raw, width_px, height_px, is_primary
                ) VALUES (
                    v_staged_id, v_img_pos,
                    v_img->>'href',
                    aircraft_ingest.resolve_href(v_img->>'href', v_base_url),
                    NULLIF(trim(v_img->>'title'), ''),
                    NULLIF(trim(v_img->>'holder'), ''),
                    v_img->>'dimensions',
                    aircraft_ingest.parse_image_dimension(v_img->>'dimensions', 'W'),
                    aircraft_ingest.parse_image_dimension(v_img->>'dimensions', 'H'),
                    (v_img_pos = 0)
                )
                ON CONFLICT (staged_aircraft_id, array_position) DO NOTHING;
                v_img_pos := v_img_pos + 1;
            END LOOP;

        END LOOP;
    END LOOP;

    UPDATE aircraft_ingest.ingest_runs SET
        total_manufacturers = v_total_mfr,
        total_aircraft      = v_total_acft,
        staged_aircraft     = (SELECT COUNT(*) FROM aircraft_ingest.staged_aircraft WHERE ingest_run_id = v_run_id),
        finished_at         = now()
    WHERE id = v_run_id;

    RAISE NOTICE 'load_seed_json: % mfr, % aircraft, % staged (run_id=%)',
        v_total_mfr, v_total_acft, v_staged_acft, v_run_id;
    RETURN v_staged_acft;
END;
$$;

-- =============================================================================
-- SECTION 3: PROMOTION PIPELINE
-- =============================================================================

CREATE OR REPLACE FUNCTION aircraft_ingest.promote_staged_aircraft()
RETURNS TABLE (
    staged_aircraft_id  BIGINT,
    manufacturer_raw    TEXT,
    aircraft_raw        TEXT,
    result_status       TEXT,
    variant_id          BIGINT,
    notes               TEXT
)
LANGUAGE plpgsql AS $$
DECLARE
    rec                  aircraft_ingest.staged_aircraft%ROWTYPE;
    v_org_id             BIGINT;
    v_family_id          BIGINT;
    v_model_id           BIGINT;
    v_variant_id         BIGINT;
    v_source_id          BIGINT;
    v_doc_id             BIGINT;
    v_eng_id             BIGINT;
    v_snap_id            BIGINT;
    v_cost_key           TEXT;
    v_cost_val           TEXT;
    v_cost_code          TEXT;
    v_cost_is_numeric    BOOLEAN;
    v_cost_is_aggregate  BOOLEAN;
    v_cost_amt           NUMERIC;
    v_flag_notes         TEXT;
    v_has_flag           BOOLEAN;
    v_perf_key           TEXT;
    v_perf_val           TEXT;
    v_num_val            NUMERIC;
    v_unit_raw           TEXT;
    v_unit_code          TEXT;
    v_metric_code        TEXT;
    v_eng_count          SMALLINT;
    v_eng_hp             NUMERIC;
    v_eng_thrust_n       NUMERIC;    -- raw Newtons from source
    v_eng_thrust_lbf     NUMERIC;    -- canonical LBF (N × 0.224809)
    v_eng_tbo_hours      INTEGER;
    v_eng_tbo_years      INTEGER;
    v_eng_slug           TEXT;
    v_passenger_ct       SMALLINT;
    v_pilot_ct           SMALLINT;
    v_papi_val           NUMERIC;

    PERF_MAP CONSTANT JSONB := '{
        "best_cruise_speed":                     "SPEED_CRUISE_BEST",
        "best_range_i":                          "RANGE_NORMAL",
        "ceiling":                               "CEILING_SERVICE",
        "fuel_burn":                             "FUEL_BURN_CRUISE",
        "fuel_burn_75":                          "FUEL_BURN_CRUISE",
        "rate_of_climb":                         "CLIMB_RATE_SL",
        "takeoff_distance":                      "DIST_TO_GROUND_ROLL",
        "takeoff_distance_over_50ft_obstacle":   "DIST_TO_50FT",
        "landing_distance":                      "DIST_LDG_GROUND_ROLL",
        "landing_distance_over_50ft_obstacle":   "DIST_LDG_50FT",
        "stall_speed":                           "SPEED_STALL_CLEAN",
        "horsepower":                            null,
        "thrust":                                null
    }'::JSONB;

    WEIGHT_MAP CONSTANT JSONB := '{
        "empty_weight":  "WEIGHT_EMPTY",
        "gross_weight":  "WEIGHT_MTOW",
        "fuel_capacity": "FUEL_CAPACITY_USABLE"
    }'::JSONB;

    N_TO_LBF CONSTANT NUMERIC := 0.224809;

BEGIN
    -- Upsert PlanePHD source record once per run
    INSERT INTO aircraft_prov.sources (
        name, slug, source_type_code, reliability_grade_code,
        base_url, default_confidence, notes
    ) VALUES (
        'PlanePHD', 'planephd', 'SCRAPED_WEB', 'UNVERIFIED',
        'https://planephd.com', 0.20,
        'Scraped marketplace/encyclopedia. Values require corroboration.'
    )
    ON CONFLICT (slug) DO UPDATE SET base_url = EXCLUDED.base_url
    RETURNING id INTO v_source_id;

    FOR rec IN
        SELECT * FROM aircraft_ingest.staged_aircraft
        WHERE stage_status = 'PENDING'
        ORDER BY manufacturer_name_raw, aircraft_name_raw
    LOOP
        v_has_flag   := FALSE;
        v_flag_notes := '';
        v_variant_id := NULL;
        v_doc_id     := NULL;

        BEGIN

        -- -----------------------------------------------------------------------
        -- STEP 1: Resolve or create manufacturer organization
        -- -----------------------------------------------------------------------
        SELECT id INTO v_org_id
        FROM aircraft_org.organizations
        WHERE name_aliases @> ARRAY[rec.manufacturer_name_raw]
           OR upper(name) = upper(rec.manufacturer_name_raw)
        LIMIT 1;

        IF v_org_id IS NULL THEN
            INSERT INTO aircraft_org.organizations (
                name, slug, org_type_code, name_aliases
            ) VALUES (
                initcap(rec.manufacturer_name_raw),
                aircraft_ref.slugify(rec.manufacturer_name_raw),
                'MANUFACTURER',
                ARRAY[rec.manufacturer_name_raw]
            )
            ON CONFLICT (slug) DO UPDATE
                SET name_aliases = aircraft_org.organizations.name_aliases
                                   || EXCLUDED.name_aliases
            RETURNING id INTO v_org_id;
        END IF;

        -- -----------------------------------------------------------------------
        -- STEP 2: Upsert aircraft family
        -- FIX: column is 'name' (004 DDL), not 'family_name'
        -- -----------------------------------------------------------------------
        INSERT INTO aircraft_core.families (
            manufacturer_org_id, name, slug
        ) VALUES (
            v_org_id,
            initcap(rec.manufacturer_name_raw),
            aircraft_ref.slugify(rec.manufacturer_name_raw || '-family')
        )
        ON CONFLICT (slug) DO UPDATE
            SET manufacturer_org_id = EXCLUDED.manufacturer_org_id
        RETURNING id INTO v_family_id;

        -- -----------------------------------------------------------------------
        -- STEP 3: Upsert aircraft model
        -- FIX: column is 'name' (004 DDL), not 'model_name'
        -- -----------------------------------------------------------------------
        INSERT INTO aircraft_core.models (
            family_id, name, slug
        ) VALUES (
            v_family_id,
            rec.aircraft_name_raw,
            aircraft_ref.slugify(rec.manufacturer_name_raw || '-' || rec.aircraft_name_raw)
        )
        ON CONFLICT (slug) DO UPDATE SET family_id = EXCLUDED.family_id
        RETURNING id INTO v_model_id;

        -- -----------------------------------------------------------------------
        -- STEP 4: Insert aircraft variant
        -- FIX: column is 'name' (004 DDL), not 'variant_name'
        -- FIX: COALESCE prevents NULL arithmetic in passenger count
        -- -----------------------------------------------------------------------
        v_passenger_ct := aircraft_ingest.extract_passenger_count(rec.description);
        v_pilot_ct     := aircraft_ingest.extract_pilot_count(rec.description);
        -- Total occupants; NULL if neither could be extracted
        v_passenger_ct := CASE
            WHEN v_passenger_ct IS NULL AND v_pilot_ct IS NULL THEN NULL
            ELSE COALESCE(v_passenger_ct, 0) + COALESCE(v_pilot_ct, 0)
        END;

        INSERT INTO aircraft_core.variants (
            model_id,
            name,                  -- FIX: was 'variant_name'
            slug,
            description,
            production_start_year,
            production_end_year,
            is_in_production,
            passenger_capacity,
            source_path,
            ingest_key
        ) VALUES (
            v_model_id,
            rec.aircraft_name_raw,
            aircraft_ref.slugify(
                rec.manufacturer_name_raw || '-' || rec.aircraft_name_raw || '-v1'
            ),
            rec.description,
            rec.start_year,
            rec.end_year,
            rec.in_production,
            v_passenger_ct,
            rec.source_link,
            rec.manufacturer_name_raw || '::' || rec.aircraft_name_raw
        )
        ON CONFLICT (ingest_key) WHERE ingest_key IS NOT NULL
            DO UPDATE SET description = EXCLUDED.description
        RETURNING id INTO v_variant_id;

        IF v_variant_id IS NULL THEN
            SELECT id INTO v_variant_id
            FROM aircraft_core.variants
            WHERE ingest_key = rec.manufacturer_name_raw || '::' || rec.aircraft_name_raw;
        END IF;

        -- -----------------------------------------------------------------------
        -- STEP 5: Create source_document
        -- -----------------------------------------------------------------------
        INSERT INTO aircraft_prov.source_documents (
            source_id, variant_id, source_system_key,
            source_url, raw_json, ingest_batch_label
        ) VALUES (
            v_source_id, v_variant_id, rec.source_link,
            rec.page_url, to_jsonb(rec),
            (SELECT run_label FROM aircraft_ingest.ingest_runs WHERE id = rec.ingest_run_id)
        )
        ON CONFLICT (source_id, source_system_key) WHERE source_system_key IS NOT NULL
            DO UPDATE SET variant_id = EXCLUDED.variant_id,
                          source_url = EXCLUDED.source_url,
                          raw_json = EXCLUDED.raw_json,
                          ingest_batch_label = EXCLUDED.ingest_batch_label,
                          retrieved_at = now(),
                          processing_status = 'PENDING'
        RETURNING id INTO v_doc_id;

        -- -----------------------------------------------------------------------
        -- STEP 6: Promote performance metrics
        -- -----------------------------------------------------------------------
        -- Every promoted variant has baseline field-level provenance even when
        -- optional performance, weight, engine, and cost objects are empty.
        INSERT INTO aircraft_prov.source_assertions (
            source_document_id, entity_type_code, entity_id,
            field_name, raw_value, asserted_value,
            status_code, is_accepted, confidence
        ) VALUES (
            v_doc_id, 'AIRCRAFT_VARIANT', v_variant_id,
            'name', rec.aircraft_name_raw, rec.aircraft_name_raw,
            'ACCEPTED', TRUE, 0.20
        ) ON CONFLICT DO NOTHING;

        FOR v_perf_key, v_perf_val IN
            SELECT key, value #>> '{}' FROM jsonb_each(rec.performance_json)
        LOOP
            IF v_perf_key IN ('horsepower', 'thrust') THEN CONTINUE; END IF;

            v_metric_code := PERF_MAP ->> v_perf_key;
            IF v_metric_code IS NULL THEN
                INSERT INTO aircraft_prov.source_assertions (
                    source_document_id, entity_type_code, entity_id,
                    field_name, raw_value, asserted_value, status_code, is_accepted, confidence
                ) VALUES (
                    v_doc_id, 'AIRCRAFT_VARIANT', v_variant_id,
                    'performance.' || v_perf_key, v_perf_val, v_perf_val,
                    'PENDING', FALSE, 0.20
                ) ON CONFLICT DO NOTHING;
                v_has_flag   := TRUE;
                v_flag_notes := v_flag_notes || 'Unmapped perf key: ' || v_perf_key || '; ';
                CONTINUE;
            END IF;

            v_num_val   := aircraft_ingest.parse_numeric(v_perf_val);
            v_unit_raw  := aircraft_ingest.parse_unit(v_perf_val);
            v_unit_code := aircraft_ingest.map_unit_code(v_unit_raw);

            INSERT INTO aircraft_specs.performance_metrics (
                variant_id, metric_type_code,
                raw_value, raw_unit_code, canonical_value,
                is_canonical, confidence
            ) VALUES (
                v_variant_id, v_metric_code,
                v_num_val, v_unit_code,
                CASE WHEN v_unit_code IS NOT NULL
                     THEN aircraft_ref.to_canonical(v_num_val, v_unit_code)
                     ELSE v_num_val END,
                TRUE, 0.20
            ) ON CONFLICT DO NOTHING;

            INSERT INTO aircraft_prov.source_assertions (
                source_document_id, entity_type_code, entity_id,
                field_name, raw_value, asserted_value, status_code, is_accepted, confidence
            ) VALUES (
                v_doc_id, 'AIRCRAFT_VARIANT', v_variant_id,
                'performance.' || v_metric_code, v_perf_val, v_perf_val,
                'ACCEPTED', TRUE, 0.20
            ) ON CONFLICT DO NOTHING;

            IF v_num_val IS NULL AND v_perf_val IS NOT NULL
               AND v_perf_val NOT ILIKE 'none%' THEN
                v_has_flag   := TRUE;
                v_flag_notes := v_flag_notes
                    || 'Parse failure perf ' || v_perf_key || '=' || v_perf_val || '; ';
            END IF;

            IF v_unit_code IS NULL AND v_unit_raw IS NOT NULL THEN
                v_has_flag   := TRUE;
                v_flag_notes := v_flag_notes
                    || 'Unknown unit ''' || v_unit_raw || ''' on ' || v_perf_key || '; ';
            END IF;
        END LOOP;

        -- -----------------------------------------------------------------------
        -- STEP 7: Promote weight metrics
        -- -----------------------------------------------------------------------
        FOR v_perf_key, v_perf_val IN
            SELECT key, value #>> '{}' FROM jsonb_each(rec.weights_json)
        LOOP
            v_metric_code := WEIGHT_MAP ->> v_perf_key;
            IF v_metric_code IS NULL THEN
                v_has_flag   := TRUE;
                v_flag_notes := v_flag_notes || 'Unmapped weight key: ' || v_perf_key || '; ';
                CONTINUE;
            END IF;

            v_num_val   := aircraft_ingest.parse_numeric(v_perf_val);
            v_unit_raw  := aircraft_ingest.parse_unit(v_perf_val);
            v_unit_code := aircraft_ingest.map_unit_code(v_unit_raw);

            INSERT INTO aircraft_specs.weight_metrics (
                variant_id, metric_type_code,
                raw_value, raw_unit_code, canonical_value
            ) VALUES (
                v_variant_id, v_metric_code,
                v_num_val, v_unit_code,
                CASE WHEN v_unit_code IS NOT NULL
                     THEN aircraft_ref.to_canonical(v_num_val, v_unit_code)
                     ELSE v_num_val END
            )
            ON CONFLICT (variant_id, metric_type_code,
                         (COALESCE(configuration, '')))
                DO NOTHING;

            INSERT INTO aircraft_prov.source_assertions (
                source_document_id, entity_type_code, entity_id,
                field_name, raw_value, asserted_value, status_code, is_accepted, confidence
            ) VALUES (
                v_doc_id, 'AIRCRAFT_VARIANT', v_variant_id,
                'weight.' || v_metric_code, v_perf_val, v_perf_val,
                'ACCEPTED', TRUE, 0.20
            ) ON CONFLICT DO NOTHING;
        END LOOP;

        -- -----------------------------------------------------------------------
        -- STEP 8: Promote engine data
        -- FIX: hp_rated (was rated_power_hp)
        -- FIX: rated_thrust_n (raw N) + thrust_lbf_dry (canonical LBF)
        -- FIX: slug supplied (NOT NULL UNIQUE in 009)
        -- FIX: tbo_years from years_before_overhaul
        -- FIX: source_document_id in variant_powerplants
        -- -----------------------------------------------------------------------
        IF rec.engine_json <> '{}'::JSONB THEN

            v_eng_hp         := aircraft_ingest.parse_numeric(rec.engine_json->>'horsepower');
            v_eng_thrust_n   := aircraft_ingest.parse_numeric(rec.engine_json->>'thrust');
            v_eng_thrust_lbf := CASE WHEN v_eng_thrust_n IS NOT NULL
                                     THEN round(v_eng_thrust_n * N_TO_LBF, 2)
                                     ELSE NULL END;

            v_eng_count := COALESCE(
                aircraft_ingest.parse_engine_count(rec.performance_json->>'horsepower'),
                aircraft_ingest.parse_engine_count(rec.performance_json->>'thrust'),
                1
            );

            v_eng_tbo_hours := aircraft_ingest.parse_numeric(
                rec.engine_json->>'overhaul_ht'
            )::INTEGER;

            -- FIX: years_before_overhaul → tbo_years
            v_eng_tbo_years := aircraft_ingest.parse_numeric(
                rec.engine_json->>'years_before_overhaul'
            )::INTEGER;

            -- Slug: slugify(manufacturer-model); must be supplied (NOT NULL UNIQUE)
            v_eng_slug := aircraft_ref.slugify(
                coalesce(rec.engine_json->>'manufacturer', 'unknown') || '-' ||
                coalesce(rec.engine_json->>'model', 'unknown')
            );

            INSERT INTO aircraft_power.engine_variants (
                slug,
                manufacturer_name_raw,
                model_designation,
                hp_rated,            -- FIX: was rated_power_hp
                rated_thrust_n,      -- FIX: raw Newtons (new column)
                thrust_lbf_dry,      -- FIX: canonical LBF
                tbo_hours,
                tbo_years            -- FIX: was missing
            ) VALUES (
                v_eng_slug,
                rec.engine_json->>'manufacturer',
                rec.engine_json->>'model',
                v_eng_hp,
                v_eng_thrust_n,
                v_eng_thrust_lbf,
                v_eng_tbo_hours,
                v_eng_tbo_years
            )
            ON CONFLICT (manufacturer_name_raw, model_designation)
                WHERE manufacturer_org_id IS NULL AND manufacturer_name_raw IS NOT NULL
                DO UPDATE SET
                    hp_rated       = COALESCE(EXCLUDED.hp_rated,
                                              aircraft_power.engine_variants.hp_rated),
                    rated_thrust_n = COALESCE(EXCLUDED.rated_thrust_n,
                                              aircraft_power.engine_variants.rated_thrust_n),
                    thrust_lbf_dry = COALESCE(EXCLUDED.thrust_lbf_dry,
                                              aircraft_power.engine_variants.thrust_lbf_dry),
                    tbo_hours      = COALESCE(EXCLUDED.tbo_hours,
                                              aircraft_power.engine_variants.tbo_hours),
                    tbo_years      = COALESCE(EXCLUDED.tbo_years,
                                              aircraft_power.engine_variants.tbo_years)
            RETURNING id INTO v_eng_id;

            IF v_eng_id IS NULL THEN
                SELECT id INTO v_eng_id
                FROM aircraft_power.engine_variants
                WHERE manufacturer_name_raw = rec.engine_json->>'manufacturer'
                  AND model_designation     = rec.engine_json->>'model'
                  AND manufacturer_org_id IS NULL;
            END IF;

            -- FIX: source_document_id (new column in 009)
            INSERT INTO aircraft_power.variant_powerplants (
                variant_id, engine_variant_id, engine_count,
                is_standard, is_optional, is_primary,
                source_document_id
            ) VALUES (
                v_variant_id, v_eng_id, v_eng_count,
                TRUE, FALSE, TRUE,
                v_doc_id
            )
            ON CONFLICT DO NOTHING;

        END IF;

        -- -----------------------------------------------------------------------
        -- STEP 9: Promote valuation
        -- FIX: for_sale_count — parse_sentinel before cast prevents runtime error
        --      on non-numeric strings such as 'N/A' or blank
        -- -----------------------------------------------------------------------
        IF rec.papi_price_estimate_raw IS NOT NULL THEN
            v_papi_val := aircraft_ingest.parse_money(rec.papi_price_estimate_raw);

            INSERT INTO aircraft_market.valuations (
                variant_id, snapshot_date, source_name,
                papi_price_estimate, for_sale_count,
                currency_code, captured_at
            ) VALUES (
                v_variant_id, CURRENT_DATE, 'PlanePHD',
                v_papi_val,
                -- FIX: parse_sentinel guards against non-numeric for_sale_count
                aircraft_ingest.parse_numeric(
                    aircraft_ingest.parse_sentinel(rec.for_sale_count_raw)
                )::INT,
                'USD', now()
            )
            ON CONFLICT DO NOTHING;
        END IF;

        -- -----------------------------------------------------------------------
        -- STEP 10: Promote ownership costs
        -- FIX: map_cost_key() now returns TABLE — use SELECT INTO
        -- FIX: aggregate totals → cost_snapshot_totals (not cost_line_items)
        -- FIX: is_numeric=FALSE (e.g. pilot_salary "Pilot training") → flag
        -- -----------------------------------------------------------------------
        IF rec.ownership_costs_json <> '{}'::JSONB THEN

            INSERT INTO aircraft_market.cost_snapshots (
                variant_id, snapshot_date, currency_code,
                source_name, assumed_fuel_price_per_gal, extra_attributes
            ) VALUES (
                v_variant_id, CURRENT_DATE, 'USD', 'PlanePHD', NULL, '{}'::JSONB
            )
            ON CONFLICT DO NOTHING
            RETURNING id INTO v_snap_id;

            IF v_snap_id IS NULL THEN
                SELECT id INTO v_snap_id
                FROM aircraft_market.cost_snapshots
                WHERE variant_id    = v_variant_id
                  AND snapshot_date = CURRENT_DATE
                  AND source_name   = 'PlanePHD';
            END IF;

            FOR v_cost_key, v_cost_val IN
                SELECT key, value #>> '{}' FROM jsonb_each(rec.ownership_costs_json)
            LOOP
                -- FIX: map_cost_key returns TABLE; use SELECT INTO
                SELECT mapped_code, is_numeric, is_aggregate
                INTO   v_cost_code, v_cost_is_numeric, v_cost_is_aggregate
                FROM   aircraft_ingest.map_cost_key(v_cost_key)
                LIMIT  1;

                -- FIX: aggregate totals → cost_snapshot_totals
                IF v_cost_code IS NOT NULL AND v_cost_is_aggregate THEN
                    v_cost_amt := aircraft_ingest.parse_money(v_cost_val);

                    INSERT INTO aircraft_market.cost_snapshot_totals (
                        snapshot_id,
                        total_annual_usd,
                        total_fixed_usd,
                        total_variable_usd,
                        source_currency,
                        captured_from_key
                    ) VALUES (
                        v_snap_id,
                        CASE WHEN v_cost_code = 'TOTAL_COST_ANNUAL'   THEN v_cost_amt END,
                        CASE WHEN v_cost_code = 'TOTAL_FIXED_COST'    THEN v_cost_amt END,
                        CASE WHEN v_cost_code = 'TOTAL_VARIABLE_COST' THEN v_cost_amt END,
                        'USD',
                        CASE WHEN v_cost_code = 'TOTAL_VARIABLE_COST' THEN v_cost_key END
                    )
                    ON CONFLICT (snapshot_id) DO UPDATE SET
                        total_annual_usd   = COALESCE(
                            cost_snapshot_totals.total_annual_usd,
                            EXCLUDED.total_annual_usd),
                        total_fixed_usd    = COALESCE(
                            cost_snapshot_totals.total_fixed_usd,
                            EXCLUDED.total_fixed_usd),
                        total_variable_usd = COALESCE(
                            cost_snapshot_totals.total_variable_usd,
                            EXCLUDED.total_variable_usd),
                        captured_from_key  = COALESCE(
                            cost_snapshot_totals.captured_from_key,
                            EXCLUDED.captured_from_key);

                -- FIX: known non-numeric field (e.g. pilot_salary = "Pilot training")
                -- Store in extra_attributes and raise flag; do NOT call parse_money()
                ELSIF v_cost_code IS NOT NULL AND NOT COALESCE(v_cost_is_numeric, TRUE) THEN
                    UPDATE aircraft_market.cost_snapshots
                    SET extra_attributes = extra_attributes
                        || jsonb_build_object(v_cost_key, v_cost_val)
                    WHERE id = v_snap_id;

                    v_has_flag   := TRUE;
                    v_flag_notes := v_flag_notes
                        || 'Non-numeric cost field ''' || v_cost_key
                        || ''' value=''' || coalesce(v_cost_val, 'NULL') || '''; ';

                ELSIF v_cost_code IS NOT NULL THEN
                    -- Normal mapped numeric cost
                    v_cost_amt := aircraft_ingest.parse_money(v_cost_val);

                    IF v_cost_amt IS NOT NULL THEN
                        INSERT INTO aircraft_market.cost_line_items (
                            snapshot_id, cost_item_type_code, amount_annual, currency_code
                        ) VALUES (
                            v_snap_id, v_cost_code, v_cost_amt, 'USD'
                        )
                        ON CONFLICT (snapshot_id, cost_item_type_code) DO NOTHING;
                    ELSE
                        -- Mapped key but unparseable amount
                        UPDATE aircraft_market.cost_snapshots
                        SET extra_attributes = extra_attributes
                            || jsonb_build_object(v_cost_key, v_cost_val)
                        WHERE id = v_snap_id;
                        v_has_flag   := TRUE;
                        v_flag_notes := v_flag_notes
                            || 'Parse failure cost ''' || v_cost_key
                            || ''' value=''' || coalesce(v_cost_val, 'NULL') || '''; ';
                    END IF;

                ELSE
                    -- Truly unmapped key → extra_attributes + flag
                    UPDATE aircraft_market.cost_snapshots
                    SET extra_attributes = extra_attributes
                        || jsonb_build_object(v_cost_key, v_cost_val)
                    WHERE id = v_snap_id;
                    v_has_flag   := TRUE;
                    v_flag_notes := v_flag_notes || 'Unmapped cost key: ' || v_cost_key || '; ';
                END IF;

                -- Source assertion for every cost key regardless of routing
                INSERT INTO aircraft_prov.source_assertions (
                    source_document_id, entity_type_code, entity_id,
                    field_name, raw_value, asserted_value, status_code, is_accepted, confidence
                ) VALUES (
                    v_doc_id, 'COST_SNAPSHOT', v_snap_id,
                    COALESCE(v_cost_code, 'EXTRA:' || v_cost_key),
                    v_cost_val, v_cost_val,
                    CASE WHEN v_cost_code IS NOT NULL AND COALESCE(v_cost_is_numeric, FALSE)
                         THEN 'ACCEPTED' ELSE 'PENDING' END,
                    v_cost_code IS NOT NULL AND COALESCE(v_cost_is_numeric, FALSE),
                    0.20
                ) ON CONFLICT DO NOTHING;

            END LOOP;

        END IF;
        UPDATE aircraft_prov.source_documents
        SET processing_status = 'PROCESSED'
        WHERE id = v_doc_id;


        -- -----------------------------------------------------------------------
        -- STEP 11: Update staging status
        -- -----------------------------------------------------------------------
        UPDATE aircraft_ingest.staged_aircraft SET
            stage_status    = CASE WHEN v_has_flag THEN 'FLAGGED' ELSE 'PROMOTED' END,
            variant_id      = v_variant_id,
            promotion_notes = NULLIF(trim(v_flag_notes), ''),
            promoted_at     = now()
        WHERE id = rec.id;

        IF v_has_flag THEN
            INSERT INTO aircraft_prov.curation_flags (
                entity_type_code, entity_id, issue_type,
                issue_description, status_code, source_assertion_id
            ) VALUES (
                'AIRCRAFT_VARIANT', v_variant_id, 'INGESTION_WARNING',
                left(v_flag_notes, 1000), 'OPEN', NULL
            ) ON CONFLICT DO NOTHING;
        END IF;

        result_status      := CASE WHEN v_has_flag THEN 'FLAGGED' ELSE 'PROMOTED' END;
        staged_aircraft_id := rec.id;
        manufacturer_raw   := rec.manufacturer_name_raw;
        aircraft_raw       := rec.aircraft_name_raw;
        notes              := NULLIF(trim(v_flag_notes), '');
        variant_id         := v_variant_id;
        RETURN NEXT;

        EXCEPTION WHEN OTHERS THEN
            IF v_doc_id IS NOT NULL THEN
                UPDATE aircraft_prov.source_documents
                SET processing_status = 'FAILED',
                    notes = 'Promotion exception: ' || SQLERRM
                WHERE id = v_doc_id;
            END IF;

            UPDATE aircraft_ingest.staged_aircraft SET
                stage_status    = 'FLAGGED',
                promotion_notes = 'EXCEPTION: ' || SQLERRM
            WHERE id = rec.id;
            result_status      := 'ERROR';
            staged_aircraft_id := rec.id;
            manufacturer_raw   := rec.manufacturer_name_raw;
            aircraft_raw       := rec.aircraft_name_raw;
            notes              := 'EXCEPTION: ' || SQLERRM;
            variant_id         := NULL;
            RETURN NEXT;
        END;

    END LOOP;

    UPDATE aircraft_ingest.ingest_runs ir SET
        promoted_aircraft = (
            SELECT COUNT(*) FROM aircraft_ingest.staged_aircraft sa
            WHERE sa.ingest_run_id = ir.id
              AND sa.stage_status IN ('PROMOTED', 'FLAGGED')
        )
    WHERE ir.id IN (
        SELECT DISTINCT ingest_run_id FROM aircraft_ingest.staged_aircraft
        WHERE stage_status IN ('PROMOTED', 'FLAGGED')
    );
END;
$$;

COMMENT ON FUNCTION aircraft_ingest.promote_staged_aircraft() IS
    'Two-pass promotion pipeline. Aggregate cost totals route to '
    'cost_snapshot_totals; component costs route to cost_line_items. '
    'Non-numeric known fields are flagged. '
    'Safe to re-run: skips PROMOTED/SKIPPED rows.';

-- =============================================================================
-- SECTION 4: EXECUTE
-- =============================================================================

SELECT aircraft_ingest.load_seed_json(:'seed_json_path') AS staged_aircraft_count;

SELECT staged_aircraft_id, manufacturer_raw, aircraft_raw, result_status, variant_id, notes
FROM   aircraft_ingest.promote_staged_aircraft()
ORDER  BY manufacturer_raw, aircraft_raw;

SELECT aircraft_read.refresh_search_matviews(FALSE);

COMMIT;