//! #1853: MCP `send_thread_reply` and the thread pane of the room the GUI shows.
//!
//! When an MCP caller posts a thread reply in the room the user is looking
//! at, the user cannot see the reply: it lands in a thread whose pane stays
//! closed. Both candidate behaviors under discussion (open the thread only
//! when its room is shown, or switch to the room and open it) require the
//! thread to open in this case. All identifiers are synthetic.

use super::*;
use crate::direct_operations::{
    DirectMessageFailure, DirectMessageRequest, DirectMessageResponse, DirectRetryKey,
    DirectThreadReplyOutcome,
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
                        thread_root_event_id: ROOT.to_owned(),
                        thread_pane_opened: false,
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

/// The state the agent must never move: this account's session, the selected
/// Space, the selected room, and the room the main pane shows.
fn pane_context(
    state: &AppState,
) -> (SessionState, Option<String>, Option<String>, Option<String>) {
    (
        state.session.clone(),
        state.navigation.active_space_id.clone(),
        state.navigation.active_room_id.clone(),
        state.timeline.room_id.clone(),
    )
}

/// A shown room that belongs to a selected Space.
fn shown_room_in_space_state() -> AppState {
    const SPACE: &str = "!mcp-thread-space:example.invalid";
    let mut state = shown_room_state();
    state.spaces = vec![koushi_state::SpaceSummary {
        space_id: SPACE.to_owned(),
        raw_name: None,
        display_name: "Synthetic space".to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: vec![ROOM.to_owned()],
        parent_side_child_room_ids: vec![ROOM.to_owned()],
    }];
    state.navigation.active_space_id = Some(SPACE.to_owned());
    state
}

/// What the fake account runtime does with the send it receives.
#[derive(Default)]
struct ReplyScript {
    answer: Option<Result<DirectMessageResponse, DirectMessageFailure>>,
    /// Signals that this runtime received the request.
    seen: Option<oneshot::Sender<()>>,
    /// Holds the answer until the test has moved the GUI.
    go: Option<oneshot::Receiver<()>>,
}

fn thread_reply_outcome(root_event_id: &str) -> DirectMessageResponse {
    DirectMessageResponse::ThreadReply(DirectThreadReplyOutcome {
        event_id: REPLY.to_owned(),
        transaction_id: "synthetic-transaction".to_owned(),
        thread_root_event_id: root_event_id.to_owned(),
        thread_pane_opened: false,
    })
}

/// Drives one direct thread reply through a real `AppActor` and a fake account
/// runtime that answers with `script`.
async fn drive_thread_reply(
    data_dir: &std::path::Path,
    state: AppState,
    request: DirectMessageRequest,
    script: ReplyScript,
) -> (
    Result<DirectMessageResponse, DirectMessageFailure>,
    AppState,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
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
    ) = app_actor_event_navigation_fixture(data_dir, state);
    let actor_task = tokio::spawn(async move {
        let _ = actor.run().await;
    });
    let account_task = tokio::spawn(async move {
        let ReplyScript {
            answer,
            seen,
            mut go,
        } = script;
        let mut seen = seen;
        let mut account_rx = account_rx;
        while let Some(message) = account_rx.recv().await {
            if let AccountMessage::DirectMessageAction { response, .. } = message {
                if let Some(seen) = seen.take() {
                    let _ = seen.send(());
                }
                if let Some(go) = go.as_mut() {
                    let _ = go.await;
                }
                let _ = response.send(
                    answer
                        .clone()
                        .unwrap_or(Err(DirectMessageFailure::Unavailable)),
                );
            }
        }
    });
    let (response, result) = oneshot::channel();
    command_tx
        .send(CoreCommandEnvelope::DirectMessageAction { request, response })
        .await
        .expect("direct thread reply command");
    let outcome = tokio::time::timeout(Duration::from_secs(5), result)
        .await
        .expect("thread reply answered in time")
        .expect("thread reply answer channel");
    let state = snapshot_rx.borrow_and_update().state.clone();
    (outcome, state, actor_task, account_task)
}

