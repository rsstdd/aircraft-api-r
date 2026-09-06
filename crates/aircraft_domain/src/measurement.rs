//! Exact decimals and the shared read-side measurement representation.
//!
//! `docs/architecture/http_v1_decisions.md` § "Exact decimals and measurements"
//! is the accepted contract these types implement: `PostgreSQL` `NUMERIC` values
//! cross application and HTTP boundaries as validated plain base-10 decimal
//! strings, never as JSON numbers, binary floating point, or scientific
//! notation; an applicable unit is never omitted; and a missing database row is
//! omitted rather than turned into zero or "not applicable".
//!
//! These are the *read* side. `aircraft_app::services::ingestion::MeasurementInput`
//! is the source-write contract and stays separate, as `crates/AGENTS.md`
//! requires of source records, domain values, and HTTP DTOs.
//!
//! Conversion is not modelled here. The decision document forbids a second
//! unit-conversion registry in Rust: `aircraft_ref.to_canonical` and
//! `aircraft_ref.measurement_units` own it.

use thiserror::Error;

/// Integer digits an unconstrained `PostgreSQL` `NUMERIC` can hold.
///
/// The ceiling is the database's, not a domain judgment: anything narrower
/// would refuse a value the canonical schema accepts, turning a legal stored
/// row into a mapping failure. `database/migrations/006`, `007`, and `008`
/// declare their measurement columns as unconstrained `NUMERIC`.
pub const MAX_INTEGER_DIGITS: usize = 131_072;

/// Fractional digits an unconstrained `PostgreSQL` `NUMERIC` can hold. See
/// [`MAX_INTEGER_DIGITS`] for why the database's ceiling is the one used.
pub const MAX_FRACTIONAL_DIGITS: usize = 16_383;

/// A validated plain base-10 decimal, preserved exactly as it arrived.
///
/// Trailing zeros are scale and therefore meaning, so nothing here normalizes,
/// trims, or re-renders: the bytes handed in are the bytes handed back. The
/// write path already decided the stored scale with `trim_scale(...)` in
/// `aircraft_db::repositories::ingestion_repository`.
///
/// **Equality is textual, not numeric.** `1.5` and `1.50` are the same quantity
/// and different values of this type, because preserving the distinction is the
/// reason the type exists. Comparing magnitudes is a job for SQL, where the
/// values are `NUMERIC`; two of these being equal means the database rendered
/// them identically, and nothing more.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecimalString(String);

impl DecimalString {
  /// Accepts an optional leading `-`, at least one ASCII digit, and at most one
  /// `.` followed by at least one ASCII digit.
  ///
  /// Rejects exponent notation, `+`, whitespace, digit separators, a bare sign,
  /// a bare point on either side, and non-ASCII digits — every form the
  /// accepted decision rules off the wire.
  ///
  /// # Errors
  ///
  /// [`DecimalSyntaxError::NotPlainDecimal`] when the text is not that grammar,
  /// and [`DecimalSyntaxError::TooManyDigits`] when either side exceeds what
  /// `PostgreSQL` can store. Both are unit variants, so rendering one cannot
  /// echo the input back into a log or a response.
  pub fn parse(text: &str) -> Result<Self, DecimalSyntaxError> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (integer, fraction) = match unsigned.split_once('.') {
      Some((integer, fraction)) => (integer, Some(fraction)),
      None => (unsigned, None),
    };

    digits_within(integer, MAX_INTEGER_DIGITS)?;
    if let Some(fraction) = fraction {
      digits_within(fraction, MAX_FRACTIONAL_DIGITS)?;
    }

    Ok(Self(text.to_owned()))
  }

  #[must_use]
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

/// A run of ASCII digits, non-empty and within `max`.
///
/// `len()` is the digit count because the run is ASCII by the time it is
/// measured. A second `.` lands in `fraction` and fails the digit test, which
/// is what refuses `1.2.3` without a separate pass.
fn digits_within(part: &str, max: usize) -> Result<(), DecimalSyntaxError> {
  if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
    return Err(DecimalSyntaxError::NotPlainDecimal);
  }
  if part.len() > max {
    return Err(DecimalSyntaxError::TooManyDigits);
  }
  Ok(())
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum DecimalSyntaxError {
  #[error("value is not a plain base-ten decimal")]
  NotPlainDecimal,
  #[error("value has more digits than the database can store")]
  TooManyDigits,
}

