//! The one premise `aircraft_domain::measurement::DecimalString` rests on and
//! cannot check for itself: that `PostgreSQL` renders `NUMERIC` as plain,
//! exact, scale-preserving text.
//!
//! `docs/architecture/http_v1_decisions.md` § "Exact decimals and measurements"
//! forbids scientific notation on the wire, and the read path reaches that wire
//! through a `::text` cast — the convention `ingestion_repository.rs` records
//! for the write side and `curation_repository.rs` already uses for reading.
//! Only a real server can say what that cast produces, so no fake stands in
//! here.
//!
//! This adds no production query. It exercises the primitive seam; each
//! consuming repository still owes its own disposable-schema mapping test.

// A failing assertion is the point of a test.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use aircraft_domain::measurement::DecimalString;
use aircraft_testsupport::{TestResult, start_postgres};
use sqlx_core::query_scalar::query_scalar;

#[tokio::test]
async fn postgres_renders_numeric_as_plain_text_this_domain_type_accepts() -> TestResult {
  // No `install_schema`: `SELECT $1::numeric::text` needs no canonical schema,
  // and installing one would test something this file does not claim.
  let (_container, pool) = start_postgres(2, Duration::from_secs(30)).await?;

  // Each case is a rendering a general-purpose decimal formatter gets wrong.
  // The first exceeds the 128 characters an earlier draft of this work would
  // have capped; the second and third are the magnitudes `bigdecimal`'s
  // `Display` switches to scientific notation for, past five leading or fifteen
  // trailing zeros; the fourth is scale that normalization would erase; the
  // fifth is the negative value `aircraft_specs.weight_metrics` permits, since
  // migration 007 carries no non-negative CHECK for `LOAD_FACTOR_NEG`.
  let wide = format!("1{}", "0".repeat(140));
  let cases =
    [wide.as_str(), "0.00000000000000000001", "100000000000000000000", "1.500", "-3.80", "0"];

  for case in cases {
    let rendered: String =
      query_scalar("SELECT $1::numeric::text").bind(case).fetch_one(&pool).await?;

    assert_eq!(rendered, case, "PostgreSQL re-rendered {case:?} rather than preserving it");
    let parsed = DecimalString::parse(&rendered)
      .unwrap_or_else(|error| panic!("{rendered:?} is not a plain decimal: {error}"));
    assert_eq!(parsed.as_str(), rendered, "the domain type preserved {rendered:?} exactly");
  }

  Ok(())
}
