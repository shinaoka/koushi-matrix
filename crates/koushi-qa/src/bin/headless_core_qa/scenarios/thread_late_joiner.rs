//! `thread_late_joiner`: a member who joins after a thread root was sent, in a
//! room whose history is visible to joined members only, sees a permanent
//! "root not visible" state instead of a retryable network failure, and can
//! still open the thread and read the replies visible to them.
//!
//! #1473: a fresh device of that member, whose sync window no longer holds the
//! visible reply, opens the thread through the Rust-owned pane. The pane must
//! settle in `Open` or in a visible `Failed { Forbidden | NotFound }` (Synapse
//! withholds `/relations` for an invisible root), never close silently, and a
//! retry must settle again instead of looping or reporting false success.

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    subscribe_timeline_for_qa, timeline_item_event_id, visit_timeline_diff_items,
    wait_for_invite_in_snapshot, wait_for_room_in_room_list, wait_for_send_completed,
    wait_for_send_flow_completion,
};
use super::fixtures::{
    accept_invite_for_qa, create_room_for_qa, invite_user_for_qa, load_room_settings_for_qa,
};
use super::participants::{
    QaParticipantLoginGate, QaParticipantLoginOutcome, login_synced_participant_for_qa,
    login_synced_participant_on_runtime_for_qa, qa_data_dir, start_isolated_qa_runtime,
};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::{
    AccountKey, AppCommand, CoreCommand, CoreConnection, CoreEvent, OperationFailureKind,
    RoomCommand, RoomEvent, RoomSettingChange, TimelineCommand, TimelineEvent, TimelineItem,
    TimelineKey, TimelineKind,
};
use koushi_protocol::event::TimelineDisplayKind;
use koushi_state::RoomHistoryVisibility;

const ROOT_BODY: &str = "Synthetic late-joiner thread root";
const EARLY_REPLY_BODY: &str = "Synthetic late-joiner early reply";
const LATE_REPLY_BODY: &str = "Synthetic late-joiner late reply";
const FILLER_BODY: &str = "Synthetic late-joiner filler";
/// More than the SDK's default room-subscription timeline limit (20), so a fresh
/// device's room sync no longer carries the visible thread reply.
const FILLER_COUNT: usize = 25;

pub(super) async fn run_thread_late_joiner_scenario(config: &QaConfig) -> Result<(), String> {
    let participant_a = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("thread-late-joiner-a"),
        &config.user_a,
        &config.password_a,
        "Koushi Thread Late Joiner QA A",
        "thread late joiner login A",
        "thread late joiner gate A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let participant_b = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("thread-late-joiner-b"),
        &config.user_b,
        &config.password_b,
        "Koushi Thread Late Joiner QA B",
        "thread late joiner login B",
        "thread late joiner gate B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let QaParticipantLoginOutcome {
        runtime: runtime_a,
        conn: mut conn_a,
        account_key: account_key_a,
        ..
    } = participant_a;
    let QaParticipantLoginOutcome {
        runtime: runtime_b,
        conn: mut conn_b,
        account_key: account_key_b,
        bootstrap_recovery_secret: recovery_secret_b,
    } = participant_b;
    let user_b_id = format!("@{}:{}", config.user_b, config.server_name);

    let flow = run_thread_late_joiner_flow(
        &mut conn_a,
        &account_key_a,
        &mut conn_b,
        &account_key_b,
        &user_b_id,
    )
    .await;
    let flow = match (flow, recovery_secret_b.as_ref()) {
        (Ok((room_id, root_event_id)), Some(recovery_secret)) => {
            run_withheld_root_pane_phase(config, recovery_secret, &room_id, &root_event_id).await
        }
        (Ok(_), None) => Err("thread late joiner: B login returned no recovery secret".to_owned()),
        (Err(error), _) => Err(error),
    };
    let cleanup_b = cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "thread late joiner cleanup B",
    )
    .await;
    let cleanup_a = cleanup_logged_in_runtime(
        conn_a,
        runtime_a,
        account_key_a,
        "thread late joiner cleanup A",
    )
    .await;
    flow?;
    cleanup_b.and(cleanup_a)?;
    println!("thread_late_joiner=ok");
    Ok(())
}

