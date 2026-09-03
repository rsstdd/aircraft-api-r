//! Credential issuance: minting the API credential a caller will later present.
//!
//! One issuance makes two independent reads of the operating-system CSPRNG:
//! 16 bytes that become the version-4 `key_id`, the public lookup handle a
//! verifier resolves with a single primary-key probe, and 32 bytes that are the
//! secret, exactly 256 bits counted on their own. The prefix, separators, UUID
//! text, and hex expansion around them add length, not entropy.
//!
//! The clear credential is `ak1_<key_id>_<64 lowercase hex>`, returned once
//! and never stored. What the store receives is SHA-256 over the exact UTF-8
//! bytes of that complete token, as 64 lowercase hex characters. Hashing the
//! whole token rather than the secret alone is what `database/data_dictionary.md`
//! specifies for `secret_digest`, and embedding `key_id` in the hashed text is
//! what lets verification stay one lookup plus one constant-time comparison.
//!
//! Persistence is a single row in `aircraft_auth.api_credentials`. The store
//! owns the transaction and commits only once the returned row has decoded, so
//! a failure at any point hands back no credential and leaves no row.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString, zeroize::Zeroizing};
use sha2::{Digest, Sha256};
use subtle::{Choice, ConstantTimeEq};
use thiserror::Error;
use uuid::{Builder, Uuid};

use super::ingestion::{PersistenceError, hex_digest};

/// Token format version. A different layout gets a different prefix so a
/// verifier can reject an unknown one before touching the database.
pub(crate) const TOKEN_PREFIX: &str = "ak1";
const KEY_ID_BYTES: usize = 16;
/// The whole of AC1: 32 bytes is 256 bits of secret material.
pub(crate) const SECRET_BYTES: usize = 32;
/// `TOKEN_PREFIX` + `_` + hyphenated UUID + `_` + hex secret.
pub(crate) const TOKEN_CHARS: usize = TOKEN_PREFIX.len() + 1 + 36 + 1 + SECRET_BYTES * 2;

/// Mirrors `chk_apc_label` in migration 025.
///
/// The migration is immutable, so the other side of the mirror is the `label`
/// row of `database/data_dictionary.md`, which names this constant.
pub const MAX_LABEL_CHARS: usize = 200;

/// SQLSTATE codes as `aircraft_db` reports them, prefixed by
/// `database_error` in `ingestion_repository.rs`.
const FOREIGN_KEY_VIOLATION: &str = "DATABASE_23503";
const UNIQUE_VIOLATION: &str = "DATABASE_23505";

/// Shortest run of hexadecimal digits treated as credential material: half a
/// SHA-256 digest. A hyphenated key identifier's longest run is twelve.
const REDACTED_HEX_RUN: usize = 32;
const REDACTED: &str = "[REDACTED]";

/// A validated request to issue one credential.
///
/// Both rules mirror migration 025: `principal_id` is a positive identity
/// column, and `chk_apc_label` requires a non-whitespace character and at most
/// 200 characters. The database still enforces them; validating here keeps a
/// rejected request from ever drawing entropy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueCredential {
  principal_id: i64,
  label: String,
}

impl IssueCredential {
  pub fn new(principal_id: i64, label: String) -> Result<Self, CredentialInputError> {
    if principal_id <= 0 {
      return Err(CredentialInputError::NonPositivePrincipalId);
    }
    if !label.chars().any(|character| !character.is_whitespace()) {
      return Err(CredentialInputError::BlankLabel);
    }
    if label.chars().count() > MAX_LABEL_CHARS {
      return Err(CredentialInputError::LabelTooLong);
    }
    Ok(Self { principal_id, label })
  }

  #[must_use]
  pub const fn principal_id(&self) -> i64 {
    self.principal_id
  }

  #[must_use]
  pub fn label(&self) -> &str {
    &self.label
  }
}

/// Input rejections. None carries the offending value, so an error can be
/// logged or returned without echoing what a caller sent.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CredentialInputError {
  #[error("principal id must be positive")]
  NonPositivePrincipalId,
  #[error("label must contain a non-whitespace character")]
  BlankLabel,
  #[error("label must be at most {MAX_LABEL_CHARS} characters")]
  LabelTooLong,
}

