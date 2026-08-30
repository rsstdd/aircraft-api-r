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

GRANT USAGE, SELECT ON SEQUENCE
    aircraft_org.organizations_id_seq,
    aircraft_core.families_id_seq,
    aircraft_core.models_id_seq,
    aircraft_core.variants_id_seq,
    aircraft_specs.performance_metrics_id_seq,
    aircraft_specs.weight_metrics_id_seq,
    aircraft_power.engine_variants_id_seq,
    aircraft_power.variant_powerplants_id_seq,
    aircraft_market.valuations_id_seq,
    aircraft_market.cost_snapshots_id_seq,
    aircraft_market.cost_line_items_id_seq,
    aircraft_market.cost_snapshot_totals_id_seq,
    aircraft_prov.sources_id_seq,
    aircraft_prov.source_documents_id_seq,
    aircraft_prov.source_assertions_id_seq,
    aircraft_prov.curation_flags_id_seq,
    aircraft_ingest.ingest_runs_id_seq,
    aircraft_ingest.ingest_run_attempts_id_seq,
    aircraft_ingest.staged_aircraft_id_seq,
    aircraft_ingest.staged_images_id_seq
TO :"ingest_role";

GRANT EXECUTE ON FUNCTION aircraft_read.refresh_search_matviews(BOOLEAN)
TO :"ingest_role";
