//! #1696 `thread_receipt_fresh_session`: an account whose read receipts already
//! cover a whole thread before a Koushi session signs in must not see that
//! thread as unread in the new session.
//!
//! B posts a main message, a thread root and five replies, so the newest event
//! in each room is reply 5. A second, raw SDK session of A then sets A's
//! receipts through the client-server API, the way another client does:
//!
//! - the unthreaded room: an unthreaded `m.read` on reply 5 only (Element's
//!   "Mark as read" targets the newest event, a thread reply). By the spec an
//!   unthreaded receipt reads every earlier event, thread replies included.
//! - the clash room: the same unthreaded `m.read` plus `m.read` with
//!   `thread_id` = root on the same reply 5 (Mark as read, then reading the
//!   thread). Synapse's sliding sync merges a user's clashing receipts into the
//!   unthreaded one for a new connection (MSC4102), so a fresh session receives
//!   only the unthreaded receipt there.
//!
//! A fresh A device then signs in and opens each room and its thread (which
//! loads every thread event). Neither the thread chip, the room's thread unread
//! nor its badge may become non-zero while both stay open for a bounded window. A then reads the thread in the clash room,
//! and after a runtime restart (store restore) both rooms are checked again.
//! Every check runs and reports its own token; the scenario fails at the end if
//! any check failed, so one run shows every broken stage.

use std::collections::HashSet;
use std::time::Duration;

use matrix_sdk::ruma::{
    OwnedEventId, OwnedRoomId,
    api::client::{receipt::create_receipt, sync::sync_events},
    events::receipt::ReceiptThread,
};

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    contains_root_thread, root_thread_unread, subscribe_timeline_for_qa, timeline_item_event_id,
    visit_timeline_diff_items, wait_for_invite_in_snapshot, wait_for_ready_snapshot,
    wait_for_room_in_room_list, wait_for_send_completed, wait_for_send_flow_completion,
    wait_for_session_restored,
};
use super::fixtures::{accept_invite_for_qa, create_room_for_qa, invite_user_for_qa};
use super::participants::{
    QaParticipantLoginGate, QaParticipantLoginOutcome, login_synced_participant_for_qa,
    login_synced_participant_on_runtime_for_qa, qa_data_dir, start_isolated_qa_runtime,
};
use super::registry::QaConfig;
use super::scenario_identity::cleanup_qa_auditor_device;
use super::{
    AccountCommand, AccountKey, CoreCommand, CoreConnection, CoreEvent, PaginationDirection,
    TimelineCommand, TimelineEvent, TimelineItem, TimelineKey, TimelineKind,
};

const MAIN_BODY: &str = "Synthetic fresh-session main message";
const ROOT_BODY: &str = "Synthetic fresh-session thread root";
const REPLY_COUNT: u32 = 5;
const FRESH_DEVICE_SUFFIX: &str = "thread-receipt-fresh-a2";
/// How long an open room must stay free of thread unread. Opening the room is
/// what loads the thread and produced the count in #1696, within seconds.
const OBSERVATION_WINDOW: Duration = Duration::from_secs(20);
const THREAD_PAGINATE_INTERVAL: Duration = Duration::from_secs(2);

/// One room's thread fixture as seen by the reader.
struct ThreadRoom {
    label: &'static str,
    room_id: String,
    root_event_id: String,
    last_reply_event_id: String,
}

pub(super) async fn run_thread_receipt_fresh_session_scenario(
    config: &QaConfig,
) -> Result<(), String> {
    let participant_a = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("thread-receipt-fresh-a"),
        &config.user_a,
        &config.password_a,
        "Koushi Thread Receipt QA A",
        "thread receipt fresh login A",
        "thread receipt fresh gate A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let participant_b = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("thread-receipt-fresh-b"),
        &config.user_b,
        &config.password_b,
        "Koushi Thread Receipt QA B",
        "thread receipt fresh login B",
        "thread receipt fresh gate B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let QaParticipantLoginOutcome {
        runtime: runtime_a,
        conn: mut conn_a,
        account_key: account_key_a,
        bootstrap_recovery_secret: recovery_secret_a,
    } = participant_a;
    let QaParticipantLoginOutcome {
        runtime: runtime_b,
        conn: mut conn_b,
        account_key: account_key_b,
        ..
    } = participant_b;

    let flow = match recovery_secret_a.as_ref() {
        Some(recovery_secret) => {
            run_flow(
                config,
                recovery_secret,
                &mut conn_a,
                &mut conn_b,
                &account_key_b,
            )
            .await
        }
        None => Err("thread receipt fresh: A login returned no recovery secret".to_owned()),
    };

    let cleanup_b = cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "thread receipt fresh cleanup B",
    )
    .await;
    let cleanup_a = cleanup_logged_in_runtime(
        conn_a,
        runtime_a,
        account_key_a,
        "thread receipt fresh cleanup A",
    )
    .await;
    flow?;
    cleanup_b.and(cleanup_a)?;
    println!("thread_receipt_fresh_session=ok");
    Ok(())
}

