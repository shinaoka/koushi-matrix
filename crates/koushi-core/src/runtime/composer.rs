//! Composer-draft lifecycle ownership for [`super::AppActor`].

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::AppActor;
use crate::composer_draft_lifecycle::{
    ComposerDraftCommandPermit, ComposerDraftPersistencePermit, ForwardedComposerDraftPermit,
};
use crate::executor;
use crate::store::{
    composer_drafts::{
        PersistedComposerDraftStoreV3, persisted_projection as persisted_composer_draft_projection,
    },
    session_key_id_from_info,
};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticLevel, record};
use koushi_protocol::command::TimelineCommand;
use koushi_protocol::ids::{RequestId, TimelineKey, TimelineKind};
use koushi_state::{
    AppAction, AppState, ComposerDraftProtection, ComposerDraftRevision, ComposerDraftStore,
    ComposerTarget, SessionState, SubmissionId, ThreadPaneState, reduce,
};

pub const COMPOSER_DRAFT_PERSIST_DEBOUNCE: Duration = Duration::from_millis(150);

pub(super) fn composer_draft_account_matches(
    state: &AppState,
    expected_account: &koushi_protocol::SessionKeyId,
) -> bool {
    matches!(
        &state.session,
        SessionState::Ready(info) if session_key_id_from_info(info) == *expected_account
    )
}

fn composer_draft_revision_for_target(
    state: &AppState,
    target: &ComposerTarget,
) -> ComposerDraftRevision {
    match target {
        ComposerTarget::Main { room_id } => state.composer_drafts.room_revision(room_id),
        ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_revision(room_id, root_event_id),
    }
}

pub(super) fn active_composer_targets(state: &AppState) -> BTreeSet<ComposerTarget> {
    let mut active = BTreeSet::new();
    if let Some(room_id) = &state.timeline.room_id {
        active.insert(ComposerTarget::Main {
            room_id: room_id.clone(),
        });
    }
    match &state.thread {
        ThreadPaneState::Opening {
            room_id,
            root_event_id,
            ..
        }
        | ThreadPaneState::Open {
            room_id,
            root_event_id,
            ..
        } => {
            active.insert(ComposerTarget::Thread {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            });
        }
        ThreadPaneState::Closed => {}
    }
    active
}

pub(super) fn composer_draft_acceptance_would_exhaust(
    state: &AppState,
    target: &ComposerTarget,
    submitted_revision: ComposerDraftRevision,
) -> bool {
    ComposerDraftRevision::checked_successor(
        composer_draft_revision_for_target(state, target),
        submitted_revision,
    )
    .is_err()
}

