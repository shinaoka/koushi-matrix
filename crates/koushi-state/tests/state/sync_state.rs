use koushi_state::{
    AppAction, AppEffect, AppState, SessionInfo, SessionState, SyncLifecycleStatus, SyncState,
    UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.org".to_owned(),
        user_id: "@user:matrix.org".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn ready_state(sync: SyncState) -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        sync,
        ..AppState::default()
    }
}

#[test]
fn sync_status_projection_converges_from_stopped_to_running() {
    let mut state = ready_state(SyncState::Stopped);

    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 1,
            status: SyncLifecycleStatus::Running,
        },
    );

    assert_eq!(state.sync, SyncState::Running);
    assert_eq!(state.sync_generation, 1);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::SyncConnectivityChanged { proven: true },
            // #1009: an unchecked Ready session arms its initial status check.
            AppEffect::ArmCurrentSessionStatusCheck {
                token: 1,
                due_at_ms: 0,
            },
        ]
    );
}

#[test]
fn sync_status_projection_discards_stale_generations() {
    let mut state = ready_state(SyncState::Running);
    state.sync_generation = 3;

    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 3,
            status: SyncLifecycleStatus::Stopped,
        },
    );

    assert_eq!(state.sync, SyncState::Running);
    assert_eq!(state.sync_generation, 3);
    assert!(effects.is_empty());
}

#[test]
fn sync_status_projection_normalizes_when_session_not_sync_capable() {
    let mut state = AppState {
        session: SessionState::Locked(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 4,
            status: SyncLifecycleStatus::Reconnecting {
                reason: "network_offline".to_owned(),
            },
        },
    );

    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(state.sync_generation, 4);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::SyncConnectivityChanged { proven: false },
        ]
    );
}

#[test]
fn sync_reconnecting_from_running_updates_state() {
    let mut state = ready_state(SyncState::Running);

    let effects = reduce(
        &mut state,
        AppAction::SyncReconnecting {
            reason: "network_offline".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Reconnecting {
            reason: "network_offline".to_owned()
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

#[test]
fn sync_reconnecting_does_not_restart_stopped_sync() {
    let mut state = ready_state(SyncState::Stopped);

    let effects = reduce(
        &mut state,
        AppAction::SyncReconnecting {
            reason: "network_offline".to_owned(),
        },
    );

    assert_eq!(state.sync, SyncState::Stopped);
    assert!(effects.is_empty());
}

#[test]
fn app_state_wire_has_no_redundant_sync_mode() {
    let value = serde_json::to_value(AppState::default()).expect("serialize app state");

    assert!(value.get("sync_mode").is_none());
}

// Diagnostic characterization for #860, using the production reducer.
#[test]
fn issue860_running_before_promotion_is_swallowed_but_fresh_projection_recovers() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: session_info(),
            phase: koushi_state::ProvisionalPhase::CheckingTrust,
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 1,
            status: SyncLifecycleStatus::Running,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.sync, SyncState::Stopped);
    let effects = reduce(
        &mut state,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 2,
            transition_id: 1,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        },
    );
    assert!(matches!(state.session, SessionState::Ready(_)));
    assert_eq!(state.sync, SyncState::Starting);
    assert_eq!(
        state.secure_backup_gate,
        koushi_state::SecureBackupGateState::Checking
    );
    assert!(effects.contains(&AppEffect::StartSync));
    assert!(effects.contains(&AppEffect::InspectSecureBackup));
    assert!(!effects.contains(&AppEffect::SyncConnectivityChanged { proven: true }));
    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 1,
            status: SyncLifecycleStatus::Running,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.sync, SyncState::Starting);
    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 2,
            status: SyncLifecycleStatus::Running,
        },
    );
    assert!(effects.contains(&AppEffect::SyncConnectivityChanged { proven: true }));
    assert_eq!(state.sync, SyncState::Running);
}