async fn run_thread_late_joiner_flow(
    conn_a: &mut CoreConnection,
    account_key_a: &AccountKey,
    conn_b: &mut CoreConnection,
    account_key_b: &AccountKey,
    user_b_id: &str,
) -> Result<(String, String), String> {
    let room_id = create_room_for_qa(
        conn_a,
        "Synthetic Late Joiner Room",
        false,
        "thread late joiner room",
    )
    .await?;
    set_history_visibility_joined(conn_a, &room_id).await?;

    let room_key_a = TimelineKey {
        account_key: account_key_a.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    subscribe_timeline_for_qa(conn_a, &room_key_a, "thread late joiner subscribe A room").await?;
    let root_event_id = send_text(
        conn_a,
        &room_key_a,
        "thread-late-joiner-root",
        ROOT_BODY,
        "thread late joiner root",
    )
    .await?;
    let thread_key_a = TimelineKey {
        account_key: account_key_a.clone(),
        kind: TimelineKind::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
    };
    subscribe_timeline_for_qa(
        conn_a,
        &thread_key_a,
        "thread late joiner subscribe A thread",
    )
    .await?;
    send_thread_reply(
        conn_a,
        &thread_key_a,
        &root_event_id,
        "thread-late-joiner-early-reply",
        EARLY_REPLY_BODY,
        "thread late joiner early reply",
    )
    .await?;

    invite_user_for_qa(conn_a, &room_id, user_b_id, "thread late joiner invite B").await?;
    wait_for_invite_in_snapshot(conn_b, &room_id, None, "thread late joiner invite B").await?;
    accept_invite_for_qa(conn_b, &room_id, "thread late joiner B joins").await?;

    let late_reply_event_id = send_thread_reply(
        conn_a,
        &thread_key_a,
        &root_event_id,
        "thread-late-joiner-late-reply",
        LATE_REPLY_BODY,
        "thread late joiner late reply",
    )
    .await?;

    // Room timeline: the reply B can see must project a terminal, permanent
    // "not visible" root row, never a retryable network failure.
    let room_key_b = TimelineKey {
        account_key: account_key_b.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let initial =
        subscribe_timeline_for_qa(conn_b, &room_key_b, "thread late joiner subscribe B room")
            .await?;
    let (root_kind, root_hidden) = wait_for_terminal_root_row(
        conn_b,
        &room_key_b,
        &initial,
        &root_event_id,
        &late_reply_event_id,
    )
    .await?;
    match root_kind {
        TimelineDisplayKind::ThreadRootFailed {
            failure_kind: OperationFailureKind::NotFound | OperationFailureKind::Forbidden,
        } => {}
        other => {
            return Err(format!(
                "thread_late_joiner failed: root row for a pre-join root was {other:?}, expected a not-visible failure"
            ));
        }
    }
    // #1141: the not-visible row is the only entry point to the replies B may
    // read, so the exported row must not be content-suppressed.
    if root_hidden {
        return Err(
            "thread_late_joiner failed: the not-visible root row was exported with is_hidden=true"
                .to_owned(),
        );
    }
    println!("thread_late_joiner_root_not_visible=ok");

    // Thread panel: opening the thread must still show the reply B can see.
    let thread_key_b = TimelineKey {
        account_key: account_key_b.clone(),
        kind: TimelineKind::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
    };
    let thread_initial = subscribe_timeline_for_qa(
        conn_b,
        &thread_key_b,
        "thread late joiner subscribe B thread",
    )
    .await?;
    wait_for_event_in_timeline(
        conn_b,
        &thread_key_b,
        &thread_initial,
        &late_reply_event_id,
        "thread late joiner B thread panel",
    )
    .await?;
    println!("thread_late_joiner_thread_panel=ok");

    for index in 0..FILLER_COUNT {
        // Only the remote send matters here; back-to-back sends need not
        // surface a local echo, so wait for the send terminal alone.
        let request_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Timeline(TimelineCommand::SendText {
                explicit_mentions: None,
                request_id,
                key: room_key_a.clone(),
                transaction_id: format!("thread-late-joiner-filler-{index}"),
                document: koushi_state::ComposerDocument::from_plain_text(format!(
                    "{FILLER_BODY} {index}"
                )),
            }))
            .await
            .map_err(|e| format!("thread late joiner filler: send submission failed: {e}"))?;
        wait_for_send_completed(conn_a, request_id, &room_key_a, "thread late joiner filler")
            .await?;
    }
    Ok((room_id, root_event_id))
}

/// #1473: a fresh B device opens the thread through `AppCommand::OpenThread`.
async fn run_withheld_root_pane_phase(
    config: &QaConfig,
    recovery_secret: &koushi_state::AuthSecret,
    room_id: &str,
    root_event_id: &str,
) -> Result<(), String> {
    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        ..
    } = login_synced_participant_on_runtime_for_qa(
        // Same-user second device: must not restore B's saved session.
        start_isolated_qa_runtime("thread-late-joiner-b-fresh")?,
        &config.homeserver,
        &config.user_b,
        &config.password_b,
        "Koushi Thread Late Joiner QA B fresh",
        "thread late joiner login B fresh",
        "thread late joiner gate B fresh",
        QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret),
    )
    .await?;
    let flow =
        run_withheld_root_pane_flow(&mut conn, &config.server_kind, room_id, root_event_id).await;
    let cleanup = cleanup_logged_in_runtime(
        conn,
        runtime,
        account_key,
        "thread late joiner cleanup B fresh",
    )
    .await;
    flow?;
    cleanup
}

