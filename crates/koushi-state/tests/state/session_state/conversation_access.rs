use super::support::session_info;
use koushi_state::{
    AppAction, AppState, ConversationAccess, RoomListReadiness, RoomListSource,
    SecureBackupGateState, SessionState, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        secure_backup_gate: SecureBackupGateState::Checking,
        ..AppState::default()
    }
}

#[test]
fn a_restored_session_may_read_before_any_admission_but_may_not_send() {
    // #1737: the restored session has passed the existing trust/session
    // admission, its cached room list is still provisional, and Secure Backup
    // inspection has not settled.
    let state = ready_state();

    assert_eq!(
        state.conversation_access(),
        ConversationAccess {
            readable: true,
            list_authoritative: false,
            send_admitted: false,
        }
    );
}

#[test]
fn uncollapsed_trust_gates_expose_no_cached_content() {
    for session in [
        SessionState::SignedOut,
        SessionState::Restoring,
        SessionState::Locked(session_info()),
    ] {
        let state = AppState {
            session,
            secure_backup_gate: SecureBackupGateState::BlockedFailed {
                failure: koushi_state::SecureBackupGateFailureKind::Network,
                detail: None,
            },
            ..AppState::default()
        };

        assert_eq!(
            state.conversation_access(),
            ConversationAccess {
                readable: false,
                list_authoritative: false,
                send_admitted: false,
            }
        );
    }
}

#[test]
fn only_a_current_generation_authoritative_snapshot_confirms_the_list() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 3,
            source: RoomListSource::Live,
        },
    );
    assert!(!effects.is_empty());
    assert!(!state.conversation_access().list_authoritative);

    // A provisional observation of the restored cache keeps the list
    // unconfirmed, so counts, badges, and read-state writes stay provisional.
    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 3,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );
    assert!(
        !state.conversation_access().readable || !state.conversation_access().list_authoritative
    );

    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 3,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );
    assert_eq!(
        state.room_list.readiness,
        RoomListReadiness::Ready {
            source: RoomListSource::Live,
            generation: 3,
        }
    );
    assert!(state.conversation_access().list_authoritative);
}

#[test]
fn secure_backup_admission_opens_sending_and_its_absence_keeps_it_closed() {
    let mut state = ready_state();
    assert!(!state.conversation_access().send_admitted);

    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::Ready),
    );
    assert!(state.conversation_access().send_admitted);

    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::ExistingBackupNeedsRecovery {
            failure: None,
        }),
    );
    assert!(state.conversation_access().readable);
    assert!(
        !state.conversation_access().send_admitted,
        "a positive recovery requirement closes sending again"
    );
}