fn thread_reply_request_to(room_id: &str, root_event_id: &str) -> DirectMessageRequest {
    DirectMessageRequest::ThreadReply {
        room_id: room_id.to_owned(),
        root_event_id: root_event_id.to_owned(),
        reply_to_event_id: None,
        body: "synthetic reply".to_owned(),
        formatting: Default::default(),
        retry: Some(DirectRetryKey {
            client: "synthetic-client".to_owned(),
            client_request_id: "synthetic-request".to_owned(),
        }),
    }
}

/// A shown room whose thread pane is already open on `root_event_id`, with the
/// thread composer holding text the user typed.
fn open_thread_state(root_event_id: &str) -> AppState {
    let mut state = shown_room_state();
    let composer = koushi_state::ComposerState {
        draft: "unsent draft".to_owned(),
        ..Default::default()
    };
    state.thread = koushi_state::ThreadPaneState::Open {
        room_id: ROOM.to_owned(),
        root_event_id: root_event_id.to_owned(),
        intent: koushi_state::ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer,
        staged_uploads: Vec::new(),
    };
    state
}

#[tokio::test]
async fn a_thread_reply_to_the_open_thread_keeps_its_composer() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let before = open_thread_state(ROOT);
    let context = pane_context(&before);
    let open_pane = before.thread.clone();
    let (outcome, state, actor_task, account_task) = drive_thread_reply(
        data_dir.path(),
        before,
        thread_reply_request_to(ROOM, ROOT),
        ReplyScript {
            answer: Some(Ok(thread_reply_outcome(ROOT))),
            ..ReplyScript::default()
        },
    )
    .await;
    actor_task.abort();
    account_task.abort();

    let DirectMessageResponse::ThreadReply(outcome) = outcome.expect("the reply was sent") else {
        panic!("a thread reply answers a thread reply");
    };
    assert!(outcome.thread_pane_opened);
    assert_eq!(
        state.thread, open_pane,
        "the pane the user already has open keeps its composer and staged uploads"
    );
    assert_eq!(pane_context(&state), context);
}

#[tokio::test]
async fn a_thread_reply_to_another_room_sends_without_opening_a_pane() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let before = shown_room_state();
    let context = pane_context(&before);
    let (outcome, state, actor_task, account_task) = drive_thread_reply(
        data_dir.path(),
        before,
        thread_reply_request_to(OTHER_ROOM, ROOT),
        ReplyScript {
            answer: Some(Ok(thread_reply_outcome(ROOT))),
            ..ReplyScript::default()
        },
    )
    .await;
    actor_task.abort();
    account_task.abort();

    let DirectMessageResponse::ThreadReply(outcome) = outcome.expect("the reply was sent") else {
        panic!("a thread reply answers a thread reply");
    };
    assert_eq!(outcome.event_id, REPLY);
    assert!(
        !outcome.thread_pane_opened,
        "a reply to another room reports that no pane opened"
    );
    assert_eq!(state.thread, koushi_state::ThreadPaneState::Closed);
    assert_eq!(pane_context(&state), context, "the view did not move");
}

#[tokio::test]
async fn a_thread_reply_opens_a_root_the_runtime_never_loaded() {
    const UNLOADED_ROOT: &str = "$mcp-thread-unloaded-root:example.invalid";
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let before = shown_room_state();
    assert!(
        !format!("{before:?}").contains(UNLOADED_ROOT),
        "the runtime state must not know this root before the reply"
    );
    let (outcome, state, actor_task, account_task) = drive_thread_reply(
        data_dir.path(),
        before,
        thread_reply_request_to(ROOM, UNLOADED_ROOT),
        ReplyScript {
            answer: Some(Ok(thread_reply_outcome(UNLOADED_ROOT))),
            ..ReplyScript::default()
        },
    )
    .await;
    actor_task.abort();
    account_task.abort();

    let DirectMessageResponse::ThreadReply(outcome) = outcome.expect("the reply was sent") else {
        panic!("a thread reply answers a thread reply");
    };
    assert!(outcome.thread_pane_opened);
    assert!(
        matches!(
            &state.thread,
            koushi_state::ThreadPaneState::Opening { room_id, root_event_id, .. }
                if room_id == ROOM && root_event_id == UNLOADED_ROOT
        ),
        "the pane opens on the root the send resolved, loaded or not: {:?}",
        state.thread
    );
}

