//! Bearer credential verification: resolving a presented `ak1` token to the
//! principal it belongs to.
//!
//! The token is the one [`super::credential_issuance`] mints,
//! `ak1_<key_id>_<64 hex>`. Parsing keeps only what verification needs -- the
//! `key_id` that selects the row and the SHA-256 of the complete token -- and
//! drops the clear text at once, so nothing downstream of the parser holds a
//! credential.
//!
//! Every well-formed candidate costs one lookup and one constant-time
//! comparison. An unknown key, a wrong secret, a revoked credential, and a
//! disabled principal all take that same path and end in the same
//! [`AuthenticationError::Rejected`]: a missing row is compared against a fixed
//! dummy digest rather than short-circuited, so no rejection depends on a
//! credential-state early return. Wall-clock latency across a `PostgreSQL` hit
//! and miss is not claimed equal; what is enforced is that this service never
//! consults the stored state before the comparison has run.
//!
//! `aircraft_api::authentication` owns header parsing and the 401 and 503
//! mappings; `aircraft_db` owns the one statement. Nothing here speaks HTTP or
//! SQL, and nothing here emits a trace event: the adapter that maps
//! [`AuthenticationError::Unavailable`] to a response is where the failure
//! class is logged, once, inside the request span.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use subtle::ConstantTimeEq as _;
use thiserror::Error;
use uuid::Uuid;

use super::{
  credential_issuance::{
    CredentialVerifier, SECRET_BYTES, TOKEN_CHARS, TOKEN_PREFIX, redact_persistence_error,
  },
  ingestion::PersistenceError,
};

/// The closed scope vocabulary a principal can hold.
///
/// One variant per `code` row that
/// `database/seeds/004_authentication_seed_data.sql` inserts, spelled by
/// [`Scope::code`]; the seed names this type in turn. Closed rather than a
/// string so a grant the seed does not know -- schema drift, or a hand-inserted
/// row -- fails the row conversion in `aircraft_db` instead of silently
/// authorizing a scope no route policy names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
  CatalogRead,
  MilitaryRead,
  CurationRead,
  CurationWrite,
  Admin,
}

impl Scope {
  const ALL: [Self; 5] =
    [Self::CatalogRead, Self::MilitaryRead, Self::CurationRead, Self::CurationWrite, Self::Admin];

  /// The `aircraft_auth.scopes.code` spelling.
  #[must_use]
  pub const fn code(self) -> &'static str {
    match self {
      Self::CatalogRead => "CATALOG_READ",
      Self::MilitaryRead => "MILITARY_READ",
      Self::CurationRead => "CURATION_READ",
      Self::CurationWrite => "CURATION_WRITE",
      Self::Admin => "ADMIN",
    }
  }
}

/// A scope code outside the seeded vocabulary. Carries no value: the code came
/// from a database row, and a row conversion failure is reported, not echoed.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("scope code is not in the closed vocabulary")]
pub struct UnknownScope;

impl TryFrom<&str> for Scope {
  type Error = UnknownScope;

  fn try_from(code: &str) -> Result<Self, UnknownScope> {
    Self::ALL.into_iter().find(|scope| scope.code() == code).ok_or(UnknownScope)
  }
}

/// A presented token reduced to what verification uses.
///
/// Built only by [`Self::parse`], which hashes the complete token and keeps no
/// copy of it. `Debug` is derived because the only sensitive field is the
/// verifier, whose own `Debug` prints nothing of the value.
#[derive(Clone, Debug)]
pub struct CredentialCandidate {
  key_id: Uuid,
  verifier: CredentialVerifier,
}

