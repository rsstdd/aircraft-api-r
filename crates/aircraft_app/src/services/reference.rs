//! Reference-catalog read port.
//!
//! `GET /v1/reference/{catalog}` serves the seeded `aircraft_ref` lookups, but
//! `aircraft_api` may depend on neither `aircraft_db` nor `SQLx`: both are
//! refused by `cargo run -p xtask -- boundaries`. The reader is therefore a port
//! declared here, implemented over a pool in `aircraft_db`, and injected by
//! `apps/server` -- the same arrangement, and for the same reason, as
//! [`crate::readiness`].

use aircraft_domain::reference::Catalog;
use async_trait::async_trait;

use super::ingestion::PersistenceError;

/// The most rows one catalog may return.
///
/// Root `AGENTS.md` requires every read to be bounded, and these tables are
/// operator-writable even though the catalogs themselves are a closed set: an
/// unbounded `SELECT` would let one grown table decide the response size. The
/// ceiling is far above the largest seeded catalog -- `aircraft_roles` seeds 57
/// rows from `database/seeds/002_lookup_seed_data.sql` -- so it bounds a
/// pathological table rather than trimming a real one.
///
/// It is deliberately not a client-supplied page size. A lookup vocabulary is
/// answered whole so that one `ETag` validates the whole catalog; see
/// `docs/architecture/http_v1_decisions.md` under "Conditional requests and
/// reference catalogs".
pub const MAX_CATALOG_ROWS: usize = 1000;

/// One published lookup row.
///
/// The three members every catalog can answer. Catalog-specific columns --
/// `measurement_units.symbol`, `currencies.decimal_places`,
/// `certification_authorities.country_codes` -- are deliberately outside this
/// contract, which publishes the common lookup shape only.
///
/// There is no `sort_order` member. It orders the result and array position
/// already conveys that to a client; publishing the number would invite sorting
/// by a value this contract does not promise to keep stable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEntry {
  pub code: String,
  pub label: String,
  pub description: Option<String>,
}

#[async_trait]
pub trait CatalogReader: Send + Sync {
  /// Reads one catalog's active rows, in that catalog's stable order.
  ///
  /// Takes a [`Catalog`] and not a name, so an adapter cannot be handed a table
  /// this vocabulary never admitted: the allowlist is enforced by the type
  /// rather than by each implementation remembering to check.
  ///
  /// # Errors
  ///
  /// [`PersistenceError::Database`] when the query fails, and
  /// [`PersistenceError::Invariant`] when a catalog holds more than
  /// [`MAX_CATALOG_ROWS`] rows -- refused rather than truncated, because a
  /// silently short catalog is indistinguishable from a complete one.
  async fn entries(&self, catalog: Catalog) -> Result<Vec<CatalogEntry>, PersistenceError>;
}
