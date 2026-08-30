//! Curation: accepting or rejecting what ingestion left pending.
//!
//! The ingestion adapter writes every measurement `is_canonical = FALSE` and
//! every assertion `PENDING`, so nothing it imports is served by the read model
//! until a curator decides. This module owns that decision as a use case; the
//! transactional mechanics live behind [`CurationStore`] in `aircraft_db`.
//!
//! Accepting is deliberately per-field rather than per-record: a source can be
//! right about cruise speed and wrong about empty weight, and the provenance
//! tables are keyed that way (`entity_type_code`, `entity_id`, `field_name`).

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ingestion::{PersistenceError, REPORT_SCHEMA_VERSION};

/// One assertion awaiting a curation decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAssertion {
  pub assertion_id: i64,
  pub entity_type_code: String,
  pub entity_id: i64,
  /// Natural name of the entity, so a curator does not have to resolve an id.
  pub entity_label: Option<String>,
  pub field_name: String,
  pub raw_value: Option<String>,
  pub raw_unit: Option<String>,
  pub asserted_numeric: Option<String>,
  pub source_slug: Option<String>,
  pub created_at: DateTime<Utc>,
}

/// What a curator asked for, and what actually changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurationOutcome {
  pub schema_version: u16,
  pub assertion_id: i64,
  pub decision: Decision,
  /// True when accepting this assertion also promoted a measurement row into
  /// the canonical set the read model serves.
  pub measurement_canonicalized: bool,
  /// Curation flags closed as a side effect of the decision.
  pub flags_resolved: u64,
  /// Whether the read model was rebuilt. Refreshing is skipped when nothing
  /// the read model exposes actually changed.
  pub read_model_refreshed: bool,
  /// True when the decision is committed but the rebuild that should have
  /// followed it did not succeed. The read model is stale and the outstanding
  /// refresh is recorded durably, so [`CurationService::refresh_read_models`]
  /// can complete it without repeating the decision.
  pub read_model_refresh_pending: bool,
}

impl CurationOutcome {
  #[must_use]
  pub const fn new(assertion_id: i64, decision: Decision) -> Self {
    Self {
      schema_version: REPORT_SCHEMA_VERSION,
      assertion_id,
      decision,
      measurement_canonicalized: false,
      flags_resolved: 0,
      read_model_refreshed: false,
      read_model_refresh_pending: false,
    }
  }
}

/// What a refresh retry settled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshOutcome {
  pub schema_version: u16,
  /// Whether the materialized views were actually rebuilt. False means nothing
  /// was outstanding, not that a rebuild failed -- a failure is an error.
  pub read_model_refreshed: bool,
  /// Outstanding refresh requests closed by this rebuild.
  pub requests_completed: u64,
}

impl RefreshOutcome {
  #[must_use]
  pub const fn nothing_pending() -> Self {
    Self {
      schema_version: REPORT_SCHEMA_VERSION,
      read_model_refreshed: false,
      requests_completed: 0,
    }
  }

  #[must_use]
  pub const fn refreshed(requests_completed: u64) -> Self {
    Self { schema_version: REPORT_SCHEMA_VERSION, read_model_refreshed: true, requests_completed }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Decision {
  Accepted,
  Rejected,
}

impl Decision {
  /// The `aircraft_ref.assertion_statuses` code this decision writes.
  #[must_use]
  pub const fn status_code(self) -> &'static str {
    match self {
      Self::Accepted => "ACCEPTED",
      Self::Rejected => "REJECTED",
    }
  }

  #[must_use]
  pub const fn is_accepted(self) -> bool {
    matches!(self, Self::Accepted)
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingFilter {
  /// Restrict to one entity, e.g. a single aircraft variant.
  pub entity_id: Option<i64>,
  /// Restrict to one field name, e.g. `performance.SPEED_CRUISE_BEST`.
  pub field_name: Option<String>,
  pub limit: u32,
}

#[async_trait]
pub trait CurationStore: Send + Sync {
  async fn pending(
    &self,
    filter: &PendingFilter,
  ) -> Result<Vec<PendingAssertion>, PersistenceError>;

  /// Applies the assertion status, canonical flag, and curation-flag changes in
  /// one transaction, enqueueing the read-model refresh that the changes make
  /// necessary within the same transaction, then performs that refresh after
  /// commit.
  ///
  /// A post-commit refresh failure is not an error: the decision is already
  /// durable, so it is reported as a successful outcome carrying
  /// `read_model_refresh_pending`, and the enqueued request keeps the stale
  /// read model recoverable through [`CurationStore::refresh_read_models`].
  async fn decide(
    &self,
    assertion_id: i64,
    decision: Decision,
  ) -> Result<CurationOutcome, CurationError>;

  /// Rebuilds the read model if any refresh request is outstanding, closing the
  /// requests the rebuild satisfied. Idempotent, and safe to call when nothing
  /// is pending.
  async fn refresh_read_models(&self) -> Result<RefreshOutcome, PersistenceError>;
}

#[derive(Debug, Error)]
pub enum CurationError {
  #[error("assertion {0} does not exist")]
  UnknownAssertion(i64),
  /// The assertion already carries the requested decision. Changing a decision
  /// is permitted; repeating one is not, so an unchanged state is never
  /// reported as a successful change.
  #[error("assertion {assertion_id} is already {status}")]
  AlreadyDecided { assertion_id: i64, status: String },
  /// A different assertion is already canonical for this field. Accepting this
  /// one would violate `uq_assertion_accepted` / `uq_perf_canonical`, so the
  /// competing decision has to be withdrawn first.
  #[error("field {field_name} already has an accepted assertion ({accepted_id})")]
  Conflict { field_name: String, accepted_id: i64 },
  /// A different snapshot of the same variant is already canonical. Market rows
  /// are unique per variant (`uq_val_canonical`, `uq_cs_canonical`) while their
  /// assertions are keyed per snapshot row, so `uq_assertion_accepted` cannot
  /// catch this the way it does for a measurement field. The standing snapshot
  /// has to be withdrawn before a newer one can be published.
  #[error(
    "variant already has a canonical {entity_type_code} snapshot ({blocking_id}); \
     withdraw it before accepting this one"
  )]
  SnapshotConflict { entity_type_code: String, blocking_id: i64 },
  /// The assertion does not describe the value its row would publish. Ingestion
  /// reuses an existing valuation or cost snapshot when a later artifact lands
  /// on the same (variant, date, source) key, recording the newer document's
  /// value as a fresh assertion against the older stored row. Accepting that
  /// assertion would publish a value it never asserted.
  #[error(
    "assertion {assertion_id} asserts {asserted} but {field_name} stores {stored}; \
     accepting it would publish a different value"
  )]
  ValueMismatch { assertion_id: i64, field_name: String, asserted: String, stored: String },
  #[error(transparent)]
  Persistence(#[from] PersistenceError),
}

