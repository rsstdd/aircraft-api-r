use aircraft_app::ingestion::{
  AircraftIdentityInput, CostItemInput, ImageMetadataInput, IngestIssue, IssueSeverity,
  LifecycleInput, MeasurementInput, OperatingCostInput, PerformanceInput, PreparedAircraftRecord,
  PropulsionInput, ProvenanceInput, ValuationInput, WeightInput,
};
use aircraft_domain::ingestion::{Confidence, ProductionYears};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use url::Url;

use crate::artifact::hex_digest;

const KNOWN_FIELDS: &[&str] = &[
  // Identity, carried inside the record as well as in the enclosing map keys the
  // parser reads. Consumed, not ignored: omitting them here made every record in
  // the published file warn twice and so never disposition clean.
  "manufacturer_name",
  "aircraft_name",
  "source_link",
  "page_url",
  "title",
  "description",
  "papi_price_estimate",
  "for_sale_count",
  "start_year",
  "end_year",
  "in_production",
  "performance",
  "weights",
  "ownership_costs",
  "engine",
  "images",
];

#[allow(clippy::too_many_lines)]
pub fn normalize_record(manufacturer: &str, aircraft: &str, raw: Value) -> PreparedAircraftRecord {
  let mut issues = Vec::new();
  let Some(object) = raw.as_object() else {
    issues.push(issue(
      "RECORD_NOT_OBJECT",
      IssueSeverity::Error,
      "$",
      "aircraft record must be a JSON object",
      Some(&raw),
    ));
    return empty_record(manufacturer, aircraft, raw, issues);
  };
  if manufacturer.trim().is_empty() {
    issues.push(issue(
      "MANUFACTURER_NAME_EMPTY",
      IssueSeverity::Error,
      "$.<manufacturer>",
      "manufacturer key must not be empty",
      None,
    ));
  }
  if aircraft.trim().is_empty() {
    issues.push(issue(
      "AIRCRAFT_NAME_EMPTY",
      IssueSeverity::Error,
      "$.<manufacturer>.<aircraft>",
      "aircraft key must not be empty",
      None,
    ));
  }
  for (field, value) in object {
    if !KNOWN_FIELDS.contains(&field.as_str()) {
      issues.push(issue(
        "UNSUPPORTED_RECORD_FIELD",
        IssueSeverity::Warning,
        field,
        "field is preserved in raw JSON but is not promoted",
        Some(value),
      ));
    }
  }

  let description = scalar(object.get("description"), "description", &mut issues);
  // Read before `description` is moved into the identity below, and once rather
  // than per use: the match lower-cases the whole string.
  // Lower-cased once: both helpers match case-insensitively on the same text, and
  // this runs for every record of every import.
  let prose = description.as_deref().map(str::to_ascii_lowercase);
  let propulsion = prose.as_deref().and_then(propulsion_category).map(str::to_owned);
  let landing_gear = prose.as_deref().and_then(landing_gear_type);
  // A feed that states the years outranks one that only spells them in a name.
  // No record in the published PlanePHD file carries either field, so in practice
  // the name is the only source; a later feed that adds them wins without a code
  // change.
  let (named_start, named_end, named_in_production) = production_range(aircraft, &mut issues);
  let in_production = boolean(object, "in_production", &mut issues).or(named_in_production);
  let start = integer::<i16>(object, "start_year", &mut issues).or(named_start);
  let end = integer::<i16>(object, "end_year", &mut issues).or(named_end);
  if let Err(error) = ProductionYears::new(start, end) {
    issues.push(issue(
      "INVALID_PRODUCTION_YEARS",
      IssueSeverity::Error,
      "production_years",
      &error.to_string(),
      None,
    ));
  }
  let (passengers, crew) = description.as_deref().map_or((None, None), occupants);

  let perf_object = nested(object, "performance", &mut issues);
  let weight_object = nested(object, "weights", &mut issues);
  let engine = nested(object, "engine", &mut issues);
  let costs = nested(object, "ownership_costs", &mut issues);
  let performance = measurements(perf_object, "performance", perf_code, &mut issues);
  let weights = measurements(weight_object, "weights", weight_code, &mut issues);

  let source_link = scalar(object.get("source_link"), "source_link", &mut issues);
  let page_url = scalar(object.get("page_url"), "page_url", &mut issues)
    .and_then(|value| normalized_url(&value, "page_url", &mut issues));
  let source_key = source_record_key(manufacturer, aircraft);
  let horsepower =
    engine.and_then(|value| numeric(value.get("horsepower"), "engine.horsepower", &mut issues));
  let thrust = engine.and_then(|value| numeric(value.get("thrust"), "engine.thrust", &mut issues));
  let engine_count = perf_object
    .and_then(|value| value.get("horsepower").or_else(|| value.get("thrust")))
    .and_then(value_text)
    .and_then(|value| parse_engine_count(&value));

  PreparedAircraftRecord {
    source_record_key: source_key.clone(),
    identity: AircraftIdentityInput {
      manufacturer_name: manufacturer.trim().to_owned(),
      aircraft_name: aircraft.trim().to_owned(),
      source_link: source_link.clone(),
      page_url: page_url.clone(),
      title: scalar(object.get("title"), "title", &mut issues),
      description,
    },
    lifecycle: LifecycleInput {
      production_start_year: start,
      production_end_year: end,
      is_in_production: in_production,
      service_status: service_status(in_production).map(str::to_owned),
      landing_gear: landing_gear.map(str::to_owned),
      variant_type: variant_type(start).map(str::to_owned),
      passenger_capacity: passengers,
      crew_count: crew,
    },
    performance: PerformanceInput { measurements: performance },
    weights: WeightInput { measurements: weights },
    propulsion: PropulsionInput {
      category: propulsion,
      manufacturer: engine
        .and_then(|value| scalar(value.get("manufacturer"), "engine.manufacturer", &mut issues)),
      model: engine.and_then(|value| scalar(value.get("model"), "engine.model", &mut issues)),
      horsepower,
      thrust_newtons: thrust,
      engine_count,
      tbo_hours: engine.and_then(|value| {
        numeric_integer(value.get("overhaul_ht"), "engine.overhaul_ht", &mut issues)
      }),
      tbo_years: engine.and_then(|value| {
        numeric_integer(
          value.get("years_before_overhaul"),
          "engine.years_before_overhaul",
          &mut issues,
        )
      }),
    },
    valuation: ValuationInput {
      papi_price_estimate: numeric(
        object.get("papi_price_estimate"),
        "papi_price_estimate",
        &mut issues,
      ),
      for_sale_count: numeric_integer(object.get("for_sale_count"), "for_sale_count", &mut issues),
    },
    operating_costs: OperatingCostInput { items: cost_items(costs, &mut issues) },
    images: image_items(object.get("images"), &mut issues),
    provenance: ProvenanceInput {
      source_system_key: source_key,
      source_url: page_url,
      source_path: source_link,
      confidence: Confidence::SCRAPED_SOURCE.get(),
    },
    issues,
    raw_document: raw,
  }
}

