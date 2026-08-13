use std::{collections::HashSet, fmt, fs::File};

use aircraft_app::ingestion::{
    IssueSeverity, PreflightSummary, PreparedAircraftRecord, PreparedInputError,
    PreparedRecordReader, SourceDescriptor,
};
use async_trait::async_trait;
use serde::{
    Deserializer,
    de::{DeserializeSeed, MapAccess, Visitor},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{artifact::InputArtifact, normalization::normalize_record};

pub trait SourceAdapter: Send + Sync {
    fn descriptor(&self) -> SourceDescriptor;
    fn preflight(&self, artifact: &InputArtifact) -> Result<PreflightSummary, SourceError>;
    fn open_records(
        &self,
        artifact: &InputArtifact,
    ) -> Result<Box<dyn PreparedRecordReader>, SourceError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    PlanePhd,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlanePhdAdapter;

impl PlanePhdAdapter {
    pub const PARSER_NAME: &'static str = "planephd-json";
    pub const PARSER_VERSION: &'static str = "1.0.0";
}

impl SourceAdapter for PlanePhdAdapter {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            slug: "planephd".to_owned(),
            name: "PlanePHD".to_owned(),
            base_url: Some("https://planephd.com".to_owned()),
            parser_name: Self::PARSER_NAME.to_owned(),
            parser_version: Self::PARSER_VERSION.to_owned(),
        }
    }

    fn preflight(&self, artifact: &InputArtifact) -> Result<PreflightSummary, SourceError> {
        let file = artifact.reopen()?;
        let mut record_count = 0_u64;
        let mut warning_count = 0_u64;
        let mut errors = Vec::new();
        let mut source_keys = HashSet::new();
        let mut record_keys = Sha256::new();

        parse_records(file, |record| {
            record_count += 1;
            warning_count += record
                .issues
                .iter()
                .filter(|issue| issue.severity == IssueSeverity::Warning)
                .count() as u64;
            let remaining = 100_usize.saturating_sub(errors.len());
            errors.extend(
                record
                    .issues
                    .iter()
                    .filter(|issue| issue.severity == IssueSeverity::Error)
                    .take(remaining)
                    .cloned(),
            );
            if !source_keys.insert(record.source_record_key.clone()) && errors.len() < 100 {
                errors.push(aircraft_app::ingestion::IngestIssue {
                    code: "DUPLICATE_SOURCE_RECORD_KEY".to_owned(),
                    severity: IssueSeverity::Error,
                    field_path: "$".to_owned(),
                    message: format!("duplicate source record key {}", record.source_record_key),
                    raw_value: None,
                });
            }
            record_keys.update(record.source_record_key.as_bytes());
            record_keys.update(b"\0");
            Ok(())
        })?;

        if record_count == 0 {
            return Err(SourceError::EmptyDocument);
        }
        if !errors.is_empty() {
            return Err(SourceError::Validation { issues: errors });
        }
        Ok(PreflightSummary {
            record_count,
            warning_count,
            record_keys_sha256: hex_digest(&record_keys.finalize()),
        })
    }

    fn open_records(
        &self,
        artifact: &InputArtifact,
    ) -> Result<Box<dyn PreparedRecordReader>, SourceError> {
        let file = artifact.reopen()?;
        let (sender, receiver) = mpsc::channel(16);
        std::thread::Builder::new()
            .name("planephd-parser".to_owned())
            .spawn(move || {
                let parsing = parse_records(file, |record| {
                    sender.blocking_send(Ok(record)).map_err(|_| SourceError::ConsumerClosed)
                });
                if let Err(error) = parsing {
                    let _ = sender.blocking_send(Err(PreparedInputError::Invalid {
                        code: error.code().to_owned(),
                        message: error.to_string(),
                    }));
                }
            })
            .map_err(SourceError::ParserThread)?;
        Ok(Box::new(PlanePhdRecordReader { receiver }))
    }
}

struct PlanePhdRecordReader {
    receiver: mpsc::Receiver<Result<PreparedAircraftRecord, PreparedInputError>>,
}

#[async_trait]
impl PreparedRecordReader for PlanePhdRecordReader {
    async fn next_record(&mut self) -> Result<Option<PreparedAircraftRecord>, PreparedInputError> {
        self.receiver.recv().await.transpose()
    }
}

