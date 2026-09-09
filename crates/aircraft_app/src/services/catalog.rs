//! What a catalog read returns, and the ports that serve it.
//!
//! Transport-independent by construction: nothing here names Axum, `SQLx`, or
//! a database row, because `aircraft_api` and `aircraft_db` are both adapters
//! onto these contracts and `cargo run -p xtask -- boundaries` refuses the
//! dependency either way. The identity values are `aircraft_domain::catalog`'s,
//! which names this module in turn.
//!
//! **Slugs name, identifiers refer.** A caller reaches a row by its slug; a
//! typed identifier appears only once a row has been resolved, as a filter on a
//! child collection. That is also why a page resumes from a [`Slug`]: it is
//! `NOT NULL UNIQUE` on all three tables in
//! `database/migrations/004_aircraft_identity_taxonomy.sql` (`:35`, `:76`,
//! `:106`), so it is a total tiebreaker, and it is a value the caller holds.
//!
//! **Summary against detail.** A summary carries what a collection row needs to
//! identify an aircraft and situate it -- its names, its parent, its lineage
//! codes and years. A detail adds the prose and the alias list, which no list
//! view reads. The split is a contract decision, not an optimisation.
//!
//! **What is deliberately not published.** Migration `004` also carries the
//! surrogate `id` of each row, the generated `name_tsv` and `description_tsv`
//! search vectors, the open-ended `extra_attributes` JSONB, the row timestamps,
//! and -- on variants -- the ingestion staging columns `ingest_key` and
//! `source_path`. None is aircraft data a client asked for, and `source_path` is
//! a host path that `rust-production` forbids leaving the boundary at all.
//!
//! Nothing here holds that list against the schema, and deliberately so: an
//! applied migration is immutable, so a column added later arrives in a *later*
//! migration, and a test reading `004` alone would pass while missing it. The
//! check belongs where the adapter meets the installed database -- reading
//! `information_schema` in a disposable `PostgreSQL`, as
//! `crates/aircraft_db/tests/reference_catalogs.rs` already does for the lookup
//! catalogs -- and arrives with the repository that first maps these rows.

use aircraft_domain::catalog::{CountryCode, FamilyId, LookupCode, ModelId, Slug};
use async_trait::async_trait;

use super::ingestion::PersistenceError;
use crate::pagination::{Page, PageLimit};

/// A family as a collection row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FamilySummary {
  pub slug: Slug,
  pub name: String,
  pub common_name: Option<String>,
  /// The manufacturing organisation's own slug. The column is a key into
  /// `aircraft_org.organizations`, so an adapter resolves it; a surrogate key
  /// is not a public identifier.
  pub manufacturer: Option<Slug>,
  pub country_of_origin: Option<CountryCode>,
  /// `aircraft_ref.year_value`, a `SMALLINT` the schema constrains to
  /// 1900-2100. Carried as the schema's own width so a read cannot lose one.
  pub first_flight_year: Option<i16>,
}

/// A family, whole.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FamilyDetail {
  pub summary: FamilySummary,
  /// The `name_aliases` array, empty rather than optional: a `NULL` array and an
  /// empty one say the same thing to a reader, and collapsing them here keeps
  /// every consumer from having to decide again.
  pub name_aliases: Vec<String>,
  pub description: Option<String>,
}

/// A model as a collection row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSummary {
  pub slug: Slug,
  pub name: String,
  pub display_name: Option<String>,
  /// The parent family's slug.
  pub family: Slug,
  pub series: Option<String>,
  pub generation: Option<i16>,
  pub first_flight_year: Option<i16>,
  pub certification_year: Option<i16>,
}

/// A model, whole.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelDetail {
  pub summary: ModelSummary,
  pub name_aliases: Vec<String>,
  pub description: Option<String>,
}

/// A variant as a collection row.
///
/// Wider than the other two because a variant is what a client compares: the
/// lineage codes, the production window, and the capacities are what a list view
/// filters and ranks on, and withholding them would make every row a second
/// request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantSummary {
  pub slug: Slug,
  pub name: String,
  pub popular_name: Option<String>,
  /// The parent model's slug.
  pub model: Slug,
  pub variant_type: Option<LookupCode>,
  pub service_status: Option<LookupCode>,
  pub country_of_origin: Option<CountryCode>,
  pub first_flight_year: Option<i16>,
  pub certification_year: Option<i16>,
  pub production_start_year: Option<i16>,
  pub production_end_year: Option<i16>,
  pub passenger_capacity: Option<i16>,
  pub crew_count: Option<i16>,
  pub engine_count: Option<i16>,
  pub landing_gear_type: Option<LookupCode>,
  pub propulsion_category: Option<LookupCode>,
  pub is_in_production: Option<bool>,
}