impl CredentialInputError {
  #[must_use]
  pub const fn code(&self) -> &'static str {
    match self {
      Self::NonPositivePrincipalId => "NON_POSITIVE_PRINCIPAL_ID",
      Self::BlankLabel => "BLANK_LABEL",
      Self::LabelTooLong => "LABEL_TOO_LONG",
    }
  }
}

/// The clear credential, handed to the caller exactly once.
///
/// Containment, not scrubbing, is what this type promises: it has no `Display`,
/// no `Clone`, no `Serialize`, and a `Debug` that prints nothing of the value.
/// The heap is wiped when the value drops, but the intermediate `String` the
/// token was formatted into is not guaranteed to have been.
pub struct ClearCredential(SecretString);

impl ExposeSecret<str> for ClearCredential {
  fn expose_secret(&self) -> &str {
    self.0.expose_secret()
  }
}

impl fmt::Debug for ClearCredential {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str("ClearCredential([REDACTED])")
  }
}

/// SHA-256 of the complete clear token: the only credential material persisted.
///
/// `uq_apc_secret_digest` enforces that two rows never share this value. Since
/// the hashed text embeds `key_id`, that guarantees distinct *identifiers*; two
/// credentials with the same 32 secret bytes under different identifiers would
/// still hash apart. Distinct *secrets* rest on the CSPRNG, not the constraint.
#[derive(Clone)]
pub struct CredentialVerifier([u8; 32]);

impl ConstantTimeEq for CredentialVerifier {
  fn ct_eq(&self, other: &Self) -> Choice {
    self.0.ct_eq(&other.0)
  }
}

/// `==` is the constant-time comparison, so it is the one a verifier may use
/// on the digest of a presented token. A derived `PartialEq` would stop at the
/// first differing byte and time the stored digest out one byte at a time.
impl PartialEq for CredentialVerifier {
  fn eq(&self, other: &Self) -> bool {
    self.ct_eq(other).into()
  }
}

impl Eq for CredentialVerifier {}

impl CredentialVerifier {
  /// Wraps a digest computed elsewhere.
  #[must_use]
  pub const fn from_digest(digest: [u8; 32]) -> Self {
    Self(digest)
  }

  /// SHA-256 over the exact UTF-8 bytes of a complete clear token.
  ///
  /// The one hash computation for both sides of a credential: issuance stores
  /// its result, and `authentication` hashes a presented token with it. A
  /// verifier that hashed the secret segment alone would never match a stored
  /// digest, so the two cannot be allowed to drift.
  #[must_use]
  pub fn of_token(token: &str) -> Self {
    Self(Sha256::digest(token.as_bytes()).into())
  }

  /// The 64 lowercase hexadecimal characters `chk_apc_secret_digest` admits.
  #[must_use]
  pub fn hex(&self) -> String {
    hex_digest(&self.0)
  }
}

impl fmt::Debug for CredentialVerifier {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str("CredentialVerifier([REDACTED])")
  }
}

/// What crosses the port. The clear token has no field here, so it has no
/// route to SQL, a bound parameter, or a database diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCredential {
  pub key_id: Uuid,
  pub principal_id: i64,
  pub verifier: CredentialVerifier,
  pub label: String,
}

/// The stored, non-secret view of a credential; timestamps are the database's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialRecord {
  pub key_id: Uuid,
  pub principal_id: i64,
  pub label: String,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

/// The one-time issuance result. Not `Clone`, so the clear credential cannot
/// fan out past the caller that received it.
#[derive(Debug)]
pub struct IssuedCredential {
  pub record: CredentialRecord,
  pub clear: ClearCredential,
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
  /// Persists one credential row and returns it as stored. Either the row
  /// exists with every column or nothing was written.
  async fn persist(&self, credential: NewCredential) -> Result<CredentialRecord, PersistenceError>;
}

/// Failures after input validation; a rejected request never reaches `issue`.
#[derive(Debug, Error)]
pub enum CredentialIssuanceError {
  #[error("the operating system random source is unavailable: {0}")]
  EntropyUnavailable(String),
  #[error("principal {0} does not exist")]
  UnknownPrincipal(i64),
  /// A fresh `key_id` or digest collided with a stored one. At 122 and 256
  /// random bits that is a broken random source, not bad luck, so there is no
  /// retry.
  #[error("a credential with this identity already exists")]
  DuplicateCredential,
  /// Built only by the service, which has scrubbed the message: a store may
  /// echo the verifier it was handed, and the port cannot forbid that.
  #[error(transparent)]
  Persistence(PersistenceError),
}

