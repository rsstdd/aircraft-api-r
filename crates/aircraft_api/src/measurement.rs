//! The published shape of a measurement.
//!
//! Separate types from `aircraft_domain::measurement` on purpose:
//! `crates/AGENTS.md` keeps domain values and HTTP DTOs as distinct
//! representations even where their shapes coincide, and only this side owns
//! `serde` spellings and `utoipa` metadata. The domain side owns the
//! invariants, so every response value here is built from one that already
//! satisfies them.
//!
//! `docs/architecture/http_v1_decisions.md` § "Exact decimals and measurements"
//! fixes what the wire looks like: decimals are plain base-10 strings, an
//! applicable unit is never omitted, and a missing row is omitted rather than
//! rendered as zero or `null`.

use aircraft_domain::measurement::{
  DecimalString, MeasuredValue, Measurement, MeasurementConditions, PowerSetting,
  PublicationStatus, SurfaceType, UnitCode,
};
use serde::Serialize;
use utoipa::ToSchema;

/// A `NUMERIC` value on the wire: always a JSON string, never a JSON number.
///
/// The published `pattern` is the grammar and deliberately not the capacity:
/// `DecimalString` bounds digits at what `PostgreSQL` can store, but
/// republishing those five-digit quantifiers as a `maxLength` would put a
/// storage detail in the public contract, which the pull-request `oasdiff`
/// gate would then freeze.
#[derive(Debug, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, pattern = "^-?[0-9]+(\\.[0-9]+)?$")]
pub struct DecimalStringResponse(String);

impl From<&DecimalString> for DecimalStringResponse {
  fn from(value: &DecimalString) -> Self {
    Self(value.as_str().to_owned())
  }
}

/// A measurement-unit code on the wire, in the shape
/// `aircraft_ref.lookup_code` requires of every seeded code.
#[derive(Debug, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, pattern = "^[A-Z][A-Z0-9_]*$")]
pub struct UnitCodeResponse(String);

impl From<&UnitCode> for UnitCodeResponse {
  fn from(code: &UnitCode) -> Self {
    Self(code.as_str().to_owned())
  }
}

/// One measured fact.
///
/// Fields are private and the type is built only through [`From`], so no caller
/// can assemble a response the domain constructor would have refused. Raw and
/// canonical units are independent members because conversion changes the unit:
/// a raw value in `METERS` has a canonical counterpart in `FT`.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeasurementResponse {
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  raw_value: Option<DecimalStringResponse>,
  /// Absent when the metric is dimensionless, which is the only reason an
  /// applicable unit is ever missing.
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  raw_unit_code: Option<UnitCodeResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  canonical_value: Option<DecimalStringResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  canonical_unit_code: Option<UnitCodeResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  conditions: Option<MeasurementConditionsResponse>,
  /// Whether this row is the one served through the canonical read model.
  /// Absent for a fact table that carries no such column at all; `false` says
  /// only that the row is unpublished, never why.
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  is_canonical: Option<bool>,
}

impl From<&Measurement> for MeasurementResponse {
  fn from(measurement: &Measurement) -> Self {
    let (raw_value, raw_unit_code) = split(measurement.raw());
    let (canonical_value, canonical_unit_code) = split(measurement.canonical());
    Self {
      raw_value,
      raw_unit_code,
      canonical_value,
      canonical_unit_code,
      conditions: measurement.conditions().map(MeasurementConditionsResponse::from),
      is_canonical: measurement.publication().map(PublicationStatus::is_canonical),
    }
  }
}

/// Splits a measured value into its two wire members. The unit member is absent
/// exactly when the domain says the metric has none.
fn split(
  value: Option<&MeasuredValue>,
) -> (Option<DecimalStringResponse>, Option<UnitCodeResponse>) {
  value.map_or((None, None), |value| {
    (Some(DecimalStringResponse::from(value.value())), value.unit().map(UnitCodeResponse::from))
  })
}

