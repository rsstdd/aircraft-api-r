-- Grants for the restricted server runtime role created by create_app_role.sql.
--
-- One CONNECT grant, two revokes, and the column-level reads that two read
-- paths need: the credential verification lookup, `LOOKUP_CREDENTIAL` in
-- crates/aircraft_db/src/repositories/authentication_repository.rs, and the
-- reference-catalog statements in
-- crates/aircraft_db/src/repositories/reference_repository.rs. Both name this
-- file in turn. Each grant below is justified by a column one of those
-- statements reads and by nothing else; the timestamps they do not read, every
-- write, and every sequence stay ungranted. A column added to a statement
-- without a grant here fails with 42501, which
-- `the_runtime_role_executes_only_the_verification_projection` in
-- crates/aircraft_db/tests/authentication.rs and
-- `the_runtime_role_reads_every_catalog_and_writes_none` in
-- crates/aircraft_db/tests/reference_catalogs.rs assert from both sides. An
-- unearned grant is harder to remove later than to add now.
--
-- The schema must already be installed: GRANT on a table that does not exist
-- fails, which is the right answer for a grant run out of order.
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

-- The verification projection, column by column. USAGE on the schema is what
-- lets the role name these tables at all. Reading a column of the lookup_code
-- domain and casting it to text needs no privilege on the domain itself.
GRANT USAGE ON SCHEMA aircraft_auth TO :"app_role";
GRANT SELECT (key_id, principal_id, secret_digest, revoked_at)
    ON aircraft_auth.api_credentials TO :"app_role";
GRANT SELECT (id, rate_limit_tier_code, disabled_at)
    ON aircraft_auth.principals TO :"app_role";
GRANT SELECT (principal_id, scope_code)
    ON aircraft_auth.principal_scope_grants TO :"app_role";

-- The reference-catalog projection, one grant per catalog, for
-- `GET /v1/reference/{catalog}`. SELECT is needed on every column a statement
-- names, not only the ones it returns: `is_active` appears in the predicate and
-- `sort_order` in the ordering, and PostgreSQL checks both. The column lists
-- below therefore differ per table exactly as the statements do -- four of the
-- thirty-six catalogs lack `description`, `sort_order`, or `is_active` in
-- database/migrations/002_core_reference_tables.sql, and their grants lack the
-- same columns.
--
-- Read-only, and deliberately so: reference data is curated through migrations
-- and seeds, never over HTTP. No INSERT, UPDATE, or DELETE is granted on any
-- of these tables, and the gate asserts the refusal.
GRANT USAGE ON SCHEMA aircraft_ref TO :"app_role";

GRANT SELECT (code, label, description, sort_order)
    ON aircraft_ref.unit_categories TO :"app_role";
GRANT SELECT (code, label, sort_order, is_active)
    ON aircraft_ref.measurement_units TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.aircraft_roles TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.service_statuses TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.variant_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.landing_gear_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.propulsion_categories TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.fuel_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.dimension_metric_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.weight_metric_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.performance_metric_types TO :"app_role";
GRANT SELECT (code, label, sort_order, is_active)
    ON aircraft_ref.certification_authorities TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.airworthiness_categories TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.pilot_certificate_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.operating_approval_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.military_mission_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.weapon_categories TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.hardpoint_position_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.stores_types TO :"app_role";
GRANT SELECT (code, label, is_active)
    ON aircraft_ref.currencies TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.cost_item_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.aircraft_condition_grades TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.ad_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.sb_compliance_statuses TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.availability_grades TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.source_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.source_reliability_grades TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.curation_flag_statuses TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.curation_entity_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.assertion_statuses TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.mission_profile_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.comparison_criterion_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.organization_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.org_relationship_types TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.systems_categories TO :"app_role";
GRANT SELECT (code, label, description, sort_order, is_active)
    ON aircraft_ref.equipment_provision_types TO :"app_role";