pub(super) fn timeline_submission_revision_exhaustion(
    state: &AppState,
    command: &TimelineCommand,
) -> Option<(RequestId, TimelineKey, SubmissionId)> {
    let (request_id, submission_id, key, submitted_revision) = match command {
        TimelineCommand::SubmitText {
            request_id,
            submission_id,
            key,
            draft_revision,
            ..
        }
        | TimelineCommand::SubmitReply {
            request_id,
            submission_id,
            key,
            draft_revision,
            ..
        } => (*request_id, submission_id, key, *draft_revision),
        _ => return None,
    };
    let target = match &key.kind {
        TimelineKind::Room { room_id } => ComposerTarget::Main {
            room_id: room_id.clone(),
        },
        TimelineKind::Thread {
            room_id,
            root_event_id,
        } => ComposerTarget::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
        TimelineKind::Focused { .. } => return None,
    };
    composer_draft_acceptance_would_exhaust(state, &target, submitted_revision)
        .then(|| (request_id, key.clone(), submission_id.clone()))
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub(super) enum ComposerAcceptanceIdentity {
    Submission(SubmissionId),
    ScheduledSend(String),
}

pub(super) struct PendingComposerAcceptance {
    pub(super) identity: ComposerAcceptanceIdentity,
    _permit: ComposerDraftCommandPermit,
}

pub(super) enum ComposerDraftLoadStatus {
    Unloaded,
    Loaded(koushi_protocol::SessionKeyId),
    Failed(koushi_protocol::SessionKeyId),
}

pub(super) struct PendingComposerDraftPersist {
    key_id: koushi_protocol::SessionKeyId,
    drafts: PersistedComposerDraftStoreV3,
    permits: Vec<ComposerDraftPersistencePermit>,
    deadline: Instant,
}

#[cfg(test)]
impl PendingComposerDraftPersist {
    pub(super) fn for_shutdown_test(key_id: koushi_protocol::SessionKeyId) -> Self {
        Self {
            key_id,
            drafts: persisted_composer_draft_projection(&Default::default(), &Default::default()),
            permits: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(3600),
        }
    }
}

impl AppActor {
    fn reconcile_composer_draft_lifecycle(&mut self) {
        self.reconcile_composer_draft_lifecycle_with_active(active_composer_targets(&self.state));
    }
    pub(super) async fn reconcile_composer_draft_lifecycle_after_permit_change(&mut self) -> bool {
        self.composer_draft_lease_changes.borrow_and_update();
        let previous_drafts = self.state.composer_drafts.clone();
        self.reconcile_composer_draft_lifecycle();
        if previous_drafts == self.state.composer_drafts {
            return false;
        }
        if let Some(key_id) = composer_draft_session_key(&self.state) {
            self.schedule_composer_draft_persist(key_id, self.state.composer_drafts.clone())
                .await;
        }
        true
    }
    pub(super) fn reconcile_composer_draft_lifecycle_with_active(
        &mut self,
        active: BTreeSet<ComposerTarget>,
    ) {
        let protection = if let Some(account) = composer_draft_session_key(&self.state) {
            ComposerDraftProtection {
                active,
                leased: self.composer_draft_leases.touch_protected_targets(&account),
                store_pending: self
                    .composer_draft_leases
                    .persistence_held_targets_excluding(&account, &[]),
            }
        } else {
            ComposerDraftProtection {
                active,
                ..ComposerDraftProtection::default()
            }
        };
        self.state.composer_drafts.reconcile_lifecycle(&protection);
    }
    fn composer_draft_persistence_protection(
        &self,
        key_id: &koushi_protocol::SessionKeyId,
        active: BTreeSet<ComposerTarget>,
    ) -> ComposerDraftProtection {
        let excluded_permits = self
            .pending_composer_draft_persist
            .as_ref()
            .filter(|pending| pending.key_id == *key_id)
            .map(|pending| pending.permits.as_slice())
            .unwrap_or_default();
        ComposerDraftProtection {
            active,
            leased: self.composer_draft_leases.touch_protected_targets(key_id),
            store_pending: self
                .composer_draft_leases
                .persistence_held_targets_excluding(key_id, excluded_permits),
        }
    }
    pub(super) fn forward_composer_draft_permit(
        &mut self,
        request_id: RequestId,
        identity: ComposerAcceptanceIdentity,
        permit: ComposerDraftCommandPermit,
    ) -> ForwardedComposerDraftPermit {
        self.pending_composer_acceptances.insert(
            request_id,
            PendingComposerAcceptance {
                identity,
                _permit: permit.clone(),
            },
        );
        ForwardedComposerDraftPermit::new(
            request_id,
            permit,
            self.composer_draft_rejected_tx.clone(),
        )
    }
    pub(super) async fn load_composer_drafts_for_current_session(&mut self) {
        let Some(key_id) = composer_draft_session_key(&self.state) else {
            self.composer_draft_load_status = ComposerDraftLoadStatus::Unloaded;
            self.composer_draft_reload_required = false;
            return;
        };
        if matches!(
            &self.composer_draft_load_status,
            ComposerDraftLoadStatus::Loaded(settled_key)
                | ComposerDraftLoadStatus::Failed(settled_key)
                if settled_key == &key_id
        ) && !self.composer_draft_reload_required
        {
            return;
        }
        // A session transition may leave the same account key (lock/unlock).
        // Flush the captured old draft before reloading it; selection state was
        // already published by the action loop before this post-commit work.
        self.flush_pending_composer_drafts().await;

        let store = self.composer_draft_store_actor.clone();
        let load_key_id = key_id.clone();
        let drafts = match executor::spawn_blocking(move || {
            store.load_composer_drafts(&load_key_id)
        })
        .await
        {
            Ok(Ok(drafts)) => drafts,
            Ok(Err(_)) | Err(_) => {
                self.composer_draft_load_status = ComposerDraftLoadStatus::Failed(key_id);
                self.composer_draft_reload_required = false;
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Error,
                    "core.composer_draft",
                    "load_failed",
                ));
                #[cfg(any(test, feature = "test-hooks"))]
                self.composer_draft_store_actor
                    .notify_composer_draft_load_completed_for_testing();
                return;
            }
        };
        let effects = reduce(&mut self.state, AppAction::ComposerDraftsLoaded { drafts });
        self.composer_draft_load_status = ComposerDraftLoadStatus::Loaded(key_id);
        self.composer_draft_reload_required = false;
        self.handle_ui_event_effects(&effects).await;
        #[cfg(any(test, feature = "test-hooks"))]
        self.composer_draft_store_actor
            .notify_composer_draft_load_completed_for_testing();
    }
    pub(super) async fn schedule_composer_draft_persist(
        &mut self,
        key_id: koushi_protocol::SessionKeyId,
        drafts: ComposerDraftStore,
    ) {
        if !matches!(
            &self.composer_draft_load_status,
            ComposerDraftLoadStatus::Loaded(loaded_key) if loaded_key == &key_id
        ) {
            return;
        }
        if self
            .pending_composer_draft_persist
            .as_ref()
            .is_some_and(|pending| pending.key_id != key_id)
        {
            self.flush_pending_composer_drafts().await;
        }
        let protection = self.composer_draft_persistence_protection(
            &key_id,
            if composer_draft_session_key(&self.state).as_ref() == Some(&key_id) {
                active_composer_targets(&self.state)
            } else {
                BTreeSet::new()
            },
        );
        let drafts = persisted_composer_draft_projection(&drafts, &protection);
        let Ok(permits) = self
            .composer_draft_leases
            .persistence_permits(&key_id, drafts.targets())
        else {
            record(DiagnosticEvent::new(
                DiagnosticLevel::Error,
                "core.composer_draft",
                "persistence_permit_exhausted",
            ));
            return;
        };
        self.pending_composer_draft_persist = Some(PendingComposerDraftPersist {
            key_id,
            drafts,
            permits,
            deadline: Instant::now() + COMPOSER_DRAFT_PERSIST_DEBOUNCE,
        });
    }
    pub(super) fn composer_draft_persist_delay(&self) -> Option<Duration> {
        self.pending_composer_draft_persist
            .as_ref()
            .map(|pending| pending.deadline.saturating_duration_since(Instant::now()))
    }
    pub(super) async fn flush_pending_composer_drafts(&mut self) -> bool {
        let Some(pending) = self.pending_composer_draft_persist.take() else {
            return true;
        };
        let store = self.composer_draft_store_actor.clone();
        let PendingComposerDraftPersist {
            key_id,
            drafts,
            permits,
            deadline: _,
        } = pending;
        matches!(
            executor::spawn_blocking(move || {
                let _permits = permits;
                store.save_composer_drafts(&key_id, &drafts)
            })
            .await,
            Ok(Ok(()))
        )
    }
}