fn empty_record(
  manufacturer: &str,
  aircraft: &str,
  raw: Value,
  issues: Vec<IngestIssue>,
) -> PreparedAircraftRecord {
  let key = source_record_key(manufacturer, aircraft);
  PreparedAircraftRecord {
    source_record_key: key.clone(),
    identity: AircraftIdentityInput {
      manufacturer_name: manufacturer.trim().to_owned(),
      aircraft_name: aircraft.trim().to_owned(),
      ..AircraftIdentityInput::default()
    },
    lifecycle: LifecycleInput::default(),
    performance: PerformanceInput::default(),
    weights: WeightInput::default(),
    propulsion: PropulsionInput::default(),
    valuation: ValuationInput::default(),
    operating_costs: OperatingCostInput::default(),
    images: Vec::new(),
    provenance: ProvenanceInput {
      source_system_key: key,
      source_url: None,
      source_path: None,
      confidence: Confidence::SCRAPED_SOURCE.get(),
    },
    issues,
    raw_document: raw,
  }
}

fn measurements(
  object: Option<&Map<String, Value>>,
  path: &str,
  mapping: fn(&str) -> Option<&'static str>,
  issues: &mut Vec<IngestIssue>,
) -> Vec<MeasurementInput> {
  let Some(object) = object else { return Vec::new() };
  object
    .iter()
    .filter_map(|(field, value)| {
      if path == "performance" && matches!(field.as_str(), "horsepower" | "thrust") {
        return None;
      }
      let field_path = format!("{path}.{field}");
      let raw_value = scalar(Some(value), &field_path, issues)?;
      let numeric_value = parse_numeric(&raw_value);
      let raw_unit = parse_unit(&raw_value);
      let unit_code = raw_unit.as_deref().and_then(unit_code).map(str::to_owned);
      let metric_code = mapping(field).map(str::to_owned);
      if metric_code.is_none() {
        issues.push(issue(
          "UNMAPPED_MEASUREMENT_FIELD",
          IssueSeverity::Warning,
          &field_path,
          "measurement is preserved as a pending assertion",
          Some(value),
        ));
      }
      if numeric_value.is_none() {
        issues.push(issue(
          "MEASUREMENT_PARSE_FAILURE",
          IssueSeverity::Warning,
          &field_path,
          "measurement has no parseable numeric prefix",
          Some(value),
        ));
      }
      if raw_unit.is_some() && unit_code.is_none() {
        issues.push(issue(
          "UNKNOWN_MEASUREMENT_UNIT",
          IssueSeverity::Warning,
          &field_path,
          "unit is not mapped and will not be canonicalized",
          Some(value),
        ));
      }
      if metric_code.is_some() && numeric_value.is_some() && raw_unit.is_none() {
        issues.push(issue(
          "MISSING_MEASUREMENT_UNIT",
          IssueSeverity::Warning,
          &field_path,
          "measurement has no unit and will not be canonicalized",
          Some(value),
        ));
      }
      // A known unit in the wrong dimension is more dangerous than an unmapped
      // one: it survives as a well-formed candidate and could be canonicalized
      // as if pounds were an airspeed. Demote it to evidence, exactly as an
      // unmapped unit already is.
      let unit_code = match (metric_code.as_deref(), unit_code) {
        (Some(metric), Some(unit)) if !unit_fits_metric(metric, &unit) => {
          issues.push(issue(
            "INCOMPATIBLE_MEASUREMENT_UNIT",
            IssueSeverity::Warning,
            &field_path,
            "unit does not measure this metric and will not be canonicalized",
            Some(value),
          ));
          None
        }
        (_, unit) => unit,
      };
      // The measurement tables reject negative canonical values
      // (chk_pm_canonical_nonneg, migration 008), so keeping this numeric would
      // let preflight pass a batch that the import transaction then aborts.
      let numeric_value = match numeric_value {
        Some(parsed) if parsed.starts_with('-') => {
          issues.push(issue(
            "NEGATIVE_MEASUREMENT",
            IssueSeverity::Warning,
            &field_path,
            "measurement is negative and will not be canonicalized",
            Some(value),
          ));
          None
        }
        other => other,
      };
      Some(MeasurementInput {
        source_field: field.clone(),
        metric_code,
        raw_value,
        numeric_value,
        raw_unit,
        unit_code,
      })
    })
    .collect()
}

fn cost_items(
  object: Option<&Map<String, Value>>,
  issues: &mut Vec<IngestIssue>,
) -> Vec<CostItemInput> {
  let Some(object) = object else { return Vec::new() };
  object
    .iter()
    .filter_map(|(key, value)| {
      let raw = scalar(Some(value), &format!("ownership_costs.{key}"), issues)?;
      let (mapped, numeric, aggregate) = cost_code(key);
      let amount = numeric.then(|| parse_numeric(&raw)).flatten();
      // Two different failures, and a curator resolves them differently: an
      // unmapped key needs a vocabulary decision, an unparseable value needs the
      // source read again. They shared one code until 2026-09, which made the
      // 737 open flags on this corpus unreadable -- 631 were a missing mapping
      // and 106 were a label the scraper had leaked into the value slot, and
      // nothing said so. An unmapped key reports only the key, never both:
      // mapping it is what unblocks the value, so the value is not yet a
      // finding.
      if mapped.is_none() {
        issues.push(issue(
          "UNMAPPED_COST_KEY",
          IssueSeverity::Warning,
          &format!("ownership_costs.{key}"),
          "no cost item type matches this key; the cost is preserved for curation \
           and excluded from canonical totals",
          Some(value),
        ));
      } else if numeric && amount.is_none() {
        issues.push(issue(
          "UNPARSEABLE_COST_VALUE",
          IssueSeverity::Warning,
          &format!("ownership_costs.{key}"),
          "the key is mapped but its value is not a number; the cost is preserved \
           for curation and excluded from canonical totals",
          Some(value),
        ));
      }
      Some(CostItemInput {
        source_key: key.clone(),
        raw_value: raw,
        mapped_code: mapped.map(str::to_owned),
        numeric_value: amount,
        is_aggregate: aggregate,
        is_numeric: numeric,
      })
    })
    .collect()
}

fn image_items(value: Option<&Value>, issues: &mut Vec<IngestIssue>) -> Vec<ImageMetadataInput> {
  let Some(value) = value else { return Vec::new() };
  let Some(images) = value.as_array() else {
    issues.push(issue(
      "IMAGES_NOT_ARRAY",
      IssueSeverity::Warning,
      "images",
      "images metadata is not an array",
      Some(value),
    ));
    return Vec::new();
  };
  let mut items: Vec<ImageMetadataInput> = images
    .iter()
    .enumerate()
    .filter_map(|(position, image)| {
      let field_path = format!("images[{position}]");
      let Some(object) = image.as_object() else {
        issues.push(issue(
          "IMAGE_NOT_OBJECT",
          IssueSeverity::Warning,
          &field_path,
          "image metadata must be an object",
          Some(image),
        ));
        return None;
      };
      let Some(href) = object.get("href").and_then(value_text) else {
        issues.push(issue(
          "IMAGE_HREF_MISSING",
          IssueSeverity::Warning,
          &format!("{field_path}.href"),
          "image metadata has no usable href",
          object.get("href"),
        ));
        return None;
      };
      let Ok(array_position) = i16::try_from(position) else {
        issues.push(issue(
          "IMAGE_POSITION_OUT_OF_RANGE",
          IssueSeverity::Warning,
          &field_path,
          "image position exceeds the supported range",
          None,
        ));
        return None;
      };
      let dimensions = object.get("dimensions").and_then(value_text);
      let parsed = dimensions.as_deref().and_then(dimensions_px);
      if dimensions.is_some() && parsed.is_none() {
        issues.push(issue(
          "INVALID_IMAGE_DIMENSIONS",
          IssueSeverity::Warning,
          &format!("{field_path}.dimensions"),
          "expected WxH dimensions",
          object.get("dimensions"),
        ));
      }
      let href_resolved = normalized_url(&href, &format!("{field_path}.href"), issues);
      Some(ImageMetadataInput {
        array_position,
        href_resolved,
        href_raw: href,
        title: object.get("title").and_then(value_text),
        holder: object.get("holder").and_then(value_text),
        dimensions_raw: dimensions,
        width_px: parsed.map(|pair| pair.0),
        height_px: parsed.map(|pair| pair.1),
        is_primary: false,
      })
    })
    .collect();
  // Source order still decides which image leads, but a discarded first entry
  // must not leave the record with no primary image at all.
  if let Some(first) = items.first_mut() {
    first.is_primary = true;
  }
  items
}

