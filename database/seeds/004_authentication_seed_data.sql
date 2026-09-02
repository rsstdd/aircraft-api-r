-- =============================================================================
-- File: database/seeds/004_authentication_seed_data.sql
-- Phase 25 — seed data for aircraft_auth.scopes.
--
-- Closed protected-policy vocabulary from the accepted HTTP v1 decision.
-- Public needs no scope. Rate tiers are operational and remain unseeded.
-- DO NOTHING makes this safe for the independently repeatable seed workflow.
-- =============================================================================

BEGIN;

INSERT INTO aircraft_auth.scopes (code, label, description, sort_order)
VALUES ('CATALOG_READ', 'Catalog read',
        'Read ordinary aircraft catalog data.', 10),
       ('MILITARY_READ', 'Military reference read',
        'Read military platform capabilities and public loadout metadata.', 20),
       ('CURATION_READ', 'Curation read',
        'Read pending source evidence, assertions, and curation flags.', 30),
       ('CURATION_WRITE', 'Curation write',
        'Accept or reject assertions and record curation decisions.', 40),
       ('ADMIN', 'Administration',
        'Credential lifecycle and other administrative operations.', 50)
ON CONFLICT (code) DO NOTHING;

COMMIT;
