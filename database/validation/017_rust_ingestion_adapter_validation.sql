DO $validation$
DECLARE
    missing TEXT[];
BEGIN
    SELECT array_agg(required.name ORDER BY required.name)
    INTO missing
    FROM (
        VALUES
            ('aircraft_ingest.ingest_runs'),
            ('aircraft_ingest.ingest_run_attempts'),
            ('aircraft_ingest.staged_aircraft'),
            ('aircraft_ingest.staged_images')
    ) required(name)
    WHERE to_regclass(required.name) IS NULL;

    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'Missing Rust ingestion tables: %', missing;
    END IF;

    IF to_regclass('aircraft_ingest.uq_ingest_run_artifact') IS NULL THEN
        RAISE EXCEPTION 'Missing ingestion artifact idempotency index';
    END IF;

    IF to_regclass('aircraft_prov.uq_sd_source_run_key') IS NULL
       OR to_regclass('aircraft_prov.uq_sd_source_key_legacy') IS NULL THEN
        RAISE EXCEPTION 'Missing immutable source-document identity indexes';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'aircraft_prov'
          AND table_name = 'source_documents'
          AND column_name = 'ingest_run_id'
    ) THEN
        RAISE EXCEPTION 'Missing source document ingest-run provenance';
    END IF;

    IF EXISTS (
        SELECT 1 FROM aircraft_ingest.ingest_runs
        WHERE content_sha256 IS NOT NULL
          AND content_sha256 !~ '^[0-9a-f]{64}$'
    ) THEN
        RAISE EXCEPTION 'Invalid ingestion content hash found';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_proc
        WHERE oid = 'aircraft_read.refresh_search_matviews(boolean)'::regprocedure
          AND prosecdef
    ) THEN
        RAISE EXCEPTION 'Read-model refresh function must be SECURITY DEFINER';
    END IF;

    IF has_function_privilege(
        'public',
        'aircraft_read.refresh_search_matviews(boolean)',
        'EXECUTE'
    ) THEN
        RAISE EXCEPTION 'Read-model refresh function must not be executable by PUBLIC';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_run_nonnegative'
          AND conrelid = 'aircraft_ingest.ingest_runs'::regclass
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_attempt_nonnegative'
          AND conrelid = 'aircraft_ingest.ingest_run_attempts'::regclass
    ) THEN
        RAISE EXCEPTION 'Missing ingestion nonnegative count constraints';
    END IF;

    -- The status CHECK constraints are what keep a run or attempt from drifting
    -- into a state the application cannot interpret; their existence was
    -- previously assumed rather than asserted.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_run_status'
          AND conrelid = 'aircraft_ingest.ingest_runs'::regclass
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_ingest_run_hash'
          AND conrelid = 'aircraft_ingest.ingest_runs'::regclass
    ) THEN
        RAISE EXCEPTION 'Missing ingest_runs status or content hash constraint';
    END IF;

    -- Attempt numbering and cascade behaviour: without the UNIQUE, a retry can
    -- duplicate an attempt number; without the cascade, deleting a run orphans
    -- its attempts.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'aircraft_ingest.ingest_run_attempts'::regclass
          AND contype = 'u'
          AND conkey = ARRAY[
                (SELECT attnum FROM pg_attribute
                 WHERE attrelid = 'aircraft_ingest.ingest_run_attempts'::regclass
                   AND attname = 'ingest_run_id'),
                (SELECT attnum FROM pg_attribute
                 WHERE attrelid = 'aircraft_ingest.ingest_run_attempts'::regclass
                   AND attname = 'attempt_number')
            ]::SMALLINT[]
    ) THEN
        RAISE EXCEPTION 'ingest_run_attempts must be unique per (run, attempt number)';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'aircraft_ingest.ingest_run_attempts'::regclass
          AND contype = 'f'
          AND confrelid = 'aircraft_ingest.ingest_runs'::regclass
          AND confdeltype = 'c'
    ) THEN
        RAISE EXCEPTION 'ingest_run_attempts must cascade from ingest_runs';
    END IF;

    -- Staging identity and lifecycle.
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'aircraft_ingest.staged_aircraft'::regclass
          AND contype = 'c'
          AND pg_get_constraintdef(oid) ILIKE '%stage_status%'
    ) THEN
        RAISE EXCEPTION 'staged_aircraft must constrain stage_status';
    END IF;

    IF to_regclass('aircraft_ingest.uq_staged_aircraft_source_key') IS NULL THEN
        RAISE EXCEPTION 'Missing uq_staged_aircraft_source_key index';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'aircraft_ingest.staged_images'::regclass
          AND contype = 'f'
          AND confrelid = 'aircraft_ingest.staged_aircraft'::regclass
    ) THEN
        RAISE EXCEPTION 'staged_images must reference staged_aircraft';
    END IF;

    -- The read model was rebuilt in place by a DROP/CREATE inside migration 017,
    -- which silently loses indexes if the rebuild block changes. Assert the two
    -- that queries depend on, and that the renamed metric codes took effect.
    IF to_regclass('aircraft_read.uq_mvs_variant') IS NULL
       OR to_regclass('aircraft_read.idx_mvs_fts') IS NULL THEN
        RAISE EXCEPTION 'Rebuilt mv_variant_search is missing its identity or search index';
    END IF;

    IF pg_get_viewdef('aircraft_read.mv_variant_search'::regclass, TRUE)
       NOT ILIKE '%SPEED_CRUISE_BEST%' THEN
        RAISE EXCEPTION 'mv_variant_search still uses the pre-017 metric codes';
    END IF;
END
$validation$;
