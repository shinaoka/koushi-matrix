//! #1644: the AppActor guards of MCP `draft_message`.

use super::*;
use crate::direct_operations::{AssistantDraftFailure, AssistantDraftRequest};

const ROOM: &str = "!target:example.invalid";
const OTHER: &str = "!other:example.invalid";

fn session() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@member:example.invalid".into(),
        device_id: "SYNTHETIC".into(),
        authentication_method: Default::default(),
    }
}

fn ready_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(session()),
        rooms: vec![unread_diagnostic_room(ROOM), unread_diagnostic_room(OTHER)],
        ..AppState::default()
    };
    state.navigation.active_room_id = Some(OTHER.to_owned());
    state.timeline.room_id = Some(OTHER.to_owned());
    state
}

fn main_target(room_id: &str) -> koushi_state::ComposerTarget {
    koushi_state::ComposerTarget::Main {
        room_id: room_id.to_owned(),
    }
}

fn request(target: koushi_state::ComposerTarget) -> AssistantDraftRequest {
    AssistantDraftRequest {
        target,
        document: koushi_state::ComposerDocument::from_plain_text("assistant text"),
        client_name: "claude-code".to_owned(),
    }
}

fn rid() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1644),
        sequence: 1,
    }
}

#[tokio::test]
async fn a_draft_reaches_only_its_target_and_keeps_navigation() {
    let data = tempfile::tempdir().unwrap();
    let (mut actor, ..) = app_actor_event_navigation_fixture(data.path(), ready_state());
    let before = actor.state.clone();

    let staged = actor
        .handle_assistant_draft(rid(), request(main_target(ROOM)))
        .await
        .expect("an empty target accepts the draft");

    assert_eq!(
        staged.revision,
        actor.state.composer_drafts.room_revision(ROOM)
    );
    let composer = actor.state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(composer.draft, "assistant text");
    assert!(composer.assistant_draft.is_some());
    let mut expected = before;
    expected.composer_drafts = actor.state.composer_drafts.clone();
    assert_eq!(actor.state, expected, "only the draft store changed");
}

#[tokio::test]
async fn an_existing_draft_is_a_conflict_and_stays() {
    let data = tempfile::tempdir().unwrap();
    let mut state = ready_state();
    state
        .composer_drafts
        .apply_room_draft(ROOM.to_owned(), "mine", 3.into())
        .unwrap();
    let (mut actor, ..) = app_actor_event_navigation_fixture(data.path(), state);

    let result = actor
        .handle_assistant_draft(rid(), request(main_target(ROOM)))
        .await;

    assert_eq!(result, Err(AssistantDraftFailure::Conflict));
    let composer = actor.state.composer_drafts.composer_for_room(ROOM);
    assert_eq!(composer.draft, "mine");
    assert_eq!(composer.assistant_draft, None);
}

#[tokio::test]
async fn a_renderer_command_in_flight_for_the_target_is_a_conflict() {
    let data = tempfile::tempdir().unwrap();
    let (mut actor, ..) = app_actor_event_navigation_fixture(data.path(), ready_state());
    let scope = crate::composer_draft_lifecycle::ComposerDraftScope {
        account: session_key_id_from_info(&session()),
        target: main_target(ROOM),
    };
    let registry = Arc::clone(&actor.composer_draft_leases);
    let generation = registry.begin_renderer_generation().unwrap();
    let lease = registry.acquire(generation, scope.clone()).unwrap();

    // An open composer alone (an activation lease) does not block the draft
    // of another target, and the permit for this target does.
    let other_scope = crate::composer_draft_lifecycle::ComposerDraftScope {
        account: scope.account.clone(),
        target: main_target(OTHER),
    };
    let other_lease = registry.acquire(generation, other_scope.clone()).unwrap();
    let _other_permit = registry
        .try_command_permit(generation, other_lease, &other_scope)
        .unwrap();
    let permit = registry
        .try_command_permit(generation, lease, &scope)
        .unwrap();

    let result = actor
        .handle_assistant_draft(rid(), request(main_target(ROOM)))
        .await;
    assert_eq!(result, Err(AssistantDraftFailure::Conflict));
    assert!(
        actor
            .state
            .composer_drafts
            .target_is_empty(&main_target(ROOM))
    );

    drop(permit);
    actor
        .handle_assistant_draft(rid(), request(main_target(ROOM)))
        .await
        .expect("an activation lease alone does not block an empty composer");
}

#[tokio::test]
async fn an_open_composer_with_a_pending_send_is_a_conflict() {
    let data = tempfile::tempdir().unwrap();
    let mut state = ready_state();
    state.timeline.room_id = Some(ROOM.to_owned());
    state.timeline.composer.pending_transaction_id = Some("txn".to_owned());
    let (mut actor, ..) = app_actor_event_navigation_fixture(data.path(), state);

    let result = actor
        .handle_assistant_draft(rid(), request(main_target(ROOM)))
        .await;

    assert_eq!(result, Err(AssistantDraftFailure::Conflict));
}

#[tokio::test]
async fn unknown_rooms_and_unloaded_stores_are_refused() {
    let data = tempfile::tempdir().unwrap();
    let (mut actor, ..) = app_actor_event_navigation_fixture(data.path(), ready_state());

    assert_eq!(
        actor
            .handle_assistant_draft(rid(), request(main_target("!gone:example.invalid")))
            .await,
        Err(AssistantDraftFailure::RoomNotFound)
    );

    actor.composer_draft_load_status = ComposerDraftLoadStatus::Unloaded;
    assert_eq!(
        actor
            .handle_assistant_draft(rid(), request(main_target(ROOM)))
            .await,
        Err(AssistantDraftFailure::Unavailable)
    );
    assert!(actor.state.composer_drafts.is_empty());
}
