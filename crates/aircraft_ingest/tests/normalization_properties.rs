//! Invariants that must hold for *any* `PlanePHD` input, not just the fixtures.
//!
//! The golden snapshots in `database/snapshots/` prove the adapter still writes
//! what it wrote before. They cannot prove it writes the right thing, because
//! they are a record of its own past output — and since the legacy SQL loader
//! was retired there is no second implementation to check against.
//!
//! These tests close part of that gap by asserting the rules the adapter claims
//! to follow, over generated input:
//!
//! - a sentinel never becomes a measurement;
//! - an unmapped unit never produces a canonical unit code;
//! - normalization is deterministic;
//! - source-record identity is stable and distinguishes distinct inputs;
//! - unrecognized input is preserved and flagged, never silently dropped.
//!
//! What they still do not cover: whether the *mapping tables themselves* are
//! right — that `best_cruise_speed` should mean `SPEED_CRUISE_BEST`, or that
//! `GAL` should mean `US_GAL`. Nothing but a specification or an independent
//! implementation can establish that.

use aircraft_app::ingestion::IssueSeverity;
use aircraft_ingest::normalization::{normalize_record, parse_numeric, parse_sentinel};
use proptest::prelude::*;
use serde_json::{Value, json};

/// Values the source uses to mean "no data", per `parse_sentinel`.
const SENTINELS: &[&str] = &["none", "None", "NONE", "n/a", "N/A", "-", "--", "", "   "];

/// Every unit `unit_code` maps. Anything else must not reach a canonical unit.
const MAPPED_UNITS: &[&str] = &[
  "KIAS", "KCAS", "KTAS", "NM", "FT", "FPM", "GPH", "LBS", "LB", "KG", "GAL", "HP", "KW", "N",
  "LBF", "HRS", "HR", "PPH",
];

/// Real aviation units this adapter does *not* map. A purely random `[A-Z]{2,6}`
/// generator will never produce one of these by chance, so a regression that
/// quietly started mapping `MPH` would go unnoticed. Drawing from this list is
/// what gives the unmapped-unit property its teeth.
const PLAUSIBLE_UNMAPPED_UNITS: &[&str] = &[
  "MPH", "KMH", "KTS", "SM", "MI", "KM", "LTR", "USG", "PSI", "RPM", "TONS", "OZ", "DEG", "MB",
  "INHG", "SEC", "MIN", "NMI",
];

/// Units drawn from both the plausible list and the random space, so the
/// property covers realistic regressions and arbitrary input alike.
fn unmapped_unit_strategy() -> impl Strategy<Value = String> {
  prop_oneof![
    4 => proptest::sample::select(PLAUSIBLE_UNMAPPED_UNITS).prop_map(str::to_owned),
    1 => "[A-Z]{2,6}".prop_map(|unit| unit),
  ]
}

fn name_strategy() -> impl Strategy<Value = String> {
  "[A-Za-z][A-Za-z0-9 .-]{0,24}"
    .prop_map(|value| value.trim().to_owned())
    .prop_filter("manufacturer and aircraft names must be non-empty after trimming", |value| {
      !value.is_empty()
    })
}