fn nested<'a>(
  object: &'a Map<String, Value>,
  field: &str,
  issues: &mut Vec<IngestIssue>,
) -> Option<&'a Map<String, Value>> {
  match object.get(field) {
    None | Some(Value::Null) => None,
    Some(Value::Object(value)) => Some(value),
    Some(value) => {
      issues.push(issue(
        "FIELD_NOT_OBJECT",
        IssueSeverity::Warning,
        field,
        "nested field must be an object",
        Some(value),
      ));
      None
    }
  }
}

fn scalar(value: Option<&Value>, field: &str, issues: &mut Vec<IngestIssue>) -> Option<String> {
  match value? {
    Value::Null => None,
    Value::String(value) => parse_sentinel(value).map(str::to_owned),
    Value::Number(value) => Some(value.to_string()),
    Value::Bool(value) => Some(value.to_string()),
    value => {
      issues.push(issue(
        "FIELD_NOT_SCALAR",
        IssueSeverity::Warning,
        field,
        "field must be a scalar value",
        Some(value),
      ));
      None
    }
  }
}

fn value_text(value: &Value) -> Option<String> {
  match value {
    Value::String(value) => parse_sentinel(value).map(str::to_owned),
    Value::Number(value) => Some(value.to_string()),
    Value::Bool(value) => Some(value.to_string()),
    _ => None,
  }
}

/// The `aircraft_ref.variant_types` code a documented production run evidences.
///
/// Set from the production range rather than from the corpus: a record naming the
/// years an aircraft was built documents a series aircraft, which is what
/// `PRODUCTION_STANDARD` means. A record without one evidences nothing and stays
/// absent, so this never asserts a type merely because `PlanePHD` listed the row.
///
/// The narrower codes -- `PROTOTYPE`, `PRE_PRODUCTION`, `EXPORT`,
/// `MILITARY_CONVERSION`, `CIVIL_CONVERSION`, `STC_CONVERSION`, `SPECIAL_MISSION`,
/// `CLASSIC_SERIES` -- describe distinctions a production range cannot evidence, and
/// nothing in this source states them: none of the 1005 records mentions any of
/// those words in its description or title. A curator narrows the code when one
/// applies.
const fn variant_type(production_start_year: Option<i16>) -> Option<&'static str> {
  match production_start_year {
    Some(_) => Some("PRODUCTION_STANDARD"),
    None => None,
  }
}

/// The `aircraft_ref.landing_gear_types` code the description states.
///
/// `PlanePHD` writes retraction but not configuration -- "with fixed landing
/// gear", never "fixed tricycle" -- and the four wheeled codes each bundle the
/// two. `FIXED_UNSPECIFIED` and `RETRACTABLE_UNSPECIFIED` exist so the half the
/// source does give is kept instead of discarded; a curator narrows them later.
///
/// Configuration is checked first, so a source that states it resolves to the
/// specific code and this never flattens a known tricycle to the unspecified
/// pair. Float, ski, and amphibious gear are deliberately not matched: the words
/// appear in aircraft names as often as in gear descriptions here, and a wrong
/// code in a filterable column is worse than an absent one.
fn landing_gear_type(lowered: &str) -> Option<&'static str> {
  let tricycle = lowered.contains("tricycle");
  let tailwheel = lowered.contains("tailwheel") || lowered.contains("taildragger");

  if lowered.contains("retractable") {
    return Some(if tricycle {
      "RETRACTABLE_TRICYCLE"
    } else if tailwheel {
      "RETRACTABLE_TAILWHEEL"
    } else {
      "RETRACTABLE_UNSPECIFIED"
    });
  }
  if lowered.contains("fixed landing gear") {
    return Some(if tricycle {
      "FIXED_TRICYCLE"
    } else if tailwheel {
      "FIXED_TAILWHEEL"
    } else {
      "FIXED_UNSPECIFIED"
    });
  }
  None
}

/// The `aircraft_ref.service_statuses` code a known production state implies.
///
/// Restatement, not inference: the source gives a production range, an open range
/// means the type is still being built and a closed one means it is not. The
/// vocabulary's own labels are "In Production" and "Discontinued".
///
/// Deliberately never `RETIRED`, which is about service rather than production --
/// a closed production run says nothing about whether airframes still fly -- and
/// never `LIMITED_PRODUCTION` or `EXPERIMENTAL`, which the source never states.
const fn service_status(in_production: Option<bool>) -> Option<&'static str> {
  match in_production {
    Some(true) => Some("IN_PRODUCTION"),
    Some(false) => Some("DISCONTINUED"),
    None => None,
  }
}

/// The `aircraft_ref.propulsion_categories` code the description states.
///
/// `PlanePHD` writes the category in prose and in no structured field: "Single
/// engine piston aircraft with fixed landing gear". Only phrases that name
/// exactly one code are mapped. `turbofan` is deliberately absent -- the
/// vocabulary distinguishes `TURBOFAN_LOW_BPR` from `TURBOFAN_HIGH_BPR` and the
/// source never states the bypass ratio, so a mapping would invent the fact into
/// a column `VariantFilter::propulsion_category` filters on.
///
/// Two-sided: the codes are seeded by
/// `database/seeds/002_lookup_seed_data.sql` and constrained by the foreign key
/// on `aircraft_core.variants.propulsion_category_code`
/// (`database/migrations/004_aircraft_identity_taxonomy.sql:123`), which is what
/// makes an unmapped spelling a failed import rather than a silent bad row.
fn propulsion_category(lowered: &str) -> Option<&'static str> {
  // Ordered longest-first: "turboprop" and "turbojet" both contain neither
  // substring of the other, but checking "piston" first keeps the common case
  // cheap.
  if lowered.contains("piston") {
    return Some("PISTON_RECIPROCATING");
  }
  if lowered.contains("turboprop") {
    return Some("TURBOPROP");
  }
  if lowered.contains("turbojet") {
    return Some("TURBOJET");
  }
  None
}

/// Production years read off an aircraft name: start, end, and whether the range
/// is still open. All three are absent together when the name carries no range.
type ProductionRange = (Option<i16>, Option<i16>, Option<bool>);

