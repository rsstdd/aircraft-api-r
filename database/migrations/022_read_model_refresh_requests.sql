-- =============================================================================
-- File: database/migrations/022_read_model_refresh_requests.sql
-- Phase 22: Make a missed read-model refresh recoverable.
--
-- A curation decision commits, and only then are the materialized views
-- rebuilt, so a non-concurrent REFRESH never holds its ACCESS EXCLUSIVE locks
-- inside the curation transaction. That ordering left a gap: when the refresh
-- failed -- lock timeout, statement timeout, a dropped connection -- the
-- decision was already durable while the read model still served the
-- pre-decision picture, and nothing recorded that it was stale. Repeating the
-- decision is rejected as ALREADY_DECIDED, so the curation path had no way at
-- all to recover the read model.
--
-- This table is that record. A request is enqueued inside the same transaction
-- as the decision, so it commits or rolls back with it, and is closed only
-- after a refresh has actually succeeded. Completed rows are retained as an
-- audit trail of when the read model was last known to be current.
--
-- One refresh satisfies every request it saw, because the matviews are rebuilt
-- whole rather than incrementally: the retry closes the requests that were
-- already pending when it started, and leaves any that arrived during the
-- rebuild for the next one.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

CREATE TABLE IF NOT EXISTS aircraft_read.read_model_refresh_requests (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    requested_by TEXT NOT NULL,
    reason TEXT,
    status_code TEXT NOT NULL DEFAULT 'PENDING',
    attempts BIGINT NOT NULL DEFAULT 0,
    last_error TEXT,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT chk_rmrr_status CHECK (status_code IN ('PENDING', 'COMPLETED')),
    CONSTRAINT chk_rmrr_attempts CHECK (attempts >= 0),
    -- Completion is one fact recorded in two columns; neither may drift from
    -- the other, or a retry would skip a request that was never satisfied.
    CONSTRAINT chk_rmrr_completed_at
        CHECK ((status_code = 'COMPLETED') = (completed_at IS NOT NULL))
);

COMMENT ON TABLE aircraft_read.read_model_refresh_requests IS
    'Durable record that aircraft_read materialized views are stale. Written '
    'in the transaction that changed what they serve, and closed only after '
    'refresh_search_matviews() succeeds, so a refresh failure after a '
    'committed decision stays retryable without repeating the decision.';
COMMENT ON COLUMN aircraft_read.read_model_refresh_requests.requested_by IS
    'Subsystem that made the read model stale, for example ''CURATION''.';
COMMENT ON COLUMN aircraft_read.read_model_refresh_requests.reason IS
    'Bounded human-readable cause, for example ''assertion 42 ACCEPTED''. '
    'Diagnostic only; never a source-controlled value.';
COMMENT ON COLUMN aircraft_read.read_model_refresh_requests.attempts IS
    'Failed refresh attempts recorded against this request. Diagnostics only: '
    'a request is never abandoned because its attempt count grew.';
COMMENT ON COLUMN aircraft_read.read_model_refresh_requests.last_error IS
    'Sanitized, length-bounded error from the most recent failed attempt.';

-- The retry drains oldest-first; completed rows are audit history and are
-- deliberately left out of the index.
-- squawk-ignore require-concurrent-index-creation
CREATE INDEX IF NOT EXISTS idx_rmrr_pending
    ON aircraft_read.read_model_refresh_requests (id)
    WHERE status_code = 'PENDING';

COMMIT;
