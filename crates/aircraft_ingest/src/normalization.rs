use std::fmt::Write as _;

use aircraft_app::ingestion::{
  AircraftIdentityInput, CostItemInput, ImageMetadataInput, IngestIssue, IssueSeverity,
  LifecycleInput, MeasurementInput, OperatingCostInput, PerformanceInput, PreparedAircraftRecord,
  PropulsionInput, ProvenanceInput, ValuationInput, WeightInput,
};
use aircraft_domain::ingestion::{Confidence, ProductionYears};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use url::Url;

const KNOWN_FIELDS: &[&str] = &[
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
  let start = integer::<i16>(object, "start_year", &mut issues);
  let end = integer::<i16>(object, "end_year", &mut issues);
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
      is_in_production: boolean(object, "in_production", &mut issues),
      passenger_capacity: passengers,
      crew_count: crew,
    },
    performance: PerformanceInput { measurements: performance },
    weights: WeightInput { measurements: weights },
    propulsion: PropulsionInput {
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
      if numeric_value.is_none() && parse_sentinel(&raw_value).is_some() {
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
      if mapped.is_none() || (numeric && amount.is_none()) {
        issues.push(issue(
          "UNMAPPED_OR_UNPARSEABLE_COST",
          IssueSeverity::Warning,
          &format!("ownership_costs.{key}"),
          "cost is preserved for curation and excluded from canonical totals",
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

fn perf_code(field: &str) -> Option<&'static str> {
  match field {
    "best_cruise_speed" => Some("SPEED_CRUISE_BEST"),
    "best_range_i" => Some("RANGE_NORMAL"),
    "ceiling" => Some("CEILING_SERVICE"),
    "fuel_burn" | "fuel_burn_75" => Some("FUEL_BURN_CRUISE"),
    "rate_of_climb" => Some("CLIMB_RATE_SL"),
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
    && (key.contains("annual") || key.contains("yearly") || key.contains("cost_per_year"))
  {
    return (Some("TOTAL_COST_ANNUAL"), true, true);
  }
  if key.contains("pilot_salary") {
    return (Some("PILOT_TRAINING"), true, false);
  }
  let mappings = [
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
  mappings
    .iter()
    .find(|(part, _)| key.contains(part))
    .map_or((None, true, false), |(_, code)| (Some(*code), true, false))
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
  let digest = hash.finalize();
  let mut output = String::with_capacity(64);
  for byte in digest {
    let _ = write!(output, "{byte:02x}");
  }
  output
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
    assert_eq!(item.mapped_code.as_deref(), Some("PILOT_TRAINING"));
    assert_eq!(
      item.numeric_value.as_deref(),
      Some("12000"),
      "a mapped cost must carry its amount into the canonical line item"
    );
    assert!(!record.issues.iter().any(|issue| issue.code == "UNMAPPED_OR_UNPARSEABLE_COST"));
  }
}
