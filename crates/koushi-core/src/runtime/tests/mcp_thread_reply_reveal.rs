//! #1853: MCP `send_thread_reply` and the thread pane of the room the GUI shows.
//!
//! When an MCP caller posts a thread reply in the room the user is looking
//! at, the user cannot see the reply: it lands in a thread whose pane stays
//! closed. Both candidate behaviors under discussion (open the thread only
//! when its room is shown, or switch to the room and open it) require the
//! thread to open in this case. All identifiers are synthetic.

use super::*;
use crate::direct_operations::{
    DirectMessageRequest, DirectMessageResponse, DirectRetryKey, DirectThreadReplyOutcome,
};

const ROOM: &str = "!mcp-thread-room:example.invalid";
const OTHER_ROOM: &str = "!mcp-thread-other:example.invalid";
const ROOT: &str = "$mcp-thread-root:example.invalid";
const REPLY: &str = "$mcp-thread-reply:example.invalid";
const USER: &str = "@synthetic:example.invalid";

/// Ready session, send admitted, with `ROOM` selected in the main pane.
fn shown_room_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: USER.to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![
            unread_diagnostic_room(ROOM),
            unread_diagnostic_room(OTHER_ROOM),
        ],
        secure_backup_gate: koushi_state::SecureBackupGateState::Ready,
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: ROOM.to_owned(),
        },
    );
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM));
    assert!(state.conversation_access().send_admitted);
    assert_eq!(state.thread, koushi_state::ThreadPaneState::Closed);
    state
}

fn thread_reply_request() -> DirectMessageRequest {
    DirectMessageRequest::ThreadReply {
        room_id: ROOM.to_owned(),
        root_event_id: ROOT.to_owned(),
        reply_to_event_id: None,
        body: "synthetic reply".to_owned(),
        formatting: Default::default(),
        retry: Some(DirectRetryKey {
            client: "synthetic-client".to_owned(),
            client_request_id: "synthetic-request".to_owned(),
        }),
    }
}

/// Answers the direct thread reply as the account runtime would after the
/// SDK send queue accepted it. Other account messages are ignored.
fn answer_thread_reply(
    mut account_rx: mpsc::Receiver<AccountMessage>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(message) = account_rx.recv().await {
            if let AccountMessage::DirectMessageAction { request, response } = message {
                assert!(matches!(request, DirectMessageRequest::ThreadReply { .. }));
                let _ = response.send(Ok(DirectMessageResponse::ThreadReply(
                    DirectThreadReplyOutcome {
                        event_id: REPLY.to_owned(),
                        transaction_id: "synthetic-transaction".to_owned(),
                    },
                )));
            }
        }
    })
}

#[tokio::test]
async fn an_mcp_thread_reply_in_the_shown_room_opens_that_thread() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (
        actor,
        command_tx,
        _action_tx,
        account_rx,
        _event_rx,
        mut snapshot_rx,
        _navigation_projection_rx,
        _event_navigation_prepared_tx,
        _focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), shown_room_state());
    let actor_task = tokio::spawn(actor.run());
    let account_task = answer_thread_reply(account_rx);

    let (response, result) = oneshot::channel();
    command_tx
        .send(CoreCommandEnvelope::DirectMessageAction {
            request: thread_reply_request(),
            response,
        })
        .await
        .expect("direct thread reply command");
    let outcome = tokio::time::timeout(Duration::from_secs(2), result)
        .await
        .expect("thread reply answered in time")
        .expect("thread reply answer channel")
        .expect("thread reply accepted");
    assert!(matches!(outcome, DirectMessageResponse::ThreadReply(_)));

    let opened = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let state = snapshot_rx.borrow_and_update().state.clone();
            if matches!(
                &state.thread,
                koushi_state::ThreadPaneState::Opening { room_id, root_event_id, .. }
                    | koushi_state::ThreadPaneState::Open { room_id, root_event_id, .. }
                    if room_id == ROOM && root_event_id == ROOT
            ) {
                break state;
            }
            snapshot_rx.changed().await.expect("snapshot channel");
        }
    })
    .await;
    actor_task.abort();
    account_task.abort();

    let state = opened.expect(
        "the thread pane opens on the replied thread when the GUI shows its room \
         (thread pane stayed closed after the MCP reply)",
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM));
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM));
}