async fn run_flow(
    config: &QaConfig,
    recovery_secret_a: &koushi_state::AuthSecret,
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_b: &AccountKey,
) -> Result<(), String> {
    let user_a_id = format!("@{}:{}", config.user_a, config.server_name);
    let unthreaded = post_thread_room(
        conn_a,
        conn_b,
        account_key_b,
        &user_a_id,
        "unthreaded",
        "Synthetic Unthreaded Receipt Room",
    )
    .await?;
    let clash = post_thread_room(
        conn_a,
        conn_b,
        account_key_b,
        &user_a_id,
        "clash",
        "Synthetic Clashing Receipt Room",
    )
    .await?;

    set_receipts_from_another_client(config, &unthreaded, &clash).await?;

    // A fresh A device signs in only now, after the receipts were set.
    let QaParticipantLoginOutcome {
        runtime,
        mut conn,
        account_key,
        ..
    } = login_synced_participant_on_runtime_for_qa(
        start_isolated_qa_runtime(FRESH_DEVICE_SUFFIX)?,
        &config.homeserver,
        &config.user_a,
        &config.password_a,
        "Koushi Thread Receipt QA A fresh",
        "thread receipt fresh login A fresh",
        "thread receipt fresh gate A fresh",
        QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret_a),
    )
    .await?;
    let mut failures = Vec::new();
    let flow =
        run_fresh_session_checks(&mut conn, &account_key, &unthreaded, &clash, &mut failures).await;
    // Restart the fresh device's runtime and restore its session from the store.
    let flow = match flow {
        Ok(()) => {
            drop(conn);
            runtime.shutdown().await;
            let restarted = start_isolated_qa_runtime(FRESH_DEVICE_SUFFIX)?;
            let mut restarted_conn = restarted.attach();
            let flow = run_restart_checks(
                &mut restarted_conn,
                &account_key,
                &unthreaded,
                &clash,
                &mut failures,
            )
            .await;
            let cleanup = cleanup_logged_in_runtime(
                restarted_conn,
                restarted,
                account_key,
                "thread receipt fresh cleanup A fresh",
            )
            .await;
            flow.and(cleanup)
        }
        Err(error) => {
            let _ = cleanup_logged_in_runtime(
                conn,
                runtime,
                account_key,
                "thread receipt fresh cleanup A fresh",
            )
            .await;
            Err(error)
        }
    };
    flow?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "thread receipt fresh failed: {}",
            failures.join("; ")
        ))
    }
}

async fn run_fresh_session_checks(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    unthreaded: &ThreadRoom,
    clash: &ThreadRoom,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    for room in [unthreaded, clash] {
        wait_for_room_in_room_list(conn, &room.room_id, "thread receipt fresh room list").await?;
    }
    record(
        failures,
        "thread_receipt_fresh_unthreaded",
        require_no_thread_unread(
            conn,
            account_key,
            unthreaded,
            "fresh open",
            Expect::NeverUnread,
        )
        .await,
    );
    record(
        failures,
        "thread_receipt_fresh_clash",
        require_no_thread_unread(conn, account_key, clash, "fresh open", Expect::NeverUnread).await,
    );

    // A reads the clash room's thread, already open, to its end.
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
        request_id,
        key: thread_key(account_key, clash),
        event_id: clash.last_reply_event_id.clone(),
    }))
    .await
    .map_err(|e| format!("thread receipt fresh: submit threaded read failed: {e}"))?;
    record(
        failures,
        "thread_receipt_fresh_read",
        require_no_thread_unread(
            conn,
            account_key,
            clash,
            "after thread read",
            Expect::SettlesRead,
        )
        .await,
    );
    Ok(())
}

async fn run_restart_checks(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    unthreaded: &ThreadRoom,
    clash: &ThreadRoom,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
        request_id,
        account_key: account_key.clone(),
    }))
    .await
    .map_err(|e| format!("thread receipt fresh: submit restore failed: {e}"))?;
    wait_for_session_restored(
        conn,
        request_id,
        account_key,
        "thread receipt fresh restore",
    )
    .await?;
    wait_for_ready_snapshot(conn, "thread receipt fresh restored Ready").await?;
    for room in [unthreaded, clash] {
        wait_for_room_in_room_list(conn, &room.room_id, "thread receipt fresh restored list")
            .await?;
    }
    let unthreaded_result = require_no_thread_unread(
        conn,
        account_key,
        unthreaded,
        "after restart",
        Expect::NeverUnread,
    )
    .await;
    let clash_result = require_no_thread_unread(
        conn,
        account_key,
        clash,
        "after restart",
        Expect::NeverUnread,
    )
    .await;
    record(
        failures,
        "thread_receipt_fresh_restart",
        unthreaded_result.and(clash_result),
    );
    Ok(())
}

