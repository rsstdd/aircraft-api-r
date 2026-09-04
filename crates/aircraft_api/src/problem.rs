//! Problem documents, as defined by RFC 9457.
//!
//! Every field is a fixed string chosen at compile time. Nothing here is
//! derived from an error value, which is what keeps `detail` -- the field the
//! specification provides for an explanation, and therefore the one a database
//! diagnostic would leak through -- safe to publish. `aircraft_api` never sees
//! a connection string, but it does see `PersistenceError`, whose message
//! carries whatever `SQLx` reported.

use std::{borrow::Cow, collections::BTreeMap};

use aircraft_app::authentication::Scope;
use axum::{
  Json as AxumJson,
  extract::{
    FromRequest, FromRequestParts, Query as AxumQuery, Request,
    rejection::{JsonRejection, QueryRejection},
  },
  http::{HeaderValue, StatusCode, Uri, header, request::Parts},
  response::{IntoResponse, Response},
};
use serde::{Serialize, de::DeserializeOwned};
use utoipa::{
  IntoResponses, ToResponse, ToSchema,
  openapi::{
    HeaderBuilder, Ref, RefOr, ResponseBuilder, ResponsesBuilder,
    content::ContentBuilder,
    schema::{AllOfBuilder, ObjectBuilder, Schema},
  },
};

/// The media type RFC 9457 assigns to these documents. A client that
/// distinguishes problems from ordinary payloads keys on this, not on the
/// status code.
const PROBLEM_MEDIA_TYPE: &str = "application/problem+json";

/// RFC 6750 section 3: the scheme, and nothing that could vary by credential
/// state. Added to every `401` by [`ApiProblem::into_response`], so no path
/// that answers one can leave the challenge off.
const BEARER_CHALLENGE: HeaderValue = HeaderValue::from_static("Bearer");

// A closed enum rather than a string because these spellings mirror the five
// `code` values seeded into `aircraft_auth.scopes` by
// `database/seeds/004_authentication_seed_data.sql`, which names this type in
// turn; `SCREAMING_SNAKE_CASE` is what makes the two agree. The guard is
// `openapi_publishes_the_scope_vocabulary_as_an_optional_never_null_member`,
// which reads the published list against the seed rather than against this enum.
/// The scope the caller's credential is missing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RequiredScope {
  CatalogRead,
  MilitaryRead,
  CurationRead,
  CurationWrite,
  Admin,
}

/// The application vocabulary a principal holds, mapped to the transport one
/// at the point of refusal, so the two types stay separate representations.
/// No wildcard arm: a sixth `Scope` does not compile until it has a published
/// spelling, pinned by
/// `every_application_scope_maps_to_the_scope_the_problem_publishes`.
impl From<Scope> for RequiredScope {
  fn from(scope: Scope) -> Self {
    match scope {
      Scope::CatalogRead => Self::CatalogRead,
      Scope::MilitaryRead => Self::MilitaryRead,
      Scope::CurationRead => Self::CurationRead,
      Scope::CurationWrite => Self::CurationWrite,
      Scope::Admin => Self::Admin,
    }
  }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
  /// A URI reference identifying the problem type. Relative by design: it
  /// resolves against the request URL, so no environment has to agree on a
  /// hostname for the link to be correct.
  #[serde(rename = "type")]
  kind: &'static str,
  title: &'static str,
  status: u16,
  detail: &'static str,
  /// The specific occurrence this problem describes.
  ///
  /// Required, because `/ready` has published one since it shipped and
  /// `oasdiff` fails the build on `response-property-became-optional`. A route
  /// names itself with a borrowed constant, a perimeter refusal owns the path
  /// it refused, and `Cow` is what lets one required field be both.
  #[schema(value_type = String)]
  instance: Cow<'static, str>,
  /// The scope required by the refused operation. Present only on an
  /// authorization problem, and absent rather than null everywhere else.
  #[serde(skip_serializing_if = "Option::is_none")]
  // `utoipa` renders any `Option<T>` as a nullable union without reading
  // `skip_serializing_if`, so without `nullable = false` the contract would
  // publish a `null` this service never emits -- the same correction
  // `VersionResponse::build_commit` carries.
  #[schema(nullable = false)]
  required_scope: Option<RequiredScope>,
}

