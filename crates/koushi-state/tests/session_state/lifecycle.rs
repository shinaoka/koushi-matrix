use super::support::{
    alternate_session_info, assert_session_scoped_workflows_cleared, session_info,
    state_with_session_scoped_workflows, visible_session_views_state,
};
use koushi_state::{
    AppAction, AppEffect, AppState, ComposerSubmissionTarget, ComposerSubmissionTerminalOutcome,
    CurrentDeviceTrustState, CurrentSessionStatusDetails, CurrentSessionStatusFailureKind,
    NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionState, NativeAttentionSummary, NavigationState, ProvisionalPhase,
    RoomAttentionKind, RoomSummary, RoomTags, SearchScope, SearchState, SessionLockReason,
    SessionState, SpaceSummary, SubmissionId, SyncState, ThreadAttentionState, ThreadPaneState,
    TimelinePaneState, UiEvent, reduce,
};

#[test]
fn account_switch_request_enters_switching_state_and_clears_views() {
    let current = session_info();
    let target = alternate_session_info();
    let mut state = AppState {
        session: SessionState::Ready(current),
        sync: SyncState::Running,
        navigation: NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            intent: koushi_state::ThreadOpenIntent::ExistingThread,
            is_subscribed: true,
            composer: Default::default(),
            staged_uploads: Vec::new(),
        },
        thread_attention: ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 2,
        },
        search: SearchState::Editing {
            query: "hello".to_owned(),
            scope: SearchScope::AllRooms,
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: target.clone(),
        },
    );

    assert_eq!(
        state.session,
        SessionState::SwitchingAccount {
            info: target.clone()
        }
    );
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(state.navigation, NavigationState::default());
    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(state.thread_attention, ThreadAttentionState::Closed);
    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn logout_stops_sync_and_clears_session() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.session, SessionState::LoggingOut);
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn logout_clears_session_views_and_notifies_ui() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        navigation: NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 3,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: true,
            composer: Default::default(),
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            intent: koushi_state::ThreadOpenIntent::ExistingThread,
            is_subscribed: true,
            composer: Default::default(),
            staged_uploads: Vec::new(),
        },
        thread_attention: ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 2,
        },
        search: SearchState::Editing {
            query: "アンケート".to_owned(),
            scope: SearchScope::AllRooms,
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.navigation, NavigationState::default());
    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(state.thread_attention, ThreadAttentionState::Closed);
    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn logout_clears_session_scoped_workflows_and_crawler_state() {
    let mut state = state_with_session_scoped_workflows();

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_session_scoped_workflows_cleared(&state);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
}

#[test]
fn logout_clears_native_attention_state_and_notifies_ui() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        native_attention: NativeAttentionState {
            summary: NativeAttentionSummary {
                unread_count: 4,
                highlight_count: 1,
                badge_count: 4,
                candidate: Some(NativeAttentionCandidate {
                    room_display_name: "Announcements".to_owned(),
                    kind: RoomAttentionKind::Mention,
                    unread_count: 4,
                    highlight_count: 1,
                }),
                capabilities: NativeAttentionCapabilities {
                    notifications: NativeAttentionCapability::Available,
                    badge: NativeAttentionCapability::Available,
                    overlay_icon: NativeAttentionCapability::Available,
                    sound: NativeAttentionCapability::Available,
                    tray: NativeAttentionCapability::Available,
                    activation: NativeAttentionCapability::Available,
                },
            },
            dispatch: Default::default(),
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.native_attention, NativeAttentionState::default());
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
        ]
    );
}

#[test]
fn authentication_invalidation_locks_ready_with_closed_reason_and_preserves_soft_logout() {
    for soft_logout in [true, false] {
        let mut state = AppState {
            session: SessionState::Ready(session_info()),
            sync: SyncState::Running,
            session_lock_reason: None,
            ..AppState::default()
        };
        state.spaces.push(SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: Vec::new(),
        });
        let effects = reduce(
            &mut state,
            AppAction::SessionAuthenticationInvalidated { soft_logout },
        );
        assert_eq!(state.session, SessionState::Locked(session_info()));
        assert_eq!(
            state.session_lock_reason,
            Some(SessionLockReason::UnknownToken { soft_logout })
        );
        assert_eq!(state.sync, SyncState::Stopped);
        assert!(state.spaces.is_empty());
        assert!(effects.contains(&AppEffect::StopSync));
    }
}

#[test]
fn session_locked_reenters_the_actionable_verification_gate() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    };
    reduce(&mut state, AppAction::SessionLocked);
    assert_eq!(
        state.session,
        SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        }
    );
    assert_eq!(state.session_lock_reason, None);
}

#[test]
fn stale_authentication_invalidation_is_whole_state_inert() {
    let mut state = AppState {
        session: SessionState::Locked(session_info()),
        session_lock_reason: Some(SessionLockReason::UnknownToken { soft_logout: false }),
        ..AppState::default()
    };
    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SessionAuthenticationInvalidated { soft_logout: true }
        )
        .is_empty()
    );
    assert_eq!(state, before);
}