impl CredentialIssuanceError {
  #[must_use]
  pub fn code(&self) -> &str {
    match self {
      Self::EntropyUnavailable(_) => "ENTROPY_UNAVAILABLE",
      Self::UnknownPrincipal(_) => "UNKNOWN_PRINCIPAL",
      Self::DuplicateCredential => "DUPLICATE_CREDENTIAL",
      Self::Persistence(error) => error.code(),
    }
  }
}

/// Fills a buffer with random bytes, in the shape of `getrandom::fill`.
pub type EntropySource = fn(&mut [u8]) -> Result<(), getrandom::Error>;

pub struct CredentialIssuanceService {
  store: Arc<dyn CredentialStore>,
  entropy: EntropySource,
}

impl fmt::Debug for CredentialIssuanceService {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.debug_struct("CredentialIssuanceService").finish_non_exhaustive()
  }
}

impl CredentialIssuanceService {
  #[must_use]
  pub fn new(store: Arc<dyn CredentialStore>) -> Self {
    Self::with_entropy(store, getrandom::fill)
  }

  /// A service drawing from `entropy` in place of the operating-system CSPRNG.
  /// It exists so a test can make the random source fail; composition roots
  /// use [`Self::new`], and nothing here would notice a weak source.
  #[must_use]
  pub fn with_entropy(store: Arc<dyn CredentialStore>, entropy: EntropySource) -> Self {
    Self { store, entropy }
  }

  pub async fn issue(
    &self,
    request: IssueCredential,
  ) -> Result<IssuedCredential, CredentialIssuanceError> {
    let mut key = Zeroizing::new([0_u8; KEY_ID_BYTES]);
    let mut secret = Zeroizing::new([0_u8; SECRET_BYTES]);
    for buffer in [key.as_mut_slice(), secret.as_mut_slice()] {
      (self.entropy)(buffer).map_err(|error| {
        refuse(CredentialIssuanceError::EntropyUnavailable(error.to_string()), request.principal_id)
      })?;
    }
    let material = CredentialMaterial::build(&key, &secret);

    let record = self
      .store
      .persist(NewCredential {
        key_id: material.key_id,
        principal_id: request.principal_id,
        verifier: material.verifier,
        label: request.label,
      })
      .await
      .map_err(|error| refuse(classify(error, request.principal_id), request.principal_id))?;

    tracing::info!(key_id = %record.key_id, principal_id = record.principal_id, "credential issued");
    Ok(IssuedCredential { record, clear: material.clear })
  }
}

/// One event for every refused issuance, whether the random source or the
/// store refused. The code is a constant; the message, even scrubbed, stays
/// out of the trace so nothing an adapter formats can reach it.
fn refuse(error: CredentialIssuanceError, principal_id: i64) -> CredentialIssuanceError {
  tracing::warn!(principal_id, code = error.code(), "credential issuance failed");
  error
}

struct CredentialMaterial {
  key_id: Uuid,
  clear: ClearCredential,
  verifier: CredentialVerifier,
}

impl CredentialMaterial {
  /// Deterministic given its input, so a golden vector can pin the exact token
  /// layout and hash input; only `issue` decides where the bytes come from.
  fn build(key: &[u8; KEY_ID_BYTES], secret: &[u8; SECRET_BYTES]) -> Self {
    let key_id = Builder::from_random_bytes(*key).into_uuid();

    // Exact capacity: `SecretString` boxes the `String` in place only when
    // capacity equals length, so no reallocation leaves a stray copy behind.
    let mut token = String::with_capacity(TOKEN_CHARS);
    token.push_str(TOKEN_PREFIX);
    token.push('_');
    token.push_str(&key_id.hyphenated().to_string());
    token.push('_');
    token.push_str(&Zeroizing::new(hex_digest(secret)));

    let verifier = CredentialVerifier::of_token(&token);
    Self { key_id, clear: ClearCredential(SecretString::from(token)), verifier }
  }
}

