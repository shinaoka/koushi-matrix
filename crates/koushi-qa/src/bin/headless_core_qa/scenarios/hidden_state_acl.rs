//! #1117 (from #1110): one ordinary message followed by repeated
//! `m.room.server_acl` updates against a disposable homeserver.
//!
//! The sender's second SDK session (a disposable auditor device of the room
//! creator) writes the ACL state, so every update is real homeserver state that
//! reaches the reader through sync. The reader must then:
//!
//! - receive every ACL update as a hidden item, never a resurrected blank row;
//! - keep the ordinary message in a re-subscription replay even though more
//!   hidden updates than the replay capacity follow it;
//! - acknowledge the message from a viewport that shows only that message,
//!   converging to the server-confirmed read boundary and a cleared unread
//!   count.

use std::collections::HashSet;
use std::time::Duration;

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    subscribe_timeline_for_qa, visit_timeline_diff_items, wait_for_invite_in_snapshot,
};
use super::fixtures::{accept_invite_for_qa, create_room_for_qa, invite_user_for_qa};
use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::scenario_identity::cleanup_qa_auditor_device;
use super::scenario_read_state::{observe_viewport, send_text_and_wait_event};
use super::{
    CoreConnection, CoreEvent, TimelineEvent, TimelineItem, TimelineItemId, TimelineKey,
    TimelineKind, TimelineReadStateSync,
};
use matrix_sdk::ruma::{
    OwnedRoomId,
    api::client::state::send_state_event,
    events::{EmptyStateKey, room::server_acl::RoomServerAclEventContent},
};

/// More hidden updates than the live-edge replay capacity (120 rows), so a
/// capacity that counted hidden rows would evict the message.
const ACL_UPDATE_COUNT: usize = 125;
const MESSAGE_BODY: &str = "Synthetic hidden-state message";

pub(super) async fn run_hidden_state_acl_scenario(config: &QaConfig) -> Result<(), String> {
    let participant_a = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("hidden-state-acl-a"),
        &config.user_a,
        &config.password_a,
        "Koushi Hidden State QA A",
        "hidden-state acl login A",
        "hidden-state acl gate A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let participant_b = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("hidden-state-acl-b"),
        &config.user_b,
        &config.password_b,
        "Koushi Hidden State QA B",
        "hidden-state acl login B",
        "hidden-state acl gate B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let super::participants::QaParticipantLoginOutcome {
        runtime: runtime_a,
        conn: mut conn_a,
        account_key: account_key_a,
        ..
    } = participant_a;
    let super::participants::QaParticipantLoginOutcome {
        runtime: runtime_b,
        conn: mut conn_b,
        account_key: account_key_b,
        ..
    } = participant_b;

    let flow = match koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_b.clone(),
        password: super::AuthSecret::new(config.password_b.clone()),
        device_display_name: Some("Koushi Hidden State Auditor".to_owned()),
    })
    .await
    {
        Ok(auditor) => {
            let flow = run_hidden_state_acl_flow(
                config,
                &mut conn_a,
                &account_key_a,
                &mut conn_b,
                &account_key_b,
                &auditor,
            )
            .await;
            let auditor_cleanup = cleanup_qa_auditor_device(&auditor, &config.password_b)
                .await
                .map_err(|error| format!("hidden-state acl: {error}"));
            let _ = koushi_sdk::close_session_stores(&auditor).await;
            drop(auditor);
            flow.and(auditor_cleanup)
        }
        Err(_) => Err("hidden-state acl: auditor login failed".to_owned()),
    };

    let cleanup_b = cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "hidden-state acl cleanup B",
    )
    .await;
    let cleanup_a = cleanup_logged_in_runtime(
        conn_a,
        runtime_a,
        account_key_a,
        "hidden-state acl cleanup A",
    )
    .await;
    flow?;
    cleanup_b.and(cleanup_a)?;
    println!("hidden_state_acl=ok");
    Ok(())
}

