use super::{classify_room_error, matrix_create_room_options, trace_room_operation};

use koushi_protocol::command::{CreateRoomOptions, CreateRoomVisibility, RoomCommand};

use koushi_protocol::event::CoreEvent;

use koushi_protocol::failure::{CoreFailure, RoomFailureKind};

use crate::room::actor::make_request_id;
use crate::room::actor::{RoomActor, RoomMessage};

use koushi_sdk::{
    MatrixCreateRoomAccessPolicy, MatrixRoomHistoryVisibility, MatrixRoomJoinRule,
    MatrixRoomOperationError,
};

use koushi_state::{RoomAccessPolicy, RoomHistoryVisibility, RoomJoinRule};

use koushi_state::RoomTagKind;

use tokio::sync::{broadcast, mpsc};

#[test]
fn create_access_policy_and_history_route_to_the_sdk_options() {
    let options = matrix_create_room_options(CreateRoomOptions {
        name: "Synthetic".to_owned(),
        topic: None,
        alias_localpart: None,
        encrypted: true,
        invited_only: false,
        visibility: CreateRoomVisibility::Private,
        parent_space: None,
        access_policy: Some(RoomAccessPolicy::new(
            RoomJoinRule::Restricted,
            vec!["!space:example.invalid".to_owned()],
        )),
        history: Some(RoomHistoryVisibility::Joined),
    });
    assert_eq!(
        options.access_policy,
        Some(MatrixCreateRoomAccessPolicy {
            rule: MatrixRoomJoinRule::Restricted,
            allow: vec!["!space:example.invalid".to_owned()],
        })
    );
    assert_eq!(options.history, Some(MatrixRoomHistoryVisibility::Joined));
    assert!(options.encrypted);
}

#[test]
fn create_policy_rejections_classify_to_typed_room_failures() {
    for (error, expected) in [
        (
            MatrixRoomOperationError::PublicRoomWithRestrictedAccess,
            RoomFailureKind::PublicRoomWithRestrictedAccess,
        ),
        (
            MatrixRoomOperationError::ExplicitAccessPolicyWithInvitedOnly,
            RoomFailureKind::ExplicitAccessPolicyWithInvitedOnly,
        ),
        (
            MatrixRoomOperationError::EmptyAccessPolicyTargets,
            RoomFailureKind::EmptyAccessPolicyTargets,
        ),
    ] {
        assert_eq!(classify_room_error(&error), expected);
    }
}

#[test]
fn room_operation_records_without_environment_switch() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    trace_room_operation("create_room", "test_always_on", make_request_id(999));
    assert!(
        koushi_diagnostics::test_support::detail_snapshot()
            .records
            .iter()
            .any(|record| {
                record.event.source == "core.room" && record.event.stage == "test_always_on"
            })
    );
}

#[test]
fn alias_collision_retains_a_closed_actionable_protocol_kind() {
    let error =
        MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::AliasInUse);
    let kind = classify_room_error(&error);
    assert_eq!(kind, RoomFailureKind::AliasInUse);
    assert_eq!(
        serde_json::to_value(CoreFailure::RoomOperationFailed { kind }).unwrap(),
        serde_json::json!({"RoomOperationFailed": {"kind": "AliasInUse"}})
    );
}

#[test]
fn forbidden_sdk_error_classifies_as_forbidden() {
    let error =
        MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::Forbidden);
    assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
}

#[test]
fn auth_required_sdk_error_classifies_as_forbidden() {
    let error = MatrixRoomOperationError::Sdk(
        koushi_sdk::MatrixRoomOperationFailureKind::AuthenticationRequired,
    );
    assert_eq!(classify_room_error(&error), RoomFailureKind::Forbidden);
}

#[test]
fn http_sdk_error_classifies_as_network() {
    let error = MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::Http);
    assert_eq!(classify_room_error(&error), RoomFailureKind::Network);
}

#[test]
fn invalid_room_id_classifies_as_not_found() {
    let error = MatrixRoomOperationError::InvalidRoomId;
    assert_eq!(classify_room_error(&error), RoomFailureKind::NotFound);
}

#[test]
fn room_unavailable_classifies_as_not_found() {
    let error = MatrixRoomOperationError::RoomUnavailable;
    assert_eq!(classify_room_error(&error), RoomFailureKind::NotFound);
}

#[test]
fn invalid_invite_classifies_as_invalid_invite() {
    let error = MatrixRoomOperationError::InvalidInvite;
    assert_eq!(classify_room_error(&error), RoomFailureKind::InvalidInvite);
}

#[test]
fn sdk_error_classifies_as_sdk() {
    let error = MatrixRoomOperationError::Sdk(koushi_sdk::MatrixRoomOperationFailureKind::Sdk);
    assert_eq!(classify_room_error(&error), RoomFailureKind::Sdk);
}

#[tokio::test]
async fn create_room_without_session_emits_session_required() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    let request_id = make_request_id(3);
    handle
        .send(RoomMessage::Command(RoomCommand::CreateRoom {
            request_id,
            options: CreateRoomOptions {
                name: "test room".to_owned(),
                topic: None,
                alias_localpart: None,
                encrypted: false,
                invited_only: false,
                visibility: CreateRoomVisibility::Private,
                parent_space: None,
                access_policy: None,
                history: None,
            },
        }))
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("timeout")
        .expect("event");

    match event {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn leave_room_without_session_emits_session_required() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    let request_id = make_request_id(4);
    handle
        .send(RoomMessage::Command(RoomCommand::LeaveRoom {
            request_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("timeout")
        .expect("event");

    match event {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn forget_room_without_session_emits_session_required() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    let request_id = make_request_id(5);
    handle
        .send(RoomMessage::Command(RoomCommand::ForgetRoom {
            request_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("timeout")
        .expect("event");

    match event {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn set_room_tag_without_session_emits_session_required() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    let request_id = make_request_id(6);
    handle
        .send(RoomMessage::Command(RoomCommand::SetTag {
            request_id,
            room_id: "!room:example.test".to_owned(),
            tag: RoomTagKind::Favourite,
            order: None,
        }))
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("timeout")
        .expect("event");

    match event {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn remove_room_tag_without_session_emits_session_required() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, mut event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    let request_id = make_request_id(7);
    handle
        .send(RoomMessage::Command(RoomCommand::RemoveTag {
            request_id,
            room_id: "!room:example.test".to_owned(),
            tag: RoomTagKind::LowPriority,
        }))
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(5), event_rx.recv())
        .await
        .expect("timeout")
        .expect("event");

    match event {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}