impl ProblemDetails {
  /// The refused request's path, bounded and kept URI-valid before reflection.
  fn bounded_instance(path: &str) -> Cow<'static, str> {
    const MAX_INSTANCE_CHARS: usize = 255;
    const INVALID_INSTANCE: &str = "/invalid-request-path";
    const OVERSIZED_INSTANCE: &str = "/request-path-too-long";

    let sanitized = path.chars().take(MAX_INSTANCE_CHARS + 1).collect::<String>();

    if sanitized.chars().count() > MAX_INSTANCE_CHARS {
      return Cow::Borrowed(OVERSIZED_INSTANCE);
    }
    let Ok(uri) = sanitized.parse::<Uri>() else {
      return Cow::Borrowed(INVALID_INSTANCE);
    };
    if sanitized.chars().any(char::is_control)
      || !sanitized.starts_with('/')
      || sanitized.starts_with("//")
      || uri.scheme().is_some()
      || uri.authority().is_some()
      || uri.query().is_some()
      || uri.path() != sanitized
      || !Self::has_valid_percent_encoding(&sanitized)
    {
      return Cow::Borrowed(INVALID_INSTANCE);
    }

    Cow::Owned(sanitized)
  }

  fn has_valid_percent_encoding(value: &str) -> bool {
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
      if byte == b'%'
        && !matches!(
          (bytes.next(), bytes.next()),
          (Some(high), Some(low)) if high.is_ascii_hexdigit() && low.is_ascii_hexdigit()
        )
      {
        return false;
      }
    }
    true
  }
}

