-- Grants for the restricted server runtime role created by create_app_role.sql.
--
-- One CONNECT grant and two revokes, and nothing else. `aircraft-server` builds
-- a pool and answers SELECT 1; it issues no application query yet, so it needs
-- no schema USAGE and no table SELECT. Read grants belong to the story that
-- adds the first route reading a table, where each one can be justified by a
-- query that exists. An unearned grant is harder to remove later than to add
-- now.
--
-- CONNECT is granted to PUBLIC by default, so this statement is close to a
-- no-op on a fresh database. It is written out anyway: the grant surface of
-- this role is defined here and nowhere else, and a database whose PUBLIC
-- CONNECT has been revoked must still admit the server.
--
-- Applied by `just db-grant-app-role` and asserted by
-- `the_runtime_role_cannot_create_schemas_extensions_tables_or_roles` in
-- crates/aircraft_db/tests/pool.rs, which runs this file verbatim.

-- The guard raises rather than only printing. A bare \quit ends psql with exit
-- status 0, which every caller -- the justfile recipe, and `run_psql` in the
-- gate -- reads as success, so a misconfigured invocation would report a grant
-- it never applied.
\if :{?app_role}
\else
DO $missing_role$ BEGIN
    RAISE EXCEPTION
        'app_role is required, for example: -v app_role=aircraft_api_app';
END $missing_role$;
\quit
\endif

GRANT CONNECT ON DATABASE :"DBNAME" TO :"app_role";

-- Two revokes, because a role's effective privilege is the sum of its own
-- grants and those it holds through PUBLIC, and each statement reaches only one
-- of those. Neither is redundant.
--
-- PostgreSQL grants TEMPORARY on a database to PUBLIC and offers no per-role
-- deny: REVOKE ... FROM :"app_role" cannot take back what the role holds
-- through PUBLIC. Without the first statement the restricted role creates
-- temporary tables -- which is exactly what its contract says it cannot do.
--
-- The first statement is the only database-wide one in this file, and its cost
-- is that every other non-superuser loses TEMPORARY as well, the ingestion role
-- included. Nothing in this repository creates a temporary table; ingestion
-- stages into permanent tables under aircraft_ingest, and superusers bypass the
-- check, so install.sql and the migrations are unaffected. Read this before
-- aiming `just db-grant-app-role` at a database shared with anything outside
-- this repository.
--
-- The second statement covers a role that arrives already holding a direct
-- grant. The conformance guard in create_app_role.sql bounds an existing role
-- by effective CREATE privilege and does not inspect TEMPORARY, so it admits
-- such a role, and the PUBLIC revoke leaves that role's own grant untouched.
--
-- Revoking is convergent where granting is not, for the same reason
-- ingest_grants.sql revokes sequence privileges rather than merely not granting
-- them: a database provisioned before these lines existed still carries the
-- PUBLIC grant, and only a revoke takes it back.
REVOKE TEMPORARY ON DATABASE :"DBNAME" FROM PUBLIC;
REVOKE TEMPORARY ON DATABASE :"DBNAME" FROM :"app_role";