/// A measurement-unit code, in the shape `aircraft_ref.lookup_code` requires.
///
/// Mirrors `CREATE DOMAIN aircraft_ref.lookup_code AS TEXT CHECK (VALUE ~
/// '^[A-Z][A-Z0-9_]*$')` in
/// `database/migrations/001_extensions_schemas_domains_triggers.sql`;
/// `database/data_dictionary.md` names this type in turn.
///
/// Shape only. Which codes exist is reference data in
/// `aircraft_ref.measurement_units`, seeded by
/// `database/seeds/001_reference_units.sql`, and the accepted decision forbids
/// a second copy of that registry in Rust. No length bound is imposed either:
/// the SQL domain constrains syntax and not length, and inventing one here
/// would be a schema restriction that no migration declares.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitCode(String);

impl UnitCode {
  /// # Errors
  ///
  /// [`UnitCodeSyntaxError`] when the code is empty, does not begin with an
  /// ASCII uppercase letter, or contains anything but uppercase letters,
  /// digits, and underscores.
  pub fn parse(code: &str) -> Result<Self, UnitCodeSyntaxError> {
    let mut characters = code.chars();
    let starts_upper = characters.next().is_some_and(|first| first.is_ascii_uppercase());
    if starts_upper
      && characters.all(|rest| rest.is_ascii_uppercase() || rest.is_ascii_digit() || rest == '_')
    {
      Ok(Self(code.to_owned()))
    } else {
      Err(UnitCodeSyntaxError)
    }
  }

  #[must_use]
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("unit code is not in the lookup-code shape")]
pub struct UnitCodeSyntaxError;

/// A decimal together with the unit it is expressed in, or the explicit
/// statement that the metric has none.
///
/// The two cases are separate variants so a value whose applicable unit was
/// merely forgotten cannot be constructed. `database/migrations/007` documents
/// the unitless case for weight metrics — "Source unit code. NULL for
/// dimensionless metrics" — and is the only migration that says what a NULL
/// `raw_unit_code` means. Migrations `006` and `008` declare the same nullable
/// column without documenting it, so a mapper for a dimension or performance
/// fact must establish the reading for its own table rather than borrowing this
/// one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeasuredValue {
  Dimensionless(DecimalString),
  InUnit { value: DecimalString, unit: UnitCode },
}

impl MeasuredValue {
  #[must_use]
  pub const fn value(&self) -> &DecimalString {
    match self {
      Self::Dimensionless(value) | Self::InUnit { value, .. } => value,
    }
  }

  #[must_use]
  pub const fn unit(&self) -> Option<&UnitCode> {
    match self {
      Self::Dimensionless(_) => None,
      Self::InUnit { unit, .. } => Some(unit),
    }
  }

  const fn is_dimensionless(&self) -> bool {
    matches!(self, Self::Dimensionless(_))
  }
}

/// Whether a fact row is the one served through the canonical read model.
///
/// `database/migrations/019_weight_metrics_curation_gate.sql` defines the
/// column it mirrors: "TRUE = this row is served through
/// `aircraft_read.mv_variant_search`". It deliberately says nothing about *why*
/// a row is unpublished: curation writes `is_canonical = false` both for a
/// rejected assertion and for one never decided, so the row cannot distinguish
/// pending from rejected and neither can this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationStatus {
  Published,
  NotPublished,
}

impl PublicationStatus {
  #[must_use]
  pub const fn from_is_canonical(is_canonical: bool) -> Self {
    if is_canonical { Self::Published } else { Self::NotPublished }
  }

  #[must_use]
  pub const fn is_canonical(self) -> bool {
    matches!(self, Self::Published)
  }
}