impl IntoResponse for ProblemDetails {
  fn into_response(self) -> Response {
    // The status is carried in the document as well as the response line, and
    // RFC 9457 section 3.1 requires them to agree. Constructing it from the
    // field rather than alongside it is what keeps them from drifting apart.
    let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, [(header::CONTENT_TYPE, PROBLEM_MEDIA_TYPE)], AxumJson(self)).into_response()
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProblemKind {
  MalformedInput,
  ValidationFailed,
  AuthenticationRequired,
  InsufficientScope { required_scope: RequiredScope },
  NotFound,
  MethodNotAllowed,
  Conflict,
  PayloadTooLarge,
  RateLimited { retry_after_seconds: u64 },
  DatabaseUnavailable,
  ShuttingDown,
  ShutdownCancelled,
  Overloaded,
  Internal,
  DeadlineExceeded,
}

#[derive(Clone, Copy)]
struct ProblemContract {
  kind: &'static str,
  title: &'static str,
  status: u16,
  detail: &'static str,
}

impl ProblemKind {
  const fn contract(self) -> ProblemContract {
    match self {
      Self::MalformedInput => ProblemContract {
        kind: "/problems/malformed-input",
        title: "Bad Request",
        status: 400,
        detail: "The request body could not be read.",
      },
      Self::ValidationFailed => ProblemContract {
        kind: "/problems/validation-failed",
        title: "Validation Failed",
        status: 400,
        detail: "The request violates the API contract.",
      },
      Self::AuthenticationRequired => ProblemContract {
        kind: "/problems/authentication-required",
        title: "Unauthorized",
        status: 401,
        detail: "Authentication is required to access this resource.",
      },
      Self::InsufficientScope { .. } => ProblemContract {
        kind: "/problems/insufficient-scope",
        title: "Forbidden",
        status: 403,
        detail: "The authenticated principal lacks the required scope.",
      },
      Self::NotFound => ProblemContract {
        kind: "/problems/not-found",
        title: "Not Found",
        status: 404,
        detail: "The requested resource was not found.",
      },
      // `Allow` is axum's, computed from the methods actually registered on the
      // matched path; the fallback returns this document and leaves that header
      // alone rather than restating a route's method list here.
      Self::MethodNotAllowed => ProblemContract {
        kind: "/problems/method-not-allowed",
        title: "Method Not Allowed",
        status: 405,
        detail: "The request method is not allowed for this resource.",
      },
      Self::Conflict => ProblemContract {
        kind: "/problems/conflict",
        title: "Conflict",
        status: 409,
        detail: "The request conflicts with the current resource state.",
      },
      // The detail names no limit: publishing the ceiling tells a caller probing
      // for it exactly how much to send, and an operator has it in configuration.
      Self::PayloadTooLarge => ProblemContract {
        kind: "/problems/payload-too-large",
        title: "Payload Too Large",
        status: 413,
        detail: "The request body is larger than this service accepts.",
      },
      Self::RateLimited { .. } => ProblemContract {
        kind: "/problems/rate-limit-exceeded",
        title: "Too Many Requests",
        status: 429,
        detail: "The request rate limit has been exceeded.",
      },
      // Three `503`s follow, distinct because an operator reading one during a
      // rollout must be able to tell a database outage from an orderly shutdown
      // from shed load without correlating logs. Only the third is something a
      // client can fix by retrying.
      //
      // One cause is reported for every dependency failure -- unreachable,
      // saturated, or too slow -- because the distinction is useful to an
      // operator reading logs and useful to nobody on the far side of the socket.
      Self::DatabaseUnavailable => ProblemContract {
        kind: "/problems/database-unavailable",
        title: "Service Unavailable",
        status: 503,
        detail: "The service cannot reach its database.",
      },
      Self::ShuttingDown => ProblemContract {
        kind: "/problems/shutting-down",
        title: "Service Unavailable",
        status: 503,
        detail: "The service is shutting down and is not accepting new work.",
      },
      // Shares `ShuttingDown`'s type because it is the same failure class, which
      // `docs/architecture/http_v1_decisions.md` gives one stable type. Only
      // `detail` separates being refused at the door from being cut off partway.
      Self::ShutdownCancelled => ProblemContract {
        kind: "/problems/shutting-down",
        title: "Service Unavailable",
        status: 503,
        detail: "The service stopped waiting for this request so it could shut down.",
      },
      Self::Overloaded => ProblemContract {
        kind: "/problems/overloaded",
        title: "Service Unavailable",
        status: 503,
        detail: "The service is at capacity and refused this request rather than queueing it.",
      },
      Self::Internal => ProblemContract {
        kind: "/problems/internal-error",
        title: "Internal Server Error",
        status: 500,
        detail: "The service encountered an unexpected error.",
      },
      // `504` rather than `408`: a `408` says the *client* was too slow to send
      // its request, which blames the wrong party for a handler that overran.
      Self::DeadlineExceeded => ProblemContract {
        kind: "/problems/deadline-exceeded",
        title: "Gateway Timeout",
        status: 504,
        detail: "The service did not produce a response within its deadline.",
      },
    }
  }
}

/// A closed transport failure classification with a bounded request-path instance.
///
/// Source error text is deliberately not accepted, so SQL and other internal
/// diagnostics cannot become client-facing detail by accident.
#[derive(Debug)]
pub struct ApiProblem {
  kind: ProblemKind,
  instance: Cow<'static, str>,
}

impl ApiProblem {
  #[must_use]
  pub fn malformed_input(instance: &str) -> Self {
    Self::new(ProblemKind::MalformedInput, instance)
  }

  #[must_use]
  pub fn validation_failed(instance: &str) -> Self {
    Self::new(ProblemKind::ValidationFailed, instance)
  }

  #[must_use]
  pub fn authentication_required(instance: &str) -> Self {
    Self::new(ProblemKind::AuthenticationRequired, instance)
  }

  #[must_use]
  pub fn insufficient_scope(instance: &str, required_scope: RequiredScope) -> Self {
    Self::new(ProblemKind::InsufficientScope { required_scope }, instance)
  }

  #[must_use]
  pub fn not_found(instance: &str) -> Self {
    Self::new(ProblemKind::NotFound, instance)
  }

  #[must_use]
  pub fn method_not_allowed(instance: &str) -> Self {
    Self::new(ProblemKind::MethodNotAllowed, instance)
  }

  #[must_use]
  pub fn conflict(instance: &str) -> Self {
    Self::new(ProblemKind::Conflict, instance)
  }

  #[must_use]
  pub fn payload_too_large(instance: &str) -> Self {
    Self::new(ProblemKind::PayloadTooLarge, instance)
  }

  #[must_use]
  pub fn rate_limited(instance: &str, retry_after_seconds: u64) -> Self {
    Self::new(ProblemKind::RateLimited { retry_after_seconds }, instance)
  }

  #[must_use]
  pub fn database_unavailable(instance: &str) -> Self {
    Self::new(ProblemKind::DatabaseUnavailable, instance)
  }

