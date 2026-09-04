//! Bearer authentication at the HTTP boundary.
//!
//! `authenticate` turns the `Authorization` header into an application
//! [`CredentialCandidate`], asks [`AuthenticationService`] for the principal,
//! and returns either the [`AuthenticatedPrincipal`] or the problem that
//! refuses the request. Its one caller is the registration
//! wrapper in `crate::routes`, which runs it for every route registered under a
//! scoped policy and, once the scope is checked, attaches the principal to the
//! request extensions; a handler reads it with axum's
//! `Extension<AuthenticatedPrincipal>`.
//!
//! Every credential rejection -- header missing, repeated, not `Bearer`, token
//! malformed, key unknown, secret wrong, credential revoked, principal
//! disabled -- is the one `401` document with a fixed `WWW-Authenticate:
//! Bearer` challenge, which the problem renderer adds. The challenge carries no
//! `realm`, `error`, or `error_description`, because any parameter would tell
//! the states apart. A missing or malformed header is refused here, before the
//! service and its lookup: neither state depends on anything stored, so there
//! is no credential state for the difference in cost to reveal, and answering
//! it from the pool would let a credential-less request compete with real
//! traffic for a bounded connection. Every well-formed token costs the one
//! lookup the service makes.
//!
//! The header is removed from the request before anything is awaited and
//! before the handler runs, so no later layer, extractor, or handler can
//! record it by accident.
//!
//! Placement follows from the wrapper being the handler: it runs inside
//! `correlate`, so a `401` carries a request ID and a completion event; inside
//! `CorsLayer`, so a preflight never authenticates and a cross-origin `401`
//! keeps its allow-origin header; inside `shed_when_saturated` and
//! `track_in_flight`, so the lookup is in-flight work that shutdown cancels;
//! inside `enforce_deadline`, so the lookup is bounded by the request deadline
//! as well as the pool; and inside `refuse_oversized_body`, so an
//! unauthenticated caller still costs one body read, bounded by that layer's
//! limit and by the semaphore's count of concurrent buffers. Refusing before
//! that read would need enforcement outside the handler, keyed by the matched
//! route; that is a perimeter change, not a registration one.

use aircraft_app::{
  authentication::{
    AuthenticatedPrincipal, AuthenticationError, AuthenticationService, CredentialCandidate,
  },
  ingestion::PersistenceError,
};
use axum::{
  extract::Request,
  http::{HeaderMap, header},
};

use crate::problem::ApiProblem;

