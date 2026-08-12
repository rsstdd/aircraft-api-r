# Aircraft Encyclopedia — Data Dictionary

---

## How to Use This Document

Each namespace section opens with a **summary paragraph** explaining the purpose,
the entities it models, and its position in the dependency chain. Detailed
column tables are provided for the principal, high-use tables. This document is
not an exhaustive catalog of all 99 migration-defined tables: junction tables,
secondary detail tables, and some supporting entities are intentionally
summarized at namespace level. The migration SQL remains authoritative for the
complete schema. Column tables use the following fields:

| Column | The column name as it appears in DDL |
|---|---|
| Type | PostgreSQL type, including domain names where applicable |
| Null | `NOT NULL` or `nullable` |
| Default | Default value or `—` |
| FK | Referenced table.column, or `—` |
| Description | What the column stores; units; special values |

---

## Namespace Dependency Map

```
aircraft_ref   ──────────────────────────────────────────▶  referenced by all namespaces
aircraft_geo   ──────────────────────────────────────────▶  aircraft_org, aircraft_core
aircraft_org   ──────────────────────────────────────────▶  aircraft_core, aircraft_power
aircraft_core  ◀─────────────────────────────────────────   hub: all domain namespaces FK here
     │
     ├──▶ aircraft_cert
     ├──▶ aircraft_specs   (dims / weights / performance)
     ├──▶ aircraft_power
     ├──▶ aircraft_systems
     ├──▶ aircraft_military
     ├──▶ aircraft_market
     ├──▶ aircraft_maint
     └──▶ aircraft_compare

aircraft_prov  ──▶  polymorphic into every namespace (source assertions)
aircraft_ingest ──▶  transient staging; feeds aircraft_prov → canonical tables
aircraft_read   ──▶  views and materialized views over all of the above
```

---

## 1. `aircraft_ref` — Reference / Lookup Tables

**Purpose.** Stores every extensible enumeration used across the schema as a proper lookup table rather than a `TEXT CHECK` constraint. Adding a new value requires only an `INSERT` into the relevant table, not a schema migration. Also houses the five cross-cutting domains, the `to_canonical()` unit-conversion function, and the three utility functions (`set_updated_at`, `slugify`, `normalize_lookup_code`).

**Key design choices.** All lookup tables share the same structural pattern: a `code aircraft_ref.lookup_code PRIMARY KEY` column (uppercase snake-case, e.g. `RETRACTABLE_TRICYCLE`), a human-readable `label TEXT NOT NULL`, an optional `description`, and `sort_order / is_active` flags. The `code` column is always the FK target, never the surrogate integer `id`, so FK values are self-documenting in query results.

### Domains

| Domain | Base Type | Constraint | Used For |
|---|---|---|---|
| `nonneg_numeric` | `NUMERIC` | `VALUE >= 0` | All physical and monetary quantities |
| `confidence_score` | `NUMERIC(3,2)` | `BETWEEN 0 AND 1` | Provenance quality scores |
| `year_value` | `SMALLINT` | `BETWEEN 1900 AND 2100` | Production and service years |
| `slug_text` | `TEXT` | `~ '^[a-z0-9]+(-[a-z0-9]+)*$'` | URL-routing identifiers |
| `lookup_code` | `TEXT` | `~ '^[A-Z][A-Z0-9_]*$'` | Lookup table primary keys |

### `aircraft_ref.measurement_units`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | PK | Canonical unit code, e.g. `KIAS`, `LBS`, `GPH` |
| `label` | `text` | NOT NULL | &mdash; | &mdash; | Display label, e.g. `Knots Indicated Airspeed` |
| `symbol` | `text` | nullable | &mdash; | &mdash; |  |
| `unit_category_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.unit_categories(code) ON DELETE NO ACTION | Groups units by physical dimension |
| `canonical_unit_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.measurement_units(code) ON DELETE NO ACTION | The unit this converts *to*; NULL if this unit is already canonical |
| `canonical_factor` | `numeric(18,10)` | nullable | &mdash; | &mdash; | Multiply any raw source value in this unit by this factor to obtain the value expressed in canonical_unit_code. NULL for canonical units themselves. |
| `si_factor` | `numeric(18,10)` | nullable | &mdash; | &mdash; |  |
| `si_base_unit_symbol` | `text` | nullable | &mdash; | &mdash; |  |
| `source_string_patterns` | `text[]` | nullable | &mdash; | &mdash; | Case-insensitive strings from raw data that map to this unit (e.g., '{kias, kts, knots}' for the KNOTS row). Used by Phase 17. |
| `sort_order` | `smallint` | NOT NULL | `0` | &mdash; |  |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; | FALSE soft-deletes deprecated unit codes |

