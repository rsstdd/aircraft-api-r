-- =============================================================================
-- File: database/migrations/026_source_license_terms.sql
-- Phase 26: Record what each source's data may be used for.
--
-- aircraft_prov.sources.license_notes has existed since migration 014 and has
-- been NULL on every row since. Provenance that does not state its own licence
-- cannot answer the only question a downstream consumer actually has, so the
-- terms are recorded next to the data they govern rather than in prose someone
-- has to go and find.
--
-- This is a migration and not a one-time UPDATE because migration 014 seeds the
-- source rows, so a fresh install would otherwise reproduce the NULL. It is not
-- a seed because aircraft_prov.sources is not seeded from database/seeds/.
--
-- Nothing in the ingestion path writes this column: promote_document
-- (crates/aircraft_db/src/repositories/ingestion_repository.rs) upserts the
-- source row with ON CONFLICT(slug) DO UPDATE SET base_url = EXCLUDED.base_url
-- and touches no other column, so a re-import cannot erase what is set here.
-- Change both together if that upsert ever grows a license_notes clause.
--
-- Checked against https://planephd.com/terms and https://planephd.com/robots.txt
-- on 2026-09-12. Both are the operator's current published position and may
-- change; this row records what was published, not a legal conclusion about it.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

UPDATE aircraft_prov.sources
   SET license_notes =
           'Terms of Service (https://planephd.com/terms, updated 2023-06-11): '
           'the data portion of the services remains the sole and exclusive '
           'property of Planephd LLC and may not be used except as expressly '
           'allowed in the current strategic vendor or client agreement in '
           'force. A recipient may not incorporate the data into another list '
           'or database, use it to compile, verify, edit, enhance, update or '
           'publish another information source, distribute any portion of it, '
           'or reproduce it in any format. The terms state that use is '
           'monitored by decoy entries and set liquidated damages of USD '
           '10,000 per violation or incident. '
           'robots.txt (retrieved 2026-09-12): User-agent * is allowed on the '
           'specification detail pages this catalogue reads, under '
           'Content-Signal search=yes, ai-train=no, use=reference -- an express '
           'reservation of rights under Article 4 of EU Directive 2019/790. '
           'Ten named agents including ClaudeBot, GPTBot, CCBot and '
           'Google-Extended are disallowed entirely. '
           'No vendor or client agreement is on file for this repository. '
           'Treat every PlanePHD-derived value as reference-only evidence and '
           'do not redistribute it.'
 WHERE slug = 'planephd';

-- A row count of zero would mean migration 014's seed did not run or the slug
-- changed, and silently recording nothing is the failure this guards against.
DO $license$
DECLARE
    recorded BIGINT;
BEGIN
    SELECT count(*) INTO recorded
      FROM aircraft_prov.sources
     WHERE slug = 'planephd' AND license_notes IS NOT NULL;
    IF recorded <> 1 THEN
        RAISE EXCEPTION
            'expected exactly one planephd source row carrying license_notes, found %',
            recorded;
    END IF;
END
$license$;

COMMIT;
