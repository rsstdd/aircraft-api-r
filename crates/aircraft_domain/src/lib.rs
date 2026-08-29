#![deny(
  clippy::arithmetic_side_effects,
  clippy::as_conversions,
  clippy::float_cmp,
  clippy::indexing_slicing
)]

pub mod aircraft;
pub mod ingestion;
pub mod manufacturer;
pub mod mission;
pub mod units;
pub mod validation;