/// The production years `PlanePHD` spells into the aircraft name.
///
/// The published file names aircraft `120 (1946 - 1946)` and
/// `777-200ER (1997 - present)`, and carries no `start_year`/`end_year` field on
/// any record, so this trailing range is the only production-year evidence the
/// source offers. These are *production* years: first flight and certification
/// are not in this source and stay absent.
///
/// A name with no parenthetical is ordinary -- the checked-in fixtures are named
/// that way -- and raises nothing. Only a parenthetical that opens with a digit
/// and holds a separator is read as a range, so `Baron G58 (Turbo)` stays quiet
/// while `120 (19xx - 1946)` is reported: a source whose range stopped parsing is
/// evidence, and guessing at it would be worse than leaving the columns empty.
fn production_range(aircraft: &str, issues: &mut Vec<IngestIssue>) -> ProductionRange {
  parsed_production_range(aircraft).unwrap_or_else(|what| {
    issues.push(issue(
      "UNPARSEABLE_PRODUCTION_RANGE",
      IssueSeverity::Warning,
      "$.<aircraft>",
      what,
      None,
    ));
    (None, None, None)
  })
}

/// The parse alone, so it is testable without an issue list.
///
/// `Err` only for a parenthetical that opens like a range and then is not one; a
/// name carrying no range at all is `Ok` with nothing found.
fn parsed_production_range(aircraft: &str) -> Result<ProductionRange, &'static str> {
  let Some(range) = trailing_parenthetical(aircraft) else { return Ok((None, None, None)) };
  let Some((start_text, end_text)) = range.split_once('-') else { return Ok((None, None, None)) };
  let (start_text, end_text) = (start_text.trim(), end_text.trim());
  if !start_text.starts_with(|character: char| character.is_ascii_digit()) {
    return Ok((None, None, None));
  }

  let start = start_text
    .parse::<i16>()
    .map_err(|_| "aircraft name opens a production range that is not a year")?;
  if end_text.eq_ignore_ascii_case("present") {
    return Ok((Some(start), None, Some(true)));
  }
  let end = end_text
    .parse::<i16>()
    .map_err(|_| "aircraft name closes a production range that is not a year")?;
  Ok((Some(start), Some(end), Some(false)))
}

/// The contents of a trailing `(...)`, trimmed, or `None` when the text has none.
fn trailing_parenthetical(text: &str) -> Option<&str> {
  let inner = text.trim_end().strip_suffix(')')?;
  let open = inner.rfind('(')?;
  Some(inner.get(open + 1..)?.trim())
}

fn integer<T: std::str::FromStr>(
  object: &Map<String, Value>,
  field: &str,
  issues: &mut Vec<IngestIssue>,
) -> Option<T> {
  let value = scalar(object.get(field), field, issues)?;
  value.parse().ok().or_else(|| {
    issues.push(issue(
      "INVALID_INTEGER_FIELD",
      IssueSeverity::Warning,
      field,
      "field is not an integer",
      object.get(field),
    ));
    None
  })
}

fn numeric(value: Option<&Value>, field: &str, issues: &mut Vec<IngestIssue>) -> Option<String> {
  let raw = scalar(value, field, issues)?;
  parse_numeric(&raw).or_else(|| {
    issues.push(issue(
      "INVALID_NUMERIC_FIELD",
      IssueSeverity::Warning,
      field,
      "field has no valid numeric prefix",
      value,
    ));
    None
  })
}

fn numeric_integer<T: std::str::FromStr>(
  value: Option<&Value>,
  field: &str,
  issues: &mut Vec<IngestIssue>,
) -> Option<T> {
  let parsed = numeric(value, field, issues)?;
  parsed.parse().ok().or_else(|| {
    issues.push(issue(
      "INVALID_INTEGER_FIELD",
      IssueSeverity::Warning,
      field,
      "numeric field is not an integer in the supported range",
      value,
    ));
    None
  })
}

fn boolean(
  object: &Map<String, Value>,
  field: &str,
  issues: &mut Vec<IngestIssue>,
) -> Option<bool> {
  match object.get(field)? {
    Value::Bool(value) => Some(*value),
    Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
      "true" | "yes" | "1" => Some(true),
      "false" | "no" | "0" => Some(false),
      _ => {
        issues.push(issue(
          "INVALID_BOOLEAN_FIELD",
          IssueSeverity::Warning,
          field,
          "field is not a boolean",
          object.get(field),
        ));
        None
      }
    },
    value => {
      issues.push(issue(
        "INVALID_BOOLEAN_FIELD",
        IssueSeverity::Warning,
        field,
        "field is not a boolean",
        Some(value),
      ));
      None
    }
  }
}

#[must_use]
pub fn parse_sentinel(raw: &str) -> Option<&str> {
  let value = raw.trim();
  let lower = value.to_ascii_lowercase();
  (!(value.is_empty() || lower.starts_with("none") || matches!(lower.as_str(), "n/a" | "-" | "--")))
    .then_some(value)
}

#[must_use]
pub fn parse_numeric(raw: &str) -> Option<String> {
  let cleaned = parse_sentinel(raw)?.trim_start_matches('$').replace(',', "");
  let mut output = String::new();
  for (index, character) in cleaned.chars().enumerate() {
    if character.is_ascii_digit() || character == '.' || (index == 0 && character == '-') {
      output.push(character);
    } else {
      break;
    }
  }
  let unsigned = output.strip_prefix('-').unwrap_or(&output);
  let mut parts = unsigned.split('.');
  let whole = parts.next().unwrap_or_default();
  let fraction = parts.next();
  let valid = parts.next().is_none()
    && (!whole.is_empty() || fraction.is_some_and(|part| !part.is_empty()))
    && whole.chars().all(|character| character.is_ascii_digit())
    && fraction.is_none_or(|part| part.chars().all(|character| character.is_ascii_digit()));
  valid.then_some(output)
}

fn normalized_url(raw: &str, field: &str, issues: &mut Vec<IngestIssue>) -> Option<String> {
  let parsed = Url::parse(raw)
    .or_else(|_| Url::parse("https://planephd.com/").and_then(|base| base.join(raw)));
  match parsed {
    Ok(url) if matches!(url.scheme(), "http" | "https") => Some(url.into()),
    _ => {
      issues.push(issue(
        "INVALID_URL",
        IssueSeverity::Warning,
        field,
        "URL is invalid or uses an unsupported scheme",
        Some(&Value::String(raw.to_owned())),
      ));
      None
    }
  }
}

fn parse_unit(raw: &str) -> Option<String> {
  let token = parse_sentinel(raw)?
    .split_whitespace()
    .last()?
    .trim_matches(|character: char| !character.is_ascii_alphabetic());
  (!token.is_empty()).then(|| token.to_ascii_uppercase())
}

fn unit_code(raw: &str) -> Option<&'static str> {
  match raw {
    "KIAS" => Some("KIAS"),
    "KCAS" => Some("KNOTS"),
    "KTAS" => Some("KTAS"),
    "NM" => Some("NM"),
    "FT" => Some("FT"),
    "FPM" => Some("FPM"),
    "GPH" => Some("GPH"),
    "LBS" | "LB" => Some("LBS"),
    "KG" => Some("KG"),
    "GAL" => Some("US_GAL"),
    "HP" => Some("HP"),
    "KW" => Some("KW"),
    "N" => Some("NEWTONS"),
    "LBF" => Some("LBF"),
    "HRS" | "HR" => Some("HRS"),
    "PPH" => Some("PPH"),
    _ => None,
  }
}

/// Physical dimension a unit measures.
///
/// Unit mapping is metric-independent, so this is what stops a known unit being
/// applied to a metric it cannot express.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dimension {
  Speed,
  Length,
  ClimbRate,
  FuelFlow,
  Mass,
  Volume,
  Power,
  Thrust,
  Time,
}

