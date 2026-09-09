//! Per-principal request rate limiting for one replica.
//!
//! `docs/architecture/http_v1_decisions.md` § "Single-replica rate limiting and
//! perimeter bounds" is the accepted contract and names this module in turn: a
//! bounded in-memory token bucket keyed by authenticated principal ID, capacity
//! and refill rate from the principal's configured tier, isolated buckets,
//! monotonic time, and the shared `429` with `Retry-After` when a bucket is
//! empty. Running out of room for buckets is the other refusal that section
//! records, and it is a `503` rather than a `429`: see [`Admission`].
//! Cross-replica enforcement is deferred there, not forgotten here.
//!
//! The quota values are configuration rather than schema, which is
//! `database/migrations/025_authentication_schema.sql`'s decision:
//! `aircraft_auth.rate_limit_tiers` stores a tier's identity and nothing else,
//! so a quota changes without a migration. `aircraft_config`'s
//! `http.rate_limit_*` settings carry the numbers and
//! `apps/server/src/main.rs` builds a [`RateLimitPolicy`] from them.
//!
//! Nothing here reads a clock. [`RateLimiter::admit`] takes the instant as an
//! argument so every behavior below is provable without sleeping, and
//! `routes::mod::Policed` is the one caller that reads `Instant::now`.

use std::{
  collections::HashMap,
  num::NonZeroU32,
  sync::{Mutex, PoisonError},
  time::{Duration, Instant},
};

use thiserror::Error;

/// Tokens are counted in thousandths so refill is exact integer arithmetic.
///
/// A rate is configured in whole tokens per second, so one millisecond of
/// elapsed time is worth exactly `refill_per_second` of these units and nothing
/// is lost to rounding. Floating point would make the tests assert around an
/// epsilon instead of an equality.
const TOKENS_PER_UNIT: u64 = 1_000;

/// What a refused caller is told to wait.
///
/// Always one second, and deliberately not a computed remainder: the smallest
/// configurable rate is one token per second, so a token is at most a second
/// away from any empty bucket, and a `Retry-After` of zero would invite an
/// immediate retry. A finer-grained rate would make this a real computation.
const RETRY_AFTER_SECONDS: u64 = 1;

/// A tier's capacity and refill rate.
///
/// Both are non-zero by construction: a capacity of zero refuses every request
/// and a refill of zero makes the first exhaustion permanent, so neither is a
/// quota, and storing them as [`NonZeroU32`] keeps every later division and
/// comparison from having to ask.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Quota {
  capacity: NonZeroU32,
  refill_per_second: NonZeroU32,
}

impl Quota {
  /// Builds a quota from configured values.
  ///
  /// # Errors
  ///
  /// [`InvalidQuota`] when either value is zero. `aircraft_config` rejects a
  /// zero under its own setting path first; this is the guard for any other
  /// caller, and it is what makes the arithmetic below total.
  pub fn new(capacity: u32, refill_per_second: u32) -> Result<Self, InvalidQuota> {
    let capacity = NonZeroU32::new(capacity).ok_or(InvalidQuota)?;
    let refill_per_second = NonZeroU32::new(refill_per_second).ok_or(InvalidQuota)?;
    Ok(Self { capacity, refill_per_second })
  }

  fn capacity_units(self) -> u64 {
    u64::from(self.capacity.get()).saturating_mul(TOKENS_PER_UNIT)
  }
}

/// A quota that would serve nobody.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("a rate-limit quota needs a non-zero capacity and refill rate")]
pub struct InvalidQuota;

/// A policy this service cannot enforce.
///
/// Carries the offending entry's position and no operator text: a 1-based index
/// is enough to find the entry in the configured list -- including the empty one
/// a trailing comma leaves behind, which is otherwise invisible in the value --
/// while echoing the entry itself would put configuration content into a second
/// diagnostic path alongside `aircraft_config`'s own error naming the setting.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum InvalidPolicy {
  #[error("a rate-limit policy needs a non-zero bucket ceiling")]
  BucketCeiling,
  #[error(
    "rate-limit tier override {position} is not CODE:capacity:refill_per_second with non-zero \
     numbers and a code no earlier entry used"
  )]
  TierOverride { position: usize },
}

