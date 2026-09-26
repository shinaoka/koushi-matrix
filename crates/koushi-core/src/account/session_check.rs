//! #1009: AccountActor side of current-session verification checks.
//!
//! The reducer decides *when* a full inspection is admitted; this module owns
//! the clock, the single due-time timer, the trust-recheck failure backoff, and
//! the coordination that keeps the full inspection and the authoritative trust
//! recheck from issuing duplicate own-identity queries.

use std::time::Duration;

use koushi_state::{AppAction, SESSION_STATUS_FRESHNESS_MS, session_status_failure_backoff_ms};

use crate::executor;
use crate::session_check_diagnostics::{TrustRecheckOutcome, record_trust_recheck};

use super::actor::{AccountActor, AccountMessage};

/// The timer re-reads the wall clock at least this often, so a machine that
/// suspends (monotonic time stops) re-evaluates the due time soon after wake.
const WALL_CLOCK_POLL: Duration = Duration::from_secs(60);

/// Wall clock used for due times and check timestamps. Tests switch the actor
/// to a virtual clock driven by (paused) tokio time.
#[derive(Clone, Debug, Default)]
pub(super) enum SessionCheckClock {
    #[default]
    Wall,
    #[cfg(test)]
    Virtual {
        base_epoch_ms: u64,
        origin: executor::Instant,
    },
}

impl SessionCheckClock {
    pub(super) fn now_ms(&self) -> u64 {
        match self {
            Self::Wall => crate::time::current_epoch_ms(),
            #[cfg(test)]
            Self::Virtual {
                base_epoch_ms,
                origin,
            } => base_epoch_ms
                .saturating_add(u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)),
        }
    }
}

/// A full inspection admitted while an authoritative trust recheck was in
/// flight; it starts once the recheck settles.
#[derive(Clone, Copy, Debug)]
pub(super) struct WaitingInspection {
    pub(super) request_id: u64,
    pub(super) generation: u64,
    pub(super) sync_state: koushi_state::CurrentSessionSyncState,
}

#[derive(Debug, Default)]
pub(super) struct SessionCheckCoordinator {
    pub(super) clock: SessionCheckClock,
    pub(super) timer: Option<executor::JoinHandle<()>>,
    /// Consecutive failed authoritative rechecks on a promoted session.
    pub(super) trust_failures: u32,
    pub(super) trust_failed_at_ms: u64,
    pub(super) trust_retry: Option<executor::JoinHandle<()>>,
    pub(super) trust_retry_serial: u64,
    /// A trust-recheck demand joined the in-flight full inspection.
    pub(super) trust_joined_inspection: bool,
    pub(super) waiting_inspection: Option<WaitingInspection>,
}

impl SessionCheckCoordinator {
    /// Whether a new authoritative recheck must wait for the failure backoff.
    pub(super) fn trust_backoff_due_at_ms(&self, now_ms: u64) -> Option<u64> {
        if self.trust_failures == 0 {
            return None;
        }
        let due = self
            .trust_failed_at_ms
            .saturating_add(session_status_failure_backoff_ms(self.trust_failures));
        // A clock behind the recorded failure is unreliable: do not wait on it.
        (now_ms >= self.trust_failed_at_ms && now_ms < due).then_some(due)
    }

    pub(super) fn retire(&mut self) {
        if let Some(task) = self.timer.take() {
            task.abort();
        }
        if let Some(task) = self.trust_retry.take() {
            task.abort();
        }
        self.trust_retry_serial = self.trust_retry_serial.wrapping_add(1);
        self.trust_failures = 0;
        self.trust_failed_at_ms = 0;
        self.trust_joined_inspection = false;
        self.waiting_inspection = None;
    }
}

/// Sleep until the wall clock reaches `due_at_ms`, or until the delay computed
/// at arm time elapses on the monotonic clock, whichever comes first. The
/// monotonic bound (at most one freshness period) keeps a wall clock that
/// jumped backwards from postponing the notification indefinitely; the reducer
/// then decides whether the state is actually due.
pub(super) async fn sleep_until_due(clock: &SessionCheckClock, due_at_ms: u64) {
    let armed_at = executor::Instant::now();
    let delay = Duration::from_millis(
        due_at_ms
            .saturating_sub(clock.now_ms())
            .min(SESSION_STATUS_FRESHNESS_MS),
    );
    loop {
        let elapsed = armed_at.elapsed();
        let now_ms = clock.now_ms();
        if now_ms >= due_at_ms || elapsed >= delay {
            return;
        }
        let step = (delay - elapsed)
            .min(Duration::from_millis(due_at_ms - now_ms))
            .min(WALL_CLOCK_POLL);
        executor::sleep(step).await;
    }
}

impl AccountActor {
    pub(super) fn session_check_now_ms(&self) -> u64 {
        self.session_check.clock.now_ms()
    }