fn record(failures: &mut Vec<String>, token: &str, result: Result<(), String>) {
    match result {
        Ok(()) => println!("{token}=ok"),
        Err(error) => {
            println!("{token}=failed");
            failures.push(format!("{token}: {error}"));
        }
    }
}

/// B creates a room with A, then posts a main message, a thread root and
/// `REPLY_COUNT` replies, so the newest event in the room is the last reply.
async fn post_thread_room(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_b: &AccountKey,
    user_a_id: &str,
    label: &'static str,
    room_name: &str,
) -> Result<ThreadRoom, String> {
    let room_id = create_room_for_qa(conn_b, room_name, false, "thread receipt fresh room").await?;
    invite_user_for_qa(conn_b, &room_id, user_a_id, "thread receipt fresh invite A").await?;
    wait_for_invite_in_snapshot(conn_a, &room_id, None, "thread receipt fresh invite A").await?;
    accept_invite_for_qa(conn_a, &room_id, "thread receipt fresh A joins").await?;

    let room_key = TimelineKey {
        account_key: account_key_b.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    subscribe_timeline_for_qa(conn_b, &room_key, "thread receipt fresh subscribe B").await?;
    send_text(
        conn_b,
        &room_key,
        &format!("thread-receipt-{label}-main"),
        MAIN_BODY,
    )
    .await?;
    let root_event_id = send_text(
        conn_b,
        &room_key,
        &format!("thread-receipt-{label}-root"),
        ROOT_BODY,
    )
    .await?;
    let thread_key = TimelineKey {
        account_key: account_key_b.clone(),
        kind: TimelineKind::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
    };
    subscribe_timeline_for_qa(
        conn_b,
        &thread_key,
        "thread receipt fresh subscribe B thread",
    )
    .await?;
    let mut last_reply_event_id = String::new();
    for index in 1..=REPLY_COUNT {
        let request_id = conn_b.next_request_id();
        conn_b
            .command(CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id,
                key: thread_key.clone(),
                transaction_id: format!("thread-receipt-{label}-reply-{index}"),
                in_reply_to_event_id: root_event_id.clone(),
                document: koushi_state::ComposerDocument::from_plain_text(format!(
                    "Synthetic fresh-session reply {index}"
                )),
            }))
            .await
            .map_err(|e| format!("thread receipt fresh: reply submission failed: {e}"))?;
        let (_, event_id) = wait_for_send_completed(
            conn_b,
            request_id,
            &thread_key,
            "thread receipt fresh reply",
        )
        .await?;
        last_reply_event_id = event_id;
    }
    Ok(ThreadRoom {
        label,
        room_id,
        root_event_id,
        last_reply_event_id,
    })
}

async fn send_text(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    transaction_id: &str,
    body: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: key.clone(),
        transaction_id: transaction_id.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
    }))
    .await
    .map_err(|e| format!("thread receipt fresh: send submission failed: {e}"))?;
    Ok(wait_for_send_flow_completion(
        conn,
        request_id,
        key,
        transaction_id,
        body,
        "thread receipt fresh send",
    )
    .await?
    .event_id)
}

/// Sets A's receipts from a raw SDK session of A, as another client would, and
/// removes that device again.
async fn set_receipts_from_another_client(
    config: &QaConfig,
    unthreaded: &ThreadRoom,
    clash: &ThreadRoom,
) -> Result<(), String> {
    let session = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: super::AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi Thread Receipt Writer".to_owned()),
    })
    .await
    .map_err(|_| "thread receipt fresh: receipt writer login failed".to_owned())?;
    let mut result = send_receipt(&session, unthreaded, ReceiptThread::Unthreaded).await;
    for thread in [
        ReceiptThread::Unthreaded,
        ReceiptThread::Thread(parse_event_id(&clash.root_event_id)?),
    ] {
        if result.is_ok() {
            result = send_receipt(&session, clash, thread).await;
        }
    }
    if result.is_ok() {
        print_server_receipts(&session, [unthreaded, clash]).await;
    }
    let cleanup = cleanup_qa_auditor_device(&session, &config.password_a)
        .await
        .map_err(|error| format!("thread receipt fresh: {error}"));
    let _ = koushi_sdk::close_session_stores(&session).await;
    drop(session);
    result.and(cleanup)
}

