//! Bearer authentication at the HTTP boundary.
//!
//! [`require_authentication`] turns the `Authorization` header into an
//! application [`CredentialCandidate`], asks [`AuthenticationService`] for the
//! principal, and either attaches
//! [`AuthenticatedPrincipal`](aircraft_app::authentication::AuthenticatedPrincipal)
//! to the request extensions or answers with the shared problem contract. A
//! handler reads the principal with axum's `Extension<AuthenticatedPrincipal>`.
//!
//! Every credential rejection -- header missing, repeated, not `Bearer`, token
//! malformed, key unknown, secret wrong, credential revoked, principal
//! disabled -- is the one `401` document with a fixed `WWW-Authenticate:
//! Bearer` challenge. The challenge carries no `realm`, `error`, or
//! `error_description`, because any parameter would tell the states apart. A
//! missing or malformed header is refused here, before the service and its
//! lookup: neither state depends on anything stored, so there is no
//! credential state for the difference in cost to reveal, and answering it
//! from the pool would let a credential-less request compete with real
//! traffic for a bounded connection. Every well-formed token costs the one
//! lookup the service makes.
//!
//! The header is removed from the request before anything is awaited and
//! before the handler runs, so no later layer, extractor, or handler can
//! record it by accident.
//!
//! Placement, for the composition that applies this layer (issue #31): inside
//! `correlate`, so a `401` carries a request ID and a completion event; inside
//! `CorsLayer`, so a preflight never authenticates and a cross-origin `401`
//! keeps its allow-origin header; inside `shed_when_saturated` and
//! `track_in_flight`, so the lookup is in-flight work that shutdown cancels;
//! and inside `enforce_deadline`, so the lookup is bounded by the request
//! deadline as well as the pool. A `route_layer` on a protected router also
//! lands inside `refuse_oversized_body`, so an unauthenticated caller still
//! causes one bounded body read; a composition that can place it outside that
//! layer should. Nothing in this crate applies it yet: `/health`, `/ready`, and
//! `/version` are `Public` and stay unauthenticated.

use std::sync::Arc;

use aircraft_app::{
  authentication::{AuthenticationError, AuthenticationService, CredentialCandidate},
  ingestion::PersistenceError,
};
use axum::{
  extract::{OriginalUri, Request, State},
  http::{HeaderMap, HeaderValue, header},
  middleware::Next,
  response::{IntoResponse as _, Response},
};

use crate::problem::ApiProblem;

/// RFC 6750 section 3: the scheme, and nothing that could vary by state.
const CHALLENGE: HeaderValue = HeaderValue::from_static("Bearer");

/// Refuses a request without an accepted credential, or attaches its
/// principal and calls inward.
pub async fn require_authentication(
  State(service): State<Arc<AuthenticationService>>,
  mut request: Request,
  next: Next,
) -> Response {
  let candidate =
    bearer_token(request.headers()).and_then(|token| CredentialCandidate::parse(token).ok());
  request.headers_mut().remove(header::AUTHORIZATION);
  // `OriginalUri` rather than `uri()`, as the fallbacks do: a nested router
  // sees its prefix stripped, and the path a refusal names must be the one
  // the caller sent. The fallback is for a service driven outside a `Router`.
  let instance = request
    .extensions()
    .get::<OriginalUri>()
    .map_or_else(|| request.uri().path().to_owned(), |original| original.0.path().to_owned());

  let Some(candidate) = candidate else {
    return unauthenticated(&instance);
  };
  match service.authenticate(candidate).await {
    Ok(principal) => {
      request.extensions_mut().insert(principal);
      next.run(request).await
    }
    Err(AuthenticationError::Rejected) => unauthenticated(&instance),
    Err(AuthenticationError::Unavailable(error)) => {
      // The class is logged and not served, as `/ready` does for its probe.
      // No `WWW-Authenticate` here: the credential was not judged, and a
      // challenge would tell the caller to try another one.
      tracing::warn!(code = error.code(), "credential lookup failed");
      match error {
        PersistenceError::Database { .. } => ApiProblem::database_unavailable(&instance),
        // A row the schema forbids is this service's defect, not the
        // database's absence.
        PersistenceError::Invariant(_) => ApiProblem::internal(&instance),
      }
      .into_response()
    }
  }
}

fn unauthenticated(instance: &str) -> Response {
  let mut response = ApiProblem::authentication_required(instance).into_response();
  response.headers_mut().insert(header::WWW_AUTHENTICATE, CHALLENGE);
  response
}