#[tokio::test]
async fn a_thread_reply_reveal_keeps_the_account_space_and_room() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let before = shown_room_in_space_state();
    let context = pane_context(&before);
    let (outcome, state, actor_task, account_task) = drive_thread_reply(
        data_dir.path(),
        before,
        thread_reply_request_to(ROOM, ROOT),
        ReplyScript {
            answer: Some(Ok(thread_reply_outcome(ROOT))),
            ..ReplyScript::default()
        },
    )
    .await;
    actor_task.abort();
    account_task.abort();

    let DirectMessageResponse::ThreadReply(outcome) = outcome.expect("the reply was sent") else {
        panic!("a thread reply answers a thread reply");
    };
    assert!(outcome.thread_pane_opened);
    assert_eq!(
        pane_context(&state),
        context,
        "opening the thread pane changes no account, Space or room"
    );
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM));
    assert!(state.rooms.iter().any(|room| room.room_id == OTHER_ROOM));
}

#[tokio::test]
async fn a_thread_reply_whose_root_lookup_failed_opens_no_pane() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let before = shown_room_state();
    let context = pane_context(&before);
    let (outcome, state, actor_task, account_task) = drive_thread_reply(
        data_dir.path(),
        before,
        thread_reply_request_to(ROOM, ROOT),
        ReplyScript {
            answer: Some(Err(DirectMessageFailure::EventNotFound)),
            ..ReplyScript::default()
        },
    )
    .await;
    actor_task.abort();
    account_task.abort();

    assert_eq!(
        outcome.expect_err("an unresolvable root fails the reply"),
        DirectMessageFailure::EventNotFound
    );
    assert_eq!(state.thread, koushi_state::ThreadPaneState::Closed);
    assert_eq!(pane_context(&state), context);
}

#[tokio::test]
async fn a_thread_reply_settled_after_the_room_changed_opens_no_pane() {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let before = shown_room_state();
    let (go, hold) = oneshot::channel();
    let (seen_tx, seen) = oneshot::channel();
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
    ) = app_actor_event_navigation_fixture(data_dir.path(), before);
    let actor_task = tokio::spawn(async move {
        let _ = actor.run().await;
    });
    let account_task = tokio::spawn(async move {
        let mut account_rx = account_rx;
        let mut seen_tx = Some(seen_tx);
        let mut hold = Some(hold);
        while let Some(message) = account_rx.recv().await {
            if let AccountMessage::DirectMessageAction { response, .. } = message {
                if let Some(seen_tx) = seen_tx.take() {
                    let _ = seen_tx.send(());
                }
                if let Some(hold) = hold.as_mut() {
                    let _ = hold.await;
                }
                let _ = response.send(Ok(thread_reply_outcome(ROOT)));
            }
        }
    });
    let (response, result) = oneshot::channel();
    command_tx
        .send(CoreCommandEnvelope::DirectMessageAction {
            request: thread_reply_request_to(ROOM, ROOT),
            response,
        })
        .await
        .expect("direct thread reply command");
    tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("the account runtime received the reply")
        .expect("account runtime seen channel");
    command_tx
        .send(CoreCommandEnvelope::Public {
            command: CoreCommand::Room(koushi_protocol::command::RoomCommand::SelectRoom {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 1,
                },
                room_id: OTHER_ROOM.to_owned(),
            }),
            composer_permit: None,
            admission: None,
        })
        .await
        .expect("select another room");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if snapshot_rx
                .borrow_and_update()
                .state
                .timeline
                .room_id
                .as_deref()
                == Some(OTHER_ROOM)
            {
                break;
            }
            snapshot_rx.changed().await.expect("snapshot channel");
        }
    })
    .await
    .expect("the other room is displayed");
    let _ = go.send(());
    let outcome = tokio::time::timeout(Duration::from_secs(5), result)
        .await
        .expect("thread reply answered in time")
        .expect("thread reply answer channel");
    let state = snapshot_rx.borrow_and_update().state.clone();
    actor_task.abort();
    account_task.abort();

    let DirectMessageResponse::ThreadReply(outcome) = outcome.expect("the reply was sent") else {
        panic!("a thread reply answers a thread reply");
    };
    assert!(!outcome.thread_pane_opened);
    assert_eq!(state.thread, koushi_state::ThreadPaneState::Closed);
    assert_eq!(state.timeline.room_id.as_deref(), Some(OTHER_ROOM));
}