/// Reads the SQLSTATE `aircraft_db` folds into the error code, so the two
/// failures a caller can act on keep their identity, and scrubs anything else
/// before it can be returned.
fn classify(error: PersistenceError, principal_id: i64) -> CredentialIssuanceError {
  match error {
    PersistenceError::Database { code, .. } if code == FOREIGN_KEY_VIOLATION => {
      CredentialIssuanceError::UnknownPrincipal(principal_id)
    }
    PersistenceError::Database { code, .. } if code == UNIQUE_VIOLATION => {
      CredentialIssuanceError::DuplicateCredential
    }
    other => CredentialIssuanceError::Persistence(redact_persistence_error(other)),
  }
}

/// [`redact_digest_runs`] applied to whichever message a persistence failure
/// carries, for the two services that accept one from a port.
pub(crate) fn redact_persistence_error(error: PersistenceError) -> PersistenceError {
  match error {
    PersistenceError::Database { code, message } => {
      PersistenceError::Database { code, message: redact_digest_runs(&message) }
    }
    PersistenceError::Invariant(message) => {
      PersistenceError::Invariant(redact_digest_runs(&message))
    }
  }
}

/// Replaces every run of 32 or more hexadecimal digits, the
/// shape of a digest or a raw secret, and leaves everything else intact.
///
/// Applied at two boundaries with one implementation: the service applies it
/// to whatever a [`CredentialStore`] returns, and `aircraft_db`'s credential
/// adapter applies it before its errors leave the adapter, so the adapter is
/// also safe to call on its own.
#[must_use]
pub fn redact_digest_runs(message: &str) -> String {
  let mut output = String::with_capacity(message.len());
  let mut run = String::new();
  let flush = |run: &mut String, output: &mut String| {
    if run.len() >= REDACTED_HEX_RUN {
      output.push_str(REDACTED);
    } else {
      output.push_str(run);
    }
    run.clear();
  };

  for character in message.chars() {
    if character.is_ascii_hexdigit() {
      run.push(character);
    } else {
      flush(&mut run, &mut output);
      output.push(character);
    }
  }
  flush(&mut run, &mut output);
  output
}

#[cfg(test)]
mod tests {
  #![allow(clippy::expect_used)]