### `aircraft_ref.performance_metric_types`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | PK | e.g. `SPEED_CRUISE_BEST`, `RANGE_NORMAL`, `SPEED_VX` |
| `label` | `text` | NOT NULL | &mdash; | &mdash; | Display label |
| `description` | `text` | nullable | &mdash; | &mdash; | Definition and measurement standard |
| `canonical_unit_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.measurement_units(code) ON DELETE NO ACTION | The unit canonical values are stored in |
| `is_higher_better` | `boolean` | nullable | &mdash; | &mdash; | Drives comparison scoring direction. NULL = non-directional (e.g. stall speed context-dependent) |
| `is_speed` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_distance` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_rate` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `sort_order` | `smallint` | NOT NULL | `0` | &mdash; |  |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |

### `aircraft_ref.cost_item_types`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | PK | e.g. `FUEL`, `ANNUAL_INSPECTION` |
| `label` | `text` | NOT NULL | &mdash; | &mdash; | Display label |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `is_fixed` | `boolean` | NOT NULL | `false` | &mdash; | TRUE = annual fixed cost; FALSE = per-hour variable cost |
| `is_aggregate` | `boolean` | NOT NULL | `false` | &mdash; | TRUE for rows that represent a pre-computed sum of other line items. The three aggregate codes are: TOTAL_COST_ANNUAL, TOTAL_FIXED_COST, TOTAL_VARIABLE_COST. These are routed to cost_snapshot_totals during Phase 17 ingestion. chk_cli_no_aggregate on cost_line_items enforces the separation at write time. |
| `sort_order` | `smallint` | NOT NULL | `0` | &mdash; |  |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |

*All other `aircraft_ref` lookup tables follow the same `code / label / description / sort_order / is_active` pattern. See Phase 2 seed files for the complete list of 36 tables and their seeded values.*

---

## 2. `aircraft_geo` — Geography

**Purpose.** Provides the geographic reference layer: ISO country records (including historical states needed for Cold War and WWII aircraft), named regional groupings, and the M:N junction mapping countries to regions. Consumed primarily by `aircraft_org` (country of HQ) and `aircraft_core` (country of origin, operator countries).

### `aircraft_geo.countries`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `code` | `character varying(3)` | NOT NULL | &mdash; | PK | ISO 3166-1 alpha-3 country code (e.g. `USA`, `GBR`). Historical states use IOC/ICAO codes (e.g. `SUN` for USSR) |
| `alpha2` | `character varying(2)` | nullable | &mdash; | UNIQUE | ISO 3166-1 alpha-2 code where applicable |
| `name` | `text` | NOT NULL | &mdash; | &mdash; | English common name |
| `official_name` | `text` | nullable | &mdash; | &mdash; | Full official name |
| `continent` | `text` | nullable | &mdash; | &mdash; | Informational geographic continent grouping for quick regional faceting. Political and aviation-authority groupings are modelled in country_regions. |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; | FALSE for dissolved or historical states still needed for origin attribution of legacy aircraft (e.g., SUN = Soviet Union). |

### `aircraft_geo.regions`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | PK | e.g. `NATO`, `EASA_STATES`, `WESTERN_EUROPE` |
| `label` | `text` | NOT NULL | &mdash; | &mdash; | Display name |
| `description` | `text` | nullable | &mdash; | &mdash; | Membership criteria or scope note |
| `region_type` | `text` | NOT NULL | `'GEOGRAPHIC'::text` | &mdash; | GEOGRAPHIC = continent/sub-region; POLITICAL = union/alliance; AVIATION = regulatory jurisdiction; MILITARY = defence alliance. A TEXT CHECK is used here (not a lookup table) because these four categories are definitionally complete and non-extensible by design. |
| `sort_order` | `smallint` | NOT NULL | `0` | &mdash; |  |

### `aircraft_geo.country_regions`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `country_code` | `character varying(3)` | NOT NULL | &mdash; | aircraft_geo.countries(code) ON DELETE CASCADE; PK (composite) |  |
| `region_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_geo.regions(code) ON DELETE CASCADE; PK (composite) |  |
| `joined_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; | Year of accession; NULL = founding member or unknown |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |

---

## 3. `aircraft_org` — Organizations

**Purpose.** A unified organization table replacing the reference schema's narrow `manufacturers` table. Covers manufacturers, design bureaux, military operators, civil operators, certification authorities, and consortium bodies. The `org_relationships` table captures historical mergers, license agreements, and parent-subsidiary structures.

### `aircraft_org.organizations`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK | Surrogate key |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE | URL-routing slug derived from the organization name. Stable identifier for front-end links and API routes. |
| `name` | `text` | NOT NULL | &mdash; | &mdash; | Canonical English name |
| `common_name` | `text` | nullable | &mdash; | &mdash; |  |
| `name_aliases` | `text[]` | nullable | &mdash; | &mdash; | Array of alternative, historical, or local-language names. Used by Phase 17 ingestion to resolve variant source manufacturer strings to the canonical organization row. |
| `org_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.organization_types(code) ON DELETE NO ACTION | e.g. `MANUFACTURER`, `DESIGN_BUREAU`, `OPERATOR_MILITARY` |
| `country_code` | `character varying(3)` | nullable | &mdash; | aircraft_geo.countries(code) ON DELETE RESTRICT | Country of HQ; NULL for international bodies |
| `icao_mfr_code` | `character varying(4)` | nullable | &mdash; | &mdash; | ICAO aircraft manufacturer designator (2–4 uppercase alphanumeric). Unique among non-NULL values (enforced by partial index). |
| `iata_code` | `character varying(3)` | nullable | &mdash; | &mdash; |  |
| `founded_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `dissolved_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; | Year the organization ceased to exist as a legal entity. NULL = still operating. is_active may be FALSE for dormant entities that have not formally dissolved. |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |
| `website_url` | `text` | nullable | &mdash; | &mdash; |  |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; | JSONB escape valve for organization-specific data not covered by dedicated columns (e.g., stock ticker, government registry number, consortium membership details). |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; | Maintained by `set_updated_at()` trigger |

