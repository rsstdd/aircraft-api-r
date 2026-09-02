-- Companion to database/migrations/025_authentication_schema.sql.
--
-- Structure only, and deliberately read-only: db-prod-validate runs this
-- directory against production. That these constraints actually reject bad
-- values is proved in crates/aircraft_db/tests/auth_schema.rs.

DO $validation$
DECLARE
    missing TEXT;
BEGIN
    IF to_regnamespace('aircraft_auth') IS NULL THEN
        RAISE EXCEPTION 'schema aircraft_auth must exist';
    END IF;

    FOREACH missing IN ARRAY ARRAY[
        'aircraft_auth.rate_limit_tiers',
        'aircraft_auth.scopes',
        'aircraft_auth.principals',
        'aircraft_auth.api_credentials',
        'aircraft_auth.principal_scope_grants'
    ] LOOP
        IF to_regclass(missing) IS NULL THEN
            RAISE EXCEPTION '% must exist', missing;
        END IF;
    END LOOP;
END
$validation$;

-- Pin the allowed credential inventory through the unfiltered system catalog.
DO $validation$
DECLARE
    expected_columns CONSTANT TEXT[] := ARRAY[
        'created_at', 'key_id', 'label', 'principal_id',
        'revoked_at', 'secret_digest', 'updated_at'
    ];
    actual_columns TEXT[];
BEGIN
    SELECT array_agg(attname::TEXT ORDER BY attname)
    INTO actual_columns
    FROM pg_attribute
    WHERE attrelid = 'aircraft_auth.api_credentials'::regclass
      AND attnum > 0
      AND NOT attisdropped;

    IF actual_columns IS DISTINCT FROM expected_columns THEN
        RAISE EXCEPTION
            'aircraft_auth.api_credentials stores %, expected %',
            actual_columns, expected_columns;
    END IF;
END
$validation$;

-- Value constraints are compared as complete normalized definitions, never by
-- substring. A substring match cannot express "and nothing else": a digest
-- constraint relaxed to '[0-9a-f]{64}' still contains the intended pattern, and
-- so does one widened with an extra disjunct such as OR secret_digest = '...'.
-- Both would be accepted by a LIKE and both would let a clear credential in,
-- because PostgreSQL's ~ is a containment match unless anchored.
--
-- The same trap applies to the non-blank tests: btrim(x) <> '' reads as
-- equivalent to a non-whitespace test but strips spaces only, so a tab- or
-- newline-only value survives it.
--
-- pg_get_constraintdef renders from the parsed expression. A later migration
-- that changes any of these constraints must change this table with it. That
-- coupling is the point of asserting the whole definition.
DO $validation$
DECLARE
    expected CONSTANT TEXT[][] := ARRAY[
        ['chk_apc_secret_digest', 'aircraft_auth.api_credentials',
         'CHECK ((secret_digest ~ ''^[0-9a-f]{64}$''::text))'],
        ['chk_apc_label', 'aircraft_auth.api_credentials',
         'CHECK (((label ~ ''[^[:space:]]''::text) AND (char_length(label) <= 200)))'],
        ['chk_prn_name', 'aircraft_auth.principals',
         'CHECK ((name ~ ''[^[:space:]]''::text))'],
        ['chk_rlt_label', 'aircraft_auth.rate_limit_tiers',
         'CHECK ((label ~ ''[^[:space:]]''::text))'],
        ['chk_scp_label', 'aircraft_auth.scopes',
         'CHECK ((label ~ ''[^[:space:]]''::text))']
    ];
    actual TEXT;
    index_position INTEGER;
BEGIN
    FOR index_position IN 1 .. array_length(expected, 1) LOOP
        SELECT pg_get_constraintdef(oid) INTO actual
        FROM pg_constraint
        WHERE conname = expected[index_position][1]
          AND conrelid = expected[index_position][2]::regclass;

        IF actual IS DISTINCT FROM expected[index_position][3] THEN
            RAISE EXCEPTION
                '% is %, expected exactly %',
                expected[index_position][1],
                coalesce(actual, 'absent'),
                expected[index_position][3];
        END IF;
    END LOOP;
END
$validation$;