async fn send_receipt(
    session: &koushi_sdk::MatrixClientSession,
    room: &ThreadRoom,
    thread: ReceiptThread,
) -> Result<(), String> {
    let room_id: OwnedRoomId = room
        .room_id
        .as_str()
        .try_into()
        .map_err(|_| "thread receipt fresh: invalid room ID".to_owned())?;
    let mut request = create_receipt::v3::Request::new(
        room_id,
        create_receipt::v3::ReceiptType::Read,
        parse_event_id(&room.last_reply_event_id)?,
    );
    request.thread = thread;
    session
        .client()
        .send(request)
        .await
        .map(|_| ())
        .map_err(|_| format!("thread receipt fresh: {} receipt write failed", room.label))
}

/// Reports which of A's receipts on the last reply the server returns to a full
/// `/sync` of the writer session. Private-data-free: flags only.
async fn print_server_receipts(session: &koushi_sdk::MatrixClientSession, rooms: [&ThreadRoom; 2]) {
    let Ok(response) = session.client().send(sync_events::v3::Request::new()).await else {
        println!("thread_receipt_fresh_server sync=failed");
        return;
    };
    let user_id = session.client().user_id().map(ToString::to_string);
    for room in rooms {
        let (mut unthreaded, mut threaded) = (false, false);
        let joined = OwnedRoomId::try_from(room.room_id.as_str())
            .ok()
            .and_then(|room_id| response.rooms.join.get(&room_id).cloned());
        for raw in joined
            .iter()
            .flat_map(|joined| joined.ephemeral.events.iter())
        {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(raw.json().get()) else {
                continue;
            };
            let receipt = &event["content"][&room.last_reply_event_id]["m.read"]
                [user_id.as_deref().unwrap_or_default()];
            match receipt.get("thread_id").and_then(serde_json::Value::as_str) {
                Some(thread_id) if thread_id == room.root_event_id => threaded = true,
                None if receipt.is_object() => unthreaded = true,
                _ => {}
            }
        }
        println!(
            "thread_receipt_fresh_server room={} unthreaded={unthreaded} threaded={threaded}",
            room.label
        );
    }
}

fn parse_event_id(event_id: &str) -> Result<OwnedEventId, String> {
    event_id
        .try_into()
        .map_err(|_| "thread receipt fresh: invalid event ID".to_owned())
}

fn thread_key(account_key: &AccountKey, room: &ThreadRoom) -> TimelineKey {
    TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Thread {
            room_id: room.room_id.clone(),
            root_event_id: room.root_event_id.clone(),
        },
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// The chip, thread unread and badge stay zero for the whole window.
    NeverUnread,
    /// They are zero at the end of the window (a read in flight may still be
    /// clearing an earlier value).
    SettlesRead,
}

