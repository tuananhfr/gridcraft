//! When the next sync asked for by a refused edit may run: at most one a minute, doubling to an
//! hour while the lock stays (the background schedule of erpcons-account lease.md §6).

use std::time::{Duration, Instant};

use erpcons_account::Error;

pub(super) const RESYNC_MIN: Duration = Duration::from_secs(60);
pub(super) const RESYNC_MAX: Duration = Duration::from_secs(3600);

pub(super) struct Resync {
    min: Duration,
    max: Duration,
    not_before: Instant,
    failures: u32,
}

impl Resync {
    pub(super) fn new(min: Duration, max: Duration, now: Instant) -> Self {
        Self { min, max, not_before: now, failures: 0 }
    }

    pub(super) fn due(&self, now: Instant) -> bool {
        now >= self.not_before
    }

    /// How long until [`Resync::due`].
    pub(super) fn wait(&self, now: Instant) -> Duration {
        self.not_before.saturating_duration_since(now)
    }

    /// The user asked for a sync (Retry): the next refused edit may sync again soon.
    pub(super) fn reset(&mut self) {
        self.failures = 0;
    }

    /// Schedules the next slot after a sync that ended with `error`, the lock still on or not.
    pub(super) fn after(&mut self, now: Instant, error: Option<&Error>, still_locked: bool) {
        let delay = if still_locked || error.is_some() {
            let doubled = self.min.saturating_mul(1 << self.failures.min(6)).min(self.max);
            self.failures = self.failures.saturating_add(1);
            error.and_then(Error::retry_after).map_or(doubled, |after| after.max(doubled))
        } else {
            self.failures = 0;
            self.min
        };
        self.not_before = now.checked_add(delay).unwrap_or(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_a_minute_and_backs_off_to_an_hour_while_locked() {
        let now = super::super::now();
        let mut r = Resync::new(RESYNC_MIN, RESYNC_MAX, now);
        assert!(r.due(now), "the first refused edit syncs at once");
        let delays: Vec<u64> = (0..8)
            .map(|_| {
                r.after(now, None, true);
                r.wait(now).as_secs()
            })
            .collect();
        assert_eq!(delays, [60, 120, 240, 480, 960, 1920, 3600, 3600]);
        r.after(now, Some(&Error::Timeout), false);
        assert_eq!(r.wait(now), Duration::from_secs(3600));
        r.after(now, None, false);
        assert_eq!(r.wait(now), Duration::from_secs(60), "a lifted lock resets the schedule");
        assert_eq!(r.wait(now + Duration::from_secs(45)), Duration::from_secs(15));
        assert!(!r.due(now) && r.due(now + Duration::from_secs(60)));
        r.after(now, None, true);
        r.after(now, None, true);
        r.reset();
        r.after(now, None, true);
        assert_eq!(r.wait(now), Duration::from_secs(60), "Retry starts the back-off over");
    }
}
