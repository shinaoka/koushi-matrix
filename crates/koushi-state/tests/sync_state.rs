use koushi_state::{
    AppAction, AppEffect, AppState, SessionInfo, SessionState, SyncState, UiEvent, reduce,
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
