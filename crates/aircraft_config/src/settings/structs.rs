use std::path::PathBuf;

use config::{Config, ConfigError, Environment, File};
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Settings {
  pub http: HttpSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HttpSettings {
  pub host: String,
  pub port: u16,
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
    base_config()
      .set_default("http.host", "127.0.0.1")?
      .set_default("http.port", 8080_u16)?
      .build()?
      .try_deserialize()
  }

  #[must_use]
  pub fn bind_address(&self) -> String {
    format!("{}:{}", self.http.host, self.http.port)
  }
}

impl IngestArtifactSettings {
  pub fn load() -> Result<Self, ConfigError> {
    let settings: IngestArtifactRoot = base_config()
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
    let settings: IngestRoot = base_config()
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
struct IngestRoot {
  ingest: IngestSettings,
}

#[derive(Debug, Deserialize)]
struct IngestArtifactRoot {
  ingest: IngestArtifactSettings,
}

fn base_config() -> config::ConfigBuilder<config::builder::DefaultState> {
  Config::builder()
    .add_source(File::with_name("config/defaults").required(false))
    .add_source(File::with_name("config/config").required(false))
    .add_source(Environment::with_prefix("APP").separator("__"))
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