### `aircraft_org.org_relationships`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `from_org_id` | `bigint` | NOT NULL | &mdash; | aircraft_org.organizations(id) ON DELETE CASCADE | The originating / subordinate / predecessor organization in the relationship. |
| `to_org_id` | `bigint` | NOT NULL | &mdash; | aircraft_org.organizations(id) ON DELETE CASCADE | The target / parent / successor organization in the relationship. |
| `relationship_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.org_relationship_types(code) ON DELETE NO ACTION | e.g. `ACQUIRED`, `LICENSE_AGREEMENT`, `PARENT_SUBSIDIARY` |
| `started_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `ended_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; | NULL = relationship still active |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |

---

## 4. `aircraft_core` — Aircraft Identity

**Purpose.** The identity backbone of the entire database. The three-tier `families → models → variants` hierarchy establishes the atomic unit (`variants`) that all domain FK chains reference. Four junction tables attach aliases, roles, manufacturers, and operators to variants.

**Why three tiers&mdash;** A *family* is the broadest grouping under one manufacturer (e.g. all Cessna 172 variants). A *model* is a distinct product name within a family (e.g. 172S, 172R). A *variant* is a specific production configuration (e.g. 172S with Garmin G1000 Nxi). This matches how aviation databases, FAA type certificates, and buyer research actually work.

### `aircraft_core.families`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; | e.g. `Cessna 172` |
| `common_name` | `text` | nullable | &mdash; | &mdash; |  |
| `name_aliases` | `text[]` | nullable | &mdash; | &mdash; | Alternative, former, or local-language family names. GIN-indexed for alias resolution during Phase 17 ingestion (manufacturer string matching). |
| `manufacturer_org_id` | `bigint` | nullable | &mdash; | aircraft_org.organizations(id) ON DELETE SET NULL | Primary manufacturer; NULL for design bureaux |
| `country_of_origin_code` | `character varying(3)` | nullable | &mdash; | aircraft_geo.countries(code) ON DELETE RESTRICT |  |
| `first_flight_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `name_tsv` | `tsvector` | nullable | generated | &mdash; | Generated stored tsvector over name + common_name + name_aliases. Backed by a GIN index for full-text family search in Phase 16. |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_core.models`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `family_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.families(id) ON DELETE RESTRICT |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; | e.g. `172S` |
| `display_name` | `text` | nullable | &mdash; | &mdash; | Human-readable full model name for display (e.g., "Cessna 172S Skyhawk SP"). name holds the bare designation; display_name adds family prefix and popular name. |
| `name_aliases` | `text[]` | nullable | &mdash; | &mdash; |  |
| `series` | `text` | nullable | &mdash; | &mdash; |  |
| `generation` | `smallint` | nullable | &mdash; | &mdash; |  |
| `first_flight_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `certification_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_core.variants`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `model_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.models(id) ON DELETE RESTRICT |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; | e.g. `172S (G1000 Nxi)` |
| `popular_name` | `text` | nullable | &mdash; | &mdash; |  |
| `variant_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.variant_types(code) ON DELETE NO ACTION | e.g. `PRODUCTION`, `PROTOTYPE`, `EXPERIMENTAL` |
| `service_status_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.service_statuses(code) ON DELETE NO ACTION | e.g. `IN_SERVICE`, `RETIRED`, `PROTOTYPE` |
| `country_of_origin_code` | `character varying(3)` | nullable | &mdash; | aircraft_geo.countries(code) ON DELETE RESTRICT | May reference inactive historical states |
| `first_flight_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `certification_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `production_start_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `production_end_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; | NULL indicates the variant is still in production (or status is unknown). Use is_in_production for an explicit three-state flag. |
| `passenger_capacity` | `smallint` | nullable | &mdash; | &mdash; | Total occupants including pilots; denormalized for faceted search |
| `crew_count` | `smallint` | nullable | &mdash; | &mdash; |  |
| `landing_gear_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.landing_gear_types(code) ON DELETE NO ACTION | Denormalized for faceted search |
| `propulsion_category_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.propulsion_categories(code) ON DELETE NO ACTION | Denormalized for faceted search |
| `engine_count` | `smallint` | nullable | &mdash; | &mdash; | Denormalized; authoritative value in `aircraft_power.variant_powerplants` |
| `is_in_production` | `boolean` | nullable | &mdash; | &mdash; |  |
| `ingest_key` | `text` | nullable | &mdash; | &mdash; | Opaque ingestion deduplication key, e.g. "AERONCA::11AC Chief". Populated by Phase 17 ingestion to prevent duplicate variant rows. Not a semantic business key; superseded by aircraft_prov.source_documents once Phase 14 is populated. |
| `source_path` | `text` | nullable | &mdash; | &mdash; | URI path from the originating source system used during Phase 17 ingestion. Canonical source URL lives in aircraft_prov.source_documents.source_url. |
| `description` | `text` | nullable | &mdash; | &mdash; | Source description text |
| `description_tsv` | `tsvector` | nullable | generated | &mdash; | Full-text search vector; GIN-indexed |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; | Sparse source-specific metadata |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_core.variant_aliases`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `alias_type` | `text` | NOT NULL | &mdash; | UNIQUE (composite) | TEXT CHECK (6 values). Uses CHECK rather than lookup FK because the set is definitionally stable and non-extensible. |
| `alias` | `text` | NOT NULL | &mdash; | UNIQUE (composite) | The alias string e.g. `Fishbed-C`, `MiG-21F-13` |
| `country_code` | `character varying(3)` | nullable | &mdash; | aircraft_geo.countries(code) ON DELETE SET NULL | Country context for the alias |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |

---

## 5. `aircraft_cert` — Certification

**Purpose.** Stores formal type certificate records and their M:N links to variants (one TC can cover multiple models in a series), operating approvals as a per-approval fact table, a 1:1 pilot requirements extension, and public safety metrics.

### `aircraft_cert.type_certificates`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `tc_number` | `text` | NOT NULL | &mdash; | UNIQUE (composite) | TC identifier as published by the authority: FAA format "3A4"; EASA format "EASA.A.064"; Transport Canada format "A-82". Combined with authority_code for uniqueness. |
| `authority_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.certification_authorities(code) ON DELETE NO ACTION; UNIQUE (composite) | e.g. `FAA`, `EASA`, `CAA_UK` |
| `tc_holder_org_id` | `bigint` | nullable | &mdash; | aircraft_org.organizations(id) ON DELETE SET NULL |  |
| `airworthiness_category_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.airworthiness_categories(code) ON DELETE NO ACTION | e.g. `NORMAL`, `UTILITY`, `AEROBATIC` |
| `certification_basis` | `text` | nullable | &mdash; | &mdash; | Regulatory standard verbatim from the TCDS (e.g., "14 CFR Part 23, effective Feb 1 1965, Amendments 23-1 through 23-48"). Stored as free-text to preserve the authoritative wording. |
| `issued_date` | `date` | nullable | &mdash; | &mdash; |  |
| `amended_date` | `date` | nullable | &mdash; | &mdash; |  |
| `tcds_url` | `text` | nullable | &mdash; | &mdash; | URL to the official Type Certificate Data Sheet document. For FAA: https://rgl.faa.gov/Regulatory_and_Guidance_Library/rgMakeModel.nsf/... |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_cert.variant_operating_approvals`

Fact table — one row per `(variant, approval_type)`.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `approval_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.operating_approval_types(code) ON DELETE NO ACTION; UNIQUE (composite) | e.g. `IFR`, `FIKI`, `AEROBATIC`, `ETOPS` |
| `is_approved` | `boolean` | nullable | &mdash; | &mdash; | TRUE = explicitly approved; FALSE = explicitly not approved; NULL = status not established from available sources (curation required). |
| `conditions` | `text` | nullable | &mdash; | &mdash; | Conditions or limitations on the approval |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; | Source reliability for this approval assertion (aircraft_ref.confidence_score domain: 0.00–1.00). Populated by Phase 17 ingestion from source reliability grade; updated during curation. |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_cert.pilot_requirements`

