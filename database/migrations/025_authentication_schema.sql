-- =============================================================================
-- File: database/migrations/025_authentication_schema.sql
-- Phase 25: Durable principals, API credentials, scopes, grants, and rate tiers.
--
-- Credential columns and digest shape are pinned by the validation companion.
-- key_id is a non-enumerable UUID lookup key; the clear secret is never stored.
-- Rate tiers carry identity only because quota values are operational settings.
-- =============================================================================

BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5min';

CREATE SCHEMA IF NOT EXISTS aircraft_auth;
COMMENT ON SCHEMA aircraft_auth IS
    'Principals, API credentials, scope grants, and rate-limit tiers for the '
    'HTTP boundary. No aircraft data or clear credential lives here.';

CREATE TABLE IF NOT EXISTS aircraft_auth.rate_limit_tiers (
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_rlt_label UNIQUE (label),
    CONSTRAINT chk_rlt_label CHECK (btrim(label) <> '')
);
COMMENT ON TABLE aircraft_auth.rate_limit_tiers IS
    'Named rate-limit tier a principal belongs to. Identity only: the capacity '
    'and refill rate behind a code are operational configuration, not schema, '
    'so a quota changes without a migration. No code is seeded.';

CREATE TRIGGER trg_rlt_updated
    BEFORE UPDATE ON aircraft_auth.rate_limit_tiers
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TABLE IF NOT EXISTS aircraft_auth.scopes (
    code        aircraft_ref.lookup_code PRIMARY KEY,
    label       TEXT NOT NULL,
    description TEXT,
    -- Match the lookup-table shape established by migration 002.
    -- squawk-ignore prefer-bigint-over-smallint
    sort_order  SMALLINT NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_scp_label UNIQUE (label),
    CONSTRAINT chk_scp_label CHECK (btrim(label) <> '')
);
COMMENT ON TABLE aircraft_auth.scopes IS
    'The closed authorization vocabulary of '
    'docs/architecture/http_v1_decisions.md: CATALOG_READ, MILITARY_READ, '
    'CURATION_READ, CURATION_WRITE, ADMIN. The Public policy requires no '
    'credential and therefore has no row. Seeded by '
    'database/seeds/004_authentication_seed_data.sql; a 403 names one of these '
    'codes, so a code is contract.';

CREATE TRIGGER trg_scp_updated
    BEFORE UPDATE ON aircraft_auth.scopes
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

CREATE TABLE IF NOT EXISTS aircraft_auth.principals (
    id                   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name                 TEXT NOT NULL,
    rate_limit_tier_code aircraft_ref.lookup_code NOT NULL,
    disabled_at          TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_prn_name UNIQUE (name),
    CONSTRAINT chk_prn_name CHECK (btrim(name) <> ''),
    CONSTRAINT chk_prn_disabled_at
        CHECK (disabled_at IS NULL OR disabled_at >= created_at),
    CONSTRAINT fk_prn_rate_limit_tier
        FOREIGN KEY (rate_limit_tier_code)
        REFERENCES aircraft_auth.rate_limit_tiers (code)
        ON DELETE RESTRICT
);
COMMENT ON TABLE aircraft_auth.principals IS
    'The authenticated identity a credential belongs to and a rate-limit bucket '
    'is keyed by. Disable principals instead of deleting credential history.';
COMMENT ON COLUMN aircraft_auth.principals.name IS
    'Stable, unique operator-facing identifier.';
COMMENT ON COLUMN aircraft_auth.principals.disabled_at IS
    'NULL means enabled; no companion boolean can drift from this state.';

CREATE TRIGGER trg_prn_updated
    BEFORE UPDATE ON aircraft_auth.principals
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- Keep the exceptional disabled set small and directly searchable.
CREATE INDEX IF NOT EXISTS idx_prn_disabled
    ON aircraft_auth.principals (disabled_at)
    WHERE disabled_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS aircraft_auth.api_credentials (
    key_id        UUID PRIMARY KEY,
    principal_id  BIGINT NOT NULL,
    secret_digest TEXT NOT NULL,
    label         TEXT NOT NULL,
    revoked_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_apc_secret_digest UNIQUE (secret_digest),
    CONSTRAINT chk_apc_secret_digest CHECK (secret_digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT chk_apc_label
        CHECK (btrim(label) <> '' AND char_length(label) <= 200),
    CONSTRAINT chk_apc_revoked_at
        CHECK (revoked_at IS NULL OR revoked_at >= created_at),
    CONSTRAINT fk_apc_principal
        FOREIGN KEY (principal_id)
        REFERENCES aircraft_auth.principals (id)
        ON DELETE RESTRICT
);
COMMENT ON TABLE aircraft_auth.api_credentials IS
    'One issued API credential. The clear credential is returned once at '
    'issuance and never stored. The validation companion pins this column list.';
COMMENT ON COLUMN aircraft_auth.api_credentials.key_id IS
    'Non-enumerable public lookup handle supplied by credential issuance.';
COMMENT ON COLUMN aircraft_auth.api_credentials.secret_digest IS
    'SHA-256 of the clear credential as 64 lowercase hexadecimal characters, '
    'for constant-time comparison by the future verifier. Never disclose it.';
COMMENT ON COLUMN aircraft_auth.api_credentials.label IS
    'Bounded non-secret operator note, for example ''ci-runner''.';
COMMENT ON COLUMN aircraft_auth.api_credentials.revoked_at IS
    'NULL means live; no companion boolean can drift from this state.';

CREATE TRIGGER trg_apc_updated
    BEFORE UPDATE ON aircraft_auth.api_credentials
    FOR EACH ROW EXECUTE FUNCTION aircraft_ref.set_updated_at();

-- Supports credential listing and the principal FK's RESTRICT check.
CREATE INDEX IF NOT EXISTS idx_apc_principal
    ON aircraft_auth.api_credentials (principal_id);

-- Keep live credentials, the common case, out of the revocation index.
CREATE INDEX IF NOT EXISTS idx_apc_revoked
    ON aircraft_auth.api_credentials (revoked_at)
    WHERE revoked_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS aircraft_auth.principal_scope_grants (
    principal_id BIGINT NOT NULL,
    scope_code   aircraft_ref.lookup_code NOT NULL,
    granted_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (principal_id, scope_code),
    CONSTRAINT fk_psg_principal
        FOREIGN KEY (principal_id)
        REFERENCES aircraft_auth.principals (id)
        ON DELETE CASCADE,
    CONSTRAINT fk_psg_scope
        FOREIGN KEY (scope_code)
        REFERENCES aircraft_auth.scopes (code)
        ON DELETE RESTRICT
);
COMMENT ON TABLE aircraft_auth.principal_scope_grants IS
    'Scopes held by a principal. Grants are inserted or deleted, never updated.';

-- Reverse lookup and the scope FK's RESTRICT check.
CREATE INDEX IF NOT EXISTS idx_psg_scope
    ON aircraft_auth.principal_scope_grants (scope_code);

COMMIT;