async fn run_hidden_state_acl_flow(
    config: &QaConfig,
    reader: &mut CoreConnection,
    reader_account_key: &koushi_core::AccountKey,
    sender: &mut CoreConnection,
    sender_account_key: &koushi_core::AccountKey,
    auditor: &koushi_sdk::MatrixClientSession,
) -> Result<(), String> {
    // The sender creates the room, so its auditor device may write ACL state.
    let room_id = create_room_for_qa(
        sender,
        "Synthetic Hidden State Room",
        false,
        "hidden-state acl room",
    )
    .await?;
    let reader_user_id = format!("@{}:{}", config.user_a, config.server_name);
    invite_user_for_qa(sender, &room_id, &reader_user_id, "hidden-state acl invite").await?;
    wait_for_invite_in_snapshot(reader, &room_id, None, "hidden-state acl reader invite").await?;
    accept_invite_for_qa(reader, &room_id, "hidden-state acl reader accepts invite").await?;
    let key = room_key(reader_account_key, &room_id);
    let sender_key = room_key(sender_account_key, &room_id);
    subscribe_timeline_for_qa(reader, &key, "hidden-state acl subscribe reader").await?;
    subscribe_timeline_for_qa(sender, &sender_key, "hidden-state acl subscribe sender").await?;

    let message_event_id = send_text_and_wait_event(
        sender,
        &sender_key,
        "hidden-state-message",
        MESSAGE_BODY,
        "hidden-state acl message",
    )
    .await?;
    wait_for_reader_items(
        reader,
        &key,
        "hidden-state acl reader receives message",
        |item| item_event_id(item) == Some(message_event_id.as_str()) && !item.is_hidden,
    )
    .await?;
    wait_for_unread(
        reader,
        &room_id,
        |count| count > 0,
        "hidden-state acl unread",
    )
    .await?;

    let acl_event_ids = send_acl_updates(auditor, &room_id).await?;
    let acl_set: HashSet<&str> = acl_event_ids.iter().map(String::as_str).collect();
    let last_acl = acl_event_ids
        .last()
        .ok_or_else(|| "hidden-state acl: no ACL update was sent".to_owned())?
        .clone();
    // Every exported ACL item must stay hidden; the last one proves arrival.
    let mut resurrected = false;
    wait_for_reader_items(
        reader,
        &key,
        "hidden-state acl reader receives updates",
        |item| {
            let id = item_event_id(item);
            if id.is_some_and(|id| acl_set.contains(id)) && !item.is_hidden {
                resurrected = true;
            }
            id == Some(last_acl.as_str())
        },
    )
    .await?;
    if resurrected {
        return Err("hidden-state acl: an ACL update was exported as a visible row".to_owned());
    }

    // Subscribing again replays the Core-held timeline through the live-edge
    // window, which must still contain the message.
    let replay = subscribe_timeline_for_qa(reader, &key, "hidden-state acl replay").await?;
    let message = replay
        .iter()
        .find(|item| item_event_id(item) == Some(message_event_id.as_str()))
        .ok_or_else(|| {
            let events = replay.iter().filter(|item| item_event_id(item).is_some()).count();
            let hidden = replay.iter().filter(|item| item.is_hidden).count();
            let acl = replay
                .iter()
                .filter(|item| item_event_id(item).is_some_and(|id| acl_set.contains(id)))
                .count();
            let visible_kinds: Vec<String> = replay
                .iter()
                .filter(|item| !item.is_hidden)
                .map(|item| match &item.id {
                    TimelineItemId::Event { .. } => format!("event:{:?}", item.message_kind),
                    TimelineItemId::Transaction { .. } => "transaction".to_owned(),
                    TimelineItemId::Synthetic { .. } => "synthetic".to_owned(),
                })
                .collect();
            format!(
                "hidden-state acl: hidden updates evicted the message (items={} events={events} hidden={hidden} acl={acl} visible={visible_kinds:?})",
                replay.len()
            )
        })?;
    if message.is_hidden || message.body.as_deref() != Some(MESSAGE_BODY) {
        return Err("hidden-state acl: the message is not visible after replay".to_owned());
    }
    if replay
        .iter()
        .any(|item| item_event_id(item).is_some_and(|id| acl_set.contains(id)) && !item.is_hidden)
    {
        return Err("hidden-state acl: replay exported an ACL update as a visible row".to_owned());
    }

    // The renderer can only show the message, so the viewport reports it.
    observe_viewport(reader, &key, &message_event_id).await?;
    wait_for_synced_read(reader, &key, &message_event_id).await?;
    wait_for_unread(
        reader,
        &room_id,
        |count| count == 0,
        "hidden-state acl read",
    )
    .await
}

