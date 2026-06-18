# Aircraft Encyclopedia — Data Dictionary

---

## How to Use This Document

Each namespace section opens with a **summary paragraph** explaining the purpose, the entities it models, and its position in the dependency chain. This is followed by **column-level tables** for every table in that namespace. Column tables use the following fields:

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

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `code` | `lookup_code` | NOT NULL | — | PK | Canonical unit code, e.g. `KIAS`, `LB`, `US_GPH` |
| `label` | `TEXT` | NOT NULL | — | — | Display label, e.g. `Knots Indicated Airspeed` |
| `unit_category_code` | `lookup_code` | NOT NULL | — | `unit_categories.code` | Groups units by physical dimension |
| `canonical_unit_code` | `lookup_code` | nullable | — | self | The unit this converts *to*; NULL if this unit is already canonical |
| `canonical_factor` | `NUMERIC` | nullable | — | — | Multiply raw value by this to get canonical value. NULL for temperature (requires offset arithmetic) |
| `source_string_patterns` | `TEXT[]` | nullable | — | — | Lowercase raw strings observed in PlanePHD data (e.g. `{kias, knots}`) used by Phase 17 parser |
| `is_active` | `BOOLEAN` | NOT NULL | `TRUE` | — | FALSE soft-deletes deprecated unit codes |

### `aircraft_ref.performance_metric_types`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `code` | `lookup_code` | NOT NULL | — | PK | e.g. `CRUISE_SPEED`, `RANGE`, `VX` |
| `label` | `TEXT` | NOT NULL | — | — | Display label |
| `canonical_unit_code` | `lookup_code` | nullable | — | `measurement_units.code` | The unit canonical values are stored in |
| `higher_is_better` | `BOOLEAN` | nullable | — | — | Drives comparison scoring direction. NULL = non-directional (e.g. stall speed context-dependent) |
| `description` | `TEXT` | nullable | — | — | Definition and measurement standard |

### `aircraft_ref.cost_item_types`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `code` | `lookup_code` | NOT NULL | — | PK | e.g. `FUEL_COST_PER_HOUR`, `ANNUAL_INSPECTION` |
| `label` | `TEXT` | NOT NULL | — | — | Display label |
| `is_fixed` | `BOOLEAN` | NOT NULL | — | — | TRUE = annual fixed cost; FALSE = per-hour variable cost |
| `is_aggregate` | `BOOLEAN` | NOT NULL | `FALSE` | — | TRUE for totals rows (`TOTAL_COST_ANNUAL`, `TOTAL_FIXED_COST`, etc.) that must not be double-counted in SUM() |

*All other `aircraft_ref` lookup tables follow the same `code / label / description / sort_order / is_active` pattern. See Phase 2 seed files for the complete list of 35 tables and their seeded values.*

---

## 2. `aircraft_geo` — Geography

**Purpose.** Provides the geographic reference layer: ISO country records (including historical states needed for Cold War and WWII aircraft), named regional groupings, and the M:N junction mapping countries to regions. Consumed primarily by `aircraft_org` (country of HQ) and `aircraft_core` (country of origin, operator countries).

### `aircraft_geo.countries`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `code` | `CHAR(3)` | NOT NULL | — | PK | ISO 3166-1 alpha-3 country code (e.g. `USA`, `GBR`). Historical states use IOC/ICAO codes (e.g. `SUN` for USSR) |
| `alpha2_code` | `CHAR(2)` | nullable | — | — | ISO 3166-1 alpha-2 code where applicable |
| `country_name` | `TEXT` | NOT NULL | — | — | English common name |
| `official_name` | `TEXT` | nullable | — | — | Full official name |
| `is_active` | `BOOLEAN` | NOT NULL | `TRUE` | — | FALSE for dissolved states (USSR, DDR, CSK, YUG) |
| `dissolved_year` | `aircraft_ref.year_value` | nullable | — | — | Year of dissolution for historical states |
| `notes` | `TEXT` | nullable | — | — | Curator notes |

### `aircraft_geo.regions`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `code` | `lookup_code` | NOT NULL | — | PK | e.g. `NATO`, `EASA_STATES`, `WESTERN_EUROPE` |
| `region_name` | `TEXT` | NOT NULL | — | — | Display name |
| `region_type` | `TEXT` | NOT NULL | — | — | One of: `GEOGRAPHIC`, `POLITICAL`, `AVIATION`, `MILITARY` |
| `description` | `TEXT` | nullable | — | — | Membership criteria or scope note |