/// The conditions a measurement was taken under. Present only when the source
/// supplied at least one; the domain constructor drops an empty set.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeasurementConditionsResponse {
  /// Pressure altitude in feet.
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  altitude_ft: Option<DecimalStringResponse>,
  /// Aircraft weight in pounds.
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  weight_lbs: Option<DecimalStringResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  weight_label: Option<String>,
  /// Deviation from ISA in degrees Celsius.
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  isa_deviation_c: Option<DecimalStringResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  power_setting: Option<PowerSettingResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  surface_type: Option<SurfaceTypeResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  #[schema(nullable = false)]
  notes: Option<String>,
}

impl From<&MeasurementConditions> for MeasurementConditionsResponse {
  fn from(conditions: &MeasurementConditions) -> Self {
    Self {
      altitude_ft: conditions.altitude_ft.as_ref().map(DecimalStringResponse::from),
      weight_lbs: conditions.weight_lbs.as_ref().map(DecimalStringResponse::from),
      weight_label: conditions.weight_label.clone(),
      isa_deviation_c: conditions.isa_deviation_c.as_ref().map(DecimalStringResponse::from),
      power_setting: conditions.power_setting.map(PowerSettingResponse::from),
      surface_type: conditions.surface_type.map(SurfaceTypeResponse::from),
      notes: conditions.notes.clone(),
    }
  }
}

/// The published spelling of an engine setting.
///
/// The variant names are Rust's; the `rename` values are the literals
/// `chk_pm_power_setting` allows in
/// `database/migrations/008_performance_metrics_conditions.sql`, which is why
/// three of them are not valid Rust identifiers.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub enum PowerSettingResponse {
  #[serde(rename = "MAX_TAKEOFF")]
  MaxTakeoff,
  #[serde(rename = "MAX_CONTINUOUS")]
  MaxContinuous,
  #[serde(rename = "MAX_CLIMB")]
  MaxClimb,
  #[serde(rename = "75_PCT")]
  Percent75,
  #[serde(rename = "65_PCT")]
  Percent65,
  #[serde(rename = "55_PCT")]
  Percent55,
  #[serde(rename = "BEST_POWER")]
  BestPower,
  #[serde(rename = "BEST_ECONOMY")]
  BestEconomy,
  #[serde(rename = "LONG_RANGE_CRUISE")]
  LongRangeCruise,
  #[serde(rename = "IDLE")]
  Idle,
}

/// No wildcard arm: an eleventh `PowerSetting` does not compile until it has a
/// published spelling, the way `Scope` maps into `RequiredScope`.
impl From<PowerSetting> for PowerSettingResponse {
  fn from(setting: PowerSetting) -> Self {
    match setting {
      PowerSetting::MaxTakeoff => Self::MaxTakeoff,
      PowerSetting::MaxContinuous => Self::MaxContinuous,
      PowerSetting::MaxClimb => Self::MaxClimb,
      PowerSetting::Percent75 => Self::Percent75,
      PowerSetting::Percent65 => Self::Percent65,
      PowerSetting::Percent55 => Self::Percent55,
      PowerSetting::BestPower => Self::BestPower,
      PowerSetting::BestEconomy => Self::BestEconomy,
      PowerSetting::LongRangeCruise => Self::LongRangeCruise,
      PowerSetting::Idle => Self::Idle,
    }
  }
}

/// The published spelling of a runway surface, from `chk_pm_surface_type` in
/// the same migration.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SurfaceTypeResponse {
  Paved,
  Grass,
  Gravel,
  Soft,
  Water,
  CarrierDeck,
}

/// No wildcard arm, for the reason [`PowerSettingResponse`]'s mapping gives.
impl From<SurfaceType> for SurfaceTypeResponse {
  fn from(surface: SurfaceType) -> Self {
    match surface {
      SurfaceType::Paved => Self::Paved,
      SurfaceType::Grass => Self::Grass,
      SurfaceType::Gravel => Self::Gravel,
      SurfaceType::Soft => Self::Soft,
      SurfaceType::Water => Self::Water,
      SurfaceType::CarrierDeck => Self::CarrierDeck,
    }
  }
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use serde_json::json;

  use super::*;

  fn decimal(text: &str) -> DecimalString {
    DecimalString::parse(text).expect("test input is a plain decimal")
  }