/// Test conditions a published measurement was taken under.
///
/// Every field is optional because every source column is, and because sources
/// differ in how much they publish. Mirrors the condition block of
/// `database/migrations/008_performance_metrics_conditions.sql`, the only table
/// in the schema that carries conditions at all.
///
/// The three numeric fields carry their unit in the column name — `NUMERIC`
/// columns named `condition_altitude_ft`, `condition_weight_lbs`, and
/// `condition_isa_dev_c` — so their unit is fixed by the schema rather than by
/// an accompanying [`UnitCode`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MeasurementConditions {
  /// Pressure altitude in feet.
  pub altitude_ft: Option<DecimalString>,
  /// Aircraft weight in pounds.
  pub weight_lbs: Option<DecimalString>,
  /// Free text, such as `MTOW` or `HALF_FUEL`; migration `008` deliberately
  /// leaves it unconstrained.
  pub weight_label: Option<String>,
  /// Deviation from ISA in degrees Celsius; zero is standard ISA.
  pub isa_deviation_c: Option<DecimalString>,
  pub power_setting: Option<PowerSetting>,
  pub surface_type: Option<SurfaceType>,
  pub notes: Option<String>,
}

impl MeasurementConditions {
  #[must_use]
  pub const fn is_empty(&self) -> bool {
    self.altitude_ft.is_none()
      && self.weight_lbs.is_none()
      && self.weight_label.is_none()
      && self.isa_deviation_c.is_none()
      && self.power_setting.is_none()
      && self.surface_type.is_none()
      && self.notes.is_none()
  }
}

/// The engine settings `chk_pm_power_setting` allows.
///
/// The ten spellings are the CHECK constraint's literals in
/// `database/migrations/008_performance_metrics_conditions.sql`, which migration
/// documents as a "small, stable, definitionally complete" set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerSetting {
  MaxTakeoff,
  MaxContinuous,
  MaxClimb,
  Percent75,
  Percent65,
  Percent55,
  BestPower,
  BestEconomy,
  LongRangeCruise,
  Idle,
}

impl PowerSetting {
  /// Every variant, in the order `chk_pm_power_setting` declares its literals.
  /// `TryFrom`, the test that reads that constraint, and `aircraft_api`'s
  /// transport mapping all iterate this one list.
  pub const ALL: [Self; 10] = [
    Self::MaxTakeoff,
    Self::MaxContinuous,
    Self::MaxClimb,
    Self::Percent75,
    Self::Percent65,
    Self::Percent55,
    Self::BestPower,
    Self::BestEconomy,
    Self::LongRangeCruise,
    Self::Idle,
  ];

  #[must_use]
  pub const fn code(self) -> &'static str {
    match self {
      Self::MaxTakeoff => "MAX_TAKEOFF",
      Self::MaxContinuous => "MAX_CONTINUOUS",
      Self::MaxClimb => "MAX_CLIMB",
      Self::Percent75 => "75_PCT",
      Self::Percent65 => "65_PCT",
      Self::Percent55 => "55_PCT",
      Self::BestPower => "BEST_POWER",
      Self::BestEconomy => "BEST_ECONOMY",
      Self::LongRangeCruise => "LONG_RANGE_CRUISE",
      Self::Idle => "IDLE",
    }
  }
}

impl TryFrom<&str> for PowerSetting {
  type Error = UnknownConditionCode;

  fn try_from(code: &str) -> Result<Self, Self::Error> {
    Self::ALL.into_iter().find(|setting| setting.code() == code).ok_or(UnknownConditionCode)
  }
}

/// The runway surfaces `chk_pm_surface_type` allows, from the same migration
/// and under the same closed-vocabulary reasoning as [`PowerSetting`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceType {
  Paved,
  Grass,
  Gravel,
  Soft,
  Water,
  CarrierDeck,
}

impl SurfaceType {
  /// Every variant, in the order `chk_pm_surface_type` declares its literals,
  /// for the reason [`PowerSetting::ALL`] gives.
  pub const ALL: [Self; 6] =
    [Self::Paved, Self::Grass, Self::Gravel, Self::Soft, Self::Water, Self::CarrierDeck];

  #[must_use]
  pub const fn code(self) -> &'static str {
    match self {
      Self::Paved => "PAVED",
      Self::Grass => "GRASS",
      Self::Gravel => "GRAVEL",
      Self::Soft => "SOFT",
      Self::Water => "WATER",
      Self::CarrierDeck => "CARRIER_DECK",
    }
  }
}

impl TryFrom<&str> for SurfaceType {
  type Error = UnknownConditionCode;

  fn try_from(code: &str) -> Result<Self, Self::Error> {
    Self::ALL.into_iter().find(|surface| surface.code() == code).ok_or(UnknownConditionCode)
  }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("condition code is not one the schema allows")]
