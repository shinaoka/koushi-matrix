//! Issue #1255: receipt summaries are scoped by the timeline that observed
//! them.
//!
//! A Room timeline publishes unthreaded (`ReceiptScope::Main`) summaries, a
//! permalink/context timeline publishes its own `ReceiptScope::Focused`
//! summaries, and a Thread timeline publishes `ReceiptScope::Thread` summaries.
//! Two actors that observe the same event ID in different scopes must not
//! overwrite each other, and a later actor start must reconcile its scope over
//! its own initial window so a retired actor's stale readers disappear.
//!
//! All identifiers are synthetic (`example.invalid`).

use std::collections::BTreeMap;

use koushi_state::{
    AppAction, AppState, LiveEventReceiptSummary, LiveEventReceiptSummaryUpdate, LiveEventReceipts,
    LiveReadReceipt, ReceiptScope, SessionInfo, SessionState, reduce,
};

const ROOM_ID: &str = "!room:example.invalid";
const OWN_USER_ID: &str = "@own:example.invalid";
const ROOT_EVENT_ID: &str = "$root:example.invalid";
const REPLY_EVENT_ID: &str = "$reply:example.invalid";
const OTHER_EVENT_ID: &str = "$other:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: OWN_USER_ID.to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn reader(user_id: &str, timestamp_ms: u64) -> LiveReadReceipt {
    LiveReadReceipt {
        user_id: user_id.to_owned(),
        display_name: Some(user_id.to_owned()),
        original_display_label: String::new(),
        avatar: None,
        timestamp_ms: Some(timestamp_ms),
    }
}

fn summary_update(event_id: &str, readers: Vec<LiveReadReceipt>) -> LiveEventReceiptSummaryUpdate {
    LiveEventReceiptSummaryUpdate {
        event_id: event_id.to_owned(),
        total_count: readers.len() as u64,
        readers,
    }
}

fn publish_summaries(
    state: &mut AppState,
    scope: ReceiptScope,
    scoped_event_ids: Vec<String>,
    updates: Vec<LiveEventReceiptSummaryUpdate>,
) {
    reduce(
        state,
        AppAction::LiveRoomReceiptSummariesUpdated {
            room_id: ROOM_ID.to_owned(),
            scope,
            scoped_event_ids,
            receipts_by_event: updates,
        },
    );
}

fn publish_reconcile(
    state: &mut AppState,
    scope: ReceiptScope,
    scoped_event_ids: Vec<String>,
    receipts_by_event: Vec<LiveEventReceipts>,
) {
    reduce(
        state,
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: ROOM_ID.to_owned(),
            scope,
            scoped_event_ids,
            receipts_by_event,
        },
    );
}

fn reader_ids(state: &AppState, scope: &ReceiptScope, event_id: &str) -> Vec<String> {
    scoped_receipts(state, scope)
        .and_then(|receipts| receipts.get(event_id))
        .map(|summary| {
            summary
                .readers
                .iter()
                .map(|reader| reader.user_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn scoped_receipts<'a>(
    state: &'a AppState,
    scope: &ReceiptScope,
) -> Option<&'a BTreeMap<String, LiveEventReceiptSummary>> {
    let room = state.live_signals.rooms.get(ROOM_ID)?;
    match scope {
        ReceiptScope::Main => Some(&room.receipts_by_event),
        ReceiptScope::Focused { event_id } => room.focused_receipts_by_event.get(event_id),
        ReceiptScope::Thread { root_event_id } => room.thread_receipts_by_event.get(root_event_id),
    }
}

fn main_scope() -> ReceiptScope {
    ReceiptScope::Main
}

fn thread_scope() -> ReceiptScope {
    ReceiptScope::Thread {
        root_event_id: ROOT_EVENT_ID.to_owned(),
    }
}

fn focused_scope(target_event_id: &str) -> ReceiptScope {
    ReceiptScope::Focused {
        event_id: target_event_id.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// 1. Room and Thread scopes for the same thread-reply event ID
// ---------------------------------------------------------------------------

#[test]
fn room_and_thread_scope_summaries_do_not_overwrite_each_other_room_first() {
    let mut state = ready_state();

    // The Room timeline (unthreaded) observed its reader on the thread reply.
    publish_summaries(
        &mut state,
        main_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@room-reader:example.invalid", 100)],
        )],
    );
    // The Thread timeline observed a different reader on the same reply.
    publish_summaries(
        &mut state,
        thread_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@thread-reader:example.invalid", 200)],
        )],
    );

    assert_eq!(
        reader_ids(&state, &main_scope(), REPLY_EVENT_ID),
        vec!["@room-reader:example.invalid".to_owned()],
        "the main scope must keep the Room timeline's readers"
    );
    assert_eq!(
        reader_ids(&state, &thread_scope(), REPLY_EVENT_ID),
        vec!["@thread-reader:example.invalid".to_owned()],
        "the thread scope must keep the Thread timeline's readers"
    );
}

