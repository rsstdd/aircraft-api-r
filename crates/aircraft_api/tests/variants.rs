// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used, clippy::panic)]

//! `GET /v1/variants`, driven through the real perimeter over fake ports.
//!
//! The handler is registered under `Public` at a `/__test/...` path, as the
//! family and model suites do and for the reason recorded there. That the route
//! is `CatalogRead` is
//! `every_served_route_is_registered_once_and_the_operational_routes_are_public`'s
//! job, and `an_anonymous_request_to_the_shipped_variant_route_is_unauthorized`
//! closes the loop against the shipped path.

use std::{
  sync::{Arc, Mutex},
  time::Duration,
};

use aircraft_api::{
  ApiState, ApplicationRouter, PerimeterLimits,
  rate_limit::{Quota, RateLimitPolicy, RateLimiter},
  router, router_with_routes,
  routes::{RouteMethod, RoutePolicy, Routes},
  shutdown::ShutdownState,
};
use aircraft_app::{
  authentication::{AuthenticationService, CredentialLookup, CredentialLookupRecord},
  catalog::{
    FamilyDetail, FamilyFilter, FamilyReader, FamilySummary, ModelDetail, ModelFilter, ModelReader,
    ModelSummary, VariantDetail, VariantFilter, VariantReadError, VariantReader, VariantSummary,
  },
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
  readiness::ReadinessProbe,
  reference::{CatalogEntry, CatalogReader},
};
use aircraft_domain::{
  catalog::{CountryCode, FamilyId, LookupCode, ModelId, Slug},
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

const LIST: &str = "/__test/variants";
const MODEL_ID: i64 = 11;
const FAMILY_ID: i64 = 7;

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

/// Resolves exactly one slug per parent, and records what it was asked for.
#[derive(Default)]
struct StubParents {
  asked: Mutex<Vec<String>>,
}

#[async_trait]
impl FamilyReader for StubParents {
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
    self.asked.lock().expect("an uncontended lock").push(format!("family:{}", slug.as_str()));
    Ok((slug.as_str() == "cessna").then(|| FamilyId::new(FAMILY_ID)))
  }
}

#[async_trait]
impl ModelReader for StubParents {
  async fn list_models(
    &self,
    _filter: &ModelFilter,
    _limit: PageLimit,
    _after: Option<&Slug>,
  ) -> Result<Page<ModelSummary, Slug>, PersistenceError> {
    panic!("no route here lists models")
  }

  async fn model(&self, _slug: &Slug) -> Result<Option<ModelDetail>, PersistenceError> {
    panic!("no route here reads a model")
  }

  async fn model_id(&self, slug: &Slug) -> Result<Option<ModelId>, PersistenceError> {
    self.asked.lock().expect("an uncontended lock").push(format!("model:{}", slug.as_str()));
    Ok((slug.as_str() == "c-172").then(|| ModelId::new(MODEL_ID)))
  }
}

/// What the handler asked the collection for, in the port's own vocabulary.
#[derive(Debug, Eq, PartialEq)]
struct RecordedList {
  model: Option<i64>,
  family: Option<i64>,
  variant_type: Option<String>,
  country_of_origin: Option<String>,
  is_in_production: Option<bool>,
  produced_in_year: Option<i16>,
  limit: u16,
  after: Option<String>,
}

#[derive(Default)]
struct StubVariants {
  rows: Vec<VariantSummary>,
  unknown_code: Option<&'static str>,
  failing: Option<PersistenceError>,
  calls: Mutex<Vec<RecordedList>>,
}

