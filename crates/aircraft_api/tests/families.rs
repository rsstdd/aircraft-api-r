// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used, clippy::panic)]

//! `GET /v1/families` and `GET /v1/families/{family}`, driven through the real
//! perimeter over fake ports.
//!
//! The handlers are registered under `Public` at `/__test/...` paths, which is
//! what `crates/aircraft_api/tests/models.rs` and the in-crate catalog tests do
//! and for the reason recorded there: it drives the handler's own behaviour
//! without a credential. That the two family routes are `CatalogRead` is
//! `every_served_route_is_registered_once_and_the_operational_routes_are_public`'s
//! job, and `an_anonymous_request_to_a_shipped_family_route_is_unauthorized`
//! below closes the loop against the shipped paths.
//!
//! #39's AC2 is the rule this file pins: a malformed identifier is a `400` and
//! an unknown one is a `404`. `docs/architecture/http_v1_decisions.md`
//! § "Problem responses" carries both as distinct failure classes, so the two
//! are deliberately not collapsed.

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
  catalog::{FamilyDetail, FamilyFilter, FamilyReader, FamilySummary},
  ingestion::PersistenceError,
  pagination::{Page, PageLimit},
  readiness::ReadinessProbe,
  reference::{CatalogEntry, CatalogReader},
};
use aircraft_domain::{
  catalog::{CountryCode, FamilyId, Slug},
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

mod common;

use common::UnreadCatalog;

const LIST: &str = "/__test/families";
const DETAIL: &str = "/__test/families/{family}";

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

/// One call the handler made, as the port saw it.
///
/// Named members rather than a tuple: each is an `Option<String>` and a tuple
/// would let an assertion compare the wrong one against the wrong position,
/// which is exactly the swapped-binding defect these tests exist to catch.
#[derive(Debug, Eq, PartialEq)]
struct RecordedList {
  manufacturer: Option<String>,
  country_of_origin: Option<String>,
  after: Option<String>,
}

/// Serves a fixed set of rows and records what the handler asked for, so a
/// filter or cursor the handler dropped is visible rather than merely plausible.
#[derive(Default)]
struct StubFamilies {
  rows: Vec<FamilySummary>,
  detail: Option<FamilyDetail>,
  failing: Option<PersistenceError>,
  calls: Mutex<Vec<RecordedList>>,
}

impl StubFamilies {
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
impl FamilyReader for StubFamilies {
  async fn list_families(
    &self,
    filter: &FamilyFilter,
    limit: PageLimit,
    after: Option<&Slug>,
  ) -> Result<Page<FamilySummary, Slug>, PersistenceError> {
    self.calls.lock().expect("an uncontended lock").push(RecordedList {
      manufacturer: filter.manufacturer.as_ref().map(|slug| slug.as_str().to_owned()),
      country_of_origin: filter.country_of_origin.as_ref().map(|code| code.as_str().to_owned()),
      after: after.map(|slug| slug.as_str().to_owned()),
    });
    if let Some(error) = self.failure() {
      return Err(error);
    }

    // Overfetch one past the page, as the adapter does, or no continuation
    // could ever be issued.
    let start = after.map_or(0, |resume| {
      self
        .rows
        .iter()
        .position(|row| row.slug.as_str() > resume.as_str())
        .unwrap_or(self.rows.len())
    });
    let window: Vec<FamilySummary> =
      self.rows[start..].iter().take(usize::from(limit.query_size())).cloned().collect();

    Ok(Page::from_overfetched(window, limit, |row: &FamilySummary| row.slug.clone()))
  }

  async fn family(&self, slug: &Slug) -> Result<Option<FamilyDetail>, PersistenceError> {
    if let Some(error) = self.failure() {
      return Err(error);
    }
    Ok(self.detail.as_ref().filter(|found| found.summary.slug.as_str() == slug.as_str()).cloned())
  }

  async fn family_id(&self, _slug: &Slug) -> Result<Option<FamilyId>, PersistenceError> {
    panic!("no family route resolves a family id")
  }
}

fn slug(value: &str) -> Slug {
  Slug::try_from(value).expect("a slug_text value")
}

fn summary(name: &str) -> FamilySummary {
  FamilySummary {
    slug: slug(name),
    name: name.to_uppercase(),
    common_name: None,
    manufacturer: None,
    country_of_origin: None,
    first_flight_year: None,
  }
}

fn state(families: Arc<StubFamilies>) -> ApiState {
  ApiState {
    readiness: Arc::new(AlwaysReady),
    catalogs: Arc::new(NoCatalogs),
    families,
    models: Arc::new(UnreadCatalog),
    variants: Arc::new(UnreadCatalog),
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

fn serving(families: Arc<StubFamilies>) -> ApplicationRouter {
  let routes = Routes::new()
    .route(
      RouteMethod::Get,
      LIST,
      RoutePolicy::Public,
      aircraft_api::routes::families::list_families,
    )
    .route(RouteMethod::Get, DETAIL, RoutePolicy::Public, aircraft_api::routes::families::family);
  router_with_routes(state(families), routes)
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

/// AC2, both halves in one test so the pair cannot drift. A segment that cannot
/// be a slug is malformed input and a `400`; a well-formed slug naming no
/// family is a `404`. Collapsing them -- which the model detail route did until
/// this issue -- loses the distinction the problem table publishes.
#[tokio::test]
async fn a_malformed_family_identifier_is_refused_and_an_unknown_one_is_not_found() -> TestResult {
  let families = Arc::new(StubFamilies::default());

  let (malformed, body) = get(serving(families.clone()), "/__test/families/NOT+A+SLUG").await?;
  assert_eq!(malformed, StatusCode::BAD_REQUEST, "a segment that cannot be a slug is malformed");
  assert_eq!(body["status"], json!(400), "an RFC 9457 document, not a bare status");

  let (unknown, body) = get(serving(families), "/__test/families/no-such-family").await?;
  assert_eq!(unknown, StatusCode::NOT_FOUND, "a well-formed slug naming nothing is absent");
  assert_eq!(body["status"], json!(404));
  Ok(())
}

/// AC1. The published page is the application type's contract: every member it
/// carries, spelled as the wire spells it, with absent optionals omitted and a
/// null next cursor on a final page. A populated row and an all-null row
/// together, so a mapper that dropped every optional and one that invented
/// defaults both fail.
#[tokio::test]
async fn a_page_of_families_publishes_the_contract_the_application_types_describe() -> TestResult {
  let mut populated = summary("cessna");
  populated.common_name = Some("Cessna".to_owned());
  populated.manufacturer = Some(slug("textron-aviation"));
  populated.country_of_origin = Some(CountryCode::try_from("USA").expect("a country code"));
  populated.first_flight_year = Some(1927);
  let families =
    Arc::new(StubFamilies { rows: vec![populated, summary("piper")], ..StubFamilies::default() });

  let (status, body) = get(serving(families), LIST).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    body,
    json!({
      "items": [
        {
          "slug": "cessna",
          "name": "CESSNA",
          "common_name": "Cessna",
          "manufacturer": "textron-aviation",
          "country_of_origin": "USA",
          "first_flight_year": 1927
        },
        { "slug": "piper", "name": "PIPER" }
      ],
      "next_cursor": null,
      "limit": 50
    }),
    "an absent optional member is omitted, a final page's cursor is null, and the \
     effective limit is reported"
  );
  Ok(())
}

/// AC1. An empty table is a successful empty collection, not a `404`.
#[tokio::test]
async fn an_empty_catalogue_is_a_successful_empty_collection() -> TestResult {
  let (status, body) = get(serving(Arc::new(StubFamilies::default())), LIST).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(body, json!({ "items": [], "next_cursor": null, "limit": 50 }));
  Ok(())
}

/// AC1, AC2. Both filters reach the port as the validated domain values, and a
/// value the domain refuses is a `400` before any read.
///
/// Each filter is asserted against a different column and the recording is
/// checked, because two wrong implementations would otherwise pass: binding the
/// two the other way round, and dropping one of them.
#[tokio::test]
async fn each_filter_reaches_the_port_validated_and_a_refused_value_never_does() -> TestResult {
  let families = Arc::new(StubFamilies::default());

  let (status, _) = get(
    serving(families.clone()),
    &format!("{LIST}?manufacturer=textron-aviation&country_of_origin=USA"),
  )
  .await?;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    *families.calls.lock().expect("an uncontended lock"),
    vec![RecordedList {
      manufacturer: Some("textron-aviation".to_owned()),
      country_of_origin: Some("USA".to_owned()),
      after: None,
    }],
    "each filter arrives in its own member, so a swapped binding fails here"
  );

  // Lower case is a legal VARCHAR(3) but not a legal CountryCode, which is the
  // boundary the domain type owns.
  for refused in ["country_of_origin=usa", "country_of_origin=UNITED", "manufacturer=NOT+A+SLUG"] {
    let before = families.calls.lock().expect("an uncontended lock").len();
    let (status, _) = get(serving(families.clone()), &format!("{LIST}?{refused}")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused} is not a valid identifier");
    assert_eq!(
      families.calls.lock().expect("an uncontended lock").len(),
      before,
      "{refused} must be refused before the collection statement"
    );
  }
  Ok(())
}

/// AC3. Pagination follows the accepted decision: a full page closes with a
/// cursor, feeding it back resumes after that page's last row, and the final
/// page's cursor is null.
///
/// The resumed key is asserted against what the port received, because a
/// handler that issued a cursor and then ignored it would still return
/// plausible rows from a fake that ignores `after`.
#[tokio::test]
async fn a_cursor_resumes_the_scan_after_the_last_row_and_the_final_page_closes_it() -> TestResult {
  let families = Arc::new(StubFamilies {
    rows: vec![summary("airbus"), summary("boeing"), summary("cessna")],
    ..StubFamilies::default()
  });

  let (status, first) = get(serving(families.clone()), &format!("{LIST}?limit=2")).await?;
  assert_eq!(status, StatusCode::OK);
  assert_eq!(first["items"].as_array().expect("an items array").len(), 2);
  let cursor = first["next_cursor"].as_str().expect("a full page carries a continuation");

  let (status, second) =
    get(serving(families.clone()), &format!("{LIST}?limit=2&cursor={cursor}")).await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(second["items"], json!([{ "slug": "cessna", "name": "CESSNA" }]));
  assert_eq!(second["next_cursor"], Value::Null, "the final page closes the scan");
  assert_eq!(
    families.calls.lock().expect("an uncontended lock")[1],
    RecordedList { manufacturer: None, country_of_origin: None, after: Some("boeing".to_owned()) },
    "the second read resumes after the first page's last row"
  );
  Ok(())
}

/// AC2, AC3. A cursor is bound to the filters and the sort it was issued for,
/// and the sort allowlist is closed.
///
/// The accepted sort is asserted alongside the refusals: a handler that refused
/// every sort would pass the negatives alone.
#[tokio::test]
async fn a_cursor_or_sort_outside_this_endpoints_allowlist_is_refused() -> TestResult {
  let families = Arc::new(StubFamilies {
    rows: vec![summary("airbus"), summary("boeing"), summary("cessna")],
    ..StubFamilies::default()
  });
  let router = || serving(families.clone());

  let (status, _) = get(router(), &format!("{LIST}?sort=slug_asc")).await?;
  assert_eq!(status, StatusCode::OK, "slug_asc is this endpoint's whole allowlist");
  for rejected in ["name_desc", "slug_desc", "SLUG_ASC", ""] {
    let (status, _) = get(router(), &format!("{LIST}?sort={rejected}")).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST, "sort={rejected:?} is not allowlisted");
  }

  let (_, unfiltered) = get(router(), &format!("{LIST}?limit=2")).await?;
  let cursor = unfiltered["next_cursor"].as_str().expect("a continuation").to_owned();
  let (status, _) =
    get(router(), &format!("{LIST}?limit=2&manufacturer=airbus-sas&cursor={cursor}")).await?;
  assert_eq!(status, StatusCode::BAD_REQUEST, "a cursor carried across a filter change is refused");

  let (status, _) = get(router(), &format!("{LIST}?cursor=not-a-cursor")).await?;
  assert_eq!(status, StatusCode::BAD_REQUEST, "a malformed token is the same refusal");
  Ok(())
}

/// AC1. The detail contract, and that a found family publishes its aliases even
/// when empty.
#[tokio::test]
async fn one_family_is_served_by_slug_with_its_aliases_and_description() -> TestResult {
  let detail = FamilyDetail {
    summary: summary("cessna"),
    name_aliases: vec!["Cessna Aircraft".to_owned()],
    description: Some("A light-aircraft family".to_owned()),
  };
  let families = Arc::new(StubFamilies { detail: Some(detail), ..StubFamilies::default() });

  let (status, body) = get(serving(families), "/__test/families/cessna").await?;

  assert_eq!(status, StatusCode::OK);
  assert_eq!(
    body,
    json!({
      "slug": "cessna",
      "name": "CESSNA",
      "name_aliases": ["Cessna Aircraft"],
      "description": "A light-aircraft family"
    })
  );
  Ok(())
}

/// A failed read is the dependency's outage, answered as a problem document
/// carrying none of it. Both failure classes map to their own status, and
/// neither serves the driver's text.
#[tokio::test]
async fn a_failed_read_is_a_problem_document_that_carries_no_diagnostic() -> TestResult {
  let outage = Arc::new(StubFamilies {
    failing: Some(PersistenceError::Database {
      code: "57P01".to_owned(),
      message: "terminating connection due to administrator command".to_owned(),
    }),
    ..StubFamilies::default()
  });
  let (status, body) = get(serving(outage), LIST).await?;
  assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
  let rendered = body.to_string();
  assert!(!rendered.contains("57P01"), "the SQLSTATE must not be served: {rendered}");
  assert!(!rendered.contains("administrator"), "nor the driver's message: {rendered}");

  let broken = Arc::new(StubFamilies {
    failing: Some(PersistenceError::Invariant("a stored country code is not one".to_owned())),
    ..StubFamilies::default()
  });
  let (status, body) = get(serving(broken), "/__test/families/cessna").await?;
  assert_eq!(
    status,
    StatusCode::INTERNAL_SERVER_ERROR,
    "a broken invariant is not a retryable outage"
  );
  assert!(!body.to_string().contains("country code"), "nor is the invariant served");
  Ok(())
}

/// The shipped paths, under the policy they are really registered with: an
/// anonymous caller is refused before any port is consulted. The fake panics if
/// read, so reaching one would fail this test rather than pass it.
#[tokio::test]
async fn an_anonymous_request_to_a_shipped_family_route_is_unauthorized() -> TestResult {
  for path in ["/v1/families", "/v1/families/cessna"] {
    let shipped = router(state(Arc::new(StubFamilies::default())));
    let (status, body) = get(shipped, path).await?;

    assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} is a scoped route");
    assert_eq!(body["status"], json!(401), "{path} answers an RFC 9457 document");
  }
  Ok(())
}
