//! Route registration, and the policy every registration carries.
//!
//! [`Routes`] is the only value [`crate::router_with_routes`] accepts, and
//! [`Routes::route`] is the only way a handler gets into one, so a route cannot
//! reach the application router without naming a [`RoutePolicy`]. What comes
//! back out is a sealed [`crate::ApplicationRouter`], so no route can be added
//! afterwards either. The policies are the closed set
//! `docs/architecture/http_v1_decisions.md` publishes under "Authentication and
//! route policies", which names this module in turn; a seventh policy is a
//! change to that decision, not to this file.
//!
//! Registration is enforcement. [`Routes::route`] wraps the handler in
//! `Policed`, which for a scoped policy authenticates the request, compares
//! the principal's grants with [`RoutePolicy::required_scope`], and only then
//! calls the handler; `Public` calls it directly. Registration also records
//! what it was given: one [`RouteMethod`], one served path, and one policy per
//! entry. That inventory is what `crate::openapi` publishes each scoped
//! operation's security requirement from, and what the tests read against the
//! generated `OpenAPI` document operation by operation. `/health`, `/ready`,
//! and `/version` are `Public`; no protected route is served yet.

pub mod health;
pub mod ready;
pub mod version;

use std::{convert::Infallible, future::Future, pin::Pin};

use aircraft_app::authentication::Scope;
use axum::{
  Router,
  extract::{OriginalUri, Request},
  handler::Handler,
  response::{IntoResponse, Response},
  routing::{MethodFilter, Route, on},
};
use tower::{Layer, Service};
use utoipa::openapi::path::{Operation, PathItem};

use crate::{ApiState, authentication, problem::ApiProblem};

/// Who may call a route.
///
/// `Public` needs no credential. Every other policy names the scope an
/// authenticated principal must hold, through [`Self::required_scope`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutePolicy {
  Public,
  CatalogRead,
  MilitaryRead,
  CurationRead,
  CurationWrite,
  Admin,
}

impl RoutePolicy {
  /// The scope this policy requires, or `None` for a route anyone may call.
  ///
  /// An application [`Scope`], the vocabulary an authenticated principal
  /// carries, rather than the `problem::RequiredScope` a `403` names: this is
  /// what enforcement compares, and the response DTO is mapped from it at the
  /// point of refusal, not the other way round.
  #[must_use]
  pub const fn required_scope(self) -> Option<Scope> {
    match self {
      Self::Public => None,
      Self::CatalogRead => Some(Scope::CatalogRead),
      Self::MilitaryRead => Some(Scope::MilitaryRead),
      Self::CurationRead => Some(Scope::CurationRead),
      Self::CurationWrite => Some(Scope::CurationWrite),
      Self::Admin => Some(Scope::Admin),
    }
  }
}

/// The method a registration answers.
///
/// Closed to the `OpenAPI` Path Item operations, one per registration, so
/// every inventory entry is exactly one published operation. `axum`'s
/// `MethodFilter` would also admit a union of methods and `CONNECT`, neither of
/// which an `OpenAPI` document can carry. `OPTIONS` is an operation the
/// document could carry but this router could never serve: the perimeter's
/// CORS layer answers every `OPTIONS` request itself without calling inward
/// (`tower-http` 0.7, `cors/mod.rs`), so a handler registered under it would
/// be unreachable, and the variant is left out rather than left as a trap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteMethod {
  Delete,
  Get,
  Head,
  Patch,
  Post,
  Put,
  Trace,
}

impl RouteMethod {
  const fn filter(self) -> MethodFilter {
    match self {
      Self::Delete => MethodFilter::DELETE,
      Self::Get => MethodFilter::GET,
      Self::Head => MethodFilter::HEAD,
      Self::Patch => MethodFilter::PATCH,
      Self::Post => MethodFilter::POST,
      Self::Put => MethodFilter::PUT,
      Self::Trace => MethodFilter::TRACE,
    }
  }

  /// The Path Item member the generated document publishes this method's
  /// operation under.
  pub(crate) const fn operation_mut(self, item: &mut PathItem) -> &mut Option<Operation> {
    match self {
      Self::Delete => &mut item.delete,
      Self::Get => &mut item.get,
      Self::Head => &mut item.head,
      Self::Patch => &mut item.patch,
      Self::Post => &mut item.post,
      Self::Put => &mut item.put,
      Self::Trace => &mut item.trace,
    }
  }
}

/// One registration, as [`Routes::route`] recorded it.
///
/// `path` is the full served path: a route registered under
/// [`Routes::nest`] carries its prefix here, so an entry can be read against
/// a request or an `OpenAPI` path without knowing how the router was assembled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RegisteredRoute {
  pub(crate) method: RouteMethod,
  pub(crate) path: String,
  pub(crate) policy: RoutePolicy,
}

