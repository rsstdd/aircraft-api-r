//! Transactional curation decisions.
//!
//! Accepting an assertion has to move three things together — the assertion's
//! own status, the canonical flag on the measurement it backs, and any curation
//! flags it closes — and then rebuild the read model. All of it commits or none
//! of it does, so the read model can never advertise a value whose assertion was
//! not accepted.

use aircraft_app::curation::{
  CurationError, CurationOutcome, CurationStore, Decision, PendingAssertion, PendingFilter,
};
use aircraft_app::ingestion::PersistenceError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx_core::transaction::Transaction;
use sqlx_core::{query::query, query_scalar::query_scalar, row::Row};
use sqlx_postgres::{PgPool, Postgres};

use super::ingestion_repository::database_error;

/// Assertion field names are namespaced by the table their value lives in, as
/// written by `SqlxIngestionUnitOfWork::promote_measurements`.
const PERFORMANCE_PREFIX: &str = "performance.";
const WEIGHT_PREFIX: &str = "weight.";

#[derive(Clone, Debug)]
pub struct SqlxCurationStore {
  pool: PgPool,
}

impl SqlxCurationStore {
  #[must_use]
  pub const fn from_pool(pool: PgPool) -> Self {
    Self { pool }
  }
}

/// Where a market assertion publishes to.
///
/// Market data is gated at the snapshot rather than the row, because a cost
/// snapshot's totals are only meaningful together and
/// `mv_ownership_cost_summary` would otherwise report a confident `$0.00` for a
/// partly-curated one (migration 020). Accepting any assertion on a snapshot
/// therefore publishes it, and it stays published while any of its assertions
/// remains accepted.
const fn market_target(entity_type_code: &str) -> Option<&'static str> {
  match entity_type_code.as_bytes() {
    b"VALUATION" => Some("aircraft_market.valuations"),
    b"COST_SNAPSHOT" => Some("aircraft_market.cost_snapshots"),
    _ => None,
  }
}

/// Which measurement table, if any, a field name refers to.
fn measurement_target(field_name: &str) -> Option<(&'static str, &str)> {
  // Identity and market fields carry provenance but have no canonical flag, so
  // they fall through to None.
  field_name
    .strip_prefix(PERFORMANCE_PREFIX)
    .map(|code| ("aircraft_specs.performance_metrics", code))
    .or_else(|| {
      field_name.strip_prefix(WEIGHT_PREFIX).map(|code| ("aircraft_specs.weight_metrics", code))
    })
}

