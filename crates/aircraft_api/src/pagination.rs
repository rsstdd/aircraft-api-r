//! The opaque cursor a collection route hands out, and the query it accepts.
//!
//! `docs/architecture/http_v1_decisions.md` § "Pagination" fixes the contract:
//! the cursor is an opaque base64url value carrying a cursor version, the
//! selected sort, the last sort value, the unique-ID tiebreaker, and a hash of
//! the normalized filters; clients must not construct or interpret it; and
//! malformed cursors, unsupported versions, and cursors reused across a filter
//! change are `400`.
//!
//! Everything here is transport. `aircraft_app::pagination` owns the bound on a
//! page and the shape of one that came back, and knows nothing of base64 or
//! JSON — which is why a repository can build a page without depending on this
//! module.

use std::num::NonZeroU16;

use aircraft_app::pagination::PageLimit;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::problem::ApiProblem;

/// The longest encoded cursor this service will look at.
///
/// Checked before decoding rather than after: a cursor is attacker-controlled
/// input, and a bound applied to the decoded bytes is a bound that has already
/// allocated. Root `AGENTS.md` requires every input bound to be explicit.
pub const MAX_CURSOR_BYTES: usize = 1024;

/// The payload shape this build issues and accepts.
///
/// A durable machine-readable contract in the sense `rust-production` describes:
/// moving it is deliberate, and a token from any other version is refused
/// rather than reinterpreted under the current shape.
const CURSOR_VERSION: u8 = 1;

/// The wire payload, versioned and strict.
///
/// `deny_unknown_fields` makes a mistyped member a refusal instead of a
/// silently ignored one, which is what keeps a version bump the only way this
/// shape can change.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CursorTokenV1 {
  version: u8,
  sort: String,
  last_value: String,
  tiebreaker: String,
  filter: String,
}

/// Reads only the version, tolerating any other shape.
///
/// Deserializing straight into [`CursorTokenV1`] would validate this version's
/// required fields before anything could look at `version`, so a future payload
/// would be reported as malformed rather than as an unsupported version — and
/// the accepted decision distinguishes the two.
#[derive(Debug, Deserialize)]
struct VersionProbe {
  version: u8,
}

/// A digest of an endpoint's normalized filters.
///
/// Guards against a client resuming a scan under filters that have changed
/// since the cursor was issued. It is **not** tamper protection: nothing signs
/// the cursor, so a client can mint one carrying any fingerprint. That is
/// acceptable because the route policy, not the cursor, decides which rows a
/// caller may read.
///
/// The endpoint owns what "normalized" means, and its normalization must be
/// stable: an unstable one silently invalidates every outstanding cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilterFingerprint([u8; 32]);

impl FilterFingerprint {
  #[must_use]
  pub fn of(normalized_filters: &str) -> Self {
    Self(Sha256::digest(normalized_filters.as_bytes()).into())
  }

  fn to_hex(self) -> String {
    self.0.iter().fold(String::with_capacity(64), |mut hex, byte| {
      use std::fmt::Write as _;
      // Writing to a String cannot fail; the result is discarded rather than
      // unwrapped so no formatting error can panic on a request path.
      let _ = write!(hex, "{byte:02x}");
      hex
    })
  }
}

/// Where a scan resumes: the sort it was issued for, the last sort value seen,
/// and the unique tiebreaker that separates rows sharing that value.
///
/// Both [`encode`] and [`decode`] speak this one type, and its members are
/// named rather than positional. Two of them are `String`s that a positional
/// signature would let a caller transpose, which would silently resume the scan
/// at the wrong row.
///
/// `sort` is **compared, never used**: the endpoint matches it against its own
/// allowlist and takes the column name from that allowlist, so no
/// cursor-derived text ever reaches a SQL statement.
///
/// `last_value` and `tiebreaker` are caller-supplied text that this module does
/// not interpret. A cursor decodes successfully carrying `"abc"` where the
/// endpoint's key is a `SMALLINT`, so the endpoint owns that conversion — and a
/// conversion that fails is a `400`, never a panic. Decoding proves the token's
/// shape, version, and filters, and nothing about its values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorPosition {
  pub sort: String,
  pub last_value: String,
  pub tiebreaker: String,
}

/// Why a cursor was refused.
///
/// Unit variants only, so rendering one cannot echo a caller's token into a log
/// or a response — the shape `CredentialSyntaxError` uses in
/// `aircraft_app::services::authentication`.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CursorDecodeError {
  #[error("cursor is not a readable token")]
  Malformed,
  #[error("cursor was issued for an unsupported version")]
  UnsupportedVersion,
  #[error("cursor was issued for different filters")]
  FilterMismatch,
}