### `aircraft_geo.country_regions`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `country_code` | `CHAR(3)` | NOT NULL | — | `countries.code` |  |
| `region_code` | `lookup_code` | NOT NULL | — | `regions.code` |  |
| `membership_start_year` | `aircraft_ref.year_value` | nullable | — | — | Year of accession; NULL = founding member or unknown |
| `membership_end_year` | `aircraft_ref.year_value` | nullable | — | — | Year of departure; NULL = current member |

---

## 3. `aircraft_org` — Organizations

**Purpose.** A unified organization table replacing the reference schema's narrow `manufacturers` table. Covers manufacturers, design bureaux, military operators, civil operators, certification authorities, and consortium bodies. The `org_relationships` table captures historical mergers, license agreements, and parent-subsidiary structures.

### `aircraft_org.organizations`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK | Surrogate key |
| `org_name` | `TEXT` | NOT NULL | — | — | Canonical English name |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE | URL-routing identifier |
| `org_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.organization_types.code` | e.g. `MANUFACTURER`, `DESIGN_BUREAU`, `OPERATOR_MILITARY` |
| `country_code` | `CHAR(3)` | nullable | — | `aircraft_geo.countries.code` | Country of HQ; NULL for international bodies |
| `founded_year` | `aircraft_ref.year_value` | nullable | — | — |  |
| `dissolved_year` | `aircraft_ref.year_value` | nullable | — | — | NULL if still active |
| `icao_mfr_code` | `TEXT` | nullable | — | — | ICAO manufacturer code where assigned |
| `name_aliases` | `TEXT[]` | nullable | — | — | Alternative names used in source data; GIN-indexed for `@>` alias resolution |
| `extra_attributes` | `JSONB` | nullable | `'{}'` | — | Spare JSONB for sparse attributes |
| `created_at` | `TIMESTAMPTZ` | NOT NULL | `now()` | — |  |
| `updated_at` | `TIMESTAMPTZ` | NOT NULL | `now()` | — | Maintained by `set_updated_at()` trigger |

### `aircraft_org.org_relationships`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `from_org_id` | `BIGINT` | NOT NULL | — | `organizations.id RESTRICT` | The acquiring, licensing, or parent entity |
| `to_org_id` | `BIGINT` | NOT NULL | — | `organizations.id RESTRICT` | The acquired, licensed, or subsidiary entity |
| `relationship_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.org_relationship_types.code` | e.g. `ACQUIRED`, `LICENSE_AGREEMENT`, `PARENT_SUBSIDIARY` |
| `started_year` | `aircraft_ref.year_value` | nullable | — | — |  |
| `ended_year` | `aircraft_ref.year_value` | nullable | — | — | NULL = relationship still active |
| `notes` | `TEXT` | nullable | — | — |  |

---

## 4. `aircraft_core` — Aircraft Identity

**Purpose.** The identity backbone of the entire database. The three-tier `families → models → variants` hierarchy establishes the atomic unit (`variants`) that all domain FK chains reference. Four junction tables attach aliases, roles, manufacturers, and operators to variants.

**Why three tiers?** A *family* is the broadest grouping under one manufacturer (e.g. all Cessna 172 variants). A *model* is a distinct product name within a family (e.g. 172S, 172R). A *variant* is a specific production configuration (e.g. 172S with Garmin G1000 Nxi). This matches how aviation databases, FAA type certificates, and buyer research actually work.

### `aircraft_core.families`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `manufacturer_org_id` | `BIGINT` | nullable | — | `aircraft_org.organizations.id SET NULL` | Primary manufacturer; NULL for design bureaux |
| `family_name` | `TEXT` | NOT NULL | — | — | e.g. `Cessna 172` |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE |  |
| `description` | `TEXT` | nullable | — | — |  |
| `name_tsv` | `TSVECTOR` | GENERATED | — | — | Full-text search vector over name and aliases; backed by GIN index |

### `aircraft_core.models`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `family_id` | `BIGINT` | NOT NULL | — | `families.id RESTRICT` |  |
| `model_name` | `TEXT` | NOT NULL | — | — | e.g. `172S` |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE |  |