  #[must_use]
  pub fn shutting_down(instance: &str) -> Self {
    Self::new(ProblemKind::ShuttingDown, instance)
  }

  #[must_use]
  pub fn shutdown_cancelled(instance: &str) -> Self {
    Self::new(ProblemKind::ShutdownCancelled, instance)
  }

  #[must_use]
  pub fn overloaded(instance: &str) -> Self {
    Self::new(ProblemKind::Overloaded, instance)
  }

  #[must_use]
  pub fn internal(instance: &str) -> Self {
    Self::new(ProblemKind::Internal, instance)
  }

  #[must_use]
  pub fn deadline_exceeded(instance: &str) -> Self {
    Self::new(ProblemKind::DeadlineExceeded, instance)
  }

  fn new(kind: ProblemKind, instance: &str) -> Self {
    Self { kind, instance: ProblemDetails::bounded_instance(instance) }
  }

  fn details(self) -> ProblemDetails {
    let contract = self.kind.contract();
    let required_scope = match self.kind {
      ProblemKind::InsufficientScope { required_scope } => Some(required_scope),
      _ => None,
    };
    ProblemDetails {
      kind: contract.kind,
      title: contract.title,
      status: contract.status,
      detail: contract.detail,
      instance: self.instance,
      required_scope,
    }
  }
}

impl IntoResponse for ApiProblem {
  fn into_response(self) -> Response {
    let kind = self.kind;
    let mut response = self.details().into_response();
    match kind {
      // RFC 9110 section 15.5.2 requires the challenge on every `401`. Only the
      // `401`: a `403` has judged the credential and accepted it, and a
      // challenge there would tell the caller to present another one.
      ProblemKind::AuthenticationRequired => {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, BEARER_CHALLENGE);
      }
      ProblemKind::RateLimited { retry_after_seconds } => {
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
      }
      _ => {}
    }
    response
  }
}

/// JSON extraction that converts Axum rejections into the shared problem contract.
///
/// A wrapper rather than `axum::Json` because axum answers its own rejections
/// with a `text/plain` body, which `docs/architecture/http_v1_decisions.md`
/// forbids for any API-originated `4xx`. It also answers a shape mismatch with
/// `422`; the accepted decision assigns validation failures `400`, so that is
/// normalized here rather than at each future call site.
#[derive(Debug)]
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
  S: Send + Sync,
  T: DeserializeOwned,
{
  type Rejection = ApiProblem;

  async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
    let instance = request.uri().path().to_owned();
    AxumJson::<T>::from_request(request, state).await.map(|AxumJson(value)| Self(value)).map_err(
      |rejection| match rejection {
        JsonRejection::JsonDataError(_) => ApiProblem::validation_failed(&instance),
        JsonRejection::BytesRejection(rejection)
          if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE =>
        {
          ApiProblem::payload_too_large(&instance)
        }
        JsonRejection::JsonSyntaxError(_)
        | JsonRejection::MissingJsonContentType(_)
        | JsonRejection::BytesRejection(_) => ApiProblem::malformed_input(&instance),
        // `JsonRejection` is `#[non_exhaustive]`, so an axum release can add a
        // variant this match has never seen. The class is recorded and the
        // rejection's own text is still discarded.
        _ => {
          tracing::warn!(class = "unknown_json_rejection", "request extraction failed");
          ApiProblem::malformed_input(&instance)
        }
      },
    )
  }
}

/// Query extraction that converts Axum rejections into the shared problem
/// contract, for the same reason [`ApiJson`] exists.
///
/// The rejection's message is discarded rather than reported: it quotes the
/// offending query value, and a query string is where a caller's credential
/// most often ends up by mistake.
#[derive(Debug)]
pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
  S: Send + Sync,
  T: DeserializeOwned,
{
  type Rejection = ApiProblem;

  async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
    let instance = parts.uri.path().to_owned();
    AxumQuery::<T>::from_request_parts(parts, state)
      .await
      .map(|AxumQuery(value)| Self(value))
      .map_err(|rejection| {
        if !matches!(rejection, QueryRejection::FailedToDeserializeQueryString(_)) {
          tracing::warn!(class = "unknown_query_rejection", "request extraction failed");
        }
        ApiProblem::validation_failed(&instance)
      })
  }
}