/// The quotas in force, and the ceiling on how many buckets may exist.
#[derive(Clone, Debug)]
pub struct RateLimitPolicy {
  default: Quota,
  tiers: HashMap<String, Quota>,
  max_buckets: usize,
}

impl RateLimitPolicy {
  /// Builds the policy from `http.rate_limit_*` configuration.
  ///
  /// `overrides` are the raw `CODE:capacity:refill_per_second` entries of
  /// `http.rate_limit_tiers`, parsed here rather than in `aircraft_config`
  /// so one parser owns the spelling. The CORS origins are validated on both
  /// sides instead, because a malformed origin panics inside `tower-http`;
  /// a malformed quota cannot panic anywhere, so a second validator would only
  /// be a second thing to keep in step.
  ///
  /// # Errors
  ///
  /// [`InvalidPolicy::BucketCeiling`] for a `max_buckets` of zero, which is a
  /// store that could hold nothing and would refuse every caller.
  /// [`InvalidPolicy::TierOverride`], naming the entry's 1-based position, for
  /// an override that is not three colon-separated parts, whose numbers do not
  /// parse or are zero, whose code is empty, or whose code repeats an earlier
  /// entry. A repeat is refused rather than resolved, because either quota is a
  /// defensible reading of the operator's intent and picking one silently would
  /// apply a limit nobody chose. An empty entry is refused for the same reason
  /// `http.cors_allowed_origins` refuses one: it is a typo, most often the
  /// trailing comma `aircraft_config`'s splitter turns into an empty string, and
  /// the position is what points at it.
  pub fn new(
    default: Quota,
    max_buckets: usize,
    overrides: &[String],
  ) -> Result<Self, InvalidPolicy> {
    if max_buckets == 0 {
      return Err(InvalidPolicy::BucketCeiling);
    }
    let mut tiers = HashMap::with_capacity(overrides.len());
    for (index, entry) in overrides.iter().enumerate() {
      let position = index + 1;
      let refused = InvalidPolicy::TierOverride { position };
      let mut parts = entry.split(':');
      let (Some(code), Some(capacity), Some(refill), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
      else {
        return Err(refused);
      };
      let quota =
        Quota::new(capacity.parse().map_err(|_| refused)?, refill.parse().map_err(|_| refused)?)
          .map_err(|_| refused)?;
      if code.is_empty() || tiers.insert(code.to_owned(), quota).is_some() {
        return Err(refused);
      }
    }
    Ok(Self { default, tiers, max_buckets })
  }

  /// The quota for a tier code, or the default for one nothing configures.
  ///
  /// A tier is an operator-created row in `aircraft_auth.rate_limit_tiers` and
  /// configuration need not mention it, so an unmatched code takes the default
  /// limit. Exempting it would make an unconfigured tier the way around this
  /// control; refusing it would turn adding a tier row into an outage.
  fn quota_for(&self, tier: &str) -> Quota {
    self.tiers.get(tier).copied().unwrap_or(self.default)
  }
}

/// One principal's tokens, the instant they were last computed, and the quota
/// they were computed under.
///
/// The quota is stored, not just its capacity: the eviction sweep runs over
/// buckets whose tier it does not know, and asking whether one has refilled by
/// some later instant needs its refill rate as well.
#[derive(Clone, Copy, Debug)]
struct Bucket {
  units: u64,
  last: Instant,
  quota: Quota,
}

impl Bucket {
  /// Whole milliseconds since this bucket was last computed.
  ///
  /// `saturating_duration_since` is the monotonic guard: an instant no later
  /// than the last one earns nothing rather than underflowing.
  fn elapsed_ms(&self, now: Instant) -> u64 {
    u64::try_from(now.saturating_duration_since(self.last).as_millis()).unwrap_or(u64::MAX)
  }