### `aircraft_core.variants`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `model_id` | `BIGINT` | NOT NULL | — | `models.id RESTRICT` |  |
| `variant_name` | `TEXT` | NOT NULL | — | — | e.g. `172S (G1000 Nxi)` |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE |  |
| `description` | `TEXT` | nullable | — | — | Source description text |
| `production_start_year` | `aircraft_ref.year_value` | nullable | — | — |  |
| `production_end_year` | `aircraft_ref.year_value` | nullable | — | — | NULL if still in production |
| `is_in_production` | `BOOLEAN` | nullable | — | — |  |
| `passenger_capacity` | `SMALLINT` | nullable | — | — | Total occupants including pilots; denormalized for faceted search |
| `engine_count` | `SMALLINT` | nullable | — | — | Denormalized; authoritative value in `aircraft_power.variant_powerplants` |
| `propulsion_category_code` | `lookup_code` | nullable | — | `aircraft_ref.propulsion_categories.code` | Denormalized for faceted search |
| `landing_gear_type_code` | `lookup_code` | nullable | — | `aircraft_ref.landing_gear_types.code` | Denormalized for faceted search |
| `service_status_code` | `lookup_code` | nullable | — | `aircraft_ref.service_statuses.code` | e.g. `IN_SERVICE`, `RETIRED`, `PROTOTYPE` |
| `variant_type_code` | `lookup_code` | nullable | — | `aircraft_ref.variant_types.code` | e.g. `PRODUCTION`, `PROTOTYPE`, `EXPERIMENTAL` |
| `country_of_origin_code` | `CHAR(3)` | nullable | — | `aircraft_geo.countries.code` | May reference inactive historical states |
| `source_path` | `TEXT` | nullable | — | — | Source relative URL; staging convenience, superseded by Phase 14 provenance |
| `ingest_key` | `TEXT` | nullable | — | — | `'MANUFACTURER::aircraft name'` dedup key; UNIQUE WHERE NOT NULL |
| `description_tsv` | `TSVECTOR` | GENERATED | — | — | Full-text search vector; GIN-indexed |
| `extra_attributes` | `JSONB` | nullable | `'{}'` | — | Sparse source-specific metadata |

### `aircraft_core.variant_aliases`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `alias_text` | `TEXT` | NOT NULL | — | — | The alias string e.g. `Fishbed-C`, `MiG-21F-13` |
| `alias_type` | `TEXT` | NOT NULL | — | — | One of: `NATO_REPORTING`, `MILITARY_DESIGNATION`, `POPULAR_NAME`, `EXPORT_DESIGNATION`, `MANUFACTURER_CODE`, `OTHER` |
| `country_code` | `CHAR(3)` | nullable | — | `aircraft_geo.countries.code SET NULL` | Country context for the alias |

---

## 5. `aircraft_cert` — Certification

**Purpose.** Stores formal type certificate records and their M:N links to variants (one TC can cover multiple models in a series), operating approvals as a per-approval fact table, a 1:1 pilot requirements extension, and public safety metrics.

### `aircraft_cert.type_certificates`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `tc_number` | `TEXT` | NOT NULL | — | UNIQUE | Official TC number, e.g. `A00009AT` |
| `authority_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.certification_authorities.code` | e.g. `FAA`, `EASA`, `CAA_UK` |
| `airworthiness_category_code` | `lookup_code` | nullable | — | `aircraft_ref.airworthiness_categories.code` | e.g. `NORMAL`, `UTILITY`, `AEROBATIC` |
| `tc_holder_org_id` | `BIGINT` | nullable | — | `aircraft_org.organizations.id SET NULL` |  |
| `issue_date` | `DATE` | nullable | — | — |  |
| `source_url` | `TEXT` | nullable | — | — | URL to official TC data sheet |

### `aircraft_cert.variant_operating_approvals`

Fact table — one row per `(variant, approval_type)`.

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `approval_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.operating_approval_types.code` | e.g. `IFR`, `FIKI`, `AEROBATIC`, `ETOPS` |
| `is_approved` | `BOOLEAN` | nullable | — | — | TRUE = approved; FALSE = explicitly not approved; NULL = unknown |
| `conditions` | `TEXT` | nullable | — | — | Conditions or limitations on the approval |
| `confidence` | `aircraft_ref.confidence_score` | nullable | `0.20` | — | Source quality score |

### `aircraft_cert.pilot_requirements`

1:1 extension — one row per variant.

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | UNIQUE FK → `variants.id` |  |
| `min_certificate_code` | `lookup_code` | nullable | — | `aircraft_ref.pilot_certificate_types.code` | Minimum required pilot certificate |
| `type_rating_required` | `BOOLEAN` | nullable | — | — |  |
| `type_rating_code` | `TEXT` | nullable | — | — | Type rating designation; only non-NULL when `type_rating_required = TRUE` |
| `instrument_rating_required` | `BOOLEAN` | nullable | — | — |  |
| `min_flight_hours` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Minimum total flight hours for insurance or operator policy |

