use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

use aircraft_app::ingestion::ArtifactDescriptor;
use chrono::Utc;
use sha2::{Digest, Sha256};
use tempfile::{Builder, NamedTempFile};
use thiserror::Error;

pub const DEFAULT_MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub struct InputArtifact {
    descriptor: ArtifactDescriptor,
    file: NamedTempFile,
}

impl InputArtifact {
    pub fn capture_path(
        path: &Path,
        temp_dir: Option<&Path>,
        max_bytes: u64,
    ) -> Result<Self, ArtifactError> {
        let display_locator = path
            .file_name()
            .map_or_else(|| "<file>".to_owned(), |name| sanitize_locator(&name.to_string_lossy()));
        let input = File::open(path)
            .map_err(|source| ArtifactError::Open { locator: display_locator.clone(), source })?;
        Self::capture(input, &display_locator, temp_dir, max_bytes)
    }

    pub fn capture_stdin(temp_dir: Option<&Path>, max_bytes: u64) -> Result<Self, ArtifactError> {
        Self::capture(io::stdin().lock(), "<stdin>", temp_dir, max_bytes)
    }

    pub fn capture<R: Read>(
        mut input: R,
        display_locator: &str,
        temp_dir: Option<&Path>,
        max_bytes: u64,
    ) -> Result<Self, ArtifactError> {
        let mut file = temp_dir
            .map_or_else(
                || Builder::new().prefix("aircraft-ingest-").tempfile(),
                |directory| Builder::new().prefix("aircraft-ingest-").tempfile_in(directory),
            )
            .map_err(ArtifactError::TemporaryFile)?;

        let mut hash = Sha256::new();
        let mut byte_length = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();

        loop {
            let read = input.read(&mut buffer).map_err(ArtifactError::Read)?;
            if read == 0 {
                break;
            }
            byte_length = byte_length
                .checked_add(read as u64)
                .ok_or(ArtifactError::InputTooLarge { max_bytes })?;
            if byte_length > max_bytes {
                return Err(ArtifactError::InputTooLarge { max_bytes });
            }
            hash.update(&buffer[..read]);
            file.write_all(&buffer[..read]).map_err(ArtifactError::TemporaryFile)?;
        }
        file.flush().map_err(ArtifactError::TemporaryFile)?;

        let digest = hash.finalize();
        let content_sha256 = hex_digest(&digest);
        Ok(Self {
            descriptor: ArtifactDescriptor {
                content_sha256,
                byte_length,
                display_locator: sanitize_locator(display_locator),
                captured_at: Utc::now(),
            },
            file,
        })
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ArtifactDescriptor {
        &self.descriptor
    }

    pub fn reopen(&self) -> Result<File, ArtifactError> {
        self.file.reopen().map_err(ArtifactError::TemporaryFile)
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn sanitize_locator(locator: &str) -> String {
    locator.chars().filter(|character| !character.is_control()).take(255).collect()
}

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("could not open input {locator}: {source}")]
    Open { locator: String, source: io::Error },
    #[error("could not read input: {0}")]
    Read(io::Error),
    #[error("could not create or write the secure temporary artifact: {0}")]
    TemporaryFile(io::Error),
    #[error("input exceeds configured limit of {max_bytes} bytes")]
    InputTooLarge { max_bytes: u64 },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    #[test]
    fn captures_exact_hash_and_enforces_limit() {
        let artifact = InputArtifact::capture(&b"abc"[..], "fixture.json", None, 3);
        let artifact = artifact.expect("three-byte fixture should fit");
        assert_eq!(
            artifact.descriptor().content_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(artifact.descriptor().byte_length, 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode =
                artifact.file.as_file().metadata().expect("artifact metadata").permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }

        assert!(matches!(
            InputArtifact::capture(&b"abcd"[..], "fixture.json", None, 3),
            Err(ArtifactError::InputTooLarge { max_bytes: 3 })
        ));
    }
}