async fn run_withheld_root_pane_flow(
    conn: &mut CoreConnection,
    server_kind: &str,
    room_id: &str,
    root_event_id: &str,
) -> Result<(), String> {
    // Synapse answers `/relations` for the withheld root with 404, so the pane
    // must reach Failed there; other servers may serve the replies (Open).
    let require_failed = server_kind == "synapse";
    wait_for_room_in_room_list(conn, room_id, "thread late joiner B fresh room list").await?;
    let select_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id: select_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("thread late joiner: submit room select failed: {e}"))?;
    wait_for_snapshot(conn, "thread late joiner B fresh selected room", |state| {
        (state.timeline.room_id.as_deref() == Some(room_id)).then_some(())
    })
    .await?;

    let first = open_thread_until_settled(conn, room_id, root_event_id, "open").await?;
    eprintln!("thread_late_joiner withheld open settled as {first:?}");
    if require_failed && first != SettledPane::Failed {
        return Err(format!(
            "thread_late_joiner failed: {server_kind} withholds the root from a fresh device, expected Failed(Forbidden|NotFound), got {first:?}"
        ));
    }
    println!("thread_late_joiner_withheld_open_settled=ok");
    if first == SettledPane::Failed {
        // Retry is the same typed command; it must pass through Opening and
        // settle again (Failed, or Open once the first attempt's reply page
        // is cached), never loop or report success over an unhydrated actor.
        let retry = open_thread_until_settled(conn, room_id, root_event_id, "retry").await?;
        eprintln!("thread_late_joiner withheld retry settled as {retry:?}");
    }
    let close_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CloseThread {
        request_id: close_id,
    }))
    .await
    .map_err(|e| format!("thread late joiner: submit close thread failed: {e}"))?;
    wait_for_snapshot(conn, "thread late joiner B fresh close", |state| {
        matches!(state.thread, koushi_state::ThreadPaneState::Closed).then_some(())
    })
    .await?;
    println!("thread_late_joiner_withheld_retry_close=ok");
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettledPane {
    Open,
    Failed,
}

async fn open_thread_until_settled(
    conn: &mut CoreConnection,
    room_id: &str,
    root_event_id: &str,
    phase: &str,
) -> Result<SettledPane, String> {
    use koushi_state::ThreadPaneState;
    let request_id = conn.next_request_id();
    // Admission means the reducer has applied OpenThread (pane = Opening), so a
    // previous Failed pane cannot be mistaken for this attempt's outcome.
    conn.command_with_admission(CoreCommand::App(AppCommand::OpenThread {
        request_id,
        room_id: room_id.to_owned(),
        root_event_id: root_event_id.to_owned(),
        intent: koushi_state::ThreadOpenIntent::ExistingThread,
    }))
    .await
    .map_err(|e| format!("thread late joiner: submit {phase} thread failed: {e}"))?;
    let settled =
        wait_for_snapshot(
            conn,
            &format!("thread late joiner {phase}"),
            |state| match &state.thread {
                ThreadPaneState::Opening { .. } => None,
                other => Some(other.clone()),
            },
        )
        .await?;
    match settled {
        ThreadPaneState::Open {
            room_id: open_room,
            root_event_id: open_root,
            is_subscribed: true,
            ..
        } if open_room == room_id && open_root == root_event_id => Ok(SettledPane::Open),
        ThreadPaneState::Failed {
            room_id: failed_room,
            root_event_id: failed_root,
            failure_kind: OperationFailureKind::Forbidden | OperationFailureKind::NotFound,
            ..
        } if failed_room == room_id && failed_root == root_event_id => Ok(SettledPane::Failed),
        other => Err(format!(
            "thread_late_joiner failed: {phase} of a withheld-root thread settled as {}, expected Open or Failed(Forbidden|NotFound)",
            thread_pane_label(&other)
        )),
    }
}

fn thread_pane_label(state: &koushi_state::ThreadPaneState) -> String {
    use koushi_state::ThreadPaneState;
    match state {
        ThreadPaneState::Closed => "Closed".to_owned(),
        ThreadPaneState::Opening { .. } => "Opening".to_owned(),
        ThreadPaneState::Open { is_subscribed, .. } => format!("Open(subscribed={is_subscribed})"),
        ThreadPaneState::Failed { failure_kind, .. } => format!("Failed({failure_kind:?})"),
    }
}

async fn wait_for_snapshot<T>(
    conn: &mut CoreConnection,
    label: &str,
    mut matches: impl FnMut(&koushi_state::AppState) -> Option<T>,
) -> Result<T, String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if let Some(value) = matches(&conn.snapshot()) {
            return Ok(value);
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for state"))?
            .map_err(|lag| format!("{label}: event stream lagged ({})", lag.skipped))?;
    }
}

