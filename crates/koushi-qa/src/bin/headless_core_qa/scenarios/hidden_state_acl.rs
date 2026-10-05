//! #1117 (from #1110): one ordinary message followed by repeated
//! `m.room.server_acl` updates against a disposable homeserver.
//!
//! The sender's second SDK session (a disposable auditor device of the room
//! creator) writes the ACL state, so every update is real homeserver state that
//! reaches the reader through sync. The reader must then:
//!
//! - export every observed ACL update as a hidden item, never a resurrected
//!   blank row (a lagged stream is re-observed through a replay);
//! - keep the room list's latest event and unread count on the message;
//! - keep the ordinary message in a re-subscription replay even though more
//!   hidden updates than the replay capacity follow it;
//! - show a message sent after the burst in the timeline and the room list;
//! - acknowledge that message from the viewport, converging to the
//!   server-confirmed read boundary and a cleared unread count.
//!
//! A fresh subscription after `Unsubscribe` is not covered: its initial
//! hydration reads a bounded event count once, so more hidden updates than that
//! window hide the message. That is tracked as #1125; the executable RED
//! reproducer is
//! `crates/koushi-core/src/timeline/actor/fresh_room_hydration_tests.rs`
//! (`cargo test -p koushi-core --lib -- --ignored fresh_room_subscription`).

use std::collections::HashSet;
use std::time::Duration;