/// Routes and the policy each was registered with.
///
/// The router inside is private, and no public method exposes it, so the inventory
/// is complete by construction: a handler is in the router only if a
/// registration put it there. Layers may be applied, because a layer wraps
/// routes and adds none.
#[derive(Debug, Default)]
pub struct Routes {
  router: Router<ApiState>,
  inventory: Vec<RegisteredRoute>,
}

impl Routes {
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Registers `handler` for `method` at `path` under `policy`, which is
  /// enforced in front of it from then on.
  ///
  /// Two registrations at one path with different methods share the path, as
  /// `Router::route` merges them, so a read and a write on one resource can
  /// carry different policies. The same method registered twice at one path
  /// panics, as it does in `axum`, and so does a path `axum` rejects: both are
  /// composition mistakes and surface when the router is built, before it
  /// serves anything.
  #[must_use]
  pub fn route<H, T>(
    mut self,
    method: RouteMethod,
    path: &str,
    policy: RoutePolicy,
    handler: H,
  ) -> Self
  where
    H: Handler<T, ApiState>,
    T: 'static,
  {
    self.router = self.router.route(path, on(method.filter(), Policed { policy, handler }));
    self.inventory.push(RegisteredRoute { method, path: path.to_owned(), policy });
    self
  }

  /// Mounts `inner` under `prefix`, as `Router::nest` does, and records each of
  /// its routes under the full path a caller sends.
  #[must_use]
  pub fn nest(mut self, prefix: &str, inner: Self) -> Self {
    self.router = self.router.nest(prefix, inner.router);
    self.inventory.extend(
      inner
        .inventory
        .into_iter()
        .map(|route| RegisteredRoute { path: nested_path(prefix, &route.path), ..route }),
    );
    self
  }

  /// Adds every route of `other`, as `Router::merge` does.
  #[must_use]
  pub fn merge(mut self, other: Self) -> Self {
    self.router = self.router.merge(other.router);
    self.inventory.extend(other.inventory);
    self
  }

  /// Wraps the routes registered so far, as `Router::route_layer` does: the
  /// layer runs only for a matched route, never for a fallback.
  #[must_use]
  pub fn route_layer<L>(mut self, layer: L) -> Self
  where
    L: Layer<Route> + Clone + Send + Sync + 'static,
    L::Service: Service<Request> + Clone + Send + Sync + 'static,
    <L::Service as Service<Request>>::Response: IntoResponse + 'static,
    <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
    <L::Service as Service<Request>>::Future: Send + 'static,
  {
    self.router = self.router.route_layer(layer);
    self
  }

  /// Wraps everything registered so far, as `Router::layer` does.
  #[must_use]
  pub fn layer<L>(mut self, layer: L) -> Self
  where
    L: Layer<Route> + Clone + Send + Sync + 'static,
    L::Service: Service<Request> + Clone + Send + Sync + 'static,
    <L::Service as Service<Request>>::Response: IntoResponse + 'static,
    <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
    <L::Service as Service<Request>>::Future: Send + 'static,
  {
    self.router = self.router.layer(layer);
    self
  }

  /// Every registration, in registration order.
  ///
  /// Crate-visible so the `OpenAPI` generator can read it without the record
  /// becoming public API.
  #[must_use]
  pub(crate) fn inventory(&self) -> &[RegisteredRoute] {
    &self.inventory
  }

  pub(crate) fn into_router(self) -> Router<ApiState> {
    self.router
  }
}

/// A handler behind the policy it was registered under.
///
/// The check is the handler, not a layer around it: [`Handler::call`] receives
/// the router state, which is where the authentication service lives, and it
/// is what axum invokes for a synthesized `HEAD` on a `GET` registration and
/// for a route under any `nest` prefix, so nothing about how the router was
/// assembled can route around it. The cost is placement inside every perimeter
/// layer, which `crate::authentication` accounts for.
#[derive(Clone)]
struct Policed<H> {
  policy: RoutePolicy,
  handler: H,
}

impl<H, T> Handler<T, ApiState> for Policed<H>
where
  H: Handler<T, ApiState>,
  T: 'static,
{
  type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

  fn call(self, mut request: Request, state: ApiState) -> Self::Future {
    Box::pin(async move {
      let Some(required) = self.policy.required_scope() else {
        return self.handler.call(request, state).await;
      };
      let instance = instance_of(&request);
      let principal =
        match authentication::authenticate(&state.authentication, &mut request, &instance).await {
          Ok(principal) => principal,
          Err(problem) => return problem.into_response(),
        };
      // The effective scopes are the principal's grants: migration 025 stores
      // `principal_scope_grants` per principal and nothing per credential, so
      // this comparison is the one place a narrower credential would change.
      if !principal.scopes().contains(&required) {
        return ApiProblem::insufficient_scope(&instance, required.into()).into_response();
      }
      request.extensions_mut().insert(principal);
      self.handler.call(request, state).await
    })
  }
}