    /// Replace the single session-status timer (reducer
    /// `ArmCurrentSessionStatusCheck`).
    pub(super) fn arm_current_session_status_timer(&mut self, token: u64, due_at_ms: u64) {
        if let Some(task) = self.session_check.timer.take() {
            task.abort();
        }
        let clock = self.session_check.clock.clone();
        let tx = self.self_tx.clone();
        self.session_check.timer = Some(executor::spawn(async move {
            sleep_until_due(&clock, due_at_ms).await;
            let _ = tx
                .send(AccountMessage::CurrentSessionStatusTimerFired { token })
                .await;
        }));
    }

    pub(super) async fn handle_current_session_status_timer_fired(&mut self, token: u64) {
        self.session_check.timer = None;
        let now_ms = self.session_check_now_ms();
        self.send_actions(vec![AppAction::CurrentSessionStatusCheckDue {
            token,
            now_ms,
        }])
        .await;
    }

    /// Apply the promoted-session trust-recheck policy before a query starts.
    /// Returns `true` when the demand was absorbed (joined or deferred) and no
    /// standalone query may start now.
    pub(super) fn absorb_promoted_trust_recheck(&mut self) -> bool {
        if !self.session_promoted {
            // Initial admission and interactive verification are never delayed.
            return false;
        }
        if self.session_check.trust_joined_inspection {
            return true;
        }
        if self.current_session_status_task.is_some()
            && self.current_session_status_request.is_some()
            && self.session_check.waiting_inspection.is_none()
        {
            // The in-flight full inspection queries the own identity and reads
            // the same verification subscriber; its result settles this demand.
            self.session_check.trust_joined_inspection = true;
            record_trust_recheck(TrustRecheckOutcome::Joined);
            return true;
        }
        let now_ms = self.session_check_now_ms();
        if let Some(due_at_ms) = self.session_check.trust_backoff_due_at_ms(now_ms) {
            record_trust_recheck(TrustRecheckOutcome::Deferred);
            if self.session_check.trust_retry.is_none() {
                self.arm_trust_recheck_retry(due_at_ms);
            }
            return true;
        }
        false
    }

    fn arm_trust_recheck_retry(&mut self, due_at_ms: u64) {
        if let Some(task) = self.session_check.trust_retry.take() {
            task.abort();
        }
        self.session_check.trust_retry_serial =
            self.session_check.trust_retry_serial.wrapping_add(1);
        let serial = self.session_check.trust_retry_serial;
        let clock = self.session_check.clock.clone();
        let tx = self.self_tx.clone();
        self.session_check.trust_retry = Some(executor::spawn(async move {
            sleep_until_due(&clock, due_at_ms).await;
            let _ = tx
                .send(AccountMessage::TrustRecheckRetryDue { serial })
                .await;
        }));
    }

    /// Record a settled authoritative recheck for the backoff. A failure on a
    /// promoted session keeps the demand pending and arms a retry at the
    /// capped backoff, so repeated failures never stop rechecks forever.
    pub(super) fn record_trust_recheck_settlement(&mut self, succeeded: bool) {
        record_trust_recheck(if succeeded {
            TrustRecheckOutcome::Succeeded
        } else {
            TrustRecheckOutcome::Failed
        });
        if succeeded {
            self.session_check.trust_failures = 0;
            return;
        }
        if !self.session_promoted {
            return;
        }
        self.session_check.trust_failures = self.session_check.trust_failures.saturating_add(1);
        self.session_check.trust_failed_at_ms = self.session_check_now_ms();
        self.trust_recheck_pending = true;
        let due_at_ms = self.session_check.trust_failed_at_ms.saturating_add(
            session_status_failure_backoff_ms(self.session_check.trust_failures),
        );
        self.arm_trust_recheck_retry(due_at_ms);
    }

    pub(super) fn handle_trust_recheck_retry_due(&mut self, serial: u64) {
        if serial != self.session_check.trust_retry_serial {
            return;
        }
        self.session_check.trust_retry = None;
        if !self.session_promoted || !self.trust_recheck_pending {
            return;
        }
        if !self.sync_connectivity_proven {
            // The next proven edge runs the pending recheck once.
            return;
        }
        if self.session.as_ref().is_some_and(|session| {
            session.current_device_trust() == koushi_state::CurrentDeviceTrustState::Verified
        }) {
            // The SDK already observes Verified; nothing to recover.
            self.trust_recheck_pending = false;
            return;
        }
        self.start_authoritative_trust_recheck_if_idle(false);
    }

    /// A joined trust demand whose inspection did not settle it runs on its
    /// own (subject to connectivity and backoff).
    pub(super) fn release_joined_trust_recheck(&mut self) {
        if !std::mem::take(&mut self.session_check.trust_joined_inspection) {
            return;
        }
        self.trust_recheck_pending = true;
        if self.sync_connectivity_proven {
            self.start_authoritative_trust_recheck_if_idle(false);
        }
    }
}

#[cfg(test)]
mod tests;