#[test]
fn room_and_thread_scope_summaries_do_not_overwrite_each_other_thread_first() {
    let mut state = ready_state();

    // The Thread timeline observed its reader first.
    publish_summaries(
        &mut state,
        thread_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@thread-reader:example.invalid", 200)],
        )],
    );
    // The Room timeline then observed its reader on the same reply.
    publish_summaries(
        &mut state,
        main_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@room-reader:example.invalid", 100)],
        )],
    );

    assert_eq!(
        reader_ids(&state, &main_scope(), REPLY_EVENT_ID),
        vec!["@room-reader:example.invalid".to_owned()],
        "the main scope must keep the Room timeline's readers"
    );
    assert_eq!(
        reader_ids(&state, &thread_scope(), REPLY_EVENT_ID),
        vec!["@thread-reader:example.invalid".to_owned()],
        "the thread scope must keep the Thread timeline's readers"
    );
}

// ---------------------------------------------------------------------------
// 2. An actor start reconcile removes stale summaries in its own window
// ---------------------------------------------------------------------------

#[test]
fn actor_start_reconcile_removes_stale_reader_summary() {
    let mut state = ready_state();

    // A retired actor left a reader summary on the reply event.
    publish_summaries(
        &mut state,
        main_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@retired-reader:example.invalid", 100)],
        )],
    );
    assert_eq!(
        reader_ids(&state, &main_scope(), REPLY_EVENT_ID),
        vec!["@retired-reader:example.invalid".to_owned()],
        "the retired actor's summary is present before the new actor starts"
    );

    // The new actor's initial window still contains the reply, but its receipt
    // snapshot no longer names the retired reader: the reader moved away, so the
    // SDK reports no receipt entry for that event at all. The actor start
    // reconciles its scope over that window instead of merging into it.
    publish_summaries(
        &mut state,
        main_scope(),
        vec![REPLY_EVENT_ID.to_owned()],
        Vec::new(),
    );

    assert!(
        scoped_receipts(&state, &main_scope())
            .is_none_or(|receipts| !receipts.contains_key(REPLY_EVENT_ID)),
        "the stale reader summary must be removed by the actor-start reconcile"
    );
}

#[test]
fn actor_start_reconcile_preserves_events_outside_its_window() {
    let mut state = ready_state();

    publish_summaries(
        &mut state,
        main_scope(),
        Vec::new(),
        vec![
            summary_update(
                REPLY_EVENT_ID,
                vec![reader("@retired-reader:example.invalid", 100)],
            ),
            summary_update(
                OTHER_EVENT_ID,
                vec![reader("@outside-reader:example.invalid", 50)],
            ),
        ],
    );

    // The new actor's window names only the reply event.
    publish_summaries(
        &mut state,
        main_scope(),
        vec![REPLY_EVENT_ID.to_owned()],
        Vec::new(),
    );

    assert_eq!(
        reader_ids(&state, &main_scope(), OTHER_EVENT_ID),
        vec!["@outside-reader:example.invalid".to_owned()],
        "receipt state outside the reconcile window must be preserved"
    );
    assert!(
        scoped_receipts(&state, &main_scope())
            .is_none_or(|receipts| !receipts.contains_key(REPLY_EVENT_ID))
    );
}

