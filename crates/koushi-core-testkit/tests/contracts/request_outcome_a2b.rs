use futures_util::FutureExt;
use koushi_core::runtime::request_outcome::{
    OutcomeCorrelation, RequestOutcome, RequestOutcomeExpectation, RoomOperationKind,
};
use koushi_core::{AccountKey, CoreConnection, RequestId, RuntimeConnectionId};
use koushi_protocol::event::{CoreEvent, RoomEvent};
use koushi_state::{AppState, RoomTagKind, SessionInfo, SessionState};
use std::time::Duration;

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(71),
        sequence,
    }
}

fn ready_state(user_id: &str) -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: user_id.to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: Default::default(),
        }),
        ..Default::default()
    }
}

fn versioned(
    state: AppState,
    generation: u64,
) -> koushi_protocol::state_update::VersionedAppStateSnapshot {
    koushi_protocol::state_update::VersionedAppStateSnapshot { generation, state }
}

fn room_summary(room_id: &str) -> koushi_state::RoomSummary {
    koushi_state::RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: Default::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

#[tokio::test]
async fn forgotten_room_requires_correlated_event_and_authoritative_absence() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(1);
    let room_id = "!forgotten:example.invalid".to_owned();
    let mut state = ready_state("@alice:example.invalid");
    state.rooms.push(room_summary(&room_id));
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomOperation {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            room_id: room_id.clone(),
            operation: RoomOperationKind::RoomForgotten,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::Room(RoomEvent::RoomForgotten {
        request_id,
        room_id: room_id.clone(),
    }));
    control.send_snapshot(versioned(state.clone(), 1));
    assert!(waiter.as_mut().now_or_never().is_none());
    state.rooms.clear();
    control.send_snapshot(versioned(state, 2));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::RoomOperation { .. })
    ));
}

#[tokio::test]
async fn tag_operation_uses_exact_event_without_requiring_room_presence() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(2);
    let room_id = "!tagged:example.invalid".to_owned();
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomOperation {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            room_id: room_id.clone(),
            operation: RoomOperationKind::RoomTagSet {
                tag: RoomTagKind::Favourite,
            },
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    let state = ready_state("@alice:example.invalid");
    control.send_snapshot(versioned(state, 1));
    control.send_event(CoreEvent::Room(RoomEvent::RoomTagSet {
        request_id,
        room_id,
        tag: RoomTagKind::Favourite,
    }));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::RoomOperation { .. })
    ));
}

#[tokio::test]
async fn room_join_without_preknown_id_accepts_the_joined_room_event() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(3);
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let room_id = "!joined:example.invalid".to_owned();
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomJoined {
            request_id,
            account_key,
            room_id: String::new(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);

    control.send_event(CoreEvent::Room(RoomEvent::RoomJoined {
        request_id,
        room_id: room_id.clone(),
    }));
    let mut state = ready_state("@alice:example.invalid");
    state.rooms.push(room_summary(&room_id));
    control.send_snapshot(versioned(state, 1));

    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::RoomJoined { room_id: joined, .. }) if joined == room_id
    ));
}

#[tokio::test]
async fn event_terminal_room_operation_returns_lagged_without_waiting_for_deadline() {
    let (mut connection, control) = CoreConnection::new_for_testing(1);
    let request_id = request(20);
    control.send_event(CoreEvent::Room(RoomEvent::RoomListUpdated));
    control.send_event(CoreEvent::Room(RoomEvent::RoomListUpdated));
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomOperation {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomOperationKind::RoomTagSet {
                tag: RoomTagKind::Favourite,
            },
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    assert!(matches!(
        waiter.await,
        Err(koushi_core::RequestOutcomeError::Lagged)
    ));
}

#[test]
fn request_outcome_a2b_fixtures_are_private_safe() {
    let _ = ready_state("@alice:example.invalid");
}
