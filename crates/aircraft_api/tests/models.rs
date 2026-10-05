// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used, clippy::panic)]

//! `GET /v1/models` and `GET /v1/models/{model}`, driven through the real
//! perimeter over fake ports.
//!
//! The handlers are registered under `Public` at `/__test/...` paths, which is
//! what `crates/aircraft_api/src/lib.rs` does for the catalog handler and for
//! the reason recorded there: it drives the handler's own behaviour without a
//! credential. What that deliberately does not prove is the policy. That the
//! two model routes are `CatalogRead` is
//! `every_served_route_is_registered_once_and_the_operational_routes_are_public`'s
//! job, and that a scoped registration refuses a caller without the scope is
//! proven once for every policy in `crates/aircraft_api/src/authentication.rs`;
//! `an_anonymous_request_to_a_shipped_model_route_is_unauthorized` below closes
//! the loop by driving the shipped router's real paths.

use std::{
  sync::{Arc, Mutex},
  time::Duration,
};

use aircraft_api::{
  ApiState, ApplicationRouter, PerimeterLimits,
  pagination::{CursorPosition, FilterFingerprint, encode},
  rate_limit::{Quota, RateLimitPolicy, RateLimiter},
  router, router_with_routes,
  routes::{RouteMethod, RoutePolicy, Routes},
  shutdown::ShutdownState,
};
use aircraft_app::{
  authentication::{AuthenticationService, CredentialLookup, CredentialLookupRecord},
  catalog::{
    FamilyDetail, FamilyFilter, FamilyReader, FamilySummary, ModelDetail, ModelFilter, ModelReader,
    ModelSummary,
  },
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
  readiness::ReadinessProbe,
  reference::{CatalogEntry, CatalogReader},
};
use aircraft_domain::{
  catalog::{FamilyId, Slug},
  reference::Catalog,
};
use aircraft_testsupport::TestResult;
use async_trait::async_trait;
use axum::{
  body::{Body, to_bytes},
  http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const LIST: &str = "/__test/models";
const DETAIL: &str = "/__test/models/{model}";
const CESSNA: i64 = 7;

struct AlwaysReady;

#[async_trait]
impl ReadinessProbe for AlwaysReady {
  async fn check(&self) -> Result<(), PersistenceError> {
    Ok(())
  }
}

struct NeverLooksUp;

#[async_trait]
impl CredentialLookup for NeverLooksUp {
  async fn resolve(
    &self,
    _key_id: Uuid,
  ) -> Result<Option<CredentialLookupRecord>, PersistenceError> {
    Ok(None)
  }
}

struct NoCatalogs;

#[async_trait]
impl CatalogReader for NoCatalogs {
  async fn entries(&self, _catalog: Catalog) -> Result<Vec<CatalogEntry>, PersistenceError> {
    panic!("no route here reads a reference catalog")
  }
}

/// Resolves exactly one slug, and records every slug it was asked for.
///
/// The recording is what proves ordering: a request whose filter is refused
/// before the lookup leaves this empty, and one refused after leaves it with
/// the slug in it.
#[derive(Default)]
struct StubFamilies {
  asked: Mutex<Vec<String>>,
  failing: bool,
}

#[async_trait]
impl FamilyReader for StubFamilies {
  async fn list_families(
    &self,
    _filter: &FamilyFilter,
    _limit: PageLimit,
    _after: Option<&Slug>,
  ) -> Result<Page<FamilySummary, Slug>, PersistenceError> {
    panic!("no route here lists families")
  }

  async fn family(&self, _slug: &Slug) -> Result<Option<FamilyDetail>, PersistenceError> {
    panic!("no route here reads a family")
  }

  async fn family_id(&self, slug: &Slug) -> Result<Option<FamilyId>, PersistenceError> {
    self.asked.lock().expect("an uncontended lock").push(slug.as_str().to_owned());
    if self.failing {
      return Err(PersistenceError::Database {
        code: "57P01".to_owned(),
        message: "terminating connection due to administrator command".to_owned(),
      });
    }
    Ok((slug.as_str() == "cessna").then(|| FamilyId::new(CESSNA)))
  }
}

/// Serves a fixed set of rows and records what the handler asked for.
#[derive(Default)]
struct StubModels {
  rows: Vec<ModelSummary>,
  detail: Option<ModelDetail>,
  failing: Option<PersistenceError>,
  calls: Mutex<Vec<(Option<i64>, Option<String>)>>,
}

impl StubModels {
  fn failure(&self) -> Option<PersistenceError> {
    self.failing.as_ref().map(|error| match error {
      PersistenceError::Database { code, message } => {
        PersistenceError::Database { code: code.clone(), message: message.clone() }
      }
      PersistenceError::Invariant(detail) => PersistenceError::Invariant(detail.clone()),
    })
  }
}

#[async_trait]
impl ModelReader for StubModels {
  async fn list_models(
    &self,
    filter: &ModelFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<ModelSummary, Slug>, PersistenceError> {
    self
      .calls
      .lock()
      .expect("an uncontended lock")
      .push((filter.family.map(FamilyId::get), after.map(|slug| slug.as_str().to_owned())));
    if let Some(error) = self.failure() {
      return Err(error);
    }

    // The adapter overfetches one row past the page, so the fake does too:
    // otherwise `Page::from_overfetched` would never see a continuation and no
    // cursor would ever be issued.
    let start = after.map_or(0, |resume| {
      self
        .rows
        .iter()
        .position(|row| row.slug.as_str() > resume.as_str())
        .unwrap_or(self.rows.len())
    });
    let window: Vec<ModelSummary> =
      self.rows[start..].iter().take(usize::from(limit.query_size())).cloned().collect();

    Ok(Page::from_overfetched(window, limit, |row: &ModelSummary| row.slug.clone()))
  }

  async fn model(&self, slug: &Slug) -> Result<Option<ModelDetail>, PersistenceError> {
    if let Some(error) = self.failure() {
      return Err(error);
    }
    Ok(self.detail.as_ref().filter(|detail| detail.summary.slug.as_str() == slug.as_str()).map(
      |detail| ModelDetail {
        summary: detail.summary.clone(),
        name_aliases: detail.name_aliases.clone(),
        description: detail.description.clone(),
      },
    ))
  }
}

fn slug(value: &str) -> Slug {
  Slug::try_from(value).expect("a slug_text value")
}

fn summary(name: &str) -> ModelSummary {
  ModelSummary {
    slug: slug(name),
    name: name.to_uppercase(),
    display_name: None,
    family: slug("cessna"),
    series: None,
    generation: None,
    first_flight_year: None,
    certification_year: None,
  }
}

fn state(families: Arc<StubFamilies>, models: Arc<StubModels>) -> ApiState {
  ApiState {
    readiness: Arc::new(AlwaysReady),
    catalogs: Arc::new(NoCatalogs),
    families,
    models,
    authentication: Arc::new(AuthenticationService::new(Arc::new(NeverLooksUp))),
    version: "9.9.9-test",
    build_commit: None,
    shutdown: ShutdownState::new(),
    limits: PerimeterLimits::new(1_048_576, Duration::from_secs(30), 256, &[])
      .expect("an empty origin list cannot fail"),
    rate_limits: Arc::new(RateLimiter::new(
      RateLimitPolicy::new(Quota::new(1_000, 1_000).expect("a usable quota"), 64, &[])
        .expect("no tier overrides"),
    )),
  }
}

fn serving(families: Arc<StubFamilies>, models: Arc<StubModels>) -> ApplicationRouter {
  let routes = Routes::new()
    .route(RouteMethod::Get, LIST, RoutePolicy::Public, aircraft_api::routes::models::list_models)
    .route(RouteMethod::Get, DETAIL, RoutePolicy::Public, aircraft_api::routes::models::model);
  router_with_routes(state(families, models), routes)
}

async fn get(router: ApplicationRouter, path: &str) -> TestResult<(StatusCode, Value)> {
  let response = router
    .oneshot(Request::builder().uri(path).body(Body::empty()).expect("a valid test request"))
    .await?;
  let status = response.status();
  let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
  let body = if bytes.is_empty() {
    Value::Null
  } else {
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
  };
  Ok((status, body))
}

/// AC1. The published page is the application type's contract: every member it
/// carries, spelled as the wire spells it, and a null next cursor on a final
/// page rather than an omitted one.
///
/// A populated row and an all-null row together, so a mapper that dropped every
/// optional member and one that invented defaults both fail.
#[tokio::test]
async fn a_page_of_models_publishes_the_contract_the_application_types_describe() -> TestResult {
  let mut populated = summary("c-172");
  populated.display_name = Some("Skyhawk".to_owned());
  populated.series = Some("Skyhawk".to_owned());
  populated.generation = Some(3);
  populated.first_flight_year = Some(1955);
  populated.certification_year = Some(1956);
  let models =
    Arc::new(StubModels { rows: vec![populated, summary("c-182")], ..StubModels::default() });

  let (status, body) = get(serving(Arc::new(StubFamilies::default()), models), LIST).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    body,
    json!({
      "items": [
        {
          "slug": "c-172",
          "name": "C-172",
          "display_name": "Skyhawk",
          "family": "cessna",
          "series": "Skyhawk",
          "generation": 3,
          "first_flight_year": 1955,
          "certification_year": 1956
        },
        { "slug": "c-182", "name": "C-182", "family": "cessna" }
      ],
      "next_cursor": null
    }),
    "an absent optional member is omitted, and a final page's cursor is null"
  );
  Ok(())
}

/// The accepted decision's pair, both halves in one test so neither can drift:
/// a family that exists with no models is a successful empty collection, and
/// the resolved id is what reached the filter.
#[tokio::test]
async fn a_family_that_exists_with_no_models_is_a_successful_empty_collection() -> TestResult {
  let families = Arc::new(StubFamilies::default());
  let models = Arc::new(StubModels::default());

  let (status, body) =
    get(serving(families.clone(), models.clone()), &format!("{LIST}?family=cessna")).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(body, json!({ "items": [], "next_cursor": null }));
  assert_eq!(
    *models.calls.lock().expect("an uncontended lock"),
    vec![(Some(CESSNA), None)],
    "the resolved family id is what the filter carried, not the slug"
  );
  Ok(())
}

/// The other half of that decision: a family slug naming no family is a missing
/// parent, which is a `404` and not an empty page -- and no collection
/// statement runs for it.
#[tokio::test]
async fn a_family_filter_naming_no_family_is_not_found() -> TestResult {
  let families = Arc::new(StubFamilies::default());
  let models = Arc::new(StubModels { rows: vec![summary("c-172")], ..StubModels::default() });

  let (status, body) =
    get(serving(families.clone(), models.clone()), &format!("{LIST}?family=piper")).await?;

  assert_eq!(status, StatusCode::NOT_FOUND);
  assert_eq!(body["status"], json!(404), "an RFC 9457 document, not a bare status");
  assert_eq!(*families.asked.lock().expect("an uncontended lock"), vec!["piper".to_owned()]);
  assert!(
    models.calls.lock().expect("an uncontended lock").is_empty(),
    "a missing parent must not reach the collection statement"
  );
  Ok(())
}

/// AC2. A filter value that is not a slug is refused as an invalid filter, and
/// refused *before* the lookup: the empty recording is the ordering proof, which
/// a status assertion alone would not give.
#[tokio::test]
async fn a_family_filter_that_is_not_a_slug_is_refused_before_any_lookup() -> TestResult {
  let families = Arc::new(StubFamilies::default());
  let models = Arc::new(StubModels::default());

  let (status, _) =
    get(serving(families.clone(), models.clone()), &format!("{LIST}?family=NOT+A+SLUG")).await?;

  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert!(
    families.asked.lock().expect("an uncontended lock").is_empty(),
    "an invalid filter is refused before the family lookup"
  );
  assert!(models.calls.lock().expect("an uncontended lock").is_empty());
  Ok(())
}

/// AC2. The sort allowlist, through the query type: the one legal value is
/// accepted and anything else is a `400`. Both halves, because a handler that
/// refused every sort would pass the negative alone.
#[tokio::test]
async fn only_the_allowlisted_sort_is_accepted() -> TestResult {
  let models = Arc::new(StubModels { rows: vec![summary("c-172")], ..StubModels::default() });
  let router = || serving(Arc::new(StubFamilies::default()), models.clone());

  let (accepted, _) = get(router(), &format!("{LIST}?sort=slug_asc")).await?;
  assert_eq!(accepted, StatusCode::OK, "slug_asc is this endpoint's whole allowlist");

  for rejected in ["name_desc", "slug_desc", "SLUG_ASC", ""] {
    let (status, _) = get(router(), &format!("{LIST}?sort={rejected}")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "sort={rejected:?} is not allowlisted");
  }
  Ok(())
}

/// AC1, and the issue's cursor-continuation case. Two pages of a three-row set
/// at two per page: the first closes with a cursor, feeding it back resumes
/// *after* the first page's last row, and the last page's cursor is null.
///
/// The resumed key is asserted against what the reader received, not only
/// against the rows returned, because a handler that issued a cursor and then
/// ignored it would still return plausible rows from a fake that ignores
/// `after`.
#[tokio::test]
async fn a_cursor_resumes_the_scan_after_the_last_row_and_the_final_page_closes_it() -> TestResult {
  let models = Arc::new(StubModels {
    rows: vec![summary("c-150"), summary("c-172"), summary("c-182")],
    ..StubModels::default()
  });

  let (status, first) =
    get(serving(Arc::new(StubFamilies::default()), models.clone()), &format!("{LIST}?limit=2"))
      .await?;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    first["items"].as_array().expect("an items array").len(),
    2,
    "the limit bounds the page"
  );
  let cursor = first["next_cursor"].as_str().expect("a full page carries a continuation");

  let (status, second) = get(
    serving(Arc::new(StubFamilies::default()), models.clone()),
    &format!("{LIST}?limit=2&cursor={cursor}"),
  )
  .await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(second["items"], json!([{ "slug": "c-182", "name": "C-182", "family": "cessna" }]));
  assert_eq!(second["next_cursor"], Value::Null, "the final page closes the scan");
  assert_eq!(
    models.calls.lock().expect("an uncontended lock")[1],
    (None, Some("c-172".to_owned())),
    "the second read resumes after the first page's last row"
  );
  Ok(())
}

