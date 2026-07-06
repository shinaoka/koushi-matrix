use koushi_state::{
    AppAction, AppEffect, AppState, SessionInfo, SessionState, SyncLifecycleStatus, SyncState,
    UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.org".to_owned(),
        user_id: "@user:matrix.org".to_owned(),
        device_id: "DEVICE".to_owned(),
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
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
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
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
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
