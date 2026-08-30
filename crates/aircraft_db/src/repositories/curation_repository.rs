//! Transactional curation decisions.
//!
//! Accepting an assertion has to move three things together — the assertion's
//! own status, the canonical flag on the measurement it backs, and any curation
//! flags it closes. The decision commits before the read model is rebuilt, so a
//! refresh never holds the curation transaction open.
//!
//! That ordering means the rebuild can fail with the decision already durable,
//! so the decision also enqueues a refresh request (migration 022) inside its
//! own transaction. The request outlives a failed rebuild and is closed only by
//! one that succeeded, which is what makes a stale read model recoverable
//! without asking for a second decision the state machine would refuse.

use aircraft_app::curation::{
  CurationError, CurationOutcome, CurationStore, Decision, PendingAssertion, PendingFilter,
  RefreshOutcome,
};
use aircraft_app::ingestion::PersistenceError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx_core::transaction::Transaction;
use sqlx_core::{error::Error as SqlxError, query::query, query_scalar::query_scalar, row::Row};
use sqlx_postgres::{PgPool, PgRow, Postgres};
use tracing::warn;

use super::ingestion_repository::database_error;

/// Assertion field names are namespaced by the table their value lives in, as
/// written by `SqlxIngestionUnitOfWork::promote_measurements`.
const PERFORMANCE_PREFIX: &str = "performance.";
const WEIGHT_PREFIX: &str = "weight.";

/// Value written to `read_model_refresh_requests.requested_by` by this adapter.
const REFRESH_REQUESTED_BY: &str = "CURATION";

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

/// Valuation columns a field name may be compared against.
///
/// Whitelisted rather than interpolated: `field_name` is source-derived, and it
/// reaches a `format!`-built query below.
const fn valuation_column(field_name: &str) -> Option<&'static str> {
  match field_name.as_bytes() {
    b"papi_price_estimate" => Some("papi_price_estimate"),
    b"for_sale_count" => Some("for_sale_count"),
    _ => None,
  }
}

