use std::collections::BTreeSet;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    ActivityState, AppAction, AppEffect, AppState, ComposerDraftStore, NavigationState, reduce,
};

use super::composer::{
    ComposerDraftTransitionPolicy, active_composer_targets, composer_draft_session_key,
    composer_draft_transition_policy,
};
use super::navigation::{NavigationPersistenceStatus, navigation_session_key};
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
    navigation: Option<(koushi_key::SessionKeyId, NavigationState, bool)>,
    composer_drafts: Option<(koushi_key::SessionKeyId, ComposerDraftStore)>,
    scheduled_sends: Option<DeferredScheduledSendPersist>,
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
            AppAction::ReorderSpaces { .. } | AppAction::SpaceOrderPreferenceRemoved { .. }
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
        let previous_scheduled_session = scheduled_send_session_key(&self.state);
        let previous_scheduled_sends = self.state.scheduled_sends.clone();
        let effects = reduce_with_unread_diagnostics(&mut self.state, action);
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
        if destructive_state_before
            .as_ref()
            .is_some_and(|before| before != &self.state)
        {
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
            ..DeferredReducerSideEffects::default()
        };
        if previous_navigation != self.state.navigation {
            let target_session =
                navigation_session_key(&self.state).or(previous_navigation_session);
            if let Some(key_id) = target_session {
                deferred.navigation = Some((
                    key_id,
                    self.state.navigation.clone(),
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
mod tests {
    use super::super::tests::unread_diagnostic_room;
    use super::*;
    use koushi_state::{RoomLatestEventSummary, SessionInfo, SessionState};

    #[test]
    fn room_list_applied_records_through_real_reducer_with_trace_env_unset() {
        let child = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "runtime::reducer_support::tests::room_list_applied_records_without_trace_environment",
            "--ignored",
            "--nocapture",
        ])
        .env_remove("KOUSHI_UNREAD_TRACE")
        .status()
        .expect("env-unset room-list diagnostic child should start");
        assert!(
            child.success(),
            "env-unset diagnostic child failed: {child}"
        );
    }

    #[test]
    #[ignore]
    fn room_list_applied_records_without_trace_environment() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        assert!(std::env::var_os("KOUSHI_UNREAD_TRACE").is_none());
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@synthetic:example.invalid".to_owned(),
                device_id: "SYNTHETIC".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            ..AppState::default()
        };
        let private_room_id = "!private-room:example.invalid";

        reduce_with_unread_diagnostics(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![unread_diagnostic_room(private_room_id)],
            },
        );

        assert_eq!(state.rooms.len(), 1, "the real reducer path should run");
        let event = koushi_diagnostics::snapshot()
            .records
            .into_iter()
            .rev()
            .find(|record| {
                record.event.source == "core.unread" && record.event.stage == "room_list_applied"
            })
            .expect("room-list applied metrics should be collected without an env switch")
            .event;
        assert_eq!(
            event
                .fields
                .iter()
                .map(|field| (field.key, field.value.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("unread", koushi_diagnostics::DiagnosticValue::Count(3)),
                (
                    "notifications",
                    koushi_diagnostics::DiagnosticValue::Count(2),
                ),
                ("highlights", koushi_diagnostics::DiagnosticValue::Count(1)),
                (
                    "marked_unread",
                    koushi_diagnostics::DiagnosticValue::Boolean(true),
                ),
                (
                    "notification_mode",
                    koushi_diagnostics::DiagnosticValue::Token("unknown"),
                ),
                (
                    "display_count",
                    koushi_diagnostics::DiagnosticValue::Count(2)
                ),
                (
                    "has_unread_content",
                    koushi_diagnostics::DiagnosticValue::Boolean(true),
                ),
                (
                    "is_attention_highlighted",
                    koushi_diagnostics::DiagnosticValue::Boolean(true),
                ),
                (
                    "has_unread_mention",
                    koushi_diagnostics::DiagnosticValue::Boolean(true),
                ),
                (
                    "is_muted",
                    koushi_diagnostics::DiagnosticValue::Boolean(false),
                ),
                (
                    "latest_event_present",
                    koushi_diagnostics::DiagnosticValue::Boolean(false),
                ),
            ]
        );
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains(private_room_id)
        );
    }

    #[test]
    fn native_attention_recomputed_diagnostic_records_private_safe_fields() {
        let child = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "runtime::reducer_support::tests::native_attention_recomputed_diagnostic_records_private_safe_fields_child",
            "--ignored",
            "--nocapture",
        ])
        .status()
        .expect("native-attention diagnostic child should start");
        assert!(
            child.success(),
            "native-attention diagnostic child failed: {child}"
        );
    }

    #[test]
    #[ignore]
    fn native_attention_recomputed_diagnostic_records_private_safe_fields_child() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@synthetic:example.invalid".to_owned(),
                device_id: "SYNTHETIC".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            ..AppState::default()
        };
        let private_room_id = "!private-native-attention:example.invalid";
        let private_event_id = "$private-event:example.invalid";
        let private_user_id = "@private-sender:example.invalid";
        let private_room_label = "Private native attention room";
        let private_message = "Private native attention body";
        let mut room = unread_diagnostic_room(private_room_id);
        room.display_name = private_room_label.to_owned();
        room.display_label = private_room_label.to_owned();
        room.original_display_label = private_room_label.to_owned();
        room.unread_count = 0;
        room.notification_count = 0;
        room.highlight_count = 0;
        room.marked_unread = false;
        room.latest_event = Some(RoomLatestEventSummary {
            event_id: private_event_id.to_owned(),
            relation_type: None,
            relation_event_id: None,
            sender_id: Some(private_user_id.to_owned()),
            sender_label: Some("Private sender".to_owned()),
            sender_avatar: None,
            preview: Some(private_message.to_owned()),
            timestamp_ms: 42,
            is_redacted: false,
        });
        reduce_with_unread_diagnostics(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![room.clone()],
            },
        );
        reduce_with_unread_diagnostics(
            &mut state,
            AppAction::NativeWindowFocusChanged {
                focused: false,
                observation_generation: 1,
            },
        );

        room.unread_count = 1;
        room.notification_count = 1;
        room.recency_stamp = Some(43);
        room.latest_event
            .as_mut()
            .expect("latest event")
            .timestamp_ms = 43;
        reduce_with_unread_diagnostics(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![room],
            },
        );

        let event = koushi_diagnostics::snapshot()
            .records
            .into_iter()
            .rev()
            .find(|record| {
                record.event.source == "native.attention" && record.event.stage == "recomputed"
            })
            .expect("native-attention recomputation should be diagnosed")
            .event;
        assert_eq!(
            event
                .fields
                .iter()
                .map(|field| (field.key, field.value.clone()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "observation",
                    koushi_diagnostics::DiagnosticValue::Token("live"),
                ),
                (
                    "unread_count",
                    koushi_diagnostics::DiagnosticValue::Count(1),
                ),
                (
                    "notification_count",
                    koushi_diagnostics::DiagnosticValue::Count(1),
                ),
                ("badge_count", koushi_diagnostics::DiagnosticValue::Count(1),),
                (
                    "badge_source",
                    koushi_diagnostics::DiagnosticValue::Token("raw_unread_messages"),
                ),
                (
                    "badge_room_count",
                    koushi_diagnostics::DiagnosticValue::Count(1),
                ),
                (
                    "badge_excluded_room_count",
                    koushi_diagnostics::DiagnosticValue::Count(0),
                ),
                (
                    "candidate",
                    koushi_diagnostics::DiagnosticValue::Token("message"),
                ),
                (
                    "suppression",
                    koushi_diagnostics::DiagnosticValue::Token("none"),
                ),
                (
                    "window_focused",
                    koushi_diagnostics::DiagnosticValue::Boolean(false),
                ),
                (
                    "active_room_match",
                    koushi_diagnostics::DiagnosticValue::Boolean(true),
                ),
            ]
        );
        let serialized = serde_json::to_string(&event).unwrap();
        for private_value in [
            private_room_id,
            private_event_id,
            private_user_id,
            private_room_label,
            private_message,
        ] {
            assert!(
                !serialized.contains(private_value),
                "diagnostic leaked private value: {private_value}"
            );
        }
    }
}
