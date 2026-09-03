// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Every credential state that only `PostgreSQL` can establish, driven through
//! the real middleware and the real lookup against the canonical install in a
//! disposable container.
//!
//! The fake-port tests beside the middleware prove the transport: byte-equal
//! rejections, the removed header, the 503 path. What a fake cannot prove is
//! that `revoked_at`, `disabled_at`, an unissued key, an altered secret, and a
//! principal's actual grants and tier produce those outcomes, so each is set
//! up here with its own credential, and every case asserts exactly one
//! statement through a counting decorator around the real adapter.

use std::{
  sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
  },
  time::Duration,
};

use aircraft_api::{
  ApiState, PerimeterLimits, authentication::require_authentication, router_with_routes,
  shutdown::ShutdownState,
};
use aircraft_app::{
  authentication::{
    AuthenticatedPrincipal, AuthenticationService, CredentialLookup, CredentialLookupRecord,
  },
  credential_issuance::{CredentialIssuanceService, IssueCredential, IssuedCredential},
  ingestion::PersistenceError,
  readiness::ReadinessProbe,
};
use aircraft_db::{SqlxCredentialLookup, SqlxCredentialStore};
use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use async_trait::async_trait;
use axum::{
  Extension, Json, Router,
  body::{Body, to_bytes},
  http::{HeaderValue, Request, StatusCode, header},
  routing::get,
};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use sqlx_core::{query::query, query_scalar::query_scalar};
use sqlx_postgres::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

const PROTECTED: &str = "/__test/protected";
const REQUEST_ID: &str = "real-database-authentication-test";
const TIER_CODE: &str = "TEST_TIER";

struct AlwaysReady;

#[async_trait]
impl ReadinessProbe for AlwaysReady {
  async fn check(&self) -> Result<(), PersistenceError> {
    Ok(())
  }
}

/// Delegates to the real adapter and counts what passed through.
struct Counting {
  inner: SqlxCredentialLookup,
  calls: AtomicUsize,
}

#[async_trait]
impl CredentialLookup for Counting {
  async fn resolve(
    &self,
    key_id: Uuid,
  ) -> Result<Option<CredentialLookupRecord>, PersistenceError> {
    self.calls.fetch_add(1, Ordering::SeqCst);
    self.inner.resolve(key_id).await
  }
}

fn state() -> ApiState {
  ApiState {
    readiness: Arc::new(AlwaysReady),
    version: "9.9.9-test",
    build_commit: None,
    shutdown: ShutdownState::new(),
    limits: PerimeterLimits::new(1_048_576, Duration::from_secs(30), 256, &[])
      .expect("an empty origin list cannot fail"),
  }
}

fn protected_router(pool: &PgPool) -> (Router, Arc<Counting>) {
  let lookup = Arc::new(Counting {
    inner: SqlxCredentialLookup::from_pool(pool.clone()),
    calls: AtomicUsize::new(0),
  });
  let routes = Router::new()
    .route(
      PROTECTED,
      get(|Extension(principal): Extension<AuthenticatedPrincipal>| async move {
        Json(json!({
          "principal_id": principal.principal_id(),
          "scopes": principal.scopes().iter().map(|scope| scope.code()).collect::<Vec<_>>(),
          "tier": principal.tier(),
        }))
      }),
    )
    .route_layer(axum::middleware::from_fn_with_state(
      Arc::new(AuthenticationService::new(lookup.clone())),
      require_authentication,
    ));
  (router_with_routes(state(), routes), lookup)
}

async fn principal(pool: &PgPool, name: &str) -> TestResult<i64> {
  query(
    "INSERT INTO aircraft_auth.rate_limit_tiers (code, label)
         VALUES ($1, 'Test tier') ON CONFLICT (code) DO NOTHING",
  )
  .bind(TIER_CODE)
  .execute(pool)
  .await?;
  let id = query_scalar(
    "INSERT INTO aircraft_auth.principals (name, rate_limit_tier_code)
         VALUES ($1, $2) RETURNING id",
  )
  .bind(name)
  .bind(TIER_CODE)
  .fetch_one(pool)
  .await?;
  Ok(id)
}

async fn grant(pool: &PgPool, principal_id: i64, scope: &str) -> TestResult {
  query(
    "INSERT INTO aircraft_auth.principal_scope_grants (principal_id, scope_code) VALUES ($1, $2)",
  )
  .bind(principal_id)
  .bind(scope)
  .execute(pool)
  .await?;
  Ok(())
}

async fn issue(pool: &PgPool, principal_id: i64) -> TestResult<IssuedCredential> {
  let service =
    CredentialIssuanceService::new(Arc::new(SqlxCredentialStore::from_pool(pool.clone())));
  Ok(service.issue(IssueCredential::new(principal_id, "ci-runner".to_owned())?).await?)
}

fn bearer(token: &str) -> TestResult<Request<Body>> {
  Ok(
    Request::get(PROTECTED)
      .header("x-request-id", REQUEST_ID)
      .header(header::AUTHORIZATION, format!("Bearer {token}"))
      .body(Body::empty())?,
  )
}

/// A response reduced to what a caller can compare: status, the headers that
/// distinguish one refusal from another, and the body bytes.
type Signature = (StatusCode, Vec<Option<HeaderValue>>, Vec<u8>);

async fn signature(response: axum::response::Response) -> TestResult<Signature> {
  let status = response.status();
  let headers = [header::CONTENT_TYPE, header::WWW_AUTHENTICATE, "x-request-id".parse()?]
    .iter()
    .map(|name| response.headers().get(name).cloned())
    .collect();
  let body = to_bytes(response.into_body(), 4096).await?.to_vec();
  Ok((status, headers, body))
}