pub(super) fn composer_draft_session_key(
    state: &AppState,
) -> Option<koushi_protocol::SessionKeyId> {
    match &state.session {
        SessionState::Ready(info) => Some(session_key_id_from_info(info)),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::Provisional { .. }
        | SessionState::AwaitingVerification { .. }
        | SessionState::Verifying { .. }
        | SessionState::AwaitingBootstrapConfirmation { .. }
        | SessionState::Rejecting { .. }
        | SessionState::LoggingOut
        | SessionState::CapabilityBlocked { .. }
        | SessionState::Locked(_) => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerDraftTransitionPolicy {
    Normal,
    PreservePrevious,
    Discard,
}

pub(super) fn composer_draft_transition_policy(
    action: &AppAction,
) -> ComposerDraftTransitionPolicy {
    match action {
        AppAction::SessionLocked
        | AppAction::SessionAuthenticationInvalidated { .. }
        | AppAction::SwitchAccountRequested { .. } => {
            ComposerDraftTransitionPolicy::PreservePrevious
        }
        AppAction::LogoutRequested
        | AppAction::LogoutFinished
        | AppAction::ResetLocalDataRequested { .. }
        | AppAction::ResetLocalDataCompleted { .. } => ComposerDraftTransitionPolicy::Discard,
        _ => ComposerDraftTransitionPolicy::Normal,
    }
}

pub(super) fn composer_acceptance_identity_for_timeline_command(
    command: &TimelineCommand,
) -> Option<ComposerAcceptanceIdentity> {
    match command {
        TimelineCommand::SubmitText { submission_id, .. }
        | TimelineCommand::SubmitReply { submission_id, .. } => Some(
            ComposerAcceptanceIdentity::Submission(submission_id.clone()),
        ),
        _ => None,
    }
}

pub(super) fn composer_acceptance_identity_for_action(
    action: &AppAction,
) -> Option<ComposerAcceptanceIdentity> {
    match action {
        AppAction::ComposerSubmissionAcceptedAtRevision { submission_id, .. }
        | AppAction::ThreadSubmissionAcceptedAtRevision { submission_id, .. } => Some(
            ComposerAcceptanceIdentity::Submission(submission_id.clone()),
        ),
        AppAction::ScheduledSendCreatedAtRevision { item, .. } => Some(
            ComposerAcceptanceIdentity::ScheduledSend(item.scheduled_id.clone()),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_protocol::ids::{AccountKey, RuntimeConnectionId};
    use koushi_state::SessionInfo;

    #[test]

    fn destructive_composer_draft_clear_does_not_schedule_resurrection() {
        assert_eq!(
            composer_draft_transition_policy(&AppAction::LogoutRequested),
            ComposerDraftTransitionPolicy::Discard
        );

        assert_eq!(
            composer_draft_transition_policy(&AppAction::LogoutFinished),
            ComposerDraftTransitionPolicy::Discard
        );

        assert_eq!(
            composer_draft_transition_policy(&AppAction::ResetLocalDataRequested { request_id: 1 }),
            ComposerDraftTransitionPolicy::Discard
        );

        assert_eq!(
            composer_draft_transition_policy(&AppAction::ResetLocalDataCompleted { request_id: 1 }),
            ComposerDraftTransitionPolicy::Discard
        );

        assert_eq!(
            composer_draft_transition_policy(&AppAction::SessionLocked),
            ComposerDraftTransitionPolicy::PreservePrevious
        );

        assert_eq!(
            composer_draft_transition_policy(&AppAction::SessionAuthenticationInvalidated {
                soft_logout: true,
            }),
            ComposerDraftTransitionPolicy::PreservePrevious
        );

        assert_eq!(
            composer_draft_transition_policy(&AppAction::SwitchAccountRequested {
                info: SessionInfo {
                    homeserver: "https://example.invalid".to_owned(),

                    user_id: "@other:example.invalid".to_owned(),

                    device_id: "OTHER".to_owned(),

                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                },
            }),
            ComposerDraftTransitionPolicy::PreservePrevious
        );
    }

    #[test]

    fn composer_revision_exhaustion_is_detected_for_room_and_thread_submissions() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(3),

            sequence: 7,
        };

        let account_key = AccountKey("@qa:example.invalid".to_owned());

        let room_id = "!room:example.invalid".to_owned();

        let root_event_id = "$root:example.invalid".to_owned();

        let mut state = AppState::default();

        state
            .composer_drafts
            .room_revisions
            .insert(room_id.clone(), ComposerDraftRevision::MAX);

        state
            .composer_drafts
            .thread_revisions
            .entry(room_id.clone())
            .or_default()
            .insert(root_event_id.clone(), ComposerDraftRevision::MAX);

        let expected_account = koushi_protocol::SessionKeyId {
            homeserver: "https://example.invalid".to_owned(),

            user_id: "@qa:example.invalid".to_owned(),

            device_id: "DEVICE".to_owned(),
        };

        let room = TimelineCommand::SubmitText {
            request_id,

            expected_account: expected_account.clone(),

            submission_id: SubmissionId::new("room-submission"),

            key: TimelineKey::room(account_key.clone(), room_id.clone()),

            transaction_id: "room-transaction".to_owned(),

            document: koushi_state::ComposerDocument::from_plain_text("body"),

            draft_revision: ComposerDraftRevision::MAX,
        };

        let thread = TimelineCommand::SubmitReply {
            request_id,

            expected_account,

            submission_id: SubmissionId::new("thread-submission"),

            key: TimelineKey {
                account_key,

                kind: TimelineKind::Thread {
                    room_id,

                    root_event_id: root_event_id.clone(),
                },
            },

            transaction_id: "thread-transaction".to_owned(),

            in_reply_to_event_id: root_event_id,

            document: koushi_state::ComposerDocument::from_plain_text("reply"),

            draft_revision: ComposerDraftRevision::MAX,
        };

        assert!(timeline_submission_revision_exhaustion(&state, &room).is_some());

        assert!(timeline_submission_revision_exhaustion(&state, &thread).is_some());
    }

    #[test]

    fn composer_revision_exhaustion_preflight_preserves_authoritative_draft() {
        let target = ComposerTarget::Main {
            room_id: "!room:example.invalid".to_owned(),
        };

        let mut state = AppState::default();

        state
            .composer_drafts
            .rooms
            .insert("!room:example.invalid".to_owned(), "keep me".into());

        state.composer_drafts.room_revisions.insert(
            "!room:example.invalid".to_owned(),
            ComposerDraftRevision::MAX,
        );

        assert!(composer_draft_acceptance_would_exhaust(
            &state,
            &target,
            ComposerDraftRevision::MAX
        ));

        assert_eq!(
            state
                .composer_drafts
                .rooms
                .get("!room:example.invalid")
                .map(koushi_state::ComposerDocument::plain_body),
            Some("keep me".to_owned())
        );

        assert_eq!(
            state
                .composer_drafts
                .room_last_accepted_clear_revisions
                .get("!room:example.invalid"),
            None
        );
    }
}