1:1 extension — one row per variant.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE |  |
| `min_certificate_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.pilot_certificate_types(code) ON DELETE NO ACTION | Minimum required pilot certificate |
| `type_rating_required` | `boolean` | nullable | &mdash; | &mdash; |  |
| `type_rating_code` | `text` | nullable | &mdash; | &mdash; | ICAO aircraft type designator for the required type rating (e.g., "B738" = Boeing 737-800, "A320" = Airbus A320 family). NULL when type_rating_required = FALSE or = NULL. |
| `min_crew` | `smallint` | nullable | &mdash; | &mdash; | Minimum certificated flight crew for this variant: 1 = single-pilot approved; 2 = two-crew (PIC + SIC required); NULL = not established from available sources. |
| `requires_complex` | `boolean` | nullable | &mdash; | &mdash; |  |
| `requires_high_perf` | `boolean` | nullable | &mdash; | &mdash; |  |
| `requires_tailwheel` | `boolean` | nullable | &mdash; | &mdash; |  |
| `requires_instrument` | `boolean` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 6. `aircraft_specs` — Specifications

**Purpose.** Three metric-fact tables covering dimensions, weights, and performance, plus the CG envelope and payload-range curve tables. All follow the **raw value / raw unit / canonical value** triple pattern that preserves source fidelity while enabling unit-normalized comparison.

**The triple pattern.** Every measurement stores: `raw_value NUMERIC` (exactly as parsed from source), `raw_unit_code` (the source's unit), and `canonical_value NUMERIC` (converted to the canonical unit via `aircraft_ref.to_canonical()`). Queries always filter and sort on `canonical_value`; `raw_value` and `raw_unit_code` are for provenance and display.

### `aircraft_specs.performance_metrics`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `metric_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.performance_metric_types(code) ON DELETE NO ACTION |  |
| `raw_value` | `numeric` | nullable | &mdash; | &mdash; | NULL for sentinel values (`None KIAS`) |
| `raw_unit_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.measurement_units(code) ON DELETE NO ACTION | NULL for dimensionless metrics |
| `canonical_value` | `numeric` | nullable | &mdash; | &mdash; | Converted to canonical unit; NULL when raw_value is NULL |
| `configuration` | `text` | nullable | &mdash; | &mdash; | Descriptive aircraft configuration at time of measurement. Common values: 'CLEAN', 'FLAPS_APPROACH', 'LANDING_CONFIG', 'GEAR_DOWN'. For V-speeds, identifies the specific flap/gear state used in the test. |
| `condition_altitude_ft` | `numeric` | nullable | &mdash; | &mdash; |  |
| `condition_weight_lbs` | `numeric` | nullable | &mdash; | &mdash; | Test weight if specified |
| `condition_weight_label` | `text` | nullable | &mdash; | &mdash; | e.g. `MTOW`, `BOW` |
| `condition_isa_dev_c` | `numeric` | nullable | &mdash; | &mdash; |  |
| `condition_power_setting` | `text` | nullable | &mdash; | &mdash; | Engine power / thrust setting at test conditions. TEXT CHECK (10 stable values). NULL when power setting is not published by the source. |
| `condition_surface_type` | `text` | nullable | &mdash; | &mdash; | Runway or water surface type for takeoff/landing distance metrics. NULL for airborne metrics (speeds, ceilings, range). TEXT CHECK (6 stable surface type values). |
| `conditions_notes` | `text` | nullable | &mdash; | &mdash; | Free-text condition qualifiers from source |
| `is_canonical` | `boolean` | NOT NULL | `false` | &mdash; | TRUE = this row is the designated cross-fleet comparison value for this (variant, metric_type) pair. At most one TRUE per pair (enforced by uq_perf_canonical partial UNIQUE index). Phase 17 ingestion sets is_canonical = TRUE for the first value; curators resolve conflicts from additional sources. |
| `is_estimated` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `source_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_specs.weight_metrics`

Same triple-pattern structure as `performance_metrics`. Key difference: no `is_canonical` flag (one row per `(variant, metric_type, COALESCE(configuration,''))` enforced by functional UNIQUE index). Allows negative load factor values (`LOAD_FACTOR_NEG`).

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `metric_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.weight_metric_types(code) ON DELETE NO ACTION |  |
| `raw_value` | `numeric` | nullable | &mdash; | &mdash; |  |
| `raw_unit_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.measurement_units(code) ON DELETE NO ACTION | Source unit code. NULL for dimensionless metrics. Phase 17 sets canonical_value = raw_value when raw_unit_code IS NULL. |
| `canonical_value` | `numeric` | nullable | &mdash; | &mdash; | Value in the metric-type canonical unit. For mass: LBS. For fuel: US_GAL. For dimensionless metrics (LOAD_FACTOR_*, WING_LOADING): equals raw_value. Populated by Phase 17 ingestion. |
| `configuration` | `text` | nullable | &mdash; | &mdash; |  |
| `is_estimated` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `source_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_specs.dimension_metrics`

Same triple-pattern structure. Non-negative values enforced (`chk_dm_canonical_nonneg`, `chk_dm_raw_nonneg`). The `configuration` column (e.g. `WINGS_FOLDED`) enables multiple wingspan values per variant.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `metric_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.dimension_metric_types(code) ON DELETE NO ACTION |  |
| `raw_value` | `numeric` | nullable | &mdash; | &mdash; |  |
| `raw_unit_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.measurement_units(code) ON DELETE NO ACTION |  |
| `canonical_value` | `numeric` | nullable | &mdash; | &mdash; | Value in the canonical unit for this metric type (from aircraft_ref.dimension_metric_types.canonical_unit_code). For DIM_WINGSPAN: feet. For DIM_BAGGAGE_VOLUME: cubic feet. Computed by aircraft_ref.to_canonical(raw_value, raw_unit_code) during Phase 17 ingestion. NULL = not yet computed or not available. |
| `configuration` | `text` | nullable | &mdash; | &mdash; | Optional discriminator for configuration-specific measurements. NULL = standard/default. Common values: 'WINGS_FOLDED', 'GEAR_DOWN', 'WITH_TIP_TANKS'. Drives the functional UNIQUE index that allows multiple rows per metric type per variant when configurations differ. |
| `is_estimated` | `boolean` | NOT NULL | `false` | &mdash; | TRUE when canonical_value is approximate, extrapolated, or derived from related data rather than directly measured from a primary source. |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `source_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_specs.cg_envelopes`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `config_label` | `text` | nullable | &mdash; | &mdash; | e.g. `STANDARD`, `WITH_TIP_TANKS` |
| `datum_description` | `text` | nullable | &mdash; | &mdash; |  |
| `cg_unit` | `text` | NOT NULL | `'INCHES_AFT_DATUM'::text` | &mdash; | CG reference datum |
| `fwd_limit_points` | `jsonb` | nullable | &mdash; | &mdash; | Forward CG limit boundary. JSON array: [{"w":<lbs>,"cg":<value>}, ...]. Points sorted by ascending w. cg value in cg_unit. Validated by trigger: every element must have numeric w and cg keys, and points must be sorted ascending by w. |
| `aft_limit_points` | `jsonb` | nullable | &mdash; | &mdash; | Aft CG limit boundary. Same validated structure as fwd_limit_points. |
| `min_weight_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `max_weight_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `most_fwd_cg` | `numeric` | nullable | &mdash; | &mdash; |  |
| `most_aft_cg` | `numeric` | nullable | &mdash; | &mdash; |  |
| `is_primary` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 7. `aircraft_power` — Propulsion

**Purpose.** A complete propulsion stack: an engine specification catalog, a many-to-many powerplant junction (replacing the reference schema's single `engine_id` FK), propeller specs, rotor systems, APU specs, and STC conversion records.

### `aircraft_power.engine_variants`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `manufacturer_org_id` | `bigint` | nullable | &mdash; | aircraft_org.organizations(id) ON DELETE SET NULL | NULL for stub rows created during ingestion before org matching |
| `manufacturer_name_raw` | `text` | nullable | &mdash; | &mdash; | Raw source string; used for dedup before org matching |
| `model_designation` | `text` | NOT NULL | &mdash; | &mdash; | e.g. `O-320-D2J`, `AI-25TL` |
| `model_family` | `text` | nullable | &mdash; | &mdash; |  |
| `name_aliases` | `text[]` | nullable | &mdash; | &mdash; | Alternative designations; GIN-indexed |
| `propulsion_category_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.propulsion_categories(code) ON DELETE NO ACTION |  |
| `fuel_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.fuel_types(code) ON DELETE NO ACTION |  |
| `hp_rated` | `numeric` | nullable | &mdash; | &mdash; | Piston/turboprop rated power |
| `hp_takeoff` | `numeric` | nullable | &mdash; | &mdash; |  |
| `rated_thrust_n` | `numeric` | nullable | &mdash; | &mdash; | Raw thrust value in Newtons, exactly as published in the source. Preserved for source fidelity following the raw/canonical pattern. Canonical comparison value is thrust_lbf_dry (N × 0.224809). |
| `thrust_lbf_dry` | `numeric` | nullable | &mdash; | &mdash; | Dry (unaugmented) thrust in LBF: the canonical comparison unit for jets. Derived from rated_thrust_n × 0.224809 during Phase 17 ingestion. NULL for piston/turboprop engines (use hp_rated for those). |
| `thrust_lbf_wet` | `numeric` | nullable | &mdash; | &mdash; | Afterburner thrust; only non-NULL when `has_afterburner = TRUE` |
| `has_afterburner` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `has_fadec` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_turbocharged` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_supercharged` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_geared` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_fuel_injected` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `displacement_cubic_in` | `numeric` | nullable | &mdash; | &mdash; |  |
| `cylinder_count` | `smallint` | nullable | &mdash; | &mdash; |  |
| `engine_weight_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `specific_fuel_consumption` | `numeric` | nullable | &mdash; | &mdash; |  |
| `sfc_unit` | `text` | nullable | &mdash; | &mdash; |  |
| `tbo_hours` | `integer` | nullable | &mdash; | &mdash; | Manufacturer Time Between Overhaul in flight hours. Per FAR 91, TBO is a recommendation for Part 91 operations; Part 135 operators must comply. |
| `tbo_years` | `integer` | nullable | &mdash; | &mdash; | Calendar TBO limit in years. Populated from the PlanePHD engine.years_before_overhaul field. Many GA engines expire at whichever limit (hours or calendar) is reached first. |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_power.variant_powerplants`

M:N junction — multiple engine options per variant.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `engine_variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_power.engine_variants(id) ON DELETE RESTRICT; UNIQUE (composite) |  |
| `engine_count` | `smallint` | NOT NULL | `1` | &mdash; | Number of installed engines of this engine_variant type. For a twin with identical engines: engine_count = 2. For a tandem helicopter with different power sections: two rows, each engine_count = 1. |
| `is_standard` | `boolean` | NOT NULL | `false` | &mdash; | Factory standard fitment |
| `is_optional` | `boolean` | NOT NULL | `false` | &mdash; | Factory option (non-exclusive with `is_standard` for conversions) |
| `is_primary` | `boolean` | NOT NULL | `false` | &mdash; | The engine used for performance comparisons; partial UNIQUE ensures one per variant |
| `install_position` | `text` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `source_document_id` | `bigint` | nullable | &mdash; | aircraft_prov.source_documents(id) ON DELETE SET NULL | FK to aircraft_prov.source_documents. Records which source established this powerplant link. SET NULL on source document deletion. |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 8. `aircraft_systems` — Avionics and Equipment