/// The refusals the perimeter can answer *any* route with.
///
/// Declared once and referenced from every `#[utoipa::path]` rather than
/// repeated per route: these four statuses come from middleware that wraps the
/// whole router, so a route-by-route copy would let one route's contract drift
/// from what the perimeter actually does.
pub(crate) struct PerimeterResponses;

impl IntoResponses for PerimeterResponses {
  fn responses() -> BTreeMap<String, RefOr<utoipa::openapi::Response>> {
    /// Kept in step with the constructors above; each entry is one of them.
    const REFUSALS: [(&str, &str); 4] = [
      ("400", "The request body could not be read"),
      ("413", "The request body is larger than the perimeter accepts"),
      ("503", "The service is at capacity, or is shutting down"),
      ("504", "The handler did not answer within the perimeter deadline"),
    ];

    REFUSALS
      .into_iter()
      .fold(ResponsesBuilder::new(), |responses, (status, description)| {
        responses.response(status, problem_openapi_response(description, None))
      })
      .build()
      .into()
  }
}

fn problem_openapi_response(
  description: &str,
  example: Option<serde_json::Value>,
) -> utoipa::openapi::Response {
  problem_openapi_response_with_schema(
    description,
    Ref::from_schema_name("ProblemDetails"),
    example,
  )
}

fn problem_openapi_response_with_schema(
  description: &str,
  schema: impl Into<RefOr<Schema>>,
  example: Option<serde_json::Value>,
) -> utoipa::openapi::Response {
  ResponseBuilder::new()
    .description(description)
    .content(
      PROBLEM_MEDIA_TYPE,
      ContentBuilder::new().schema(Some(schema)).example(example).build(),
    )
    .header("X-Request-Id", string_header(REQUEST_ID_DESCRIPTION))
    .build()
}

fn string_header(description: &str) -> utoipa::openapi::Header {
  HeaderBuilder::new()
    .schema(ObjectBuilder::new().schema_type(utoipa::openapi::schema::Type::String))
    .description(Some(description))
    .build()
}

macro_rules! problem_response {
  ($name:ident, $description:literal, $kind:expr) => {
    pub(crate) struct $name;

    impl<'response> ToResponse<'response> for $name {
      fn response() -> (&'response str, RefOr<utoipa::openapi::Response>) {
        let example = serde_json::to_value(ApiProblem::new($kind, "/example").details()).ok();
        (stringify!($name), problem_openapi_response($description, example).into())
      }
    }
  };
}

problem_response!(
  MalformedInputProblem,
  "The request body could not be read",
  ProblemKind::MalformedInput
);
problem_response!(
  ValidationFailedProblem,
  "The request violates the API contract",
  ProblemKind::ValidationFailed
);
problem_response!(NotFoundProblem, "The requested resource was not found", ProblemKind::NotFound);
problem_response!(
  ConflictProblem,
  "The request conflicts with current resource state",
  ProblemKind::Conflict
);
problem_response!(
  PayloadTooLargeProblem,
  "The request body is too large",
  ProblemKind::PayloadTooLarge
);
problem_response!(
  DatabaseUnavailableProblem,
  "The database is unavailable",
  ProblemKind::DatabaseUnavailable
);
problem_response!(ShuttingDownProblem, "The service is shutting down", ProblemKind::ShuttingDown);
problem_response!(OverloadedProblem, "The service is at capacity", ProblemKind::Overloaded);
problem_response!(
  InternalErrorProblem,
  "The service encountered an unexpected error",
  ProblemKind::Internal
);
problem_response!(
  DeadlineExceededProblem,
  "The request deadline expired",
  ProblemKind::DeadlineExceeded
);

pub(crate) struct AuthenticationRequiredProblem;

impl<'response> ToResponse<'response> for AuthenticationRequiredProblem {
  fn response() -> (&'response str, RefOr<utoipa::openapi::Response>) {
    let example = serde_json::to_value(
      ApiProblem::new(ProblemKind::AuthenticationRequired, "/example").details(),
    )
    .ok();
    let mut response = problem_openapi_response("Authentication is required", example);
    response.headers.insert(
      "WWW-Authenticate".to_owned(),
      string_header("The `Bearer` challenge, with no parameters."),
    );
    ("AuthenticationRequiredProblem", response.into())
  }
}