#[async_trait]
impl CurationStore for SqlxCurationStore {
  async fn pending(
    &self,
    filter: &PendingFilter,
  ) -> Result<Vec<PendingAssertion>, PersistenceError> {
    let limit = i64::from(filter.limit.max(1));
    let rows = query(
      "SELECT sa.id, sa.entity_type_code, sa.entity_id, sa.field_name,
                    sa.raw_value, sa.raw_unit, sa.asserted_numeric::text AS asserted_numeric,
                    sa.created_at, s.slug AS source_slug,
                    v.name AS entity_label
             FROM aircraft_prov.source_assertions sa
             JOIN aircraft_prov.source_documents sd ON sd.id = sa.source_document_id
             LEFT JOIN aircraft_prov.sources s ON s.id = sd.source_id
             LEFT JOIN aircraft_core.variants v
                    ON sa.entity_type_code = 'AIRCRAFT_VARIANT' AND v.id = sa.entity_id
             WHERE sa.status_code = 'PENDING'
               AND ($1::bigint IS NULL OR sa.entity_id = $1)
               AND ($2::text IS NULL OR sa.field_name = $2)
             ORDER BY sa.entity_id, sa.field_name, sa.id
             LIMIT $3",
    )
    .bind(filter.entity_id)
    .bind(filter.field_name.as_deref())
    .bind(limit)
    .fetch_all(&self.pool)
    .await
    .map_err(database_error)?;

    rows
      .iter()
      .map(|row| {
        Ok(PendingAssertion {
          assertion_id: row.get("id"),
          entity_type_code: row.get("entity_type_code"),
          entity_id: row.get("entity_id"),
          entity_label: row.try_get("entity_label").ok(),
          field_name: row.get("field_name"),
          raw_value: row.try_get("raw_value").ok(),
          raw_unit: row.try_get("raw_unit").ok(),
          asserted_numeric: row.try_get("asserted_numeric").ok(),
          source_slug: row.try_get("source_slug").ok(),
          created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
      })
      .collect()
  }

  async fn decide(
    &self,
    assertion_id: i64,
    decision: Decision,
  ) -> Result<CurationOutcome, CurationError> {
    let mut transaction = self.pool.begin().await.map_err(database_error)?;

    // Lock the assertion first so two curators cannot both accept a competing
    // value for the same field and race the partial unique index.
    let row = query(
      "SELECT entity_type_code, entity_id, field_name, status_code
             FROM aircraft_prov.source_assertions
             WHERE id = $1
             FOR UPDATE",
    )
    .bind(assertion_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?;

    let Some(row) = row else {
      return Err(CurationError::UnknownAssertion(assertion_id));
    };
    // Changing your mind is allowed — a curator must be able to withdraw a value
    // they accepted in error. Only repeating the decision already recorded is
    // refused, so a no-op cannot masquerade as a change.
    let status: String = row.get("status_code");
    if status == decision.status_code() {
      return Err(CurationError::AlreadyDecided { assertion_id, status });
    }
    let entity_type_code: String = row.get("entity_type_code");
    let entity_id: i64 = row.get("entity_id");
    let field_name: String = row.get("field_name");

    if decision.is_accepted() {
      let competitor: Option<i64> = query_scalar(
        "SELECT id FROM aircraft_prov.source_assertions
                 WHERE entity_type_code = $1 AND entity_id = $2 AND field_name = $3
                   AND is_accepted AND id <> $4
                 LIMIT 1",
      )
      .bind(&entity_type_code)
      .bind(entity_id)
      .bind(&field_name)
      .bind(assertion_id)
      .fetch_optional(&mut *transaction)
      .await
      .map_err(database_error)?;
      if let Some(accepted_id) = competitor {
        return Err(CurationError::Conflict { field_name, accepted_id });
      }
    }

    query(
      "UPDATE aircraft_prov.source_assertions
             SET status_code = $2, is_accepted = $3
             WHERE id = $1",
    )
    .bind(assertion_id)
    .bind(decision.status_code())
    .bind(decision.is_accepted())
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;

    let mut outcome = CurationOutcome::new(assertion_id, decision);
    let (canonicalized, refreshed) = publish_decision(
      &mut transaction,
      assertion_id,
      &entity_type_code,
      entity_id,
      &field_name,
      decision,
    )
    .await?;
    outcome.measurement_canonicalized = canonicalized;
    outcome.read_model_refreshed = refreshed;

    let flags = query(
      "UPDATE aircraft_prov.curation_flags
             SET status_code = $4, resolved_at = clock_timestamp(),
                 resolution_notes = $5
             WHERE entity_type_code = $1 AND entity_id = $2 AND field_name = $3
               AND status_code = 'OPEN'",
    )
    .bind(&entity_type_code)
    .bind(entity_id)
    .bind(&field_name)
    .bind(if decision.is_accepted() { "RESOLVED" } else { "DISMISSED" })
    .bind(format!("assertion {assertion_id} {}", decision.status_code().to_lowercase()))
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?
    .rows_affected();
    outcome.flags_resolved = flags;

    // Refreshing is the expensive part, so it is skipped when the decision
    // changed nothing the read model exposes.
    if outcome.read_model_refreshed {
      query("SELECT aircraft_read.refresh_search_matviews(FALSE)")
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    }

    transaction.commit().await.map_err(database_error)?;
    Ok(outcome)
  }
}

/// Applies an accepted or withdrawn decision to whatever the assertion backs,
/// returning `(canonicalized, read_model_changed)`.
///
/// An assertion may back a measurement row, a market snapshot, or nothing at all
/// — an identity assertion such as `name` carries provenance without any
/// publishable row behind it.
async fn publish_decision(
  transaction: &mut Transaction<'_, Postgres>,
  assertion_id: i64,
  entity_type_code: &str,
  entity_id: i64,
  field_name: &str,
  decision: Decision,
) -> Result<(bool, bool), CurationError> {
  if entity_type_code == "AIRCRAFT_VARIANT" {
    if let Some((table, metric_code)) = measurement_target(field_name) {
      // Migration 020 links each stored measurement to the exact assertion
      // that produced it. The variant and metric predicates defend that link's
      // semantic identity as well as following its foreign key.
      let affected = query(&format!(
        "UPDATE {table} SET is_canonical = $4
                 WHERE source_assertion_id = $1
                   AND variant_id = $2 AND metric_type_code = $3
                   AND is_canonical IS DISTINCT FROM $4"
      ))
      .bind(assertion_id)
      .bind(entity_id)
      .bind(metric_code)
      .bind(decision.is_accepted())
      .execute(&mut **transaction)
      .await
      .map_err(database_error)?
      .rows_affected();
      return Ok((decision.is_accepted() && affected > 0, affected > 0));
    }
  }

  if let Some(table) = market_target(entity_type_code) {
    // Market rows are keyed by the assertion's entity_id, and stay published
    // while any assertion on them is still accepted.
    let still_accepted: bool = query_scalar(
      "SELECT EXISTS(
                SELECT 1 FROM aircraft_prov.source_assertions
                WHERE entity_type_code = $1 AND entity_id = $2 AND is_accepted)",
    )
    .bind(entity_type_code)
    .bind(entity_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;

    let affected = query(&format!("UPDATE {table} SET is_canonical = $2 WHERE id = $1"))
      .bind(entity_id)
      .bind(still_accepted)
      .execute(&mut **transaction)
      .await
      .map_err(database_error)?
      .rows_affected();
    return Ok((still_accepted && affected > 0, affected > 0));
  }

  Ok((false, false))
}

#[cfg(test)]
mod tests {
  use super::{market_target, measurement_target};

  #[test]
  fn field_names_route_to_the_table_holding_their_value() {
    assert_eq!(
      measurement_target("performance.SPEED_CRUISE_BEST"),
      Some(("aircraft_specs.performance_metrics", "SPEED_CRUISE_BEST"))
    );
    assert_eq!(
      measurement_target("weight.WEIGHT_MTOW"),
      Some(("aircraft_specs.weight_metrics", "WEIGHT_MTOW"))
    );
  }

  #[test]
  fn fields_without_a_measurement_row_route_nowhere() {
    // Identity assertions carry provenance but back no measurement row.
    assert_eq!(measurement_target("name"), None);
    // Market assertions publish through market_target instead.
    assert_eq!(measurement_target("ANNUAL_INSPECTION"), None);
  }

  #[test]
  fn market_assertions_publish_through_their_snapshot() {
    assert_eq!(market_target("VALUATION"), Some("aircraft_market.valuations"));
    assert_eq!(market_target("COST_SNAPSHOT"), Some("aircraft_market.cost_snapshots"));
    assert_eq!(market_target("AIRCRAFT_VARIANT"), None);
  }
}