/// AC2. A cursor is bound to the filters and the sort it was issued for, so one
/// carried across a filter change is refused rather than silently resuming a
/// different scan.
#[tokio::test]
async fn a_cursor_carried_across_a_filter_change_is_refused() -> TestResult {
  let models = Arc::new(StubModels {
    rows: vec![summary("c-150"), summary("c-172"), summary("c-182")],
    ..StubModels::default()
  });
  let router = || serving(Arc::new(StubFamilies::default()), models.clone());

  let (_, unfiltered) = get(router(), &format!("{LIST}?limit=2")).await?;
  let cursor = unfiltered["next_cursor"].as_str().expect("a continuation").to_owned();

  // The same token, now presented with a family filter: the fingerprint the
  // endpoint computes no longer matches the one inside the token.
  let (status, _) = get(router(), &format!("{LIST}?limit=2&family=cessna&cursor={cursor}")).await?;
  assert_eq!(status, StatusCode::BAD_REQUEST);

  let (status, _) = get(router(), &format!("{LIST}?limit=2&cursor=not-a-cursor")).await?;
  assert_eq!(status, StatusCode::BAD_REQUEST, "a malformed token is the same refusal");
  Ok(())
}

/// AC2. A cursor names the sort it was issued for, and this endpoint compares
/// that name against its own allowlist before taking the order from the
/// allowlist. Nothing signs a cursor, so a hand-made token carrying another
/// order is reachable, and it must not resume the scan.
///
/// Both halves run, and the second is the anti-vacuity guard: the forged
/// fingerprint literal mirrors the handler's own normalization, so if it were
/// wrong *both* tokens would be refused as filter mismatches and the first
/// assertion would pass for the wrong reason. An accepted token with the right
/// sort proves the literal is right, which leaves the sort as the only thing
/// the refusal can be about.
#[tokio::test]
async fn a_forged_cursor_naming_another_sort_does_not_resume_the_scan() -> TestResult {
  let models = Arc::new(StubModels {
    rows: vec![summary("c-150"), summary("c-172"), summary("c-182")],
    ..StubModels::default()
  });
  let router = || serving(Arc::new(StubFamilies::default()), models.clone());
  let unfiltered = FilterFingerprint::of("family=");
  let forge = |sort: &str| {
    encode(
      &CursorPosition {
        sort: sort.to_owned(),
        last_value: "c-150".to_owned(),
        tiebreaker: "c-150".to_owned(),
      },
      unfiltered,
    )
  };

  let (status, _) = get(router(), &format!("{LIST}?cursor={}", forge("slug_desc"))).await?;
  assert_eq!(status, StatusCode::BAD_REQUEST, "a cursor for another order is refused");
  assert!(
    models.calls.lock().expect("an uncontended lock").is_empty(),
    "and refused before the collection statement"
  );

  let (status, _) = get(router(), &format!("{LIST}?cursor={}", forge("slug_asc"))).await?;
  assert_eq!(
    status,
    StatusCode::OK,
    "the same token naming this endpoint's own order is accepted, so the refusal above was \
     about the sort and not about the filters"
  );
  Ok(())
}

