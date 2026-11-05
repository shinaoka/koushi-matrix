//! #1799: a published attention candidate lives until a canonical clear or a
//! newer live candidate, not until the next room-list observation.
//!
//! The desktop adapter reads the candidate asynchronously, so a room-list update
//! that raises nothing new must leave the candidate and its notification payload
//! published, and must not re-publish them as a second alert.

use super::*;
use crate::state::{NativeAttentionDispatchId, NativeAttentionDispatchState, RoomSummary};

const ACTIVE_ROOM: &str = "!active:example.invalid";
const NOTIFIED_ROOM: &str = "!other:example.invalid";
const NEWEST_ROOM: &str = "!newest:example.invalid";
const MUTED_ROOM: &str = "!muted:example.invalid";
const TWIN_ROOM: &str = "!twin:example.invalid";

fn active_room() -> RoomSummary {
    test_room(ACTIVE_ROOM, None)
}

fn room_without_attention(room_id: &str) -> RoomSummary {
    test_room(room_id, None)
}

/// The published room list: the notified room plus the rooms a later observation
/// keeps unchanged.
fn raised_room_list() -> Vec<RoomSummary> {
    vec![
        active_room(),
        notified_room(),
        room_without_attention(NEWEST_ROOM),
    ]
}

fn notified_room() -> RoomSummary {
    let mut room = test_room(NOTIFIED_ROOM, None);
    room.unread_count = 1;
    room.notification_count = 1;
    room.recency_stamp = Some(43);
    let mut event = latest_event("$notified:example.invalid", 43);
    event.message_text = Some("Synthetic body".to_owned());
    room.latest_event = Some(event);
    room
}

fn newest_room() -> RoomSummary {
    let mut room = test_room(NEWEST_ROOM, None);
    room.unread_count = 2;
    room.notification_count = 2;
    room.recency_stamp = Some(50);
    room.latest_event = Some(latest_event("$newest:example.invalid", 50));
    room
}

fn room_list_update(state: &mut AppState, rooms: Vec<RoomSummary>) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    )
}

fn published_room_id(state: &AppState) -> Option<&str> {
    state
        .native_attention
        .notification
        .as_ref()
        .map(|payload| payload.target.room_id.as_str())
}

/// The reported precondition: a live candidate is published for a room that is
/// not the active one, with message previews on so the payload is distinguishable
/// from the count body an unrelated observation would rebuild.
fn state_with_published_candidate() -> AppState {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    state.settings.values.notifications.message_previews = true;
    room_list_update(
        &mut state,
        vec![
            active_room(),
            room_without_attention(NOTIFIED_ROOM),
            room_without_attention(NEWEST_ROOM),
        ],
    );
    room_list_update(&mut state, raised_room_list());

    assert!(
        state.native_attention.summary.candidate.is_some(),
        "precondition: the live room-list update raises a candidate"
    );
    assert_eq!(
        published_room_id(&state),
        Some(NOTIFIED_ROOM),
        "precondition: the payload targets the notified room"
    );
    assert_eq!(
        state
            .native_attention
            .notification
            .as_ref()
            .map(|payload| payload.body.as_str()),
        Some("Synthetic body"),
        "precondition: the payload carries the verified preview"
    );
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Idle
    );
    state
}

#[test]
fn live_candidate_survives_activity_in_another_room() {
    let mut state = state_with_published_candidate();
    let candidate = state.native_attention.summary.candidate.clone();
    let payload = state.native_attention.notification.clone();

    // The user's own message in the active room: no unread count moves anywhere,
    // so this observation raises no candidate of its own.
    let mut active = active_room();
    let mut own_event = latest_event("$own:example.invalid", 44);
    own_event.sender_id = Some("@alice:example.invalid".to_owned());
    active.latest_event = Some(own_event);
    active.recency_stamp = Some(44);
    room_list_update(
        &mut state,
        vec![active, notified_room(), room_without_attention(NEWEST_ROOM)],
    );

    assert_eq!(
        state.native_attention.summary.candidate, candidate,
        "an unrelated room-list observation must not consume the candidate"
    );
    assert_eq!(
        state.native_attention.notification, payload,
        "the published payload must survive with its candidate"
    );
    assert_eq!(published_room_id(&state), Some(NOTIFIED_ROOM));
}

#[test]
fn live_candidate_survives_a_reemitted_room_list_snapshot() {
    let mut state = state_with_published_candidate();
    let candidate = state.native_attention.summary.candidate.clone();
    let payload = state.native_attention.notification.clone();

    room_list_update(&mut state, raised_room_list());

    assert_eq!(
        state.native_attention.summary.candidate, candidate,
        "the same room set re-emitted unchanged must not consume the candidate"
    );
    assert_eq!(state.native_attention.notification, payload);
}

#[test]
fn a_surviving_candidate_is_not_republished_to_the_webview() {
    let mut state = state_with_published_candidate();

    let effects = room_list_update(&mut state, raised_room_list());

    assert!(
        !effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)),
        "an unchanged candidate must not notify the webview again: {effects:?}"
    );
}

#[test]
fn sound_dispatch_starts_for_a_surviving_candidate() {
    let mut state = state_with_published_candidate();
    room_list_update(&mut state, raised_room_list());
    assert!(state.settings.values.notifications.sound);

    // The sound path shares the banner's candidate gate.
    let effects = reduce(
        &mut state,
        AppAction::NativeAttentionDispatchStarted {
            dispatch_id: NativeAttentionDispatchId::new(1, 1),
        },
    );

    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Dispatching {
            dispatch_id: NativeAttentionDispatchId::new(1, 1),
        }
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)));
}