pub struct UnknownConditionCode;

/// One measured fact: what the source said, what it converts to, whether it is
/// published, and the conditions it was taken under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Measurement {
  raw: Option<MeasuredValue>,
  canonical: Option<MeasuredValue>,
  publication: Option<PublicationStatus>,
  conditions: Option<MeasurementConditions>,
}

impl Measurement {
  /// # Errors
  ///
  /// [`MeasurementError::NoValue`] when neither a raw nor a canonical value is
  /// present, so a missing row can never become an empty measurement rather
  /// than nothing at all, and [`MeasurementError::MixedDimensionality`] when one
  /// of the pair carries a unit and the other does not. Migration `007` states
  /// the rule the second guard enforces: "For dimensionless metrics
  /// (`LOAD_FACTOR_*`, `WING_LOADING`): equals `raw_value`."
  ///
  /// `publication` is `None` for a table with no gate column at all —
  /// `aircraft_specs.dimension_metrics` has none — which is a different fact
  /// from a row that has one and is unpublished.
  pub fn new(
    raw: Option<MeasuredValue>,
    canonical: Option<MeasuredValue>,
    publication: Option<PublicationStatus>,
    conditions: Option<MeasurementConditions>,
  ) -> Result<Self, MeasurementError> {
    if raw.is_none() && canonical.is_none() {
      return Err(MeasurementError::NoValue);
    }
    // Not a let-chain: those stabilized in 1.88 and the workspace MSRV is 1.85.
    if let (Some(raw), Some(canonical)) = (raw.as_ref(), canonical.as_ref()) {
      if raw.is_dimensionless() != canonical.is_dimensionless() {
        return Err(MeasurementError::MixedDimensionality);
      }
    }

    // An all-empty condition set is the absence of conditions. Keeping it would
    // publish a `{}` that says a measurement has conditions and names none.
    let conditions = conditions.filter(|conditions| !conditions.is_empty());

    Ok(Self { raw, canonical, publication, conditions })
  }

  #[must_use]
  pub const fn raw(&self) -> Option<&MeasuredValue> {
    self.raw.as_ref()
  }

  #[must_use]
  pub const fn canonical(&self) -> Option<&MeasuredValue> {
    self.canonical.as_ref()
  }

  #[must_use]
  pub const fn publication(&self) -> Option<PublicationStatus> {
    self.publication
  }

