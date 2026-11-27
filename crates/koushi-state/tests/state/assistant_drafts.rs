//! Reducer contract for MCP assistant drafts (#1644).
//!
//! State-machine contract: docs/architecture/state-machine.md,
//! `AssistantDraftStaged` and `AssistantDraftMarkerDismissed`.

use koushi_state::{
    AppAction, AppEffect, AppState, AssistantDraftMarker, ComposerDocument, ComposerDraftRevision,
    ComposerMode, ComposerState, ComposerTarget, RoomSummary, RoomTags, SessionInfo, SessionState,
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
    stage_reply(state, target, None)
}

fn stage_reply(
    state: &mut AppState,
    target: ComposerTarget,
    reply_to_event_id: Option<&str>,
) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::AssistantDraftStaged {
            target,
            document: staged_text(),
            client_name: CLIENT.to_owned(),
            subject: format!("client:{CLIENT}"),
            replace: false,
            reply_to_event_id: reply_to_event_id.map(str::to_owned),
        },
    )
}

fn replace_stage(state: &mut AppState, target: ComposerTarget, text: &str) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::AssistantDraftStaged {
            target,
            document: ComposerDocument::from_plain_text(text),
            client_name: CLIENT.to_owned(),
            subject: format!("client:{CLIENT}"),
            replace: true,
            reply_to_event_id: None,
        },
    )
}

fn reply(reply_to_event_id: &str) -> ComposerMode {
    ComposerMode::Reply {
        in_reply_to_event_id: reply_to_event_id.to_owned(),
    }
}

fn select_room(state: &mut AppState, room_id: &str) {
    reduce(
        state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );
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
            subject: format!("client:{CLIENT}"),
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

/// #1905 + #1906: a replacement carries its own reply relation and none
/// otherwise, so replacing a reply draft with a plain one drops the reply.
#[test]
fn a_replacement_replaces_the_reply_relation_too() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    stage_reply(&mut state, main_target(ROOM), Some("$question"));
    assert_eq!(
        state.timeline.composer.mode,
        reply("$question"),
        "the staged reply shows in the open composer"
    );

    replace_stage(&mut state, main_target(ROOM), "the short version");

    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

/// #1905: a replacement keeps the open composer live: the user sees the new text
/// without navigation, a remount, or a lost composer mode.
#[test]
fn replacing_hydrates_the_open_composer_at_a_new_revision() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    stage(&mut state, main_target(ROOM));
    let staged = state.composer_drafts.room_revision(ROOM);
    let mode = state.timeline.composer.mode.clone();

    let effects = replace_stage(&mut state, main_target(ROOM), "the short version");

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: ROOM.to_owned()
        })]
    );
    assert_eq!(state.timeline.composer.draft, "the short version");
    assert_eq!(state.timeline.composer.mode, mode);
    assert!(state.composer_drafts.room_revision(ROOM) > staged);
    assert_eq!(
        state
            .timeline
            .composer
            .assistant_draft
            .as_ref()
            .map(|marker| marker.client_name.as_str()),
        Some(CLIENT)
    );
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM));
}

/// The replacement is the assistant's own draft only while the marker names the
/// stored revision; a hand edit or a dismissed label ends that.
#[test]
fn replace_never_touches_text_the_user_changed() {
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    stage(&mut state, main_target(ROOM));
    let staged = state.composer_drafts.room_revision(ROOM);
    type_draft(&mut state, ROOM, "edited by hand", next(staged, 1));

    assert!(replace_stage(&mut state, main_target(ROOM), "the short version").is_empty());
    let composer = state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(composer.draft, "edited by hand");
    assert_eq!(
        composer.draft_revision,
        ComposerDraftRevision::from_u64(next(staged, 1))
    );
    assert_eq!(state.timeline.composer.draft, "edited by hand");

    // Hiding the label drops the provenance, so the text is the user's.
    reduce(
        &mut state,
        AppAction::AssistantDraftMarkerDismissed {
            target: main_target(ROOM),
        },
    );
    assert!(replace_stage(&mut state, main_target(ROOM), "again").is_empty());
    assert_eq!(
        state.composer_drafts.composer_for_room(ROOM).draft,
        "edited by hand"
    );
}

