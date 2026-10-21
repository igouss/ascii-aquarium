//! When losing focus stops the screensaver.

use std::time::Duration;

/// How long the screensaver may be out of focus (or the session locked)
/// before it stops: while instances launch on several monitors, focus hops
/// between outputs before all have mapped.
pub const GRACE: Duration = Duration::from_millis(1500);

/// How long the screensaver has been absent.
#[derive(Debug, Clone, Default)]
pub struct FocusWatch {
    absent_since: Option<Duration>,
}

impl FocusWatch {
    /// Notes whether the screensaver is `present` at `now`: focused, with the
    /// session unlocked. True once it has been absent for the grace period.
    pub fn observe(&mut self, present: bool, now: Duration) -> bool {
        if present {
            self.absent_since = None;
            return false;
        }
        let since = *self.absent_since.get_or_insert(now);
        now.saturating_sub(since) >= GRACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn absence_counts_once_it_outlasts_the_grace_period() {
        let mut focus = FocusWatch::default();
        assert!(!focus.observe(false, ms(1000)));
        assert!(!focus.observe(false, ms(2499)));
        assert!(focus.observe(false, ms(2500)));
        assert!(focus.observe(false, ms(9000)));
    }

    #[test]
    fn regaining_focus_starts_the_grace_period_over() {
        let mut focus = FocusWatch::default();
        assert!(!focus.observe(false, ms(0)));
        assert!(!focus.observe(true, ms(1400)));
        assert!(!focus.observe(false, ms(1500)));
        assert!(!focus.observe(false, ms(2999)));
        assert!(focus.observe(false, ms(3000)));
    }

    #[test]
    fn presence_is_never_absence() {
        let mut focus = FocusWatch::default();
        for t in 0..10 {
            assert!(!focus.observe(true, ms(t * 1000)));
        }
    }
}
