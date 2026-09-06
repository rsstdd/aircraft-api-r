//! Transport-independent keyset pagination.
//!
//! `docs/architecture/http_v1_decisions.md` § "Pagination" is the accepted
//! contract: collection routes page by keyset with a bounded `limit`, each
//! endpoint owns an allowlist of deterministic sort orders ending in a unique
//! tiebreaker, and empty and final pages carry no next cursor.
//!
//! Nothing here knows about HTTP, base64, or JSON. The opaque cursor a client
//! sees is `aircraft_api::pagination`'s: this module owns only the bound on a
//! page and the shape of a page that came back, so a repository can build one
//! without depending on the transport that will render it.

use std::num::NonZeroU16;

/// How many rows one page may carry.
///
/// The value is always between one and [`PageLimit::MAX`], so a caller cannot
/// ask a repository for an unbounded read. Zero is unrepresentable rather than
/// substituted: `aircraft_api` types the query field as `NonZeroU16`, which
/// makes `limit=0` an ordinary query-validation refusal instead of a value this
/// crate invents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageLimit(u16);

impl PageLimit {
  /// Applied when a request names no limit.
  pub const DEFAULT: u16 = 50;
  /// The effective maximum: a larger request is capped, not refused. The
  /// accepted decision enumerates the `400` cases and an oversized limit is not
  /// among them.
  pub const MAX: u16 = 200;

  #[must_use]
  pub fn from_requested(requested: Option<NonZeroU16>) -> Self {
    Self(requested.map_or(Self::DEFAULT, |limit| limit.get().min(Self::MAX)))
  }

  #[must_use]
  pub const fn get(self) -> u16 {
    self.0
  }

  /// Rows a repository must ask for: one more than the page, so a full page can
  /// be told from a final one without a second query.
  ///
  /// [`Page::from_overfetched`] is the other half of this convention and will
  /// report no continuation if the caller fetches only [`PageLimit::get`] rows.
  #[must_use]
  pub const fn query_size(self) -> u16 {
    self.0.saturating_add(1)
  }
}

/// One page of `T`, and the key a caller resumes from.
///
/// Fields are private and [`Page::from_overfetched`] is the only constructor,
/// because the resume key must come from the **last retained** row. Taking it
/// from the discarded lookahead row would skip that row when the next page is
/// requested, and nothing downstream could detect it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<T, K> {
  items: Vec<T>,
  next: Option<K>,
}

impl<T, K> Page<T, K> {
  /// Builds a page from the rows a [`PageLimit::query_size`] query returned.
  ///
  /// A continuation exists only when the lookahead row came back. Empty, short,
  /// and exactly-`limit` results are all final pages, which is what the
  /// accepted decision means by a null next cursor on empty and final pages.
  pub fn from_overfetched(mut rows: Vec<T>, limit: PageLimit, key_of: impl Fn(&T) -> K) -> Self {
    let page = usize::from(limit.get());
    if rows.len() > page {
      rows.truncate(page);
      let next = rows.last().map(&key_of);
      return Self { items: rows, next };
    }
    Self { items: rows, next: None }
  }

  #[must_use]
  pub fn items(&self) -> &[T] {
    &self.items
  }

  #[must_use]
  pub const fn next(&self) -> Option<&K> {
    self.next.as_ref()
  }
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use super::*;

  fn limit(value: u16) -> PageLimit {
    PageLimit::from_requested(NonZeroU16::new(value))
  }

  #[test]
  fn an_absent_limit_is_the_default_and_an_oversized_one_is_the_maximum() {
    // Values read from `http_v1_decisions.md` § "Pagination", not from the
    // constants: "The default limit is 50 and the effective maximum is 200."
    assert_eq!(PageLimit::from_requested(None).get(), 50, "an absent limit");
    assert_eq!(limit(1).get(), 1, "the smallest representable request");
    assert_eq!(limit(200).get(), 200, "the maximum itself");
    assert_eq!(limit(201).get(), 200, "one past the maximum is capped");
    assert_eq!(limit(u16::MAX).get(), 200, "and so is anything larger");
  }

  #[test]
  fn a_page_is_queried_with_one_row_of_lookahead() {
    assert_eq!(limit(50).query_size(), 51);
    assert_eq!(limit(200).query_size(), 201, "the largest read this contract allows");
  }

  #[test]
  fn only_a_returned_lookahead_row_yields_a_continuation() {
    let key_of = |row: &u16| *row;

    let empty = Page::from_overfetched(Vec::new(), limit(2), key_of);
    let short = Page::from_overfetched(vec![10], limit(2), key_of);
    let exact = Page::from_overfetched(vec![10, 20], limit(2), key_of);
    let overfetched = Page::from_overfetched(vec![10, 20, 30], limit(2), key_of);

    assert_eq!((empty.items(), empty.next()), (&[][..], None), "an empty page is final");
    assert_eq!((short.items(), short.next()), (&[10][..], None), "a short page is final");
    assert_eq!(
      (exact.items(), exact.next()),
      (&[10, 20][..], None),
      "exactly the limit is final: no lookahead row came back"
    );
    assert_eq!(
      (overfetched.items(), overfetched.next()),
      (&[10, 20][..], Some(&20)),
      "the key is the last retained row, not the discarded lookahead"
    );
  }
}