---

## 6. `aircraft_specs` — Specifications

**Purpose.** Three metric-fact tables covering dimensions, weights, and performance, plus the CG envelope and payload-range curve tables. All follow the **raw value / raw unit / canonical value** triple pattern that preserves source fidelity while enabling unit-normalized comparison.

**The triple pattern.** Every measurement stores: `raw_value NUMERIC` (exactly as parsed from source), `raw_unit_code` (the source's unit), and `canonical_value NUMERIC` (converted to the canonical unit via `aircraft_ref.to_canonical()`). Queries always filter and sort on `canonical_value`; `raw_value` and `raw_unit_code` are for provenance and display.

### `aircraft_specs.performance_metrics`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `metric_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.performance_metric_types.code` |  |
| `raw_value` | `NUMERIC` | nullable | — | — | NULL for sentinel values (`None KIAS`) |
| `raw_unit_code` | `lookup_code` | nullable | — | `aircraft_ref.measurement_units.code` | NULL for dimensionless metrics |
| `canonical_value` | `NUMERIC` | nullable | — | — | Converted to canonical unit; NULL when raw_value is NULL |
| `is_canonical` | `BOOLEAN` | NOT NULL | `TRUE` | — | Only one TRUE per `(variant_id, metric_type_code)` via partial UNIQUE index |
| `condition_weight_lbs` | `NUMERIC` | nullable | — | — | Test weight if specified |
| `condition_weight_label` | `TEXT` | nullable | — | — | e.g. `MTOW`, `BOW` |
| `condition_altitude_ft` | `NUMERIC` | nullable | — | — |  |
| `condition_power_setting` | `TEXT` | nullable | — | — | One of: `MAX_CONTINUOUS`, `75_PERCENT`, `65_PERCENT`, `BEST_ECONOMY`, `BEST_POWER`, `MAX_TAKEOFF`, `IDLE`, `MILITARY`, `AFTERBURNER`, `NOT_SPECIFIED` |
| `condition_surface_type` | `TEXT` | nullable | — | — | One of: `PAVED`, `GRASS`, `GRAVEL`, `TURF`, `DIRT`, `WATER` |
| `conditions_notes` | `TEXT` | nullable | — | — | Free-text condition qualifiers from source |
| `confidence` | `aircraft_ref.confidence_score` | NOT NULL | `0.20` | — |  |
| `source_document_id` | `BIGINT` | nullable | — | `aircraft_prov.source_documents.id SET NULL` |  |

### `aircraft_specs.weight_metrics`

Same triple-pattern structure as `performance_metrics`. Key difference: no `is_canonical` flag (one row per `(variant, metric_type, COALESCE(configuration,''))` enforced by functional UNIQUE index). Allows negative load factor values (`LOAD_FACTOR_NEG`).

### `aircraft_specs.dimension_metrics`

Same triple-pattern structure. Non-negative values enforced (`chk_dm_canonical_nonneg`, `chk_dm_raw_nonneg`). The `configuration` column (e.g. `WINGS_FOLDED`) enables multiple wingspan values per variant.

### `aircraft_specs.cg_envelopes`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `config_label` | `TEXT` | nullable | — | — | e.g. `STANDARD`, `WITH_TIP_TANKS` |
| `fwd_limit_points` | `JSONB` | NOT NULL | — | — | Array of `{"w": weight_lbs, "cg": cg_inches}` objects |
| `aft_limit_points` | `JSONB` | NOT NULL | — | — | Same structure as `fwd_limit_points` |
| `cg_unit` | `TEXT` | NOT NULL | `'INCHES_AFT_DATUM'` | — | CG reference datum |
| `most_fwd_cg` / `most_aft_cg` | `NUMERIC` | nullable | — | — | Pre-computed extremes for scalar filtering |

---

## 7. `aircraft_power` — Propulsion

**Purpose.** A complete propulsion stack: an engine specification catalog, a many-to-many powerplant junction (replacing the reference schema's single `engine_id` FK), propeller specs, rotor systems, APU specs, and STC conversion records.

### `aircraft_power.engine_variants`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `manufacturer_org_id` | `BIGINT` | nullable | — | `aircraft_org.organizations.id SET NULL` | NULL for stub rows created during ingestion before org matching |
| `manufacturer_name_raw` | `TEXT` | nullable | — | — | Raw source string; used for dedup before org matching |
| `model_designation` | `TEXT` | NOT NULL | — | — | e.g. `O-320-D2J`, `AI-25TL` |
| `propulsion_category_code` | `lookup_code` | nullable | — | `aircraft_ref.propulsion_categories.code` |  |
| `rated_power_hp` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Piston/turboprop rated power |
| `rated_thrust_n` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Jet/turbofan rated thrust in Newtons |
| `rated_thrust_lbf` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Derived from `rated_thrust_n` for display |
| `tbo_hours` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Time between overhaul |
| `name_aliases` | `TEXT[]` | nullable | — | — | Alternative designations; GIN-indexed |
| `has_afterburner` | `BOOLEAN` | NOT NULL | `FALSE` | — |  |
| `thrust_lbf_wet` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Afterburner thrust; only non-NULL when `has_afterburner = TRUE` |

### `aircraft_power.variant_powerplants`

M:N junction — multiple engine options per variant.

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `engine_variant_id` | `BIGINT` | NOT NULL | — | `engine_variants.id RESTRICT` |  |
| `engine_count` | `SMALLINT` | NOT NULL | `1` | — | Number of identical engines installed |
| `is_standard` | `BOOLEAN` | NOT NULL | — | — | Factory standard fitment |
| `is_optional` | `BOOLEAN` | NOT NULL | — | — | Factory option (non-exclusive with `is_standard` for conversions) |
| `is_primary` | `BOOLEAN` | NOT NULL | `TRUE` | — | The engine used for performance comparisons; partial UNIQUE ensures one per variant |
| `source_document_id` | `BIGINT` | nullable | — | `aircraft_prov.source_documents.id SET NULL` |  |

---

## 8. `aircraft_systems` — Avionics and Equipment

**Purpose.** A named equipment catalog with variant-level links (individual items) and named avionics suite bundles. Enables both item-level filtering ("has ADS-B Out") and bundle-level filtering ("equipped with G1000 Nxi").

### `aircraft_systems.equipment_catalog`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `equipment_name` | `TEXT` | NOT NULL | — | UNIQUE |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE |  |
| `category_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.systems_categories.code` | e.g. `AVIONICS_NAV`, `SAFETY_SYSTEM`, `ICE_PROTECTION` |
| `manufacturer_org_id` | `BIGINT` | nullable | — | `aircraft_org.organizations.id SET NULL` |  |
| `model_number` | `TEXT` | nullable | — | — |  |
| `name_aliases` | `TEXT[]` | nullable | — | — | GIN-indexed; used for ingestion alias resolution |

### `aircraft_systems.variant_equipment`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `equipment_id` | `BIGINT` | NOT NULL | — | `equipment_catalog.id RESTRICT` |  |
| `provision_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.equipment_provision_types.code` | e.g. `STANDARD`, `OPTIONAL`, `FIELD_RETROFIT` |
| UNIQUE | `(variant_id, equipment_id)` | — | — | — | One provision type per (variant, item) — see key design decision in Phase 10 |

---

## 9. `aircraft_military` — Military Reference Data

**Purpose.** Public, unclassified encyclopedia-only reference data: structural hardpoint capability, store-type compatibility, a weapons/stores catalog with mandatory source citations, representative loadout examples, mission capability assertions, and military sensor links. **No classified data. No operational, tactical, or employment guidance.**

### `aircraft_military.hardpoints`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `station_designation` | `TEXT` | NOT NULL | — | — | e.g. `Station 3`, `Wing Root Port` |
| `position_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.hardpoint_position_types.code` | e.g. `WING_INBOARD`, `FUSELAGE_CENTERLINE` |
| `max_load_kg` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Maximum external store weight per station |
| `is_wet` | `BOOLEAN` | NOT NULL | `FALSE` | — | TRUE if the station has fuel plumbing for drop tanks |

### `aircraft_military.weapons_catalog`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `designation` | `TEXT` | NOT NULL | — | — | Official designation, e.g. `AIM-9X Sidewinder` |
| `nato_reporting_name` | `TEXT` | nullable | — | — |  |
| `weapon_category_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.weapon_categories.code` |  |
| `stores_type_code` | `lookup_code` | nullable | — | `aircraft_ref.stores_types.code` |  |
| `country_of_origin_code` | `CHAR(3)` | nullable | — | `aircraft_geo.countries.code` |  |
| `description` | `TEXT` | nullable | — | — | Public reference description only |

### `aircraft_military.representative_loadouts`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `loadout_name` | `TEXT` | NOT NULL | — | — | e.g. `Standard Air Defence` |
| `description` | `TEXT` | nullable | — | — |  |
| `source_notes` | `TEXT` | nullable | — | — | **Every row must cite a public reference.** Design intent: required field. |

---

## 10. `aircraft_market` — Market Data

**Purpose.** Two separated concerns: `valuations` (market price time-series) and `cost_snapshots` + `cost_line_items` (ownership cost estimates). Separated because they have different sources, update frequencies, and query patterns. The normalization of cost items into a fact table replaces the reference schema's fixed columns + JSONB overflow pattern.

### `aircraft_market.valuations`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `snapshot_date` | `DATE` | NOT NULL | — | — |  |
| `source_name` | `TEXT` | NOT NULL | — | — | Source system name; dedup uses functional UNIQUE on `(variant_id, snapshot_date, COALESCE(source_name,''))` |
| `papi_price_usd` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Source system's estimated market value in USD. Column name retained from PlanePHD PAPI methodology; semantics = "source's estimated typical market value" |
| `for_sale_count` | `INTEGER` | nullable | — | — | Marketplace listings count at snapshot date |
| `currency_code` | `VARCHAR(3)` | NOT NULL | `'USD'` | `aircraft_ref.currencies.code` |  |
| `captured_at` | `TIMESTAMPTZ` | NOT NULL | `now()` | — |  |

### `aircraft_market.cost_snapshots`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `snapshot_date` | `DATE` | NOT NULL | — | — |  |
| `currency_code` | `VARCHAR(3)` | NOT NULL | `'USD'` | `aircraft_ref.currencies.code` |  |
| `source_name` | `TEXT` | NOT NULL | — | — |  |
| `assumed_annual_hours` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Annual utilization assumption underlying variable cost calculations |
| `assumed_fuel_price_per_gal` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Fuel price assumption; embedded in PlanePHD dynamic cost keys |
| `extra_attributes` | `JSONB` | NOT NULL | `'{}'` | — | Unmapped and dynamic cost keys stored verbatim; populated from Phase 17 promotion |

### `aircraft_market.cost_line_items`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `snapshot_id` | `BIGINT` | NOT NULL | — | `cost_snapshots.id CASCADE` |  |
| `cost_item_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.cost_item_types.code` |  |
| `amount_annual` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Annual cost in snapshot currency |
| `amount_per_hour` | `aircraft_ref.nonneg_numeric` | nullable | — | — | Per-flight-hour cost in snapshot currency |
| `currency_code` | `VARCHAR(3)` | NOT NULL | — | `aircraft_ref.currencies.code` |  |
| UNIQUE | `(snapshot_id, cost_item_type_code)` | — | — | — | One row per cost type per snapshot |
| CHECK | `chk_cli_has_amount` | — | — | — | At least one of `amount_annual`, `amount_per_hour` must be non-NULL |

**Aggregation note.** To compute total annual cost from a snapshot, use:
```sql
SELECT SUM(amount_annual) + SUM(amount_per_hour) * cs.assumed_annual_hours
FROM cost_line_items cli
JOIN cost_snapshots cs ON cs.id = cli.snapshot_id
WHERE cli.snapshot_id = :snapshot_id
  AND NOT (SELECT is_aggregate FROM aircraft_ref.cost_item_types WHERE code = cli.cost_item_type_code);
```
Filter out `is_aggregate = TRUE` rows (`TOTAL_COST_ANNUAL`, `TOTAL_FIXED_COST`, `TOTAL_VARIABLE_COST`) to avoid double-counting.

---

## 11. `aircraft_maint` — Maintenance and Reliability

**Purpose.** Airworthiness directives (ADs), service bulletins (SBs), life-limited parts, and a 1:1 editorial support assessment. ADs and SBs use M:N junctions (one AD can affect many variants; one variant can be subject to many ADs).

### `aircraft_maint.airworthiness_directives`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `ad_number` | `TEXT` | NOT NULL | — | UNIQUE | Official AD number, e.g. `2023-14-08` |
| `authority_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.certification_authorities.code` |  |
| `subject` | `TEXT` | NOT NULL | — | — | Brief subject description |
| `effective_date` | `DATE` | nullable | — | — |  |
| `compliance_type` | `TEXT` | NOT NULL | — | — | One of: `RECURRING`, `ONE_TIME`, `ON_CONDITION`, `AIRWORTHINESS_LIMITATION` |
| `source_url` | `TEXT` | nullable | — | — | Link to official AD text |

### `aircraft_maint.support_assessments`

1:1 editorial assessment per variant.

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | UNIQUE FK → `variants.id` |  |
| `oem_support_status` | `TEXT` | NOT NULL | — | — | One of: `FULL`, `LIMITED`, `THIRD_PARTY_ONLY`, `NONE` |
| `parts_availability_score` | `SMALLINT` | nullable | — | — | 1–5 scale; 5 = readily available worldwide |
| `corrosion_risk_level` | `TEXT` | nullable | — | — | One of: `LOW`, `MODERATE`, `HIGH`, `VERY_HIGH` |
| `dispatch_reliability_pct` | `NUMERIC(5,2)` | nullable | — | — | Typical fleet dispatch reliability percentage; CHECK (0–100) |
| `assessment_notes` | `TEXT` | nullable | — | — | Curator editorial notes |

---

## 12. `aircraft_prov` — Provenance and Curation

**Purpose.** The provenance backbone. Every field value in canonical tables should have at least one `source_assertions` row linking it to its originating `source_documents` record. The `is_accepted` flag marks the current winning value per `(entity_type, entity_id, field_name)`, enabling multi-source conflict resolution without overwriting previous values.

### `aircraft_prov.sources`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `source_name` | `TEXT` | NOT NULL | — | — |  |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE |  |
| `source_type_code` | `lookup_code` | nullable | — | `aircraft_ref.source_types.code` | e.g. `OFFICIAL_TC`, `MARKETPLACE_DB`, `MANUFACTURER_SPEC` |
| `reliability_grade_code` | `lookup_code` | nullable | — | `aircraft_ref.source_reliability_grades.code` | e.g. `AUTHORITATIVE`, `VERIFIED`, `UNVERIFIED` |
| `base_url` | `TEXT` | nullable | — | — |  |
| `default_confidence` | `aircraft_ref.confidence_score` | NOT NULL | `0.50` | — | Default confidence applied to assertions from this source; overridden per-assertion |

### `aircraft_prov.source_assertions`

The core provenance fact table.

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `source_document_id` | `BIGINT` | NOT NULL | — | `source_documents.id CASCADE` |  |
| `entity_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.curation_entity_types.code` | e.g. `VARIANT`, `ENGINE`, `COST_SNAPSHOT` |
| `entity_id` | `BIGINT` | NOT NULL | — | — | PK value in the target table (polymorphic; no DDL-enforced FK) |
| `field_name` | `TEXT` | NOT NULL | — | — | Dotted field path, e.g. `performance.CRUISE_SPEED`, `weight.WEIGHT_EMPTY` |
| `raw_key` | `TEXT` | nullable | — | — | Original source key before mapping |
| `asserted_value` | `TEXT` | nullable | — | — | Value exactly as it appeared in the source (including units) |
| `status_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.assertion_statuses.code` | `PENDING`, `ACCEPTED`, `REJECTED`, `SUPERSEDED` |
| `is_accepted` | `BOOLEAN` | NOT NULL | `FALSE` | — | Exactly one TRUE per `(entity_type, entity_id, field_name)` enforced by partial UNIQUE index `uq_assertion_accepted` |
| `confidence` | `aircraft_ref.confidence_score` | NOT NULL | `0.20` | — |  |

### `aircraft_prov.curation_flags`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `entity_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.curation_entity_types.code` |  |
| `entity_id` | `BIGINT` | NOT NULL | — | — | Polymorphic FK |
| `flag_type` | `TEXT` | NOT NULL | — | — | e.g. `INGESTION_WARNING`, `CONFLICTING_SOURCES`, `MISSING_DATA` |
| `description` | `TEXT` | NOT NULL | — | — | Human-readable issue description; max 1,000 chars at ingestion |
| `status_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.curation_flag_statuses.code` | `OPEN`, `IN_REVIEW`, `RESOLVED`, `DISMISSED` |
| `resolved_at` | `TIMESTAMPTZ` | nullable | — | — | Only non-NULL when `status_code` is terminal (`RESOLVED` or `DISMISSED`) |

### `aircraft_prov.audit_log`

Append-only change history. INSERT only — no UPDATE or DELETE. Enforced by application-level GRANT policy (INSERT only on this table for service accounts).

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `entity_type_code` | `lookup_code` | nullable | — | — |  |
| `entity_id` | `BIGINT` | nullable | — | — |  |
| `action` | `TEXT` | NOT NULL | — | — | e.g. `ASSERTION_ACCEPTED`, `FLAG_RESOLVED`, `VARIANT_CREATED` |
| `old_value` | `JSONB` | nullable | — | — | Previous state snapshot |
| `new_value` | `JSONB` | nullable | — | — | New state snapshot |
| `changed_by` | `TEXT` | nullable | — | — | Application user or service account identifier |
| `created_at` | `TIMESTAMPTZ` | NOT NULL | `now()` | — |  |

---

## 13. `aircraft_compare` — Mission Profiles and Scoring

**Purpose.** The comparison engine: weighted mission profiles, per-criterion score storage, and aggregate suitability scores. Scores are pre-computed by a scoring job that runs after each significant data update and stored in `variant_suitability` / `criterion_scores`. The `aircraft_read.mv_variant_search` matview joins to `variant_suitability` for ranked search results.

### `aircraft_compare.mission_profiles`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `id` | `BIGINT` | NOT NULL | identity | PK |  |
| `profile_name` | `TEXT` | NOT NULL | — | — | e.g. `IFR Cross-Country` |
| `slug` | `aircraft_ref.slug_text` | NOT NULL | — | UNIQUE |  |
| `profile_type_code` | `lookup_code` | NOT NULL | — | UNIQUE FK → `aircraft_ref.mission_profile_types.code` | One profile per type; prevents duplicate active profiles |
| `description` | `TEXT` | nullable | — | — |  |

### `aircraft_compare.mission_criteria`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `mission_profile_id` | `BIGINT` | NOT NULL | — | `mission_profiles.id CASCADE` |  |
| `criterion_type_code` | `lookup_code` | NOT NULL | — | `aircraft_ref.comparison_criterion_types.code` |  |
| `weight` | `NUMERIC(5,4)` | NOT NULL | — | — | Fractional weight; all weights for a profile must sum to 1.0000 (enforced by `v_weight_criteria_validation` view, not DDL) |
| UNIQUE | `(mission_profile_id, criterion_type_code)` | — | — | — | One criterion type per profile |

### `aircraft_compare.variant_suitability`

| Column | Type | Null | Default | FK | Description |
|---|---|---|---|---|---|
| `variant_id` | `BIGINT` | NOT NULL | — | `variants.id CASCADE` |  |
| `mission_profile_id` | `BIGINT` | NOT NULL | — | `mission_profiles.id CASCADE` |  |
| `overall_score` | `NUMERIC(5,4)` | nullable | — | — | Weighted composite score in [0,1] |
| `scored_at` | `TIMESTAMPTZ` | NOT NULL | `now()` | — | Timestamp of last score computation |
| `scoring_notes` | `TEXT` | nullable | — | — |  |
| UNIQUE | `(variant_id, mission_profile_id)` | — | — | — |  |

---

## 14. `aircraft_ingest` — Staging (Transient)

**Purpose.** Transient ETL namespace. Three tables hold the intermediate state of the Phase 17 ingestion pipeline: `ingest_runs` (batch metadata), `staged_aircraft` (flattened JSON rows), and `staged_images` (image array rows). These tables are not part of the canonical data model and are safe to truncate after provenance data has been validated.

Key columns are documented inline in `901_seed_data_staging.sql` via `COMMENT ON COLUMN` statements. See Phase 17 summary for full field descriptions.

---

## 15. `aircraft_read` — Read Models

**Purpose.** Denormalized views and materialized views optimized for frontend query patterns. Application code should query this namespace, not the normalized source tables directly.

| Object | Type | Description |
|---|---|---|
| `v_current_valuation` | VIEW | `DISTINCT ON` latest valuation per variant; one row per variant |
| `v_hangar_fit` | VIEW | Derives T-hangar fit from `dimension_metrics`; `fits_standard_t_hangar BOOLEAN` |
| `v_weight_criteria_validation` | VIEW | QA: checks mission_criteria weights sum to 1.0000 per profile |
| `mv_ownership_cost_summary` | MATVIEW | Aggregated annual + per-hour cost from latest snapshot; ~6 columns |
| `mv_variant_search` | MATVIEW | 45-column denormalized search surface; GIN full-text + 20 B-tree/partial indexes; **must be refreshed after data changes** |
| `refresh_search_matviews(concurrent BOOLEAN)` | FUNCTION | Refreshes `mv_ownership_cost_summary` then `mv_variant_search` in correct order. Pass `FALSE` for initial population (no `CONCURRENTLY`); default `TRUE` for live updates |

**Critical refresh note.** `mv_variant_search` is created `WITH NO DATA`. It returns zero rows until `refresh_search_matviews(FALSE)` is called. Subsequent incremental updates use `refresh_search_matviews()` (concurrent = TRUE by default), which does not block reads.