use koushi_state::RoomSummary;

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    QaEventDeadline, drain_queued_events_from_source, subscribe_timeline_for_qa,
    visit_timeline_diff_items, wait_for_invite_in_snapshot,
    wait_for_room_list_publication_from_source,
};
use super::fixtures::{accept_invite_for_qa, create_room_for_qa, invite_user_for_qa};
use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::scenario_identity::cleanup_qa_auditor_device;
use super::scenario_read_state::{observe_viewport, send_text_and_wait_event};
use super::{
    CoreCommand, CoreConnection, CoreEvent, RoomEvent, TimelineCommand, TimelineEvent,
    TimelineItem, TimelineItemId, TimelineKey, TimelineKind, TimelineReadStateSync,
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
const FOLLOW_UP_BODY: &str = "Synthetic message after hidden state";

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
    wait_for_room(
        reader,
        &room_id,
        |room| room.unread_count > 0,
        "hidden-state acl unread",
    )
    .await?;

    // Empty the queue before writing the burst, so the publication counted
    // below cannot be one that was merely still queued. `CoreConnection`
    // exposes no burst marker, so an ordered pre-write drain is the strongest
    // attribution available.
    drain_queued_events_from_source(
        reader,
        "hidden-state acl pre-burst drain",
        Duration::from_millis(20),
        EVENT_TIMEOUT,
    )
    .await?;
    let acl_event_ids = send_acl_updates(auditor, &room_id).await?;
    let acl_set: HashSet<&str> = acl_event_ids.iter().map(String::as_str).collect();
    let last_acl = acl_event_ids
        .last()
        .ok_or_else(|| "hidden-state acl: no ACL update was sent".to_owned())?
        .clone();
    // Every observed ACL item must stay hidden; the last one proves arrival.
    // A lagged stream is re-observed through a replay inside the wait.
    let mut acl_observation = AclObservation::default();
    let wait = wait_for_reader_items(
        reader,
        &key,
        "hidden-state acl reader receives updates",
        |item| {
            acl_observation.observe(item, &acl_set);
            item_event_id(item) == Some(last_acl.as_str())
        },
    )
    .await?;
    acl_observation.require_all_hidden()?;
    // Hidden updates must not move the room list off the message. The summary
    // already matched before the burst, so require a room-list publication
    // observed after the burst before accepting it: the queue was emptied
    // before the writes and the item wait counts publications instead of
    // discarding them, so a counted publication was judged after them.
    //
    // Limit: this observes that, after the pre-write drain, an explicit
    // room-list publication arrived and the current summary still points at the
    // message. Publication causality (the wake could have been produced by an
    // earlier update in the burst) and applied-projection freshness (Core emits
    // `RoomListUpdated` when the projection enqueues its reducer actions and
    // exposes no per-burst revision) are not established.
    let summary_has_message = |room: &RoomSummary| {
        room.unread_count > 0
            && room
                .latest_event
                .as_ref()
                .is_some_and(|latest| latest.event_id == message_event_id)
    };
    let snapshot_has_message = |snapshot: &koushi_state::AppState| {
        snapshot
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && summary_has_message(room))
    };
    wait_for_room_list_publication_from_source(
        reader,
        wait.room_list_publications,
        snapshot_has_message,
        "hidden-state acl room list after updates",
        EVENT_TIMEOUT,
    )
    .await?;
    wait_for_room(
        reader,
        &room_id,
        summary_has_message,
        "hidden-state acl room list after updates",
    )
    .await?;

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

    // A message after the burst reaches the timeline and the room list.
    let follow_up_event_id = send_text_and_wait_event(
        sender,
        &sender_key,
        "hidden-state-follow-up",
        FOLLOW_UP_BODY,
        "hidden-state acl follow-up",
    )
    .await?;
    wait_for_reader_items(
        reader,
        &key,
        "hidden-state acl reader receives follow-up",
        |item| {
            item_event_id(item) == Some(follow_up_event_id.as_str())
                && !item.is_hidden
                && item.body.as_deref() == Some(FOLLOW_UP_BODY)
        },
    )
    .await?;
    wait_for_room(
        reader,
        &room_id,
        |room| {
            room.unread_count > 0
                && room
                    .latest_event
                    .as_ref()
                    .is_some_and(|latest| latest.event_id == follow_up_event_id)
        },
        "hidden-state acl room list after follow-up",
    )
    .await?;

    // The viewport reports the newest visible message.
    observe_viewport(reader, &key, &follow_up_event_id).await?;
    wait_for_synced_read(reader, &key, &follow_up_event_id).await?;
    wait_for_room(
        reader,
        &room_id,
        |room| room.unread_count == 0,
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

/// Counts the ACL updates observed on the event stream and records any that
/// were exported as visible rows.
#[derive(Default)]
struct AclObservation {
    seen: HashSet<String>,
    visible: HashSet<String>,
}

impl AclObservation {
    fn observe(&mut self, item: &TimelineItem, acl_set: &HashSet<&str>) {
        let Some(id) = item_event_id(item) else {
            return;
        };
        if !acl_set.contains(id) {
            return;
        }
        self.seen.insert(id.to_owned());
        if !item.is_hidden {
            self.visible.insert(id.to_owned());
        }
    }

    fn require_all_hidden(&self) -> Result<(), String> {
        if !self.visible.is_empty() {
            return Err(format!(
                "hidden-state acl: {} of {} observed ACL updates were exported as visible rows",
                self.visible.len(),
                self.seen.len()
            ));
        }
        if self.seen.is_empty() {
            return Err("hidden-state acl: no ACL update was observed".to_owned());
        }
        Ok(())
    }
}

/// What a reader-item wait observed besides the items themselves.
#[derive(Default)]
struct ReaderItemWait {
    /// `RoomListUpdated` publications observed while waiting.
    room_list_publications: u64,
    /// Events the stream skipped.
    skipped: u64,
}

/// Feed one reader event to `visit`, counting room-list publications and
/// timeline items for `key`.
fn observe_reader_event(
    event: CoreEvent,
    key: &TimelineKey,
    wait: &mut ReaderItemWait,
    mut visit: impl FnMut(&TimelineItem),
) -> Result<(), String> {
    match event {
        CoreEvent::Room(RoomEvent::RoomListUpdated) => wait.room_list_publications += 1,
        CoreEvent::Timeline(TimelineEvent::InitialItems {
            key: event_key,
            items,
            ..
        }) if event_key == *key => {
            for item in &items {
                visit(item);
            }
        }
        CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: event_key,
            diffs,
            ..
        }) if event_key == *key => {
            visit_timeline_diff_items(&diffs, |item| {
                visit(item);
                Ok(())
            })?;
        }
        _ => {}
    }
    Ok(())
}