/// A replacement in a thread composer hydrates that pane only.
#[test]
fn replacing_hydrates_only_the_open_thread_composer() {
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
    stage(&mut state, thread_target());

    let effects = replace_stage(&mut state, thread_target(), "thread, revised");

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
    let ThreadPaneState::Open { composer, .. } = &state.thread else {
        panic!("thread pane stays open");
    };
    assert_eq!(composer.draft, "thread, revised");
    assert!(composer.assistant_draft.is_some());
    assert!(state.timeline.composer.document.is_empty());
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

// --- #1906: a staged draft can carry a reply relation ---

/// Acceptance #1 at the reducer tier: an assistant replies to a message in a
/// room nobody has open. The draft and its reply target are staged together,
/// opening the room puts that composer in reply mode for the event, and the
/// send is submitted as a reply to it.
#[test]
fn a_reply_draft_in_a_closed_room_opens_in_reply_mode_and_is_sent_as_a_reply() {
    let mut state = ready_state();
    select_room(&mut state, OTHER);
    let before = state.clone();

    assert!(stage_reply(&mut state, main_target(ROOM), Some("$question")).is_empty());

    // Nothing but the target's draft entry changed: no selection, focus, pane,
    // or read state (#1524).
    let mut expected = before;
    expected.composer_drafts = state.composer_drafts.clone();
    assert_eq!(state, expected);
    let staged = state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(staged.document, staged_text());
    assert_eq!(staged.mode, reply("$question"));
    assert_eq!(
        state.composer_drafts.room_reply_target(ROOM),
        Some("$question")
    );

    // Opening the room shows the draft as a reply with the usual preview.
    select_room(&mut state, ROOM);
    assert_eq!(state.timeline.composer.draft, staged_text().plain_body());
    assert_eq!(state.timeline.composer.mode, reply("$question"));

    // A send is submitted as a reply to that event; accepting it keeps the
    // reply mode until the send terminal.
    let draft_revision = state.composer_drafts.room_revision(ROOM);
    reduce(
        &mut state,
        AppAction::SendTextSubmittedAtRevision {
            room_id: ROOM.to_owned(),
            transaction_id: "txn-reply".to_owned(),
            body: staged_text().plain_body(),
            draft_revision,
        },
    );
    assert_eq!(
        state.timeline.composer.pending_send_kind,
        Some(koushi_state::PendingComposerSendKind::Reply {
            in_reply_to_event_id: "$question".to_owned(),
        })
    );
    assert_eq!(state.timeline.composer.mode, reply("$question"));

    reduce(
        &mut state,
        AppAction::SendTextFinished {
            room_id: ROOM.to_owned(),
            transaction_id: "txn-reply".to_owned(),
        },
    );
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);
}

/// Acceptance #2: a thread draft carries both relations.
#[test]
fn a_thread_reply_draft_stages_both_relations_and_hydrates_the_thread() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    // The thread pane is closed, so the staged draft waits for it.
    assert!(stage_reply(&mut state, thread_target(), Some("$in-thread")).is_empty());

    let staged = state.composer_drafts.composer_for_thread(ROOM, ROOT);
    assert_eq!(staged.document, staged_text());
    assert_eq!(staged.mode, reply("$in-thread"));
    // The room's own composer is a different target.
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);

    state.thread = ThreadPaneState::Open {
        room_id: ROOM.to_owned(),
        root_event_id: ROOT.to_owned(),
        intent: ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer: state.composer_drafts.composer_for_thread(ROOM, ROOT),
        staged_uploads: Vec::new(),
    };
    let ThreadPaneState::Open { composer, .. } = &state.thread else {
        panic!("thread pane stays open");
    };
    assert_eq!(composer.document, staged_text());
    assert_eq!(composer.mode, reply("$in-thread"));
    assert!(composer.assistant_draft.is_some());
}

/// Acceptance #5: cancelling the reply keeps the text and drops only the
/// relation, so reopening that composer starts plain.
#[test]
fn cancelling_a_staged_reply_keeps_the_text_and_drops_the_relation() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    stage_reply(&mut state, main_target(ROOM), Some("$question"));
    assert_eq!(state.timeline.composer.mode, reply("$question"));

    let effects = reduce(&mut state, AppAction::ComposerReplyCancelled);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
            room_id: ROOM.to_owned()
        })]
    );
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
    assert_eq!(state.timeline.composer.document, staged_text());
    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);

    // Leaving and reopening the composer keeps the text and stays plain.
    select_room(&mut state, OTHER);
    select_room(&mut state, ROOM);
    assert_eq!(state.timeline.composer.document, staged_text());
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

/// The thread counterpart: cancelling in the thread composer keeps its text.
#[test]
fn cancelling_a_staged_thread_reply_keeps_the_thread_text() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    state.thread = ThreadPaneState::Open {
        room_id: ROOM.to_owned(),
        root_event_id: ROOT.to_owned(),
        intent: ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer: ComposerState::default(),
        staged_uploads: Vec::new(),
    };
    stage_reply(&mut state, thread_target(), Some("$in-thread"));

    let effects = reduce(
        &mut state,
        AppAction::ThreadComposerReplyCancelled {
            room_id: ROOM.to_owned(),
            root_event_id: ROOT.to_owned(),
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
    );
    let ThreadPaneState::Open { composer, .. } = &state.thread else {
        panic!("thread pane stays open");
    };
    assert_eq!(composer.mode, ComposerMode::Plain);
    assert_eq!(composer.document, staged_text());
    assert_eq!(state.composer_drafts.thread_reply_target(ROOM, ROOT), None);
}