#[async_trait]
impl VariantReader for StubVariants {
  async fn list_variants(
    &self,
    filter: &VariantFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<VariantSummary, Slug>, VariantReadError> {
    self.calls.lock().expect("an uncontended lock").push(RecordedList {
      model: filter.model.map(ModelId::get),
      family: filter.family.map(FamilyId::get),
      variant_type: filter.variant_type.as_ref().map(|code| code.as_str().to_owned()),
      country_of_origin: filter.country_of_origin.as_ref().map(|code| code.as_str().to_owned()),
      is_in_production: filter.is_in_production,
      produced_in_year: filter.produced_in_year,
      limit: limit.get(),
      after: after.map(|slug| slug.as_str().to_owned()),
    });
    if let Some(filter_name) = self.unknown_code {
      return Err(VariantReadError::UnknownFilterCode(filter_name));
    }
    if let Some(error) = &self.failing {
      return Err(VariantReadError::Persistence(match error {
        PersistenceError::Database { code, message } => {
          PersistenceError::Database { code: code.clone(), message: message.clone() }
        }
        PersistenceError::Invariant(detail) => PersistenceError::Invariant(detail.clone()),
      }));
    }

    let start = after.map_or(0, |resume| {
      self
        .rows
        .iter()
        .position(|row| row.slug.as_str() > resume.as_str())
        .unwrap_or(self.rows.len())
    });
    let window: Vec<VariantSummary> =
      self.rows[start..].iter().take(usize::from(limit.query_size())).cloned().collect();

    Ok(Page::from_overfetched(window, limit, |row: &VariantSummary| row.slug.clone()))
  }

  async fn variant(&self, _slug: &Slug) -> Result<Option<VariantDetail>, PersistenceError> {
    panic!("the detail route is issue #44")
  }
}

fn slug(value: &str) -> Slug {
  Slug::try_from(value).expect("a slug_text value")
}

fn summary(name: &str) -> VariantSummary {
  VariantSummary {
    slug: slug(name),
    name: name.to_uppercase(),
    popular_name: None,
    model: slug("c-172"),
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

fn state(parents: Arc<StubParents>, variants: Arc<StubVariants>) -> ApiState {
  ApiState {
    readiness: Arc::new(AlwaysReady),
    catalogs: Arc::new(NoCatalogs),
    families: parents.clone(),
    models: parents,
    variants,
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

fn serving(parents: Arc<StubParents>, variants: Arc<StubVariants>) -> ApplicationRouter {
  let routes = Routes::new().route(
    RouteMethod::Get,
    LIST,
    RoutePolicy::Public,
    aircraft_api::routes::variants::list_variants,
  );
  router_with_routes(state(parents, variants), routes)
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

/// AC1 and AC4. The published row is identity and lineage only: no measurement
/// member appears, canonical or pending, which is what the fourth criterion
/// asks for. Asserted as an exact body, so a specification member added to the
/// DTO would fail here rather than ship.
#[tokio::test]
async fn a_summary_carries_identity_and_lineage_and_no_specification_values() -> TestResult {
  let mut row = summary("c-172s");
  row.popular_name = Some("Skyhawk SP".to_owned());
  row.variant_type = Some(LookupCode::try_from("PRODUCTION_STANDARD").expect("a code"));
  row.country_of_origin = Some(CountryCode::try_from("USA").expect("a country"));
  row.production_start_year = Some(1998);
  row.passenger_capacity = Some(3);
  row.engine_count = Some(1);
  row.is_in_production = Some(true);
  let variants = Arc::new(StubVariants { rows: vec![row], ..StubVariants::default() });

  let (status, body) = get(serving(Arc::new(StubParents::default()), variants), LIST).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    body,
    json!({
      "items": [{
        "slug": "c-172s",
        "name": "C-172S",
        "popular_name": "Skyhawk SP",
        "model": "c-172",
        "variant_type": "PRODUCTION_STANDARD",
        "country_of_origin": "USA",
        "production_start_year": 1998,
        "passenger_capacity": 3,
        "engine_count": 1,
        "is_in_production": true
      }],
      "next_cursor": null,
      "limit": 50
    }),
    "every member is a variants column or the parent model's slug, and absent \
     optionals are omitted"
  );

  let members = body["items"][0].as_object().expect("an object");
  for forbidden in ["performance", "weights", "costs", "specifications", "assertions"] {
    assert!(!members.contains_key(forbidden), "{forbidden} must not appear in a summary");
  }
  Ok(())
}

/// AC2. The reported limit is the one applied: the default when none was sent,
/// the requested value when it is within bounds, and the ceiling when it is
/// above. All three, because an implementation echoing the request would pass
/// the middle case alone.
#[tokio::test]
async fn the_reported_limit_is_the_one_applied_and_is_capped_at_two_hundred() -> TestResult {
  let variants =
    Arc::new(StubVariants { rows: vec![summary("c-172s")], ..StubVariants::default() });
  let parents = Arc::new(StubParents::default());

  for (query, expected) in
    [("", 50_u16), ("?limit=10", 10), ("?limit=200", 200), ("?limit=5000", 200)]
  {
    let (status, body) =
      get(serving(parents.clone(), variants.clone()), &format!("{LIST}{query}")).await?;
    assert_eq!(status, StatusCode::OK, "{query:?}");
    assert_eq!(body["limit"], json!(expected), "the reported limit for {query:?}");
  }

  // And the bound the handler reports is the bound the port received, so the
  // number is not decorative.
  let applied: Vec<u16> =
    variants.calls.lock().expect("an uncontended lock").iter().map(|call| call.limit).collect();
  assert_eq!(applied, [50, 10, 200, 200], "the reported limit is what bounded the read");

  let (status, _) = get(serving(parents, variants), &format!("{LIST}?limit=0")).await?;
  assert_eq!(status, StatusCode::BAD_REQUEST, "a limit of zero is refused, not defaulted");
  Ok(())
}

/// AC1. Combined filters reach the port together, resolved where the port takes
/// a key and validated where it takes a code.
///
/// Each filter carries a distinct value and is asserted in its own member, so a
/// binding swapped between two of them fails here.
#[tokio::test]
async fn combined_filters_reach_the_port_resolved_and_validated() -> TestResult {
  let parents = Arc::new(StubParents::default());
  let variants = Arc::new(StubVariants::default());

  let (status, _) = get(
    serving(parents.clone(), variants.clone()),
    &format!(
      "{LIST}?model=c-172&family=cessna&variant_type=PRODUCTION_STANDARD\
       &country_of_origin=USA&is_in_production=true&produced_in_year=1999"
    ),
  )
  .await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    *variants.calls.lock().expect("an uncontended lock"),
    vec![RecordedList {
      model: Some(MODEL_ID),
      family: Some(FAMILY_ID),
      variant_type: Some("PRODUCTION_STANDARD".to_owned()),
      country_of_origin: Some("USA".to_owned()),
      is_in_production: Some(true),
      produced_in_year: Some(1999),
      limit: 50,
      after: None,
    }],
    "every filter arrives in its own member, with both parents resolved to ids"
  );
  assert_eq!(
    *parents.asked.lock().expect("an uncontended lock"),
    vec!["model:c-172".to_owned(), "family:cessna".to_owned()],
    "each parent slug is resolved exactly once"
  );
  Ok(())
}

/// AC3. Every class of invalid input is a `400`, and each is refused before the
/// collection read.
///
/// The year cases are the boundary pair plus one outside it, because an
/// implementation checking the wrong comparison would pass a single case.
#[tokio::test]
async fn an_invalid_code_range_sort_or_cursor_is_refused_before_the_read() -> TestResult {
  let parents = Arc::new(StubParents::default());
  let variants = Arc::new(StubVariants::default());

  let refused = [
    "?variant_type=not+a+code",
    "?service_status=lower_case",
    "?country_of_origin=usa",
    "?produced_in_year=1899",
    "?produced_in_year=2101",
    "?sort=name_desc",
    "?cursor=not-a-cursor",
    "?model=NOT+A+SLUG",
    "?is_in_production=maybe",
    "?produced_in_year=abc",
  ];
  for query in refused {
    let before = variants.calls.lock().expect("an uncontended lock").len();
    let (status, body) =
      get(serving(parents.clone(), variants.clone()), &format!("{LIST}{query}")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{query} must be refused");
    assert_eq!(body["status"], json!(400), "{query} answers an RFC 9457 document");
    assert_eq!(
      variants.calls.lock().expect("an uncontended lock").len(),
      before,
      "{query} must be refused before the collection read"
    );
  }

  // The accepted boundary values, so the range check is not simply refusing
  // every year.
  for query in ["?produced_in_year=1900", "?produced_in_year=2100"] {
    let (status, _) =
      get(serving(parents.clone(), variants.clone()), &format!("{LIST}{query}")).await?;
    assert_eq!(status, StatusCode::OK, "{query} is inside the schema's range");
  }
  Ok(())
}

/// AC3. A reference code the vocabulary does not carry is the adapter's
/// `UnknownFilterCode`, which this route answers `400` rather than passing off
/// as an outage or an empty page.
#[tokio::test]
async fn a_reference_code_no_vocabulary_carries_is_a_bad_request() -> TestResult {
  let variants =
    Arc::new(StubVariants { unknown_code: Some("variant_type"), ..StubVariants::default() });

  let (status, body) = get(
    serving(Arc::new(StubParents::default()), variants),
    &format!("{LIST}?variant_type=NO_SUCH_CODE"),
  )
  .await?;

  assert_eq!(status, StatusCode::BAD_REQUEST);
  assert_eq!(body["status"], json!(400));
  Ok(())
}

/// A parent slug naming nothing is a missing parent, which the accepted
/// decision answers `404` -- and no collection read happens for it.
#[tokio::test]
async fn a_parent_filter_naming_nothing_is_not_found() -> TestResult {
  let parents = Arc::new(StubParents::default());
  let variants = Arc::new(StubVariants::default());

  for query in ["?model=no-such-model", "?family=no-such-family"] {
    let (status, body) =
      get(serving(parents.clone(), variants.clone()), &format!("{LIST}{query}")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "{query} names no parent");
    assert_eq!(body["status"], json!(404));
  }
  assert!(
    variants.calls.lock().expect("an uncontended lock").is_empty(),
    "a missing parent must not reach the collection read"
  );
  Ok(())
}

/// AC1. Multi-page traversal: a page with more behind it closes with a cursor,
/// feeding it back resumes after that page's last row, and the final page's
/// cursor is null.
///
/// Four rows at two per page is two pages, not three: the second page is
/// exactly full and still final, because `Page::from_overfetched` issues a
/// continuation only when the lookahead row came back. A test asserting three
/// pages here would be asserting a row that does not exist.
#[tokio::test]
async fn a_multi_page_traversal_visits_every_row_exactly_once() -> TestResult {
  const MAX_PAGES: usize = 32;

  let variants = Arc::new(StubVariants {
    rows: vec![summary("c-172m"), summary("c-172n"), summary("c-172r"), summary("c-172s")],
    ..StubVariants::default()
  });
  let parents = Arc::new(StubParents::default());

  let mut visited: Vec<String> = Vec::new();
  let mut cursor: Option<String> = None;
  let mut pages = 0;
  for _ in 0..MAX_PAGES {
    let path = cursor
      .as_ref()
      .map_or_else(|| format!("{LIST}?limit=2"), |token| format!("{LIST}?limit=2&cursor={token}"));
    let (status, body) = get(serving(parents.clone(), variants.clone()), &path).await?;
    assert_eq!(status, StatusCode::OK);
    pages += 1;
    for row in body["items"].as_array().expect("an items array") {
      visited.push(row["slug"].as_str().expect("a slug").to_owned());
    }
    match body["next_cursor"].as_str() {
      Some(token) => cursor = Some(token.to_owned()),
      None => break,
    }
  }

  assert_eq!(visited, ["c-172m", "c-172n", "c-172r", "c-172s"], "every row exactly once, in order");
  assert_eq!(pages, 2, "an exactly-full final page closes the scan rather than issuing a cursor");
  assert_eq!(
    variants.calls.lock().expect("an uncontended lock")[1].after,
    Some("c-172n".to_owned()),
    "the second read resumes after the first page's last row"
  );
  Ok(())
}

/// An empty collection is a successful empty page carrying the applied limit.
#[tokio::test]
async fn an_empty_collection_is_a_successful_empty_page() -> TestResult {
  let (status, body) =
    get(serving(Arc::new(StubParents::default()), Arc::new(StubVariants::default())), LIST).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(body, json!({ "items": [], "next_cursor": null, "limit": 50 }));
  Ok(())
}

/// A failed read is the dependency's outage, answered as a problem document
/// carrying none of it; a broken invariant is not a retryable outage.
#[tokio::test]
async fn a_failed_read_is_a_problem_document_that_carries_no_diagnostic() -> TestResult {
  let outage = Arc::new(StubVariants {
    failing: Some(PersistenceError::Database {
      code: "57P01".to_owned(),
      message: "terminating connection due to administrator command".to_owned(),
    }),
    ..StubVariants::default()
  });
  let (status, body) = get(serving(Arc::new(StubParents::default()), outage), LIST).await?;
  assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
  let rendered = body.to_string();
  assert!(!rendered.contains("57P01"), "the SQLSTATE must not be served: {rendered}");
  assert!(!rendered.contains("administrator"), "nor the driver's message: {rendered}");

  let broken = Arc::new(StubVariants {
    failing: Some(PersistenceError::Invariant("a stored lookup code is not one".to_owned())),
    ..StubVariants::default()
  });
  let (status, body) = get(serving(Arc::new(StubParents::default()), broken), LIST).await?;
  assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
  assert!(!body.to_string().contains("lookup code"), "nor is the invariant served");
  Ok(())
}

/// The shipped path, under the policy it is really registered with.
#[tokio::test]
async fn an_anonymous_request_to_the_shipped_variant_route_is_unauthorized() -> TestResult {
  let shipped = router(state(Arc::new(StubParents::default()), Arc::new(StubVariants::default())));
  let (status, body) = get(shipped, "/v1/variants").await?;

  assert_eq!(status, StatusCode::UNAUTHORIZED);
  assert_eq!(body["status"], json!(401));
  Ok(())
}