/// Fails with a fixed sentence: rendering either argument would print the
/// very material the assertion exists to keep out of output.
fn assert_absent(haystack: &[u8], needle: &str, what: &str) {
  assert!(
    !haystack.windows(needle.len()).any(|window| window == needle.as_bytes()),
    "{what} was disclosed"
  );
}

/// Acceptance criteria 1 and 3 over real rows: the principal's own grants,
/// in code order, and its tier, after one statement.
#[tokio::test]
async fn a_real_issued_credential_resolves_its_principal_grants_and_tier_in_one_lookup()
-> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  let owner = principal(&pool, "granted").await?;
  grant(&pool, owner, "CURATION_WRITE").await?;
  grant(&pool, owner, "CATALOG_READ").await?;
  let bystander = principal(&pool, "bystander").await?;
  grant(&pool, bystander, "ADMIN").await?;
  let issued = issue(&pool, owner).await?;
  let (router, lookup) = protected_router(&pool);

  let response = router.oneshot(bearer(issued.clear.expose_secret())?).await?;

  assert_eq!(response.status(), StatusCode::OK);
  let document: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
  assert_eq!(document.pointer("/principal_id"), Some(&json!(owner)));
  assert_eq!(
    document.pointer("/scopes"),
    Some(&json!(["CATALOG_READ", "CURATION_WRITE"])),
    "only the principal's own grants, sorted"
  );
  assert_eq!(document.pointer("/tier"), Some(&json!(TIER_CODE)));
  assert_eq!(lookup.calls.load(Ordering::SeqCst), 1, "exactly one statement");
  Ok(())
}

/// Acceptance criterion 2 over real rows. Each rejected state has its own
/// credential and principal, so no case depends on another's mutation, and
/// every rejection's signature is compared to the first one's under the same
/// request ID. A zero-grant principal is the accepted row of the same table,
/// because the shape that distinguishes it -- an empty list rather than a
/// missing row -- is a database fact too.
#[tokio::test]
async fn every_real_rejected_state_answers_the_same_401_after_one_lookup() -> TestResult {
  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;

  let ungranted = principal(&pool, "ungranted").await?;
  let live = issue(&pool, ungranted).await?;

  let altered = issue(&pool, principal(&pool, "altered").await?).await?;
  let mut wrong_secret = altered.clear.expose_secret().to_owned();
  wrong_secret.pop();
  wrong_secret.push(if altered.clear.expose_secret().ends_with('0') { '1' } else { '0' });

  let revoked = issue(&pool, principal(&pool, "revoked").await?).await?;
  query("UPDATE aircraft_auth.api_credentials SET revoked_at = now() WHERE key_id = $1")
    .bind(revoked.record.key_id)
    .execute(&pool)
    .await?;

  let disabled_owner = principal(&pool, "disabled").await?;
  let disabled = issue(&pool, disabled_owner).await?;
  query("UPDATE aircraft_auth.principals SET disabled_at = now() WHERE id = $1")
    .bind(disabled_owner)
    .execute(&pool)
    .await?;

  let unknown = live.clear.expose_secret().replacen(
    &live.record.key_id.to_string(),
    &Uuid::new_v4().to_string(),
    1,
  );

  let cases: [(&str, &str, bool); 5] = [
    ("live credential, no grants", live.clear.expose_secret(), true),
    ("unknown key", &unknown, false),
    ("altered secret", &wrong_secret, false),
    ("revoked credential", revoked.clear.expose_secret(), false),
    ("disabled principal", disabled.clear.expose_secret(), false),
  ];
  let issued = [&live, &altered, &revoked, &disabled];

  let mut baseline: Option<Signature> = None;
  for (case, token, accepted) in cases {
    let (router, lookup) = protected_router(&pool);

    let response = router.oneshot(bearer(token)?).await?;

    assert_eq!(lookup.calls.load(Ordering::SeqCst), 1, "{case}: exactly one statement");
    let (status, headers, body) = signature(response).await?;
    for credential in issued {
      assert_absent(&body, credential.clear.expose_secret(), "a clear token");
      let stored: String =
        query_scalar("SELECT secret_digest FROM aircraft_auth.api_credentials WHERE key_id = $1")
          .bind(credential.record.key_id)
          .fetch_one(&pool)
          .await?;
      assert_absent(&body, &stored, "a stored digest");
    }
    if accepted {
      assert_eq!(status, StatusCode::OK, "{case}");
      let document: Value = serde_json::from_slice(&body)?;
      assert_eq!(document.pointer("/principal_id"), Some(&json!(ungranted)), "{case}");
      assert_eq!(document.pointer("/scopes"), Some(&json!([])), "{case}: no grants is empty");
      continue;
    }
    match &baseline {
      None => {
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{case}");
        assert_eq!(headers[1].as_ref().map(HeaderValue::as_bytes), Some(&b"Bearer"[..]), "{case}");
        assert_eq!(headers[2].as_ref().map(HeaderValue::as_bytes), Some(REQUEST_ID.as_bytes()));
        let document: Value = serde_json::from_slice(&body)?;
        assert_eq!(document.pointer("/type"), Some(&json!("/problems/authentication-required")));
        assert_eq!(document.pointer("/instance"), Some(&json!(PROTECTED)));
        baseline = Some((status, headers, body));
      }
      Some(first) => assert_eq!(&(status, headers, body), first, "{case}: signature differs"),
    }
  }
  Ok(())
}