  use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
  };

  use async_trait::async_trait;
  use chrono::Utc;
  use secrecy::{ExposeSecret as _, zeroize::ZeroizeOnDrop};
  use sha2::Sha256;
  use subtle::ConstantTimeEq;

  use super::{
    CredentialInputError, CredentialIssuanceError, CredentialIssuanceService, CredentialMaterial,
    CredentialRecord, CredentialStore, CredentialVerifier, IssueCredential, KEY_ID_BYTES,
    MAX_LABEL_CHARS, NewCredential, PersistenceError, REDACTED, SECRET_BYTES, TOKEN_CHARS,
    redact_digest_runs,
  };

  /// Published test vector, so a diff of it discloses nothing.
  const SECRET_HEX: &str = "101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f";
  /// `printf '<token>' | sha256sum` over the token below; not recomputed here.
  const TOKEN_SHA256: &str = "9fdc9e0584edf8647a3677f4e45dd77c303caf819eace32829d3a1ae5e21b4b5";

  /// Key bytes 0x00..0x0f and secret bytes 0x10..0x2f, so each is recognizable
  /// in the token.
  const DIGEST: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
  const KEY_ID: &str = "0b2f4c6e-8a1d-4e3f-9b5c-7d9e1f3a5b7c";

  /// A run one short of the threshold is what a key identifier or short code
  /// looks like and must survive; the threshold and everything longer is a
  /// digest shape and must not.
  #[test]
  fn digest_length_hex_runs_are_redacted_but_identifiers_survive() {
    let cases = [
      ("64-digit digest", format!("Key (secret_digest)=({DIGEST}) already exists."), false),
      ("32-digit run", format!("half {}", &DIGEST[..32]), false),
      ("31-digit run", format!("short {}", &DIGEST[..31]), true),
      ("key identifier", format!("row {KEY_ID} is fine"), true),
    ];

    for (case, message, survives) in cases {
      let redacted = redact_digest_runs(&message);
      assert_eq!(redacted == message, survives, "{case}");
      assert_eq!(redacted.contains(REDACTED), !survives, "{case}");
    }
    assert_eq!(
      redact_digest_runs(&format!("Key (secret_digest)=({DIGEST}) already exists.")),
      "Key (secret_digest)=([REDACTED]) already exists.",
      "the surrounding diagnostic must survive intact"
    );
  }

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

  fn fixed_material() -> CredentialMaterial {
    let byte = |index: usize| u8::try_from(index).expect("48 fits in a byte");
    let key: [u8; KEY_ID_BYTES] = std::array::from_fn(byte);
    let secret: [u8; SECRET_BYTES] = std::array::from_fn(|index| byte(KEY_ID_BYTES + index));
    CredentialMaterial::build(&key, &secret)
  }

  /// The key bytes become the UUID with its version and variant bits forced;
  /// all 32 secret bytes appear verbatim as the hex secret.
  #[test]
  fn credential_material_from_fixed_random_bytes_is_a_256_bit_versioned_token() {
    let material = fixed_material();

    let token = material.clear.expose_secret();
    assert_eq!(token, format!("ak1_00010203-0405-4607-8809-0a0b0c0d0e0f_{SECRET_HEX}"));
    assert_eq!(material.key_id.get_version_num(), 4);
    let secret = token.rsplit('_').next().expect("token has a secret segment");
    assert_eq!(secret.len(), 64, "32 secret bytes encode to 64 hex characters");
    assert!(secret.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(material.verifier.hex(), TOKEN_SHA256, "the digest covers the complete token");
  }

  /// `build` reserves exactly `TOKEN_CHARS`. A token of any other length was
  /// reallocated on its way into the `SecretString`, leaving an unwiped copy
  /// of the secret behind, and nothing at the call site would show it.
  #[test]
  fn the_clear_token_fills_its_reserved_capacity_exactly() {
    assert_eq!(fixed_material().clear.expose_secret().len(), TOKEN_CHARS);
  }

  /// The hasher's block buffer holds the token's final partial block after
  /// `Sha256::digest`. Only `sha2`'s `zeroize` feature makes dropping it wipe
  /// that buffer, and only this bound notices the feature going missing.
  #[test]
  fn the_token_hasher_wipes_its_buffer_on_drop() {
    fn wipes_on_drop<T: ZeroizeOnDrop>() {}

    wipes_on_drop::<Sha256>();
  }

  /// Timing is not observable from a test. What is: the comparison is the
  /// `subtle` one, and it still answers exactly for a first-byte and a
  /// last-byte difference, the two cases a short-circuiting compare treats
  /// differently.
  #[test]
  fn verifier_equality_is_constant_time_and_exact() {
    fn compares_in_constant_time<T: ConstantTimeEq>() {}
    compares_in_constant_time::<CredentialVerifier>();

    let digest = fixed_material().verifier.0;
    let mut first_byte = digest;
    first_byte[0] ^= 1;
    let mut last_byte = digest;
    last_byte[31] ^= 1;

    assert_eq!(CredentialVerifier(digest), CredentialVerifier::from_digest(digest));
    assert_ne!(CredentialVerifier(digest), CredentialVerifier(first_byte), "first byte");
    assert_ne!(CredentialVerifier(digest), CredentialVerifier(last_byte), "last byte");
  }

  #[test]
  fn issuance_input_is_rejected_before_any_secret_exists() {
    let cases: [(&str, i64, String, Result<(), CredentialInputError>); 8] = [
      (
        "zero principal",
        0,
        "unmistakable-label".to_owned(),
        Err(CredentialInputError::NonPositivePrincipalId),
      ),
      (
        "negative principal",
        -1,
        "unmistakable-label".to_owned(),
        Err(CredentialInputError::NonPositivePrincipalId),
      ),
      ("empty label", 1, String::new(), Err(CredentialInputError::BlankLabel)),
      ("tab label", 1, "\t".to_owned(), Err(CredentialInputError::BlankLabel)),
      ("newline label", 1, "\n".to_owned(), Err(CredentialInputError::BlankLabel)),
      (
        "201 characters",
        1,
        "x".repeat(MAX_LABEL_CHARS + 1),
        Err(CredentialInputError::LabelTooLong),
      ),
      ("200 characters", 1, "x".repeat(MAX_LABEL_CHARS), Ok(())),
      ("usable", 1, "ci-runner".to_owned(), Ok(())),
    ];

    for (case, principal_id, label, expected) in cases {
      let outcome = IssueCredential::new(principal_id, label.clone()).map(|_| ());
      assert_eq!(outcome, expected, "{case}");
      // An empty label is contained in every string, so it has nothing to echo.
      if let (Err(error), false) = (outcome, label.is_empty()) {
        assert!(!error.to_string().contains(&label), "{case}: the rejection must not echo input");
      }
    }
  }

  /// Answers every `persist` with the configured error and records what it saw.
  struct FailingStore {
    error_code: &'static str,
    persisted: Mutex<Vec<NewCredential>>,
  }

  #[async_trait]
  impl CredentialStore for FailingStore {
    async fn persist(
      &self,
      credential: NewCredential,
    ) -> Result<CredentialRecord, PersistenceError> {
      self.persisted.lock().expect("fake store lock").push(credential);
      Err(PersistenceError::Database {
        code: self.error_code.to_owned(),
        message: "refused".to_owned(),
      })
    }
  }

  #[tokio::test]
  async fn a_store_failure_yields_no_credential() {
    let cases = [
      ("foreign key", "DATABASE_23503", "UNKNOWN_PRINCIPAL"),
      ("unique", "DATABASE_23505", "DUPLICATE_CREDENTIAL"),
      ("other", "DATABASE_57P01", "DATABASE_57P01"),
    ];

    for (case, error_code, expected_code) in cases {
      let store = Arc::new(FailingStore { error_code, persisted: Mutex::new(Vec::new()) });
      let service = CredentialIssuanceService::new(store.clone());
      let request = IssueCredential::new(7, "ci-runner".to_owned()).expect("valid input");

      let error = service.issue(request).await.expect_err("a failing store must not issue");

      assert_eq!(error.code(), expected_code, "{case}");
      assert!(
        matches!(
          (error_code, &error),
          ("DATABASE_23503", CredentialIssuanceError::UnknownPrincipal(7))
            | ("DATABASE_23505", CredentialIssuanceError::DuplicateCredential)
            | ("DATABASE_57P01", CredentialIssuanceError::Persistence(_))
        ),
        "{case}: unexpected mapping {error:?}"
      );
      let persisted = store.persisted.lock().expect("fake store lock").clone();
      assert_eq!(persisted.len(), 1, "{case}: the store was asked exactly once");
      assert_eq!(persisted[0].principal_id, 7);
      assert_eq!(persisted[0].label, "ci-runner");
    }
  }

  #[test]
  fn credential_types_render_no_secret_material() {
    let material = fixed_material();
    let token = material.clear.expose_secret().to_owned();
    let verifier_hex = material.verifier.hex();
    let record = CredentialRecord {
      key_id: material.key_id,
      principal_id: 7,
      label: "ci-runner".to_owned(),
      created_at: Utc::now(),
      updated_at: Utc::now(),
    };
    let new = NewCredential {
      key_id: material.key_id,
      principal_id: 7,
      verifier: material.verifier.clone(),
      label: "ci-runner".to_owned(),
    };
    let issued = super::IssuedCredential { record: record.clone(), clear: material.clear };
    let errors = [
      CredentialIssuanceError::EntropyUnavailable("closed".to_owned()),
      CredentialIssuanceError::UnknownPrincipal(7),
      CredentialIssuanceError::DuplicateCredential,
      CredentialIssuanceError::Persistence(PersistenceError::Invariant("x".to_owned())),
    ];

    assert!(
      format!("{:?}", issued.clear) == "ClearCredential([REDACTED])",
      "ClearCredential must render the fixed redaction"
    );
    assert!(
      format!("{:?}", CredentialVerifier([0; 32])) == "CredentialVerifier([REDACTED])",
      "CredentialVerifier must render the fixed redaction"
    );
    let rendered: Vec<String> = [format!("{new:?}"), format!("{issued:?}")]
      .into_iter()
      .chain(errors.iter().map(|error| format!("{error:?} {error}")))
      .collect();
    for text in &rendered {
      assert_absent(text, &token, "the clear token");
      assert_absent(text, SECRET_HEX, "the secret");
      assert_absent(text, &verifier_hex, "the verifier");
    }
    assert!(rendered[1].contains(&record.key_id.to_string()), "the record itself still renders");
  }
}
