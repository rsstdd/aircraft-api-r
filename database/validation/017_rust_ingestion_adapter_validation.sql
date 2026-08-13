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
END
$validation$;
