DO $validation$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_pm_source_assertion'
          AND conrelid = 'aircraft_specs.performance_metrics'::regclass
          AND convalidated
    ) OR NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'fk_wm_source_assertion'
          AND conrelid = 'aircraft_specs.weight_metrics'::regclass
          AND convalidated
    ) THEN
        RAISE EXCEPTION 'Measurement-to-assertion foreign keys must be validated';
    END IF;
END
$validation$;
