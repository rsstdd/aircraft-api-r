-- Grants for the restricted server runtime role created by create_app_role.sql.
--
-- CONNECT and nothing else, on purpose. `aircraft-server` builds a pool and
-- answers SELECT 1; it issues no application query yet, so it needs no schema
-- USAGE and no table SELECT. Read grants belong to the story that adds the
-- first route reading a table, where each one can be justified by a query that
-- exists. An unearned grant is harder to remove later than to add now.
--
-- CONNECT is granted to PUBLIC by default, so this statement is close to a
-- no-op on a fresh database. It is written out anyway: the grant surface of
-- this role is defined here and nowhere else, and a database whose PUBLIC
-- CONNECT has been revoked must still admit the server.
--
-- Applied by `just db-grant-app-role` and asserted by
-- `the_runtime_role_cannot_create_schemas_extensions_tables_or_roles` in
-- crates/aircraft_db/tests/pool.rs, which runs this file verbatim.

\if :{?app_role}
\else
\echo 'app_role is required, for example: -v app_role=aircraft_api_app'
\quit
\endif

GRANT CONNECT ON DATABASE :"DBNAME" TO :"app_role";
