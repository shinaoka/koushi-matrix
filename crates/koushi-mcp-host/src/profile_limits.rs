//! The subscription's tunables, in one place (#2132, design section 0).
//!
//! These are derived by judgement, not measurement: nobody has profiled a real
//! agent harness consuming wake rows, and the design says so. Each value has its
//! reason next to it. They are not API and not settings; `Limits` exists so a
//! test can shrink the timeouts instead of sleeping for a minute.

use std::time::Duration;

/// Most rows sent and not yet acknowledged per subscription. A starting value,
/// not derived: large enough to cover a burst of mentions in one sync, small
/// enough that a stalled consumer pins little and redelivery after a resync is
/// cheap.
pub(crate) const PUSH_WINDOW: usize = 64;

/// Frames the writer task may queue. Twice `PUSH_WINDOW`, so a full window plus
/// the responses to requests never blocks the reader loop.
pub(crate) const WRITER_CHANNEL: usize = 128;

/// How long a full window may go without an `ack` before the subscription ends
/// `ack_stalled`. Longer than a slow model turn and a harness restart, shorter
/// than the time a missing wake-up would go unnoticed.
pub(crate) const ACK_STALL: Duration = Duration::from_secs(60);

/// How long one frame may take to write before the peer is considered not
/// reading. A local socket write that blocks this long means exactly that.
pub(crate) const WRITE_STALL: Duration = Duration::from_secs(15);

/// Bounds acknowledged-cursor writes; a crash redelivers at most this much.
pub(crate) const ACK_FLUSH: Duration = Duration::from_secs(1);

/// How often an idle pump re-checks that the account, the feed and the read
/// grant still hold, since a quiet feed gives it no other reason to wake.
pub(crate) const RECHECK: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub(crate) push_window: usize,
    pub(crate) ack_stall: Duration,
    pub(crate) write_stall: Duration,
    pub(crate) ack_flush: Duration,
    pub(crate) recheck: Duration,
}

impl Limits {
    pub(crate) const DEFAULT: Self = Self {
        push_window: PUSH_WINDOW,
        ack_stall: ACK_STALL,
        write_stall: WRITE_STALL,
        ack_flush: ACK_FLUSH,
        recheck: RECHECK,
    };
}

/// Overrides for tests, which cannot wait a minute for a stall.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct TestLimits {
    pub push_window: usize,
    pub ack_stall: Duration,
    pub write_stall: Duration,
    pub recheck: Duration,
}

impl From<TestLimits> for Limits {
    fn from(test: TestLimits) -> Self {
        Self {
            push_window: test.push_window,
            ack_stall: test.ack_stall,
            write_stall: test.write_stall,
            ack_flush: ACK_FLUSH,
            recheck: test.recheck,
        }
    }
}