impl CredentialCandidate {
  /// Accepts exactly the layout issuance produces: `ak1`, one `_`, a
  /// hyphenated UUID, one `_`, and 64 secret characters.
  ///
  /// The checks are the ones the lookup needs and no more. A token that
  /// differs from the issued one in any other way -- uppercase hex, say --
  /// hashes to a different digest and is refused by the comparison, after the
  /// same single lookup a wrong secret costs.
  ///
  /// # Errors
  ///
  /// [`CredentialSyntaxError::UnexpectedLength`] when the token is not exactly
  /// the issued length, [`CredentialSyntaxError::UnexpectedShape`] when it does
  /// not split into the three segments with the `ak1` prefix and a 64-character
  /// secret, and [`CredentialSyntaxError::MalformedKeyId`] when the middle
  /// segment is not a UUID. No variant carries any part of the input.
  pub fn parse(token: &str) -> Result<Self, CredentialSyntaxError> {
    if token.len() != TOKEN_CHARS {
      return Err(CredentialSyntaxError::UnexpectedLength);
    }
    // Bounded at four: the fourth `next()` exists only to refuse a token with
    // an extra separator, and nothing walks further into the input.
    let mut segments = token.splitn(4, '_');
    let (Some(prefix), Some(key), Some(secret), None) =
      (segments.next(), segments.next(), segments.next(), segments.next())
    else {
      return Err(CredentialSyntaxError::UnexpectedShape);
    };
    if prefix != TOKEN_PREFIX || secret.len() != SECRET_BYTES * 2 {
      return Err(CredentialSyntaxError::UnexpectedShape);
    }
    let key_id = Uuid::try_parse(key).map_err(|_| CredentialSyntaxError::MalformedKeyId)?;

    Ok(Self { key_id, verifier: CredentialVerifier::of_token(token) })
  }
}

/// Why a token could not be reduced to a candidate. Unit variants only, so
/// rendering one can never echo what a caller sent.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CredentialSyntaxError {
  #[error("credential has an unexpected length")]
  UnexpectedLength,
  #[error("credential does not have the ak1 layout")]
  UnexpectedShape,
  #[error("credential key identifier is not a UUID")]
  MalformedKeyId,
}

/// What one lookup returns: every fact verification reads, from one statement.
///
/// Revoked and disabled rows are returned with their flags set rather than
/// filtered out by the adapter, so the service compares the digest on the same
/// path for every state and only then reads the flags.
#[derive(Clone, Debug)]
pub struct CredentialLookupRecord {
  pub verifier: CredentialVerifier,
  pub revoked: bool,
  pub disabled: bool,
  pub principal_id: i64,
  /// Sorted by the adapter, so two lookups of one principal agree.
  pub scopes: Vec<Scope>,
  pub tier: String,
}

#[async_trait]
pub trait CredentialLookup: Send + Sync {
  /// The stored state behind `key_id`, or `None` when no credential has it.
  ///
  /// One bounded statement: the implementation is required to resolve the
  /// credential, its principal, the principal's grants, and the tier in one
  /// round trip, and to filter nothing -- a revoked or disabled row is
  /// returned with its flag set.
  async fn resolve(&self, key_id: Uuid)
  -> Result<Option<CredentialLookupRecord>, PersistenceError>;
}

/// The identity a request carries once its credential has been accepted.
///
/// Fields are private so only [`AuthenticationService`] can mint one: a
/// handler that could construct this type could also forge it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
  principal_id: i64,
  scopes: Vec<Scope>,
  tier: String,
}

impl AuthenticatedPrincipal {
  #[must_use]
  pub const fn principal_id(&self) -> i64 {
    self.principal_id
  }

  #[must_use]
  pub fn scopes(&self) -> &[Scope] {
    &self.scopes
  }

  #[must_use]
  pub fn tier(&self) -> &str {
    &self.tier
  }
}

#[derive(Debug, Error)]
pub enum AuthenticationError {
  /// Unknown key, wrong secret, revoked credential, or disabled principal.
  /// One variant on purpose: the states are not distinguished here so they
  /// cannot be distinguished by anything built on this.
  #[error("the credential was not accepted")]
  Rejected,
  /// The lookup failed, so nothing is known about the credential. Built only
  /// by the service, which has scrubbed the message: an adapter may echo the
  /// digest it read, and the port cannot forbid that.
  #[error(transparent)]
  Unavailable(PersistenceError),
}

impl AuthenticationError {
  #[must_use]
  pub fn code(&self) -> &str {
    match self {
      Self::Rejected => "CREDENTIAL_REJECTED",
      Self::Unavailable(error) => error.code(),
    }
  }
}

pub struct AuthenticationService {
  lookup: Arc<dyn CredentialLookup>,
}

