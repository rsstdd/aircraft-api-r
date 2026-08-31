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

-- A variable that is defined but empty passes the presence check above, and
-- PASSWORD '' is not an empty password: PostgreSQL 10 and later store it as a
-- null password, which makes password authentication always fail. Without this,
-- an empty API_ROLE_PASSWORD creates a role nothing can log in as, while the
-- script still exits reporting success.
SELECT length(:'app_password') = 0 AS app_password_empty \gset
\if :app_password_empty
DO $empty_password$ BEGIN
    RAISE EXCEPTION 'API_ROLE_PASSWORD must not be empty';
END $empty_password$;
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
        -- Role attributes alone do not bound what a role may do, and this file
        -- is the gate that decides whether app_grants.sql may run against an
        -- existing account. CREATE on the database allows new schemas and
        -- trusted extensions; CREATE on a schema allows new objects inside it;
        -- and an owner may alter or drop what it owns regardless of either.
        -- A pre-existing role holding any of those keeps them afterwards,
        -- because app_grants.sql adds only CONNECT and takes back nothing
        -- but TEMPORARY -- so the restricted contract has to be checked as
        -- effective privilege, not as flags.
        --
        -- System schemas are excluded deliberately: CREATE on a pg_temp schema
        -- follows the database TEMPORARY privilege, which PostgreSQL grants to
        -- PUBLIC, so including them would reject every role. This guard
        -- cannot test that privilege directly either: it runs before
        -- app_grants.sql, so on a fresh database TEMPORARY is still present
        -- and the clause would reject every role on the first run and accept
        -- the same role on the second. app_grants.sql revokes it instead --
        -- from PUBLIC and from the role -- so a role admitted here holding a
        -- direct grant does not keep it.
        AND NOT has_database_privilege(pg_roles.oid, current_database(), 'CREATE')
        AND NOT EXISTS (
            SELECT 1
            FROM pg_catalog.pg_namespace
            WHERE nspname NOT LIKE 'pg\_%'
              AND nspname <> 'information_schema'
              AND has_schema_privilege(pg_roles.oid, pg_namespace.oid, 'CREATE')
        )
        AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_class WHERE relowner = pg_roles.oid
        )
        AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_namespace WHERE nspowner = pg_roles.oid
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
        'NOCREATEDB, NOCREATEROLE, NOREPLICATION, NOBYPASSRLS, holds no '
        'role memberships, has no CREATE privilege on the database or any '
        'schema, and owns no schema or table; do not run app_grants.sql '
        'against it';
END $nonconforming_role$;
\quit
\endif

SELECT CASE
    WHEN EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_role')
    THEN format('role %I is present; run app_grants.sql next', :'app_role')
    ELSE format('role %I was not created', :'app_role')
END AS result;
