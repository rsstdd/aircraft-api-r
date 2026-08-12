-- =============================================================================
-- File: database/staging/901_seed_data_staging.sql
-- Phase 17a: Staging schema for PlanePHD JSON seed ingestion.
--
-- Creates aircraft_ingest.ingest_runs, aircraft_ingest.staged_aircraft, and
-- aircraft_ingest.staged_images.  No canonical tables are written here.
-- All three tables survive the session so the promotion pipeline in 902 can
-- run (and re-run) independently of the staging load.
--
-- Re-runnable: all DDL uses CREATE TABLE IF NOT EXISTS.  Reloading the same
-- JSON simply adds rows; the UNIQUE index on staged_aircraft prevents true
-- duplicates within a single run.
-- =============================================================================

BEGIN;

-- ---------------------------------------------------------------------------
-- aircraft_ingest.ingest_runs
-- One row per bulk JSON load.  Mirrors aircraft_prov.sources' batch concept
-- but lives in the transient ingest namespace so Phase 14 provenance tables
-- are not required before staging can begin.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS aircraft_ingest.ingest_runs (
    id                  BIGINT          GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_label           TEXT            NOT NULL,           -- e.g. 'planephd_bulk_2024_01'
    source_name         TEXT            NOT NULL DEFAULT 'PlanePHD',
    source_base_url     TEXT            NOT NULL DEFAULT 'https://planephd.com',
    json_file_path      TEXT,                               -- :seed_json_path psql variable
    started_at          TIMESTAMPTZ     NOT NULL DEFAULT now(),
    finished_at         TIMESTAMPTZ,
    total_manufacturers INT,
    total_aircraft      INT,
    staged_aircraft     INT,
    promoted_aircraft   INT,
    notes               TEXT,
    CONSTRAINT uq_ingest_run_label UNIQUE (run_label)
);
COMMENT ON TABLE  aircraft_ingest.ingest_runs IS
    'One row per bulk seed-data load.  Labels like ''planephd_bulk_2024_01'' tie '
    'staged rows to the originating batch.';
COMMENT ON COLUMN aircraft_ingest.ingest_runs.json_file_path IS
    'Absolute path supplied via :seed_json_path psql variable at load time.';
COMMENT ON COLUMN aircraft_ingest.ingest_runs.staged_aircraft IS
    'Count written by 902 after flattening the JSON; compared against total_aircraft '
    'in Phase 19 validation.';
COMMENT ON COLUMN aircraft_ingest.ingest_runs.promoted_aircraft IS
    'Count updated by the promotion step inside 902.';

-- ---------------------------------------------------------------------------
-- aircraft_ingest.staged_aircraft
-- One row per (manufacturer_name, aircraft_name) pair extracted from the
-- two-level JSON.  The entire source record is preserved in raw_json so the
-- promotion pipeline can re-parse any field without re-reading the file.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS aircraft_ingest.staged_aircraft (
    id                      BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ingest_run_id           BIGINT      NOT NULL
                                        REFERENCES aircraft_ingest.ingest_runs (id)
                                        ON DELETE CASCADE,

    -- Source identity (extracted from the JSON structure itself)
    manufacturer_name_raw   TEXT        NOT NULL,   -- outer key, e.g. 'AERONCA'
    aircraft_name_raw       TEXT        NOT NULL,   -- inner key, e.g. '11AC Chief'

    -- Scalar fields extracted during flattening (preserved as TEXT to avoid
    -- lossy coercion before the parsing functions run)
    source_link             TEXT,       -- relative path, e.g. '/wizard/details/1/...'
    page_url                TEXT,       -- absolute URL
    title                   TEXT,       -- '1946 AERONCA 11AC Chief'
    description             TEXT,
    papi_price_estimate_raw TEXT,       -- '$27,921' or NULL
    for_sale_count_raw      TEXT,       -- '4' (stored as text; nulls are rare)
    start_year              SMALLINT,
    end_year                SMALLINT,
    in_production           BOOLEAN,

    -- Sub-object blobs: preserved verbatim for parsing by promotion functions
    performance_json        JSONB,      -- { "best_cruise_speed": "72 KIAS", ... }
    weights_json            JSONB,      -- { "empty_weight": "786 LBS", ... }  — may be {}
    ownership_costs_json    JSONB,      -- { "annual_inspection_cost": "$1,896.00", ... }
    engine_json             JSONB,      -- { "manufacturer": "...", "model": "...", ... }

    -- Processing state
    stage_status            TEXT        NOT NULL DEFAULT 'PENDING'
                                        CHECK (stage_status IN (
                                            'PENDING',      -- loaded, awaiting promotion
                                            'PROMOTED',     -- canonical rows created
                                            'FLAGGED',      -- raised ≥1 curation flag
                                            'SKIPPED'       -- duplicate / suppressed
                                        )),
    variant_id              BIGINT,     -- populated after successful promotion
    promotion_notes         TEXT,       -- free-text from the promotion function

    staged_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    promoted_at             TIMESTAMPTZ,

    -- Dedup: one staged row per (run, manufacturer, aircraft)
    CONSTRAINT uq_staged_aircraft_per_run
        UNIQUE (ingest_run_id, manufacturer_name_raw, aircraft_name_raw)
);