async fn set_history_visibility_joined(
    conn: &mut CoreConnection,
    room_id: &str,
) -> Result<(), String> {
    load_room_settings_for_qa(conn, room_id, "thread late joiner settings").await?;
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::UpdateRoomSetting {
        request_id,
        room_id: room_id.to_owned(),
        change: RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Joined),
    }))
    .await
    .map_err(|e| format!("thread late joiner: submit history visibility failed: {e}"))?;
    loop {
        let event = tokio::time::timeout(EVENT_TIMEOUT, conn.recv_event())
            .await
            .map_err(|_| "thread late joiner: history visibility update timed out".to_owned())?
            .map_err(|lag| format!("thread late joiner: event stream lagged ({})", lag.skipped))?;
        match event {
            CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                request_id: ev_id,
                settings,
            }) if ev_id == request_id => {
                return if settings.history_visibility == RoomHistoryVisibility::Joined {
                    Ok(())
                } else {
                    Err("thread late joiner: history visibility did not become joined".to_owned())
                };
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!(
                    "thread late joiner: history visibility update failed: {failure:?}"
                ));
            }
            _ => {}
        }
    }
}

async fn send_text(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        explicit_mentions: None,
        request_id,
        key: key.clone(),
        transaction_id: transaction_id.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
    }))
    .await
    .map_err(|e| format!("{label}: send submission failed: {e}"))?;
    Ok(
        wait_for_send_flow_completion(conn, request_id, key, transaction_id, body, label)
            .await?
            .event_id,
    )
}

async fn send_thread_reply(
    conn: &mut CoreConnection,
    thread_key: &TimelineKey,
    root_event_id: &str,
    transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendReply {
        explicit_mentions: None,
        request_id,
        key: thread_key.clone(),
        transaction_id: transaction_id.to_owned(),
        in_reply_to_event_id: root_event_id.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
    }))
    .await
    .map_err(|e| format!("{label}: send submission failed: {e}"))?;
    let (_transaction_id, event_id) =
        wait_for_send_completed(conn, request_id, thread_key, label).await?;
    Ok(event_id)
}

/// Observes the Room display row whose content is the thread root until it
/// leaves the pending state, and returns its terminal display kind and the
/// exported `is_hidden` of that row.
async fn wait_for_terminal_root_row(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial: &[TimelineItem],
    root_event_id: &str,
    activity_event_id: &str,
) -> Result<(TimelineDisplayKind, bool), String> {
    let terminal_kind = |item: &TimelineItem| {
        let metadata = item.display_metadata.as_ref()?;
        (metadata.content_event_id.as_deref() == Some(root_event_id)
            && !matches!(
                metadata.kind,
                TimelineDisplayKind::Event | TimelineDisplayKind::ThreadRootPending
            ))
        .then_some((metadata.kind, item.is_hidden))
    };
    if let Some(kind) = initial.iter().find_map(terminal_kind) {
        return Ok(kind);
    }
    let mut saw_activity = initial
        .iter()
        .any(|item| timeline_item_event_id(item) == Some(activity_event_id));
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "thread_late_joiner failed: root row never became terminal (saw_activity={saw_activity})"
                )
            })?
            .map_err(|lag| format!("thread late joiner: event stream lagged ({})", lag.skipped))?;
        let mut found = None;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                found = items.iter().find_map(terminal_kind);
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    saw_activity |= timeline_item_event_id(item) == Some(activity_event_id);
                    if found.is_none() {
                        found = terminal_kind(item);
                    }
                    Ok(())
                })?;
            }
            _ => {}
        }
        if let Some(kind) = found {
            return Ok(kind);
        }
    }
}

async fn wait_for_event_in_timeline(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    initial: &[TimelineItem],
    event_id: &str,
    label: &str,
) -> Result<(), String> {
    if initial
        .iter()
        .any(|item| timeline_item_event_id(item) == Some(event_id))
    {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(remaining, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: visible reply never appeared in the thread"))?
            .map_err(|lag| format!("{label}: event stream lagged ({})", lag.skipped))?;
        let mut found = false;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: ref ev_key,
                items,
                ..
            }) if ev_key == key => {
                found = items
                    .iter()
                    .any(|item| timeline_item_event_id(item) == Some(event_id));
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: ref ev_key,
                diffs,
                ..
            }) if ev_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    found |= timeline_item_event_id(item) == Some(event_id);
                    Ok(())
                })?;
            }
            CoreEvent::OperationFailed { failure, .. } => {
                return Err(format!("{label}: operation failed: {failure:?}"));
            }
            _ => {}
        }
        if found {
            return Ok(());
        }
    }
}
