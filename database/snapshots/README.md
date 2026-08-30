# Ingestion snapshots

Each `*.sql` file here is one normalized business snapshot of what the
`aircraft-ingest` adapter writes. `cargo xtask snapshots` imports a fixture into a
disposable database, runs every query, and diffs the result against the committed
golden output in `golden/<fixture-stem>/<query-stem>.txt`.

This began as a parity gate against the legacy server-side SQL loader. That
loader has been retired, so the gate no longer validates the adapter against a
second implementation — it catches regressions. A golden file changing is a
review event: read the diff, decide whether the new output is correct, and only
then re-record it with `--update`.

Files execute in filename order and must each return a single deterministic,
fully ordered result set.

Rules for a snapshot query:

- **No generated identifiers.** Surrogate keys differ between two independently
  loaded databases. Join through to a natural key (`slug`, `metric_type_code`,
  `field_name`) instead of selecting `id` or `*_id`.
- **No timestamps.** `created_at`, `retrieved_at`, and friends record when the
  load ran, not what it produced.
- **No curation state.** `is_canonical`, `is_accepted`, `status_code`, and
  `confidence` stay out of the snapshots. Ingestion always writes them the same
  way (non-canonical and pending), so they carry no regression signal, and
  including them would couple these files to the curation tests.
- **Fully ordered.** `ORDER BY` every selected column, so row order cannot vary
  between runs.

Everything else — identity, measurements, propulsion, market data, provenance,
assertions, and flags — is in scope, per
`docs/architecture/rust_ingestion_adapter.md`.

## Fixtures

- `planephd_minimal.json` — one clean record, the happy path.
- `planephd_edge_cases.json` — three records across two manufacturers, covering
  sentinels, unknown units, unmapped fields and costs, malformed images and
  dimensions, an unsupported URL scheme, a non-integer count, a twin engine,
  and identical ceiling assertions on two distinct variants.

`just snapshots` runs both. Adding a fixture means recording its golden output:

```bash
cargo xtask snapshots --fixture tests/fixtures/<new>.json --update
```

The goldens encode conservative behavior that is easy to regress. In
`planephd_edge_cases`, the Piper has no `SPEED_CRUISE_BEST` row because its unit
(`MPH`) is unmapped, and the Bonanza has no `FUEL_BURN_CRUISE` row because its
value is a sentinel. If either row appears, the adapter has started writing a
measurement it cannot justify.