/// Opens the thread, which loads all of its events as the thread pane does, and
/// the room, then keeps both open for `OBSERVATION_WINDOW`. Fails if the thread
/// root never shows its replies (the check would be vacuous), or if the root's
/// chip, the room's thread unread or its badge breaks `expect`.
async fn require_no_thread_unread(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room: &ThreadRoom,
    phase: &str,
    expect: Expect,
) -> Result<(), String> {
    // The thread first, so that every room-window event after it is observed.
    let thread = thread_key(account_key, room);
    let thread_initial =
        subscribe_timeline_for_qa(conn, &thread, "thread receipt fresh open thread").await?;
    let key = TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Room {
            room_id: room.room_id.clone(),
        },
    };
    let initial = subscribe_timeline_for_qa(conn, &key, "thread receipt fresh open room").await?;
    let mut observed = ThreadUnreadObservation::default();
    observed.observe_thread_items(&thread_initial, &room.root_event_id);
    observed.observe_items(&initial, &room.root_event_id);
    observed.observe_snapshot(&conn.snapshot(), &room.room_id);
    let deadline = tokio::time::Instant::now() + OBSERVATION_WINDOW;
    let mut next_paginate = tokio::time::Instant::now();
    loop {
        // Scrolling the thread pane up: page the thread backwards until every
        // reply is loaded, as a reader of the whole thread does.
        if observed.thread_replies.len() < REPLY_COUNT as usize
            && tokio::time::Instant::now() >= next_paginate
        {
            let request_id = conn.next_request_id();
            conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key: thread.clone(),
                direction: PaginationDirection::Backward,
                event_count: 20,
            }))
            .await
            .map_err(|e| format!("thread receipt fresh: submit thread paginate failed: {e}"))?;
            next_paginate = tokio::time::Instant::now() + THREAD_PAGINATE_INTERVAL;
        }
        let wake = if observed.thread_replies.len() < REPLY_COUNT as usize {
            deadline.min(next_paginate)
        } else {
            deadline
        };
        let event = match tokio::time::timeout_at(wake, conn.recv_event()).await {
            Err(_) if wake < deadline => continue,
            Err(_) => break,
            Ok(Ok(event)) => event,
            Ok(Err(lag)) => {
                return Err(format!(
                    "{} {phase}: event stream lagged (skipped={})",
                    room.label, lag.skipped
                ));
            }
        };
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                items,
                ..
            }) if event_key == key => observed.observe_items(&items, &room.root_event_id),
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: event_key,
                diffs,
                ..
            }) if event_key == key => {
                visit_timeline_diff_items(&diffs, |item| {
                    observed.observe_items(std::slice::from_ref(item), &room.root_event_id);
                    Ok(())
                })?;
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                items,
                ..
            }) if event_key == thread => observed.observe_thread_items(&items, &room.root_event_id),
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: event_key,
                diffs,
                ..
            }) if event_key == thread => {
                visit_timeline_diff_items(&diffs, |item| {
                    observed.observe_thread_items(std::slice::from_ref(item), &room.root_event_id);
                    Ok(())
                })?;
            }
            _ => {}
        }
        observed.observe_snapshot(&conn.snapshot(), &room.room_id);
    }
    let ThreadUnreadObservation {
        max_reply_count,
        thread_replies,
        max_chip,
        max_thread_unread,
        max_badge,
        last_chip,
        last_thread_unread,
        last_badge,
    } = observed;
    // Private-data-free: counts only.
    println!(
        "thread_receipt_fresh_detail room={} phase={} summary_replies={max_reply_count} \
         thread_replies={} chip_max={max_chip} \
         thread_unread_max={max_thread_unread} badge_max={max_badge} chip_last={last_chip} \
         thread_unread_last={last_thread_unread} badge_last={last_badge}",
        room.label,
        phase.replace(' ', "_"),
        thread_replies.len(),
    );
    // The thread timeline is the loaded-events evidence; a server's bundled
    // summary on the root can lag behind it.
    if thread_replies.len() < REPLY_COUNT as usize {
        return Err(format!(
            "{} {phase}: the thread never loaded its {REPLY_COUNT} replies (saw {})",
            room.label,
            thread_replies.len()
        ));
    }
    let (chip, thread_unread, badge) = match expect {
        Expect::NeverUnread => (max_chip, max_thread_unread, max_badge),
        Expect::SettlesRead => (last_chip, last_thread_unread, last_badge),
    };
    if chip > 0 || thread_unread > 0 || badge > 0 {
        return Err(format!(
            "{} {phase}: a read thread showed unread (chip={chip} thread_unread={thread_unread} badge={badge})",
            room.label
        ));
    }
    Ok(())
}

#[derive(Default)]
struct ThreadUnreadObservation {
    max_reply_count: u32,
    thread_replies: HashSet<String>,
    max_chip: u32,
    max_thread_unread: u64,
    max_badge: u64,
    last_chip: u32,
    last_thread_unread: u64,
    last_badge: u64,
}

impl ThreadUnreadObservation {
    fn observe_thread_items(&mut self, items: &[TimelineItem], root_event_id: &str) {
        for item in items {
            if let Some(event_id) = timeline_item_event_id(item)
                && event_id != root_event_id
                && !item.is_hidden
            {
                self.thread_replies.insert(event_id.to_owned());
            }
        }
    }

    fn observe_items(&mut self, items: &[TimelineItem], root_event_id: &str) {
        if !contains_root_thread(items, root_event_id) {
            return;
        }
        self.last_chip = root_thread_unread(items, root_event_id);
        self.max_chip = self.max_chip.max(self.last_chip);
        let replies = items
            .iter()
            .filter_map(|item| item.thread_summary.as_ref())
            .map(|summary| summary.reply_count)
            .max()
            .unwrap_or(0);
        self.max_reply_count = self.max_reply_count.max(replies);
    }

    fn observe_snapshot(&mut self, snapshot: &koushi_state::AppState, room_id: &str) {
        if let Some(room) = snapshot.rooms.iter().find(|room| room.room_id == room_id) {
            self.last_thread_unread = room.thread_unread_count;
            self.last_badge = koushi_state::room_activity_unread_count(room);
            self.max_thread_unread = self.max_thread_unread.max(self.last_thread_unread);
            self.max_badge = self.max_badge.max(self.last_badge);
        }
    }
}