/// Narrows the shared schema rather than reusing it: `required_scope` is
/// optional on `ProblemDetails` because every other problem omits it, and this
/// one never does, so a generated client reads it as present.
pub(crate) struct InsufficientScopeProblem;

impl<'response> ToResponse<'response> for InsufficientScopeProblem {
  fn response() -> (&'response str, RefOr<utoipa::openapi::Response>) {
    let example = serde_json::to_value(
      ApiProblem::new(
        ProblemKind::InsufficientScope { required_scope: RequiredScope::CatalogRead },
        "/example",
      )
      .details(),
    )
    .ok();
    let schema = AllOfBuilder::new()
      .item(Ref::from_schema_name("ProblemDetails"))
      .item(ObjectBuilder::new().required("required_scope"));
    let response = problem_openapi_response_with_schema(
      "The credential lacks the required scope",
      schema,
      example,
    );
    ("InsufficientScopeProblem", response.into())
  }
}

pub(crate) struct MethodNotAllowedProblem;

impl<'response> ToResponse<'response> for MethodNotAllowedProblem {
  fn response() -> (&'response str, RefOr<utoipa::openapi::Response>) {
    let example =
      serde_json::to_value(ApiProblem::new(ProblemKind::MethodNotAllowed, "/example").details())
        .ok();
    let mut response = problem_openapi_response("The request method is not allowed", example);
    response
      .headers
      .insert("Allow".to_owned(), string_header("Methods supported by the requested resource."));
    ("MethodNotAllowedProblem", response.into())
  }
}

pub(crate) struct RateLimitedProblem;

impl<'response> ToResponse<'response> for RateLimitedProblem {
  fn response() -> (&'response str, RefOr<utoipa::openapi::Response>) {
    let example = serde_json::to_value(
      ApiProblem::new(ProblemKind::RateLimited { retry_after_seconds: 60 }, "/example").details(),
    )
    .ok();
    let mut response = problem_openapi_response("The request rate limit was exceeded", example);
    response
      .headers
      .insert("Retry-After".to_owned(), string_header("Seconds until the caller should retry."));
    ("RateLimitedProblem", response.into())
  }
}

