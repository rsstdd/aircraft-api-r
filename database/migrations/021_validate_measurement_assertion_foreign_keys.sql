-- Validate the measurement-to-assertion foreign keys added NOT VALID by
-- migration 020. Keeping validation in its own transaction avoids holding the
-- stronger constraint-creation locks while PostgreSQL scans the tables.

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

ALTER TABLE aircraft_specs.performance_metrics
    VALIDATE CONSTRAINT fk_pm_source_assertion;
ALTER TABLE aircraft_specs.weight_metrics
    VALIDATE CONSTRAINT fk_wm_source_assertion;

COMMIT;