  #[must_use]
  pub const fn conditions(&self) -> Option<&MeasurementConditions> {
    self.conditions.as_ref()
  }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum MeasurementError {
  #[error("a measurement carries neither a raw nor a canonical value")]
  NoValue,
  #[error("one of the raw and canonical pair has a unit and the other does not")]
  MixedDimensionality,
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, and the migration reader below
  // fails by panicking when the constraint it names is gone.
  #![allow(clippy::expect_used, clippy::panic)]

  use super::*;

  fn decimal(text: &str) -> DecimalString {
    DecimalString::parse(text).expect("test input is a plain decimal")
  }

  fn knots(text: &str) -> MeasuredValue {
    MeasuredValue::InUnit {
      value: decimal(text),
      unit: UnitCode::parse("KTAS").expect("KTAS is a lookup code"),
    }
  }

  #[test]
  fn a_high_precision_decimal_is_preserved_byte_for_byte() {
    // Longer than the 128 characters an earlier draft would have capped, and
    // carrying trailing zeros that a normalizing implementation would drop.
    let text = "-12345678901234567890123456789012345678901234567890.0980000";

    let parsed = decimal(text);

    assert_eq!(parsed.as_str(), text, "no normalization, rounding, or re-rendering");
  }

  #[test]
  fn only_a_plain_base_ten_decimal_is_accepted() {
    // Expected values come from `http_v1_decisions.md` § "Exact decimals and
    // measurements", not from the parser: exponent notation and binary floating
    // point are named there as forms that must never appear.
    const CASES: [(&str, Option<DecimalSyntaxError>); 16] = [
      ("0", None),
      ("-0", None),
      ("1.500", None),
      ("0.0000000001", None),
      ("", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("-", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("+1", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("1e5", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("1E5", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("NaN", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("Infinity", Some(DecimalSyntaxError::NotPlainDecimal)),
      (" 1", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("1,000", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("1.2.3", Some(DecimalSyntaxError::NotPlainDecimal)),
      (".5", Some(DecimalSyntaxError::NotPlainDecimal)),
      ("5.", Some(DecimalSyntaxError::NotPlainDecimal)),
    ];

    for (input, expected) in CASES {
      assert_eq!(DecimalString::parse(input).err(), expected, "input {input:?}");
    }
    // Arabic-Indic digits are `char::is_numeric` but not `is_ascii_digit`, so
    // this is the case a `is_numeric` implementation would wrongly accept.
    assert_eq!(
      DecimalString::parse("١٢٣").err(),
      Some(DecimalSyntaxError::NotPlainDecimal),
      "non-ASCII digits"
    );
  }

  #[test]
  fn a_decimal_is_bounded_by_what_postgres_can_store_rather_than_by_taste() {
    let widest = "9".repeat(MAX_INTEGER_DIGITS);
    let deepest = format!("0.{}", "9".repeat(MAX_FRACTIONAL_DIGITS));

    assert!(DecimalString::parse(&widest).is_ok(), "the widest storable integer is legal");
    assert!(DecimalString::parse(&deepest).is_ok(), "the deepest storable scale is legal");
    assert_eq!(
      DecimalString::parse(&format!("{widest}9")).err(),
      Some(DecimalSyntaxError::TooManyDigits)
    );
    assert_eq!(
      DecimalString::parse(&format!("{deepest}9")).err(),
      Some(DecimalSyntaxError::TooManyDigits),
      "the fractional side is bounded separately, not by the total length"
    );
  }

  #[test]
  fn a_unit_code_must_have_the_shape_the_lookup_code_domain_requires() {
    // Accepted spellings are seeded rows of `aircraft_ref.measurement_units`;
    // the rejections are what `^[A-Z][A-Z0-9_]*$` in migration 001 refuses.
    const CASES: [(&str, bool); 8] = [
      ("KTAS", true),
      ("DEG_C", true),
      ("US_GAL", true),
      ("", false),
      ("ktas", false),
      ("75_PCT", false),
      ("KT-AS", false),
      ("_KTAS", false),
    ];

    for (input, accepted) in CASES {
      assert_eq!(UnitCode::parse(input).is_ok(), accepted, "input {input:?}");
    }
  }

  #[test]
  fn a_dimensionless_value_and_a_unitful_one_are_distinct_cases() {
    let dimensionless = MeasuredValue::Dimensionless(decimal("3.8"));
    let unitful = knots("122");

    assert_eq!(dimensionless.unit(), None, "a dimensionless metric names no unit");
    assert_eq!(
      unitful.unit().map(UnitCode::as_str),
      Some("KTAS"),
      "a unitful value cannot be built without its code"
    );
    assert_eq!(dimensionless.value().as_str(), "3.8");
  }

  #[test]
  fn a_measurement_without_any_value_is_refused() {
    assert_eq!(Measurement::new(None, None, None, None).err(), Some(MeasurementError::NoValue));
    assert!(
      Measurement::new(Some(knots("122")), None, None, None).is_ok(),
      "a raw-only fact is ordinary, not empty"
    );
    assert!(
      Measurement::new(None, Some(knots("122")), None, None).is_ok(),
      "a canonical-only fact is ordinary, not empty"
    );
  }

  #[test]
  fn a_raw_and_canonical_pair_must_agree_on_having_a_unit() {
    let mixed = Measurement::new(
      Some(MeasuredValue::Dimensionless(decimal("3.8"))),
      Some(knots("122")),
      None,
      None,
    );

    assert_eq!(mixed.err(), Some(MeasurementError::MixedDimensionality));
    assert!(
      Measurement::new(
        Some(MeasuredValue::Dimensionless(decimal("3.8"))),
        Some(MeasuredValue::Dimensionless(decimal("3.8"))),
        None,
        None,
      )
      .is_ok(),
      "a dimensionless pair is what migration 007 describes"
    );
  }

  #[test]
  fn conditions_survive_only_when_the_source_supplied_some() {
    let empty =
      Measurement::new(Some(knots("122")), None, None, Some(MeasurementConditions::default()))
        .expect("a raw value is present");
    let partial = Measurement::new(
      Some(knots("122")),
      None,
      None,
      Some(MeasurementConditions {
        altitude_ft: Some(decimal("8000")),
        ..MeasurementConditions::default()
      }),
    )
    .expect("a raw value is present");

    assert_eq!(empty.conditions(), None, "an all-empty condition set is no conditions");
    assert_eq!(
      partial.conditions().and_then(|conditions| conditions.altitude_ft.as_ref()),
      Some(&decimal("8000")),
      "a partial set keeps exactly what was supplied"
    );
    assert_eq!(
      partial.conditions().and_then(|conditions| conditions.power_setting),
      None,
      "and invents nothing for what was not"
    );
  }

  #[test]
  fn an_absent_publication_gate_is_not_an_unpublished_one() {
    let ungated = Measurement::new(Some(knots("122")), None, None, None).expect("has a value");
    let unpublished =
      Measurement::new(Some(knots("122")), None, Some(PublicationStatus::NotPublished), None)
        .expect("has a value");

    assert_eq!(
      ungated.publication(),
      None,
      "aircraft_specs.dimension_metrics carries no is_canonical column"
    );
    assert_eq!(unpublished.publication(), Some(PublicationStatus::NotPublished));
    assert!(!PublicationStatus::NotPublished.is_canonical());
    assert!(PublicationStatus::from_is_canonical(true).is_canonical());
  }

  /// The migration itself, so the expectations below are the schema's own
  /// literals rather than a second copy of `code()`. `include_str!` is
  /// compile-time and confined to this test module, which is how
  /// `aircraft_testsupport::SCHEMA_STEPS` embeds the canonical install order.
  const MIGRATION_008: &str =
    include_str!("../../../database/migrations/008_performance_metrics_conditions.sql");

  /// The quoted literals of one named `CHECK`, in declaration order.
  ///
  /// The slice runs from the constraint's opening line to the next four-space
  /// `)`, which is its own close; inside it the only single-quoted tokens are
  /// the allowlist values, since the trailing `--` comments carry no
  /// apostrophes. Panics rather than returning an empty list when the
  /// constraint is absent, so a renamed constraint fails loudly instead of
  /// making the assertions below vacuous.
  fn codes_in_declaration_order(constraint: &str) -> Vec<&str> {
    let opening = format!("CONSTRAINT {constraint} CHECK (");
    let body = MIGRATION_008
      .split_once(&opening)
      .unwrap_or_else(|| panic!("migration 008 declares no {constraint}"))
      .1;
    let body = body
      .split_once("\n    )")
      .unwrap_or_else(|| panic!("{constraint} is not closed as expected"))
      .0;
    body.split('\'').skip(1).step_by(2).collect()
  }

  /// Reads both vocabularies against `chk_pm_power_setting` and
  /// `chk_pm_surface_type` in migration `008` — the constraints that decide
  /// what may be stored — rather than against a transcription of `code()`.
  ///
  /// Matching is positional, which set membership could not do: two variants
  /// whose codes were swapped leave the set unchanged and still round-trip
  /// through `try_from`, and would publish a 65 %-power measurement as
  /// `55_PCT`. Position is a stable key because an applied migration is
  /// immutable once hashed in `database/migrations.lock.json`.
  #[test]
  fn the_condition_vocabularies_are_the_ones_the_check_constraints_allow() {
    let power = codes_in_declaration_order("chk_pm_power_setting");
    let surface = codes_in_declaration_order("chk_pm_surface_type");

    assert_eq!(power.len(), PowerSetting::ALL.len(), "power settings: {power:?}");
    assert_eq!(surface.len(), SurfaceType::ALL.len(), "surface types: {surface:?}");

    for (setting, code) in PowerSetting::ALL.into_iter().zip(power) {
      assert_eq!(setting.code(), code, "{setting:?} against migration 008");
      assert_eq!(PowerSetting::try_from(code), Ok(setting), "{code}");
    }
    for (surface_type, code) in SurfaceType::ALL.into_iter().zip(surface) {
      assert_eq!(surface_type.code(), code, "{surface_type:?} against migration 008");
      assert_eq!(SurfaceType::try_from(code), Ok(surface_type), "{code}");
    }
    assert_eq!(PowerSetting::try_from("CLIMB_POWER"), Err(UnknownConditionCode));
    assert_eq!(SurfaceType::try_from("ICE"), Err(UnknownConditionCode));
  }
}
