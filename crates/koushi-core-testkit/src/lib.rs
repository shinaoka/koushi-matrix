//! Shared integration-test package for `koushi-core`.

use std::time::{Duration, Instant};

/// Bounded silence a contract wait tolerates before it fails (#1759).
///
/// A contract wait is causal: only an observed state transition can satisfy it.
/// A loaded machine that keeps publishing states keeps the wait alive, so the
/// failure condition is silence rather than elapsed time.
pub const PROGRESS_WINDOW: Duration = Duration::from_secs(30);

/// Absolute cap for a Core that keeps publishing unrelated progress without
/// ever reaching the asserted state. It is a hang backstop, not a speed budget:
/// a healthy wait settles in milliseconds and never approaches it.
pub const HANG_BACKSTOP: Duration = Duration::from_secs(180);

/// Budget for one state wait: a silence window re-armed by every progress
/// observation, plus a monotonic absolute backstop that bounds the whole wait.
/// Each caller defines what one observation is: a published state event for the
/// event fence, a poll that differed from the previous poll for the polling one.
#[derive(Debug)]
pub struct StateWaitWatchdog {
    window: Duration,
    backstop: Duration,
    started: Instant,
    last_progress: Instant,
    events: u64,
}

impl StateWaitWatchdog {
    /// A watchdog whose budget starts now.
    pub fn start() -> Self {
        Self::start_at(Instant::now(), PROGRESS_WINDOW, HANG_BACKSTOP)
    }

    /// A watchdog that started at `now`, so a caller or a test can age it
    /// without sleeping.
    pub fn start_at(now: Instant, window: Duration, backstop: Duration) -> Self {
        Self {
            window,
            backstop,
            started: now,
            last_progress: now,
            events: 0,
        }
    }

    /// One progress observation in the caller's sense: it counts as progress and
    /// re-arms the window.
    pub fn progress(&mut self) {
        self.events += 1;
        self.last_progress = Instant::now();
    }

    /// Progress observations so far, whether related to the predicate or not.
    pub fn events(&self) -> u64 {
        self.events
    }

    /// Time left before the next failure: the nearer of the silence deadline
    /// and the backstop. `None` once either one has passed.
    pub fn remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        let window_left = self
            .window
            .saturating_sub(now.duration_since(self.last_progress));
        let backstop_left = self
            .backstop
            .saturating_sub(now.duration_since(self.started));
        if window_left.is_zero() || backstop_left.is_zero() {
            return None;
        }
        Some(window_left.min(backstop_left))
    }

    /// Failure diagnostic: which budget expired, how long the wait had been
    /// silent, and how many progress observations arrived. `progress` names what
    /// counts as one for the caller's wait, because an event-driven fence counts
    /// published state events while a polling one counts differing polls.
    pub fn report(&self, progress: &str) -> String {
        let now = Instant::now();
        let silence = now.duration_since(self.last_progress);
        let elapsed = now.duration_since(self.started);
        // Whichever deadline came first ended the wait, so a late wake must not
        // relabel a silence failure as a backstop failure.
        let cause = if self.started + self.backstop < self.last_progress + self.window {
            "backstop"
        } else {
            "silence"
        };
        format!(
            "{cause} for {silence:?} after {} {progress} in {elapsed:?} (window {:?}, backstop {:?})",
            self.events, self.window, self.backstop,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A watchdog that started `since_start` ago, so no test has to sleep.
    fn aged(since_start: Duration, window: Duration, backstop: Duration) -> StateWaitWatchdog {
        StateWaitWatchdog::start_at(Instant::now() - since_start, window, backstop)
    }

    #[test]
    fn a_silent_wait_fails_before_the_backstop_expires() {
        let watchdog = aged(
            Duration::from_secs(6),
            Duration::from_secs(5),
            Duration::from_secs(60),
        );
        assert_eq!(watchdog.remaining(), None);
        let report = watchdog.report("state events");
        assert!(report.starts_with("silence for"), "{report}");
        assert!(report.contains("after 0 state events"), "{report}");
    }

    #[test]
    fn an_observed_state_event_re_arms_a_wait_that_outlived_the_window() {
        let mut watchdog = aged(
            Duration::from_secs(50),
            Duration::from_secs(5),
            Duration::from_secs(60),
        );
        assert_eq!(
            watchdog.remaining(),
            None,
            "50s of silence exceeds the 5s window"
        );
        watchdog.progress();
        let left = watchdog.remaining().expect("progress re-arms the wait");
        assert!(left > Duration::from_secs(4), "{left:?}");
    }

    #[test]
    fn the_backstop_still_ends_a_wait_that_keeps_receiving_progress() {
        let mut watchdog = aged(
            Duration::from_secs(61),
            Duration::from_secs(5),
            Duration::from_secs(60),
        );
        watchdog.progress();
        assert_eq!(watchdog.remaining(), None);
        let report = watchdog.report("state events");
        assert!(report.starts_with("backstop for"), "{report}");
        assert!(report.contains("after 1 state events"), "{report}");
    }

    #[test]
    fn a_late_wake_reports_the_deadline_that_came_first() {
        let watchdog = aged(
            Duration::from_secs(12),
            Duration::from_secs(2),
            Duration::from_secs(10),
        );
        // Elapsed time already exceeds the backstop, but silence expired 8 s
        // before it, so silence is what ended the wait.
        assert_eq!(watchdog.remaining(), None);
        assert!(watchdog.report("state events").starts_with("silence for"));
    }
}