#[test]
fn a_settled_dispatch_is_not_rearmed_by_an_unrelated_room_list_update() {
    let mut state = state_with_published_candidate();
    let dispatch_id = NativeAttentionDispatchId::new(1, 1);
    reduce(
        &mut state,
        AppAction::NativeAttentionDispatchStarted { dispatch_id },
    );
    reduce(
        &mut state,
        AppAction::NativeAttentionDispatchSettled {
            dispatch_id,
            outcome: crate::state::NativeAttentionSoundOutcome::Played,
        },
    );

    room_list_update(&mut state, raised_room_list());

    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Delivered { dispatch_id },
        "a surviving candidate keeps the dispatch it was published with, so no second banner or sound"
    );
}

#[test]
fn a_newer_live_candidate_replaces_the_surviving_one() {
    let mut state = state_with_published_candidate();
    room_list_update(&mut state, raised_room_list());

    let mut rooms = raised_room_list();
    rooms[2] = newest_room();
    room_list_update(&mut state, rooms);

    assert_eq!(
        state
            .native_attention
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.room_display_name.as_str()),
        Some(NEWEST_ROOM),
        "a newer live candidate replaces the surviving one"
    );
    assert_eq!(published_room_id(&state), Some(NEWEST_ROOM));
}

#[test]
fn candidate_and_payload_clear_together_when_the_room_is_read() {
    let mut state = state_with_published_candidate();

    // The notified room's unread falls to zero: reading it is a canonical clear.
    room_list_update(
        &mut state,
        vec![active_room(), room_without_attention(NOTIFIED_ROOM)],
    );

    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(state.native_attention.notification, None);
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Idle
    );
}

#[test]
fn a_focus_change_clears_the_surviving_candidate_and_its_payload() {
    let mut state = state_with_published_candidate();
    room_list_update(&mut state, raised_room_list());

    reduce(
        &mut state,
        AppAction::NativeWindowFocusChanged {
            focused: false,
            observation_generation: 1,
        },
    );

    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(
        state.native_attention.notification, None,
        "a cleared candidate must not leave its payload behind"
    );
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Idle
    );
}

#[test]
fn live_candidate_survives_a_muted_room_that_cannot_notify() {
    let mut state = state_with_published_candidate();
    let candidate = state.native_attention.summary.candidate.clone();
    state.room_notification_settings.insert(
        MUTED_ROOM.to_owned(),
        crate::state::RoomNotificationSettings {
            mode: crate::state::RoomNotificationMode::Mute,
            operation: Default::default(),
        },
    );

    // The muted room's unread rises, so it is the live room of this observation, but it
    // can never be a candidate: the published one must survive it.
    let mut muted = test_room(MUTED_ROOM, None);
    muted.unread_count = 3;
    muted.notification_count = 3;
    muted.latest_event = Some(latest_event("$muted:example.invalid", 46));
    muted.recency_stamp = Some(46);
    let mut rooms = raised_room_list();
    rooms.push(muted);
    room_list_update(&mut state, rooms);

    assert_eq!(state.native_attention.summary.candidate, candidate);
    assert_eq!(published_room_id(&state), Some(NOTIFIED_ROOM));
}

#[test]
fn an_equally_named_room_does_not_inherit_a_read_rooms_payload() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    // The twin already holds its unread attention, so it never becomes the live room of a
    // later observation and its candidate value equals the published one.
    let twin = || {
        let mut room = test_room(TWIN_ROOM, None);
        room.display_name = "Twin".to_owned();
        room.display_label = "Twin".to_owned();
        room.original_display_label = "Twin".to_owned();
        room.unread_count = 1;
        room.notification_count = 1;
        room.recency_stamp = Some(42);
        room.latest_event = Some(latest_event("$twin:example.invalid", 42));
        room
    };
    let notified = || {
        let mut room = notified_room();
        room.display_name = "Twin".to_owned();
        room.display_label = "Twin".to_owned();
        room.original_display_label = "Twin".to_owned();
        room
    };
    room_list_update(
        &mut state,
        vec![active_room(), room_without_attention(NOTIFIED_ROOM), twin()],
    );
    room_list_update(&mut state, vec![active_room(), notified(), twin()]);
    assert_eq!(published_room_id(&state), Some(NOTIFIED_ROOM));

    // The read room leaves; its equally named, equally counted twin stays. The published
    // candidate belongs to the room that was read, so it clears instead of retargeting.
    room_list_update(
        &mut state,
        vec![active_room(), room_without_attention(NOTIFIED_ROOM), twin()],
    );

    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(state.native_attention.notification, None);
}

#[test]
fn a_suppressed_raise_consumes_the_published_candidate() {
    let mut state = state_with_published_candidate();

    // A message in the focused active room raises attention that is suppressed
    // (`Candidate --> Suppressed: AttentionSuppressed`), which ends the published
    // candidate: carrying it here would contradict that transition.
    let mut active = active_room();
    active.unread_count = 1;
    active.notification_count = 1;
    active.recency_stamp = Some(47);
    active.latest_event = Some(latest_event("$active:example.invalid", 47));
    let mut rooms = raised_room_list();
    rooms[0] = active;
    room_list_update(&mut state, rooms);

    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(state.native_attention.notification, None);
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Suppressed {
            reason: crate::state::NativeAttentionSuppressionReason::WindowFocused,
        }
    );
}