COMMENT ON TABLE  aircraft_ingest.staged_aircraft IS
    'Flattened staging rows from the two-level PlanePHD JSON.  '
    'Each row corresponds to one inner key (aircraft) under one outer key (manufacturer).';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.manufacturer_name_raw IS
    'Outer JSON key exactly as it appears in the source file (e.g. ''AERO VODOCHODY'').  '
    'Matched to aircraft_org.organizations via name_aliases during promotion.';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.aircraft_name_raw IS
    'Inner JSON key exactly as it appears (e.g. ''11AC Chief'').  '
    'Used as the variant model name after slug normalization.';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.performance_json IS
    'Raw performance sub-object.  Field names vary across records '
    '(fuel_burn_75, fuel_burn, horsepower, thrust, landing_distance, '
    'landing_distance_over_50ft_obstacle, etc.).';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.weights_json IS
    'Raw weights sub-object.  May be an empty object {} for records where '
    'PlanePHD has not collected weight data (e.g. L-39 Albatross).';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.ownership_costs_json IS
    'Raw ownership cost sub-object.  Keys embed computed values '
    '(e.g. ''fuel_cost_per_hour_3_5_gallons_hr_5_40_gal'') and must be '
    'mapped to aircraft_ref.cost_item_types by prefix matching, not exact key lookup.';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.engine_json IS
    'Raw engine sub-object.  Either horsepower (piston) or thrust (jet/turbofan) '
    'is present, never both for the same record.';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.variant_id IS
    'FK into aircraft_core.variants populated after the promotion step succeeds.  '
    'Intentionally not a DDL-enforced FK so staging can be loaded before '
    'aircraft_core is populated.';

-- Indexes for promotion queries and monitoring dashboards
CREATE INDEX IF NOT EXISTS idx_sa_run_status
    ON aircraft_ingest.staged_aircraft (ingest_run_id, stage_status);

CREATE INDEX IF NOT EXISTS idx_sa_manufacturer_raw
    ON aircraft_ingest.staged_aircraft (manufacturer_name_raw);

CREATE INDEX IF NOT EXISTS idx_sa_variant_id
    ON aircraft_ingest.staged_aircraft (variant_id)
    WHERE variant_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- aircraft_ingest.staged_images
-- One row per image in the images[] array of each staged aircraft.
-- Separate from staged_aircraft so image counts can be validated
-- independently and images can be re-processed without re-staging the
-- parent record.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS aircraft_ingest.staged_images (
    id                  BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    staged_aircraft_id  BIGINT      NOT NULL
                                    REFERENCES aircraft_ingest.staged_aircraft (id)
                                    ON DELETE CASCADE,
    array_position      SMALLINT    NOT NULL,   -- 0-based position within images[]
    href_raw            TEXT        NOT NULL,   -- may be relative ('/static/acftref/...')
    href_resolved       TEXT,                   -- absolute URL after resolution
    title               TEXT,                   -- caption / attribution text (may be '')
    holder              TEXT,                   -- image credit / source name
    dimensions_raw      TEXT,                   -- '420x280' — WxH pixel string
    width_px            SMALLINT,               -- parsed from dimensions_raw
    height_px           SMALLINT,               -- parsed from dimensions_raw
    is_primary          BOOLEAN     NOT NULL DEFAULT FALSE,  -- TRUE for array_position = 0
    stage_status        TEXT        NOT NULL DEFAULT 'PENDING'
                                    CHECK (stage_status IN ('PENDING','PROMOTED','SKIPPED')),
    staged_at           TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_staged_image_position
        UNIQUE (staged_aircraft_id, array_position)
);

COMMENT ON TABLE  aircraft_ingest.staged_images IS
    'One row per element of the images[] array on each staged aircraft record.';
COMMENT ON COLUMN aircraft_ingest.staged_images.href_raw IS
    'href exactly as found in JSON.  Relative paths (/static/acftref/...) are '
    'resolved to absolute URLs by prepending the source base URL during staging.';
COMMENT ON COLUMN aircraft_ingest.staged_images.href_resolved IS
    'Absolute URL populated during the flattening step in 902.  '
    'NULL only if href_raw is empty or malformed.';
COMMENT ON COLUMN aircraft_ingest.staged_images.dimensions_raw IS
    'WxH pixel string exactly as found in JSON (e.g. ''420x280'', ''800x516'').  '
    'width_px and height_px are parsed from this during staging.';
COMMENT ON COLUMN aircraft_ingest.staged_images.is_primary IS
    'TRUE for the first image (array_position = 0).  '
    'Mirrors the is_primary + partial-UNIQUE idiom used across the schema.';

CREATE INDEX IF NOT EXISTS idx_si_staged_aircraft
    ON aircraft_ingest.staged_images (staged_aircraft_id);

CREATE INDEX IF NOT EXISTS idx_si_primary
    ON aircraft_ingest.staged_images (staged_aircraft_id)
    WHERE is_primary = TRUE;

COMMIT;