  /// The units that much elapsed time is worth, uncapped.
  fn earned(&self, elapsed_ms: u64) -> u64 {
    elapsed_ms.saturating_mul(u64::from(self.quota.refill_per_second.get()))
  }

  /// Adds the tokens `now` has earned, saturating at capacity.
  ///
  /// Only whole milliseconds are consumed, and `last` advances by exactly that
  /// much, so a burst of sub-millisecond requests cannot repeatedly discard a
  /// remainder and stall the refill. A zero gap needs no special case: nothing
  /// is earned and `last` does not move.
  fn refill(&mut self, quota: Quota, now: Instant) {
    self.quota = quota;
    let elapsed_ms = self.elapsed_ms(now);
    self.units = self.units.saturating_add(self.earned(elapsed_ms)).min(quota.capacity_units());
    self.last += Duration::from_millis(elapsed_ms);
  }

  /// Whether this bucket would be at capacity at `now`.
  ///
  /// As of `now`, not as of its last request, and that is the whole point: a
  /// bucket is refilled only when its own principal returns, so every bucket in
  /// the store sits at least one token below capacity from the request that
  /// created it. Asking `units >= capacity` against the stored value is false
  /// for every bucket, which would leave the sweep below evicting nothing and
  /// the store refusing every new principal once it filled.
  fn is_full_at(&self, now: Instant) -> bool {
    self.units.saturating_add(self.earned(self.elapsed_ms(now))) >= self.quota.capacity_units()
  }
}

/// Whether a request may proceed, and if not, whose doing it is.
///
/// The two refusals are separate because a caller and an operator must be told
/// different things. [`Admission::Refused`] is the principal's own rate and
/// clears on its own within `retry_after_seconds`. [`Admission::Saturated`] is
/// this replica having no room to track another principal: the caller has spent
/// nothing, slowing down does not help it, and it clears only when the tracked
/// principals go idle or an operator raises `http.rate_limit_max_buckets`.
/// Reporting the second as the first tells a blameless caller it exceeded a
/// quota it never touched, and hides a capacity problem from the operator who
/// owns it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
  Admitted,
  Refused { retry_after_seconds: u64 },
  Saturated,
}

/// The buckets in force for this replica.
///
/// Shared by every request through `ApiState`, so it is stored behind one
/// `Arc` rather than rebuilt per router -- the opposite of
/// `PerimeterLimits::permits`, whose whole point is that a clone carries a
/// bound and not a share of live permits.
#[derive(Debug)]
pub struct RateLimiter {
  policy: RateLimitPolicy,
  buckets: Mutex<HashMap<i64, Bucket>>,
}

impl RateLimiter {
  #[must_use]
  pub fn new(policy: RateLimitPolicy) -> Self {
    Self { policy, buckets: Mutex::new(HashMap::new()) }
  }

  /// Spends one token for `principal_id`, or refuses.
  ///
  /// Synchronous and holding the lock only across a map lookup and integer
  /// arithmetic: nothing is awaited inside, so this cannot park a request
  /// behind another principal's work. A poisoned lock is recovered rather than
  /// unwrapped, because a panic elsewhere must not turn every later request
  /// into a panic on the request path.
  ///
  /// `tier` is re-read on every call, so a principal moved to another tier in
  /// the database takes its new quota on its next request rather than when its
  /// bucket happens to be evicted.
  pub fn admit(&self, principal_id: i64, tier: &str, now: Instant) -> Admission {
    let quota = self.policy.quota_for(tier);
    let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
    self.spend(&mut buckets, principal_id, quota, now)
  }

