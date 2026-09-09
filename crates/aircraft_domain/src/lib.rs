#![deny(
  clippy::arithmetic_side_effects,
  clippy::as_conversions,
  clippy::float_cmp,
  clippy::indexing_slicing
)]

pub mod catalog;
pub mod ingestion;
pub mod measurement;
pub mod reference;