**Purpose.** A named equipment catalog with variant-level links (individual items) and named avionics suite bundles. Enables both item-level filtering ("has ADS-B Out") and bundle-level filtering ("equipped with G1000 Nxi").

### `aircraft_systems.equipment_catalog`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; |  |
| `short_name` | `text` | nullable | &mdash; | &mdash; |  |
| `name_aliases` | `text[]` | nullable | &mdash; | &mdash; | Alternative names and ingestion-matching aliases for this equipment item (e.g., ["KAP 140","King KAP-140","Bendix KAP140"] for one autopilot model). GIN-indexed for WHERE name_aliases @> ARRAY['KAP140'] lookups. |
| `manufacturer_org_id` | `bigint` | nullable | &mdash; | aircraft_org.organizations(id) ON DELETE SET NULL |  |
| `manufacturer_name_raw` | `text` | nullable | &mdash; | &mdash; |  |
| `category_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.systems_categories(code) ON DELETE NO ACTION | e.g. `AVIONICS_NAV`, `SAFETY_SYSTEM`, `ICE_PROTECTION` |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_systems.variant_equipment`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `equipment_id` | `bigint` | NOT NULL | &mdash; | aircraft_systems.equipment_catalog(id) ON DELETE RESTRICT; UNIQUE (composite) |  |
| `provision_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.equipment_provision_types(code) ON DELETE NO ACTION | e.g. `STANDARD`, `OPTIONAL`, `FIELD_RETROFIT` |
| `stc_number` | `text` | nullable | &mdash; | &mdash; | STC approval number for retrofit or STC-based equipment installations. e.g., "SA02386NY" for a specific ADS-B upgrade STC. NULL for STANDARD and OPTIONAL_FACTORY provision types. |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 9. `aircraft_military` — Military Reference Data

**Purpose.** Public, unclassified encyclopedia-only reference data: structural hardpoint capability, store-type compatibility, a weapons/stores catalog with mandatory source citations, representative loadout examples, mission capability assertions, and military sensor links. **No classified data. No operational, tactical, or employment guidance.**

### `aircraft_military.hardpoints`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `station_number` | `text` | NOT NULL | &mdash; | UNIQUE (composite) |  |
| `station_label` | `text` | nullable | &mdash; | &mdash; | e.g. `Station 3`, `Wing Root Port` |
| `position_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.hardpoint_position_types(code) ON DELETE NO ACTION | e.g. `WING_INBOARD`, `FUSELAGE_CENTERLINE` |
| `max_load_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `ejector_capacity_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `is_wet` | `boolean` | NOT NULL | `false` | &mdash; | TRUE when the station has fuel system plumbing enabling external fuel tanks. Dry stations can carry weapons and sensor pods but not fuel tanks. |
| `is_internal_bay` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |

### `aircraft_military.weapons_catalog`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; |  |
| `common_name` | `text` | nullable | &mdash; | &mdash; |  |
| `designation` | `text` | nullable | &mdash; | &mdash; | Official designation, e.g. `AIM-9X Sidewinder` |
| `stores_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.stores_types(code) ON DELETE NO ACTION |  |
| `manufacturer_org_id` | `bigint` | nullable | &mdash; | aircraft_org.organizations(id) ON DELETE SET NULL |  |
| `manufacturer_name_raw` | `text` | nullable | &mdash; | &mdash; |  |
| `country_of_origin_code` | `character varying(3)` | nullable | &mdash; | aircraft_geo.countries(code) ON DELETE NO ACTION |  |
| `weight_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `length_in` | `numeric` | nullable | &mdash; | &mdash; |  |
| `diameter_in` | `numeric` | nullable | &mdash; | &mdash; |  |
| `description` | `text` | nullable | &mdash; | &mdash; | Brief public description of the store. Must not contain classified performance data, targeting parameters, or employment guidance. |
| `reference_url` | `text` | nullable | &mdash; | &mdash; |  |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_military.representative_loadouts`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; | e.g. `Standard Air Defence` |
| `mission_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.military_mission_types(code) ON DELETE NO ACTION |  |
| `total_stores_weight_lbs` | `numeric` | nullable | &mdash; | &mdash; |  |
| `fuel_state_pct` | `numeric` | nullable | &mdash; | &mdash; |  |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `source_notes` | `text` | nullable | &mdash; | &mdash; | REQUIRED citation: public source for this loadout example (e.g., "Jane's All the World's Aircraft 2023 p.412", "Lockheed Martin F-35 fact sheet rev 2022", "USAF photo caption 2019"). |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 10. `aircraft_market` — Market Data

**Purpose.** Two separated concerns: `valuations` (market price time-series) and
`cost_snapshots` + `cost_line_items` + `cost_snapshot_totals` (ownership cost
estimates). Component line items and source-provided totals are stored separately
to prevent aggregate source values from being counted again as components.