/// Renders the cursor a client sends back to resume this scan.
#[must_use]
pub fn encode(position: &CursorPosition, filters: FilterFingerprint) -> String {
  let token = CursorTokenV1 {
    version: CURSOR_VERSION,
    sort: position.sort.clone(),
    last_value: position.last_value.clone(),
    tiebreaker: position.tiebreaker.clone(),
    filter: filters.to_hex(),
  };
  // Serializing four owned `String`s and a `u8` cannot fail. An empty token is
  // refused on the way back in rather than panicking on the way out.
  let json = serde_json::to_vec(&token).unwrap_or_default();
  URL_SAFE_NO_PAD.encode(json)
}

/// # Errors
///
/// [`CursorDecodeError::Malformed`] when the token is over
/// [`MAX_CURSOR_BYTES`], is not base64url, or is not this version's payload;
/// [`CursorDecodeError::UnsupportedVersion`] when it names another version; and
/// [`CursorDecodeError::FilterMismatch`] when it was issued for different
/// filters. The order matters and is fixed: length, base64, version, payload,
/// filters.
pub fn decode(
  token: &str,
  filters: FilterFingerprint,
) -> Result<CursorPosition, CursorDecodeError> {
  if token.len() > MAX_CURSOR_BYTES {
    return Err(CursorDecodeError::Malformed);
  }
  let json = URL_SAFE_NO_PAD.decode(token).map_err(|_| CursorDecodeError::Malformed)?;

  let probe: VersionProbe =
    serde_json::from_slice(&json).map_err(|_| CursorDecodeError::Malformed)?;
  if probe.version != CURSOR_VERSION {
    return Err(CursorDecodeError::UnsupportedVersion);
  }

  let token: CursorTokenV1 =
    serde_json::from_slice(&json).map_err(|_| CursorDecodeError::Malformed)?;
  if token.filter != filters.to_hex() {
    return Err(CursorDecodeError::FilterMismatch);
  }

  Ok(CursorPosition {
    sort: token.sort,
    last_value: token.last_value,
    tiebreaker: token.tiebreaker,
  })
}

/// The query parameters every collection route accepts.
///
/// `limit` is `NonZeroU16` so `limit=0` is refused by the same query validation
/// that already refuses `limit=abc`, rather than being silently replaced by a
/// value this service invented.
///
/// There is deliberately no `sort` member. The accepted decision gives each
/// endpoint its own allowlist of sort orders, so their legal values differ and
/// a shared type could only accept the parameter and ignore it. An endpoint
/// with more than one order declares its own typed member and flattens this
/// struct beside it, then compares that member with [`CursorPosition::sort`].
#[derive(Debug, Deserialize)]
pub struct ListQuery {
  limit: Option<NonZeroU16>,
  cursor: Option<String>,
}

impl ListQuery {
  /// Turns validated query parameters into the page bound and, if one was sent,
  /// the cursor to resume from.
  ///
  /// # Errors
  ///
  /// Every cursor refusal becomes the one published `400` document. The three
  /// causes are deliberately indistinguishable to the caller: naming which one
  /// applies would tell someone probing cursors what to change next, and the
  /// accepted decision assigns all three the same status.
  pub fn into_page_request(
    self,
    filters: FilterFingerprint,
    instance: &str,
  ) -> Result<(PageLimit, Option<CursorPosition>), ApiProblem> {
    let cursor = self
      .cursor
      .as_deref()
      .map(|token| decode(token, filters))
      .transpose()
      .map_err(|_| ApiProblem::validation_failed(instance))?;
    Ok((PageLimit::from_requested(self.limit), cursor))
  }
}

/// One page on the wire.
///
/// `next_cursor` is serialized as `null` on a final page rather than omitted,
/// because the accepted decision says "Empty and final pages return a null next
/// cursor" — the opposite of the omit-when-absent rule the measurement
/// representation follows, and deliberate: a client polling for more pages
/// reads the member rather than testing for its presence.
#[derive(Debug, Serialize)]
pub struct PageResponse<T> {
  pub items: Vec<T>,
  pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use serde_json::json;

  use super::*;

  fn filters(normalized: &str) -> FilterFingerprint {
    FilterFingerprint::of(normalized)
  }

  fn position(sort: &str, last_value: &str, tiebreaker: &str) -> CursorPosition {
    CursorPosition {
      sort: sort.to_owned(),
      last_value: last_value.to_owned(),
      tiebreaker: tiebreaker.to_owned(),
    }
  }