fn room_key(account_key: &koushi_core::AccountKey, room_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.to_owned(),
        },
    }
}

fn item_event_id(item: &TimelineItem) -> Option<&str> {
    match &item.id {
        TimelineItemId::Event { event_id } => Some(event_id.as_str()),
        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
    }
}

async fn send_acl_updates(
    auditor: &koushi_sdk::MatrixClientSession,
    room_id: &str,
) -> Result<Vec<String>, String> {
    let room_id: OwnedRoomId = room_id
        .try_into()
        .map_err(|_| "hidden-state acl: invalid room ID".to_owned())?;
    let mut event_ids = Vec::with_capacity(ACL_UPDATE_COUNT);
    for index in 0..ACL_UPDATE_COUNT {
        // Distinct content per update: homeservers may deduplicate a no-op
        // state write.
        let content = RoomServerAclEventContent::new(
            false,
            vec!["*".to_owned()],
            vec![format!("blocked-{index}.example.invalid")],
        );
        let request = send_state_event::v3::Request::new(room_id.clone(), &EmptyStateKey, &content)
            .map_err(|_| "hidden-state acl: ACL request encoding failed".to_owned())?;
        let response = auditor
            .client()
            .send(request)
            .await
            .map_err(|_| "hidden-state acl: ACL state write failed".to_owned())?;
        event_ids.push(response.event_id.to_string());
    }
    Ok(event_ids)
}

/// Waits until `predicate` matches an item exported for `key`. Lag is
/// tolerated because the ACL burst can outrun the event stream; the caller
/// then relies on later items (or a fresh replay) for its evidence.
async fn wait_for_reader_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
    mut predicate: impl FnMut(&TimelineItem) -> bool,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let event = match tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out"))?
        {
            Ok(event) => event,
            Err(_lag) => continue,
        };
        let mut matched = false;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                items,
                ..
            }) if event_key == *key => {
                for item in &items {
                    matched |= predicate(item);
                }
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: event_key,
                diffs,
                ..
            }) if event_key == *key => {
                visit_timeline_diff_items(&diffs, |item| {
                    matched |= predicate(item);
                    Ok(())
                })?;
            }
            _ => {}
        }
        if matched {
            return Ok(());
        }
    }
}

async fn wait_for_synced_read(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    event_id: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        let event = match tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "hidden-state acl: read boundary did not converge".to_owned())?
        {
            Ok(event) => event,
            Err(_lag) => continue,
        };
        if let CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
            key: event_key,
            snapshot,
        }) = event
            && event_key == *key
            && snapshot.local_viewed_event_id.as_deref() == Some(event_id)
            && snapshot.server_confirmed_read_event_id.as_deref() == Some(event_id)
            && snapshot.read_state_sync == TimelineReadStateSync::Synced
        {
            return Ok(());
        }
    }
}

/// Polls the published room summary while draining the event stream.
async fn wait_for_unread(
    conn: &mut CoreConnection,
    room_id: &str,
    predicate: impl Fn(u64) -> bool,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if conn
            .snapshot()
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && predicate(room.unread_count))
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("{label}: room unread count did not converge"));
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), conn.recv_event()).await;
    }
}