/// A variant, whole.
///
/// There is no alias list here: a variant's aliases are rows of
/// `aircraft_core.variant_aliases`, a table this contract does not read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantDetail {
  pub summary: VariantSummary,
  pub description: Option<String>,
}

/// Which families a read returns. Every field absent means unfiltered.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FamilyFilter {
  pub manufacturer: Option<Slug>,
  pub country_of_origin: Option<CountryCode>,
}

/// Which models a read returns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelFilter {
  /// A resolved parent, not a slug: the caller that has a family to filter by
  /// has already looked it up, and a second lookup inside the adapter would be
  /// a query nobody asked for.
  pub family: Option<FamilyId>,
}

/// Which variants a read returns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VariantFilter {
  pub model: Option<ModelId>,
  pub variant_type: Option<LookupCode>,
  pub service_status: Option<LookupCode>,
  pub landing_gear_type: Option<LookupCode>,
  pub propulsion_category: Option<LookupCode>,
  pub country_of_origin: Option<CountryCode>,
  pub is_in_production: Option<bool>,
}

/// Reads families.
///
/// Three ports rather than one, because `aircraft_db` implements them in
/// separate stories: a single trait would force one adapter to exist before
/// either half was ready.
///
/// # Errors
///
/// [`PersistenceError`] as the repository reported it. A row that does not
/// exist is `Ok(None)` and not an error -- absence is not a failure, and turning
/// it into `404` is the HTTP boundary's decision, which is why no `NotFound`
/// variant exists to prejudge it. `CredentialLookup::resolve` reads the same
/// way.
#[async_trait]
pub trait FamilyReader: Send + Sync {
  /// One page of families, bounded by `limit` and resuming after `after`.
  async fn list_families(
    &self,
    filter: &FamilyFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<FamilySummary, Slug>, PersistenceError>;

  async fn family(&self, slug: &Slug) -> Result<Option<FamilyDetail>, PersistenceError>;
}

/// Reads models. See [`FamilyReader`] for the error and absence contract.
#[async_trait]
pub trait ModelReader: Send + Sync {
  async fn list_models(
    &self,
    filter: &ModelFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<ModelSummary, Slug>, PersistenceError>;

  async fn model(&self, slug: &Slug) -> Result<Option<ModelDetail>, PersistenceError>;
}

/// Reads variants. See [`FamilyReader`] for the error and absence contract.
#[async_trait]
pub trait VariantReader: Send + Sync {
  async fn list_variants(
    &self,
    filter: &VariantFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<VariantSummary, Slug>, PersistenceError>;

  async fn variant(&self, slug: &Slug) -> Result<Option<VariantDetail>, PersistenceError>;
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used, clippy::panic)]

  use std::num::NonZeroU16;

  use super::*;

  fn slug(value: &str) -> Slug {
    Slug::try_from(value).expect("a slug_text value")
  }

  fn summary(name: &str) -> VariantSummary {
    VariantSummary {
      slug: slug(name),
      name: name.to_owned(),
      popular_name: None,
      model: slug("cessna-172"),
      variant_type: None,
      service_status: None,
      country_of_origin: None,
      first_flight_year: None,
      certification_year: None,
      production_start_year: None,
      production_end_year: None,
      passenger_capacity: None,
      crew_count: None,
      engine_count: None,
      landing_gear_type: None,
      propulsion_category: None,
      is_in_production: None,
    }
  }

  fn limit(value: u16) -> PageLimit {
    PageLimit::from_requested(NonZeroU16::new(value))
  }

  /// Only what this port owns: the summaries reach the caller as the repository
  /// built them. How a page is cut and where its continuation comes from is
  /// `crate::pagination`'s, and `only_a_returned_lookahead_row_yields_a_continuation`
  /// already proves it.
  #[tokio::test]
  async fn a_populated_read_returns_the_summaries_unchanged() {
    let reader = FakeCatalog::serving(vec![summary("a"), summary("b")]);

    let page = reader
      .list_variants(&VariantFilter::default(), limit(50), None)
      .await
      .expect("a populated read");

    assert_eq!(
      page.items().iter().map(|row| row.slug.as_str()).collect::<Vec<_>>(),
      ["a", "b"],
      "the port passes its rows through in order"
    );
  }

  #[tokio::test]
  async fn an_empty_collection_is_a_successful_empty_page() {
    let reader = FakeCatalog::serving(Vec::new());

    let page = reader
      .list_variants(&VariantFilter::default(), limit(2), None)
      .await
      .expect("an empty collection is not a failure");

    assert!(page.items().is_empty());
    assert!(page.next().is_none(), "an empty page has no continuation");
  }