### `aircraft_market.valuations`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `snapshot_date` | `date` | NOT NULL | `CURRENT_DATE` | &mdash; |  |
| `source_name` | `text` | nullable | &mdash; | &mdash; | Source system name; dedup uses functional UNIQUE on `(variant_id, snapshot_date, COALESCE(source_name,''))` |
| `source_url` | `text` | nullable | &mdash; | &mdash; |  |
| `papi_price_estimate` | `numeric(14,2)` | nullable | &mdash; | &mdash; | Source system's estimated typical market value. Column name retained for PlanePHD ETL compatibility (PAPI = Price Aircraft Price Index). Treat as "estimated_market_value" when sourced from non-PlanePHD systems. |
| `listing_price_low` | `numeric(14,2)` | nullable | &mdash; | &mdash; |  |
| `listing_price_high` | `numeric(14,2)` | nullable | &mdash; | &mdash; |  |
| `listing_price_median` | `numeric(14,2)` | nullable | &mdash; | &mdash; |  |
| `for_sale_count` | `integer` | nullable | &mdash; | &mdash; | Number of active listings observed at snapshot_date. NULL = not reported by source. Used for market liquidity comparison: few listings = illiquid market. |
| `currency_code` | `character varying(3)` | NOT NULL | `'USD'::character varying` | aircraft_ref.currencies(code) ON DELETE NO ACTION |  |
| `region_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_geo.regions(code) ON DELETE NO ACTION |  |
| `condition_grade_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.aircraft_condition_grades(code) ON DELETE NO ACTION |  |
| `assumed_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `assumed_airframe_hours` | `integer` | nullable | &mdash; | &mdash; |  |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `captured_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_market.cost_snapshots`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE |  |
| `snapshot_date` | `date` | NOT NULL | `CURRENT_DATE` | &mdash; |  |
| `source_name` | `text` | nullable | &mdash; | &mdash; |  |
| `condition_grade_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.aircraft_condition_grades(code) ON DELETE NO ACTION |  |
| `assumed_airframe_hours` | `integer` | nullable | &mdash; | &mdash; |  |
| `assumed_engine_hours_smoh` | `integer` | nullable | &mdash; | &mdash; |  |
| `assumed_prop_hours_smoh` | `integer` | nullable | &mdash; | &mdash; |  |
| `assumed_annual_hours` | `numeric` | nullable | &mdash; | &mdash; | Annual flight hours assumed for this owner profile. Critical: it converts per-hour variable costs to annual equivalents. Example: 100 hr/year ≈ private owner; 400 hr/year ≈ charter. |
| `assumed_fuel_price_per_gal` | `numeric` | nullable | &mdash; | &mdash; | Fuel price assumption; embedded in PlanePHD dynamic cost keys |
| `assumed_fuel_burn_gph` | `numeric` | nullable | &mdash; | &mdash; | Assumed fuel consumption rate in US GPH for cost calculation. May differ from aircraft_specs.performance_metrics (FUEL_BURN_CRUISE) if the source used a different cruise power setting in their cost model. Preserve source value; cross-reference against spec data in curation. |
| `currency_code` | `character varying(3)` | NOT NULL | `'USD'::character varying` | aircraft_ref.currencies(code) ON DELETE NO ACTION |  |
| `region_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_geo.regions(code) ON DELETE NO ACTION |  |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `extra_attributes` | `jsonb` | NOT NULL | `'{}'::jsonb` | &mdash; | Unmapped and dynamic cost keys stored verbatim; populated from Phase 17 promotion |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_market.cost_line_items`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `snapshot_id` | `bigint` | NOT NULL | &mdash; | aircraft_market.cost_snapshots(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `cost_item_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.cost_item_types(code) ON DELETE NO ACTION; UNIQUE (composite) |  |
| `amount_annual` | `numeric(12,2)` | nullable | &mdash; | &mdash; | Annual cost in snapshot currency |
| `amount_per_hour` | `numeric(10,4)` | nullable | &mdash; | &mdash; | Per-flight-hour operating cost in currency_code. NUMERIC(10,4): four decimal places for low-cost fractional hourly items (e.g., oil consumption $0.0083/hr). Phase 17 ingestion maps PlanePHD ownership_costs keys to these rows via aircraft_ref.cost_item_types lookup. |
| `currency_code` | `character varying(3)` | NOT NULL | `'USD'::character varying` | aircraft_ref.currencies(code) ON DELETE NO ACTION |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |

### `aircraft_market.cost_snapshot_totals`

Source-provided aggregate totals are deliberately separate from component line
items. There is at most one totals row per snapshot.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `snapshot_id` | `bigint` | NOT NULL | &mdash; | aircraft_market.cost_snapshots(id) ON DELETE CASCADE; UNIQUE | Owning cost snapshot |
| `total_annual_usd` | `numeric(14,2)` | nullable | &mdash; | &mdash; | Source-provided total annual ownership cost |
| `total_fixed_usd` | `numeric(14,2)` | nullable | &mdash; | &mdash; | Source-provided annual fixed-cost total |
| `total_variable_usd` | `numeric(14,2)` | nullable | &mdash; | &mdash; | Source-provided variable-cost total |
| `assumed_hours` | `numeric` | nullable | &mdash; | positive when present | Flight-hours assumption used by the source |
| `source_currency` | `character varying(3)` | NOT NULL | `'USD'::character varying` | aircraft_ref.currencies(code) ON DELETE NO ACTION | Currency of the source totals |
| `captured_from_key` | `text` | nullable | &mdash; | &mdash; | Raw ownership-cost key from which the total was captured |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

**Aggregation note.** Query the supported ownership-cost read model rather than
summing component and aggregate records together:

```sql
SELECT source_total_annual_usd,
       source_total_fixed_usd,
       source_total_variable_usd,
       computed_total_annual_usd,
       cost_per_hour_usd
FROM aircraft_read.mv_ownership_cost_summary
WHERE snapshot_id = :snapshot_id;
```

The trigger `aircraft_market.reject_aggregate_line_item()` rejects aggregate
cost types in `cost_line_items`; those values belong in
`cost_snapshot_totals`. Migration 016 currently contains a known fuel-code and
mixed-variable aggregation defect, documented in `implementation_notes.md`.
Until it is fixed, do not rely on the hourly fuel/maintenance split or the
computed annual total for snapshots containing mixed annual and hourly variable
items.

---

## 11. `aircraft_maint` — Maintenance and Reliability

**Purpose.** Airworthiness directives (ADs), service bulletins (SBs), life-limited parts, and a 1:1 editorial support assessment. ADs and SBs use M:N junctions (one AD can affect many variants; one variant can be subject to many ADs).

### `aircraft_maint.airworthiness_directives`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `ad_number` | `text` | NOT NULL | &mdash; | UNIQUE (composite) | Official AD number, e.g. `2023-14-08` |
| `authority_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.certification_authorities(code) ON DELETE NO ACTION; UNIQUE (composite) |  |
| `ad_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.ad_types(code) ON DELETE NO ACTION | One of: `RECURRING`, `ONE_TIME`, `ON_CONDITION`, `AIRWORTHINESS_LIMITATION` |
| `subject` | `text` | NOT NULL | &mdash; | &mdash; | Brief subject description |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `effective_date` | `date` | nullable | &mdash; | &mdash; |  |
| `compliance_interval_hours` | `numeric` | nullable | &mdash; | &mdash; | Repeat inspection interval in flight hours (RECURRING ADs). NULL for ONE_TIME ADs where compliance is a single event. |
| `compliance_interval_months` | `smallint` | nullable | &mdash; | &mdash; |  |
| `initial_compliance_date` | `date` | nullable | &mdash; | &mdash; |  |
| `superseded_by_ad_number` | `text` | nullable | &mdash; | &mdash; | AD number of the replacement (newer revision). Free text since the replacement may not yet be in the database. |
| `reference_url` | `text` | nullable | &mdash; | &mdash; | Link to official AD text |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_maint.support_assessments`

1:1 editorial assessment per variant.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE |  |
| `snapshot_date` | `date` | NOT NULL | `CURRENT_DATE` | &mdash; |  |
| `fleet_size_estimate` | `integer` | nullable | &mdash; | &mdash; | Approximate number of this variant in active service worldwide. Source typically FAA Registry (US), GAMA statistics, or Jane's census. Larger fleets generally correlate with better parts and MRO availability. |
| `fleet_size_source` | `text` | nullable | &mdash; | &mdash; |  |
| `fleet_size_year` | `aircraft_ref.year_value` | nullable | &mdash; | &mdash; |  |
| `oem_support_status` | `text` | nullable | &mdash; | &mdash; | One of: `FULL`, `LIMITED`, `THIRD_PARTY_ONLY`, `NONE` |
| `parts_availability_grade_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.availability_grades(code) ON DELETE NO ACTION |  |
| `maintenance_network_grade_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.availability_grades(code) ON DELETE NO ACTION |  |
| `corrosion_risk_level` | `text` | nullable | &mdash; | &mdash; | One of: `LOW`, `MODERATE`, `HIGH`, `VERY_HIGH` |
| `common_issues_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `mod_stc_ecosystem_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `owner_community_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `dispatch_reliability_pct` | `numeric(5,2)` | nullable | &mdash; | &mdash; | Percentage of scheduled flights completed without an AOG (Aircraft on Ground) technical delay. Only available for commercial-operation types with public statistical reporting. NULL for most GA types. Must cite dispatch_reliability_source. |
| `dispatch_reliability_source` | `text` | nullable | &mdash; | &mdash; |  |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 12. `aircraft_prov` — Provenance and Curation

**Purpose.** The provenance backbone. Every field value in canonical tables should have at least one `source_assertions` row linking it to its originating `source_documents` record. The `is_accepted` flag marks the current winning value per `(entity_type, entity_id, field_name)`, enabling multi-source conflict resolution without overwriting previous values.

### `aircraft_prov.sources`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `name` | `text` | NOT NULL | &mdash; | &mdash; |  |
| `source_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.source_types(code) ON DELETE NO ACTION | e.g. `OFFICIAL_TC`, `MARKETPLACE_DB`, `MANUFACTURER_SPEC` |
| `reliability_grade_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.source_reliability_grades(code) ON DELETE NO ACTION | e.g. `AUTHORITATIVE`, `VERIFIED`, `UNVERIFIED` |
| `base_url` | `text` | nullable | &mdash; | &mdash; |  |
| `license_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `default_confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; | Baseline confidence for assertions from this source (0.00–1.00). Derived from reliability_grade_code.numeric_score / 5 at creation. Curators may override per-assertion in source_assertions.confidence. |
| `refresh_interval_days` | `smallint` | nullable | &mdash; | &mdash; |  |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_prov.source_assertions`

The core provenance fact table.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `source_document_id` | `bigint` | NOT NULL | &mdash; | aircraft_prov.source_documents(id) ON DELETE CASCADE |  |
| `entity_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.curation_entity_types(code) ON DELETE NO ACTION | e.g. `AIRCRAFT_VARIANT`, `ENGINE_SPEC`, `COST_SNAPSHOT` |
| `entity_id` | `bigint` | NOT NULL | &mdash; | &mdash; | PK value in the target table (polymorphic; no DDL-enforced FK) |
| `field_name` | `text` | NOT NULL | &mdash; | &mdash; | PostgreSQL column name of the asserted field on the entity table, or a dotted path for JSONB sub-fields (e.g., "canonical_value", "extra_attributes.engine_category_hint"). Standardised to the column name for direct mapping in curation tooling. |
| `raw_value` | `text` | nullable | &mdash; | &mdash; |  |
| `raw_unit` | `text` | nullable | &mdash; | &mdash; |  |
| `asserted_value` | `text` | nullable | &mdash; | &mdash; | Value exactly as it appeared in the source (including units) |
| `asserted_numeric` | `numeric` | nullable | &mdash; | &mdash; |  |
| `status_code` | `aircraft_ref.lookup_code` | NOT NULL | `'PENDING'::text` | aircraft_ref.assertion_statuses(code) ON DELETE NO ACTION | `PENDING`, `ACCEPTED`, `REJECTED`, `SUPERSEDED` |
| `is_accepted` | `boolean` | NOT NULL | `false` | &mdash; | TRUE = this assertion is the designated canonical value for this field. At most one TRUE per (entity_type_code, entity_id, field_name), enforced by the partial UNIQUE index uq_assertion_accepted. Phase 17 ingestion sets is_accepted = TRUE for the first assertion per field; subsequent conflicting assertions arrive as PENDING and require curator review. |
| `confidence` | `aircraft_ref.confidence_score` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_prov.curation_flags`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `entity_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.curation_entity_types(code) ON DELETE NO ACTION |  |
| `entity_id` | `bigint` | NOT NULL | &mdash; | &mdash; | Polymorphic FK |
| `field_name` | `text` | nullable | &mdash; | &mdash; |  |
| `issue_type` | `text` | NOT NULL | &mdash; | &mdash; | Free-text issue classification. Common values: 'MISSING_VALUE', 'CONFLICTING_SOURCES', 'IMPLAUSIBLE_VALUE', 'PARSE_FAILURE', 'DESCRIPTION_PARSE_INCOMPLETE', 'STALE_DATA'. Not FK-constrained: extensibility over strictness. |
| `issue_description` | `text` | nullable | &mdash; | &mdash; |  |
| `status_code` | `aircraft_ref.lookup_code` | NOT NULL | `'OPEN'::text` | aircraft_ref.curation_flag_statuses(code) ON DELETE NO ACTION | `OPEN`, `UNDER_REVIEW`, `RESOLVED`, `DISMISSED` |
| `priority` | `smallint` | NOT NULL | `3` | &mdash; |  |
| `assigned_to` | `text` | nullable | &mdash; | &mdash; |  |
| `resolved_by` | `text` | nullable | &mdash; | &mdash; |  |
| `resolved_at` | `timestamp with time zone` | nullable | &mdash; | &mdash; | Only non-NULL when `status_code` is terminal (`RESOLVED` or `DISMISSED`) |
| `resolution_notes` | `text` | nullable | &mdash; | &mdash; |  |
| `source_assertion_id` | `bigint` | nullable | &mdash; | aircraft_prov.source_assertions(id) ON DELETE SET NULL |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |
| `updated_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_prov.audit_log`

Append-only change history by convention. The schema documents INSERT-only
intent, but currently has no GRANT policy, RLS policy, rule, or trigger that
prevents UPDATE or DELETE. See the deferred decision in
`implementation_notes.md`.

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `entity_type_code` | `aircraft_ref.lookup_code` | nullable | &mdash; | aircraft_ref.curation_entity_types(code) ON DELETE NO ACTION |  |
| `entity_id` | `bigint` | NOT NULL | &mdash; | &mdash; |  |
| `field_name` | `text` | NOT NULL | &mdash; | &mdash; |  |
| `old_value` | `text` | nullable | &mdash; | &mdash; | Previous state snapshot |
| `new_value` | `text` | nullable | &mdash; | &mdash; | New state snapshot |
| `change_source` | `text` | NOT NULL | &mdash; | &mdash; |  |
| `changed_by` | `text` | nullable | &mdash; | &mdash; | Application user or service account identifier |
| `change_reason` | `text` | nullable | &mdash; | &mdash; |  |
| `source_assertion_id` | `bigint` | nullable | &mdash; | aircraft_prov.source_assertions(id) ON DELETE SET NULL |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

---

## 13. `aircraft_compare` — Mission Profiles and Scoring

**Purpose.** The comparison engine: weighted mission profiles, per-criterion score storage, and aggregate suitability scores. Scores are pre-computed by a scoring job that runs after each significant data update and stored in `variant_suitability` / `criterion_scores`. The `aircraft_read.mv_variant_search` matview joins to `variant_suitability` for ranked search results.

### `aircraft_compare.mission_profiles`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `profile_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.mission_profile_types(code) ON DELETE NO ACTION; UNIQUE | One profile per type; prevents duplicate active profiles |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | &mdash; | UNIQUE |  |
| `title` | `text` | NOT NULL | &mdash; | &mdash; | e.g. `IFR Cross-Country` |
| `description` | `text` | nullable | &mdash; | &mdash; |  |
| `typical_range_nm` | `numeric` | nullable | &mdash; | &mdash; |  |
| `typical_pax_count` | `smallint` | nullable | &mdash; | &mdash; |  |
| `typical_altitude_ft` | `numeric` | nullable | &mdash; | &mdash; |  |
| `applies_to_civilian` | `boolean` | NOT NULL | `true` | &mdash; |  |
| `applies_to_military` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `is_active` | `boolean` | NOT NULL | `true` | &mdash; |  |
| `sort_order` | `smallint` | NOT NULL | `0` | &mdash; |  |
| `created_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; |  |

### `aircraft_compare.mission_criteria`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `mission_profile_id` | `bigint` | NOT NULL | &mdash; | aircraft_compare.mission_profiles(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `criterion_type_code` | `aircraft_ref.lookup_code` | NOT NULL | &mdash; | aircraft_ref.comparison_criterion_types(code) ON DELETE NO ACTION; UNIQUE (composite) |  |
| `weight` | `numeric(4,3)` | NOT NULL | &mdash; | &mdash; | Fractional weight; all weights for a profile must sum to 1.0000 (enforced by `v_weight_criteria_validation` view, not DDL) |
| `is_required` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `scoring_lower_bound` | `numeric` | nullable | &mdash; | &mdash; | For is_higher_better = TRUE: canonical values below this yield raw_score = 0.0. For is_higher_better = FALSE: canonical values above this yield raw_score = 0.0. NULL = no explicit lower bound (criterion scored without disqualification threshold). |
| `scoring_upper_bound` | `numeric` | nullable | &mdash; | &mdash; |  |
| `notes` | `text` | nullable | &mdash; | &mdash; |  |

### `aircraft_compare.variant_suitability`

| Column | Type | Null | Default | Constraint / FK | Description |
|---|---|---|---|---|---|
| `id` | `bigint` | NOT NULL | identity | PK |  |
| `variant_id` | `bigint` | NOT NULL | &mdash; | aircraft_core.variants(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `mission_profile_id` | `bigint` | NOT NULL | &mdash; | aircraft_compare.mission_profiles(id) ON DELETE CASCADE; UNIQUE (composite) |  |
| `overall_score` | `numeric(5,3)` | nullable | &mdash; | &mdash; | Weighted composite score in [0,1] |
| `is_disqualified` | `boolean` | NOT NULL | `false` | &mdash; |  |
| `disqualification_reason` | `text` | nullable | &mdash; | &mdash; |  |
| `scored_criteria_count` | `smallint` | nullable | &mdash; | &mdash; |  |
| `total_criteria_count` | `smallint` | nullable | &mdash; | &mdash; |  |
| `required_criteria_met` | `smallint` | nullable | &mdash; | &mdash; |  |
| `total_required_criteria` | `smallint` | nullable | &mdash; | &mdash; |  |
| `computed_at` | `timestamp with time zone` | NOT NULL | `now()` | &mdash; | Timestamp of last score computation |

---

## 14. `aircraft_ingest` — Staging (Transient)

**Purpose.** Transient ETL namespace. Three tables hold the intermediate state of the Phase 17 ingestion pipeline: `ingest_runs` (batch metadata), `staged_aircraft` (flattened JSON rows), and `staged_images` (image array rows). These tables are not part of the canonical data model and are safe to truncate after provenance data has been validated.

Key columns are documented inline in `901_seed_data_staging.sql` via
`COMMENT ON COLUMN` statements. The staging DDL and promotion behavior are
defined by `901_seed_data_staging.sql` and
`902_server_side_json_ingestion.sql`.

---

## 15. `aircraft_read` — Read Models

**Purpose.** Denormalized views and materialized views optimized for frontend query patterns. Application code should query this namespace, not the normalized source tables directly.

| Object | Type | Description |
|---|---|---|
| `v_current_valuation` | VIEW | `DISTINCT ON` latest valuation per variant; one row per variant |
| `v_hangar_fit` | VIEW | One row per variant, with nullable dimensions and the derived fields `fits_t_hangar_36ft`, `fits_t_hangar_40ft`, `fits_t_hangar_50ft`, `fits_40ft_with_fold`, and `clears_14ft_door` |
| `v_weight_criteria_validation` | VIEW | QA: checks mission_criteria weights sum to 1.0000 per profile |
| `mv_ownership_cost_summary` | MATVIEW | Aggregated annual + per-hour cost from latest snapshot; 17 columns |
| `mv_variant_search` | MATVIEW | 48-column denormalized search surface with 20 indexes total: three GIN/trigram indexes and 17 B-tree/partial indexes; **must be refreshed after data changes** |
| `refresh_search_matviews(concurrent BOOLEAN)` | FUNCTION | Refreshes `mv_ownership_cost_summary` then `mv_variant_search` in correct order. Pass `FALSE` for initial population (no `CONCURRENTLY`); default `TRUE` for live updates |

**Critical refresh note.** `mv_variant_search` is created `WITH NO DATA`. It returns zero rows until `refresh_search_matviews(FALSE)` is called. Subsequent incremental updates use `refresh_search_matviews()` (concurrent = TRUE by default), which does not block reads.