#[test]
fn verified_trust_does_not_unlock_authentication_lock_and_logout_clears_reason() {
    let mut locked = AppState {
        session: SessionState::Locked(session_info()),
        session_lock_reason: Some(SessionLockReason::UnknownToken { soft_logout: true }),
        ..AppState::default()
    };
    let before = locked.clone();
    assert!(
        reduce(
            &mut locked,
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 0,
                transition_id: 0,
                trust: CurrentDeviceTrustState::Verified,
            },
        )
        .is_empty()
    );
    assert_eq!(locked, before);

    reduce(&mut locked, AppAction::LogoutRequested);
    assert_eq!(locked.session_lock_reason, None);
}

#[test]
fn session_locked_stops_sync_and_clears_session_views() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec![],
        }],
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SessionLocked);

    assert_eq!(
        state.session,
        SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        }
    );
    assert_eq!(state.sync, SyncState::Stopped);
    assert!(state.spaces.is_empty());
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn lock_preserves_global_submission_registry_and_records_terminal() {
    let id = SubmissionId::new("locked-submission");
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::ComposerSubmissionAccepted {
            submission_id: id.clone(),
            room_id: "room-a".to_owned(),
            transaction_id: "txn".to_owned(),
            body: "body".to_owned(),
        },
    );
    reduce(&mut state, AppAction::SessionLocked);
    assert!(
        state
            .timeline
            .submission_registry
            .accepted_submission_ids
            .contains(&id)
    );
    reduce(
        &mut state,
        AppAction::ComposerSubmissionSettled {
            submission_id: id.clone(),
            transaction_id: "wrong-txn".to_owned(),
            target: ComposerSubmissionTarget::Main {
                room_id: "room-a".to_owned(),
            },
            outcome: ComposerSubmissionTerminalOutcome::Succeeded,
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .accepted_submission_ids
            .contains(&id)
    );
    reduce(
        &mut state,
        AppAction::ComposerSubmissionSettled {
            submission_id: id.clone(),
            transaction_id: "txn".to_owned(),
            target: ComposerSubmissionTarget::Main {
                room_id: "room-a".to_owned(),
            },
            outcome: ComposerSubmissionTerminalOutcome::Succeeded,
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .settled_submission_ids
            .contains(&id)
    );
}

#[test]
fn account_replacement_clears_registry_and_ignores_unaccepted_late_terminal() {
    let old = SubmissionId::new("old-account-submission");
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    };
    state
        .timeline
        .submission_registry
        .accepted_submission_ids
        .push_back(old.clone());
    reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: alternate_session_info(),
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .accepted_submission_ids
            .is_empty()
    );
    reduce(
        &mut state,
        AppAction::ComposerSubmissionSettled {
            submission_id: old,
            transaction_id: "txn".to_owned(),
            target: ComposerSubmissionTarget::Main {
                room_id: "old-room".to_owned(),
            },
            outcome: ComposerSubmissionTerminalOutcome::Succeeded,
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .settled_submission_ids
            .is_empty()
    );
}

#[test]
fn session_locked_clears_session_scoped_workflows_and_crawler_state() {
    let mut state = state_with_session_scoped_workflows();

    let effects = reduce(&mut state, AppAction::SessionLocked);

    assert_session_scoped_workflows_cleared(&state);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
}

#[test]
fn switch_account_clears_session_scoped_workflows_and_crawler_state() {
    let mut state = state_with_session_scoped_workflows();

    let effects = reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: alternate_session_info(),
        },
    );

    assert_session_scoped_workflows_cleared(&state);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
}