fn unit_dimension(unit: &str) -> Option<Dimension> {
  match unit {
    "KIAS" | "KNOTS" | "KTAS" => Some(Dimension::Speed),
    "NM" | "FT" => Some(Dimension::Length),
    "FPM" => Some(Dimension::ClimbRate),
    "GPH" | "PPH" => Some(Dimension::FuelFlow),
    "LBS" | "KG" => Some(Dimension::Mass),
    "US_GAL" => Some(Dimension::Volume),
    "HP" | "KW" => Some(Dimension::Power),
    "NEWTONS" | "LBF" => Some(Dimension::Thrust),
    "HRS" => Some(Dimension::Time),
    _ => None,
  }
}

/// Dimensions a metric accepts. An empty slice means the metric is not
/// classified and therefore cannot be contradicted.
fn metric_dimensions(metric: &str) -> &'static [Dimension] {
  match metric {
    "SPEED_CRUISE_BEST" | "SPEED_STALL_CLEAN" => &[Dimension::Speed],
    "RANGE_NORMAL"
    | "CEILING_SERVICE"
    | "CEILING_OEI"
    | "DIST_TO_GROUND_ROLL"
    | "DIST_TO_50FT"
    | "DIST_LDG_GROUND_ROLL"
    | "DIST_LDG_50FT" => &[Dimension::Length],
    "CLIMB_RATE_SL" | "CLIMB_RATE_OEI" => &[Dimension::ClimbRate],
    "FUEL_BURN_CRUISE" => &[Dimension::FuelFlow],
    "WEIGHT_EMPTY" | "WEIGHT_MTOW" | "WEIGHT_PAYLOAD" => &[Dimension::Mass],
    // Usable fuel is quoted by volume or by weight depending on the source.
    "FUEL_CAPACITY_USABLE" => &[Dimension::Volume, Dimension::Mass],
    _ => &[],
  }
}

/// Whether a mapped unit can express a mapped metric. Unclassified metrics and
/// units pass, so this only ever rejects a pairing it positively understands.
fn unit_fits_metric(metric: &str, unit: &str) -> bool {
  let Some(actual) = unit_dimension(unit) else { return true };
  let expected = metric_dimensions(metric);
  expected.is_empty() || expected.contains(&actual)
}

fn perf_code(field: &str) -> Option<&'static str> {
  match field {
    "best_cruise_speed" => Some("SPEED_CRUISE_BEST"),
    "best_range_i" => Some("RANGE_NORMAL"),
    "ceiling" => Some("CEILING_SERVICE"),
    "fuel_burn" | "fuel_burn_75" => Some("FUEL_BURN_CRUISE"),
    "rate_of_climb" => Some("CLIMB_RATE_SL"),
    // One engine inoperative. Distinct metrics, not variants of the all-engine
    // figures: a twin's OEI ceiling is the one that decides terrain clearance.
    "rate_of_climb_1_engine_out" => Some("CLIMB_RATE_OEI"),
    "ceiling_1_engine_out" => Some("CEILING_OEI"),
    "takeoff_distance" => Some("DIST_TO_GROUND_ROLL"),
    "takeoff_distance_over_50ft_obstacle" => Some("DIST_TO_50FT"),
    "landing_distance" => Some("DIST_LDG_GROUND_ROLL"),
    "landing_distance_over_50ft_obstacle" => Some("DIST_LDG_50FT"),
    "stall_speed" => Some("SPEED_STALL_CLEAN"),
    _ => None,
  }
}

fn weight_code(field: &str) -> Option<&'static str> {
  match field {
    "empty_weight" => Some("WEIGHT_EMPTY"),
    "gross_weight" => Some("WEIGHT_MTOW"),
    "fuel_capacity" => Some("FUEL_CAPACITY_USABLE"),
    "maximum_payload" => Some("WEIGHT_PAYLOAD"),
    _ => None,
  }
}

fn cost_code(key: &str) -> (Option<&'static str>, bool, bool) {
  let key = key.to_ascii_lowercase();
  if key.contains("total") && key.contains("fixed") {
    return (Some("TOTAL_FIXED_COST"), true, true);
  }
  if key.contains("total") && key.contains("variable") {
    return (Some("TOTAL_VARIABLE_COST"), true, true);
  }
  if key.contains("total")
    && (key.contains("annual")
      || key.contains("yearly")
      || key.contains("cost_per_year")
      || key.contains("ownership"))
  {
    return (Some("TOTAL_COST_ANNUAL"), true, true);
  }
  // Before the substring table, whose "training" entry would otherwise claim
  // `pilot_salary_taxes_and_benefits`. Both keys appear on the same aircraft in
  // this corpus, and while they shared `PILOT_TRAINING` the line-item writer's
  // ON CONFLICT(snapshot_id,cost_item_type_code) DO NOTHING
  // (`crates/aircraft_db/src/repositories/ingestion_repository.rs`) silently
  // discarded whichever arrived second. `PILOT_SALARY`
  // (`database/seeds/002_lookup_seed_data.sql`) keeps both.
  if key.contains("pilot_salary") {
    return (Some("PILOT_SALARY"), true, false);
  }
  let mappings = [
    ("miscellaneous", "MISC_VARIABLE"),
    ("inspection", "ANNUAL_INSPECTION"),
    ("insurance", "INSURANCE"),
    ("hangar", "HANGAR_STORAGE"),
    ("storage", "HANGAR_STORAGE"),
    ("depreciation", "DEPRECIATION"),
    ("weather", "WEATHER_SERVICE"),
    ("training", "PILOT_TRAINING"),
    ("refurbish", "REFURBISHING"),
    ("registration", "REGISTRATION_TAXES"),
    ("financ", "FINANCING"),
    ("fuel", "FUEL"),
    ("oil", "OIL"),
    ("avionics", "AVIONICS_RESERVE"),
    ("landing", "LANDING_FEES"),
    ("unscheduled", "UNSCHEDULED_MAINT"),
    ("maint", "HOURLY_MAINTENANCE"),
  ];
  if key.contains("engine")
    && ["reserve", "overhaul", "fund", "tbo"].iter().any(|part| key.contains(part))
  {
    return (Some("ENGINE_RESERVE"), true, false);
  }
  if key.contains("prop") && ["reserve", "overhaul", "fund"].iter().any(|part| key.contains(part)) {
    return (Some("PROP_RESERVE"), true, false);
  }
  if let Some((_, code)) = mappings.iter().find(|(part, _)| key.contains(part)) {
    return (Some(*code), true, false);
  }
  // Last, so that every key naming a specific part has already been claimed:
  // `engine` and `prop` above, and `avionics` in the table, all contain
  // "reserve" and would be swallowed by this arm if it ran first. What reaches
  // here names an accrual and no part, which is what `OVERHAUL_RESERVE`
  // (`database/seeds/002_lookup_seed_data.sql`, which names this function) means.
  // PlanePHD publishes one `overhaul_reserves` figure and no engine- or
  // propeller-specific key anywhere in the corpus, so splitting it would invent
  // a division the source never made.
  if ["reserve", "overhaul"].iter().any(|part| key.contains(part)) {
    return (Some("OVERHAUL_RESERVE"), true, false);
  }
  (None, true, false)
}

fn parse_engine_count(raw: &str) -> Option<i16> {
  let raw = parse_sentinel(raw)?;
  if raw.to_ascii_lowercase().contains(" x ") {
    raw.split_whitespace().next()?.parse().ok()
  } else {
    Some(1)
  }
}