impl CurationError {
  #[must_use]
  pub fn code(&self) -> &str {
    match self {
      Self::UnknownAssertion(_) => "UNKNOWN_ASSERTION",
      Self::AlreadyDecided { .. } => "ALREADY_DECIDED",
      Self::Conflict { .. } => "CURATION_CONFLICT",
      Self::SnapshotConflict { .. } => "SNAPSHOT_CONFLICT",
      Self::ValueMismatch { .. } => "VALUE_MISMATCH",
      Self::Persistence(error) => error.code(),
    }
  }
}

pub struct CurationService {
  store: Arc<dyn CurationStore>,
}

impl fmt::Debug for CurationService {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.debug_struct("CurationService").finish_non_exhaustive()
  }
}

impl CurationService {
  #[must_use]
  pub fn new(store: Arc<dyn CurationStore>) -> Self {
    Self { store }
  }

  pub async fn pending(
    &self,
    filter: &PendingFilter,
  ) -> Result<Vec<PendingAssertion>, PersistenceError> {
    self.store.pending(filter).await
  }

  pub async fn accept(&self, assertion_id: i64) -> Result<CurationOutcome, CurationError> {
    self.store.decide(assertion_id, Decision::Accepted).await
  }

  pub async fn reject(&self, assertion_id: i64) -> Result<CurationOutcome, CurationError> {
    self.store.decide(assertion_id, Decision::Rejected).await
  }

  /// Completes a refresh a committed decision could not, without asking for
  /// another decision.
  pub async fn refresh_read_models(&self) -> Result<RefreshOutcome, PersistenceError> {
    self.store.refresh_read_models().await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decisions_map_to_the_seeded_assertion_status_codes() {
    assert_eq!(Decision::Accepted.status_code(), "ACCEPTED");
    assert_eq!(Decision::Rejected.status_code(), "REJECTED");
    assert!(Decision::Accepted.is_accepted());
    assert!(!Decision::Rejected.is_accepted());
  }

  #[test]
  fn a_fresh_outcome_claims_no_side_effects() {
    let outcome = CurationOutcome::new(7, Decision::Rejected);
    assert_eq!(outcome.assertion_id, 7);
    assert!(!outcome.measurement_canonicalized);
    assert!(!outcome.read_model_refreshed);
    assert!(!outcome.read_model_refresh_pending);
    assert_eq!(outcome.flags_resolved, 0);
  }

  #[test]
  fn a_refresh_that_found_nothing_pending_does_not_claim_a_rebuild() {
    let idle = RefreshOutcome::nothing_pending();
    assert!(!idle.read_model_refreshed);
    assert_eq!(idle.requests_completed, 0);

    let settled = RefreshOutcome::refreshed(3);
    assert!(settled.read_model_refreshed);
    assert_eq!(settled.requests_completed, 3);
  }

  #[test]
  fn error_codes_are_stable_for_scripting() {
    assert_eq!(CurationError::UnknownAssertion(1).code(), "UNKNOWN_ASSERTION");
    assert_eq!(
      CurationError::AlreadyDecided { assertion_id: 1, status: "ACCEPTED".to_owned() }.code(),
      "ALREADY_DECIDED"
    );
    assert_eq!(
      CurationError::Conflict { field_name: "f".to_owned(), accepted_id: 2 }.code(),
      "CURATION_CONFLICT"
    );
    assert_eq!(
      CurationError::SnapshotConflict { entity_type_code: "VALUATION".to_owned(), blocking_id: 3 }
        .code(),
      "SNAPSHOT_CONFLICT"
    );
    assert_eq!(
      CurationError::ValueMismatch {
        assertion_id: 4,
        field_name: "papi_price_estimate".to_owned(),
        asserted: "325000".to_owned(),
        stored: "310000.00".to_owned(),
      }
      .code(),
      "VALUE_MISMATCH"
    );
  }
}
