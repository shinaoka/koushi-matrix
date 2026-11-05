//! #1802: a raised attention candidate carries its own alert identity.
//!
//! The webview dedupes on `alert_id` alone, so the identity has to be a Rust fact
//! with a stated lifetime: minted where a candidate is raised, kept while the same
//! alert is relabelled or survives an observation that raises nothing (#1799), and
//! cleared with the candidate.

use super::*;
use crate::state::{NativeAttentionDispatchState, RoomSummary};

const ACTIVE_ROOM: &str = "!active:example.invalid";
const NOTIFIED_ROOM: &str = "!other:example.invalid";
const NEWEST_ROOM: &str = "!newest:example.invalid";
const DM_ROOM: &str = "!dm:example.invalid";
const DM_PARTNER: &str = "@bob:example.invalid";

fn active_room() -> RoomSummary {
    test_room(ACTIVE_ROOM, None)
}

fn read_room(room_id: &str) -> RoomSummary {
    test_room(room_id, None)
}

fn unread_room(room_id: &str, event_id: &str, timestamp_ms: u64, unread: u64) -> RoomSummary {
    let mut room = test_room(room_id, None);
    room.unread_count = unread;
    room.notification_count = unread;
    room.recency_stamp = Some(timestamp_ms);
    room.latest_event = Some(latest_event(event_id, timestamp_ms));
    room
}

fn dm_room(unread: u64) -> RoomSummary {
    let mut room = test_room(DM_ROOM, None);
    room.is_dm = true;
    room.dm_user_ids = vec![DM_PARTNER.to_owned()];
    room.display_name = "Bob".to_owned();
    room.display_label = "Bob".to_owned();
    room.original_display_label = "Bob".to_owned();
    if unread > 0 {
        room.unread_count = unread;
        room.notification_count = unread;
        room.recency_stamp = Some(43);
        room.latest_event = Some(latest_event("$dm:example.invalid", 43));
    }
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

fn alert_id(state: &AppState) -> Option<u64> {
    state.native_attention.summary.alert_id
}

fn published_room_id(state: &AppState) -> Option<&str> {
    state
        .native_attention
        .notification
        .as_ref()
        .map(|payload| payload.target.room_id.as_str())
}

fn candidate_label(state: &AppState) -> Option<&str> {
    state
        .native_attention
        .summary
        .candidate
        .as_ref()
        .map(|candidate| candidate.room_display_name.as_str())
}

/// A live candidate published for a room that is not the active one.
fn state_with_published_candidate() -> AppState {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    room_list_update(
        &mut state,
        vec![
            active_room(),
            read_room(NOTIFIED_ROOM),
            read_room(NEWEST_ROOM),
        ],
    );
    room_list_update(
        &mut state,
        vec![
            active_room(),
            unread_room(NOTIFIED_ROOM, "$notified:example.invalid", 43, 1),
            read_room(NEWEST_ROOM),
        ],
    );

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
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Idle
    );
    state
}

#[test]
fn a_relabel_of_the_published_alert_keeps_its_alert_id() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    room_list_update(&mut state, vec![active_room(), dm_room(0)]);
    room_list_update(&mut state, vec![active_room(), dm_room(1)]);

    let raised = alert_id(&state).expect("a live raise mints an alert identity");
    assert_eq!(published_room_id(&state), Some(DM_ROOM));
    assert_eq!(candidate_label(&state), Some("Bob"));

    // A local alias for the DM partner relabels the published candidate; the alert
    // it publishes is the same one.
    let effects = reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 64,
            user_id: DM_PARTNER.to_owned(),
            alias: Some("Zelda".to_owned()),
        },
    );

    assert_eq!(
        candidate_label(&state),
        Some("Zelda"),
        "the published candidate is relabelled"
    );
    assert_eq!(
        state
            .native_attention
            .notification
            .as_ref()
            .map(|payload| payload.title.as_str()),
        Some("Direct message in Zelda"),
        "the published payload is relabelled with it"
    );
    assert_eq!(
        alert_id(&state),
        Some(raised),
        "a relabel must not mint a second identity for the same alert"
    );
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)),
        "the relabelled candidate still reaches the webview: {effects:?}"
    );
}

