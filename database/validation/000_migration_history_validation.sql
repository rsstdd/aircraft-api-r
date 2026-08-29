-- The installer must record every canonical migration exactly once.
DO $validation$
DECLARE
    expected_versions CONSTANT TEXT[] := ARRAY[
        '001', '002', '003', '004', '005', '006', '007', '008',
        '009', '010', '011', '012', '013', '014', '015', '016', '017', '018',
        '019', '020', '021'
    ];
    actual_versions TEXT[];
BEGIN
    SELECT array_agg(version ORDER BY version)
    INTO actual_versions
    FROM public.aircraft_schema_migrations;

    IF actual_versions IS DISTINCT FROM expected_versions THEN
        RAISE EXCEPTION
            'Migration history is %, expected %',
            actual_versions,
            expected_versions;
    END IF;
END
$validation$;