fn occupants(description: &str) -> (Option<i16>, Option<i16>) {
  (number_after(description, "seats up to "), number_after(description, "plus "))
}

fn number_after(value: &str, marker: &str) -> Option<i16> {
  let lower = value.to_ascii_lowercase();
  let start = lower.find(marker)? + marker.len();
  lower[start..].split_whitespace().next()?.parse().ok()
}

fn dimensions_px(raw: &str) -> Option<(i16, i16)> {
  let lower = raw.to_ascii_lowercase();
  let (width, height) = lower.split_once('x')?;
  Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn source_record_key(manufacturer: &str, aircraft: &str) -> String {
  let mut hash = Sha256::new();
  hash.update(b"planephd\0");
  hash.update(manufacturer.as_bytes());
  hash.update(b"\0");
  hash.update(aircraft.as_bytes());
  hex_digest(&hash.finalize())
}

fn issue(
  code: &str,
  severity: IssueSeverity,
  path: &str,
  message: &str,
  raw: Option<&Value>,
) -> IngestIssue {
  IngestIssue {
    code: code.to_owned(),
    severity,
    field_path: path.to_owned(),
    message: message.to_owned(),
    raw_value: raw.map(|value| {
      value.to_string().chars().filter(|character| !character.is_control()).take(256).collect()
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  /// A documented production run evidences a series aircraft; nothing else here
  /// evidences a type at all. The absent case is the one that matters -- without
  /// it this would stamp a type on every row `PlanePHD` happened to list.
  #[test]
  fn a_documented_production_run_evidences_a_standard_variant_and_nothing_else_does() {
    let kind =
      |aircraft: &str| normalize_record("CESSNA", aircraft, json!({})).lifecycle.variant_type;

    assert_eq!(kind("172S (1998 - present)").as_deref(), Some("PRODUCTION_STANDARD"));
    assert_eq!(kind("310R (1975 - 1980)").as_deref(), Some("PRODUCTION_STANDARD"));
    assert_eq!(kind("172S Skyhawk SP"), None, "no production run, no type asserted");
  }

  /// The source states retraction and withholds configuration, so the unspecified
  /// codes keep the half it gives. A source that does state configuration must
  /// still reach the specific code -- otherwise adding those two codes would have
  /// thrown information away rather than kept it.
  #[test]
  fn landing_gear_keeps_stated_retraction_and_narrows_when_configuration_is_stated() {
    let gear = |prose: &str| {
      normalize_record("CESSNA", "310R", json!({"description": prose})).lifecycle.landing_gear
    };

    assert_eq!(
      gear("Single engine piston aircraft with fixed landing gear.").as_deref(),
      Some("FIXED_UNSPECIFIED")
    );
    assert_eq!(
      gear("Twin engine piston aircraft with retractable landing gear.").as_deref(),
      Some("RETRACTABLE_UNSPECIFIED")
    );
    assert_eq!(
      gear("Piston aircraft with retractable tricycle landing gear.").as_deref(),
      Some("RETRACTABLE_TRICYCLE"),
      "a stated configuration must not be flattened"
    );
    assert_eq!(
      gear("Piston taildragger with fixed landing gear.").as_deref(),
      Some("FIXED_TAILWHEEL")
    );
    assert_eq!(gear("Single engine piston aircraft."), None, "no gear stated, none recorded");
  }

  /// The production range already tells us whether the type is still built, so the
  /// status restates it rather than guessing. A name with no range leaves it
  /// absent: `VariantFilter::service_status` must not partition on an assumption.
  #[test]
  fn service_status_restates_the_production_range_and_is_absent_without_one() {
    let status =
      |aircraft: &str| normalize_record("CESSNA", aircraft, json!({})).lifecycle.service_status;

    assert_eq!(status("172S (1998 - present)").as_deref(), Some("IN_PRODUCTION"));
    assert_eq!(status("310R (1975 - 1980)").as_deref(), Some("DISCONTINUED"));
    assert_eq!(status("172S Skyhawk SP"), None, "no range means nothing is known");
  }

  /// `PlanePHD` states the propulsion category in prose -- "Single engine piston
  /// aircraft with fixed landing gear" -- and nowhere else. Three of the four
  /// phrases it uses map onto exactly one `aircraft_ref.propulsion_categories`
  /// code; `turbofan` does not, because that vocabulary splits low- and
  /// high-bypass and the source never says which. Guessing would put an invented
  /// fact in a filterable column, so a turbofan stays absent.
  #[test]
  fn a_stated_propulsion_category_maps_and_an_ambiguous_one_stays_absent() {
    let category = |prose: &str| {
      normalize_record("CESSNA", "310R", json!({"description": prose})).propulsion.category
    };

    assert_eq!(
      category("Single engine piston aircraft with fixed landing gear.").as_deref(),
      Some("PISTON_RECIPROCATING")
    );
    assert_eq!(category("Twin engine turboprop aircraft.").as_deref(), Some("TURBOPROP"));
    assert_eq!(category("Single engine turbojet aircraft.").as_deref(), Some("TURBOJET"));
    assert_eq!(
      category("Twin engine turbofan aircraft."),
      None,
      "low- and high-bypass are distinct codes and the source does not choose"
    );
    assert_eq!(category("Nothing about propulsion here."), None);
  }

  /// The three measurements the shipped file carries that reached the database
  /// with no `metric_code`: 1,039 curation flags between them. All three codes
  /// were already seeded, so this was a missing mapping rather than missing
  /// vocabulary. Each pairing also asserts the unit survived, because a metric
  /// absent from `metric_dimensions` silently loses its unit instead.
  #[test]
  fn one_engine_out_metrics_and_maximum_payload_map_to_their_reference_codes() {
    let record = normalize_record(
      "BEECHCRAFT",
      "Baron G58",
      json!({
        "performance": {
          "rate_of_climb_1_engine_out": "390 FPM",
          "ceiling_1_engine_out": "7284 FT"
        },
        "weights": {"maximum_payload": "1000 LBS"}
      }),
    );

    let coded: Vec<(&str, Option<&str>, Option<&str>)> = record
      .performance
      .measurements
      .iter()
      .chain(record.weights.measurements.iter())
      .map(|measurement| {
        (
          measurement.source_field.as_str(),
          measurement.metric_code.as_deref(),
          measurement.unit_code.as_deref(),
        )
      })
      .collect();

    assert!(
      coded.contains(&("rate_of_climb_1_engine_out", Some("CLIMB_RATE_OEI"), Some("FPM"))),
      "{coded:?}"
    );
    assert!(
      coded.contains(&("ceiling_1_engine_out", Some("CEILING_OEI"), Some("FT"))),
      "{coded:?}"
    );
    assert!(coded.contains(&("maximum_payload", Some("WEIGHT_PAYLOAD"), Some("LBS"))), "{coded:?}");
  }

  /// `overhaul_reserves` was withheld here while the vocabulary offered only
  /// `ENGINE_RESERVE` and `PROP_RESERVE`, because the bare key says neither and
  /// picking one would have been an invention. Counting the corpus settled it:
  /// the key appears 631 times and is the *only* reserve or overhaul key in all
  /// 1,005 records, so the source is not withholding a split, it does not make
  /// one. `OVERHAUL_RESERVE` now says that, and the 631 values -- all clean
  /// per-hour dollars -- are canonical rather than evidence.
  #[test]
  fn the_named_ownership_cost_keys_map_to_their_cost_codes() {
    let record = normalize_record(
      "CESSNA",
      "172S",
      json!({"ownership_costs": {
        "total_cost_of_ownership": "$25,000",
        "miscellaneous_expenses": "$1,200",
        "overhaul_reserves": "$3,000"
      }}),
    );

    let coded: Vec<(&str, Option<&str>)> = record
      .operating_costs
      .items
      .iter()
      .map(|item| (item.source_key.as_str(), item.mapped_code.as_deref()))
      .collect();

    assert!(coded.contains(&("total_cost_of_ownership", Some("TOTAL_COST_ANNUAL"))), "{coded:?}");
    assert!(coded.contains(&("miscellaneous_expenses", Some("MISC_VARIABLE"))), "{coded:?}");
    assert!(
      coded.contains(&("overhaul_reserves", Some("OVERHAUL_RESERVE"))),
      "a reserve naming no part is the undifferentiated one: {coded:?}"
    );
  }

  /// The undifferentiated reserve runs last precisely because it would otherwise
  /// swallow the specific ones: `engine`, `prop` and `avionics` reserve keys all
  /// contain "reserve". Moving the `OVERHAUL_RESERVE` arm above any of them
  /// makes this fail, which is the only thing holding that order in place.
  #[test]
  fn a_reserve_that_names_its_part_keeps_its_own_code() {
    let record = normalize_record(
      "CESSNA",
      "172S",
      json!({"ownership_costs": {
        "engine_overhaul_fund": "$40.00",
        "prop_reserve": "$5.00",
        "avionics_reserve": "$7.00",
        "overhaul_reserves": "$31.42"
      }}),
    );

    let coded: Vec<(&str, Option<&str>)> = record
      .operating_costs
      .items
      .iter()
      .map(|item| (item.source_key.as_str(), item.mapped_code.as_deref()))
      .collect();

    assert!(coded.contains(&("engine_overhaul_fund", Some("ENGINE_RESERVE"))), "{coded:?}");
    assert!(coded.contains(&("prop_reserve", Some("PROP_RESERVE"))), "{coded:?}");
    assert!(coded.contains(&("avionics_reserve", Some("AVIONICS_RESERVE"))), "{coded:?}");
    assert!(coded.contains(&("overhaul_reserves", Some("OVERHAUL_RESERVE"))), "{coded:?}");
  }

  /// Both shapes are taken from the real corpus. `pilot_salary_taxes_and_benefits`
  /// maps to `PILOT_SALARY` and carries the literal string `"Pilot training"`
  /// in 106 of its 173 occurrences -- a label the scraper put in the value slot,
  /// next to the sibling `pilot_training` key it belongs to. There is no number
  /// to recover, so the flag's job is to say the value was the problem.
  /// `mystery_surcharge` stands for the other cause: a key the vocabulary has no
  /// code for, whose value is fine.
  #[test]
  fn an_unmapped_cost_key_and_an_unparseable_cost_value_are_flagged_apart() {
    let record = normalize_record(
      "AERO VODOCHODY",
      "L-39 Albatross",
      json!({"ownership_costs": {
        "pilot_salary_taxes_and_benefits": "Pilot training",
        "mystery_surcharge": "$250"
      }}),
    );

    let flagged: Vec<(&str, &str)> =
      record.issues.iter().map(|issue| (issue.field_path.as_str(), issue.code.as_str())).collect();

    assert!(
      flagged
        .contains(&("ownership_costs.pilot_salary_taxes_and_benefits", "UNPARSEABLE_COST_VALUE")),
      "a mapped key with a non-numeric value is a value problem: {flagged:?}"
    );
    assert!(
      flagged.contains(&("ownership_costs.mystery_surcharge", "UNMAPPED_COST_KEY")),
      "a key with no cost item type is a vocabulary problem: {flagged:?}"
    );
  }

  /// Employment and training are separate costs and `PlanePHD` states both on the
  /// same aircraft. While they shared one code the second one written was dropped
  /// by the line-item writer's `ON CONFLICT ... DO NOTHING`, with no flag: the
  /// import looked clean and a figure was simply gone. Mapping them apart is what
  /// makes the two rows survive, so this asserts the codes differ rather than
  /// asserting either one in isolation.
  #[test]
  fn pilot_employment_and_pilot_training_do_not_share_a_cost_code() {
    let record = normalize_record(
      "BOMBARDIER",
      "CHALLENGER 350",
      json!({"ownership_costs": {
        "pilot_salary_taxes_and_benefits": "$250,425.00",
        "pilot_training": "$53,993.24"
      }}),
    );

    let coded: Vec<(&str, Option<&str>)> = record
      .operating_costs
      .items
      .iter()
      .map(|item| (item.source_key.as_str(), item.mapped_code.as_deref()))
      .collect();

    assert!(
      coded.contains(&("pilot_salary_taxes_and_benefits", Some("PILOT_SALARY"))),
      "{coded:?}"
    );
    assert!(coded.contains(&("pilot_training", Some("PILOT_TRAINING"))), "{coded:?}");
  }

  /// The real `PlanePHD` file carries `manufacturer_name` and `aircraft_name` in
  /// every record, and the parser consumes both -- as the enclosing map keys.
  /// Flagging them as unpromoted made `warning_count > 0` for all 1005 records,
  /// so none was ever dispositioned clean. The checked-in fixtures omit both
  /// keys, which is why no test caught it.
  #[test]
  fn the_identity_fields_the_parser_consumes_are_not_flagged_as_unsupported() {
    let record = normalize_record(
      "CESSNA",
      "120 (1946 - 1946)",
      json!({"manufacturer_name": "CESSNA", "aircraft_name": "120 (1946 - 1946)"}),
    );

    assert!(
      !record.issues.iter().any(|issue| issue.code == "UNSUPPORTED_RECORD_FIELD"),
      "the two identity fields are consumed, not ignored: {:?}",
      record.issues
    );
  }

  /// `start_year`/`end_year` are in `KNOWN_FIELDS` but absent from every record
  /// of the shipped file; the years live in the aircraft name instead.
  #[test]
  fn production_years_are_read_from_the_aircraft_name_when_the_record_omits_them() {
    let record = normalize_record("CESSNA", "120 (1946 - 1946)", json!({}));

    assert_eq!(record.lifecycle.production_start_year, Some(1946));
    assert_eq!(record.lifecycle.production_end_year, Some(1946));
    assert_eq!(record.lifecycle.is_in_production, Some(false));
  }

  #[test]
  fn an_open_ended_range_leaves_the_end_absent_and_marks_the_aircraft_in_production() {
    let record = normalize_record("BOEING", "777-200ER (1997 - present)", json!({}));

    assert_eq!(record.lifecycle.production_start_year, Some(1997));
    assert_eq!(record.lifecycle.production_end_year, None, "`present` is not a year");
    assert_eq!(record.lifecycle.is_in_production, Some(true));
  }

  /// A feed that states the years outranks one that only spells them in a name.
  #[test]
  fn an_explicit_year_field_outranks_the_range_in_the_name() {
    let record = normalize_record(
      "CESSNA",
      "120 (1946 - 1946)",
      json!({"start_year": "1950", "end_year": "1951", "in_production": false}),
    );

    assert_eq!(record.lifecycle.production_start_year, Some(1950));
    assert_eq!(record.lifecycle.production_end_year, Some(1951));
  }

  /// The checked-in fixtures carry names with no parenthetical at all. Absence is
  /// ordinary, so it must not raise a warning -- that mistake is what buried the
  /// signal in 2010 false ones.
  #[test]
  fn a_name_without_a_year_range_leaves_the_lifecycle_empty_and_raises_nothing() {
    let record = normalize_record("CESSNA", "172S Skyhawk SP", json!({}));

    assert_eq!(record.lifecycle.production_start_year, None);
    assert_eq!(record.lifecycle.is_in_production, None);
    assert!(
      !record.issues.iter().any(|issue| issue.code == "UNPARSEABLE_PRODUCTION_RANGE"),
      "a name with no range is not malformed: {:?}",
      record.issues
    );
  }

  /// A parenthetical that looks like a range but is not one is evidence of a
  /// source change, so it is flagged rather than guessed at.
  #[test]
  fn a_malformed_year_range_is_flagged_rather_than_guessed() {
    let record = normalize_record("CESSNA", "120 (19xx - 1946)", json!({}));

    assert_eq!(record.lifecycle.production_start_year, None);
    assert!(
      record.issues.iter().any(|issue| issue.code == "UNPARSEABLE_PRODUCTION_RANGE"),
      "{:?}",
      record.issues
    );
  }

  #[test]
  fn known_measurement_is_mapped_and_unknown_unit_is_flagged() {
    let record = normalize_record(
      "CESSNA",
      "172S",
      json!({

          "performance": {"best_cruise_speed": "124 KIAS", "mystery": "7 FURLONGS"}
      }),
    );
    assert_eq!(
      record.performance.measurements[0].metric_code.as_deref(),
      Some("SPEED_CRUISE_BEST")
    );
    assert!(record.issues.iter().any(|issue| issue.code == "UNKNOWN_MEASUREMENT_UNIT"));
  }

  #[test]
  fn a_unit_that_cannot_measure_the_metric_is_demoted_to_evidence() {
    // Unit mapping is metric-independent, so "LBS" maps cleanly on its own.
    // Paired with a speed it must not survive as a canonicalizable candidate.
    let record =
      normalize_record("CESSNA", "172S", json!({"performance": {"best_cruise_speed": "124 LBS"}}));
    let measurement = &record.performance.measurements[0];
    assert_eq!(measurement.metric_code.as_deref(), Some("SPEED_CRUISE_BEST"));
    assert_eq!(measurement.unit_code, None, "a mass unit cannot canonicalize a speed");
    assert_eq!(measurement.raw_value, "124 LBS", "the raw value stays as evidence");
    assert_eq!(measurement.raw_unit.as_deref(), Some("LBS"), "the raw unit stays as evidence");
    assert!(record.issues.iter().any(|issue| issue.code == "INCOMPATIBLE_MEASUREMENT_UNIT"));
  }

  #[test]
  fn a_unit_matching_its_metric_is_kept() {
    // Guards the dimension check against rejecting the pairings the fixtures
    // actually carry, including fuel capacity quoted by volume or by weight.
    let record = normalize_record(
      "CESSNA",
      "172S",
      json!({
          "performance": {"best_cruise_speed": "124 KIAS", "ceiling": "14000 FT"},
          "weights": {"gross_weight": "2550 LBS", "fuel_capacity": "56 GAL"}
      }),
    );
    for measurement in
      record.performance.measurements.iter().chain(record.weights.measurements.iter())
    {
      assert!(
        measurement.unit_code.is_some(),
        "{} lost its unit: {measurement:?}",
        measurement.source_field
      );
    }
    assert!(!record.issues.iter().any(|issue| issue.code == "INCOMPATIBLE_MEASUREMENT_UNIT"));
  }

  #[test]
  fn a_negative_measurement_is_demoted_to_evidence() {
    // chk_pm_canonical_nonneg (migration 008) rejects this, so preflight must
    // not report a batch clean that the import transaction would then abort.
    let record =
      normalize_record("CESSNA", "172S", json!({"performance": {"best_cruise_speed": "-10 KIAS"}}));
    let measurement = &record.performance.measurements[0];
    assert_eq!(measurement.numeric_value, None, "a negative value cannot be canonicalized");
    assert_eq!(measurement.raw_value, "-10 KIAS", "the raw value stays as evidence");
    assert!(record.issues.iter().any(|issue| issue.code == "NEGATIVE_MEASUREMENT"));
    // The value parsed, so it must not also be reported as unparseable.
    assert!(!record.issues.iter().any(|issue| issue.code == "MEASUREMENT_PARSE_FAILURE"));
  }

  #[test]
  fn mapped_measurement_without_unit_is_flagged() {
    let record =
      normalize_record("CESSNA", "172S", json!({"performance": {"best_cruise_speed": "124"}}));
    assert!(record.issues.iter().any(|issue| issue.code == "MISSING_MEASUREMENT_UNIT"));
    assert_eq!(record.performance.measurements[0].unit_code, None);
  }

  #[test]
  fn source_record_identity_includes_the_documented_namespace_and_separators() {
    assert_eq!(
      source_record_key("CESSNA", "172S"),
      "1e3683a45e3dd20fa7c025a5fb1bc07b454a98e37c7995cbda6a2b1605f6dc86"
    );
  }

  #[test]
  fn sentinels_and_numeric_prefixes_match_legacy_contract() {
    assert_eq!(parse_sentinel(" None KIAS "), None);
    assert_eq!(parse_numeric("$27,921 USD").as_deref(), Some("27921"));
    assert_eq!(parse_numeric("1.2.3 KIAS"), None);
  }

  #[test]
  fn malformed_optional_values_are_preserved_as_warnings() {
    let record = normalize_record(
      "CESSNA",
      "172S",
      json!({
          "in_production": "sometimes",
          "papi_price_estimate": "1.2.3 USD",
          "page_url": "javascript:alert(1)",
          "images": [42, {"title": "missing href"}, {"href": "javascript:alert(1)"}]
      }),
    );

    for code in [
      "INVALID_BOOLEAN_FIELD",
      "INVALID_NUMERIC_FIELD",
      "INVALID_URL",
      "IMAGE_NOT_OBJECT",
      "IMAGE_HREF_MISSING",
    ] {
      assert!(record.issues.iter().any(|issue| issue.code == code), "{code}");
    }
    assert_eq!(record.images.len(), 1);
    assert!(record.images[0].href_resolved.is_none());
  }

  #[test]
  fn the_first_surviving_image_is_primary() {
    let record = normalize_record(
      "CESSNA",
      "172S",
      json!({
          "images": [
              42,
              {"href": "https://example.test/a.jpg"},
              {"href": "https://example.test/b.jpg"}
          ]
      }),
    );

    assert_eq!(record.images.len(), 2);
    assert!(record.images[0].is_primary, "discarding the first entry must not drop the primary");
    assert!(!record.images[1].is_primary, "exactly one image may be primary");
  }

  #[test]
  fn pilot_salary_amount_is_parsed_like_any_other_mapped_cost() {
    let record =
      normalize_record("CESSNA", "172S", json!({"ownership_costs": {"pilot_salary": "$12,000"}}));

    let item = &record.operating_costs.items[0];
    assert_eq!(item.mapped_code.as_deref(), Some("PILOT_SALARY"));
    assert_eq!(
      item.numeric_value.as_deref(),
      Some("12000"),
      "a mapped cost must carry its amount into the canonical line item"
    );
    assert!(
      !record
        .issues
        .iter()
        .any(|issue| issue.code.ends_with("_COST_KEY") || issue.code.ends_with("_COST_VALUE"))
    );
  }
}