  fn in_unit(value: &str, unit: &str) -> MeasuredValue {
    MeasuredValue::InUnit {
      value: decimal(value),
      unit: UnitCode::parse(unit).expect("test input is a lookup code"),
    }
  }

  fn document(measurement: &Measurement) -> serde_json::Value {
    serde_json::to_value(MeasurementResponse::from(measurement)).expect("the DTO serializes")
  }

  #[test]
  fn a_measurement_publishes_its_decimals_as_exact_strings() {
    let measurement = Measurement::new(
      Some(in_unit("11.900", "METERS")),
      Some(in_unit("39.0420000000", "FT")),
      Some(PublicationStatus::Published),
      None,
    )
    .expect("the pair is unitful on both sides");

    assert_eq!(
      document(&measurement),
      json!({
        "raw_value": "11.900",
        "raw_unit_code": "METERS",
        "canonical_value": "39.0420000000",
        "canonical_unit_code": "FT",
        "is_canonical": true
      }),
      "decimals are JSON strings with their scale intact, and each side names its own unit"
    );
  }

  #[test]
  fn an_absent_member_is_omitted_rather_than_published_as_null() {
    let raw_only = Measurement::new(Some(in_unit("122", "KTAS")), None, None, None)
      .expect("a raw value is present");

    assert_eq!(
      document(&raw_only),
      json!({ "raw_value": "122", "raw_unit_code": "KTAS" }),
      "no canonical members, no conditions, and no is_canonical key at all"
    );
  }

  #[test]
  fn a_dimensionless_measurement_publishes_no_unit_member() {
    let dimensionless = Measurement::new(
      Some(MeasuredValue::Dimensionless(decimal("3.8"))),
      None,
      Some(PublicationStatus::NotPublished),
      None,
    )
    .expect("a raw value is present");

    assert_eq!(
      document(&dimensionless),
      json!({ "raw_value": "3.8", "is_canonical": false }),
      "absence is how the wire says a metric has no applicable unit"
    );
  }

  #[test]
  fn conditions_publish_exactly_what_the_source_supplied() {
    let measurement = Measurement::new(
      Some(in_unit("1450", "FT")),
      None,
      None,
      Some(MeasurementConditions {
        altitude_ft: Some(decimal("8000")),
        isa_deviation_c: Some(decimal("-20")),
        power_setting: Some(PowerSetting::Percent75),
        surface_type: Some(SurfaceType::CarrierDeck),
        ..MeasurementConditions::default()
      }),
    )
    .expect("a raw value is present");

    assert_eq!(
      document(&measurement),
      json!({
        "raw_value": "1450",
        "raw_unit_code": "FT",
        "conditions": {
          "altitude_ft": "8000",
          "isa_deviation_c": "-20",
          "power_setting": "75_PCT",
          "surface_type": "CARRIER_DECK"
        }
      }),
      "supplied conditions survive; unsupplied ones leave no key"
    );
  }

  /// The mapping, not the vocabulary: the generated document pins the enum
  /// lists, but only this pins which domain variant becomes which spelling, so
  /// two variants swapped in a `From` impl fail here and nowhere else.
  ///
  /// The expected spelling is the domain's own `code()` rather than a literal
  /// written out again, and
  /// `the_condition_vocabularies_are_the_ones_the_check_constraints_allow` pins
  /// `code()` to `chk_pm_power_setting` and `chk_pm_surface_type` in migration
  /// `008`. The chain therefore ends at the schema instead of at a copy.
  #[test]
  fn every_domain_condition_code_maps_to_the_spelling_the_schema_allows() {
    for setting in PowerSetting::ALL {
      let published = serde_json::to_value(PowerSettingResponse::from(setting))
        .expect("the transport enum serializes");
      assert_eq!(published, json!(setting.code()), "{setting:?}");
    }
    for surface in SurfaceType::ALL {
      let published = serde_json::to_value(SurfaceTypeResponse::from(surface))
        .expect("the transport enum serializes");
      assert_eq!(published, json!(surface.code()), "{surface:?}");
    }
  }
}