/// The one description every correlated response gives for `X-Request-Id`.
const REQUEST_ID_DESCRIPTION: &str =
  "The correlation identifier for this request, echoed from the client or generated here.";

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use aircraft_app::authentication::Scope;
  use axum::{http::header, response::IntoResponse as _};
  use serde_json::json;

  use super::{ApiProblem, ProblemKind, RequiredScope};

  /// The scope spelling an authorization problem publishes.
  ///
  /// Exhaustive on purpose: a scope added to `RequiredScope` stops this test
  /// compiling until its wire spelling is written here, and the spelling is a
  /// literal read against `database/seeds/004_authentication_seed_data.sql`
  /// rather than re-derived through the same `rename_all` the code under test
  /// uses.
  const fn published_scope(scope: RequiredScope) -> &'static str {
    match scope {
      RequiredScope::CatalogRead => "CATALOG_READ",
      RequiredScope::MilitaryRead => "MILITARY_READ",
      RequiredScope::CurationRead => "CURATION_READ",
      RequiredScope::CurationWrite => "CURATION_WRITE",
      RequiredScope::Admin => "ADMIN",
    }
  }

  /// The published wire form of every problem this service can emit.
  ///
  /// One table so the contract is read in one place rather than reassembled
  /// from the behavioural tests, which assert only the status, type, and
  /// instance that tell one refusal from another. The expected values are
  /// literals a reviewer checks against
  /// `docs/architecture/http_v1_decisions.md`, not values re-derived from
  /// [`ProblemKind::contract`] -- a copy of the code under test would pass for
  /// any contract, including a wrong one.
  ///
  /// The `match` is exhaustive with no `_` arm, so a new `ProblemKind` stops
  /// this test compiling until somebody writes the document it publishes.
  /// That is the half a hand-kept array cannot do: an array stays green while
  /// the new variant goes unpinned.
  // One arm per variant is what makes the match exhaustive, and splitting the
  // table across functions would undo the "read in one place" the doc above
  // describes. The length is the contract's, not the function's.
  #[allow(clippy::too_many_lines)]
  fn published_document(kind: ProblemKind) -> serde_json::Value {
    match kind {
      ProblemKind::MalformedInput => json!({
        "type": "/problems/malformed-input",
        "title": "Bad Request",
        "status": 400,
        "detail": "The request body could not be read.",
        "instance": INSTANCE,
      }),
      ProblemKind::ValidationFailed => json!({
        "type": "/problems/validation-failed",
        "title": "Validation Failed",
        "status": 400,
        "detail": "The request violates the API contract.",
        "instance": INSTANCE,
      }),
      ProblemKind::AuthenticationRequired => json!({
        "type": "/problems/authentication-required",
        "title": "Unauthorized",
        "status": 401,
        "detail": "Authentication is required to access this resource.",
        "instance": INSTANCE,
      }),
      ProblemKind::InsufficientScope { required_scope } => json!({
        "type": "/problems/insufficient-scope",
        "title": "Forbidden",
        "status": 403,
        "detail": "The authenticated principal lacks the required scope.",
        "instance": INSTANCE,
        "required_scope": published_scope(required_scope),
      }),
      ProblemKind::NotFound => json!({
        "type": "/problems/not-found",
        "title": "Not Found",
        "status": 404,
        "detail": "The requested resource was not found.",
        "instance": INSTANCE,
      }),
      ProblemKind::MethodNotAllowed => json!({
        "type": "/problems/method-not-allowed",
        "title": "Method Not Allowed",
        "status": 405,
        "detail": "The request method is not allowed for this resource.",
        "instance": INSTANCE,
      }),
      ProblemKind::Conflict => json!({
        "type": "/problems/conflict",
        "title": "Conflict",
        "status": 409,
        "detail": "The request conflicts with the current resource state.",
        "instance": INSTANCE,
      }),
      ProblemKind::PayloadTooLarge => json!({
        "type": "/problems/payload-too-large",
        "title": "Payload Too Large",
        "status": 413,
        "detail": "The request body is larger than this service accepts.",
        "instance": INSTANCE,
      }),
      ProblemKind::RateLimited { .. } => json!({
        "type": "/problems/rate-limit-exceeded",
        "title": "Too Many Requests",
        "status": 429,
        "detail": "The request rate limit has been exceeded.",
        "instance": INSTANCE,
      }),
      ProblemKind::DatabaseUnavailable => json!({
        "type": "/problems/database-unavailable",
        "title": "Service Unavailable",
        "status": 503,
        "detail": "The service cannot reach its database.",
        "instance": INSTANCE,
      }),
      ProblemKind::ShuttingDown => json!({
        "type": "/problems/shutting-down",
        "title": "Service Unavailable",
        "status": 503,
        "detail": "The service is shutting down and is not accepting new work.",
        "instance": INSTANCE,
      }),
      ProblemKind::ShutdownCancelled => json!({
        "type": "/problems/shutting-down",
        "title": "Service Unavailable",
        "status": 503,
        "detail": "The service stopped waiting for this request so it could shut down.",
        "instance": INSTANCE,
      }),
      ProblemKind::Overloaded => json!({
        "type": "/problems/overloaded",
        "title": "Service Unavailable",
        "status": 503,
        "detail": "The service is at capacity and refused this request rather than queueing it.",
        "instance": INSTANCE,
      }),
      ProblemKind::Internal => json!({
        "type": "/problems/internal-error",
        "title": "Internal Server Error",
        "status": 500,
        "detail": "The service encountered an unexpected error.",
        "instance": INSTANCE,
      }),
      ProblemKind::DeadlineExceeded => json!({
        "type": "/problems/deadline-exceeded",
        "title": "Gateway Timeout",
        "status": 504,
        "detail": "The service did not produce a response within its deadline.",
        "instance": INSTANCE,
      }),
    }
  }

  /// The path every case below refuses; it stands for whatever a caller sent,
  /// since `instance` is supplied by the constructor's caller.
  const INSTANCE: &str = "/refused";

  /// Every constructor publishes the document its kind is contracted to.
  ///
  /// The cases go through the public constructors rather than
  /// `ApiProblem::new`, so a constructor wired to the wrong kind fails here
  /// too; [`published_document`] is what stops a new kind from being added
  /// without a document.
  #[test]
  fn each_problem_serializes_to_its_published_document() {
    let cases = [
      ApiProblem::malformed_input(INSTANCE),
      ApiProblem::validation_failed(INSTANCE),
      ApiProblem::authentication_required(INSTANCE),
      ApiProblem::insufficient_scope(INSTANCE, RequiredScope::CatalogRead),
      ApiProblem::insufficient_scope(INSTANCE, RequiredScope::MilitaryRead),
      ApiProblem::insufficient_scope(INSTANCE, RequiredScope::CurationRead),
      ApiProblem::insufficient_scope(INSTANCE, RequiredScope::CurationWrite),
      ApiProblem::insufficient_scope(INSTANCE, RequiredScope::Admin),
      ApiProblem::not_found(INSTANCE),
      ApiProblem::method_not_allowed(INSTANCE),
      ApiProblem::conflict(INSTANCE),
      ApiProblem::payload_too_large(INSTANCE),
      ApiProblem::rate_limited(INSTANCE, 60),
      ApiProblem::database_unavailable(INSTANCE),
      ApiProblem::shutting_down(INSTANCE),
      ApiProblem::shutdown_cancelled(INSTANCE),
      ApiProblem::overloaded(INSTANCE),
      ApiProblem::internal(INSTANCE),
      ApiProblem::deadline_exceeded(INSTANCE),
    ];

    for problem in cases {
      let kind = problem.kind;
      let expected = published_document(kind);
      let serialized =
        serde_json::to_value(problem.details()).expect("a problem document must serialize");

      assert_eq!(serialized, expected, "the published document changed: {kind:?}");
    }
  }

  #[test]
  fn rate_limit_responses_carry_retry_after() {
    let response = ApiProblem::rate_limited("/catalog", 37).into_response();

    assert_eq!(response.headers().get(header::RETRY_AFTER), Some(&"37".parse().expect("valid")));
  }

  /// The challenge is the renderer's, so no path that answers a `401` can
  /// leave it off; the `403` half is the anti-vacuity guard, because a
  /// renderer that challenged on every problem would pass the first assertion.
  #[test]
  fn only_authentication_problems_carry_the_bare_bearer_challenge() {
    let unauthenticated = ApiProblem::authentication_required("/catalog").into_response();
    let forbidden =
      ApiProblem::insufficient_scope("/catalog", RequiredScope::Admin).into_response();

    assert_eq!(
      unauthenticated.headers().get(header::WWW_AUTHENTICATE),
      Some(&"Bearer".parse().expect("valid"))
    );
    assert!(
      forbidden.headers().get(header::WWW_AUTHENTICATE).is_none(),
      "a 403 has judged the credential and must not challenge"
    );
  }

  /// One row per application scope, read against the seed literals through
  /// [`published_scope`] and the wire form, not through either enum's own
  /// spelling. Two arms swapped in the mapping fail here.
  #[test]
  fn every_application_scope_maps_to_the_scope_the_problem_publishes() {
    const SCOPES: [(Scope, &str); 5] = [
      (Scope::CatalogRead, "CATALOG_READ"),
      (Scope::MilitaryRead, "MILITARY_READ"),
      (Scope::CurationRead, "CURATION_READ"),
      (Scope::CurationWrite, "CURATION_WRITE"),
      (Scope::Admin, "ADMIN"),
    ];

    for (scope, code) in SCOPES {
      let published = RequiredScope::from(scope);

      assert_eq!(published_scope(published), code, "{scope:?}");
      assert_eq!(serde_json::to_value(published).expect("serializes"), json!(code), "{scope:?}");
    }
  }

  #[test]
  fn an_invalid_percent_encoding_is_not_reflected_as_an_instance() {
    let problem = ApiProblem::not_found("/invalid%").details();

    assert_eq!(problem.instance, "/invalid-request-path");
  }

  #[test]
  fn only_an_origin_relative_path_can_be_reflected_as_an_instance() {
    for unsafe_instance in [
      "postgres://curator:hunter2@db.internal/aircraft",
      "/catalog?authorization=Bearer%20ak1_secret",
      "relative/path",
    ] {
      let problem = ApiProblem::not_found(unsafe_instance).details();

      assert_eq!(
        problem.instance, "/invalid-request-path",
        "unsafe instance was reflected: {unsafe_instance}",
      );
    }
  }
}
