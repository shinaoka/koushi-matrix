//! #1505: the own-session check.
//!
//! Canon: `docs/architecture/state-machine.md`, "Own-session check". A
//! promoted session with proven sync connectivity arms one timer for the
//! account's next due check: at once for a session never checked (a new
//! login), otherwise 24 hours after the persisted last check, so restarts do
//! not reset the interval. Device-list changes never trigger a check. The
//! reducer turns a settled inventory into the rendered list and alert.

use koushi_protocol::SessionKeyId;
use koushi_state::{AppAction, OWN_SESSION_CHECK_RETRY_MS, own_session_check_due_at_ms};

use crate::executor;

use super::actor::{AccountActor, AccountMessage};
use super::session_check::sleep_until_due;

#[derive(Debug, Default)]
pub(super) struct OwnSessionCheck {
    /// Advanced on every arm and retire, so a stale timer or a check of a
    /// replaced session is inert.
    pub(super) generation: u64,
    /// The session this schedule belongs to; `None` while unarmed.
    pub(super) key_id: Option<SessionKeyId>,
    pub(super) timer: Option<executor::JoinHandle<()>>,
    pub(super) task: Option<executor::JoinHandle<()>>,
}

impl AccountActor {
    /// Arm the check for the promoted session once its sync connectivity is
    /// proven. Idempotent for a session already armed: a reconnect or a second
    /// promotion edge keeps the existing due time.
    pub(super) fn start_own_session_check_schedule(&mut self) {
        if !self.session_promoted || !self.sync_connectivity_proven || self.session.is_none() {
            return;
        }
        let Some(key_id) = self.session_key_id.clone() else {
            return;
        };
        if self.own_session_check.key_id.as_ref() == Some(&key_id) {
            return;
        }
        let last_checked_at_ms = self.store.load_own_session_last_checked_at(&key_id);
        self.own_session_check.key_id = Some(key_id);
        self.arm_own_session_check(own_session_check_due_at_ms(last_checked_at_ms));
    }

    fn arm_own_session_check(&mut self, due_at_ms: u64) {
        if let Some(task) = self.own_session_check.timer.take() {
            task.abort();
        }
        self.own_session_check.generation = self.own_session_check.generation.wrapping_add(1);
        let generation = self.own_session_check.generation;
        let clock = self.session_check.clock.clone();
        let tx = self.self_tx.clone();
        self.own_session_check.timer = Some(executor::spawn(async move {
            sleep_until_due(&clock, due_at_ms).await;
            let _ = tx
                .send(AccountMessage::OwnSessionCheckDue { generation })
                .await;
        }));
    }

    pub(super) fn handle_own_session_check_due(&mut self, generation: u64) {
        if generation != self.own_session_check.generation {
            return;
        }
        self.own_session_check.timer = None;
        let Some(session) = self.session.clone() else {
            return;
        };
        if !self.session_promoted || self.own_session_check.task.is_some() {
            return;
        }
        let tx = self.self_tx.clone();
        self.own_session_check.task = Some(executor::spawn(async move {
            let result = koushi_sdk::inspect_own_sessions(&session).await;
            let _ = tx
                .send(AccountMessage::OwnSessionCheckFinished { generation, result })
                .await;
        }));
    }

    pub(super) async fn handle_own_session_check_finished(
        &mut self,
        generation: u64,
        result: Result<koushi_sdk::OwnSessionsInventory, koushi_sdk::OwnSessionsInventoryError>,
    ) {
        if generation != self.own_session_check.generation {
            return;
        }
        self.own_session_check.task = None;
        let Some(key_id) = self.own_session_check.key_id.clone() else {
            return;
        };
        let now_ms = self.session_check_now_ms();
        match result {
            Ok(inventory) => {
                // A failed write costs one extra check after a restart, never
                // a missed one.
                let _ = self.store.save_own_session_last_checked_at(&key_id, now_ms);
                self.arm_own_session_check(own_session_check_due_at_ms(Some(now_ms)));
                self.send_actions(vec![AppAction::OwnSessionsChecked {
                    devices: inventory.devices,
                    current_session_verified: inventory.current_session_verified,
                }])
                .await;
            }
            Err(koushi_sdk::OwnSessionsInventoryError) => {
                self.arm_own_session_check(now_ms.saturating_add(OWN_SESSION_CHECK_RETRY_MS));
            }
        }
    }

    pub(super) fn retire_own_session_check(&mut self) {
        if let Some(task) = self.own_session_check.timer.take() {
            task.abort();
        }
        if let Some(task) = self.own_session_check.task.take() {
            task.abort();
        }
        self.own_session_check.key_id = None;
        self.own_session_check.generation = self.own_session_check.generation.wrapping_add(1);
    }
}
