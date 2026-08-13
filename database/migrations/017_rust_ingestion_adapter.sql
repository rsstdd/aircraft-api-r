-- Rust ingestion adapter infrastructure.
-- This migration is compatible with databases that previously ran the
-- non-canonical 901 staging script.
BEGIN;

CREATE TABLE IF NOT EXISTS aircraft_ingest.ingest_runs (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_label TEXT NOT NULL UNIQUE,
    source_name TEXT NOT NULL,
    source_base_url TEXT,
    json_file_path TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    total_manufacturers INT,
    total_aircraft INT,
    staged_aircraft INT,
    promoted_aircraft INT,
    notes TEXT
);

ALTER TABLE aircraft_ingest.ingest_runs
    ADD COLUMN IF NOT EXISTS source_slug TEXT,
    ADD COLUMN IF NOT EXISTS content_sha256 TEXT,
    ADD COLUMN IF NOT EXISTS parser_name TEXT,
    ADD COLUMN IF NOT EXISTS parser_version TEXT,
    ADD COLUMN IF NOT EXISTS input_byte_length BIGINT,
    ADD COLUMN IF NOT EXISTS input_locator TEXT,
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'RECEIVED',
    ADD COLUMN IF NOT EXISTS flagged_aircraft INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS skipped_aircraft INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS warning_count INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS failure_code TEXT,
    ADD COLUMN IF NOT EXISTS failure_message TEXT;

DO $migration$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_run_status'
          AND conrelid = 'aircraft_ingest.ingest_runs'::regclass
    ) THEN
        ALTER TABLE aircraft_ingest.ingest_runs
            ADD CONSTRAINT chk_ingest_run_status
            CHECK (status IN (
                'RECEIVED', 'IMPORTING', 'SUCCEEDED', 'VALIDATION_FAILED', 'FAILED'
            ));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_run_hash'
          AND conrelid = 'aircraft_ingest.ingest_runs'::regclass
    ) THEN
        ALTER TABLE aircraft_ingest.ingest_runs
            ADD CONSTRAINT chk_ingest_run_hash
            CHECK (content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$');
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_run_nonnegative'
          AND conrelid = 'aircraft_ingest.ingest_runs'::regclass
    ) THEN
        ALTER TABLE aircraft_ingest.ingest_runs
            ADD CONSTRAINT chk_ingest_run_nonnegative CHECK (
                COALESCE(input_byte_length, 0) >= 0
                AND COALESCE(total_aircraft, 0) >= 0
                AND COALESCE(staged_aircraft, 0) >= 0
                AND COALESCE(promoted_aircraft, 0) >= 0
                AND flagged_aircraft >= 0
                AND skipped_aircraft >= 0
                AND warning_count >= 0
            );
    END IF;
END
$migration$;

CREATE UNIQUE INDEX IF NOT EXISTS uq_ingest_run_artifact
    ON aircraft_ingest.ingest_runs (
        source_slug, content_sha256, parser_name, parser_version
    )
    WHERE content_sha256 IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_ingest_runs_status_started
    ON aircraft_ingest.ingest_runs (status, started_at DESC);

CREATE TABLE IF NOT EXISTS aircraft_ingest.ingest_run_attempts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ingest_run_id BIGINT NOT NULL
        REFERENCES aircraft_ingest.ingest_runs(id) ON DELETE CASCADE,
    attempt_number INT NOT NULL,
    status TEXT NOT NULL DEFAULT 'IMPORTING'
        CHECK (status IN ('IMPORTING', 'SUCCEEDED', 'VALIDATION_FAILED', 'FAILED')),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    failure_code TEXT,
    failure_message TEXT,
    staged_aircraft INT NOT NULL DEFAULT 0,
    promoted_aircraft INT NOT NULL DEFAULT 0,
    flagged_aircraft INT NOT NULL DEFAULT 0,
    skipped_aircraft INT NOT NULL DEFAULT 0,
    warning_count INT NOT NULL DEFAULT 0,
    UNIQUE (ingest_run_id, attempt_number)
);

ALTER TABLE aircraft_ingest.ingest_run_attempts
    DROP CONSTRAINT IF EXISTS chk_ingest_attempt_nonnegative;
ALTER TABLE aircraft_ingest.ingest_run_attempts
    ADD CONSTRAINT chk_ingest_attempt_nonnegative CHECK (
        attempt_number > 0
        AND staged_aircraft >= 0
        AND promoted_aircraft >= 0
        AND flagged_aircraft >= 0
        AND skipped_aircraft >= 0
        AND warning_count >= 0
    );