proptest! {
  /// A sentinel means the source had no value, so it must not become a
  /// measurement — while a real value alongside it still must. The control
  /// field is what stops this passing vacuously: asserting only "no numeric
  /// measurement" would hold trivially if normalization produced none at all.
  #[test]
  fn a_sentinel_yields_no_measurement_while_a_real_value_still_does(
    sentinel in proptest::sample::select(SENTINELS),
    manufacturer in name_strategy(),
    aircraft in name_strategy(),
  ) {
    let record = normalize_record(
      &manufacturer,
      &aircraft,
      json!({
        "performance": {"best_cruise_speed": sentinel, "ceiling": "14000 FT"},
        "weights": {"gross_weight": sentinel}
      }),
    );

    let control = record
      .performance
      .measurements
      .iter()
      .find(|measurement| measurement.source_field == "ceiling");
    prop_assert!(
      control.is_some_and(|measurement| measurement.numeric_value.is_some()),
      "the control measurement disappeared, so this property proves nothing: {:?}",
      record.performance.measurements
    );

    for measurement in
      record.performance.measurements.iter().chain(record.weights.measurements.iter())
    {
      prop_assert_ne!(
        &measurement.source_field,
        "best_cruise_speed",
        "sentinel {:?} produced a performance measurement",
        sentinel
      );
      prop_assert_ne!(
        &measurement.source_field,
        "gross_weight",
        "sentinel {:?} produced a weight measurement",
        sentinel
      );
    }
  }

  /// An unmapped unit must never acquire a canonical unit code. This is what
  /// stops '213 MPH' being stored as though 213 were already canonical.
  #[test]
  fn an_unmapped_unit_never_yields_a_canonical_unit_code(
    magnitude in 1_u32..100_000,
    unit in unmapped_unit_strategy(),
    manufacturer in name_strategy(),
    aircraft in name_strategy(),
  ) {
    prop_assume!(!MAPPED_UNITS.contains(&unit.as_str()));

    let record = normalize_record(
      &manufacturer,
      &aircraft,
      json!({"performance": {"best_cruise_speed": format!("{magnitude} {unit}")}}),
    );

    for measurement in &record.performance.measurements {
      prop_assert!(
        measurement.unit_code.is_none(),
        "unmapped unit {:?} produced unit_code {:?}",
        unit,
        measurement.unit_code
      );
    }
    prop_assert!(
      record.issues.iter().any(|issue| issue.code == "UNKNOWN_MEASUREMENT_UNIT"),
      "unmapped unit {:?} was accepted without a warning",
      unit
    );
  }

  /// Normalization is a pure function of its input. The two-pass parser depends
  /// on this: preflight and the import pass must agree exactly, or the run is
  /// rejected as inconsistent.
  #[test]
  fn normalization_is_deterministic(
    manufacturer in name_strategy(),
    aircraft in name_strategy(),
    speed in "[0-9]{1,4}( (KIAS|KTAS|MPH|none))?",
    year in 1900_i32..2100,
  ) {
    let document = json!({
      "start_year": year,
      "performance": {"best_cruise_speed": speed},
      "title": "generated"
    });
    let first = normalize_record(&manufacturer, &aircraft, document.clone());
    let second = normalize_record(&manufacturer, &aircraft, document);

    prop_assert_eq!(first.source_record_key, second.source_record_key);
    prop_assert_eq!(first.performance.measurements, second.performance.measurements);
    prop_assert_eq!(
      first.issues.iter().map(|issue| issue.code.clone()).collect::<Vec<_>>(),
      second.issues.iter().map(|issue| issue.code.clone()).collect::<Vec<_>>()
    );
  }

  /// Identity depends on the manufacturer and aircraft keys and nothing else,
  /// and distinct pairs must not collide — a collision would let one aircraft
  /// silently overwrite another.
  #[test]
  fn source_record_identity_is_stable_and_distinguishes_inputs(
    left_manufacturer in name_strategy(),
    left_aircraft in name_strategy(),
    right_manufacturer in name_strategy(),
    right_aircraft in name_strategy(),
  ) {
    let left = normalize_record(&left_manufacturer, &left_aircraft, json!({"title": "a"}));
    // Same keys, entirely different body: identity must not move.
    let left_again =
      normalize_record(&left_manufacturer, &left_aircraft, json!({"title": "b", "ceiling": "1"}));
    prop_assert_eq!(&left.source_record_key, &left_again.source_record_key);

    let right = normalize_record(&right_manufacturer, &right_aircraft, json!({"title": "a"}));
    let same_pair =
      left_manufacturer == right_manufacturer && left_aircraft == right_aircraft;
    prop_assert_eq!(
      left.source_record_key == right.source_record_key,
      same_pair,
      "identity collision between {:?}/{:?} and {:?}/{:?}",
      left_manufacturer, left_aircraft, right_manufacturer, right_aircraft
    );
  }

  /// An unrecognized top-level field is preserved in the raw document and
  /// flagged. Ingestion may decline to interpret input, but never to record it.
  #[test]
  fn unrecognized_fields_are_preserved_and_flagged(
    field in "[a-z][a-z_]{2,20}",
    value in "[A-Za-z0-9 ]{1,20}",
    manufacturer in name_strategy(),
    aircraft in name_strategy(),
  ) {
    const KNOWN: &[&str] = &[
      "source_link", "page_url", "title", "description", "papi_price_estimate",
      "for_sale_count", "start_year", "end_year", "in_production", "performance",
      "weights", "ownership_costs", "engine", "images",
    ];
    prop_assume!(!KNOWN.contains(&field.as_str()));

    let expected = Value::String(value.clone());
    let record = normalize_record(&manufacturer, &aircraft, json!({ field.clone(): value }));

    prop_assert_eq!(
      record.raw_document.get(&field),
      Some(&expected),
      "unrecognized field {:?} was dropped from the raw document",
      field
    );
    prop_assert!(
      record.issues.iter().any(|issue| issue.code == "UNSUPPORTED_RECORD_FIELD"),
      "unrecognized field {:?} was accepted without a warning",
      field
    );
  }

  /// Warnings are recoverable by definition: only an error-severity issue may
  /// stop a record, and the codes that do so are a closed, documented set.
  #[test]
  fn only_the_documented_codes_are_error_severity(
    manufacturer in name_strategy(),
    aircraft in name_strategy(),
    start in 1800_i32..2200,
    end in 1800_i32..2200,
  ) {
    const ERROR_CODES: &[&str] = &[
      "RECORD_NOT_OBJECT",
      "MANUFACTURER_NAME_EMPTY",
      "AIRCRAFT_NAME_EMPTY",
      "INVALID_PRODUCTION_YEARS",
    ];
    let record = normalize_record(
      &manufacturer,
      &aircraft,
      json!({
        "start_year": start,
        "end_year": end,
        "in_production": "maybe",
        "page_url": "ftp://example.com/x",
        "performance": {"unmapped_field": "1 ZZZ"}
      }),
    );

    for issue in &record.issues {
      if issue.severity == IssueSeverity::Error {
        prop_assert!(
          ERROR_CODES.contains(&issue.code.as_str()),
          "undocumented error-severity code {:?}",
          issue.code
        );
      }
    }
  }

  /// `parse_numeric` reads a leading number; whatever it returns must itself be
  /// a parseable number. The retired legacy loader's version violated this by
  /// returning a bare decimal tail ('8.5' -> '.5' -> 0.5).
  #[test]
  fn parse_numeric_returns_a_parseable_number_or_nothing(raw in "[-$0-9., A-Za-z]{0,24}") {
    if let Some(parsed) = parse_numeric(&raw) {
      prop_assert!(
        parsed.parse::<f64>().is_ok(),
        "parse_numeric({:?}) returned {:?}, which is not a number",
        raw,
        parsed
      );
    }
  }

  /// A value `parse_sentinel` accepts is never blank, so downstream code can
  /// treat "Some" as "the source said something".
  #[test]
  fn parse_sentinel_never_returns_a_blank_value(raw in "[ A-Za-z0-9/-]{0,16}") {
    if let Some(value) = parse_sentinel(&raw) {
      prop_assert!(!value.trim().is_empty(), "parse_sentinel({:?}) returned blank", raw);
    }
  }
}
