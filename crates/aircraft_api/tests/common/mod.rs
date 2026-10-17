// `pub(crate)` is the honest visibility for an item a sibling test file uses,
// but a test binary's root has no external consumer, so `unreachable_pub`
// rejects plain `pub` here while `redundant_pub_crate` rejects `pub(crate)`
// inside a private module. The two cannot both be satisfied for a
// `tests/common` module; the narrower visibility wins and the lint that
// objects to it is allowed for this file alone.
#![allow(clippy::redundant_pub_crate, reason = "unreachable_pub rejects the alternative")]

//! Helpers shared by this crate's integration tests only.
//!
//! A `tests/common/mod.rs` and not `aircraft_testsupport`, because
//! `cargo run -p xtask -- boundaries` forbids that crate from depending on
//! `aircraft_domain`, and the catalog ports' signatures name `Slug`,
//! `FamilyId`, and `ModelId` from it. The shared harness therefore cannot
//! implement these ports at all, and this is the only home that both reaches
//! the domain vocabulary and is shared. `apps/server/tests/shutdown.rs` and the
//! two `#[cfg(test)]` modules under `src/` are outside any `tests/` tree and
//! keep their own copies for the same reason.

use aircraft_app::{
  catalog::{
    FamilyDetail, FamilyFilter, FamilyReader, FamilySummary, ModelDetail, ModelFilter, ModelReader,
    ModelSummary, VariantDetail, VariantFilter, VariantReadError, VariantReader, VariantSummary,
  },
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
};
use aircraft_domain::catalog::{FamilyId, ModelId, Slug};
use async_trait::async_trait;

/// Satisfies all three catalog reader ports and serves none of them.
///
/// `ApiState` carries the family, model, and variant readers because every
/// router registers the catalog routes, so every test that builds a state must
/// supply all three -- including those whose subject is shutdown, tracing,
/// authentication, or a problem document and which issue no catalog request.
///
/// It panics rather than returning an empty page: a test that unexpectedly
/// reaches a catalog read should fail loudly, not quietly observe a collection
/// it never asked for. A test whose subject *is* the catalog builds its own
/// recording fake instead, and several in this directory do.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UnreadCatalog;

#[async_trait]
impl FamilyReader for UnreadCatalog {
  async fn list_families(
    &self,
    _filter: &FamilyFilter,
    _limit: PageLimit,
    _after: Option<&Slug>,
  ) -> Result<Page<FamilySummary, Slug>, PersistenceError> {
    panic!("this test builds a state without expecting a family list")
  }

  async fn family(&self, _slug: &Slug) -> Result<Option<FamilyDetail>, PersistenceError> {
    panic!("this test builds a state without expecting a family read")
  }

  async fn family_id(&self, _slug: &Slug) -> Result<Option<FamilyId>, PersistenceError> {
    panic!("this test builds a state without expecting a family resolution")
  }
}

#[async_trait]
impl ModelReader for UnreadCatalog {
  async fn list_models(
    &self,
    _filter: &ModelFilter,
    _limit: PageLimit,
    _after: Option<&Slug>,
  ) -> Result<Page<ModelSummary, Slug>, PersistenceError> {
    panic!("this test builds a state without expecting a model list")
  }

  async fn model(&self, _slug: &Slug) -> Result<Option<ModelDetail>, PersistenceError> {
    panic!("this test builds a state without expecting a model read")
  }

  async fn model_id(&self, _slug: &Slug) -> Result<Option<ModelId>, PersistenceError> {
    panic!("this test builds a state without expecting a model resolution")
  }
}

#[async_trait]
impl VariantReader for UnreadCatalog {
  async fn list_variants(
    &self,
    _filter: &VariantFilter,
    _limit: PageLimit,
    _after: Option<&Slug>,
  ) -> Result<Page<VariantSummary, Slug>, VariantReadError> {
    panic!("this test builds a state without expecting a variant list")
  }

  async fn variant(&self, _slug: &Slug) -> Result<Option<VariantDetail>, PersistenceError> {
    panic!("this test builds a state without expecting a variant read")
  }
}
