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
}

/// The `PostgreSQL` connection the server's runtime role uses.
///
/// Loaded on demand rather than as a field of [`Settings`] so that starting the
/// server needs no database configuration until a story opens a pool. The URL
/// carries a password, so it is a [`SecretString`] and must never reach a log,
/// a `Debug` rendering, or a diagnostic.
#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseSettings {
  pub url: SecretString,
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
}