CREATE INDEX IF NOT EXISTS idx_ingest_attempts_run_started
    ON aircraft_ingest.ingest_run_attempts (ingest_run_id, started_at DESC);

-- Rust runs preserve each retrieved record as immutable evidence. Legacy
-- source documents remain deduplicated by source key when ingest_run_id is NULL.
ALTER TABLE aircraft_prov.source_documents
    ADD COLUMN IF NOT EXISTS ingest_run_id BIGINT
        REFERENCES aircraft_ingest.ingest_runs(id) ON DELETE RESTRICT;

DROP INDEX IF EXISTS aircraft_prov.uq_sd_source_key;
CREATE UNIQUE INDEX IF NOT EXISTS uq_sd_source_key_legacy
    ON aircraft_prov.source_documents (source_id, source_system_key)
    WHERE source_system_key IS NOT NULL AND ingest_run_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_sd_source_run_key
    ON aircraft_prov.source_documents (source_id, source_system_key, ingest_run_id)
    WHERE source_system_key IS NOT NULL AND ingest_run_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_sdo_ingest_run
    ON aircraft_prov.source_documents (ingest_run_id)
    WHERE ingest_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS aircraft_ingest.staged_aircraft (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ingest_run_id BIGINT NOT NULL
        REFERENCES aircraft_ingest.ingest_runs(id) ON DELETE CASCADE,
    manufacturer_name_raw TEXT NOT NULL,
    aircraft_name_raw TEXT NOT NULL,
    source_link TEXT,
    page_url TEXT,
    title TEXT,
    description TEXT,
    papi_price_estimate_raw TEXT,
    for_sale_count_raw TEXT,
    start_year SMALLINT,
    end_year SMALLINT,
    in_production BOOLEAN,
    performance_json JSONB,
    weights_json JSONB,
    ownership_costs_json JSONB,
    engine_json JSONB,
    stage_status TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (stage_status IN ('PENDING', 'PROMOTED', 'FLAGGED', 'SKIPPED')),
    variant_id BIGINT,
    promotion_notes TEXT,
    staged_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    promoted_at TIMESTAMPTZ,
    UNIQUE (ingest_run_id, manufacturer_name_raw, aircraft_name_raw)
);

ALTER TABLE aircraft_ingest.staged_aircraft
    ADD COLUMN IF NOT EXISTS source_record_key TEXT,
    ADD COLUMN IF NOT EXISTS raw_json JSONB,
    ADD COLUMN IF NOT EXISTS issues JSONB NOT NULL DEFAULT '[]'::jsonb;

CREATE UNIQUE INDEX IF NOT EXISTS uq_staged_aircraft_source_key
    ON aircraft_ingest.staged_aircraft (ingest_run_id, source_record_key)
    WHERE source_record_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_staged_aircraft_run_status
    ON aircraft_ingest.staged_aircraft (ingest_run_id, stage_status);

CREATE TABLE IF NOT EXISTS aircraft_ingest.staged_images (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    staged_aircraft_id BIGINT NOT NULL
        REFERENCES aircraft_ingest.staged_aircraft(id) ON DELETE CASCADE,
    array_position SMALLINT NOT NULL,
    href_raw TEXT NOT NULL,
    href_resolved TEXT,
    title TEXT,
    holder TEXT,
    dimensions_raw TEXT,
    width_px SMALLINT,
    height_px SMALLINT,
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    stage_status TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (stage_status IN ('PENDING', 'PROMOTED', 'SKIPPED')),
    staged_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (staged_aircraft_id, array_position)
);

ALTER FUNCTION aircraft_read.refresh_search_matviews(BOOLEAN) SECURITY DEFINER;
ALTER FUNCTION aircraft_read.refresh_search_matviews(BOOLEAN) SET search_path = pg_catalog;
REVOKE ALL ON FUNCTION aircraft_read.refresh_search_matviews(BOOLEAN) FROM PUBLIC;

COMMENT ON TABLE aircraft_ingest.ingest_runs IS
    'Logical source artifact imports, keyed by source, content hash, and parser version.';
COMMENT ON TABLE aircraft_ingest.ingest_run_attempts IS
    'Durable audit rows for every attempt at a logical ingestion run.';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.raw_json IS
    'Complete parsed source record retained for parser evolution and curation audits.';
COMMENT ON COLUMN aircraft_ingest.staged_aircraft.issues IS
    'Stable structured warnings produced by the Rust source adapter.';

COMMENT ON COLUMN aircraft_prov.source_documents.ingest_run_id IS
    'Rust logical run that captured this immutable raw document. NULL identifies legacy ingestion.';
COMMIT;