/// Resolves the request's credential to its principal, or refuses for it.
///
/// `instance` is the path the refusal names, computed by the caller because
/// the `403` it may answer next names the same one.
///
/// # Errors
///
/// The `401` problem for every rejected credential state, including a missing
/// or malformed header. When the lookup itself failed, the database
/// unavailable `503`, or the internal `500` for stored state the schema
/// forbids; neither carries a challenge, because no credential was judged and
/// a challenge would tell the caller to try another one.
pub(crate) async fn authenticate(
  service: &AuthenticationService,
  request: &mut Request,
  instance: &str,
) -> Result<AuthenticatedPrincipal, ApiProblem> {
  let candidate =
    bearer_token(request.headers()).and_then(|token| CredentialCandidate::parse(token).ok());
  request.headers_mut().remove(header::AUTHORIZATION);

  let Some(candidate) = candidate else {
    return Err(ApiProblem::authentication_required(instance));
  };
  match service.authenticate(candidate).await {
    Ok(principal) => Ok(principal),
    Err(AuthenticationError::Rejected) => Err(ApiProblem::authentication_required(instance)),
    Err(AuthenticationError::Unavailable(error)) => {
      // The class is logged and not served, as `/ready` does for its probe.
      tracing::warn!(code = error.code(), "credential lookup failed");
      Err(match error {
        PersistenceError::Database { .. } => ApiProblem::database_unavailable(instance),
        // A row the schema forbids is this service's defect, not the
        // database's absence.
        PersistenceError::Invariant(_) => ApiProblem::internal(instance),
      })
    }
  }
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
    Extension, Json,
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::Response,
  };
  use serde_json::{Value, json};
  use tower::ServiceExt as _;
  use uuid::Uuid;

  use crate::{
    ApiState, ApplicationRouter, PerimeterLimits, router_with_routes,
    routes::{RouteMethod, RoutePolicy, Routes},
    shutdown::ShutdownState,
  };

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

  fn state(lookup: Arc<dyn CredentialLookup>) -> ApiState {
    ApiState {
      readiness: Arc::new(AlwaysReady),
      authentication: Arc::new(AuthenticationService::new(lookup)),
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

  fn record_with(scopes: Vec<Scope>) -> CredentialLookupRecord {
    CredentialLookupRecord { scopes, ..live_record() }
  }

  /// A route under a policy inside the real perimeter, beside a public one,
  /// with the handler counting its calls and reporting exactly what it
  /// received. No layer is applied by hand: the registration is the
  /// enforcement, which is what every test here is about.
  struct Protected {
    router: ApplicationRouter,
    lookup: Arc<FakeLookup>,
    handler_calls: Arc<AtomicUsize>,
  }

  fn protected(outcome: Result<Option<CredentialLookupRecord>, PersistenceError>) -> Protected {
    protected_under(RoutePolicy::CatalogRead, PROTECTED, outcome)
  }

  fn protected_under(
    policy: RoutePolicy,
    path: &str,
    outcome: Result<Option<CredentialLookupRecord>, PersistenceError>,
  ) -> Protected {
    let lookup = Arc::new(FakeLookup { outcome: Mutex::new(outcome), calls: AtomicUsize::new(0) });
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&handler_calls);
    // The principal is optional so the same handler can sit under `Public`,
    // where none is attached; a scoped test reads it back and so proves it was.
    let whoami = move |principal: Option<Extension<AuthenticatedPrincipal>>, headers: HeaderMap| {
      let calls = Arc::clone(&calls);
      async move {
        calls.fetch_add(1, Ordering::SeqCst);
        let principal = principal.map(|Extension(principal)| principal);
        Json(json!({
          "principal_id": principal.as_ref().map(AuthenticatedPrincipal::principal_id),
          "scopes": principal
            .as_ref()
            .map(|principal| principal.scopes().iter().map(|scope| scope.code()).collect::<Vec<_>>()),
          "tier": principal.as_ref().map(AuthenticatedPrincipal::tier),
          "authorization_present": headers.contains_key(header::AUTHORIZATION),
        }))
      }
    };
    let routes = Routes::new().route(RouteMethod::Get, path, policy, whoami).route(
      RouteMethod::Get,
      PUBLIC,
      RoutePolicy::Public,
      || async { StatusCode::NO_CONTENT },
    );

    Protected { router: router_with_routes(state(lookup.clone()), routes), lookup, handler_calls }
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
    let inner =
      Routes::new().route(RouteMethod::Get, "/protected", RoutePolicy::CatalogRead, || async {
        StatusCode::OK
      });
    let router = router_with_routes(state(lookup), Routes::new().nest("/__test/nested", inner));

    let response = router.oneshot(request(NESTED, &[])?).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_of(response).await?.pointer("/instance"), Some(&json!(NESTED)));
    Ok(())
  }

  /// Enforcement is per registration, not per router: the public route beside
  /// the protected one is answered with no credential and no lookup.
  #[tokio::test]
  async fn a_public_registration_needs_no_credential_and_makes_no_lookup() -> Result<()> {
    let protected = protected(Ok(None));

    let response = protected.router.oneshot(request(PUBLIC, &[])?).await?;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(protected.lookup.calls.load(Ordering::SeqCst), 0, "no lookup for a public route");
    Ok(())
  }

  /// Acceptance criterion 1 of issue #31: a registration under a scoped
  /// policy, and nothing else, is what protects the handler. No layer is
  /// applied by hand anywhere in this module, so a router that only enforces
  /// what a caller composed answers `200` and fails this.
  #[tokio::test]
  async fn a_scoped_registration_refuses_an_unauthenticated_request() -> Result<()> {
    let protected = protected(Ok(Some(live_record())));

    let response = protected.router.oneshot(request(PROTECTED, &[])?).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers().get(header::WWW_AUTHENTICATE), Some(&"Bearer".parse()?));
    assert_eq!(
      body_of(response).await?.pointer("/type"),
      Some(&json!("/problems/authentication-required"))
    );
    assert_eq!(protected.lookup.calls.load(Ordering::SeqCst), 0, "no header, so no lookup");
    assert_eq!(
      protected.handler_calls.load(Ordering::SeqCst),
      0,
      "the handler ran unauthenticated"
    );
    Ok(())
  }

  /// Acceptance criteria 1 to 3 of issue #31 for every policy class. Each
  /// scoped policy is driven three ways: no credential is the `401`; a
  /// principal holding every scope but the required one is the `403` naming
  /// that scope, after one lookup and before the handler; a principal holding
  /// exactly the required scope reaches the handler after one lookup, which
  /// reads that principal back. `Public` reaches its handler with no
  /// credential and no lookup.
  ///
  /// The required scope is the table in `docs/architecture/http_v1_decisions.md`,
  /// matched exhaustively, so a seventh policy stops this compiling rather
  /// than going untested. The "every scope but the required one" row is what
  /// separates "holds the scope" from "holds a scope".
  #[tokio::test]
  async fn every_policy_class_answers_401_403_and_authorized_through_the_router() -> Result<()> {
    const POLICIES: [RoutePolicy; 6] = [
      RoutePolicy::Public,
      RoutePolicy::CatalogRead,
      RoutePolicy::MilitaryRead,
      RoutePolicy::CurationRead,
      RoutePolicy::CurationWrite,
      RoutePolicy::Admin,
    ];
    const ALL_SCOPES: [Scope; 5] = [
      Scope::CatalogRead,
      Scope::MilitaryRead,
      Scope::CurationRead,
      Scope::CurationWrite,
      Scope::Admin,
    ];
    const fn decided_scope(policy: RoutePolicy) -> Option<Scope> {
      match policy {
        RoutePolicy::Public => None,
        RoutePolicy::CatalogRead => Some(Scope::CatalogRead),
        RoutePolicy::MilitaryRead => Some(Scope::MilitaryRead),
        RoutePolicy::CurationRead => Some(Scope::CurationRead),
        RoutePolicy::CurationWrite => Some(Scope::CurationWrite),
        RoutePolicy::Admin => Some(Scope::Admin),
      }
    }
    let bearer = format!("Bearer {TOKEN}");

    for policy in POLICIES {
      let path = format!("/__test/{policy:?}");
      let Some(required) = decided_scope(policy) else {
        let public = protected_under(policy, &path, Ok(Some(live_record())));

        let response = public.router.oneshot(request(&path, &[])?).await?;

        assert_eq!(response.status(), StatusCode::OK, "{policy:?}");
        assert_eq!(public.lookup.calls.load(Ordering::SeqCst), 0, "{policy:?}: lookups");
        assert_eq!(public.handler_calls.load(Ordering::SeqCst), 1, "{policy:?}: handler");
        continue;
      };

      let unauthenticated =
        protected_under(policy, &path, Ok(Some(record_with(ALL_SCOPES.to_vec()))));
      let response = unauthenticated.router.oneshot(request(&path, &[])?).await?;
      assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{policy:?}: no credential");
      assert_eq!(unauthenticated.lookup.calls.load(Ordering::SeqCst), 0, "{policy:?}: lookups");
      assert_eq!(unauthenticated.handler_calls.load(Ordering::SeqCst), 0, "{policy:?}: handler");

      let others = ALL_SCOPES.into_iter().filter(|scope| *scope != required).collect();
      let forbidden = protected_under(policy, &path, Ok(Some(record_with(others))));
      let response = forbidden.router.oneshot(request(&path, &[&bearer])?).await?;
      assert_eq!(response.status(), StatusCode::FORBIDDEN, "{policy:?}: every other scope");
      assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none(), "{policy:?}: challenged");
      let document = body_of(response).await?;
      assert_eq!(
        document.pointer("/type"),
        Some(&json!("/problems/insufficient-scope")),
        "{policy:?}"
      );
      assert_eq!(document.pointer("/required_scope"), Some(&json!(required.code())), "{policy:?}");
      assert_eq!(forbidden.lookup.calls.load(Ordering::SeqCst), 1, "{policy:?}: lookups");
      assert_eq!(forbidden.handler_calls.load(Ordering::SeqCst), 0, "{policy:?}: handler");

      let authorized = protected_under(policy, &path, Ok(Some(record_with(vec![required]))));
      let response = authorized.router.oneshot(request(&path, &[&bearer])?).await?;
      assert_eq!(response.status(), StatusCode::OK, "{policy:?}: exactly the scope");
      let document = body_of(response).await?;
      assert_eq!(document.pointer("/scopes"), Some(&json!([required.code()])), "{policy:?}");
      assert_eq!(authorized.lookup.calls.load(Ordering::SeqCst), 1, "{policy:?}: lookups");
      assert_eq!(authorized.handler_calls.load(Ordering::SeqCst), 1, "{policy:?}: handler");
    }
    Ok(())
  }

  /// axum answers a `HEAD` with the `GET` handler when no `HEAD` is registered
  /// (`method_routing.rs`: `call!(req, HEAD, get)`), so the `GET`'s wrapper
  /// is what must run for it.
  #[tokio::test]
  async fn a_head_request_to_a_scoped_get_route_is_refused_without_a_credential() -> Result<()> {
    let protected = protected(Ok(Some(live_record())));

    let response = protected.router.oneshot(Request::head(PROTECTED).body(Body::empty())?).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(protected.handler_calls.load(Ordering::SeqCst), 0, "the handler ran for HEAD");
    Ok(())
  }

  /// The `403` in full, on a nested route: the caller's path, the problem
  /// media type, the caller's request ID, no challenge, and a handler that
  /// never ran.
  #[tokio::test]
  async fn an_insufficient_scope_refusal_is_the_403_document_with_no_challenge() -> Result<()> {
    const NESTED: &str = "/__test/nested/decisions";
    let lookup = Arc::new(FakeLookup {
      outcome: Mutex::new(Ok(Some(record_with(vec![Scope::CurationRead])))),
      calls: AtomicUsize::new(0),
    });
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&handler_calls);
    let inner =
      Routes::new().route(RouteMethod::Post, "/decisions", RoutePolicy::CurationWrite, move || {
        let calls = Arc::clone(&calls);
        async move {
          calls.fetch_add(1, Ordering::SeqCst);
          StatusCode::OK
        }
      });
    let router =
      router_with_routes(state(lookup.clone()), Routes::new().nest("/__test/nested", inner));

    let response = router
      .oneshot(
        Request::post(NESTED)
          .header("x-request-id", REQUEST_ID)
          .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
          .body(Body::empty())?,
      )
      .await?;

    let (status, [content_type, challenge, request_id], body) = signature(response).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(content_type.as_ref(), Some(&"application/problem+json".parse()?));
    assert!(challenge.is_none(), "a 403 has judged the credential and must not challenge");
    assert_eq!(request_id.as_ref(), Some(&REQUEST_ID.parse()?), "correlated");
    let document: Value = serde_json::from_slice(&body)?;
    assert_eq!(document.pointer("/type"), Some(&json!("/problems/insufficient-scope")));
    assert_eq!(document.pointer("/instance"), Some(&json!(NESTED)), "the caller's path");
    assert_eq!(document.pointer("/required_scope"), Some(&json!("CURATION_WRITE")));
    assert_eq!(lookup.calls.load(Ordering::SeqCst), 1, "one lookup");
    assert_eq!(handler_calls.load(Ordering::SeqCst), 0, "the handler ran");
    Ok(())
  }
}