/// The token from exactly one `Authorization: Bearer <token>` field.
///
/// Exactly one, for the reason `RequestId::from_headers` gives: a repeated
/// field is ambiguous about which credential the sender meant, and this
/// service will not pick one for them. The scheme is compared
/// case-insensitively (RFC 7235 section 2.1) and separated by one space;
/// whatever follows is the token, and [`CredentialCandidate::parse`] decides
/// whether it has the issued shape.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
  let mut values = headers.get_all(header::AUTHORIZATION).iter();
  let (Some(value), None) = (values.next(), values.next()) else {
    return None;
  };
  let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
  scheme.eq_ignore_ascii_case("Bearer").then_some(token)
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, so panicking accessors are fine.
  #![allow(clippy::expect_used)]

  use std::{
    sync::{
      Arc, Mutex,
      atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
  };

  use aircraft_app::{
    authentication::{
      AuthenticatedPrincipal, AuthenticationService, CredentialLookup, CredentialLookupRecord,
      Scope,
    },
    credential_issuance::CredentialVerifier,
    ingestion::PersistenceError,
    readiness::ReadinessProbe,
  };
  use anyhow::Result;
  use async_trait::async_trait;
  use axum::{
    Extension, Json, Router,
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::Response,
    routing::get,
  };
  use serde_json::{Value, json};
  use tower::ServiceExt as _;
  use uuid::Uuid;

  use super::require_authentication;
  use crate::{ApiState, PerimeterLimits, router_with_routes, shutdown::ShutdownState};

  /// The published vector from `aircraft_app::credential_issuance`'s tests.
  const TOKEN: &str = "ak1_00010203-0405-4607-8809-0a0b0c0d0e0f_\
                       101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f";
  const PROTECTED: &str = "/__test/protected";
  const PUBLIC: &str = "/__test/public";
  const REQUEST_ID: &str = "authentication-test";

  struct AlwaysReady;

  #[async_trait]
  impl ReadinessProbe for AlwaysReady {
    async fn check(&self) -> Result<(), PersistenceError> {
      Ok(())
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

  /// Answers every `resolve` with the configured outcome and counts the calls.
  struct FakeLookup {
    outcome: Mutex<Result<Option<CredentialLookupRecord>, PersistenceError>>,
    calls: AtomicUsize,
  }

  #[async_trait]
  impl CredentialLookup for FakeLookup {
    async fn resolve(
      &self,
      _key_id: Uuid,
    ) -> Result<Option<CredentialLookupRecord>, PersistenceError> {
      self.calls.fetch_add(1, Ordering::SeqCst);
      match &*self.outcome.lock().expect("fake lookup lock") {
        Ok(record) => Ok(record.clone()),
        Err(PersistenceError::Database { code, message }) => {
          Err(PersistenceError::Database { code: code.clone(), message: message.clone() })
        }
        Err(PersistenceError::Invariant(message)) => {
          Err(PersistenceError::Invariant(message.clone()))
        }
      }
    }
  }

  fn live_record() -> CredentialLookupRecord {
    CredentialLookupRecord {
      verifier: CredentialVerifier::of_token(TOKEN),
      revoked: false,
      disabled: false,
      principal_id: 7,
      scopes: vec![Scope::CatalogRead, Scope::CurationRead],
      tier: "TEST_TIER".to_owned(),
    }
  }

  /// A protected route inside the real perimeter, beside a public one, with
  /// the handler counting its calls and reporting exactly what it received.
  struct Protected {
    router: Router,
    lookup: Arc<FakeLookup>,
    handler_calls: Arc<AtomicUsize>,
  }

  fn protected(outcome: Result<Option<CredentialLookupRecord>, PersistenceError>) -> Protected {
    let lookup = Arc::new(FakeLookup { outcome: Mutex::new(outcome), calls: AtomicUsize::new(0) });
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&handler_calls);
    let whoami = move |Extension(principal): Extension<AuthenticatedPrincipal>,
                       headers: HeaderMap| {
      let calls = Arc::clone(&calls);
      async move {
        calls.fetch_add(1, Ordering::SeqCst);
        Json(json!({
          "principal_id": principal.principal_id(),
          "scopes": principal.scopes().iter().map(|scope| scope.code()).collect::<Vec<_>>(),
          "tier": principal.tier(),
          "authorization_present": headers.contains_key(header::AUTHORIZATION),
        }))
      }
    };
    let routes = Router::new()
      .route(PROTECTED, get(whoami))
      .route_layer(axum::middleware::from_fn_with_state(
        Arc::new(AuthenticationService::new(lookup.clone())),
        require_authentication,
      ))
      .route(PUBLIC, get(|| async { StatusCode::NO_CONTENT }));

    Protected { router: router_with_routes(state(), routes), lookup, handler_calls }
  }

  fn request(path: &str, authorization: &[&str]) -> Result<Request<Body>> {
    let mut request = Request::get(path).header("x-request-id", REQUEST_ID);
    for value in authorization {
      request = request.header(header::AUTHORIZATION, *value);
    }
    Ok(request.body(Body::empty())?)
  }

  async fn body_of(response: Response) -> Result<Value> {
    let bytes = to_bytes(response.into_body(), 4096).await?;
    Ok(serde_json::from_slice(&bytes)?)
  }

  /// A response reduced to what a caller can compare: status, the three
  /// headers that could tell one refusal from another, and the body bytes.
  type Signature = (StatusCode, [Option<HeaderValue>; 3], Vec<u8>);

  async fn signature(response: Response) -> Result<Signature> {
    let status = response.status();
    let headers = [header::CONTENT_TYPE, header::WWW_AUTHENTICATE, "x-request-id".parse()?]
      .map(|name| response.headers().get(&name).cloned());
    let body = to_bytes(response.into_body(), 4096).await?.to_vec();
    Ok((status, headers, body))
  }

  #[tokio::test]
  async fn a_valid_bearer_credential_attaches_the_expected_principal_scopes_and_tier() -> Result<()>
  {
    let protected = protected(Ok(Some(live_record())));

    let response =
      protected.router.oneshot(request(PROTECTED, &[&format!("Bearer {TOKEN}")])?).await?;

    assert_eq!(response.status(), StatusCode::OK);
    let document = body_of(response).await?;
    assert_eq!(document.pointer("/principal_id"), Some(&json!(7)));
    assert_eq!(document.pointer("/scopes"), Some(&json!(["CATALOG_READ", "CURATION_READ"])));
    assert_eq!(document.pointer("/tier"), Some(&json!("TEST_TIER")));
    assert_eq!(protected.lookup.calls.load(Ordering::SeqCst), 1, "exactly one lookup");
    assert_eq!(protected.handler_calls.load(Ordering::SeqCst), 1);
    Ok(())
  }

  /// Acceptance criterion 2. Every rejected state answers with the same bytes
  /// under the same request ID: status, media type, challenge, correlation,
  /// and body are compared against the first case rather than each asserted
  /// separately, so a header or a detail that varies by state fails here
  /// whatever it says. The lookup count per row is criterion 3: none for the
  /// four the header alone decides, one for every well-formed token.
  #[tokio::test]
  async fn every_rejected_credential_state_answers_the_same_401() -> Result<()> {
    let bearer = format!("Bearer {TOKEN}");
    let wrong_secret = CredentialLookupRecord {
      verifier: CredentialVerifier::of_token(&TOKEN.replace("2e2f", "2e2e")),
      ..live_record()
    };
    let cases: [(&str, Vec<&str>, Option<CredentialLookupRecord>, usize); 8] = [
      ("missing header", vec![], Some(live_record()), 0),
      ("repeated header", vec![&bearer, &bearer], Some(live_record()), 0),
      ("basic scheme", vec!["Basic YWxhZGRpbjpvcGVuc2VzYW1l"], Some(live_record()), 0),
      ("malformed token", vec!["Bearer ak1_not-a-credential"], Some(live_record()), 0),
      ("unknown key", vec![&bearer], None, 1),
      ("wrong secret", vec![&bearer], Some(wrong_secret), 1),
      (
        "revoked",
        vec![&bearer],
        Some(CredentialLookupRecord { revoked: true, ..live_record() }),
        1,
      ),
      (
        "disabled",
        vec![&bearer],
        Some(CredentialLookupRecord { disabled: true, ..live_record() }),
        1,
      ),
    ];

    let mut baseline: Option<Signature> = None;
    for (case, authorization, record, lookups) in cases {
      let protected = protected(Ok(record));

      let response = protected.router.oneshot(request(PROTECTED, &authorization)?).await?;

      assert_eq!(protected.lookup.calls.load(Ordering::SeqCst), lookups, "{case}: lookups");
      assert_eq!(protected.handler_calls.load(Ordering::SeqCst), 0, "{case}: handler ran");
      let (status, headers, body) = signature(response).await?;
      match &baseline {
        None => {
          let [content_type, challenge, request_id] = &headers;
          assert_eq!(status, StatusCode::UNAUTHORIZED, "{case}");
          assert_eq!(content_type.as_ref(), Some(&"application/problem+json".parse()?), "{case}");
          assert_eq!(challenge.as_ref(), Some(&"Bearer".parse()?), "{case}: the scheme alone");
          assert_eq!(request_id.as_ref(), Some(&REQUEST_ID.parse()?), "{case}: correlated");
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

  #[tokio::test]
  async fn the_handler_sees_no_authorization_header_after_authentication() -> Result<()> {
    let protected = protected(Ok(Some(live_record())));

    let response =
      protected.router.oneshot(request(PROTECTED, &[&format!("Bearer {TOKEN}")])?).await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      body_of(response).await?.pointer("/authorization_present"),
      Some(&json!(false)),
      "the credential reached the handler"
    );
    Ok(())
  }

  /// A lookup that fails has judged no credential, so the answer is the
  /// dependency problem or, for stored state outside the schema's contract,
  /// the internal one -- never the `401`, and never with a challenge.
  #[tokio::test]
  async fn a_lookup_failure_answers_a_dependency_or_internal_problem_not_a_401() -> Result<()> {
    let cases = [
      (
        "database failure",
        PersistenceError::Database {
          code: "DATABASE_57P01".to_owned(),
          message: "gone".to_owned(),
        },
        StatusCode::SERVICE_UNAVAILABLE,
        "/problems/database-unavailable",
      ),
      (
        "stored-state invariant",
        PersistenceError::Invariant("stored credential digest is not 32 bytes".to_owned()),
        StatusCode::INTERNAL_SERVER_ERROR,
        "/problems/internal-error",
      ),
    ];

    for (case, error, status, kind) in cases {
      let protected = protected(Err(error));

      let response =
        protected.router.oneshot(request(PROTECTED, &[&format!("Bearer {TOKEN}")])?).await?;

      assert_eq!(response.status(), status, "{case}");
      assert_eq!(protected.lookup.calls.load(Ordering::SeqCst), 1, "{case}");
      assert_eq!(protected.handler_calls.load(Ordering::SeqCst), 0, "{case}: handler ran");
      assert!(
        response.headers().get(header::WWW_AUTHENTICATE).is_none(),
        "{case}: a failed lookup must not challenge"
      );
      assert_eq!(response.headers().get("x-request-id"), Some(&REQUEST_ID.parse()?), "{case}");
      let document = body_of(response).await?;
      assert_eq!(document.pointer("/type"), Some(&json!(kind)), "{case}");
      assert_eq!(document.pointer("/instance"), Some(&json!(PROTECTED)), "{case}");
    }
    Ok(())
  }

  /// A nested router sees its prefix stripped from `uri()`, so a refusal that
  /// read the path from there would name `/protected` for a request the caller
  /// sent to `/__test/nested/protected`.
  #[tokio::test]
  async fn a_nested_protected_route_names_the_path_the_caller_sent() -> Result<()> {
    const NESTED: &str = "/__test/nested/protected";
    let lookup = Arc::new(FakeLookup { outcome: Mutex::new(Ok(None)), calls: AtomicUsize::new(0) });
    let inner = Router::new().route("/protected", get(|| async { StatusCode::OK })).route_layer(
      axum::middleware::from_fn_with_state(
        Arc::new(AuthenticationService::new(lookup)),
        require_authentication,
      ),
    );
    let router = router_with_routes(state(), Router::new().nest("/__test/nested", inner));

    let response = router.oneshot(request(NESTED, &[])?).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_of(response).await?.pointer("/instance"), Some(&json!(NESTED)));
    Ok(())
  }

  /// The layer is scoped to the routes it is applied to, not the router.
  #[tokio::test]
  async fn a_route_outside_the_layer_needs_no_credential() -> Result<()> {
    let protected = protected(Ok(None));

    let response = protected.router.oneshot(request(PUBLIC, &[])?).await?;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(protected.lookup.calls.load(Ordering::SeqCst), 0, "no lookup for a public route");
    Ok(())
  }
}
