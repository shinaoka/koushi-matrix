//! Reducer contract for MCP assistant drafts (#1644).
//!
//! State-machine contract: docs/architecture/state-machine.md,
//! `AssistantDraftStaged` and `AssistantDraftMarkerDismissed`.

use koushi_state::{
    AppAction, AppEffect, AppState, AssistantDraftMarker, ComposerDocument, ComposerDraftRevision,
    ComposerState, ComposerTarget, RoomSummary, RoomTags, SessionInfo, SessionState,
    ThreadOpenIntent, ThreadPaneState, UiEvent, reduce,
};

const ROOM: &str = "!room-a:example.test";
const OTHER: &str = "!room-b:example.test";
const ROOT: &str = "$root";
const CLIENT: &str = "claude-code";

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
        unread_count: 3,
        notification_count: 1,
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
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.test".to_owned(),
            user_id: "@me:example.test".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![room(ROOM), room(OTHER)],
        ..AppState::default()
    }
}

fn main_target(room_id: &str) -> ComposerTarget {
    ComposerTarget::Main {
        room_id: room_id.to_owned(),
    }
}

fn thread_target() -> ComposerTarget {
    ComposerTarget::Thread {
        room_id: ROOM.to_owned(),
        root_event_id: ROOT.to_owned(),
    }
}

fn staged_text() -> ComposerDocument {
    ComposerDocument::from_plain_text("Here is the summary you asked for.")
}

fn stage(state: &mut AppState, target: ComposerTarget) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::AssistantDraftStaged {
            target,
            document: staged_text(),
            client_name: CLIENT.to_owned(),
        },
    )
}

fn next(revision: ComposerDraftRevision, steps: u64) -> u64 {
    revision
        .to_wire_string()
        .parse::<u64>()
        .expect("small revision")
        + steps
}

fn type_draft(state: &mut AppState, room_id: &str, text: &str, revision: u64) {
    reduce(
        state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: room_id.to_owned(),
            document: ComposerDocument::from_plain_text(text),
            revision: ComposerDraftRevision::from_u64(revision),
        },
    );
}

#[test]
fn staging_a_closed_room_writes_only_that_draft_and_leaves_navigation_alone() {
    let mut state = ready_state();
    state.timeline.room_id = Some(OTHER.to_owned());
    type_draft(&mut state, OTHER, "user text elsewhere", 4);
    let before = state.clone();

    let effects = stage(&mut state, main_target(ROOM));

    // A closed target emits nothing: the draft appears when the room is opened.
    assert!(effects.is_empty());
    let composer = state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(composer.document, staged_text());
    assert!(composer.draft_revision > ComposerDraftRevision::ZERO);
    assert_eq!(
        composer.assistant_draft,
        Some(AssistantDraftMarker {
            client_name: CLIENT.to_owned(),
            revision: composer.draft_revision,
        })
    );

    // Everything but the target's draft entry is unchanged: selection, the
    // other room's draft, the open composer, unread counts, and the thread pane.
    let mut expected = before;
    expected.composer_drafts = state.composer_drafts.clone();
    assert_eq!(state, expected);
    assert_eq!(
        state.composer_drafts.composer_for_room(OTHER).draft,
        "user text elsewhere"
    );
}

#[test]
fn staging_an_open_empty_composer_hydrates_it_without_dropping_its_mode() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    state.timeline.composer.accepted_submission_ids = Default::default();
    let mode = state.timeline.composer.mode.clone();

    let effects = stage(&mut state, main_target(ROOM));

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: ROOM.to_owned()
        })]
    );
    assert_eq!(state.timeline.composer.document, staged_text());
    assert_eq!(state.timeline.composer.mode, mode);
    assert_eq!(
        state
            .timeline
            .composer
            .assistant_draft
            .as_ref()
            .map(|m| m.client_name.as_str()),
        Some(CLIENT)
    );
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM));
}

#[test]
fn an_existing_draft_is_never_overwritten() {
    let mut state = ready_state();
    type_draft(&mut state, ROOM, "my own words", 5);

    assert!(stage(&mut state, main_target(ROOM)).is_empty());

    let composer = state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(composer.draft, "my own words");
    assert_eq!(composer.draft_revision, ComposerDraftRevision::from_u64(5));
    assert_eq!(composer.assistant_draft, None);
}

#[test]
fn staging_moves_past_an_accepted_clear_tombstone() {
    let mut state = ready_state();
    type_draft(&mut state, ROOM, "", 9);
    stage(&mut state, main_target(ROOM));
    let composer = state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(composer.document, staged_text());
    assert!(composer.draft_revision > ComposerDraftRevision::from_u64(9));
}