/// Aggregate-total columns a cost field name may be compared against.
///
/// The three `is_aggregate` cost codes (`aircraft_ref.cost_item_types`, seeded by
/// `database/seeds/002_lookup_seed_data.sql`) are routed to
/// `aircraft_market.cost_snapshot_totals` rather than `cost_line_items`
/// (`chk_cli_no_aggregate`, migration 012), so a line-item comparison alone can
/// never see them. Whitelisted for the same reason as [`valuation_column`].
const fn cost_total_column(field_name: &str) -> Option<&'static str> {
  match field_name.as_bytes() {
    b"TOTAL_COST_ANNUAL" => Some("total_annual_usd"),
    b"TOTAL_FIXED_COST" => None,
    b"TOTAL_VARIABLE_COST" => Some("total_variable_usd"),
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
      .map(|row| -> Result<PendingAssertion, SqlxError> {
        Ok(PendingAssertion {
          assertion_id: row.get("id"),
          entity_type_code: row.get("entity_type_code"),
          entity_id: row.get("entity_id"),
          entity_label: row.try_get::<Option<String>, _>("entity_label")?,
          field_name: row.get("field_name"),
          raw_value: row.try_get::<Option<String>, _>("raw_value")?,
          raw_unit: row.try_get::<Option<String>, _>("raw_unit")?,
          asserted_numeric: row.try_get::<Option<String>, _>("asserted_numeric")?,
          source_slug: row.try_get::<Option<String>, _>("source_slug")?,
          created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
      })
      .collect::<Result<_, _>>()
      .map_err(database_error)
  }

  async fn decide(
    &self,
    assertion_id: i64,
    decision: Decision,
  ) -> Result<CurationOutcome, CurationError> {
    let mut transaction = self.pool.begin().await.map_err(database_error)?;

    let assertions = lock_assertion_field(&mut transaction, assertion_id).await?;

    let Some(row) = assertions.iter().find(|row| row.get::<i64, _>("id") == assertion_id) else {
      return Err(CurationError::UnknownAssertion(assertion_id));
    };
    let status: String = row.get("status_code");
    if status == decision.status_code() {
      return Err(CurationError::AlreadyDecided { assertion_id, status });
    }
    let entity_type_code: String = row.get("entity_type_code");
    let entity_id: i64 = row.get("entity_id");
    let field_name: String = row.get("field_name");

    if decision.is_accepted() {
      let competitor = assertions
        .iter()
        .find(|row| row.get::<i64, _>("id") != assertion_id && row.get::<bool, _>("is_accepted"))
        .map(|row| row.get::<i64, _>("id"));
      if let Some(accepted_id) = competitor {
        return Err(CurationError::Conflict { field_name, accepted_id });
      }
    }

    let updated = query(
      "UPDATE aircraft_prov.source_assertions
             SET status_code = $2, is_accepted = $3
             WHERE id = $1",
    )
    .bind(assertion_id)
    .bind(decision.status_code())
    .bind(decision.is_accepted())
    .execute(&mut *transaction)
    .await;
    if let Err(error) = updated {
      if decision.is_accepted() && is_assertion_acceptance_conflict(&error) {
        transaction.rollback().await.map_err(database_error)?;
        let accepted_id: i64 = query_scalar(
          "SELECT id FROM aircraft_prov.source_assertions
                 WHERE entity_type_code = $1 AND entity_id = $2 AND field_name = $3
                   AND is_accepted AND id <> $4
                 LIMIT 1",
        )
        .bind(&entity_type_code)
        .bind(entity_id)
        .bind(&field_name)
        .bind(assertion_id)
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        return Err(CurationError::Conflict { field_name, accepted_id });
      }
      return Err(database_error(error).into());
    }

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
    // `refreshed` says the read model's inputs moved, not that it was rebuilt;
    // whether the rebuild happened is only known after the commit below.

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

    // Enqueued inside the decision's transaction so the two facts -- what was
    // decided and that the read model no longer reflects it -- cannot diverge.
    if refreshed {
      enqueue_refresh_request(&mut transaction, assertion_id, decision).await?;
    }
    transaction.commit().await.map_err(database_error)?;

    if refreshed {
      match refresh_and_settle(&self.pool).await {
        Ok(_) => outcome.read_model_refreshed = true,
        Err(error) => {
          // The decision is durable, so a stale read model is reported rather
          // than raised: failing here would tell the caller the decision did
          // not happen, and no retry of it would ever be accepted.
          warn!(
            assertion_id,
            error = %error,
            "curation committed but the read model refresh failed; retry with `curate refresh`"
          );
          outcome.read_model_refreshed = false;
          outcome.read_model_refresh_pending = true;
        }
      }
    }
    Ok(outcome)
  }

  async fn refresh_read_models(&self) -> Result<RefreshOutcome, PersistenceError> {
    refresh_and_settle(&self.pool).await
  }
}

async fn enqueue_refresh_request(
  transaction: &mut Transaction<'_, Postgres>,
  assertion_id: i64,
  decision: Decision,
) -> Result<(), CurationError> {
  query(
    "INSERT INTO aircraft_read.read_model_refresh_requests(requested_by, reason)
         VALUES ($1, $2)",
  )
  .bind(REFRESH_REQUESTED_BY)
  .bind(format!("assertion {assertion_id} {}", decision.status_code()))
  .execute(&mut **transaction)
  .await
  .map_err(database_error)?;
  Ok(())
}

/// Rebuilds the read model when anything is outstanding and closes the requests
/// that rebuild satisfied.
///
/// Only requests already visible before the rebuild started are closed. One
/// committed during the rebuild may describe a change the rebuild did not read,
/// and leaving it outstanding costs one extra refresh, whereas closing it would
/// leave the read model quietly stale.
async fn refresh_and_settle(pool: &PgPool) -> Result<RefreshOutcome, PersistenceError> {
  let outstanding: Vec<i64> = query_scalar(
    "SELECT id FROM aircraft_read.read_model_refresh_requests
         WHERE status_code = 'PENDING' ORDER BY id",
  )
  .fetch_all(pool)
  .await
  .map_err(database_error)?;
  if outstanding.is_empty() {
    return Ok(RefreshOutcome::nothing_pending());
  }

  if let Err(error) =
    query("SELECT aircraft_read.refresh_search_matviews(FALSE)").execute(pool).await
  {
    let failure = database_error(error);
    record_refresh_failure(pool, &outstanding, &failure).await;
    return Err(failure);
  }

  let completed = query(
    "UPDATE aircraft_read.read_model_refresh_requests
         SET status_code = 'COMPLETED', completed_at = clock_timestamp()
         WHERE id = ANY($1) AND status_code = 'PENDING'",
  )
  .bind(&outstanding)
  .execute(pool)
  .await
  .map_err(database_error)?
  .rows_affected();
  Ok(RefreshOutcome::refreshed(completed))
}

/// Best-effort diagnostics on the outstanding requests.
///
/// The request rows are already durable, so they stay retryable whether or not
/// this bookkeeping lands; a second failure here must not mask the first.
async fn record_refresh_failure(pool: &PgPool, outstanding: &[i64], failure: &PersistenceError) {
  let recorded = query(
    "UPDATE aircraft_read.read_model_refresh_requests
         SET attempts = attempts + 1, last_error = $2
         WHERE id = ANY($1) AND status_code = 'PENDING'",
  )
  .bind(outstanding)
  // `PersistenceError`'s message is already control-stripped and length-bounded
  // by `database_error`.
  .bind(failure.to_string())
  .execute(pool)
  .await;
  if let Err(error) = recorded {
    warn!(error = %error, "could not record a read-model refresh failure");
  }
}

async fn lock_assertion_field(
  transaction: &mut Transaction<'_, Postgres>,
  assertion_id: i64,
) -> Result<Vec<PgRow>, CurationError> {
  query(
    "SELECT id, entity_type_code, entity_id, field_name, status_code, is_accepted
           FROM aircraft_prov.source_assertions
           WHERE (entity_type_code, entity_id, field_name) = (
             SELECT entity_type_code, entity_id, field_name
             FROM aircraft_prov.source_assertions WHERE id = $1
           )
           ORDER BY id
           FOR UPDATE",
  )
  .bind(assertion_id)
  .fetch_all(&mut **transaction)
  .await
  .map_err(|error| database_error(error).into())
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

    if decision.is_accepted()
      && let Some((asserted, stored)) =
        market_value_mismatch(transaction, assertion_id, entity_type_code, entity_id, field_name)
          .await?
    {
      return Err(CurationError::ValueMismatch {
        assertion_id,
        field_name: field_name.to_owned(),
        asserted,
        stored,
      });
    }

    // Market snapshots are unique per variant (uq_val_canonical, uq_cs_canonical)
    // while their assertions are keyed per snapshot row, so the uq_assertion_accepted
    // guard in `decide` cannot see a sibling snapshot of the same variant -- a
    // backfilled, next-day, or second-source row. Name the snapshot that stands
    // instead of letting the curator receive a raw constraint violation naming an
    // index. The index stays the authority: two curators publishing different
    // snapshots of one variant concurrently still lose that race there, not here.
    if still_accepted {
      let blocking_id: Option<i64> = query_scalar(&format!(
        "SELECT id FROM {table}
                 WHERE variant_id = (SELECT variant_id FROM {table} WHERE id = $1)
                   AND is_canonical AND id <> $1
                 LIMIT 1"
      ))
      .bind(entity_id)
      .fetch_optional(&mut **transaction)
      .await
      .map_err(database_error)?;
      if let Some(blocking_id) = blocking_id {
        return Err(CurationError::SnapshotConflict {
          entity_type_code: entity_type_code.to_owned(),
          blocking_id,
        });
      }
    }

    let affected = query(&format!(
      "UPDATE {table} SET is_canonical = $2
             WHERE id = $1 AND is_canonical IS DISTINCT FROM $2"
    ))
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

/// The value an accepted market assertion would publish, when the row it points
/// at stores a different one.
///
/// Ingestion reuses an existing valuation or cost snapshot when a later artifact
/// lands on the same (variant, date, source) key -- `uq_val_variant_date_source`
/// and `ON CONFLICT DO NOTHING` -- while still asserting the newer document's
/// value against that older row. Publishing on such an assertion would serve a
/// price or cost the assertion never carried.
///
/// A cost assertion is compared against whichever table its code was routed to:
/// an aggregate total against `cost_snapshot_totals`, everything else against
/// its `cost_line_items` row.
///
/// Only a positively identified disagreement is reported. An assertion with no
/// numeric value, a field with no comparable column, and a snapshot with no
/// matching row (an `EXTRA:` key, or a snapshot whose totals row was never
/// written) all return `None`, so this can refuse a decision only when it can
/// name both values.
async fn market_value_mismatch(
  transaction: &mut Transaction<'_, Postgres>,
  assertion_id: i64,
  entity_type_code: &str,
  entity_id: i64,
  field_name: &str,
) -> Result<Option<(String, String)>, CurationError> {
  let row = match entity_type_code {
    "VALUATION" => {
      let Some(column) = valuation_column(field_name) else { return Ok(None) };
      query(&format!(
        "SELECT sa.asserted_numeric::text AS asserted, valuation.{column}::text AS stored
                 FROM aircraft_prov.source_assertions sa
                 JOIN aircraft_market.valuations valuation ON valuation.id = $2
                 WHERE sa.id = $1
                   AND sa.asserted_numeric IS NOT NULL
                   AND sa.asserted_numeric IS DISTINCT FROM valuation.{column}::numeric"
      ))
      .bind(assertion_id)
      .bind(entity_id)
      .fetch_optional(&mut **transaction)
      .await
      .map_err(database_error)?
    }
    "COST_SNAPSHOT" => match cost_total_column(field_name) {
      Some(column) => query(&format!(
        "SELECT sa.asserted_numeric::text AS asserted, totals.{column}::text AS stored
                 FROM aircraft_prov.source_assertions sa
                 JOIN aircraft_market.cost_snapshot_totals totals ON totals.snapshot_id = $2
                 WHERE sa.id = $1
                   AND sa.asserted_numeric IS NOT NULL
                   AND sa.asserted_numeric IS DISTINCT FROM totals.{column}"
      ))
      .bind(assertion_id)
      .bind(entity_id)
      .fetch_optional(&mut **transaction)
      .await
      .map_err(database_error)?,
      None => query(
        "SELECT sa.asserted_numeric::text AS asserted,
                      COALESCE(line.amount_annual, line.amount_per_hour)::text AS stored
               FROM aircraft_prov.source_assertions sa
               JOIN aircraft_market.cost_line_items line
                 ON line.snapshot_id = $2 AND line.cost_item_type_code = $3
               WHERE sa.id = $1
                 AND sa.asserted_numeric IS NOT NULL
                 AND sa.asserted_numeric
                     IS DISTINCT FROM COALESCE(line.amount_annual, line.amount_per_hour)",
      )
      .bind(assertion_id)
      .bind(entity_id)
      .bind(field_name)
      .fetch_optional(&mut **transaction)
      .await
      .map_err(database_error)?,
    },
    _ => return Ok(None),
  };

  Ok(row.and_then(|row| {
    let asserted: Option<String> = row.get("asserted");
    let stored: Option<String> = row.get("stored");
    // A stored NULL is a real disagreement with a non-null assertion, and is
    // named as such rather than dropped.
    asserted.map(|asserted| (asserted, stored.unwrap_or_else(|| "no value".to_owned())))
  }))
}

fn is_assertion_acceptance_conflict(error: &SqlxError) -> bool {
  error.as_database_error().is_some_and(|error| {
    error.code().as_deref() == Some("23505") && error.constraint() == Some("uq_assertion_accepted")
  })
}

#[cfg(test)]
mod tests {
  use super::{cost_total_column, market_target, measurement_target};

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

  #[test]
  fn every_aggregate_cost_code_compares_against_its_totals_column() {
    // The three is_aggregate codes and their columns, read from
    // database/seeds/002_lookup_seed_data.sql and migration 012. A code that
    // stopped being comparable would let curation publish an unasserted total.
    const CASES: [(&str, Option<&str>); 5] = [
      ("TOTAL_COST_ANNUAL", Some("total_annual_usd")),
      ("TOTAL_FIXED_COST", Some("total_fixed_usd")),
      ("TOTAL_VARIABLE_COST", Some("total_variable_usd")),
      // Line items and unmapped keys stay with the cost_line_items comparison.
      ("ANNUAL_INSPECTION", None),
      ("EXTRA:total_cost_note", None),
    ];

    for (field_name, expected) in CASES {
      assert_eq!(cost_total_column(field_name), expected, "field {field_name}");
    }
  }
}