/// The path a refusal names: `OriginalUri` rather than `uri()`, as the
/// fallbacks do, because a nested router sees its prefix stripped and the path
/// must be the one the caller sent. The fallback is for a service driven
/// outside a `Router`.
fn instance_of(request: &Request) -> String {
  request
    .extensions()
    .get::<OriginalUri>()
    .map_or_else(|| request.uri().path().to_owned(), |original| original.0.path().to_owned())
}

/// The path `axum` serves an `inner` route at once nested under `prefix`.
///
/// `Router::nest`'s own rule, which is not concatenation: an inner `/` is
/// served at the prefix itself, not at `prefix/`, and a prefix ending in `/`
/// absorbs the inner path's leading slash rather than doubling it. Mirrored
/// here because `axum` keeps its function private; the guard is
/// `nested_registrations_are_inventoried_at_the_paths_axum_serves` in
/// `lib.rs`, which requests each recorded path through the real router, so a
/// future `axum` that changes the rule fails that test rather than this table.
fn nested_path(prefix: &str, inner: &str) -> String {
  if prefix.ends_with('/') {
    format!("{prefix}{}", inner.trim_start_matches('/'))
  } else if inner == "/" {
    prefix.to_owned()
  } else {
    format!("{prefix}{inner}")
  }
}

#[cfg(test)]
mod tests {
  use aircraft_app::authentication::Scope;
  use axum::http::StatusCode;

  use super::{RegisteredRoute, RouteMethod, RoutePolicy, Routes, nested_path};

  async fn ok() -> StatusCode {
    StatusCode::OK
  }

  /// The policy-to-scope table from `docs/architecture/http_v1_decisions.md`:
  /// catalog reads use `CatalogRead`, military data `MilitaryRead`, pending
  /// evidence `CurationRead`, decisions `CurationWrite`, administration
  /// `Admin`, and only `Public` needs no scope. Exhaustive on purpose, so a
  /// seventh policy is a compile error here rather than an unmapped one.
  #[test]
  fn only_public_requires_no_scope_and_every_other_policy_names_its_own() {
    const POLICIES: [RoutePolicy; 6] = [
      RoutePolicy::Public,
      RoutePolicy::CatalogRead,
      RoutePolicy::MilitaryRead,
      RoutePolicy::CurationRead,
      RoutePolicy::CurationWrite,
      RoutePolicy::Admin,
    ];

    for policy in POLICIES {
      let expected = match policy {
        RoutePolicy::Public => None,
        RoutePolicy::CatalogRead => Some(Scope::CatalogRead),
        RoutePolicy::MilitaryRead => Some(Scope::MilitaryRead),
        RoutePolicy::CurationRead => Some(Scope::CurationRead),
        RoutePolicy::CurationWrite => Some(Scope::CurationWrite),
        RoutePolicy::Admin => Some(Scope::Admin),
      };
      assert_eq!(policy.required_scope(), expected, "{policy:?}");
    }
  }

  /// `Router::nest`'s path rule, as `axum` 0.8 documents it: nesting `/`
  /// under `/api` serves `/api`, and a prefix that already ends in `/` does
  /// not gain a second separator. Plain concatenation gets the first two
  /// rows wrong.
  #[test]
  fn a_nested_path_follows_axum_and_not_concatenation() {
    const CASES: [(&str, &str, &str); 4] = [
      ("/api", "/", "/api"),
      ("/api/", "/users", "/api/users"),
      ("/api/", "/", "/api/"),
      ("/api", "/users", "/api/users"),
    ];

    for (prefix, inner, expected) in CASES {
      assert_eq!(nested_path(prefix, inner), expected, "nest {inner:?} under {prefix:?}");
    }
  }

  /// A nested registration is recorded under the path a caller sends, and a
  /// merged one keeps its own; neither loses its method or policy on the way.
  #[test]
  fn nested_and_merged_registrations_are_inventoried_under_their_served_paths() {
    let inner =
      Routes::new().route(RouteMethod::Post, "/decisions", RoutePolicy::CurationWrite, ok);
    let sibling =
      Routes::new().route(RouteMethod::Get, "/v1/aircraft", RoutePolicy::CatalogRead, ok);

    let routes = Routes::new()
      .route(RouteMethod::Get, "/health", RoutePolicy::Public, ok)
      .nest("/v1/curation", inner)
      .merge(sibling);

    assert_eq!(
      routes.inventory(),
      [
        RegisteredRoute {
          method: RouteMethod::Get,
          path: "/health".into(),
          policy: RoutePolicy::Public
        },
        RegisteredRoute {
          method: RouteMethod::Post,
          path: "/v1/curation/decisions".into(),
          policy: RoutePolicy::CurationWrite,
        },
        RegisteredRoute {
          method: RouteMethod::Get,
          path: "/v1/aircraft".into(),
          policy: RoutePolicy::CatalogRead,
        },
      ]
    );
  }
}
