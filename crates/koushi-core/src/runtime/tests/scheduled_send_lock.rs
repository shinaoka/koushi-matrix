//! #1983: a scheduled send is edited by a command, not only by the panel, so the
//! same Secure Backup admission that closes the composer body refuses the
//! reschedule too. The panel lock is the surface; this is the write path.
//!
//! Both sides run the same command on the same state and differ only in the
//! gate, so the assertions below are about admission, not about the command.

use super::*;
use koushi_state::{ScheduledSendHandle, ScheduledSendItem};

const ROOM: &str = "!scheduled-lock-room:example.invalid";
const SCHEDULED_ID: &str = "scheduled-lock-1";
const ORIGINAL_BODY: &str = "original scheduled body";
const ORIGINAL_SEND_AT_MS: u64 = 1_900_000_000_000;
const EDITED_BODY: &str = "edited scheduled body";
const EDITED_SEND_AT_MS: u64 = 1_900_000_600_000;

fn request() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1983),
        sequence: 1,
    }
}

/// Ready session holding one locally scheduled send. `gate_admits` chooses
/// whether Secure Backup admission is open, which is the only difference
/// between the locked and unlocked runs.
fn state_with_scheduled_send(gate_admits: bool) -> AppState {
    let mut state = ready_session_with_gate(gate_admits);
    state.scheduled_sends.items.insert(
        SCHEDULED_ID.to_owned(),
        ScheduledSendItem {
            scheduled_id: SCHEDULED_ID.to_owned(),
            room_id: ROOM.to_owned(),
            thread_root_event_id: None,
            body: ORIGINAL_BODY.to_owned(),
            send_at_ms: ORIGINAL_SEND_AT_MS,
            handle: ScheduledSendHandle::Local,
            is_dispatching: false,
        },
    );
    state
}

fn reschedule_command() -> CoreCommand {
    CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id: request(),
        scheduled_id: SCHEDULED_ID.to_owned(),
        body: EDITED_BODY.to_owned(),
        send_at_ms: EDITED_SEND_AT_MS,
    })
}

/// Runs the reschedule against one gate state and returns `(dispatched, events)`.
async fn dispatch_reschedule(gate_admits: bool) -> (bool, Vec<CoreEvent>) {
    let data = tempfile::tempdir().expect("runtime data directory");
    let (mut actor, _, _, mut account, mut events, ..) =
        app_actor_event_navigation_fixture(data.path(), state_with_scheduled_send(gate_admits));
    let dispatched = actor
        .handle_public_command(CoreCommandEnvelope::Public {
            command: reschedule_command(),
            composer_permit: None,
            admission: None,
        })
        .await;
    // The locked run emits its refusal; drain whatever is already queued so the
    // assertions do not depend on scheduling.
    let mut drained = Vec::new();
    while let Ok(event) = events.try_recv() {
        drained.push(event);
    }
    // Drained unconditionally: an account message this stub left queued would
    // make teardown wait, and nothing here asserts on that channel.
    while account.try_recv().is_ok() {}
    assert_eq!(
        actor.state.scheduled_sends.items.len(),
        1,
        "a reschedule edits one item; it never adds or removes one"
    );
    let item = actor
        .state
        .scheduled_sends
        .items
        .get(SCHEDULED_ID)
        .expect("the item stays put");
    if gate_admits {
        assert_eq!(item.body, EDITED_BODY, "an admitted reschedule applies");
        assert_eq!(item.send_at_ms, EDITED_SEND_AT_MS);
    } else {
        assert_eq!(
            item.body, ORIGINAL_BODY,
            "a refused reschedule changes nothing"
        );
        assert_eq!(item.send_at_ms, ORIGINAL_SEND_AT_MS);
    }
    (dispatched, drained)
}

#[tokio::test]
async fn a_locked_session_refuses_to_reschedule_a_scheduled_send() {
    let (dispatched, events) = dispatch_reschedule(false).await;

    assert!(!dispatched, "the runtime must not dispatch a locked write");
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
async fn an_admitted_session_reschedules_the_same_command() {
    let (dispatched, events) = dispatch_reschedule(true).await;

    assert!(
        dispatched,
        "the same command must land once the gate is open"
    );
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
        "an admitted reschedule is not a Secure Backup refusal: {events:?}"
    );
}
