#!/usr/bin/env bash
# Squawk migration lint, shared by `just migrations-lint` and the CI workflow so
# the two cannot drift.
#
# Migrations 001-016 predate Squawk enforcement and are excluded in
# .squawk.toml, which `cargo xtask migrations` pins to exactly that range.
# Migrations 017-018 are checksum-locked and immutable, so their reviewed
# findings are held as file-scoped rule baselines rather than edited away. Every
# later migration is linted with the complete default ruleset.
set -euo pipefail

cd "$(dirname "$0")/.."

squawk=(npm exec --yes --package squawk-cli@2.51.0 -- squawk)

"${squawk[@]}" \
  --exclude=adding-foreign-key-constraint,prefer-bigint-over-int,prefer-bigint-over-smallint,require-concurrent-index-creation,require-concurrent-index-deletion,require-timeout-settings \
  database/migrations/017_rust_ingestion_adapter.sql

"${squawk[@]}" \
  --exclude=adding-foreign-key-constraint,constraint-missing-not-valid,require-concurrent-index-creation,require-timeout-settings \
  database/migrations/018_staged_aircraft_variant_fk.sql

current_migrations=()
for migration in database/migrations/*.sql; do
  filename="${migration##*/}"
  version="${filename%%_*}"
  if ((10#$version >= 19)); then
    current_migrations+=("$migration")
  fi
done

if ((${#current_migrations[@]} > 0)); then
  "${squawk[@]}" "${current_migrations[@]}"
fi