DO $validation$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_attribute
        WHERE attrelid = 'aircraft_auth.api_credentials'::regclass
          AND attname = 'key_id'
          AND atttypid = 'uuid'::regtype
    ) THEN
        RAISE EXCEPTION 'api_credentials.key_id must be a uuid';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_index
        JOIN pg_attribute
          ON pg_attribute.attrelid = pg_index.indrelid
         AND pg_attribute.attnum = pg_index.indkey[0]
        WHERE pg_index.indrelid = 'aircraft_auth.api_credentials'::regclass
          AND pg_index.indisprimary
          AND pg_index.indnatts = 1
          AND pg_index.indpred IS NULL
          AND pg_attribute.attname = 'key_id'
    ) THEN
        RAISE EXCEPTION
            'api_credentials must be keyed by key_id alone, and unconditionally';
    END IF;
END
$validation$;

-- confdeltype: r = RESTRICT, c = CASCADE.
DO $validation$
DECLARE
    expected CONSTANT TEXT[][] := ARRAY[
        ['fk_prn_rate_limit_tier', 'aircraft_auth.principals', 'r'],
        ['fk_apc_principal', 'aircraft_auth.api_credentials', 'r'],
        ['fk_psg_principal', 'aircraft_auth.principal_scope_grants', 'c'],
        ['fk_psg_scope', 'aircraft_auth.principal_scope_grants', 'r']
    ];
    index_position INTEGER;
BEGIN
    FOR index_position IN 1 .. array_length(expected, 1) LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_constraint
            WHERE conname = expected[index_position][1]
              AND conrelid = expected[index_position][2]::regclass
              AND contype = 'f'
              AND confdeltype = expected[index_position][3]
        ) THEN
            RAISE EXCEPTION
                '% on % must exist with ON DELETE %',
                expected[index_position][1],
                expected[index_position][2],
                CASE expected[index_position][3]
                    WHEN 'r' THEN 'RESTRICT' ELSE 'CASCADE'
                END;
        END IF;
    END LOOP;
END
$validation$;

DO $validation$
DECLARE
    expected CONSTANT TEXT[][] := ARRAY[
        ['aircraft_auth', 'principals', 'idx_prn_disabled'],
        ['aircraft_auth', 'api_credentials', 'idx_apc_principal'],
        ['aircraft_auth', 'api_credentials', 'idx_apc_revoked'],
        ['aircraft_auth', 'principal_scope_grants', 'idx_psg_scope']
    ];
    index_position INTEGER;
BEGIN
    FOR index_position IN 1 .. array_length(expected, 1) LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_indexes
            WHERE schemaname = expected[index_position][1]
              AND tablename = expected[index_position][2]
              AND indexname = expected[index_position][3]
        ) THEN
            RAISE EXCEPTION
                'index % must index revocation, disablement, ownership, and grants',
                expected[index_position][3];
        END IF;
    END LOOP;
END
$validation$;

-- Grants are inserted or deleted, so only mutable entity tables need triggers.
DO $validation$
DECLARE
    expected CONSTANT TEXT[][] := ARRAY[
        ['trg_rlt_updated', 'aircraft_auth.rate_limit_tiers'],
        ['trg_scp_updated', 'aircraft_auth.scopes'],
        ['trg_prn_updated', 'aircraft_auth.principals'],
        ['trg_apc_updated', 'aircraft_auth.api_credentials']
    ];
    index_position INTEGER;
BEGIN
    FOR index_position IN 1 .. array_length(expected, 1) LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_trigger
            WHERE tgname = expected[index_position][1]
              AND tgrelid = expected[index_position][2]::regclass
              AND tgfoid = 'aircraft_ref.set_updated_at'::regproc
        ) THEN
            RAISE EXCEPTION
                '% must stamp updated_at through aircraft_ref.set_updated_at()',
                expected[index_position][1];
        END IF;
    END LOOP;
END
$validation$;

-- Pin the closed protected-policy set from the accepted HTTP decision.
DO $validation$
DECLARE
    expected_scopes CONSTANT TEXT[] := ARRAY[
        'ADMIN', 'CATALOG_READ', 'CURATION_READ', 'CURATION_WRITE', 'MILITARY_READ'
    ];
    actual_scopes TEXT[];
BEGIN
    SELECT array_agg(code::TEXT ORDER BY code) INTO actual_scopes FROM aircraft_auth.scopes;

    IF actual_scopes IS DISTINCT FROM expected_scopes THEN
        RAISE EXCEPTION
            'aircraft_auth.scopes holds %, expected the protected policies %',
            actual_scopes, expected_scopes;
    END IF;
END
$validation$;
