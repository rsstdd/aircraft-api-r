//! Catalog identity: the keys and codes a family, model, or variant is named by.
//!
//! The public identifier is the slug, which
//! `database/migrations/004_aircraft_identity_taxonomy.sql` declares
//! `NOT NULL UNIQUE` on families, models, and variants alike; the numeric
//! identifiers are internal keys a port takes so one aggregate's key cannot be
//! passed for another's. Nothing
//! here is a projection: what a read returns is `aircraft_app::catalog`'s, which
//! names this module in turn.
//!
//! The three value types mirror constraints
//! `database/migrations/001_extensions_schemas_domains_triggers.sql` declares --
//! the `aircraft_ref.slug_text` domain (`:124`), the `aircraft_ref.lookup_code`
//! domain (`:131`), and the `VARCHAR(3)` ISO 3166-1 alpha-3 primary key of
//! `aircraft_geo.countries` -- so a value that could not have come out of the
//! database cannot be built here either. `crates/AGENTS.md` requires the domain
//! constructor to validate rather than lean on the constraint alone.
//!
//! ```compile_fail,E0308
//! use aircraft_domain::catalog::{FamilyId, ModelId};
//!
//! fn models_of(family: FamilyId) -> i64 {
//!   family.get()
//! }
//!
//! // A model's key is not a family's: this is the whole point of three types.
//! let _ = models_of(ModelId::new(1));
//! ```

use thiserror::Error;

/// A value that could not have come from the column it names.
///
/// One error with a variant per vocabulary rather than three error types: no
/// caller reacts differently to them -- a route refuses the request either way
/// -- but a test and a log still need to say which rule was broken. It carries
/// no rejected text: the value is caller-supplied, and a variant holding it
/// would invite echoing it into a log or a response.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum InvalidCatalogValue {
  #[error("a slug is lower-case alphanumeric words joined by single hyphens")]
  Slug,
  #[error("a lookup code is upper-case alphanumeric words joined by underscores")]
  LookupCode,
  #[error("a country code is three upper-case ASCII letters")]
  CountryCode,
}

/// Declares a validated string newtype whose parser is `check`.
///
/// The three vocabularies differ only in their predicate and their error
/// variant; writing the same three items out three times would leave three
/// places for a future `TryFrom` to drift apart. The documentation is
/// passed as attributes rather than a string literal so each type's prose reads
/// as ordinary rustdoc at its own declaration.
macro_rules! validated_text {
  ($(#[$doc:meta])* $name:ident, $variant:ident, |$value:ident| $check:expr) => {
    $(#[$doc])*
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct $name(String);

    impl $name {
      #[must_use]
      pub fn as_str(&self) -> &str {
        &self.0
      }
    }

    impl TryFrom<&str> for $name {
      type Error = InvalidCatalogValue;

      fn try_from(value: &str) -> Result<Self, Self::Error> {
        let $value = value;
        if $check {
          return Ok(Self(value.to_owned()));
        }
        Err(InvalidCatalogValue::$variant)
      }
    }
  };
}

validated_text!(
  /// The public identifier of a family, model, or variant.
  ///
  /// Mirrors the `aircraft_ref.slug_text` domain: lower-case ASCII alphanumeric
  /// groups joined by single hyphens, with no leading, trailing, or doubled
  /// separator.
  Slug,
  Slug,
  |value| !value.is_empty()
    && value.split('-').all(|part| {
      !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
);

validated_text!(
  /// A row of one of the `aircraft_ref` lookup catalogs.
  ///
  /// Mirrors the `aircraft_ref.lookup_code` domain: an upper-case ASCII letter,
  /// then upper-case letters, digits, and underscores. Carried as a value rather
  /// than a `String` so a projection cannot hold a spelling the lookup tables
  /// never defined.
  ///
  /// [`crate::measurement::UnitCode`] validates the same syntax for a different
  /// vocabulary -- a measurement unit is not a variant type, and collapsing the
  /// two would let one be passed for the other.
  LookupCode,
  LookupCode,
  |value| {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_uppercase())
      && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
  }
);

validated_text!(
  /// An ISO 3166-1 alpha-3 country code.
  ///
  /// The primary key of `aircraft_geo.countries`, which is `VARCHAR(3)`; the
  /// `alpha2` column is a different vocabulary and is not this type.
  CountryCode,
  CountryCode,
  |value| value.len() == 3 && value.bytes().all(|b| b.is_ascii_uppercase())
);

/// Declares an aggregate's internal key.
///
/// Three separate types rather than one generic `Id<T>` or a shared alias: the
/// point is that no conversion exists between them, and a phantom parameter
/// would still let `Id::<Family>::new` be written where a model was meant.
macro_rules! identifier {
  ($(#[$doc:meta])* $name:ident) => {
    $(#[$doc])*
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct $name(i64);

    impl $name {
      #[must_use]
      pub const fn new(key: i64) -> Self {
        Self(key)
      }

      #[must_use]
      pub const fn get(self) -> i64 {
        self.0
      }
    }
  };
}

identifier!(
  /// The key of an `aircraft_core.families` row.
  ///
  /// Internal: a caller names a family by its slug, and this is what a port
  /// carries once one has been resolved.
  FamilyId
);
identifier!(
  /// The key of an `aircraft_core.models` row.
  ModelId
);
identifier!(
  /// The key of an `aircraft_core.variants` row.
  VariantId
);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_slug_admits_what_the_schema_admits_and_refuses_the_rest() {
    for accepted in ["cessna", "cessna-172", "f-16", "a380-800", "x1"] {
      assert_eq!(
        Slug::try_from(accepted).map(|value| value.as_str().to_owned()),
        Ok(accepted.to_owned())
      );
    }
    for refused in ["", "-cessna", "cessna-", "cessna--172", "Cessna", "cessna_172", "cessna 172"] {
      assert_eq!(
        Slug::try_from(refused),
        Err(InvalidCatalogValue::Slug),
        "segment {refused:?} is not a slug_text value"
      );
    }
  }

  #[test]
  fn a_lookup_code_admits_what_the_schema_admits_and_refuses_the_rest() {
    for accepted in ["JET", "RETRACTABLE_TRICYCLE", "AD1", "A"] {
      assert_eq!(
        LookupCode::try_from(accepted).map(|value| value.as_str().to_owned()),
        Ok(accepted.to_owned())
      );
    }
    for refused in ["", "jet", "_JET", "1JET", "JET-A", "JET A"] {
      assert_eq!(
        LookupCode::try_from(refused),
        Err(InvalidCatalogValue::LookupCode),
        "code {refused:?} is not a lookup_code value"
      );
    }
  }

  #[test]
  fn a_country_code_is_three_upper_case_letters() {
    for accepted in ["USA", "GBR", "FRA"] {
      assert_eq!(
        CountryCode::try_from(accepted).map(|value| value.as_str().to_owned()),
        Ok(accepted.to_owned())
      );
    }
    for refused in ["", "US", "USAA", "usa", "US1", "U S"] {
      assert_eq!(
        CountryCode::try_from(refused),
        Err(InvalidCatalogValue::CountryCode),
        "code {refused:?} is not an alpha-3 country code"
      );
    }
  }

  /// The identifiers are distinct types, not aliases: each carries its own row's
  /// key and nothing converts between them. The compile-time half of this is the
  /// `compile_fail` doctest on the module.
  #[test]
  fn each_identifier_carries_its_own_key() {
    assert_eq!(FamilyId::new(7).get(), 7);
    assert_eq!(ModelId::new(7).get(), 7);
    assert_eq!(VariantId::new(7).get(), 7);
  }
}