impl fmt::Debug for AuthenticationService {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.debug_struct("AuthenticationService").finish_non_exhaustive()
  }
}

/// Compared against when no row exists, so an unknown key still pays for one
/// digest comparison. Its value is irrelevant: a missing row is rejected
/// whatever the comparison says.
const ABSENT_VERIFIER: CredentialVerifier = CredentialVerifier::from_digest([0; 32]);

impl AuthenticationService {
  #[must_use]
  pub fn new(lookup: Arc<dyn CredentialLookup>) -> Self {
    Self { lookup }
  }

  /// Resolves a candidate to its principal: one lookup, one comparison.
  ///
  /// # Errors
  ///
  /// [`AuthenticationError::Rejected`] for every credential state that is not
  /// a live credential of an enabled principal with a matching digest, and
  /// [`AuthenticationError::Unavailable`] when the lookup itself failed.
  pub async fn authenticate(
    &self,
    candidate: CredentialCandidate,
  ) -> Result<AuthenticatedPrincipal, AuthenticationError> {
    let record = self
      .lookup
      .resolve(candidate.key_id)
      .await
      .map_err(|error| AuthenticationError::Unavailable(redact_persistence_error(error)))?;

    // The comparison runs before any stored state is read, and runs for a
    // missing row too. Reordering this below the flag checks would give a
    // revoked credential a cheaper rejection than a wrong secret.
    let stored = record.as_ref().map_or(&ABSENT_VERIFIER, |record| &record.verifier);
    let digest_matches = bool::from(candidate.verifier.ct_eq(stored));

    match record {
      Some(record) if digest_matches && !record.revoked && !record.disabled => {
        Ok(AuthenticatedPrincipal {
          principal_id: record.principal_id,
          scopes: record.scopes,
          tier: record.tier,
        })
      }
      _ => Err(AuthenticationError::Rejected),
    }
  }
}

#[cfg(test)]
mod tests {
  #![allow(clippy::expect_used, clippy::panic)]

