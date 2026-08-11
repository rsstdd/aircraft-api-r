-- =============================================================================
-- File: database/migrations/014_sources_provenance_curation_audit.sql
-- Phase 14 — aircraft_prov: the multi-source provenance backbone.
--
-- Five-table architecture:
--   sources           — catalog of data sources (PlanePHD, Jane's, manual, …)
--   source_documents  — raw documents/pages retrieved per source
--   source_assertions — field-level attribution: every canonical value traces here
--   curation_flags    — curation issue tracker (polymorphic entity reference)
--   audit_log         — immutable change history for canonical data
--
-- Polymorphic reference pattern used in source_assertions and curation_flags:
--   (entity_type_code → aircraft_ref.curation_entity_types,
--    entity_id         → PK of the target table named by the entity type)
-- This avoids dozens of per-table assertion/flag tables while maintaining
-- type safety through the entity_types FK.
--
-- Phase 2 lookups consumed here:
--   aircraft_ref.source_types              (8 rows)
--   aircraft_ref.source_reliability_grades (5 rows)
--   aircraft_ref.assertion_statuses        (5 rows)
--   aircraft_ref.curation_flag_statuses    (5 rows, including is_terminal)
--   aircraft_ref.curation_entity_types     (11 rows with schema/table names)
--
-- Spec coverage (requirement 12):
--   source catalog, reliability           → sources
--   source URL, retrieval timestamp       → source_documents
--   raw document retention                → source_documents.raw_json JSONB
--   parser versioning                     → source_documents.parser_version
--   field-level source attribution        → source_assertions
--   conflicting values from multiple srcs → source_assertions (multiple rows per field)
--   confidence scores                     → source_assertions.confidence
--   is_accepted flag                      → source_assertions.is_accepted
--   human review status                   → curation_flags.status_code
--   audit history                         → audit_log
--   stale-data detection                  → source_documents.retrieved_at + Phase 16 view
-- =============================================================================

BEGIN;

-- =============================================================================
-- aircraft_prov.sources
-- Catalog of data sources. One row per named source system or publication.
-- (PlanePHD, Jane's All the World's Aircraft, FAA TCDS, manual curation, …)
-- default_confidence: baseline confidence score for assertions from this source.
-- Derived from: reliability_grade.numeric_score / 5 at seed time;
-- can be overridden by the curator.
-- refresh_interval_days: for scraped sources, desired re-retrieval cadence.
-- =============================================================================

CREATE TABLE aircraft_prov.sources (
    id                      BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug                    aircraft_ref.slug_text NOT NULL UNIQUE,
    name                    TEXT NOT NULL,
    source_type_code        aircraft_ref.lookup_code
                                REFERENCES aircraft_ref.source_types(code),
    reliability_grade_code  aircraft_ref.lookup_code
                                REFERENCES aircraft_ref.source_reliability_grades(code),
    base_url                TEXT,
    license_notes           TEXT,         -- data license or terms of use summary
    default_confidence      aircraft_ref.confidence_score,
    refresh_interval_days   SMALLINT,     -- NULL = manual / one-time import
    is_active               BOOLEAN NOT NULL DEFAULT TRUE,
    notes                   TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_prov.sources IS
    'Catalog of data sources. One row per named source (PlanePHD, Jane''s, manual). '
    'default_confidence is the baseline aircraft_ref.confidence_score for assertions '
    'originating from this source; derived from reliability_grade.numeric_score / 5. '
    'Source documents (individual pages/files) are in source_documents.';
COMMENT ON COLUMN aircraft_prov.sources.default_confidence IS
    'Baseline confidence for assertions from this source (0.00–1.00). '
    'Derived from reliability_grade_code.numeric_score / 5 at creation. '
    'Curators may override per-assertion in source_assertions.confidence.';

-- =============================================================================
-- aircraft_prov.source_documents
-- Raw document archive: one row per page, file, or API response retrieved.
-- raw_json: preserved verbatim for re-parsing when the schema evolves.
-- source_system_key: the source system's own identifier for this document
--   (e.g., PlanePHD numeric source_id). Partial UNIQUE index prevents
--   ingesting the same document twice from the same source.
-- ingest_batch_label: optional grouping label for documents ingested together
--   (e.g., "planephd_bulk_2024_01"). Phase 17 sets this during ingestion.
-- variant_id: linked once the variant is created/matched post-ingestion.
-- =============================================================================

CREATE TABLE aircraft_prov.source_documents (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source_id           BIGINT NOT NULL
                            REFERENCES aircraft_prov.sources(id)       ON DELETE RESTRICT,
    source_url          TEXT,             -- specific URL of this document
    source_path         TEXT,             -- URI path within the source system
    source_system_key   TEXT,             -- source's own ID (e.g., PlanePHD source_id)
    -- Phase 17 ingestion label for grouping documents from one import run.
    ingest_batch_label  TEXT,
    parser_version      TEXT,             -- e.g., 'planephd_etl_v2.1.0'
    retrieved_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Preserved raw content for re-parsing.
    raw_json            JSONB,
    -- Linked variant (set after aircraft_core.variants row is created/matched).
    -- NULL until the document is promoted into the canonical model.
    variant_id          BIGINT
                            REFERENCES aircraft_core.variants(id)      ON DELETE SET NULL,
    processing_status   TEXT NOT NULL DEFAULT 'PENDING',
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_sdo_status CHECK (
        processing_status IN ('PENDING', 'PROCESSED', 'FAILED', 'REPROCESS')
    )
);

COMMENT ON TABLE aircraft_prov.source_documents IS
    'Raw document archive. One row per page, file, or API response. '
    'raw_json preserves the source payload verbatim for re-parsing. '
    'source_system_key + source_id are deduplicated by a partial UNIQUE index '
    'to prevent duplicate ingestion of the same source document. '
    'variant_id is filled after the variant row is created; NULL = unlinked. '
    'processing_status tracks the ETL lifecycle per document.';
COMMENT ON COLUMN aircraft_prov.source_documents.source_system_key IS
    'Source system''s own identifier for this document '
    '(e.g., PlanePHD numeric source_id). '
    'Used by Phase 17 ingestion to check "did I already process this document?" '
    'via the uq_sd_source_key partial UNIQUE index.';
COMMENT ON COLUMN aircraft_prov.source_documents.raw_json IS
    'Verbatim raw content from the source. Preserved to support: '
    '(1) re-parsing after schema changes; (2) conflict resolution audits; '
    '(3) source-fidelity verification. Indexed with GIN jsonb_path_ops.';

-- =============================================================================
-- aircraft_prov.source_assertions
-- Field-level provenance: one row per (source_document, entity, field) assertion.
-- Multiple rows per field are expected — one per source that reported a value.
-- is_accepted: at most one TRUE per (entity_type, entity_id, field_name),
--   enforced by partial UNIQUE index. The accepted row is the value the
--   Phase 16 read models use for cross-fleet comparison.
-- Conflicts arise when multiple assertions for the same field disagree;
-- the curation workflow resolves conflicts by accepting one and rejecting others.
-- =============================================================================

CREATE TABLE aircraft_prov.source_assertions (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source_document_id  BIGINT NOT NULL
                            REFERENCES aircraft_prov.source_documents(id) ON DELETE CASCADE,
    -- Polymorphic entity reference (entity_type → curation_entity_types.schema/table).
    entity_type_code    aircraft_ref.lookup_code NOT NULL
                            REFERENCES aircraft_ref.curation_entity_types(code),
    entity_id           BIGINT NOT NULL,
    -- The field within the entity this assertion pertains to.
    -- Matches the PostgreSQL column name or a dotted path for JSONB sub-fields.
    field_name          TEXT NOT NULL,
    -- Source-fidelity values (verbatim from document)
    raw_value           TEXT,             -- '122 KIAS', '$27,921', '1 x 65 HP'
    raw_unit            TEXT,             -- unit string extracted from raw_value
    -- Parsed / normalised values
    asserted_value      TEXT,             -- normalised string (canonical form)
    asserted_numeric    NUMERIC,          -- numeric value (if field is numeric)
    -- Review lifecycle
    status_code         aircraft_ref.lookup_code NOT NULL DEFAULT 'PENDING'
                            REFERENCES aircraft_ref.assertion_statuses(code),
    -- TRUE = this assertion is designated as the canonical value for this field.
    -- At most one TRUE per (entity_type, entity_id, field_name).
    is_accepted         BOOLEAN NOT NULL DEFAULT FALSE,
    confidence          aircraft_ref.confidence_score,
    notes               TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE aircraft_prov.source_assertions IS
    'Field-level provenance. One row per (source_document, entity, field) assertion. '
    'Multiple sources may assert different values for the same field; '
    'is_accepted marks the chosen canonical value (at most one per field per entity, '
    'enforced by uq_assertion_accepted partial UNIQUE index). '
    'Conflicts (multiple PENDING assertions, no ACCEPTED) generate curation_flags. '
    'Phase 17 ingestion creates PENDING assertions; curators accept/reject them.';
COMMENT ON COLUMN aircraft_prov.source_assertions.field_name IS
    'PostgreSQL column name of the asserted field on the entity table, '
    'or a dotted path for JSONB sub-fields (e.g., "canonical_value", '
    '"extra_attributes.engine_category_hint"). '
    'Standardised to the column name for direct mapping in curation tooling.';
COMMENT ON COLUMN aircraft_prov.source_assertions.is_accepted IS
    'TRUE = this assertion is the designated canonical value for this field. '
    'At most one TRUE per (entity_type_code, entity_id, field_name), enforced '
    'by the partial UNIQUE index uq_assertion_accepted. '
    'Phase 17 ingestion sets is_accepted = TRUE for the first assertion per field; '
    'subsequent conflicting assertions arrive as PENDING and require curator review.';

-- =============================================================================
-- aircraft_prov.curation_flags
-- Curation issue tracker. Polymorphic (entity_type_code, entity_id) reference
-- to any entity in the schema. May also reference a specific field_name.
-- issue_type: free-text classification (e.g., 'MISSING_VALUE',
--   'CONFLICTING_SOURCES', 'IMPLAUSIBLE_VALUE', 'PARSE_FAILURE').
-- Free-text (not FK) for maximum extensibility without schema changes.
-- priority: 1 = urgent/blocking, 5 = low/cosmetic.
--
-- FIX: chk_cf_resolution was originally written as a CHECK constraint with a
-- subquery against aircraft_ref.curation_flag_statuses. PostgreSQL does not
-- allow subqueries in CHECK constraints ("cannot use subquery in check
-- constraint"), so CREATE TABLE would fail outright. Enforcement is now done
-- with a BEFORE INSERT/UPDATE trigger (see aircraft_prov.enforce_curation_flag_resolution
-- below and trg_cf_enforce_resolution in the TRIGGERS section), mirroring the
-- same cross-table-CHECK workaround used in
-- aircraft_market.reject_aggregate_line_item() (Phase 12).
-- =============================================================================

CREATE TABLE aircraft_prov.curation_flags (
    id                  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    entity_type_code    aircraft_ref.lookup_code NOT NULL
                            REFERENCES aircraft_ref.curation_entity_types(code),
    entity_id           BIGINT NOT NULL,
    -- NULL = flag applies to the whole entity; non-NULL = specific field.
    field_name          TEXT,
    -- Free-text issue classification for flexible categorisation.
    issue_type          TEXT NOT NULL,
    issue_description   TEXT,
    status_code         aircraft_ref.lookup_code NOT NULL DEFAULT 'OPEN'
                            REFERENCES aircraft_ref.curation_flag_statuses(code),
    priority            SMALLINT NOT NULL DEFAULT 3,
    assigned_to         TEXT,             -- curator username or team
    resolved_by         TEXT,
    resolved_at         TIMESTAMPTZ,
    resolution_notes    TEXT,
    -- Optional link to a specific conflicting assertion.
    source_assertion_id BIGINT
                            REFERENCES aircraft_prov.source_assertions(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_cf_priority CHECK (priority BETWEEN 1 AND 5)
    -- chk_cf_resolution removed: enforced instead by trg_cf_enforce_resolution
    -- (see TRIGGERS section) because Postgres CHECK constraints cannot
    -- contain subqueries against another table.
);

COMMENT ON TABLE aircraft_prov.curation_flags IS
    'Curation issue tracker. Polymorphic (entity_type_code, entity_id) reference '
    'points to any catalogued entity type. issue_type is free-text for extensibility. '
    'priority 1=urgent (blocking), 5=low (cosmetic). '
    'status_code FK to aircraft_ref.curation_flag_statuses governs the lifecycle; '
    'RESOLVED and DISMISSED are terminal states (is_terminal = TRUE). '
    'resolved_at may only be set when status_code is terminal — enforced by '
    'trg_cf_enforce_resolution, not a CHECK, because the rule spans two tables. '
    'Replaces the reference schema''s flat curation_flags table, adding polymorphic '
    'entity reference and field-level granularity.';
COMMENT ON COLUMN aircraft_prov.curation_flags.issue_type IS
    'Free-text issue classification. Common values: '
    '''MISSING_VALUE'', ''CONFLICTING_SOURCES'', ''IMPLAUSIBLE_VALUE'', '
    '''PARSE_FAILURE'', ''DESCRIPTION_PARSE_INCOMPLETE'', ''STALE_DATA''. '
    'Not FK-constrained: extensibility over strictness.';
COMMENT ON CONSTRAINT chk_cf_priority ON aircraft_prov.curation_flags IS
    'Priority scale: 1 = urgent/blocking, 5 = low/cosmetic.';

-- Cross-table rule: resolved_at may only be set when status_code is a
-- terminal aircraft_ref.curation_flag_statuses row. Postgres CHECK
-- constraints cannot reference another table, so this is enforced here.
CREATE OR REPLACE FUNCTION aircraft_prov.enforce_curation_flag_resolution()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.resolved_at IS NOT NULL THEN
        IF NOT EXISTS (
            SELECT 1
            FROM aircraft_ref.curation_flag_statuses
            WHERE code = NEW.status_code
              AND is_terminal
        ) THEN
            RAISE EXCEPTION
                'aircraft_prov.curation_flags.resolved_at may only be set when status_code (%) is terminal',
                NEW.status_code;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION aircraft_prov.enforce_curation_flag_resolution() IS
    'BEFORE INSERT/UPDATE trigger function for aircraft_prov.curation_flags: '
    'ensures resolved_at is only set when status_code references a terminal '
    'row in aircraft_ref.curation_flag_statuses (RESOLVED, DISMISSED). '
    'Replaces the invalid chk_cf_resolution CHECK-with-subquery.';

-- =============================================================================
-- aircraft_prov.audit_log
-- Immutable change history for canonical data. Append-only (no UPDATE/DELETE).
-- Every change to a canonical field that is accepted via source_assertions
-- or direct curator edit should generate an audit_log row.
-- old_value / new_value: serialised to TEXT for cross-table generality.
-- change_source: distinguishes automated ingestion from human curation.
-- =============================================================================

CREATE TABLE aircraft_prov.audit_log (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    entity_type_code     aircraft_ref.lookup_code
                             REFERENCES aircraft_ref.curation_entity_types(code),
    entity_id            BIGINT NOT NULL,
    field_name           TEXT NOT NULL,
    old_value            TEXT,             -- previous value serialised as text
    new_value            TEXT,             -- new value serialised as text
    -- 'INGESTION'  = automated Phase 17 pipeline
    -- 'CURATOR'    = human curator action
    -- 'AUTOMATED'  = system reconciliation or derived update
    -- 'BULK_IMPORT'= batch import operation
    change_source        TEXT NOT NULL,
    changed_by           TEXT,             -- curator username, pipeline name, or service account
    change_reason        TEXT,
    source_assertion_id  BIGINT
                             REFERENCES aircraft_prov.source_assertions(id) ON DELETE SET NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_al_change_source CHECK (
        change_source IN ('INGESTION', 'CURATOR', 'AUTOMATED', 'BULK_IMPORT')
    )
);

COMMENT ON TABLE aircraft_prov.audit_log IS
    'Immutable change history. Append-only: no UPDATE or DELETE. '
    'One row per field change to any canonical entity. '
    'source_assertion_id links to the assertion that drove this change (if any). '
    'change_source TEXT CHECK (4 values) distinguishes automated pipeline changes '
    'from human curator edits. '
    'Enables: "show me every change to this variant''s MTOW over the past year", '
    '"who last changed this cruise speed, and from which source?"';

-- =============================================================================
-- TRIGGERS
-- =============================================================================

CREATE TRIGGER trg_curation_flags_updated
    BEFORE UPDATE ON aircraft_prov.curation_flags
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TRIGGER trg_cf_enforce_resolution
    BEFORE INSERT OR UPDATE ON aircraft_prov.curation_flags
    FOR EACH ROW EXECUTE FUNCTION aircraft_prov.enforce_curation_flag_resolution();

-- =============================================================================
-- INDEXES
-- =============================================================================

-- ── aircraft_prov.sources ─────────────────────────────────────────────────────

CREATE INDEX idx_src_type
    ON aircraft_prov.sources (source_type_code);
CREATE INDEX idx_src_reliability
    ON aircraft_prov.sources (reliability_grade_code);
CREATE INDEX idx_src_active
    ON aircraft_prov.sources (is_active)
    WHERE is_active;

-- ── aircraft_prov.source_documents ────────────────────────────────────────────

-- Ingestion dedup: one document per (source, source_system_key).
CREATE UNIQUE INDEX uq_sd_source_key
    ON aircraft_prov.source_documents (source_id, source_system_key)
    WHERE source_system_key IS NOT NULL;

-- All documents from one source.
CREATE INDEX idx_sdo_source
    ON aircraft_prov.source_documents (source_id, retrieved_at DESC);

-- Documents linked to one variant (for provenance drill-down).
CREATE INDEX idx_sdo_variant
    ON aircraft_prov.source_documents (variant_id)
    WHERE variant_id IS NOT NULL;

-- Documents needing reprocessing.
CREATE INDEX idx_sdo_status
    ON aircraft_prov.source_documents (processing_status, created_at)
    WHERE processing_status IN ('PENDING', 'FAILED', 'REPROCESS');

-- Batch label grouping (Phase 17 ingestion run queries).
CREATE INDEX idx_sdo_batch
    ON aircraft_prov.source_documents (ingest_batch_label, source_id)
    WHERE ingest_batch_label IS NOT NULL;

-- Full raw_json search.
CREATE INDEX idx_sdo_raw_json
    ON aircraft_prov.source_documents USING gin (raw_json jsonb_path_ops)
    WHERE raw_json IS NOT NULL;

-- ── aircraft_prov.source_assertions ──────────────────────────────────────────

-- Core invariant: at most one accepted assertion per (entity, field).
CREATE UNIQUE INDEX uq_assertion_accepted
    ON aircraft_prov.source_assertions (entity_type_code, entity_id, field_name)
    WHERE is_accepted;

-- All assertions for one entity (detail provenance page).
CREATE INDEX idx_sa_entity
    ON aircraft_prov.source_assertions (entity_type_code, entity_id);

-- All assertions for one entity + field (conflict detection query).
CREATE INDEX idx_sa_entity_field
    ON aircraft_prov.source_assertions (entity_type_code, entity_id, field_name);

-- All assertions from one source document.
CREATE INDEX idx_sa_document
    ON aircraft_prov.source_assertions (source_document_id);

-- Filter by status (find all PENDING assertions needing review).
CREATE INDEX idx_sa_status
    ON aircraft_prov.source_assertions (status_code, created_at)
    WHERE status_code IN ('PENDING', 'CONFLICT');

-- ── aircraft_prov.curation_flags ─────────────────────────────────────────────

-- Entity-level flag lookup.
CREATE INDEX idx_cf_entity
    ON aircraft_prov.curation_flags (entity_type_code, entity_id);

-- Field-level flag lookup (more specific).
CREATE INDEX idx_cf_entity_field
    ON aircraft_prov.curation_flags (entity_type_code, entity_id, field_name)
    WHERE field_name IS NOT NULL;

-- Working set: open and in-progress flags by priority.
CREATE INDEX idx_cf_open_priority
    ON aircraft_prov.curation_flags (priority, created_at)
    WHERE status_code IN ('OPEN', 'UNDER_REVIEW', 'DEFERRED');

-- ── aircraft_prov.audit_log ───────────────────────────────────────────────────

-- Entity change history (primary audit query: show all changes to this entity).
CREATE INDEX idx_al_entity
    ON aircraft_prov.audit_log (entity_type_code, entity_id, created_at DESC);

-- Field-specific history.
CREATE INDEX idx_al_entity_field
    ON aircraft_prov.audit_log (entity_type_code, entity_id, field_name, created_at DESC);

-- Trace a change back to its source assertion.
CREATE INDEX idx_al_assertion
    ON aircraft_prov.audit_log (source_assertion_id)
    WHERE source_assertion_id IS NOT NULL;

-- =============================================================================
-- SEED DATA — aircraft_prov.sources (3 bootstrap rows)
-- Phase 17 ingestion requires a PlanePHD source row to exist.
-- =============================================================================

INSERT INTO aircraft_prov.sources
    (slug, name, source_type_code, reliability_grade_code,
     base_url, default_confidence, refresh_interval_days, notes)
VALUES
    ('planephd',
     'PlanePHD',
     'SCRAPED_WEB', 'UNVERIFIED',
     'https://planephd.com',
     0.20,   -- Scraped, user-maintained marketplace data; low baseline confidence.
     90,     -- Re-scrape quarterly.
     'Primary seed data source. Values require corroboration from authoritative '
     'sources (manufacturer specs, FAA TCDS) before acceptance.'),

    ('manual-editorial',
     'Manual Editorial Entry',
     'MANUAL_ENTRY', 'HIGH',
     NULL,
     0.80,
     NULL,   -- Manual entries are one-time; no scheduled refresh.
     'Direct curator entries, typically sourced from POH/AFM, TCDS, '
     'or manufacturer specifications cross-checked by editorial staff.'),

    ('faa-tcds',
     'FAA Type Certificate Data Sheet (TCDS)',
     'TYPE_CERTIFICATE', 'AUTHORITATIVE',
     'https://rgl.faa.gov/Regulatory_and_Guidance_Library/rgMakeModel.nsf',
     1.00,
     NULL,   -- Re-check when variants are recertified.
     'FAA official type certificate data; highest reliability. '
     'Covers certificated models under US jurisdiction only.')

ON CONFLICT (slug) DO NOTHING;

-- =============================================================================
-- DEFERRED FOREIGN KEY (resolves the Phase 9 → Phase 14 ordering dependency)
-- aircraft_power.variant_powerplants.source_document_id was created as a plain
-- BIGINT in Phase 9 because aircraft_prov.source_documents did not yet exist.
-- Now that source_documents is defined, attach the FK constraint.
-- =============================================================================

ALTER TABLE aircraft_power.variant_powerplants
    ADD CONSTRAINT fk_vp_source_document
        FOREIGN KEY (source_document_id)
            REFERENCES aircraft_prov.source_documents(id) ON DELETE SET NULL;

COMMIT;
