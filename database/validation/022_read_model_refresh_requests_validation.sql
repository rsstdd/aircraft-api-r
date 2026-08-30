DO $validation$
BEGIN
    IF to_regclass('aircraft_read.read_model_refresh_requests') IS NULL THEN
        RAISE EXCEPTION 'aircraft_read.read_model_refresh_requests must exist';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_rmrr_status'
          AND conrelid = 'aircraft_read.read_model_refresh_requests'::regclass
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_rmrr_completed_at'
          AND conrelid = 'aircraft_read.read_model_refresh_requests'::regclass
    ) THEN
        RAISE EXCEPTION
            'read_model_refresh_requests must constrain status_code and completion';
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_indexes
        WHERE schemaname = 'aircraft_read'
          AND tablename = 'read_model_refresh_requests'
          AND indexname = 'idx_rmrr_pending'
    ) THEN
        RAISE EXCEPTION 'idx_rmrr_pending must index the outstanding requests';
    END IF;
END
$validation$;

-- A request may not claim completion without a completion timestamp, or the
-- retry path would treat an unsatisfied request as settled.
DO $validation$
BEGIN
    BEGIN
        INSERT INTO aircraft_read.read_model_refresh_requests(
            requested_by, status_code, completed_at)
        VALUES ('VALIDATION', 'COMPLETED', NULL);
        RAISE EXCEPTION
            'completion without a timestamp must violate chk_rmrr_completed_at';
    EXCEPTION
        WHEN check_violation THEN
            NULL;
    END;
END
$validation$;