  /// The whole of the guarded work, so the lock is taken in one place and held
  /// across exactly this and nothing more.
  fn spend(
    &self,
    buckets: &mut HashMap<i64, Bucket>,
    principal_id: i64,
    quota: Quota,
    now: Instant,
  ) -> Admission {
    // Admission control, before the lookup that would grow the store: a
    // principal the store already tracks always has its bucket, and only a new
    // one can push the map past its ceiling.
    if buckets.len() >= self.policy.max_buckets && !buckets.contains_key(&principal_id) {
      // ponytail: evict every bucket that has refilled to capacity by now,
      // which is indistinguishable from one that never existed, and report
      // saturation when none has. The bound is the control this enforces, so
      // exceeding it is the failure to avoid; an exhausted bucket is never
      // evicted, because that would let a caller clear its own debt by cycling
      // principals. The sweep
      // is `O(max_buckets)` and runs only while the store is full, behind a
      // lookup that already cost a credential round trip; an LRU or a sharded
      // map is the upgrade path if it ever matters.
      buckets.retain(|_, bucket| !bucket.is_full_at(now));
      if buckets.len() >= self.policy.max_buckets {
        return Admission::Saturated;
      }
    }

    // A principal the store has not seen starts full, which is what makes a
    // bucket's absence and a bucket at capacity the same state -- the property
    // the eviction above relies on.
    let bucket = buckets.entry(principal_id).or_insert_with(|| Bucket {
      units: quota.capacity_units(),
      last: now,
      quota,
    });

    bucket.refill(quota, now);
    if bucket.units < TOKENS_PER_UNIT {
      return Admission::Refused { retry_after_seconds: RETRY_AFTER_SECONDS };
    }
    bucket.units -= TOKENS_PER_UNIT;
    Admission::Admitted
  }

  /// How many buckets are live. The ceiling is not observable any other way,
  /// and evicting a full bucket is deliberately invisible to a caller.
  #[cfg(test)]
  fn bucket_count(&self) -> usize {
    self.buckets.lock().unwrap_or_else(PoisonError::into_inner).len()
  }
}

#[cfg(test)]
mod tests {
  // A failing assertion is the point of a test.
  #![allow(clippy::expect_used)]

  use std::time::{Duration, Instant};

  use super::*;

  fn quota(capacity: u32, refill_per_second: u32) -> Quota {
    Quota::new(capacity, refill_per_second).expect("a non-zero quota")
  }

  fn limiter(capacity: u32, refill_per_second: u32) -> RateLimiter {
    RateLimiter::new(
      RateLimitPolicy::new(quota(capacity, refill_per_second), 16, &[]).expect("no overrides"),
    )
  }

  #[test]
  fn a_bucket_admits_its_capacity_and_then_refuses() {
    let limiter = limiter(3, 1);
    let now = Instant::now();

    for attempt in 1..=3 {
      assert_eq!(limiter.admit(7, "STANDARD", now), Admission::Admitted, "attempt {attempt}");
    }

    assert_eq!(
      limiter.admit(7, "STANDARD", now),
      Admission::Refused { retry_after_seconds: 1 },
      "the fourth request exceeds a capacity of three"
    );
  }

  #[test]
  fn a_bucket_refills_at_its_tier_rate_over_elapsed_time() {
    let limiter = limiter(10, 2);
    let start = Instant::now();
    for _ in 0..10 {
      assert_eq!(limiter.admit(1, "STANDARD", start), Admission::Admitted);
    }

    let later = start + Duration::from_secs(3);
    for attempt in 1..=6 {
      assert_eq!(limiter.admit(1, "STANDARD", later), Admission::Admitted, "attempt {attempt}");
    }

    assert!(
      matches!(limiter.admit(1, "STANDARD", later), Admission::Refused { .. }),
      "three seconds at two per second returns exactly six tokens"
    );
    let much_later = later + Duration::from_secs(3_600);
    for attempt in 1..=10 {
      assert_eq!(
        limiter.admit(1, "STANDARD", much_later),
        Admission::Admitted,
        "attempt {attempt}"
      );
    }
    assert!(
      matches!(limiter.admit(1, "STANDARD", much_later), Admission::Refused { .. }),
      "an idle hour refills to capacity and no further"
    );
  }

