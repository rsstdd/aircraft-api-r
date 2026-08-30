\if :{?ingest_role}
\else
\echo 'ingest_role is required, for example: -v ingest_role=aircraft_ingest_app'
\quit
\endif

GRANT CONNECT ON DATABASE :"DBNAME" TO :"ingest_role";

GRANT USAGE ON SCHEMA
    aircraft_ref,
    aircraft_org,
    aircraft_core,
    aircraft_specs,
    aircraft_power,
    aircraft_market,
    aircraft_prov,
    aircraft_ingest,
    aircraft_read
TO :"ingest_role";

GRANT SELECT ON ALL TABLES IN SCHEMA aircraft_ref
TO :"ingest_role";

GRANT SELECT, INSERT, UPDATE ON TABLE
    aircraft_org.organizations,
    aircraft_core.families,
    aircraft_core.models,
    aircraft_core.variants,
    aircraft_core.variant_manufacturers,
    aircraft_specs.performance_metrics,
    aircraft_specs.weight_metrics,
    aircraft_power.engine_variants,
    aircraft_power.variant_powerplants,
    aircraft_market.valuations,
    aircraft_market.cost_snapshots,
    aircraft_market.cost_line_items,
    aircraft_market.cost_snapshot_totals,
    aircraft_prov.sources,
    aircraft_prov.source_documents,
    aircraft_prov.source_assertions,
    aircraft_prov.curation_flags,
    aircraft_ingest.ingest_runs,
    aircraft_ingest.ingest_run_attempts,
    aircraft_ingest.staged_aircraft,
    aircraft_ingest.staged_images,
    aircraft_read.read_model_refresh_requests
TO :"ingest_role";

-- Sequence privileges are deliberately absent, and this revoke keeps them that
-- way. Every surrogate key above is GENERATED ALWAYS AS IDENTITY, whose
-- sequence is advanced through an internal expression node that performs no
-- privilege check -- unlike a serial column, where the nextval() default is
-- evaluated as the inserting role and does require USAGE.
--
-- Revisions of this file through 2026-08 granted USAGE, SELECT on twenty
-- identity sequences regardless. Granting is not convergent: dropping those
-- lines leaves the privileges standing on any database already provisioned
-- from an older revision, so the role would hold a surface this file no longer
-- defines. The revoke closes that gap and is a no-op on a fresh install.
REVOKE ALL ON ALL SEQUENCES IN SCHEMA
    aircraft_ref,
    aircraft_org,
    aircraft_core,
    aircraft_specs,
    aircraft_power,
    aircraft_market,
    aircraft_prov,
    aircraft_ingest,
    aircraft_read
FROM :"ingest_role";

GRANT EXECUTE ON FUNCTION aircraft_read.refresh_search_matviews(BOOLEAN)
TO :"ingest_role";
