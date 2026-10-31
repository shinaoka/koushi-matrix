//! Reducer contract for sender device advisories and the advisory draft seed
//! (#1494).
//!
//! State-machine contract: docs/architecture/state-machine.md, "Sender Device
//! Advisories" and the `ComposerDraftSeeded` main-composer draft rule. The
//! device classification itself is the SDK adapter's (`koushi-sdk`
//! contact_security tests drive it with real signed keys).

use std::collections::BTreeSet;

use koushi_state::{
    AppAction, AppEffect, AppState, ComposerDocument, ComposerDraftRevision, RoomSummary, RoomTags,
    SessionInfo, SessionState, UiEvent, reduce,
};

const ROOM: &str = "!room-a:example.test";
const DM: &str = "!dm-bob:example.test";
const BOB: &str = "@bob:example.test";
const CAROL: &str = "@carol:example.test";

fn session_info(user_id: &str) -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: user_id.to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn room(room_id: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: true,
        joined_members: 2,
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info("@me:example.test")),
        rooms: vec![room(ROOM), room(DM)],
        ..AppState::default()
    }
}

fn users(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn update(state: &mut AppState, room_id: &str, ids: &[&str]) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::SenderDeviceAdvisoriesUpdated {
            room_id: room_id.to_owned(),
            user_ids: users(ids),
        },
    )
}

fn changed() -> AppEffect {
    AppEffect::EmitUiEvent(UiEvent::SenderDeviceAdvisoriesChanged)
}

#[test]
fn ready_update_for_a_known_room_records_its_senders() {
    let mut state = ready_state();
    let effects = update(&mut state, ROOM, &[BOB, CAROL]);
    assert_eq!(effects, vec![changed()]);
    assert_eq!(
        state.sender_device_advisories.rooms.get(ROOM),
        Some(&users(&[BOB, CAROL]))
    );
}

#[test]
fn unchanged_set_emits_nothing() {
    let mut state = ready_state();
    update(&mut state, ROOM, &[BOB]);
    assert!(update(&mut state, ROOM, &[BOB]).is_empty());
}

#[test]
fn empty_set_removes_the_room_entry() {
    let mut state = ready_state();
    update(&mut state, ROOM, &[BOB]);
    assert_eq!(update(&mut state, ROOM, &[]), vec![changed()]);
    assert!(state.sender_device_advisories.rooms.is_empty());
    // Absent --> Absent is a no-op.
    assert!(update(&mut state, ROOM, &[]).is_empty());
}

#[test]
fn updates_outside_ready_or_for_unknown_rooms_are_dropped() {
    let mut state = AppState {
        rooms: vec![room(ROOM)],
        ..AppState::default()
    };
    assert!(update(&mut state, ROOM, &[BOB]).is_empty());
    assert!(state.sender_device_advisories.rooms.is_empty());

    let mut state = ready_state();
    assert!(update(&mut state, "!unknown:example.test", &[BOB]).is_empty());
    assert!(state.sender_device_advisories.rooms.is_empty());
}

#[test]
fn cleared_room_drops_only_that_entry() {
    let mut state = ready_state();
    update(&mut state, ROOM, &[BOB]);
    update(&mut state, DM, &[BOB]);
    let effects = reduce(
        &mut state,
        AppAction::SenderDeviceAdvisoriesCleared {
            room_id: ROOM.to_owned(),
        },
    );
    assert_eq!(effects, vec![changed()]);
    assert_eq!(
        state
            .sender_device_advisories
            .rooms
            .keys()
            .collect::<Vec<_>>(),
        vec![DM]
    );
    assert!(
        reduce(
            &mut state,
            AppAction::SenderDeviceAdvisoriesCleared {
                room_id: ROOM.to_owned(),
            },
        )
        .is_empty()
    );
}

#[test]
fn account_switch_clears_every_room() {
    let mut state = ready_state();
    update(&mut state, ROOM, &[BOB]);
    let effects = reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: session_info("@other:example.test"),
        },
    );
    assert!(effects.contains(&changed()));
    assert_eq!(state.sender_device_advisories, Default::default());
}

fn advisory() -> ComposerDocument {
    ComposerDocument::from_plain_text("Hi Bob, one of your devices is not verified.")
}

fn seed(state: &mut AppState, room_id: &str) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::ComposerDraftSeeded {
            room_id: room_id.to_owned(),
            document: advisory(),
        },
    )
}

#[test]
fn seed_writes_an_empty_draft_with_a_core_chosen_successor_revision() {
    let mut state = ready_state();
    let before = state.composer_drafts.room_revision(DM);
    seed(&mut state, DM);
    let composer = state.composer_drafts.composer_for_room(DM);
    assert_eq!(composer.document, advisory());
    assert!(composer.draft_revision > before);
}

#[test]
fn seed_never_replaces_an_existing_draft() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: DM.to_owned(),
            document: ComposerDocument::from_plain_text("my own words"),
            revision: ComposerDraftRevision::from_u64(5),
        },
    );
    assert!(seed(&mut state, DM).is_empty());
    let composer = state.composer_drafts.composer_for_room(DM);
    assert_eq!(composer.draft, "my own words");
    assert_eq!(composer.draft_revision, ComposerDraftRevision::from_u64(5));
}

#[test]
fn seed_after_an_accepted_clear_moves_past_its_tombstone() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: DM.to_owned(),
            document: ComposerDocument::from_plain_text(""),
            revision: ComposerDraftRevision::from_u64(9),
        },
    );
    seed(&mut state, DM);
    let composer = state.composer_drafts.composer_for_room(DM);
    assert_eq!(composer.document, advisory());
    assert!(composer.draft_revision > ComposerDraftRevision::from_u64(9));
}

#[test]
fn seed_outside_ready_or_for_unknown_rooms_is_dropped() {
    let mut state = AppState {
        rooms: vec![room(DM)],
        ..AppState::default()
    };
    assert!(seed(&mut state, DM).is_empty());
    assert!(state.composer_drafts.is_empty());

    let mut state = ready_state();
    assert!(seed(&mut state, "!unknown:example.test").is_empty());
    assert!(state.composer_drafts.is_empty());
}

#[test]
fn seed_hydrates_the_open_empty_composer() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: DM.to_owned(),
        },
    );
    let effects = seed(&mut state, DM);
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: DM.to_owned()
        }))
    );
    assert_eq!(state.timeline.composer.document, advisory());
    assert_eq!(
        state.timeline.composer.draft_revision,
        state.composer_drafts.room_revision(DM)
    );
}

#[test]
fn seed_does_not_hydrate_a_composer_with_a_pending_submission() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: DM.to_owned(),
        },
    );
    state.timeline.composer.pending_transaction_id = Some("txn-1".to_owned());
    seed(&mut state, DM);
    assert_eq!(
        state.timeline.composer.document,
        ComposerDocument::default()
    );
    assert_eq!(
        state.composer_drafts.composer_for_room(DM).document,
        advisory()
    );
}
