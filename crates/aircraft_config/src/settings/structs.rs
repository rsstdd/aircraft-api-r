use std::path::PathBuf;

use config::{Config, ConfigError, Environment, File};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

#[derive(Clone, Debug, Deserialize)]
pub struct Settings {
  pub http: HttpSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HttpSettings {
  pub host: String,
  pub port: u16,
  /// How long shutdown waits for in-flight requests before cancelling them.
  ///
  /// Zero is accepted and means "cancel immediately". It is a coherent
  /// operational choice, and rejecting it here while `aircraft_server::serve`
  /// accepts a zero [`std::time::Duration`] would put two different contracts
  /// on one value.
  pub shutdown_grace_seconds: u64,
  /// The largest request body the perimeter accepts, in bytes.
  pub max_request_body_bytes: usize,
  /// How long a handler may run before the perimeter answers `504`.
  pub request_timeout_seconds: u64,
  /// How many requests may be in flight before the perimeter sheds with `503`.
  pub max_concurrent_requests: usize,
  /// Origins allowed to read a cross-origin response, as a comma-separated list.
  ///
  /// Empty is default-deny: no cross-origin request is allowed until an origin
  /// is named. Entries are validated while settings load and
  /// consumed by `aircraft_api::PerimeterLimits::new`, which turns each into a
  /// header value.
  #[serde(deserialize_with = "comma_separated")]
  pub cors_allowed_origins: Vec<String>,
}

/// The `PostgreSQL` connection and pool bounds the server's runtime role uses.
///
/// Loaded on demand rather than as a field of [`Settings`] so a component that
/// needs no database is not made to configure one. The URL carries a password,
/// so it is a [`SecretString`] and must never reach a log, a `Debug` rendering,
/// or a diagnostic.
///
/// There is deliberately no lock timeout here. The ingestion pool sets one
/// because it takes advisory locks and writes; the server does neither, and a
/// bound with no contended lock to bound is a setting nobody can tune against
/// an observed failure.
#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseSettings {
  pub url: SecretString,
  pub max_connections: u32,
  pub acquire_timeout_seconds: u64,
  pub statement_timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IngestSettings {
  pub database_url: SecretString,
  pub max_input_bytes: u64,
  pub lock_timeout_seconds: u64,
  pub statement_timeout_seconds: u64,
  pub temp_dir: Option<PathBuf>,
  pub max_connections: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IngestArtifactSettings {
  pub max_input_bytes: u64,
  pub temp_dir: Option<PathBuf>,
}

impl Settings {
  /// Loads HTTP settings, defaulting to loopback so a fresh clone can start the
  /// server without a `.env`.
  ///
  /// The default host is deliberately `127.0.0.1` rather than `0.0.0.0`: a
  /// process that binds every interface the moment it is run should be an
  /// explicit deployment choice, not what happens when configuration is absent.
  pub fn load() -> Result<Self, ConfigError> {
    Self::load_from(CONFIG_DIR, app_environment())
  }

  fn load_from(directory: &str, environment: Environment) -> Result<Self, ConfigError> {
    let settings: Self = base_config(directory, environment)
      .set_default("http.host", "127.0.0.1")?
      .set_default("http.port", 8080_u16)?
      // Thirty seconds is the common platform default -- Kubernetes spells it
      // `terminationGracePeriodSeconds` -- so it is a recognisable ceiling
      // rather than a window guaranteed to fit inside one. Left at both
      // defaults the drain is still running, and a flush still pending, when
      // SIGKILL arrives. A deployment must set this strictly below its own
      // termination grace period, which is what `.env.example` says.
      .set_default("http.shutdown_grace_seconds", 30_u64)?
      // One mebibyte is far above any request these routes accept -- every one
      // of them is a bodyless GET -- and far below what a body would have to be
      // to cost real memory. It is the ceiling `.env.example` already named.
      .set_default("http.max_request_body_bytes", 1_048_576_u64)?
      // Thirty seconds clears the 250 ms readiness deadline in
      // `aircraft_db::readiness` by two orders of magnitude, so the timeout ends
      // a genuinely stuck handler rather than a merely slow dependency.
      .set_default("http.request_timeout_seconds", 30_u64)?
      // Well above the database pool's default of ten, so the pool stays the
      // first constraint under normal load and this bound only engages when
      // something is wrong. Shedding below the pool size would refuse work the
      // service could have served.
      .set_default("http.max_concurrent_requests", 256_u64)?
      // Empty is default-deny. It is also why the wildcard rejection below can
      // only ever fire on a value somebody set deliberately.
      .set_default("http.cors_allowed_origins", "")?
      .build()?
      .try_deserialize()?;

    // An empty host is not a harmless blank: `bind_address` renders it as
    // `:8080`, which binds every interface — the outcome the loopback default
    // exists to keep deliberate.
    if settings.http.host.trim().is_empty() {
      return Err(ConfigError::Message("http.host must not be empty".to_owned()));
    }

    // Zero is the one representable port that cannot describe a deployment: it
    // asks the OS for an arbitrary free port, leaving nothing able to reach the
    // process at a known address. Deserializing into `u16` already rejects the
    // negative and above-65535 ends of the range, so this closes the third.
    //
    // Needing an ephemeral port is not a reason to accept it here.
    // `aircraft_server::serve` takes an already-bound listener precisely so a
    // caller that wants one binds it itself and reads the assigned port back.
    if settings.http.port == 0 {
      return Err(ConfigError::Message(
        "http.port must be between 1 and 65535; zero requests an arbitrary OS-assigned port"
          .to_owned(),
      ));
    }

    validate_perimeter_limits(&settings.http)?;

    Ok(settings)
  }

  #[must_use]
  pub fn bind_address(&self) -> String {
    format!("{}:{}", self.http.host, self.http.port)
  }
}

impl DatabaseSettings {
  pub fn load() -> Result<Self, ConfigError> {
    Self::load_from(CONFIG_DIR, app_environment())
  }

  fn load_from(directory: &str, environment: Environment) -> Result<Self, ConfigError> {
    let settings: DatabaseRoot = base_config(directory, environment)
      // Defaulted to empty rather than left absent so an unset value and a
      // blank one fail through the same message naming the full setting path,
      // instead of surfacing serde's "missing field `database`".
      .set_default("database.url", "")?
      // Ten connections is the pool the server can hold without crowding the
      // ingestion role on a shared local PostgreSQL, whose default
      // `max_connections` is 100 and which already grants five to ingestion.
      .set_default("database.max_connections", 10_u32)?
      // Five seconds is longer than a healthy acquire and shorter than an
      // HTTP client's patience, so exhaustion surfaces as a fast failure
      // rather than a request queue growing behind a saturated pool.
      .set_default("database.acquire_timeout_seconds", 5_u64)?
      // Thirty seconds bounds a single statement well above any read this
      // service is designed to serve, so it ends a runaway query without
      // cancelling work that was merely slow.
      .set_default("database.statement_timeout_seconds", 30_u64)?
      .build()?
      .try_deserialize()?;
    validate_database_settings(&settings.database)?;
    Ok(settings.database)
  }
}

impl IngestArtifactSettings {
  pub fn load() -> Result<Self, ConfigError> {
    let settings: IngestArtifactRoot = base_config(CONFIG_DIR, app_environment())
      .set_default("ingest.max_input_bytes", 512_u64 * 1024 * 1024)?
      .build()?
      .try_deserialize()?;
    if settings.ingest.max_input_bytes == 0 {
      return Err(ConfigError::Message(
        "ingest.max_input_bytes must be greater than zero".to_owned(),
      ));
    }
    Ok(settings.ingest)
  }
}

impl IngestSettings {
  pub fn load() -> Result<Self, ConfigError> {
    let settings: IngestRoot = base_config(CONFIG_DIR, app_environment())
      .set_default("ingest.max_input_bytes", 512_u64 * 1024 * 1024)?
      .set_default("ingest.lock_timeout_seconds", 5_u64)?
      .set_default("ingest.statement_timeout_seconds", 1_800_u64)?
      .set_default("ingest.max_connections", 5_u32)?
      .build()?
      .try_deserialize()?;
    validate_ingest_settings(&settings.ingest)?;
    Ok(settings.ingest)
  }
}

#[derive(Debug, Deserialize)]
struct DatabaseRoot {
  database: DatabaseSettings,
}

#[derive(Debug, Deserialize)]
struct IngestRoot {
  ingest: IngestSettings,
}

#[derive(Debug, Deserialize)]
struct IngestArtifactRoot {
  ingest: IngestArtifactSettings,
}

/// The `APP__`-prefixed environment layer, which overrides every source below it.
fn app_environment() -> Environment {
  Environment::with_prefix("APP").separator("__")
}

/// Where the optional override files live, relative to the working directory.
const CONFIG_DIR: &str = "config";

/// Builds the shared source chain: optional override files, then `environment`.
///
/// Source order is the precedence contract, lowest first: built-in defaults,
/// then `defaults`, then `config`, then the environment. `add_source` appends,
/// so reordering these lines changes which source wins.
///
/// Both the directory and the environment are parameters rather than constants
/// so tests can drive the real chain hermetically -- a temporary directory for
/// the file layer, and a map through `Environment::source` for the environment.
/// Mutating the real environment is not an option here: Rust 2024 makes
/// `std::env::set_var` `unsafe`, and the workspace sets `unsafe_code =
/// "forbid"`.
fn base_config(
  directory: &str,
  environment: Environment,
) -> config::ConfigBuilder<config::builder::DefaultState> {
  Config::builder()
    // `config` is built with the `json5` feature only, so these resolve
    // `<directory>/defaults.json5` and `<directory>/config.json5` and no other
    // extension.
    .add_source(File::with_name(&format!("{directory}/defaults")).required(false))
    .add_source(File::with_name(&format!("{directory}/config")).required(false))
    .add_source(environment)
}

/// Reads a comma-separated list from one scalar setting.
///
/// The list arrives as a scalar rather than as a sequence because `config`'s
/// environment source only yields a `Vec` under `try_parsing`, which re-types
/// *every* variable on that source through bool, then integer, then float
/// before any list handling reaches it. `app_environment` is shared by all
/// four loaders in this module, so enabling it to read one field would change
/// how `database.url`, `http.port`, and the ingest bounds are typed. Splitting
/// one string keeps the blast radius to this field.
///
/// A wholly blank value is an empty list, which is how an unset setting spells
/// itself. Interior blanks are *kept* rather than filtered, so
/// `"https://a,,https://b"` is rejected by the perimeter validator instead
/// of quietly losing an entry the operator meant to write.
///
/// An override file supplies the same scalar string. A JSON5 array is
/// deliberately not accepted; one spelling per setting is easier to document
/// than two.
fn comma_separated<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let raw = String::deserialize(deserializer)?;
  if raw.trim().is_empty() {
    return Ok(Vec::new());
  }

  Ok(raw.split(',').map(str::trim).map(str::to_owned).collect())
}

/// The origin spelling that `tower_http::cors::AllowOrigin::list` panics on.
const WILDCARD_ORIGIN: &str = "*";

/// Rejects a perimeter bound the server could not enforce.
///
/// Every branch names its full setting path, the way the host, port, and pool
/// bounds already do, so a misconfigured deployment fails naming the value that
/// is wrong.
fn validate_perimeter_limits(settings: &HttpSettings) -> Result<(), ConfigError> {
  // Each zero is rejected under its own path. A body limit of zero refuses
  // every request, a deadline of zero times out every handler before it runs,
  // and a concurrency limit of zero sheds everything -- three different ways to
  // configure a server that serves nothing.
  if settings.max_request_body_bytes == 0 {
    return Err(ConfigError::Message(
      "http.max_request_body_bytes must be greater than zero".to_owned(),
    ));
  }
  if settings.request_timeout_seconds == 0 {
    return Err(ConfigError::Message(
      "http.request_timeout_seconds must be greater than zero".to_owned(),
    ));
  }
  if settings.max_concurrent_requests == 0 {
    return Err(ConfigError::Message(
      "http.max_concurrent_requests must be greater than zero".to_owned(),
    ));
  }

  settings.cors_allowed_origins.iter().try_for_each(|origin| validate_cors_origin(origin))
}

/// Rejects anything that is not exactly what a browser puts in `Origin`.
///
/// The wildcard rejection is the load-bearing one.
/// `aircraft_api::PerimeterLimits::cors_layer` hands these to
/// `tower_http::cors::AllowOrigin::list`, which **panics** on a wildcard, so
/// this check is what makes that panic unreachable. The two sides name each
/// other deliberately: deleting this one moves a configuration mistake into a
/// startup crash.
///
/// The rest exist because an allowed origin is matched by byte equality against
/// the request's `Origin` header. Anything that is not already in the spelling a
/// browser sends -- a trailing slash, an uppercase host, an explicit default
/// port, a Unicode host that should be punycode -- would be accepted here,
/// stored, and then silently match nothing, leaving an operator with CORS that
/// looks configured and denies every request with no diagnostic anywhere.
///
/// No rejection renders `origin`. A malformed origin may contain userinfo,
/// query credentials, or a fragment token, and startup diagnostics are not a
/// safe place to publish any of them. The canonical spelling is rendered only
/// after those components have been rejected.
fn validate_cors_origin(origin: &str) -> Result<(), ConfigError> {
  // Checked before parsing so the diagnostic names the wildcard rather than
  // reporting `*` as a malformed URL. That message is the operator-facing half
  // of "production configuration rejects wildcard CORS origins".
  if origin == WILDCARD_ORIGIN {
    return Err(ConfigError::Message(
      "http.cors_allowed_origins must not contain the wildcard `*`; name each allowed origin"
        .to_owned(),
    ));
  }
  if origin.is_empty() {
    return Err(ConfigError::Message(
      "http.cors_allowed_origins must not contain a blank entry".to_owned(),
    ));
  }

  let Ok(url) = Url::parse(origin) else {
    return Err(ConfigError::Message(
      "http.cors_allowed_origins contains an entry that is not a valid absolute URL".to_owned(),
    ));
  };
  if url.host_str().is_none_or(str::is_empty) {
    return Err(ConfigError::Message(
      "http.cors_allowed_origins contains an entry that names no host".to_owned(),
    ));
  }
  // An `Origin` header carries a scheme, a host, and an optional port, and
  // nothing else. This runs before the spelling check below so that check can
  // rely on the path being exactly one slash.
  if url.path() != "/"
    || url.query().is_some()
    || url.fragment().is_some()
    || !url.username().is_empty()
    || url.password().is_some()
  {
    return Err(ConfigError::Message(
      "http.cors_allowed_origins entries must contain only a scheme, host, and optional port"
        .to_owned(),
    ));
  }

  // `Url` normalises case, default ports, and Unicode hosts, so parsing and
  // re-rendering yields the spelling a browser would send. The path is `/` by
  // the check above, so stripping that one slash is exact. Comparing the two is
  // what turns four separate silent-mismatch bugs into one message that names
  // the spelling to use.
  let canonical = url.as_str().strip_suffix('/').unwrap_or_else(|| url.as_str());
  if canonical != origin {
    return Err(ConfigError::Message(format!(
      "http.cors_allowed_origins contains an entry that is not the spelling a browser sends; \
       write {canonical:?}"
    )));
  }

  Ok(())
}

/// The URL schemes `SQLx` accepts for `PostgreSQL`.
const POSTGRES_SCHEMES: [&str; 2] = ["postgres", "postgresql"];

/// Rejects a database URL the server could not connect with.
///
/// No branch quotes the value it rejected: the URL embeds a password, and a
/// rejected URL is exactly the one most likely to be pasted into a bug report.
fn validate_database_settings(settings: &DatabaseSettings) -> Result<(), ConfigError> {
  let raw = settings.url.expose_secret();
  if raw.trim().is_empty() {
    return Err(ConfigError::Message(
      "database.url must be set to a PostgreSQL connection URL".to_owned(),
    ));
  }

  // Parsed rather than prefix-matched. `postgres://[invalid` opens with an
  // accepted scheme yet is not a URL at all, and a prefix check would pass it
  // through to fail later inside pool construction -- far from the setting that
  // is actually wrong, and at a point where the diagnostic may carry the
  // credential.
  let Ok(url) = Url::parse(raw) else {
    return Err(ConfigError::Message("database.url is not a valid URL".to_owned()));
  };

  if !POSTGRES_SCHEMES.contains(&url.scheme()) {
    return Err(ConfigError::Message(
      "database.url must use a postgres:// or postgresql:// scheme".to_owned(),
    ));
  }

  // A URL naming neither a host nor a database names nothing to connect to.
  // The path form stays acceptable because `postgres:///aircraft` is the local
  // socket connection `SQLx` supports.
  if url.host_str().is_none_or(str::is_empty) && url.path().trim_matches('/').is_empty() {
    return Err(ConfigError::Message("database.url must name a host or a database".to_owned()));
  }

  // Each bound is rejected at zero under its own setting path. A pool of zero
  // admits nothing, an acquire timeout of zero gives up before waiting, and
  // `PostgreSQL` reads a statement timeout of zero as no timeout at all -- the
  // opposite of the ceiling the setting exists to impose.
  if settings.max_connections == 0 {
    return Err(ConfigError::Message(
      "database.max_connections must be greater than zero".to_owned(),
    ));
  }
  if settings.acquire_timeout_seconds == 0 {
    return Err(ConfigError::Message(
      "database.acquire_timeout_seconds must be greater than zero".to_owned(),
    ));
  }
  if settings.statement_timeout_seconds == 0 {
    return Err(ConfigError::Message(
      "database.statement_timeout_seconds must be greater than zero".to_owned(),
    ));
  }

  Ok(())
}

fn validate_ingest_settings(settings: &IngestSettings) -> Result<(), ConfigError> {
  if settings.max_input_bytes == 0 {
    return Err(ConfigError::Message(
      "ingest.max_input_bytes must be greater than zero".to_owned(),
    ));
  }
  if settings.lock_timeout_seconds == 0 {
    return Err(ConfigError::Message(
      "ingest.lock_timeout_seconds must be greater than zero".to_owned(),
    ));
  }
  if settings.statement_timeout_seconds == 0 {
    return Err(ConfigError::Message(
      "ingest.statement_timeout_seconds must be greater than zero".to_owned(),
    ));
  }
  if settings.max_connections < 2 {
    return Err(ConfigError::Message("ingest.max_connections must be at least two".to_owned()));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test, so panicking accessors are fine.
  #![allow(clippy::expect_used)]

  use std::{collections::HashMap, fs};

  use config::ConfigError;
  use secrecy::ExposeSecret as _;
  use tempfile::TempDir;

  use super::{DatabaseSettings, Environment, POSTGRES_SCHEMES, Settings, app_environment};

  /// A password that must never reach a diagnostic, distinctive enough that a
  /// substring search cannot match it by accident.
  const PASSWORD: &str = "n0t-in-any-diagnostic";

  /// Injects `pairs` in place of the real process environment.
  ///
  /// `Environment::source` routes an explicit map through the same prefix and
  /// separator handling as `env::vars_os`, so these tests exercise the shipped
  /// mapping while staying hermetic under a concurrent test runner.
  fn environment(pairs: &[(&str, &str)]) -> Environment {
    let source: HashMap<String, String> =
      pairs.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect();
    app_environment().source(Some(source))
  }

  /// An override directory guaranteed to be empty, so a test that means to
  /// exercise built-in defaults cannot be steered by a file someone drops into
  /// the crate directory later.
  fn empty_overrides() -> TempDir {
    tempfile::tempdir().expect("a temporary override directory")
  }

  fn directory_path(directory: &TempDir) -> String {
    directory.path().to_string_lossy().into_owned()
  }

  fn load_http(pairs: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let directory = empty_overrides();
    Settings::load_from(&directory_path(&directory), environment(pairs))
  }

  fn load_database(pairs: &[(&str, &str)]) -> Result<DatabaseSettings, ConfigError> {
    let directory = empty_overrides();
    DatabaseSettings::load_from(&directory_path(&directory), environment(pairs))
  }

  #[test]
  fn http_settings_load_without_any_environment_or_override_file() {
    let settings = load_http(&[]).expect("defaults must load with no configuration");

    assert_eq!(settings.bind_address(), "127.0.0.1:8080");
  }

  #[test]
  fn an_app_environment_value_overrides_the_built_in_default() {
    let settings = load_http(&[("APP__HTTP__HOST", "10.0.0.7"), ("APP__HTTP__PORT", "9443")])
      .expect("APP__ values must load");

    assert_eq!(settings.bind_address(), "10.0.0.7:9443");
  }

  /// Pins the whole precedence chain, not just its top. The environment beating
  /// a built-in default says nothing about the two file sources between them:
  /// moving `add_source(environment)` above either `File` would leave that
  /// assertion green. Each step here fails under a different reordering --
  /// `defaults` over built-ins, `config` over `defaults`, environment over
  /// both -- so the ordering cannot be changed silently.
  #[test]
  fn each_override_file_beats_the_source_below_it_and_the_environment_beats_both() {
    let directory = empty_overrides();
    fs::write(
      directory.path().join("defaults.json5"),
      r#"{"http": {"host": "10.0.0.1", "port": 7000}}"#,
    )
    .expect("writing the defaults override");
    fs::write(directory.path().join("config.json5"), r#"{"http": {"port": 7100}}"#)
      .expect("writing the config override");
    let path = directory_path(&directory);

    let from_files = Settings::load_from(&path, environment(&[]))
      .expect("the override files must load without any environment");
    assert_eq!(
      from_files.bind_address(),
      "10.0.0.1:7100",
      "`defaults` must beat the built-in default and `config` must beat `defaults`"
    );

    let overridden = Settings::load_from(&path, environment(&[("APP__HTTP__PORT", "9443")]))
      .expect("an APP__ value must load over the override files");
    assert_eq!(
      overridden.bind_address(),
      "10.0.0.1:9443",
      "the environment must beat every file source"
    );
  }

  #[test]
  fn an_empty_bind_host_is_rejected_with_its_setting_path() {
    for host in ["", "   "] {
      let error = load_http(&[("APP__HTTP__HOST", host)])
        .expect_err("an empty host must not bind every interface");

      assert!(
        error.to_string().contains("http.host"),
        "the failure must name its setting path for host {host:?}: {error}"
      );
    }
  }

  /// Acceptance criterion 4 of the graceful-shutdown story: the default is 30
  /// seconds and the value stays configurable. Zero is asserted as *accepted*
  /// rather than rejected, because the drain window is the one HTTP bound here
  /// whose zero is a decision rather than a mistake.
  #[test]
  fn the_drain_window_defaults_to_thirty_seconds_and_stays_configurable() {
    let default = load_http(&[]).expect("HTTP settings must load without any environment");
    assert_eq!(default.http.shutdown_grace_seconds, 30);

    for (configured, expected) in [("0", 0_u64), ("5", 5), ("120", 120)] {
      let settings = load_http(&[("APP__HTTP__SHUTDOWN_GRACE_SECONDS", configured)])
        .expect("a drain window must load from the environment");

      assert_eq!(
        settings.http.shutdown_grace_seconds, expected,
        "the configured window {configured} did not reach the settings"
      );
    }
  }

  /// The four perimeter defaults, so a fresh clone starts the server bounded
  /// without configuring anything.
  ///
  /// The origin list is asserted empty rather than merely present: an empty
  /// allow-list is default-deny, and a loader that defaulted it to `["*"]` --
  /// the shape most CORS examples show -- would satisfy every other assertion
  /// in this file while opening the service to every origin on the web.
  #[test]
  fn the_perimeter_bounds_load_with_built_in_defaults() {
    let settings = load_http(&[]).expect("the perimeter bounds must load with no configuration");

    assert_eq!(settings.http.max_request_body_bytes, 1_048_576);
    assert_eq!(settings.http.request_timeout_seconds, 30);
    assert_eq!(settings.http.max_concurrent_requests, 256);
    assert!(
      settings.http.cors_allowed_origins.is_empty(),
      "an unset origin list must deny every origin, not allow every origin: {:?}",
      settings.http.cors_allowed_origins
    );
  }

  #[test]
  fn an_app_environment_value_overrides_each_perimeter_bound() {
    let settings = load_http(&[
      ("APP__HTTP__MAX_REQUEST_BODY_BYTES", "2048"),
      ("APP__HTTP__REQUEST_TIMEOUT_SECONDS", "7"),
      ("APP__HTTP__MAX_CONCURRENT_REQUESTS", "9"),
    ])
    .expect("APP__ perimeter bounds must load");

    assert_eq!(settings.http.max_request_body_bytes, 2048);
    assert_eq!(settings.http.request_timeout_seconds, 7);
    assert_eq!(settings.http.max_concurrent_requests, 9);
  }

  /// Each bound is asserted through its own setting path rather than a shared
  /// `is_err`, which would still pass with two of the three checks deleted.
  #[test]
  fn each_perimeter_bound_rejects_zero_naming_its_setting() {
    const CASES: [(&str, &str); 3] = [
      ("APP__HTTP__MAX_REQUEST_BODY_BYTES", "http.max_request_body_bytes"),
      ("APP__HTTP__REQUEST_TIMEOUT_SECONDS", "http.request_timeout_seconds"),
      ("APP__HTTP__MAX_CONCURRENT_REQUESTS", "http.max_concurrent_requests"),
    ];

    for (key, path) in CASES {
      let error =
        load_http(&[(key, "0")]).expect_err("a zero perimeter bound must not reach the router");

      assert!(
        error.to_string().contains(path),
        "the failure must name its setting path for {key}: {error}"
      );
    }
  }

  /// Acceptance criterion 4.
  ///
  /// The wildcard is refused wherever it appears in the list, not merely when
  /// it is the only entry: `AllowOrigin::list` panics on a `*` anywhere among
  /// its origins, so a check that only inspected the first would turn this
  /// configuration mistake into a startup crash.
  #[test]
  fn a_wildcard_cors_origin_is_rejected_with_its_setting_path() {
    for configured in ["*", "https://app.example.com,*", "*,https://app.example.com"] {
      let error = load_http(&[("APP__HTTP__CORS_ALLOWED_ORIGINS", configured)])
        .expect_err("a wildcard origin must not reach the CORS layer");
      let message = error.to_string();

      assert!(
        message.contains("http.cors_allowed_origins"),
        "the failure must name its setting path for {configured:?}: {message}"
      );
      assert!(
        message.contains("wildcard"),
        "the failure must name the wildcard rule rather than fall through to the generic \
         parse error, which quotes the value and would satisfy a laxer assertion, for \
         {configured:?}: {message}"
      );
    }
  }

  /// The accepted side, and the rule that keeps an accepted origin usable.
  ///
  /// The round trip is the load-bearing assertion. `Url::parse` is what
  /// validates these, and `Url::parse("https://app.example.com").to_string()`
  /// yields a trailing slash -- an origin no browser ever sends. A validator
  /// that stored what it parsed instead of what it was given would satisfy
  /// every other assertion here and allowlist two origins that match nothing.
  #[test]
  fn an_origin_survives_configuration_without_gaining_a_trailing_slash() {
    let settings = load_http(&[(
      "APP__HTTP__CORS_ALLOWED_ORIGINS",
      "https://app.example.com, http://localhost:5173",
    )])
    .expect("explicit origins must load");

    assert_eq!(
      settings.http.cors_allowed_origins,
      vec!["https://app.example.com".to_owned(), "http://localhost:5173".to_owned()],
      "each entry must be trimmed and stored exactly as written"
    );
  }

  /// Everything that is not an origin, including the two shapes that parse as
  /// URLs perfectly well and would then match nothing at runtime.
  #[test]
  fn a_value_that_is_not_an_origin_is_rejected_with_its_setting_path() {
    const CASES: [&str; 6] = [
      "https://app.example.com,,http://localhost:5173",
      "app.example.com",
      "https://",
      "https://app.example.com/callback",
      "https://app.example.com?tenant=1",
      "https://curator:hunter2@app.example.com",
    ];

    for configured in CASES {
      let error = load_http(&[("APP__HTTP__CORS_ALLOWED_ORIGINS", configured)])
        .expect_err("only a bare origin may be allowlisted");

      assert!(
        error.to_string().contains("http.cors_allowed_origins"),
        "the failure must name its setting path for {configured:?}: {error}"
      );
    }
  }

  #[test]
  fn rejected_cors_origins_do_not_reach_diagnostics() {
    let cases = [
      format!("https://curator:{PASSWORD}@app.example.com"),
      format!("https://app.example.com?token={PASSWORD}"),
      format!("https://app.example.com#{PASSWORD}"),
    ];

    for configured in cases {
      let error = load_http(&[("APP__HTTP__CORS_ALLOWED_ORIGINS", configured.as_str())])
        .expect_err("an origin carrying a secret must be rejected");
      let message = error.to_string();

      assert!(
        message.contains("http.cors_allowed_origins"),
        "the failure must identify the rejected setting"
      );
      assert!(!message.contains(&configured), "the failure must not echo the rejected origin");
      assert!(!message.contains(PASSWORD), "the failure must not disclose embedded credentials");
    }
  }

  /// An allowed origin is matched against the request's `Origin` header by byte
  /// equality, so a value that is merely *equivalent* to what a browser sends
  /// is not good enough -- it would be accepted, stored, and then match
  /// nothing, leaving CORS that looks configured and denies everything with no
  /// diagnostic anywhere.
  ///
  /// Every case here parses as a perfectly good URL naming the right host, and
  /// every one of them was accepted before this rule existed. The expected
  /// spelling is asserted rather than merely the rejection, because the whole
  /// point is to hand the operator the string to paste.
  #[test]
  fn an_origin_that_is_not_the_spelling_a_browser_sends_is_rejected_with_the_one_to_use() {
    const CASES: [(&str, &str); 5] = [
      ("https://app.example.com/", "https://app.example.com"),
      ("HTTPS://App.Example.COM", "https://app.example.com"),
      ("https://app.example.com:443", "https://app.example.com"),
      ("http://localhost:80", "http://localhost"),
      ("https://caf\u{e9}.example.com", "https://xn--caf-dma.example.com"),
    ];

    for (configured, expected) in CASES {
      let error = load_http(&[("APP__HTTP__CORS_ALLOWED_ORIGINS", configured)])
        .expect_err("an origin no browser would send must not be allowlisted");
      let message = error.to_string();

      assert!(
        message.contains("http.cors_allowed_origins"),
        "the failure must name its setting path for {configured:?}: {message}"
      );
      assert!(
        message.contains(expected),
        "the failure must name the spelling to use ({expected}) for {configured:?}: {message}"
      );
    }
  }

  /// Zero is the only port boundary `u16` cannot enforce for itself, so it is
  /// the only one a refactor can drop without the compiler noticing.
  #[test]
  fn port_zero_is_rejected_with_its_setting_path() {
    let error = load_http(&[("APP__HTTP__PORT", "0")])
      .expect_err("port zero names no address anything could connect to");

    assert!(
      error.to_string().contains("http.port"),
      "the failure must name its setting path: {error}"
    );
  }

  /// The accepted end of the port range. Paired with the rejection test below,
  /// this fixes the boundary exactly at 65535 rather than merely somewhere near
  /// it: a check that stopped one short would fail here, and one that stopped
  /// one late would fail there.
  #[test]
  fn the_lowest_and_highest_assignable_ports_are_accepted() {
    for port in ["1", "65535"] {
      let settings = load_http(&[("APP__HTTP__PORT", port)]).expect("an in-range port must load");

      assert_eq!(settings.bind_address(), format!("127.0.0.1:{port}"));
    }
  }

  /// The rejected end. `u16` does this work, so the test is here to prove the
  /// rejection happens while loading configuration and names `http.port` --
  /// widening the field to `i32` or `u32` to "be lenient" would surface these
  /// as a bind failure deep in startup instead.
  #[test]
  fn a_port_outside_the_representable_range_is_rejected_with_its_setting_path() {
    for port in ["-1", "65536", "4294967296", "8o80"] {
      let error =
        load_http(&[("APP__HTTP__PORT", port)]).expect_err("an unrepresentable port must not load");

      assert!(
        error.to_string().contains("http.port"),
        "the failure must name its setting path for port {port:?}: {error}"
      );
    }
  }

  /// Asserts the *unset* wording, not merely that some error occurred. Both
  /// halves are load-bearing: an unset URL is also caught by the scheme check
  /// below it, so a path-only assertion passes whether or not this case keeps
  /// the diagnostic an operator can act on.
  #[test]
  fn a_missing_or_blank_database_url_fails_with_its_setting_path() {
    for pairs in [&[][..], &[("APP__DATABASE__URL", "")][..], &[("APP__DATABASE__URL", "  ")][..]] {
      let error =
        load_database(pairs).expect_err("the server must not start without a database URL");
      let message = error.to_string();

      assert!(
        message.contains("database.url") && message.contains("must be set"),
        "an unset URL must say so, naming its setting path, for {pairs:?}: {message}"
      );
    }
  }

  /// Every case here opens with an accepted scheme, so a prefix check passes
  /// all of them and fails only when the pool is constructed. The first two are
  /// not URLs at all; the last two parse but name nothing to connect to.
  #[test]
  fn a_malformed_database_url_is_rejected_before_any_connection_attempt() {
    for url in [
      "postgres://[invalid",
      &format!("postgresql://aircraft_app:{PASSWORD}@[::1"),
      "postgres://",
      "postgresql://",
    ] {
      let error = load_database(&[("APP__DATABASE__URL", url)])
        .expect_err("a malformed URL must be rejected while loading configuration");
      let message = error.to_string();

      assert!(
        message.contains("database.url"),
        "the failure must name its setting path: {message}"
      );
      // Deliberately does not render the message: printing it on failure would
      // put the credential in the very output this test exists to keep clean.
      assert!(!message.contains(PASSWORD), "a malformed URL echoed the credential");
    }
  }

  #[test]
  fn a_non_postgres_database_url_is_rejected_without_echoing_the_credential() {
    for scheme in ["mysql", "https"] {
      let url = format!("{scheme}://aircraft_app:{PASSWORD}@localhost:5432/aircraft");
      let error = load_database(&[("APP__DATABASE__URL", &url)])
        .expect_err("only a PostgreSQL URL may be accepted");
      let message = error.to_string();

      assert!(
        message.contains("database.url"),
        "the failure must name its setting path for {scheme}: {message}"
      );
      assert!(!message.contains(PASSWORD), "the {scheme} failure echoed the credential");
    }
  }

  #[test]
  fn an_accepted_database_url_loads_without_revealing_its_credential() {
    for scheme in POSTGRES_SCHEMES {
      let url = format!("{scheme}://aircraft_app:{PASSWORD}@localhost:5432/aircraft");
      let settings =
        load_database(&[("APP__DATABASE__URL", &url)]).expect("a PostgreSQL URL must load");

      // The round trip is the anti-vacuity half: a loader that dropped the URL
      // would satisfy the redaction assertion below without carrying the value.
      assert!(settings.url.expose_secret() == url, "the {scheme} secret must survive loading");
      assert!(
        !format!("{settings:?}").contains(PASSWORD),
        "Debug output revealed the {scheme} credential"
      );
    }
  }

  /// A URL every pool-bound test can reuse, valid enough to reach the bound
  /// checks that follow it.
  fn database_url() -> String {
    format!("postgres://aircraft_api_app:{PASSWORD}@localhost:5432/aircraft")
  }

  /// The bounds carry defaults so a fresh clone starts the server with the URL
  /// as its only required database setting, matching how the ingestion settings
  /// default everything except their URL.
  #[test]
  fn database_pool_bounds_load_with_built_in_defaults() {
    let url = database_url();

    let settings = load_database(&[("APP__DATABASE__URL", url.as_str())])
      .expect("the pool bounds must load with no configuration");

    assert_eq!(settings.max_connections, 10);
    assert_eq!(settings.acquire_timeout_seconds, 5);
    assert_eq!(settings.statement_timeout_seconds, 30);
  }

  #[test]
  fn an_app_environment_value_overrides_each_pool_bound() {
    let url = database_url();

    let settings = load_database(&[
      ("APP__DATABASE__URL", url.as_str()),
      ("APP__DATABASE__MAX_CONNECTIONS", "24"),
      ("APP__DATABASE__ACQUIRE_TIMEOUT_SECONDS", "9"),
      ("APP__DATABASE__STATEMENT_TIMEOUT_SECONDS", "45"),
    ])
    .expect("APP__ pool bounds must load");

    assert_eq!(settings.max_connections, 24);
    assert_eq!(settings.acquire_timeout_seconds, 9);
    assert_eq!(settings.statement_timeout_seconds, 45);
  }

  /// Each bound is asserted through its own setting path rather than a shared
  /// `is_err`, which would still pass with two of the three checks deleted.
  ///
  /// Zero means something different and useless for each: a pool admitting no
  /// connection, an acquire that gives up before waiting, and a statement
  /// timeout that `PostgreSQL` reads as no timeout at all -- the opposite of the
  /// bound being asked for.
  #[test]
  fn a_zero_pool_bound_is_rejected_with_its_setting_path() {
    const CASES: [(&str, &str); 3] = [
      ("APP__DATABASE__MAX_CONNECTIONS", "database.max_connections"),
      ("APP__DATABASE__ACQUIRE_TIMEOUT_SECONDS", "database.acquire_timeout_seconds"),
      ("APP__DATABASE__STATEMENT_TIMEOUT_SECONDS", "database.statement_timeout_seconds"),
    ];
    let url = database_url();

    for (key, path) in CASES {
      let error = load_database(&[("APP__DATABASE__URL", url.as_str()), (key, "0")])
        .expect_err("a zero bound must not reach pool construction");

      assert!(
        error.to_string().contains(path),
        "the failure must name its setting path for {key}: {error}"
      );
    }
  }

  /// The accepted side of the boundary above. Paired with it, this fixes each
  /// rejection at zero rather than somewhere below the default: a check that
  /// demanded two or more would fail here.
  #[test]
  fn the_smallest_usable_pool_bounds_load() {
    let url = database_url();

    let settings = load_database(&[
      ("APP__DATABASE__URL", url.as_str()),
      ("APP__DATABASE__MAX_CONNECTIONS", "1"),
      ("APP__DATABASE__ACQUIRE_TIMEOUT_SECONDS", "1"),
      ("APP__DATABASE__STATEMENT_TIMEOUT_SECONDS", "1"),
    ])
    .expect("the smallest usable bounds must load");

    assert_eq!(settings.max_connections, 1);
    assert_eq!(settings.acquire_timeout_seconds, 1);
    assert_eq!(settings.statement_timeout_seconds, 1);
  }
}
