use std::collections::BTreeSet;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    ActivityState, AppAction, AppEffect, AppState, ComposerDraftStore, NavigationState, reduce,
};

use super::composer::{
    ComposerDraftTransitionPolicy, active_composer_targets, composer_draft_session_key,
    composer_draft_transition_policy,
};
use super::navigation::{
    NavigationPersistenceStatus, event_navigation_owner_cleanup_required,
    is_internal_event_navigation_select, navigation_session_key,
};
use super::profile_display_diagnostics::{
    live_receipt_profile_diagnostic_event, profile_resolution_diagnostic_event,
    record_native_attention_recomputed,
};
use super::scheduled_send::{DeferredScheduledSendPersist, scheduled_send_session_key};

use crate::account::AccountMessage;
use crate::unread_trace;

fn reduce_with_unread_diagnostics(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    let room_list_trace = match &action {
        AppAction::RoomListUpdated { rooms, .. }
        | AppAction::RoomListSnapshotProvisional { rooms, .. }
        | AppAction::RoomListSnapshotAuthoritative { rooms, .. } => Some(
            unread_trace::capture_room_list_applied(rooms, &state.room_notification_settings),
        ),
        _ => None,
    };
    if let Some(event) = live_receipt_profile_diagnostic_event(state, &action) {
        record(event);
    }
    if let Some(event) = profile_resolution_diagnostic_event(state, &action) {
        record(event);
    }
    let effects = reduce(state, action);
    if let Some(input) = room_list_trace {
        unread_trace::trace_room_list_applied(&input, &state.rooms);
    }
    for effect in &effects {
        record_native_attention_recomputed(effect);
    }
    effects
}

#[derive(Default)]
pub(super) struct DeferredReducerSideEffects {
    cancel_activity_resolution: bool,
    cancel_event_navigation_owner: bool,
    navigation: Option<(koushi_protocol::SessionKeyId, NavigationState, bool)>,
    composer_drafts: Option<(koushi_protocol::SessionKeyId, ComposerDraftStore)>,
    composer_drafts_discarded: bool,
    scheduled_sends: Option<DeferredScheduledSendPersist>,
}

impl DeferredReducerSideEffects {
    pub(super) fn discards_composer_drafts(&self) -> bool {
        self.composer_drafts_discarded
    }

    pub(super) fn cancel_composer_draft_persist(&mut self) {
        self.composer_drafts = None;
    }

    pub(super) fn has_navigation_persist(&self) -> bool {
        self.navigation.is_some()
    }

    pub(super) fn has_composer_draft_persist(&self) -> bool {
        self.composer_drafts.is_some()
    }

    pub(super) fn has_scheduled_send_persist(&self) -> bool {
        self.scheduled_sends.is_some()
    }
}

impl super::AppActor {
    pub(super) async fn reduce_app_action(&mut self, action: AppAction) -> Vec<AppEffect> {
        let (effects, deferred) = self.reduce_app_action_state(action);
        self.apply_deferred_reducer_side_effects(deferred).await;
        effects
    }