#[test]
fn staging_outside_ready_or_for_unknown_rooms_is_dropped() {
    let mut state = ready_state();
    assert!(stage(&mut state, main_target("!unknown:example.test")).is_empty());
    assert!(state.composer_drafts.is_empty());

    let mut signed_out = AppState {
        rooms: vec![room(ROOM)],
        ..AppState::default()
    };
    assert!(stage(&mut signed_out, main_target(ROOM)).is_empty());
    assert!(signed_out.composer_drafts.is_empty());
}

#[test]
fn editing_keeps_the_marker_and_clearing_removes_it() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    stage(&mut state, main_target(ROOM));
    let staged = state.composer_drafts.room_revision(ROOM);

    type_draft(&mut state, ROOM, "edited by the user", next(staged, 1));
    assert!(state.timeline.composer.assistant_draft.is_some());
    assert!(state.composer_drafts.room_assistant_marker(ROOM).is_some());

    type_draft(&mut state, ROOM, "", next(staged, 2));
    assert_eq!(state.timeline.composer.assistant_draft, None);
    assert_eq!(state.composer_drafts.room_assistant_marker(ROOM), None);

    // A later user draft never inherits the old marker.
    type_draft(&mut state, ROOM, "fresh text", next(staged, 3));
    assert_eq!(state.composer_drafts.room_assistant_marker(ROOM), None);
}

#[test]
fn an_accepted_send_removes_the_marker_with_the_content() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    stage(&mut state, main_target(ROOM));
    let staged = state.composer_drafts.room_revision(ROOM);

    reduce(
        &mut state,
        AppAction::ComposerDraftAccepted {
            target: main_target(ROOM),
            submitted_revision: staged,
            consumes_draft: true,
        },
    );

    assert_eq!(state.composer_drafts.room_assistant_marker(ROOM), None);
    assert_eq!(state.timeline.composer.assistant_draft, None);
}

#[test]
fn dismissing_removes_only_the_marker() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    stage(&mut state, main_target(ROOM));
    let revision = state.composer_drafts.room_revision(ROOM);

    let effects = reduce(
        &mut state,
        AppAction::AssistantDraftMarkerDismissed {
            target: main_target(ROOM),
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: ROOM.to_owned()
        })]
    );
    assert_eq!(state.timeline.composer.assistant_draft, None);
    assert_eq!(state.timeline.composer.document, staged_text());
    assert_eq!(state.composer_drafts.room_revision(ROOM), revision);
    // Idempotent.
    assert!(
        reduce(
            &mut state,
            AppAction::AssistantDraftMarkerDismissed {
                target: main_target(ROOM),
            },
        )
        .is_empty()
    );
}

#[test]
fn a_user_typed_draft_never_has_a_marker() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    type_draft(&mut state, ROOM, "typed by hand", 1);
    assert_eq!(state.timeline.composer.assistant_draft, None);
    assert_eq!(
        state
            .composer_drafts
            .composer_for_room(ROOM)
            .assistant_draft,
        None
    );
}

#[test]
fn staging_an_open_empty_thread_composer_hydrates_only_the_thread() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    state.thread = ThreadPaneState::Open {
        room_id: ROOM.to_owned(),
        root_event_id: ROOT.to_owned(),
        intent: ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer: ComposerState::default(),
        staged_uploads: Vec::new(),
    };

    let effects = stage(&mut state, thread_target());

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
    let ThreadPaneState::Open { composer, .. } = &state.thread else {
        panic!("thread pane stays open");
    };
    assert_eq!(composer.document, staged_text());
    assert!(composer.assistant_draft.is_some());
    // The main composer of the same room is a different target.
    assert!(state.timeline.composer.document.is_empty());
    assert_eq!(state.timeline.composer.assistant_draft, None);
}

#[test]
fn the_marker_survives_persistence_and_is_dropped_without_content() {
    let mut state = ready_state();
    stage(&mut state, main_target(ROOM));
    stage(&mut state, thread_target());

    let projection = state
        .composer_drafts
        .persisted_projection(&Default::default());
    let restored = koushi_state::ComposerDraftStore::from_persisted_projection(projection.clone())
        .expect("valid projection");
    assert_eq!(
        restored.room_assistant_marker(ROOM),
        state.composer_drafts.room_assistant_marker(ROOM)
    );
    assert!(restored.thread_assistant_marker(ROOM, ROOT).is_some());

    // An entry without content cannot carry a marker into the store.
    let mut emptied = projection;
    let entry = emptied.rooms.get_mut(ROOM).expect("room entry");
    entry.content = None;
    emptied.quiescent_room_order.push(ROOM.to_owned());
    let restored = koushi_state::ComposerDraftStore::from_persisted_projection(emptied)
        .expect("valid projection");
    assert_eq!(restored.room_assistant_marker(ROOM), None);
    assert!(restored.room_assistant_markers.is_empty());
}