fn parse_records(
    file: File,
    callback: impl FnMut(PreparedAircraftRecord) -> Result<(), SourceError>,
) -> Result<(), SourceError> {
    let mut deserializer = serde_json::Deserializer::from_reader(file);
    PlanePhdSeed { callback }
        .deserialize(&mut deserializer)
        .map_err(|error| SourceError::Json(error.to_string()))?;
    deserializer.end().map_err(|error| SourceError::Json(error.to_string()))
}

struct PlanePhdSeed<F> {
    callback: F,
}

impl<'de, F> DeserializeSeed<'de> for PlanePhdSeed<F>
where
    F: FnMut(PreparedAircraftRecord) -> Result<(), SourceError>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(PlanePhdVisitor { callback: self.callback })
    }
}

struct PlanePhdVisitor<F> {
    callback: F,
}

impl<'de, F> Visitor<'de> for PlanePhdVisitor<F>
where
    F: FnMut(PreparedAircraftRecord) -> Result<(), SourceError>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a PlanePHD manufacturer-to-aircraft JSON object")
    }

    fn visit_map<M>(mut self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        while let Some(manufacturer) = map.next_key::<String>()? {
            map.next_value_seed(AircraftMapSeed { manufacturer, callback: &mut self.callback })?;
        }
        Ok(())
    }
}

struct AircraftMapSeed<'a, F> {
    manufacturer: String,
    callback: &'a mut F,
}

impl<'de, F> DeserializeSeed<'de> for AircraftMapSeed<'_, F>
where
    F: FnMut(PreparedAircraftRecord) -> Result<(), SourceError>,
{
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(AircraftMapVisitor {
            manufacturer: self.manufacturer,
            callback: self.callback,
        })
    }
}

struct AircraftMapVisitor<'a, F> {
    manufacturer: String,
    callback: &'a mut F,
}

impl<'de, F> Visitor<'de> for AircraftMapVisitor<'_, F>
where
    F: FnMut(PreparedAircraftRecord) -> Result<(), SourceError>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an aircraft-name-to-record JSON object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        while let Some((aircraft_name, raw_record)) = map.next_entry::<String, Value>()? {
            (self.callback)(normalize_record(&self.manufacturer, &aircraft_name, raw_record))
                .map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}
fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[derive(Debug, Error)]
pub enum SourceError {
    #[error(transparent)]
    Artifact(#[from] crate::artifact::ArtifactError),
    #[error("invalid PlanePHD JSON: {0}")]
    Json(String),
    #[error("PlanePHD document contains no aircraft records")]
    EmptyDocument,
    #[error("PlanePHD validation failed")]
    Validation { issues: Vec<aircraft_app::ingestion::IngestIssue> },
    #[error("could not start parser thread: {0}")]
    ParserThread(std::io::Error),
    #[error("record consumer closed")]
    ConsumerClosed,
}

impl SourceError {
    #[must_use]
    pub const fn code(&self) -> &str {
        match self {
            Self::Artifact(_) => "INPUT_ARTIFACT_ERROR",
            Self::Json(_) => "INVALID_JSON",
            Self::EmptyDocument => "EMPTY_DOCUMENT",
            Self::Validation { .. } => "VALIDATION_FAILED",
            Self::ParserThread(_) => "PARSER_THREAD_ERROR",
            Self::ConsumerClosed => "RECORD_CONSUMER_CLOSED",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use crate::artifact::InputArtifact;

    use super::*;

    #[test]
    fn preflight_streams_two_level_planephd_documents() {
        let json = br#"{
            "CESSNA": {
                "172S": {"performance": {"best_cruise_speed": "124 KIAS"}},
                "182T": {"weights": {"gross_weight": "3100 LBS"}}
            }
        }"#;
        let artifact = InputArtifact::capture(&json[..], "fixture.json", None, 1024)
            .expect("fixture should be captured");
        let report = PlanePhdAdapter.preflight(&artifact).expect("fixture should validate");
        assert_eq!(report.record_count, 2);
        assert_eq!(report.warning_count, 0);
    }
}