    pub(super) fn reduce_app_action_state(
        &mut self,
        action: AppAction,
    ) -> (Vec<AppEffect>, DeferredReducerSideEffects) {
        let explicit_navigation_preference_mutation = matches!(
            &action,
            AppAction::ReorderSpaces { .. }
                | AppAction::SpaceOrderPreferenceRemoved { .. }
                | AppAction::NavigationPreferenceUpdated { .. }
        );
        let composer_draft_transition = composer_draft_transition_policy(&action);
        let destructive_state_before = (composer_draft_transition
            == ComposerDraftTransitionPolicy::Discard)
            .then(|| self.state.clone());
        let activity_was_open = matches!(self.state.activity, ActivityState::Open { .. });
        let previous_session = composer_draft_session_key(&self.state);
        let previous_drafts = self.state.composer_drafts.clone();
        let previous_composer_targets = active_composer_targets(&self.state);
        let previous_navigation_session = navigation_session_key(&self.state);
        let previous_navigation = self.state.navigation.clone();
        let previous_event_navigation = self.state.navigation.event_navigation;
        let previous_scheduled_session = scheduled_send_session_key(&self.state);
        let previous_scheduled_sends = self.state.scheduled_sends.clone();
        let internal_event_navigation_select = is_internal_event_navigation_select(
            self.pending_event_navigation.as_ref(),
            &self.pending_select,
            &action,
        );
        let effects = reduce_with_unread_diagnostics(&mut self.state, action);
        if composer_draft_session_key(&self.state) != previous_session {
            self.composer_draft_reload_required = true;
        }
        if previous_navigation.space_order != self.state.navigation.space_order
            || explicit_navigation_preference_mutation
        {
            let visible_space_ids = self
                .state
                .spaces
                .iter()
                .map(|space| space.space_id.as_str())
                .collect::<BTreeSet<_>>();
            let missing_space_count = self
                .state
                .navigation
                .space_order
                .iter()
                .filter(|space_id| !visible_space_ids.contains(space_id.as_str()))
                .count();
            let outcome = if explicit_navigation_preference_mutation {
                if previous_navigation.space_order != self.state.navigation.space_order {
                    "accepted"
                } else {
                    "rejected_or_noop"
                }
            } else {
                "projected"
            };
            record(
                DiagnosticEvent::new(DiagnosticLevel::Debug, "core.space_order", "projected")
                    .field(DiagnosticField::token("outcome", outcome))
                    .field(DiagnosticField::count(
                        "ledger_entries",
                        self.state.navigation.space_order.len() as u64,
                    ))
                    .field(DiagnosticField::count(
                        "visible_spaces",
                        self.state.spaces.len() as u64,
                    ))
                    .field(DiagnosticField::count(
                        "missing_space_count",
                        missing_space_count as u64,
                    )),
            );
        }
        let destructive_state_changed = destructive_state_before
            .as_ref()
            .is_some_and(|before| before != &self.state);
        if destructive_state_changed {
            self.pending_composer_draft_persist.take();
        }
        let current_composer_targets = active_composer_targets(&self.state);
        if previous_drafts != self.state.composer_drafts
            || previous_composer_targets != current_composer_targets
        {
            self.reconcile_composer_draft_lifecycle_with_active(current_composer_targets);
        }
        let mut deferred = DeferredReducerSideEffects {
            cancel_activity_resolution: activity_was_open
                && matches!(self.state.activity, ActivityState::Closed { .. }),
            cancel_event_navigation_owner: !internal_event_navigation_select
                && event_navigation_owner_cleanup_required(
                    &previous_event_navigation,
                    &self.state.navigation.event_navigation,
                ),
            composer_drafts_discarded: destructive_state_changed,
            ..DeferredReducerSideEffects::default()
        };
        let previous_persisted_navigation = previous_navigation.persistence_view();
        let current_persisted_navigation = self.state.navigation.persistence_view();
        if previous_persisted_navigation != current_persisted_navigation {
            let current_navigation_session = navigation_session_key(&self.state);
            let cleared_for_session_transition = previous_navigation_session.is_some()
                && current_navigation_session.is_none()
                && current_persisted_navigation == NavigationState::default();
            if !cleared_for_session_transition
                && let Some(key_id) = current_navigation_session.or(previous_navigation_session)
            {
                deferred.navigation = Some((
                    key_id,
                    current_persisted_navigation,
                    explicit_navigation_preference_mutation,
                ));
            }
        }
        if previous_drafts != self.state.composer_drafts {
            match composer_draft_transition {
                ComposerDraftTransitionPolicy::Discard => {}
                ComposerDraftTransitionPolicy::PreservePrevious => {
                    if let Some(key_id) = previous_session {
                        deferred.composer_drafts = Some((key_id, previous_drafts));
                    }
                }
                ComposerDraftTransitionPolicy::Normal => {
                    let current_session = composer_draft_session_key(&self.state);
                    let session_changed = current_session != previous_session;
                    let target_session = if session_changed {
                        previous_session
                    } else {
                        current_session
                    };
                    if let Some(key_id) = target_session {
                        deferred.composer_drafts = Some((
                            key_id,
                            if session_changed {
                                previous_drafts
                            } else {
                                self.state.composer_drafts.clone()
                            },
                        ));
                    }
                }
            }
        }
        if previous_scheduled_sends != self.state.scheduled_sends {
            let current_scheduled_session = scheduled_send_session_key(&self.state);
            let cleared_for_session_transition = self.state.scheduled_sends.items.is_empty()
                && previous_scheduled_session.is_some()
                && current_scheduled_session.is_none();

            deferred.scheduled_sends = if cleared_for_session_transition {
                Some(DeferredScheduledSendPersist::ClearLoadedMarker)
            } else {
                current_scheduled_session
                    .or(previous_scheduled_session)
                    .map(|key_id| DeferredScheduledSendPersist::Persist {
                        key_id,
                        scheduled_sends: self.state.scheduled_sends.clone(),
                    })
            };
        }
        (effects, deferred)
    }

    pub(super) async fn apply_deferred_reducer_side_effects(
        &mut self,
        deferred: DeferredReducerSideEffects,
    ) {
        if deferred.cancel_event_navigation_owner {
            self.cancel_event_navigation_owner().await;
        }
        if deferred.cancel_activity_resolution {
            let _ = self
                .account_actor
                .send(AccountMessage::CancelActivityResolution)
                .await;
        }
        if let Some((key_id, navigation, explicit_preference_mutation)) = deferred.navigation {
            let load_failed = self.navigation_persistence_status
                == NavigationPersistenceStatus::LoadFailed(key_id.clone());
            if !load_failed || explicit_preference_mutation {
                self.persist_navigation(key_id, navigation).await;
            } else {
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Warn,
                        "core.space_order",
                        "persistence_skipped_after_load_failure",
                    )
                    .field(DiagnosticField::count(
                        "ledger_entries",
                        navigation.space_order.len() as u64,
                    ))
                    .field(DiagnosticField::boolean(
                        "explicit_preference_mutation",
                        false,
                    )),
                );
            }
        }
        if let Some((key_id, drafts)) = deferred.composer_drafts {
            self.schedule_composer_draft_persist(key_id, drafts).await;
        }
        match deferred.scheduled_sends {
            Some(DeferredScheduledSendPersist::ClearLoadedMarker) => {
                // `clear_session_views` intentionally clears the in-memory
                // projection on lock, logout, and account switch. That is not
                // a user cancellation, so do not overwrite the account's
                // persisted scheduled sends with an empty store.
                self.scheduled_sends_loaded_for = None;
            }
            Some(DeferredScheduledSendPersist::Persist {
                key_id,
                scheduled_sends,
            }) => {
                self.persist_scheduled_sends(key_id, scheduled_sends).await;
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests;