#[test]
fn a_new_raise_after_a_clear_mints_a_new_alert_id() {
    let mut state = state_with_published_candidate();
    let first = alert_id(&state).expect("precondition: the raised candidate has an identity");

    room_list_update(
        &mut state,
        vec![
            active_room(),
            read_room(NOTIFIED_ROOM),
            read_room(NEWEST_ROOM),
        ],
    );
    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(
        alert_id(&state),
        None,
        "the identity clears with the candidate"
    );

    room_list_update(
        &mut state,
        vec![
            active_room(),
            unread_room(NOTIFIED_ROOM, "$notified-again:example.invalid", 45, 1),
            read_room(NEWEST_ROOM),
        ],
    );

    let second = alert_id(&state).expect("the re-raise mints an identity");
    assert!(
        second > first,
        "a re-raise after a clear must be a newer identity, not a reused one: {second} <= {first}"
    );
}

#[test]
fn a_raise_in_another_room_mints_a_different_alert_id() {
    let mut state = state_with_published_candidate();
    let first = alert_id(&state).expect("precondition: the raised candidate has an identity");

    room_list_update(
        &mut state,
        vec![
            active_room(),
            unread_room(NOTIFIED_ROOM, "$notified:example.invalid", 43, 1),
            unread_room(NEWEST_ROOM, "$newest:example.invalid", 50, 2),
        ],
    );

    assert_eq!(
        published_room_id(&state),
        Some(NEWEST_ROOM),
        "the newer live candidate replaces the published one"
    );
    let second = alert_id(&state).expect("the replacement mints an identity");
    assert_ne!(
        second, first,
        "a different room's raise is a different alert"
    );
}

#[test]
fn an_observation_that_raises_nothing_keeps_the_published_alert_id() {
    let mut state = state_with_published_candidate();
    let first = alert_id(&state).expect("precondition: the raised candidate has an identity");

    // A message the user sent themselves in the active room: no unread count rises,
    // so this observation raises nothing and the published candidate survives it.
    let mut active = active_room();
    let mut own_event = latest_event("$own:example.invalid", 44);
    own_event.sender_id = Some("@alice:example.invalid".to_owned());
    active.latest_event = Some(own_event);
    active.recency_stamp = Some(44);
    room_list_update(
        &mut state,
        vec![
            active,
            unread_room(NOTIFIED_ROOM, "$notified:example.invalid", 43, 1),
            read_room(NEWEST_ROOM),
        ],
    );

    assert_eq!(
        alert_id(&state),
        Some(first),
        "a surviving candidate keeps the identity it was published with"
    );
}

#[test]
fn a_cleared_candidate_clears_the_alert_identity_with_its_payload() {
    let mut state = state_with_published_candidate();
    assert!(alert_id(&state).is_some());
    assert!(state.native_attention.notification.is_some());

    room_list_update(&mut state, vec![active_room(), read_room(NOTIFIED_ROOM)]);

    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(
        state.native_attention.notification, None,
        "a cleared candidate must not leave its payload behind"
    );
    assert_eq!(
        alert_id(&state),
        None,
        "an identity without a candidate would be a banner the webview can never show"
    );
}

#[test]
fn a_restored_state_cannot_reissue_a_published_alert_id() {
    let state = state_with_published_candidate();
    let published = alert_id(&state).expect("precondition: the raise mints an identity");

    // The attention summary is serialized into the app state, while the sequence that
    // mints identities is process-local: a restore keeps the published identity and
    // drops the counter back to its default.
    let serialized = serde_json::to_string(&state).expect("the app state serializes");
    let mut restored: AppState = serde_json::from_str(&serialized).expect("the app state restores");
    assert_eq!(
        alert_id(&restored),
        Some(published),
        "the restored summary keeps the identity it published"
    );
    assert_eq!(
        restored.native_attention_context.alert_sequence, 0,
        "the minting sequence is process-local and restarts at its default"
    );

    // A new live raise for the same room after the restore must not hand out a value
    // that is already published: the webview would read it as the same alert and drop
    // a banner it has to show.
    room_list_update(
        &mut restored,
        vec![
            active_room(),
            unread_room(NOTIFIED_ROOM, "$notified-later:example.invalid", 46, 2),
            read_room(NEWEST_ROOM),
        ],
    );

    let reissued = alert_id(&restored).expect("the re-raise mints an identity");
    assert!(
        reissued > published,
        "a restored state must mint above every published identity: {reissued} <= {published}"
    );
}
