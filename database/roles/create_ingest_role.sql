-- Create the restricted ingestion login role that ingest_grants.sql expects.
--
-- install.sql deliberately does not create roles, so this is a separate,
-- explicitly invoked step. Run it as an administrator, then run
-- ingest_grants.sql with the same -v ingest_role value.
--
-- The password is read from the INGEST_ROLE_PASSWORD environment variable
-- rather than a -v argument so it never appears in the process argument list.
-- \getenv requires psql 15 or newer.
--
-- This creates the login role only. Every privilege the role holds comes from
-- ingest_grants.sql, which is the single place the ingestion grant surface is
-- defined.

-- Each guard raises rather than only printing, so a misconfigured invocation
-- fails under ON_ERROR_STOP instead of exiting zero having created nothing.
\if :{?ingest_role}
\else
DO $missing_role$ BEGIN
    RAISE EXCEPTION
        'ingest_role is required, for example: -v ingest_role=aircraft_ingest_app';
END $missing_role$;
\quit
\endif

\getenv ingest_password INGEST_ROLE_PASSWORD
\if :{?ingest_password}
\else
DO $missing_password$ BEGIN
    RAISE EXCEPTION 'INGEST_ROLE_PASSWORD must be set in the environment';
END $missing_password$;
\quit
\endif

-- \gexec runs the generated statement only when the role is absent, so
-- re-running this file leaves an existing role and its password untouched.
SELECT format(
    'CREATE ROLE %I LOGIN PASSWORD %L '
    'NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS',
    :'ingest_role',
    :'ingest_password'
)
WHERE NOT EXISTS (
    SELECT 1 FROM pg_roles WHERE rolname = :'ingest_role'
)
\gexec

SELECT CASE
    WHEN EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'ingest_role')
    THEN format('role %I is present; run ingest_grants.sql next', :'ingest_role')
    ELSE format('role %I was not created', :'ingest_role')
END AS result;
