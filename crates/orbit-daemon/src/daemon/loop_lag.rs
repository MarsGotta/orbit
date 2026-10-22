//! How late the daemon's main loop runs, measured on its one-second tick.
use std::time::{Duration, Instant};

/// How much later than its period the main loop's tick may come before the
/// daemon logs `daemon_loop_stalled`. What it protects: the next stall is seen
/// in the log with its length, instead of being inferred from missing lines
/// (on 2026-10-22 the daemon wrote nothing for 11.3 s during a meeting). Why
/// 2 s: it is the helper's beat period, so a hold that long is one the app
/// and the helper can notice; shorter holds are routine (a disk flush, a busy
/// turn), and each logged line costs a disk flush of its own. It stops and
/// delays nothing: it only writes one line per stall.
pub(super) const LOOP_STALL_LOG_THRESHOLD: Duration = Duration::from_secs(2);

/// The main loop's last tick, to tell a late one.
#[derive(Debug, Default)]
pub(super) struct LoopLag {
    previous: Option<Instant>,
}

impl LoopLag {
    /// The tick at `now` came this much later than `period` after the last
    /// one, when that is more than [`LOOP_STALL_LOG_THRESHOLD`].
    pub(super) fn observe(&mut self, now: Instant, period: Duration) -> Option<Duration> {
        let previous = self.previous.replace(now)?;
        let late = now
            .saturating_duration_since(previous)
            .checked_sub(period)?;
        (late > LOOP_STALL_LOG_THRESHOLD).then_some(late)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: Duration = Duration::from_secs(1);

    #[test]
    fn an_on_time_loop_reports_nothing() {
        let start = Instant::now();
        let mut lag = LoopLag::default();
        for second in 0..10 {
            assert_eq!(
                lag.observe(start + Duration::from_secs(second), PERIOD),
                None
            );
        }
    }

    #[test]
    fn the_first_tick_has_nothing_to_compare_with() {
        let mut lag = LoopLag::default();
        assert_eq!(lag.observe(Instant::now(), PERIOD), None);
    }

    #[test]
    fn a_held_loop_reports_how_late_its_tick_came_once() {
        let start = Instant::now();
        let mut lag = LoopLag::default();
        assert_eq!(lag.observe(start, PERIOD), None);
        // Held for 11.3 s: the tick comes 10.3 s later than its period.
        let late = start + Duration::from_millis(11_300);
        assert_eq!(
            lag.observe(late, PERIOD),
            Some(Duration::from_millis(10_300))
        );
        // The ticks missed meanwhile fire back to back: nothing more to say.
        assert_eq!(lag.observe(late + Duration::from_millis(1), PERIOD), None);
        assert_eq!(lag.observe(late + Duration::from_secs(1), PERIOD), None);
    }

    #[test]
    fn a_hold_up_to_the_threshold_is_not_logged() {
        let start = Instant::now();
        let mut lag = LoopLag::default();
        assert_eq!(lag.observe(start, PERIOD), None);
        let at_threshold = start + PERIOD + LOOP_STALL_LOG_THRESHOLD;
        assert_eq!(lag.observe(at_threshold, PERIOD), None);
        let past = at_threshold + PERIOD + LOOP_STALL_LOG_THRESHOLD + Duration::from_millis(1);
        assert_eq!(
            lag.observe(past, PERIOD),
            Some(LOOP_STALL_LOG_THRESHOLD + Duration::from_millis(1))
        );
    }
}