  #[test]
  fn an_instant_that_is_not_later_grants_no_tokens() {
    let limiter = limiter(1, 100);
    let now = Instant::now();
    let earlier =
      now.checked_sub(Duration::from_secs(60)).expect("an instant a minute in the past");

    assert_eq!(limiter.admit(4, "STANDARD", now), Admission::Admitted);

    assert!(
      matches!(limiter.admit(4, "STANDARD", earlier), Admission::Refused { .. }),
      "an earlier instant must not refill the bucket"
    );
    assert!(
      matches!(limiter.admit(4, "STANDARD", now), Admission::Refused { .. }),
      "and must not have advanced the bucket's clock either"
    );
  }

  #[test]
  fn one_principals_exhaustion_does_not_refuse_another() {
    let limiter = limiter(1, 1);
    let now = Instant::now();

    assert_eq!(limiter.admit(1, "STANDARD", now), Admission::Admitted);
    assert!(matches!(limiter.admit(1, "STANDARD", now), Admission::Refused { .. }));

    assert_eq!(
      limiter.admit(2, "STANDARD", now),
      Admission::Admitted,
      "a second principal holds its own allowance"
    );
  }

  #[test]
  fn a_tier_override_is_used_and_an_unconfigured_tier_gets_the_default() {
    let policy =
      RateLimitPolicy::new(quota(1, 1), 16, &["GENEROUS:3:1".to_owned()]).expect("one override");
    let limiter = RateLimiter::new(policy);
    let now = Instant::now();

    for attempt in 1..=3 {
      assert_eq!(limiter.admit(1, "GENEROUS", now), Admission::Admitted, "attempt {attempt}");
    }
    assert!(matches!(limiter.admit(1, "GENEROUS", now), Admission::Refused { .. }));

    assert_eq!(limiter.admit(2, "UNLISTED", now), Admission::Admitted);
    assert!(
      matches!(limiter.admit(2, "UNLISTED", now), Admission::Refused { .. }),
      "an unconfigured tier is limited by the default, not exempt from limiting"
    );
  }

  /// Churn cannot clear a principal's own debt: only a bucket that has refilled
  /// to capacity is evictable, and an exhausted one is still exhausted after the
  /// store has turned over.
  ///
  /// The two refusals are asserted together because they are the pair that must
  /// stay apart: the principal that spent its allowance is [`Admission::Refused`]
  /// and the one the full store cannot track at all is [`Admission::Saturated`],
  /// and a limiter that answered the same thing to both would pass either
  /// assertion alone.
  #[test]
  fn an_exhausted_bucket_survives_the_churn_of_other_principals() {
    const CEILING: usize = 4;
    let limiter =
      RateLimiter::new(RateLimitPolicy::new(quota(2, 1), CEILING, &[]).expect("no overrides"));
    let now = Instant::now();

    // One principal spends everything it has; the rest arrive and spend one of
    // their two tokens, so none of them is evictable at this instant either.
    assert_eq!(limiter.admit(0, "STANDARD", now), Admission::Admitted);
    assert_eq!(limiter.admit(0, "STANDARD", now), Admission::Admitted);
    for principal in 1..20 {
      limiter.admit(principal, "STANDARD", now);
      assert!(limiter.bucket_count() <= CEILING, "principal {principal} grew the store");
    }

    assert_eq!(
      limiter.admit(20, "STANDARD", now),
      Admission::Saturated,
      "a principal the full store cannot track is refused for this replica's capacity"
    );
    assert!(
      matches!(limiter.admit(0, "STANDARD", now), Admission::Refused { .. }),
      "an exhausted bucket is not evicted, so its debt cannot be cleared by churn"
    );
  }