  use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
  };

  use async_trait::async_trait;
  use uuid::Uuid;

  use super::{
    AuthenticatedPrincipal, AuthenticationError, AuthenticationService, CredentialCandidate,
    CredentialLookup, CredentialLookupRecord, CredentialSyntaxError, CredentialVerifier,
    PersistenceError, Scope, UnknownScope,
  };

  /// The token `credential_material_from_fixed_random_bytes_is_a_256_bit_versioned_token`
  /// pins in `credential_issuance`, and the digest that test computed outside
  /// the service; both published vectors, so a diff of them discloses nothing.
  const TOKEN: &str = "ak1_00010203-0405-4607-8809-0a0b0c0d0e0f_\
                       101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f";
  const TOKEN_SHA256: &str = "9fdc9e0584edf8647a3677f4e45dd77c303caf819eace32829d3a1ae5e21b4b5";
  const KEY_ID: &str = "00010203-0405-4607-8809-0a0b0c0d0e0f";
  const SECRET: &str = "101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f";
  const DIGEST: &str = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

  /// Fails with a fixed sentence: rendering either argument would print the
  /// very material the assertion exists to keep out of output.
  fn assert_absent(haystack: &str, needle: &str, what: &str) {
    assert!(!haystack.contains(needle), "{what} was disclosed");
  }

  fn candidate() -> CredentialCandidate {
    CredentialCandidate::parse(TOKEN).expect("the published token parses")
  }

  /// One spelling per variant, read against the seed rather than re-derived
  /// through `code()`; the exhaustive `match` is what makes a new variant a
  /// compile error here rather than an unpinned spelling.
  #[test]
  fn scope_codes_mirror_the_seeded_vocabulary() {
    const fn seeded(scope: Scope) -> &'static str {
      match scope {
        Scope::CatalogRead => "CATALOG_READ",
        Scope::MilitaryRead => "MILITARY_READ",
        Scope::CurationRead => "CURATION_READ",
        Scope::CurationWrite => "CURATION_WRITE",
        Scope::Admin => "ADMIN",
      }
    }

    for scope in Scope::ALL {
      assert_eq!(scope.code(), seeded(scope), "{scope:?}");
      assert_eq!(Scope::try_from(seeded(scope)), Ok(scope), "{scope:?} must round-trip");
    }
    assert_eq!(Scope::try_from("catalog_read"), Err(UnknownScope), "spelling is exact");
    assert_eq!(Scope::try_from("PUBLIC"), Err(UnknownScope), "Public has no scope row");
  }

  /// The grammar table. The accepted row also pins that the verifier is the
  /// digest of the *complete* token, against a vector computed outside the
  /// service; hashing the secret segment alone would fail it.
  #[test]
  fn only_the_issued_token_layout_parses() {
    let mut shorter = TOKEN.to_owned();
    shorter.pop();
    let cases: [(&str, String, Result<(), CredentialSyntaxError>); 8] = [
      ("canonical", TOKEN.to_owned(), Ok(())),
      // Uppercase parses: the digest, not the parser, is what refuses it.
      ("uppercase secret", TOKEN.to_uppercase().replacen("AK1", "ak1", 1), Ok(())),
      ("one character short", shorter, Err(CredentialSyntaxError::UnexpectedLength)),
      ("one character long", format!("{TOKEN}a"), Err(CredentialSyntaxError::UnexpectedLength)),
      (
        "wrong prefix",
        TOKEN.replacen("ak1", "ak2", 1),
        Err(CredentialSyntaxError::UnexpectedShape),
      ),
      (
        "two segments",
        format!("ak1_{}", TOKEN[4..].replace('_', "-")),
        Err(CredentialSyntaxError::UnexpectedShape),
      ),
      (
        "four segments",
        format!("{}_a", &TOKEN[..TOKEN.len() - 2]),
        Err(CredentialSyntaxError::UnexpectedShape),
      ),
      (
        "key not a uuid",
        TOKEN.replacen(KEY_ID, "00010203-0405-4607-8809-0a0b0c0d0e0g", 1),
        Err(CredentialSyntaxError::MalformedKeyId),
      ),
    ];

    for (case, token, expected) in cases {
      let outcome = CredentialCandidate::parse(&token);
      assert_eq!(outcome.as_ref().map(|_| ()).map_err(|error| *error), expected, "{case}");
      if let Err(error) = outcome {
        assert_absent(&format!("{error} {error:?}"), &token[4..40], "the key segment");
        assert_absent(&format!("{error} {error:?}"), SECRET, "the secret");
      }
    }

    let parsed = candidate();
    assert_eq!(parsed.key_id.to_string(), KEY_ID);
    assert_eq!(parsed.verifier.hex(), TOKEN_SHA256, "the digest must cover the complete token");
  }

  /// Answers every `resolve` with the configured outcome and counts the calls.
  struct FakeLookup {
    outcome: Mutex<Result<Option<CredentialLookupRecord>, &'static str>>,
    calls: AtomicUsize,
  }

  impl FakeLookup {
    fn returning(record: Option<CredentialLookupRecord>) -> Arc<Self> {
      Arc::new(Self { outcome: Mutex::new(Ok(record)), calls: AtomicUsize::new(0) })
    }

    fn failing(message: &'static str) -> Arc<Self> {
      Arc::new(Self { outcome: Mutex::new(Err(message)), calls: AtomicUsize::new(0) })
    }
  }

  #[async_trait]
  impl CredentialLookup for FakeLookup {
    async fn resolve(
      &self,
      _key_id: Uuid,
    ) -> Result<Option<CredentialLookupRecord>, PersistenceError> {
      self.calls.fetch_add(1, Ordering::SeqCst);
      match &*self.outcome.lock().expect("fake lookup lock") {
        Ok(record) => Ok(record.clone()),
        Err(message) => Err(PersistenceError::Database {
          code: "DATABASE_57P01".to_owned(),
          message: (*message).to_owned(),
        }),
      }
    }
  }

  fn live_record() -> CredentialLookupRecord {
    CredentialLookupRecord {
      verifier: candidate().verifier,
      revoked: false,
      disabled: false,
      principal_id: 7,
      scopes: vec![Scope::CatalogRead, Scope::CurationRead],
      tier: "TEST_TIER".to_owned(),
    }
  }

  fn with_digest_byte_flipped(index: usize) -> CredentialLookupRecord {
    let mut digest = [0_u8; 32];
    for (position, pair) in candidate().verifier.hex().as_bytes().chunks(2).enumerate() {
      digest[position] =
        u8::from_str_radix(std::str::from_utf8(pair).expect("hex"), 16).expect("hex");
    }
    digest[index] ^= 1;
    CredentialLookupRecord { verifier: CredentialVerifier::from_digest(digest), ..live_record() }
  }

  /// Every rejected state, in one table, against the same candidate. The
  /// first-byte and last-byte secrets are the two cases a short-circuiting
  /// compare treats differently. The lookup count is asserted per row, and the
  /// same `Rejected` is required of every failure, so a distinct variant for
  /// revocation, or a second lookup for scopes, fails here.
  #[tokio::test]
  async fn every_rejected_credential_state_is_the_same_error_after_one_lookup() {
    let cases: [(&str, Option<CredentialLookupRecord>, bool); 6] = [
      ("live credential", Some(live_record()), true),
      ("unknown key", None, false),
      ("wrong secret, first byte", Some(with_digest_byte_flipped(0)), false),
      ("wrong secret, last byte", Some(with_digest_byte_flipped(31)), false),
      (
        "revoked credential",
        Some(CredentialLookupRecord { revoked: true, ..live_record() }),
        false,
      ),
      (
        "disabled principal",
        Some(CredentialLookupRecord { disabled: true, ..live_record() }),
        false,
      ),
    ];

    for (case, record, accepted) in cases {
      let lookup = FakeLookup::returning(record);
      let service = AuthenticationService::new(lookup.clone());

      let outcome = service.authenticate(candidate()).await;

      assert_eq!(lookup.calls.load(Ordering::SeqCst), 1, "{case}: exactly one lookup");
      match (accepted, outcome) {
        (true, Ok(principal)) => {
          assert_eq!(principal.principal_id(), 7, "{case}");
          assert_eq!(principal.scopes(), [Scope::CatalogRead, Scope::CurationRead], "{case}");
          assert_eq!(principal.tier(), "TEST_TIER", "{case}");
        }
        (false, Err(AuthenticationError::Rejected)) => {}
        (_, other) => panic!("{case}: unexpected outcome {other:?}"),
      }
    }
  }

  /// A lookup failure is not a credential rejection, and the message it
  /// carries has been scrubbed by the time it leaves the service.
  #[tokio::test]
  async fn a_lookup_failure_is_unavailable_and_scrubbed_rather_than_rejected() {
    let lookup = FakeLookup::failing(
      "row deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef rejected",
    );

    let error = AuthenticationService::new(lookup)
      .authenticate(candidate())
      .await
      .expect_err("a failing lookup must not authenticate");

    let AuthenticationError::Unavailable(inner) = &error else {
      panic!("a lookup failure must be Unavailable, not {error:?}");
    };
    assert_eq!(inner.code(), "DATABASE_57P01", "the failure class survives");
    let rendered = format!("{error} {error:?}");
    assert!(rendered.contains("rejected"), "the surrounding diagnostic survives");
    assert_absent(&rendered, DIGEST, "a digest-shaped value in the lookup failure");
  }

  #[test]
  fn authentication_types_render_no_credential_material() {
    let principal = AuthenticatedPrincipal {
      principal_id: 7,
      scopes: vec![Scope::Admin],
      tier: "TEST_TIER".to_owned(),
    };
    let rendered = [
      format!("{:?}", candidate()),
      format!("{:?}", live_record()),
      format!("{principal:?}"),
      format!("{0} {0:?}", AuthenticationError::Rejected),
      format!("{0} {0:?}", CredentialSyntaxError::UnexpectedShape),
    ];

    for text in &rendered {
      assert_absent(text, TOKEN, "the clear token");
      assert_absent(text, SECRET, "the secret");
      assert_absent(text, TOKEN_SHA256, "the verifier");
    }
    assert!(rendered[0].contains(KEY_ID), "the candidate still renders its key identifier");
    assert!(rendered[2].contains("TEST_TIER"), "the principal still renders its tier");
  }
}
