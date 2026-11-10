//! Location lookup state: what was detected, where it came from, and when to try again

use lumina::location::Location;
use std::time::{Duration, Instant};

/// Delay before the first retry of a failed location lookup (seconds)
const BACKOFF_BASE_SECS: u64 = 5;
/// Longest delay between location lookup retries (seconds)
const BACKOFF_MAX_SECS: u64 = 300;

/// How long to wait before retrying a live lookup after falling back to the cached location
/// (seconds)
const CACHE_REFRESH_SECS: u64 = 3600;

/// Delay before the next location lookup after `failures` consecutive failures
fn backoff(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(6);
    Duration::from_secs((BACKOFF_BASE_SECS << exp).min(BACKOFF_MAX_SECS))
}

/// Tracks the detected location and decides when the next lookup attempt is due
pub(super) struct LocationTracker {
    /// Location found by lookup, used when the config has none
    detected: Option<Location>,
    /// `true` when `detected` came from the on-disk cache because the live lookup failed
    from_cache: bool,
    /// Consecutive failed lookups
    failures: u32,
    /// Earliest time of the next lookup attempt
    retry_at: Instant,
}

impl LocationTracker {
    /// A tracker with nothing detected and a lookup due immediately
    pub(super) fn new(now: Instant) -> Self {
        Self {
            detected: None,
            from_cache: false,
            failures: 0,
            retry_at: now,
        }
    }

    /// The location found so far, if any
    pub(super) fn detected(&self) -> Option<Location> {
        self.detected
    }

    /// `true` when a lookup should run now: nothing detected yet, or the detected location came
    /// from the cache. `retry_at` holds back failed lookups and cache refreshes
    pub(super) fn is_due(&self, now: Instant) -> bool {
        (self.detected.is_none() || self.from_cache) && now >= self.retry_at
    }

    /// Record a successful lookup. A cached answer schedules another live attempt. Returns `true`
    /// when the location or its source changed, so the caller knows whether to log it
    pub(super) fn record_found(&mut self, loc: Location, from_cache: bool, now: Instant) -> bool {
        let changed = self.detected != Some(loc) || self.from_cache != from_cache;

        self.detected = Some(loc);
        self.from_cache = from_cache;
        self.failures = 0;

        if from_cache {
            self.retry_at = now + Duration::from_secs(CACHE_REFRESH_SECS);
        }

        changed
    }

    /// Record a failed lookup and back off before the next one. Returns the delay chosen
    pub(super) fn record_failure(&mut self, now: Instant) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let wait = backoff(self.failures);
        self.retry_at = now + wait;

        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(latitude: f64, longitude: f64) -> Location {
        Location {
            latitude,
            longitude,
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let s = |n| backoff(n).as_secs();
        assert_eq!((s(1), s(2), s(3)), (5, 10, 20));
        assert_eq!((s(7), s(100)), (300, 300));
    }

    #[test]
    fn lookup_is_due_immediately() {
        let t0 = Instant::now();
        assert!(LocationTracker::new(t0).is_due(t0));
    }

    #[test]
    fn live_result_is_never_due_again() {
        let t0 = Instant::now();
        let mut t = LocationTracker::new(t0);
        t.record_found(loc(47.0, -122.0), false, t0);

        assert_eq!(t.detected(), Some(loc(47.0, -122.0)));
        assert!(!t.is_due(t0));
        assert!(!t.is_due(t0 + secs(86_400)));
    }

    #[test]
    fn cached_result_schedules_a_refresh() {
        let t0 = Instant::now();
        let mut t = LocationTracker::new(t0);
        t.record_found(loc(47.0, -122.0), true, t0);

        assert!(!t.is_due(t0));
        assert!(!t.is_due(t0 + secs(CACHE_REFRESH_SECS - 1)));
        assert!(t.is_due(t0 + secs(CACHE_REFRESH_SECS)));
    }

    #[test]
    fn failures_back_off_and_success_resets() {
        let t0 = Instant::now();
        let mut t = LocationTracker::new(t0);

        assert_eq!(t.record_failure(t0), secs(5));
        assert!(!t.is_due(t0 + secs(4)));
        assert!(t.is_due(t0 + secs(5)));

        assert_eq!(t.record_failure(t0 + secs(5)), secs(10));

        t.record_found(loc(47.0, -122.0), false, t0 + secs(15));
        assert_eq!(t.record_failure(t0 + secs(20)), secs(5));
    }

    #[test]
    fn record_found_reports_changes_only() {
        let t0 = Instant::now();
        let mut t = LocationTracker::new(t0);
        let a = loc(47.0, -122.0);

        assert!(t.record_found(a, false, t0), "first fix is a change");
        assert!(!t.record_found(a, false, t0), "same fix, same source");
        assert!(t.record_found(a, true, t0), "source flipped to cache");
        assert!(t.record_found(loc(48.0, -122.0), true, t0), "new location");
    }
}
