-- =============================================================================
-- File: database/migrations/024_promote_existing_manufacturer_links.sql
-- Phase 24: Repair the manufacturer projection migration 023 could not reach.
--
-- Migration 023 inserts the variant-level manufacturer link with
-- ON CONFLICT (variant_id, org_id) DO NOTHING behind a "variant has no primary
-- link" predicate. A variant that already carried the correct link as
-- is_primary = FALSE -- an early curation pass, or a hand-built fixture --
-- satisfies that predicate, collides on the primary key, and is left exactly as
-- it was: still no primary manufacturer, and still NULL in mv_variant_search.
-- Its own validation companion then aborts the upgrade.
--
-- Promote those links instead. The predicate is the same one 023 used, so this
-- touches only variants with a Rust ingestion document whose family names an
-- unambiguous manufacturer, and only where no primary link stands -- which is
-- what keeps uq_variant_primary_mfr (migration 004) satisfied. role is left
-- alone: a curator may have set DESIGNER on the link, and is_primary is the
-- column the read model reads. A database where 023 had nothing to skip finds
-- nothing to promote here.
--
-- The matching runtime defect is fixed in
-- SqlxIngestionUnitOfWork::link_primary_manufacturer
-- (crates/aircraft_db/src/repositories/ingestion_repository.rs), which now
-- upserts to is_primary = TRUE for the same reason.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

UPDATE aircraft_core.variant_manufacturers AS link
SET is_primary = TRUE
FROM aircraft_core.variants AS variant
JOIN aircraft_core.models AS model ON model.id = variant.model_id
JOIN aircraft_core.families AS family ON family.id = model.family_id
WHERE link.variant_id = variant.id
  AND link.org_id = family.manufacturer_org_id
  AND NOT link.is_primary
  AND EXISTS (
      SELECT 1
      FROM aircraft_prov.source_documents AS document
      WHERE document.variant_id = variant.id
        AND document.ingest_run_id IS NOT NULL
  )
  AND NOT EXISTS (
      SELECT 1
      FROM aircraft_core.variant_manufacturers AS standing
      WHERE standing.variant_id = variant.id
        AND standing.is_primary
  );

SELECT aircraft_read.refresh_search_matviews(FALSE);

COMMIT;
