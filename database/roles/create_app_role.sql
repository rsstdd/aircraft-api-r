-- Create the restricted server login role that app_grants.sql expects.
--
-- This is the role `aircraft-server` connects as through APP__DATABASE__URL. It
-- is deliberately not the schema owner: the owner can create schemas, tables,
-- and extensions, and a runtime that can rewrite the schema it queries is a
-- migration path nobody approved.
--
-- install.sql deliberately does not create roles, so this is a separate,
-- explicitly invoked step. Run it as an administrator, then run app_grants.sql
-- with the same -v app_role value. `just db-create-app-role` does both halves
-- against the local container.
--
-- The password is read from the API_ROLE_PASSWORD environment variable rather
-- than a -v argument so it never appears in the process argument list.
-- \getenv requires psql 15 or newer.
--
-- This creates the login role only. Every privilege the role holds comes from
-- app_grants.sql, which is the single place the server grant surface is
-- defined. The restriction clauses below are asserted by
-- `the_runtime_role_cannot_create_schemas_extensions_tables_or_roles` in
-- crates/aircraft_db/tests/pool.rs, which runs this file verbatim.

-- Each guard raises rather than only printing, so a misconfigured invocation
-- fails under ON_ERROR_STOP instead of exiting zero having created nothing.
\if :{?app_role}
\else
DO $missing_role$ BEGIN
    RAISE EXCEPTION
        'app_role is required, for example: -v app_role=aircraft_api_app';
END $missing_role$;
\quit
\endif

\getenv app_password API_ROLE_PASSWORD
\if :{?app_password}
\else
DO $missing_password$ BEGIN
    RAISE EXCEPTION 'API_ROLE_PASSWORD must be set in the environment';
END $missing_password$;
\quit
\endif

-- \gexec runs the generated statement only when the role is absent, so
-- re-running this file leaves an existing role and its password untouched.
SELECT format(
    'CREATE ROLE %I LOGIN PASSWORD %L '
    'NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS',
    :'app_role',
    :'app_password'
)
WHERE NOT EXISTS (
    SELECT 1 FROM pg_roles WHERE rolname = :'app_role'
)
\gexec

-- An existing role is not assumed to be the role this file would have created.
-- app_grants.sql must never widen a privileged or membership-bearing account,
-- so an existing role has to satisfy the same restricted contract before the
-- grant step is allowed to run against it.
SELECT coalesce(
    bool_and(
        rolcanlogin
        AND NOT (
            rolsuper
            OR rolcreatedb
            OR rolcreaterole
            OR rolreplication
            OR rolbypassrls
        )
        AND NOT EXISTS (
            SELECT 1
            FROM pg_catalog.pg_auth_members
            WHERE member = pg_roles.oid
        )
    ),
    false
)::TEXT AS app_role_conforms
FROM pg_catalog.pg_roles
WHERE rolname = :'app_role'
\gset

\if :app_role_conforms
\else
\echo 'nonconforming app role:' :app_role
DO $nonconforming_role$ BEGIN
    RAISE EXCEPTION
        'existing app role must be a LOGIN role that is NOSUPERUSER, '
        'NOCREATEDB, NOCREATEROLE, NOREPLICATION, NOBYPASSRLS and holds no '
        'role memberships; do not run app_grants.sql against it';
END $nonconforming_role$;
\quit
\endif

SELECT CASE
    WHEN EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_role')
    THEN format('role %I is present; run app_grants.sql next', :'app_role')
    ELSE format('role %I was not created', :'app_role')
END AS result;