  #[tokio::test]
  async fn a_missing_detail_is_ok_none_rather_than_an_error() {
    let reader = FakeCatalog::serving(Vec::new());

    // `matches!` rather than equality: `PersistenceError` carries no `PartialEq`,
    // and widening a shared error type to shorten a test is not this issue's
    // change. The three arms still separate `Ok(None)` from `Ok(Some)` and from
    // `Err`, which is the whole distinction.
    assert!(matches!(reader.variant(&slug("nothing-here")).await, Ok(None)), "a variant");
    assert!(matches!(reader.family(&slug("nothing-here")).await, Ok(None)), "a family");
    assert!(matches!(reader.model(&slug("nothing-here")).await, Ok(None)), "a model");
  }

  #[tokio::test]
  async fn a_repository_failure_propagates_its_exact_variant() {
    let reader = FakeCatalog::failing();

    let listed = reader.list_variants(&VariantFilter::default(), limit(2), None).await;
    let detailed = reader.variant(&slug("cessna-172s")).await;

    for outcome in [listed.err(), detailed.err()] {
      match outcome {
        Some(PersistenceError::Database { code, .. }) => assert_eq!(code, "57P01"),
        other => panic!("the port must pass the repository's own failure through, got {other:?}"),
      }
    }
  }

  /// All three ports are usable seams, not just the one the cases above drive.
  #[tokio::test]
  async fn a_family_and_a_model_are_read_through_their_own_ports() {
    let reader = FakeCatalog::serving(Vec::new());

    let families =
      reader.list_families(&FamilyFilter::default(), limit(2), None).await.expect("a family read");
    let models = reader
      .list_models(&ModelFilter { family: Some(FamilyId::new(1)) }, limit(2), None)
      .await
      .expect("a model read");

    assert!(families.items().is_empty() && models.items().is_empty());
  }

  /// Answers every read from one canned outcome and counts nothing: these tests
  /// are about the shape of the contract, not about how often it is called.
  ///
  /// The failure is rebuilt per call rather than stored, because
  /// `PersistenceError` is not `Clone` and widening a shared error type to
  /// shorten a test is not this issue's change.
  struct FakeCatalog {
    variants: Vec<VariantSummary>,
    failing: bool,
  }

  impl FakeCatalog {
    fn serving(variants: Vec<VariantSummary>) -> Self {
      Self { variants, failing: false }
    }

    fn failing() -> Self {
      Self { variants: Vec::new(), failing: true }
    }

    /// The failure a repository would report: `57P01` is `PostgreSQL`'s
    /// admin-shutdown class, so the test asserts a code an adapter could really
    /// produce rather than an invented one.
    fn failure(&self) -> Option<PersistenceError> {
      self.failing.then(|| PersistenceError::Database {
        code: "57P01".to_owned(),
        message: "terminating connection due to administrator command".to_owned(),
      })
    }
  }

  #[async_trait]
  impl FamilyReader for FakeCatalog {
    async fn list_families(
      &self,
      _filter: &FamilyFilter,
      limit: PageLimit,
      _after: Option<&Slug>,
    ) -> Result<Page<FamilySummary, Slug>, PersistenceError> {
      self.failure().map_or_else(
        || Ok(Page::from_overfetched(Vec::new(), limit, |row: &FamilySummary| row.slug.clone())),
        Err,
      )
    }

    async fn family(&self, _slug: &Slug) -> Result<Option<FamilyDetail>, PersistenceError> {
      self.failure().map_or_else(|| Ok(None), Err)
    }
  }

  #[async_trait]
  impl ModelReader for FakeCatalog {
    async fn list_models(
      &self,
      _filter: &ModelFilter,
      limit: PageLimit,
      _after: Option<&Slug>,
    ) -> Result<Page<ModelSummary, Slug>, PersistenceError> {
      self.failure().map_or_else(
        || Ok(Page::from_overfetched(Vec::new(), limit, |row: &ModelSummary| row.slug.clone())),
        Err,
      )
    }

    async fn model(&self, _slug: &Slug) -> Result<Option<ModelDetail>, PersistenceError> {
      self.failure().map_or_else(|| Ok(None), Err)
    }
  }

  #[async_trait]
  impl VariantReader for FakeCatalog {
    async fn list_variants(
      &self,
      _filter: &VariantFilter,
      limit: PageLimit,
      _after: Option<&Slug>,
    ) -> Result<Page<VariantSummary, Slug>, PersistenceError> {
      if let Some(failure) = self.failure() {
        return Err(failure);
      }
      Ok(Page::from_overfetched(self.variants.clone(), limit, |row: &VariantSummary| {
        row.slug.clone()
      }))
    }

    async fn variant(&self, _slug: &Slug) -> Result<Option<VariantDetail>, PersistenceError> {
      self.failure().map_or_else(|| Ok(None), Err)
    }
  }
}