/// AC3. Present and absent together, so a handler answering `404` for
/// everything passes neither half. A path segment that is not a slug is the
/// same `404`: it names nothing, and a `400` would tell a caller probing
/// identifiers which guesses were even shaped right.
#[tokio::test]
async fn one_model_is_served_by_slug_and_anything_else_is_not_found() -> TestResult {
  let detail = ModelDetail {
    summary: summary("c-172"),
    name_aliases: vec!["Skyhawk".to_owned()],
    description: Some("The one everyone learned in".to_owned()),
  };
  let models = Arc::new(StubModels { detail: Some(detail), ..StubModels::default() });
  let router = || serving(Arc::new(StubFamilies::default()), models.clone());

  let (status, body) = get(router(), "/__test/models/c-172").await?;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    body,
    json!({
      "slug": "c-172",
      "name": "C-172",
      "family": "cessna",
      "name_aliases": ["Skyhawk"],
      "description": "The one everyone learned in"
    }),
    "an empty alias list is still published; absent optionals are omitted"
  );

  for missing in ["c-999", "NOT+A+SLUG"] {
    let (status, body) = get(router(), &format!("/__test/models/{missing}")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing} names no model");
    assert_eq!(body["status"], json!(404));
  }
  Ok(())
}

/// A failed read is the dependency's outage, answered as a problem document
/// that carries none of it. Both ports are covered: the filter resolution and
/// the collection read reach the same mapping.
#[tokio::test]
async fn a_failed_read_is_a_problem_document_that_carries_no_diagnostic() -> TestResult {
  let failing_models = Arc::new(StubModels {
    failing: Some(PersistenceError::Database {
      code: "57P01".to_owned(),
      message: "terminating connection due to administrator command".to_owned(),
    }),
    ..StubModels::default()
  });

  let (status, body) =
    get(serving(Arc::new(StubFamilies::default()), failing_models), LIST).await?;
  assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
  let rendered = body.to_string();
  assert!(!rendered.contains("57P01"), "the SQLSTATE must not be served: {rendered}");
  assert!(!rendered.contains("administrator"), "nor the driver's message: {rendered}");

  let failing_families = Arc::new(StubFamilies { failing: true, ..StubFamilies::default() });
  let (status, _) = get(
    serving(failing_families, Arc::new(StubModels::default())),
    &format!("{LIST}?family=cessna"),
  )
  .await?;
  assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "the resolution read maps the same way");

  let invariant = Arc::new(StubModels {
    failing: Some(PersistenceError::Invariant("a model has no family".to_owned())),
    ..StubModels::default()
  });
  let (status, body) = get(serving(Arc::new(StubFamilies::default()), invariant), LIST).await?;
  assert_eq!(
    status,
    StatusCode::INTERNAL_SERVER_ERROR,
    "a broken invariant is not a retryable outage"
  );
  assert!(!body.to_string().contains("no family"), "nor is the invariant served");
  Ok(())
}

/// The shipped paths, under the policy they are really registered with: an
/// anonymous caller is refused before any port is consulted. The fakes here
/// panic if read, so reaching one would fail this test rather than pass it.
#[tokio::test]
async fn an_anonymous_request_to_a_shipped_model_route_is_unauthorized() -> TestResult {
  for path in ["/v1/models", "/v1/models/c-172"] {
    let shipped = router(state(Arc::new(StubFamilies::default()), Arc::new(StubModels::default())));
    let (status, body) = get(shipped, path).await?;

    assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} is a scoped route");
    assert_eq!(body["status"], json!(401), "{path} answers an RFC 9457 document");
  }
  Ok(())
}