/// The thread composer stores a reply selected before typing the same way.
#[test]
fn a_thread_reply_selected_on_an_empty_draft_is_stored_once_the_text_arrives() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    state.thread = ThreadPaneState::Open {
        room_id: ROOM.to_owned(),
        root_event_id: ROOT.to_owned(),
        intent: ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer: ComposerState::default(),
        staged_uploads: Vec::new(),
    };
    reduce(
        &mut state,
        AppAction::ThreadComposerReplyTargetSelected {
            room_id: ROOM.to_owned(),
            root_event_id: ROOT.to_owned(),
            event_id: "$in-thread".to_owned(),
        },
    );
    assert_eq!(state.composer_drafts.thread_reply_target(ROOM, ROOT), None);

    reduce(
        &mut state,
        AppAction::ThreadComposerDraftChangedAtRevision {
            room_id: ROOM.to_owned(),
            root_event_id: ROOT.to_owned(),
            document: ComposerDocument::from_plain_text("my question"),
            revision: ComposerDraftRevision::from_u64(1),
        },
    );
    assert_eq!(
        state.composer_drafts.thread_reply_target(ROOM, ROOT),
        Some("$in-thread")
    );
}

/// The reply target is stored with the draft, so it survives a restart, and a
/// user-selected target is stored the same way while the draft has content.
#[test]
fn the_reply_relation_survives_persistence_and_dies_with_the_content() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    stage_reply(&mut state, main_target(ROOM), Some("$question"));
    stage_reply(&mut state, thread_target(), Some("$in-thread"));

    let projection = state
        .composer_drafts
        .persisted_projection(&Default::default());
    let restored = koushi_state::ComposerDraftStore::from_persisted_projection(projection.clone())
        .expect("valid projection");
    assert_eq!(restored.room_reply_target(ROOM), Some("$question"));
    assert_eq!(restored.thread_reply_target(ROOM, ROOT), Some("$in-thread"));
    assert_eq!(restored.composer_for_room(ROOM).mode, reply("$question"));

    // A relation without content is not a valid stored entry.
    let mut orphan = projection.clone();
    let entry = orphan.rooms.get_mut(ROOM).expect("room entry");
    entry.content = None;
    orphan.quiescent_room_order.push(ROOM.to_owned());
    assert!(
        koushi_state::ComposerDraftStore::from_persisted_projection(orphan).is_err(),
        "a reply target cannot outlive its draft"
    );

    // Clearing the draft drops both.
    let revision =
        ComposerDraftRevision::from_u64(next(state.composer_drafts.room_revision(ROOM), 1));
    reduce(
        &mut state,
        AppAction::ComposerDraftChangedAtRevision {
            room_id: ROOM.to_owned(),
            document: ComposerDocument::default(),
            revision,
        },
    );
    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

/// A user-picked reply target is stored with the draft too, so one owner
/// decides the relation for both the GUI and the assistant.
#[test]
fn a_user_selected_reply_target_is_stored_with_the_draft() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    type_draft(&mut state, ROOM, "my question", 1);
    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: ROOM.to_owned(),
            event_id: "$quoted".to_owned(),
        },
    );

    assert_eq!(
        state.composer_drafts.room_reply_target(ROOM),
        Some("$quoted")
    );
    reduce(&mut state, AppAction::ComposerReplyCancelled);
    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);
    assert_eq!(state.timeline.composer.draft, "my question");
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

/// Selecting a reply before typing stores the relation with the text: the
/// store is what a reopen and `get_draft` read.
#[test]
fn a_reply_selected_on_an_empty_draft_is_stored_once_the_text_arrives() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: ROOM.to_owned(),
            event_id: "$quoted".to_owned(),
        },
    );
    // An empty draft carries no relation, so it is not stored yet.
    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);

    type_draft(&mut state, ROOM, "my question", 1);
    assert_eq!(
        state.composer_drafts.room_reply_target(ROOM),
        Some("$quoted")
    );
    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$quoted".to_owned()
        }
    );
}

/// Staging a plain draft never puts a composer in reply mode.
#[test]
fn a_staged_draft_without_a_reply_target_leaves_the_composer_plain() {
    let mut state = ready_state();
    select_room(&mut state, ROOM);
    stage(&mut state, main_target(ROOM));
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
    assert_eq!(state.composer_drafts.room_reply_target(ROOM), None);
}

#[test]
fn a_draft_with_no_owning_subject_is_not_staged() {
    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::AssistantDraftStaged {
            target: main_target(ROOM),
            document: staged_text(),
            client_name: CLIENT.to_owned(),
            subject: String::new(),
            replace: false,
            reply_to_event_id: None,
        },
    );
    assert!(state.composer_drafts.room_assistant_marker(ROOM).is_none());
    assert!(state.composer_drafts.target_is_empty(&main_target(ROOM)));
    let _ = effects;
}