  /// A principal that stopped calling is reclaimed, so a full store is not a
  /// permanent refusal for everyone who arrives after it.
  ///
  /// This is the half a ceiling assertion cannot prove on its own: a sweep that
  /// evicts nothing keeps the store just as bounded, and refuses every new
  /// principal forever.
  #[test]
  fn an_idle_bucket_is_reclaimed_so_a_new_principal_is_still_admitted() {
    const CEILING: usize = 4;
    let limiter =
      RateLimiter::new(RateLimitPolicy::new(quota(2, 1), CEILING, &[]).expect("no overrides"));
    let now = Instant::now();

    for principal in 0..i64::try_from(CEILING).expect("a small ceiling") {
      assert_eq!(limiter.admit(principal, "STANDARD", now), Admission::Admitted);
    }
    assert_eq!(limiter.bucket_count(), CEILING, "the store is full of now-idle principals");

    let later = now + Duration::from_secs(3_600);

    assert_eq!(
      limiter.admit(99, "STANDARD", later),
      Admission::Admitted,
      "an idle store must make room for a principal that arrives later"
    );
    assert!(limiter.bucket_count() <= CEILING, "and must still respect its ceiling");
  }

  #[test]
  fn a_store_that_can_hold_no_buckets_is_refused() {
    assert!(
      RateLimitPolicy::new(quota(1, 1), 0, &[]).is_err(),
      "a ceiling of zero is a store that could refuse every caller"
    );
    assert!(RateLimitPolicy::new(quota(1, 1), 1, &[]).is_ok(), "or this proves nothing");
  }

  #[test]
  fn a_quota_of_zero_is_refused() {
    assert!(Quota::new(0, 1).is_err(), "a capacity of zero would refuse every request");
    assert!(Quota::new(1, 0).is_err(), "a refill of zero would never return a token");
    assert!(Quota::new(1, 1).is_ok(), "or these assertions prove nothing");
  }

  /// Every rejected spelling, and the position that points at it. The empty
  /// entry is the one an operator never wrote: `aircraft_config`'s splitter
  /// turns the trailing comma in `BURST:600:100,` into one, and a diagnostic
  /// that named neither the entry nor its place would leave nothing to look at.
  #[test]
  fn a_malformed_tier_override_is_refused_by_position_naming_nothing_it_was_given() {
    let refused = |overrides: &[&str]| {
      RateLimitPolicy::new(
        quota(1, 1),
        16,
        &overrides.iter().map(|entry| (*entry).to_owned()).collect::<Vec<_>>(),
      )
      .err()
    };

    for malformed in ["STANDARD", "STANDARD:5", "STANDARD:5:", "STANDARD:x:1", "STANDARD:0:1", ""] {
      let error = refused(&["BURST:6:1", malformed]);

      assert_eq!(
        error,
        Some(InvalidPolicy::TierOverride { position: 2 }),
        "override {malformed:?} must be refused by its 1-based position"
      );
      assert!(
        !error.expect("refused above").to_string().contains("STANDARD"),
        "the failure must not echo the entry it was given for {malformed:?}"
      );
    }

    assert_eq!(
      refused(&[":5:1"]),
      Some(InvalidPolicy::TierOverride { position: 1 }),
      "an empty code is refused, and the first entry is position one and not zero"
    );
    assert_eq!(refused(&["STANDARD:5:1"]), None, "or the negatives prove nothing");
    assert_eq!(
      refused(&["STANDARD:5:1", "STANDARD:6:1"]),
      Some(InvalidPolicy::TierOverride { position: 2 }),
      "a repeated tier code is ambiguous rather than last-wins, and the repeat is what is named"
    );
    assert_eq!(
      RateLimitPolicy::new(quota(1, 1), 0, &[]).err(),
      Some(InvalidPolicy::BucketCeiling),
      "and a ceiling of zero is its own cause, not a tier override"
    );
  }
}
