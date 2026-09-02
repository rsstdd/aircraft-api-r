// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used)]

//! Diagnostic gates for credential issuance against a store that misbehaves.
//!
//! The port hands a store the verifier and accepts any `PersistenceError`
//! back, so a store can echo that verifier in its message. These gates prove
//! the service scrubs what it returns and traces nothing an adapter formats.
//!
//! They live in their own binary for the reason
//! `crates/aircraft_api/tests/request_tracing.rs` records: a callsite first
//! reached with no dispatch is disabled for the process.

use std::{
  panic::{AssertUnwindSafe, catch_unwind},
  sync::{Arc, Mutex},
};

use aircraft_app::credential_issuance::{
  CredentialIssuanceError, CredentialIssuanceService, CredentialRecord, CredentialStore,
  IssueCredential, NewCredential,
};
use aircraft_app::ingestion::PersistenceError;
use async_trait::async_trait;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::fmt::MakeWriter;

/// Fails with a fixed sentence: rendering either argument would print the
/// very material the assertion exists to keep out of output.
fn assert_absent(haystack: &str, needle: &str, what: &str) {
  assert!(!haystack.contains(needle), "{what} was disclosed");
}

#[test]
fn a_failing_absence_assertion_discloses_nothing() {
  let failure = catch_unwind(AssertUnwindSafe(|| {
    assert_absent("before NEEDLE after", "NEEDLE", "the needle");
  }))
  .expect_err("the assertion must fail when the needle is present");

  let message = failure
    .downcast_ref::<String>()
    .cloned()
    .or_else(|| failure.downcast_ref::<&str>().map(|text| (*text).to_owned()))
    .expect("panic payload is text");
  assert!(message.contains("the needle was disclosed"), "the fixed sentence must be the message");
  assert!(!message.contains("NEEDLE"), "the panic message must not carry the needle");
  assert!(!message.contains("before"), "the panic message must not carry the haystack");
}

/// Collects formatted `tracing` output so a test can read an event back; the
/// same twenty lines as `crates/aircraft_api/tests/request_tracing.rs`.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
  fn contents(&self) -> String {
    String::from_utf8_lossy(&self.0.lock().expect("the log buffer lock is poisoned")).into_owned()
  }
}

impl std::io::Write for CapturedLogs {
  fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
    self.0.lock().expect("the log buffer lock is poisoned").extend_from_slice(buffer);
    Ok(buffer.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
  type Writer = Self;

  fn make_writer(&'writer self) -> Self::Writer {
    self.clone()
  }
}

#[derive(Clone, Copy)]
enum Echo {
  Invariant,
  Database,
}

/// Refuses every credential with a message that quotes the verifier it was
/// handed, which is the worst a conforming store can do.
struct EchoingStore {
  echo: Echo,
  verifier_hex: Mutex<Option<String>>,
}

#[async_trait]
impl CredentialStore for EchoingStore {
  async fn persist(&self, credential: NewCredential) -> Result<CredentialRecord, PersistenceError> {
    let hex = credential.verifier.hex();
    *self.verifier_hex.lock().expect("fake store lock") = Some(hex.clone());
    Err(match self.echo {
      Echo::Invariant => PersistenceError::Invariant(format!("verifier {hex} rejected")),
      Echo::Database => PersistenceError::Database {
        code: "DATABASE_XX000".to_owned(),
        message: format!("row ({hex}) rejected"),
      },
    })
  }
}

#[tokio::test]
async fn a_verifier_echoed_by_the_store_reaches_neither_the_error_nor_the_trace() {
  for (case, echo, expected_code) in [
    ("invariant", Echo::Invariant, "PERSISTENCE_INVARIANT"),
    ("database", Echo::Database, "DATABASE_XX000"),
  ] {
    let store = Arc::new(EchoingStore { echo, verifier_hex: Mutex::new(None) });
    let service = CredentialIssuanceService::new(store.clone());
    let request = IssueCredential::new(7, "ci-runner".to_owned()).expect("valid input");
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
      .json()
      .flatten_event(true)
      .with_max_level(tracing::Level::TRACE)
      .with_writer(logs.clone())
      .finish();

    let error = service
      .issue(request)
      .with_subscriber(tracing::Dispatch::new(subscriber))
      .await
      .expect_err("an echoing store must not issue");

    let verifier =
      store.verifier_hex.lock().expect("fake store lock").clone().expect("persist ran");
    let rendered = format!("{error} {error:?}");
    let raw = logs.contents();
    assert!(matches!(error, CredentialIssuanceError::Persistence(_)), "{case}: variant");
    assert_eq!(error.code(), expected_code, "{case}: the stable code survives");
    assert!(rendered.contains("rejected"), "{case}: the store's own wording survives");
    assert!(rendered.contains("[REDACTED]"), "{case}: the verifier is replaced, not dropped");
    assert_absent(&rendered, &verifier, "the verifier");
    assert!(raw.contains(expected_code), "{case}: the failure event was not captured");
    assert_absent(&raw, &verifier, "the verifier");
    assert_absent(&raw, "rejected", "the store's message");
  }
}
