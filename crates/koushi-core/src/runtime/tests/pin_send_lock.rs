//! #2087: a pin is room state every member reads, so it is a user-content write
//! and needs the same Secure Backup admission the composer body does. The
//! Pinned messages panel is the surface; this is the write path that used to
//! pass through while the composer said it could not send.
//!
//! Both sides run the same command on the same state and differ only in the
//! gate, so the assertions below are about admission, not about the command.

use super::*;
use koushi_protocol::command::RoomCommand;

const ROOM: &str = "!pin-lock-room:example.invalid";
const EVENT: &str = "$pin-lock-event:example.invalid";

fn request() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(2087),
        sequence: 1,
    }
}

fn unpin_command() -> CoreCommand {
    CoreCommand::Room(RoomCommand::UnpinEvent {
        request_id: request(),
        room_id: ROOM.to_owned(),
        event_id: EVENT.to_owned(),
    })
}

/// Runs the unpin against one gate state and returns `(dispatched, events)`.
/// This fixture owns no room, so the room command's return value says nothing
/// about admission here — only the emitted events do.
async fn dispatch_unpin(gate_admits: bool) -> (bool, Vec<CoreEvent>) {
    let data = tempfile::tempdir().expect("runtime data directory");
    let (mut actor, _, _, mut account, mut events, ..) =
        app_actor_event_navigation_fixture(data.path(), ready_session_with_gate(gate_admits));
    let dispatched = actor
        .handle_public_command(CoreCommandEnvelope::Public {
            command: unpin_command(),
            composer_permit: None,
            admission: None,
        })
        .await;
    let mut drained = Vec::new();
    while let Ok(event) = events.try_recv() {
        drained.push(event);
    }
    // Drained unconditionally: an account message this stub left queued would
    // make teardown wait, and nothing here asserts on that channel.
    while account.try_recv().is_ok() {}
    (dispatched, drained)
}

#[tokio::test]
async fn a_locked_session_refuses_to_unpin() {
    let (dispatched, events) = dispatch_unpin(false).await;

    assert!(
        !dispatched,
        "the runtime must not route a pin write to the room while sending is closed"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::SecureBackupRequired,
                },
            } if *request_id == request()
        )),
        "the refusal must name Secure Backup, not fail silently: {events:?}"
    );
}

#[tokio::test]
async fn an_admitted_session_does_not_refuse_the_same_command() {
    // The same command with the gate open leaves no Secure Backup refusal, which
    // is what makes the locked case above a statement about the gate.
    let (_, events) = dispatch_unpin(true).await;

    assert!(
        !events.iter().any(|event| matches!(
            event,
            CoreEvent::OperationFailed {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::SecureBackupRequired,
                },
                ..
            }
        )),
        "an admitted unpin is not a Secure Backup refusal: {events:?}"
    );
}