#[test]
fn trust_loss_resets_status_and_visible_views_once_with_ordered_effects() {
    let cases = [
        (
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unverified),
            ProvisionalPhase::DiscoveringMethods,
        ),
        (
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unknown),
            ProvisionalPhase::RecheckingTrust { failure: None },
        ),
        (
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 7,
                transition_id: 9,
                trust: CurrentDeviceTrustState::Unverified,
            },
            ProvisionalPhase::DiscoveringMethods,
        ),
        (
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 8,
                transition_id: 10,
                trust: CurrentDeviceTrustState::Unknown,
            },
            ProvisionalPhase::RecheckingTrust { failure: None },
        ),
        (
            AppAction::SessionLocked,
            ProvisionalPhase::DiscoveringMethods,
        ),
    ];

    for (action, phase) in cases {
        let mut state = visible_session_views_state();
        let effects = reduce(&mut state, action);

        assert_eq!(
            state.session,
            SessionState::Provisional {
                info: session_info(),
                phase,
            }
        );
        assert_eq!(state.sync, SyncState::Stopped);
        assert_eq!(
            state.current_session_status,
            koushi_state::CurrentSessionStatusState::Idle
        );
        assert_eq!(
            state.invite_workflow,
            koushi_state::InviteWorkflowState::default()
        );
        assert_eq!(
            state.focused_context,
            koushi_state::FocusedContextState::Closed
        );
        assert_eq!(
            effects,
            vec![
                AppEffect::StopSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
                AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged),
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                    room_id: "room-a".to_owned(),
                }),
                AppEffect::EmitUiEvent(UiEvent::FocusedContextChanged),
            ]
        );

        let stale_details = CurrentSessionStatusDetails::new(
            Some("Stale device".to_owned()),
            "STALE".to_owned(),
            koushi_state::SessionAuthenticationMethod::Unknown,
            koushi_state::CurrentSessionSyncState::Running,
            CurrentDeviceTrustState::Verified,
            true,
            koushi_state::OwnIdentityVerification::Verified,
            koushi_state::CurrentSessionBackupState::Ready,
            2_000,
        );
        assert!(
            reduce(
                &mut state,
                AppAction::CurrentSessionStatusRefreshed {
                    request_id: 41,
                    details: stale_details,
                }
            )
            .is_empty()
        );
        assert!(
            reduce(
                &mut state,
                AppAction::CurrentSessionStatusRefreshFailed {
                    request_id: 41,
                    kind: CurrentSessionStatusFailureKind::Sdk,
                    checked_at_ms: 2_001,
                }
            )
            .is_empty()
        );
        assert_eq!(
            state.current_session_status,
            koushi_state::CurrentSessionStatusState::Idle
        );
    }
}

#[test]
fn duplicate_session_locked_does_not_reset_newer_status_or_emit_effects() {
    let mut state = visible_session_views_state();
    state.session = SessionState::Locked(session_info());
    state.current_session_status = koushi_state::CurrentSessionStatusState::Checking {
        request_id: 99,
        trigger: koushi_state::SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
    };

    let before = state.clone();
    let effects = reduce(&mut state, AppAction::SessionLocked);

    assert!(effects.is_empty());
    assert_eq!(state, before);
}

#[test]
fn verified_observation_and_non_ready_trust_loss_are_inert_for_status() {
    let mut ready = visible_session_views_state();
    let before_status = ready.current_session_status.clone();
    assert!(
        reduce(
            &mut ready,
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified)
        )
        .is_empty()
    );
    assert_eq!(ready.current_session_status, before_status);

    let mut locked = visible_session_views_state();
    locked.session = SessionState::Locked(session_info());
    locked.current_session_status = koushi_state::CurrentSessionStatusState::Ready {
        request_id: 100,
        details: CurrentSessionStatusDetails::new(
            None,
            "DEVICE".to_owned(),
            koushi_state::SessionAuthenticationMethod::Unknown,
            koushi_state::CurrentSessionSyncState::Running,
            CurrentDeviceTrustState::Verified,
            true,
            koushi_state::OwnIdentityVerification::Verified,
            koushi_state::CurrentSessionBackupState::Ready,
            3_000,
        ),
    };
    let before = locked.clone();
    assert!(
        reduce(
            &mut locked,
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unverified)
        )
        .is_empty()
    );
    assert_eq!(locked, before);

    let before = locked.clone();
    assert!(
        reduce(
            &mut locked,
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 11,
                transition_id: 12,
                trust: CurrentDeviceTrustState::Unknown,
            }
        )
        .is_empty()
    );
    assert_eq!(locked, before);
}

#[test]
fn sync_failure_enters_failed_state_before_retry() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "limited network".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Failed {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::StartSync,
        ]
    );
}

#[test]
fn sync_auth_failure_locks_session_and_does_not_retry() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "sync_failed_auth".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Failed {
            reason: "sync_failed_auth".to_owned(),
        }
    );
    assert_eq!(state.session, SessionState::Locked(session_info()));
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "sync_auth_required" && error.recoverable)
    );
    // Auth failures must NOT emit StartSync: the refresh token is invalid and
    // retrying creates an infinite loop with HTTP 401 on every attempt.
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn sync_retry_enters_reconnecting_state() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Failed {
            reason: "limited network".to_owned(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncReconnecting {
            reason: "limited network".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Reconnecting {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

#[test]
fn late_sync_signals_after_logout_are_ignored() {
    let mut state = AppState {
        session: SessionState::LoggingOut,
        sync: SyncState::Stopped,
        ..AppState::default()
    };

    assert_eq!(reduce(&mut state, AppAction::SyncStarted), Vec::new());
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SyncFailed {
                reason: "late failure".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SyncReconnecting {
                reason: "late reconnect".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(reduce(&mut state, AppAction::SyncRecovered), Vec::new());
    assert_eq!(state.sync, SyncState::Stopped);
}

#[test]
fn sync_stopped_is_a_completion_signal() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SyncStopped);

    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}