  fn token_of(payload: &serde_json::Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("the test payload serializes"))
  }

  #[test]
  fn a_cursor_survives_the_round_trip_that_issued_it() {
    let fingerprint = filters("category=SPEED");

    let issued = position("code_asc", "20", "KNOTS");

    let decoded = decode(&encode(&issued, fingerprint), fingerprint)
      .expect("a cursor this build issued decodes");

    assert_eq!(decoded, issued, "every member survives the round trip");
  }

  /// The member names outstanding cursors carry, pinned in both directions.
  ///
  /// The round-trip test cannot see a rename: `encode` and `decode` would move
  /// together and cancel out, leaving every cursor a client already holds
  /// undecodable while `CURSOR_VERSION` still said `1`. `rust-production`
  /// treats a format this project defines as a durable contract whose
  /// spellings a test pins, the way `REPORT_SCHEMA_VERSION` is pinned.
  ///
  /// Reading the emitted payload as JSON is the half that catches a rename in
  /// what this build writes; decoding a hand-written token is the half that
  /// catches one in what it will still accept.
  #[test]
  fn the_version_one_payload_carries_the_members_outstanding_cursors_hold() {
    let fingerprint = filters("category=SPEED");
    let issued = position("code_asc", "20", "KNOTS");
    let expected = json!({
      "version": 1,
      "sort": "code_asc",
      "last_value": "20",
      "tiebreaker": "KNOTS",
      "filter": fingerprint.to_hex()
    });

    let emitted =
      URL_SAFE_NO_PAD.decode(encode(&issued, fingerprint)).expect("this build emits base64url");

    assert_eq!(
      serde_json::from_slice::<serde_json::Value>(&emitted).expect("the payload is JSON"),
      expected,
      "the emitted payload's members are the wire contract"
    );
    assert_eq!(
      decode(&token_of(&expected), fingerprint),
      Ok(issued),
      "a token written with those members still decodes"
    );
  }

  #[test]
  fn an_unreadable_cursor_is_refused_without_being_echoed() {
    let fingerprint = filters("");
    // A *valid* token that is merely too long. An invalid-base64 string of the
    // same length would be refused by the base64 step, leaving the byte bound
    // untested: this one decodes cleanly if the bound is removed.
    let oversized =
      encode(&position("code_asc", &"9".repeat(2 * MAX_CURSOR_BYTES), "KNOTS"), fingerprint);
    assert!(oversized.len() > MAX_CURSOR_BYTES, "the oversized case must exceed the bound");
    let base64_nonsense = URL_SAFE_NO_PAD.encode("{");
    let unknown_member = token_of(&json!({
      "version": 1,
      "sort": "a",
      "last_value": "b",
      "tiebreaker": "c",
      "filter": fingerprint.to_hex(),
      "extra": true
    }));
    let missing_member = token_of(&json!({ "version": 1, "sort": "a" }));
    let cases = [
      ("not base64url", "!!!not-base64!!!"),
      ("base64 of nonsense", base64_nonsense.as_str()),
      ("a payload with an unknown member", unknown_member.as_str()),
      ("a payload missing a member", missing_member.as_str()),
      ("over the byte bound", oversized.as_str()),
    ];

    for (case, token) in cases {
      assert_eq!(decode(token, fingerprint), Err(CursorDecodeError::Malformed), "{case}");
    }
    for error in [
      CursorDecodeError::Malformed,
      CursorDecodeError::UnsupportedVersion,
      CursorDecodeError::FilterMismatch,
    ] {
      let rendered = error.to_string();
      assert!(!rendered.contains("base64"), "{error:?} must not describe the input: {rendered}");
      assert!(!rendered.contains('{'), "{error:?} must not echo a payload: {rendered}");
    }
  }

  #[test]
  fn a_cursor_from_another_version_is_refused_as_a_version_not_as_malformed() {
    let fingerprint = filters("");
    // A future payload with a different shape: strict deserialization of this
    // version would call it malformed, which is the failure the probe exists
    // to prevent.
    let future = token_of(&json!({ "version": 2, "keyset": ["20", "KNOTS"] }));

    assert_eq!(decode(&future, fingerprint), Err(CursorDecodeError::UnsupportedVersion));
  }

  #[test]
  fn a_cursor_issued_for_other_filters_is_refused() {
    let issued = encode(&position("code_asc", "20", "KNOTS"), filters("category=SPEED"));

    assert_eq!(decode(&issued, filters("category=WEIGHT")), Err(CursorDecodeError::FilterMismatch));
    assert_eq!(
      FilterFingerprint::of("category=SPEED"),
      FilterFingerprint::of("category=SPEED"),
      "the same normalized filters fingerprint identically"
    );
  }

  #[test]
  fn a_final_page_publishes_a_null_next_cursor_rather_than_omitting_it() {
    let page = PageResponse { items: vec!["KNOTS"], next_cursor: None };

    assert_eq!(
      serde_json::to_value(page).expect("the envelope serializes"),
      json!({ "items": ["KNOTS"], "next_cursor": null }),
      "the decision says a null next cursor, not an absent member"
    );
  }
}
