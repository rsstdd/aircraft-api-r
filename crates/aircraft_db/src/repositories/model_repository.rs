//! The model adapter behind `aircraft_app`'s catalog ports.
//!
//! Implements [`ModelReader`] over the pool built by
//! [`connect`](crate::pool::connect), reading `aircraft_core.models` as
//! `database/migrations/004_aircraft_identity_taxonomy.sql` declares it. As with
//! [`family_repository`](crate::repositories::family_repository), the
//! projections and the port are `aircraft_app::catalog`'s; this module owns only
//! the statements and the row conversion.
//!
//! The runtime role's access to `aircraft_core.models` is the column-level
//! `SELECT` in `database/roles/app_grants.sql`, which names this file in turn. A
//! column added to a statement here without a grant there fails with `42501`,
//! and `the_runtime_role_reads_the_catalog_and_writes_none` in
//! `crates/aircraft_db/tests/family_repository.rs` is what catches it: every
//! other test connects as the container owner and cannot see a missing grant.

use aircraft_app::{
  catalog::{ModelDetail, ModelFilter, ModelReader, ModelSummary},
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
};
use aircraft_domain::catalog::{FamilyId, Slug};
use async_trait::async_trait;
use sqlx_core::{query::query, row::Row};
use sqlx_postgres::{PgPool, PgRow};

use crate::repositories::ingestion_repository::database_error;

/// The published summary columns, and the parent family's public identifier.
///
/// `LEFT JOIN` although `family_id` is `NOT NULL` with `ON DELETE RESTRICT`
/// (`004:74-75`): an inner join would turn a model whose parent had somehow gone
/// into a row that is silently not there, while the outer join hands the
/// missing family to [`summary`], which refuses it by name. The filter is on
/// `m.family_id`, not on the joined slug, so it holds whether or not the join
/// found a row.
///
/// Ordered by `slug` alone and resumed by `slug > $2`, for the reason
/// `family_repository::LIST` gives: `slug` is `NOT NULL UNIQUE` (`004:76`), so
/// the order is total and the resume exact. The family predicate and the
/// cursor predicate are joined by `AND`, so a filtered walk never crosses into a
/// neighbouring family between pages.
const LIST: &str = "\
SELECT m.slug, m.name, m.display_name, f.slug AS family, \
       m.series, m.generation, m.first_flight_year, m.certification_year \
  FROM aircraft_core.models m \
  LEFT JOIN aircraft_core.families f ON f.id = m.family_id \
 WHERE ($1::bigint IS NULL OR m.family_id = $1) \
   AND ($2::text IS NULL OR m.slug > $2) \
 ORDER BY m.slug \
 LIMIT $3";

/// The summary columns plus the two a detail adds.
const DETAIL: &str = "\
SELECT m.slug, m.name, m.display_name, f.slug AS family, \
       m.series, m.generation, m.first_flight_year, m.certification_year, \
       m.name_aliases, m.description \
  FROM aircraft_core.models m \
  LEFT JOIN aircraft_core.families f ON f.id = m.family_id \
 WHERE m.slug = $1";

/// A stored value the catalog contract cannot represent.
///
/// Names the column and never the value, as `family_repository::unrepresentable`
/// does and for the same reason: the row is operator data and stays out of a
/// diagnostic that reaches a log. Here it also covers the parent that is not
/// there: `ModelSummary::family` is a [`Slug`], not an option, because the
/// schema promises every model a family, and a row that breaks that promise is
/// a refusal rather than a fabricated or omitted parent.
fn unrepresentable(column: &str) -> PersistenceError {
  PersistenceError::Invariant(format!(
    "{column} holds a value the catalog contract cannot represent"
  ))
}

/// The summary columns every model read projects.
///
/// `try_get` rather than indexing, for the reason
/// `reference_repository::SqlxCatalogReader::entries` gives: the indexing form
/// panics on a column whose name or type does not match.
fn summary(row: &PgRow) -> Result<ModelSummary, PersistenceError> {
  let slug: String = row.try_get("slug").map_err(database_error)?;
  let family: Option<String> = row.try_get("family").map_err(database_error)?;

  Ok(ModelSummary {
    slug: Slug::try_from(slug.as_str())
      .map_err(|_| unrepresentable("aircraft_core.models.slug"))?,
    name: row.try_get("name").map_err(database_error)?,
    display_name: row.try_get("display_name").map_err(database_error)?,
    family: family.ok_or_else(|| unrepresentable("aircraft_core.models.family_id")).and_then(
      |value| {
        Slug::try_from(value.as_str()).map_err(|_| unrepresentable("aircraft_core.families.slug"))
      },
    )?,
    series: row.try_get("series").map_err(database_error)?,
    generation: row.try_get("generation").map_err(database_error)?,
    first_flight_year: row.try_get("first_flight_year").map_err(database_error)?,
    certification_year: row.try_get("certification_year").map_err(database_error)?,
  })
}

/// Serves models from the application pool.
#[derive(Debug)]
pub struct SqlxModelReader {
  pool: PgPool,
}

impl SqlxModelReader {
  /// Shares the pool the server already holds, as
  /// [`SqlxFamilyReader::new`](crate::SqlxFamilyReader::new) does and for the
  /// same reason: the bounds an operator configured are the process's.
  #[must_use]
  pub const fn new(pool: PgPool) -> Self {
    Self { pool }
  }
}

#[async_trait]
impl ModelReader for SqlxModelReader {
  /// # Errors
  ///
  /// [`PersistenceError::Database`] carrying the sanitized `SQLx` failure, and
  /// [`PersistenceError::Invariant`] when a stored value cannot become the
  /// domain type the contract publishes, including a model whose family row is
  /// gone.
  async fn list_models(
    &self,
    filter: &ModelFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<ModelSummary, Slug>, PersistenceError> {
    let rows = query(LIST)
      .bind(filter.family.map(FamilyId::get))
      .bind(after.map(Slug::as_str))
      // One row past the page, so `Page::from_overfetched` can tell a full page
      // from a final one without a second query.
      .bind(i64::from(limit.query_size()))
      .fetch_all(&self.pool)
      .await
      .map_err(database_error)?;

    let summaries =
      rows.iter().map(summary).collect::<Result<Vec<ModelSummary>, PersistenceError>>()?;

    Ok(Page::from_overfetched(summaries, limit, |row: &ModelSummary| row.slug.clone()))
  }

  /// # Errors
  ///
  /// As [`Self::list_models`]. A slug no model carries is `Ok(None)`: absence is
  /// not a failure, and turning it into `404` is the HTTP boundary's decision.
  async fn model(&self, slug: &Slug) -> Result<Option<ModelDetail>, PersistenceError> {
    let Some(row) =
      query(DETAIL).bind(slug.as_str()).fetch_optional(&self.pool).await.map_err(database_error)?
    else {
      return Ok(None);
    };

    let aliases: Option<Vec<String>> = row.try_get("name_aliases").map_err(database_error)?;

    Ok(Some(ModelDetail {
      summary: summary(&row)?,
      // A NULL array and an empty one say the same thing to a reader, which is
      // what `ModelDetail::name_aliases` being a `Vec` already promises.
      name_aliases: aliases.unwrap_or_default(),
      description: row.try_get("description").map_err(database_error)?,
    }))
  }
}
