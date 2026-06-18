use config::{Config, ConfigError, Environment, File};
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

impl Settings {
    pub fn load() -> Result<Self, ConfigError> {
        Config::builder()
            .add_source(File::with_name("config/defaults").required(false))
            .add_source(File::with_name("config/config").required(false))
            .add_source(Environment::with_prefix("APP").separator("__"))
            .build()?
            .try_deserialize()
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.http.host, self.http.port)
    }
}
