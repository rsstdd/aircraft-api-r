// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use aircraft_api::{
  ApiState, PerimeterLimits,
  problem::ApiProblem,
  router, router_with_routes,
  routes::{RouteMethod, RoutePolicy, Routes},
  shutdown::ShutdownState,
};
use aircraft_app::{
  credential_issuance::{CredentialStore, CredentialVerifier, NewCredential},
  ingestion::PersistenceError,
  readiness::ReadinessProbe,
};
use aircraft_db::{SqlxCredentialStore, readiness::PoolReadiness};
use aircraft_testsupport::{TestResult, install_schema, start_postgres};
use async_trait::async_trait;
use axum::{
  body::{Body, to_bytes},
  http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx_core::{query::query, query_scalar::query_scalar};
use tower::ServiceExt as _;
use uuid::Uuid;

const REQUEST_ID: &str = "real-database-problem-test";

struct AlwaysReady;

#[async_trait]
impl ReadinessProbe for AlwaysReady {
  async fn check(&self) -> Result<(), PersistenceError> {
    Ok(())
  }
}

fn state(readiness: Arc<dyn ReadinessProbe>) -> ApiState {
  ApiState {
    readiness,
    version: "9.9.9-test",
    build_commit: None,
    shutdown: ShutdownState::new(),
    limits: PerimeterLimits::new(1_048_576, Duration::from_secs(30), 256, &[])
      .expect("an empty origin list cannot fail"),
  }
}

async fn problem_body(
  response: axum::response::Response,
  status: StatusCode,
) -> TestResult<String> {
  assert_eq!(response.status(), status);
  assert_eq!(response.headers().get("x-request-id"), Some(&REQUEST_ID.parse()?));
  assert_eq!(
    response.headers().get(header::CONTENT_TYPE),
    Some(&"application/problem+json".parse()?)
  );
  Ok(String::from_utf8(to_bytes(response.into_body(), 4096).await?.to_vec())?)
}

fn assert_absent(text: &str, needle: &str, description: &str) {
  assert!(!text.contains(needle), "the response disclosed {description}");
}

#[tokio::test]
async fn a_stopped_database_returns_a_correlated_redacted_dependency_problem() -> TestResult {
  let (container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  let database_url = container.database_url.clone();
  let readiness = Arc::new(PoolReadiness::new(pool));
  drop(container);

  let internal = readiness.check().await.expect_err("the stopped database must reject a query");
  assert!(internal.code().starts_with("DATABASE_"), "the failure must retain its database class");

  let response = router(state(readiness))
    .oneshot(Request::get("/ready").header("x-request-id", REQUEST_ID).body(Body::empty())?)
    .await?;
  let body = problem_body(response, StatusCode::SERVICE_UNAVAILABLE).await?;
  let document: Value = serde_json::from_str(&body)?;
  assert_eq!(document.pointer("/type"), Some(&json!("/problems/database-unavailable")));
  assert_eq!(document.pointer("/instance"), Some(&json!("/ready")));

  for (needle, description) in [
    (database_url.as_str(), "the database URL"),
    ("postgres:postgres", "database credentials"),
    ("127.0.0.1", "the database host"),
    ("SELECT 1", "SQL text"),
    ("/srv/private", "a host path"),
  ] {
    assert_absent(&body, needle, description);
  }
  Ok(())
}

#[tokio::test]
async fn a_real_unique_violation_maps_to_a_correlated_redacted_conflict_problem() -> TestResult {
  const DIGEST: [u8; 32] = [0xde; 32];
  const PATH: &str = "/__test/credential";

  let (_container, pool) = start_postgres(2, Duration::from_secs(2)).await?;
  install_schema(&pool).await?;
  query(
    "INSERT INTO aircraft_auth.rate_limit_tiers (code, label)
       VALUES ('TEST_TIER', 'Test tier')",
  )
  .execute(&pool)
  .await?;
  let principal_id: i64 = query_scalar(
    "INSERT INTO aircraft_auth.principals (name, rate_limit_tier_code)
       VALUES ('problem-test', 'TEST_TIER') RETURNING id",
  )
  .fetch_one(&pool)
  .await?;
  let verifier = CredentialVerifier::from_digest(DIGEST);
  query(
    "INSERT INTO aircraft_auth.api_credentials (key_id, principal_id, secret_digest, label)
       VALUES ($1, $2, $3, 'existing')",
  )
  .bind(Uuid::new_v4())
  .bind(principal_id)
  .bind(verifier.hex())
  .execute(&pool)
  .await?;

  let error = SqlxCredentialStore::from_pool(pool)
    .persist(NewCredential {
      key_id: Uuid::new_v4(),
      principal_id,
      verifier: verifier.clone(),
      label: "duplicate".to_owned(),
    })
    .await
    .expect_err("the duplicate verifier must violate the unique constraint");
  let internal = error.to_string();
  assert_eq!(error.code(), "DATABASE_23505");
  assert!(
    internal.contains("uq_apc_secret_digest"),
    "the server-side diagnostic must prove which constraint PostgreSQL rejected"
  );

  let source = Arc::new(error);
  let routes = Routes::new().route(RouteMethod::Post, PATH, RoutePolicy::Public, move || {
    let source = Arc::clone(&source);
    async move {
      match source.as_ref() {
        PersistenceError::Database { code, .. } if code == "DATABASE_23505" => {
          ApiProblem::conflict(PATH)
        }
        _ => ApiProblem::internal(PATH),
      }
    }
  });
  let response = router_with_routes(state(Arc::new(AlwaysReady)), routes)
    .oneshot(Request::post(PATH).header("x-request-id", REQUEST_ID).body(Body::empty())?)
    .await?;
  let body = problem_body(response, StatusCode::CONFLICT).await?;
  let document: Value = serde_json::from_str(&body)?;
  assert_eq!(document.pointer("/type"), Some(&json!("/problems/conflict")));
  assert_eq!(document.pointer("/instance"), Some(&json!(PATH)));

  for (needle, description) in [
    ("DATABASE_23505", "SQLSTATE"),
    ("uq_apc_secret_digest", "a constraint name"),
    (verifier.hex().as_str(), "a credential verifier"),
    ("INSERT INTO", "SQL text"),
    ("postgres://", "a database URL"),
    ("postgres:postgres", "database credentials"),
    ("127.0.0.1", "the database host"),
    ("/srv/private", "a host path"),
  ] {
    assert_absent(&body, needle, description);
  }
  Ok(())
}