// ---------------------------------------------------------------------------
// 3. Focused scope isolation
// ---------------------------------------------------------------------------

#[test]
fn focused_scope_publishes_without_touching_main_or_thread() {
    let mut state = ready_state();

    publish_summaries(
        &mut state,
        main_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@room-reader:example.invalid", 100)],
        )],
    );
    publish_summaries(
        &mut state,
        thread_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@thread-reader:example.invalid", 200)],
        )],
    );

    // A permalink/context timeline publishes its own scope.
    publish_summaries(
        &mut state,
        focused_scope(REPLY_EVENT_ID),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@focused-reader:example.invalid", 300)],
        )],
    );

    assert_eq!(
        reader_ids(&state, &main_scope(), REPLY_EVENT_ID),
        vec!["@room-reader:example.invalid".to_owned()],
        "a focused publication must leave the main scope untouched"
    );
    assert_eq!(
        reader_ids(&state, &thread_scope(), REPLY_EVENT_ID),
        vec!["@thread-reader:example.invalid".to_owned()],
        "a focused publication must leave the thread scope untouched"
    );
    assert_eq!(
        reader_ids(&state, &focused_scope(REPLY_EVENT_ID), REPLY_EVENT_ID),
        vec!["@focused-reader:example.invalid".to_owned()],
    );
}

#[test]
fn focused_permalink_on_thread_reply_cannot_change_thread_scope() {
    let mut state = ready_state();

    // The Thread timeline owns the threaded readers of its root.
    publish_summaries(
        &mut state,
        thread_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@thread-reader:example.invalid", 200)],
        )],
    );

    // The focused permalink target is itself a thread reply. The SDK resolves
    // that timeline's receipts to the thread, but the focused timeline's own
    // scope keys on the permalink target, not on the thread root.
    publish_summaries(
        &mut state,
        focused_scope(REPLY_EVENT_ID),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@focused-reader:example.invalid", 300)],
        )],
    );

    assert_eq!(
        reader_ids(&state, &thread_scope(), REPLY_EVENT_ID),
        vec!["@thread-reader:example.invalid".to_owned()],
        "a focused permalink on a thread reply must not rewrite the thread scope"
    );
    assert_eq!(
        reader_ids(&state, &focused_scope(REPLY_EVENT_ID), REPLY_EVENT_ID),
        vec!["@focused-reader:example.invalid".to_owned()],
    );
    assert!(
        scoped_receipts(&state, &main_scope())
            .is_none_or(|receipts| !receipts.contains_key(REPLY_EVENT_ID)),
        "a focused permalink must not write the main scope"
    );
}

#[test]
fn reconciliation_replaces_only_its_own_scope() {
    let mut state = ready_state();

    publish_summaries(
        &mut state,
        thread_scope(),
        Vec::new(),
        vec![summary_update(
            REPLY_EVENT_ID,
            vec![reader("@thread-reader:example.invalid", 200)],
        )],
    );

    // A Room-window reconcile names the same event ID.
    publish_reconcile(
        &mut state,
        main_scope(),
        vec![REPLY_EVENT_ID.to_owned()],
        vec![LiveEventReceipts {
            event_id: REPLY_EVENT_ID.to_owned(),
            receipts: vec![reader("@room-reader:example.invalid", 100)],
        }],
    );

    assert_eq!(
        reader_ids(&state, &main_scope(), REPLY_EVENT_ID),
        vec!["@room-reader:example.invalid".to_owned()],
    );
    assert_eq!(
        reader_ids(&state, &thread_scope(), REPLY_EVENT_ID),
        vec!["@thread-reader:example.invalid".to_owned()],
        "a main-scope reconcile must preserve the thread scope"
    );
}