/// Waits until `predicate` matches an item exported for `key`, also counting
/// room-list publications and stream lag so a caller can fence on them. The ACL
/// burst can outrun the event stream; after a lag the Core-held timeline is
/// re-observed and the predicate sees the replayed items before waiting
/// resumes.
async fn wait_for_reader_items(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
    mut predicate: impl FnMut(&TimelineItem) -> bool,
) -> Result<ReaderItemWait, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let mut wait = ReaderItemWait::default();
    loop {
        let event = match deadline.recv(conn).await {
            Err(_) => {
                return Err(format!(
                    "{label}: timed out (skipped={}, room_list_publications={})",
                    wait.skipped, wait.room_list_publications
                ));
            }
            Ok(Ok(event)) => event,
            Ok(Err(lag)) => {
                // Lag skips events, so the Core-held timeline is re-observed
                // authoritatively under the same deadline instead of continuing
                // as if nothing were missed.
                wait.skipped += lag.skipped;
                if reobserve_replay_snapshot(conn, key, label, deadline, &mut wait, &mut predicate)
                    .await?
                {
                    return Ok(wait);
                }
                continue;
            }
        };
        let mut matched = false;
        observe_reader_event(event, key, &mut wait, |item| matched |= predicate(item))?;
        if matched {
            return Ok(wait);
        }
    }
}

/// Re-observe the Core-held timeline after a lag: request a fresh subscription
/// snapshot and feed every item the actor exports (diffs and initial items that
/// arrive while the snapshot is awaited) into `predicate`, counting room-list
/// publications and lag into `wait`.
///
/// Command submission and the snapshot wait both run under the caller's
/// absolute `deadline`, and the correlated snapshot is always awaited before
/// returning, so a hit on an intervening diff cannot stand in for the
/// authoritative re-observation. A timeout is a failure, never a silent
/// continue.
async fn reobserve_replay_snapshot(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    label: &str,
    deadline: QaEventDeadline,
    wait: &mut ReaderItemWait,
    predicate: &mut impl FnMut(&TimelineItem) -> bool,
) -> Result<bool, String> {
    let request_id = conn.next_request_id();
    let initial_backfill = if matches!(key.kind, TimelineKind::Thread { .. }) {
        koushi_protocol::command::InitialBackfillPolicy::RequiredForExistingThread
    } else {
        koushi_protocol::command::InitialBackfillPolicy::Disabled
    };
    tokio::time::timeout_at(
        deadline.instant,
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id,
            key: key.clone(),
            initial_backfill,
        })),
    )
    .await
    .map_err(|_| {
        format!(
            "{label}: replay subscribe timed out (skipped={})",
            wait.skipped
        )
    })?
    .map_err(|_| {
        format!(
            "{label}: replay subscribe failed (skipped={})",
            wait.skipped
        )
    })?;
    let mut matched = false;
    loop {
        let event = match deadline.recv(conn).await {
            Err(_) => {
                return Err(format!(
                    "{label}: replay snapshot timed out (skipped={})",
                    wait.skipped
                ));
            }
            Ok(Ok(event)) => event,
            Ok(Err(lag)) => {
                wait.skipped += lag.skipped;
                continue;
            }
        };
        let correlated = matches!(
            &event,
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                cause_request_id: Some(cause_request_id),
                key: event_key,
                ..
            }) if event_key == key && *cause_request_id == request_id
        );
        observe_reader_event(event, key, wait, |item| matched |= predicate(item))?;
        if correlated {
            return Ok(matched);
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
            Err(lag) => {
                return Err(format!(
                    "hidden-state acl: read boundary event stream lagged (skipped={})",
                    lag.skipped
                ));
            }
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
async fn wait_for_room(
    conn: &mut CoreConnection,
    room_id: &str,
    predicate: impl Fn(&RoomSummary) -> bool,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if conn
            .snapshot()
            .rooms
            .iter()
            .any(|room| room.room_id == room_id && predicate(room))
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("{label}: room summary did not converge"));
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), conn.recv_event()).await;
    }
}
