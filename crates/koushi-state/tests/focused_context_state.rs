use koushi_state::{
    AppAction, AppEffect, AppState, ComposerMode, ComposerState, FocusedContextState,
    PendingComposerSendKind, RoomSummary, RoomTags, SessionInfo, SessionState, TimelinePaneState,
    reduce,
};
use serde_json::json;

fn ready_selected_room_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        }),
        rooms: vec![RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Synthetic Room".to_owned(),
            display_label: "Synthetic Room".to_owned(),
            original_display_label: "Synthetic Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        timeline: TimelinePaneState {
            room_id: Some("!room:example.invalid".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: ComposerState {
                pending_transaction_id: Some("txn-1".to_owned()),
                pending_send_kind: Some(PendingComposerSendKind::Reply {
                    in_reply_to_event_id: "$reply-root:example.invalid".to_owned(),
                }),
                draft: "draft".to_owned(),
                mode: ComposerMode::Reply {
                    in_reply_to_event_id: "$reply-root:example.invalid".to_owned(),
                },
            },
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
        },
        ..AppState::default()
    }
}

#[test]
fn app_state_serializes_a_focused_context_state_slot() {
    let state = AppState::default();
    let value = serde_json::to_value(state).expect("AppState serializes");

    assert_eq!(value["focused_context"], json!({ "kind": "closed" }));
}

#[test]
fn reducer_source_mentions_the_focused_context_state_machine() {
    // The reducer is split into submodules; check the delegating root and the
    // thread submodule (which owns focused-context handlers).
    let root = include_str!("../src/reducer/mod.rs");
    let thread = include_str!("../src/reducer/thread.rs");
    let source = format!("{root}{thread}");

    assert!(
        source.contains("OpenFocusedContext"),
        "focused context open action should be reduced in AppState"
    );
    assert!(
        source.contains("FocusedContextSubscribed"),
        "focused context subscription success should be reduced in AppState"
    );
    assert!(
        source.contains("CloseFocusedContext"),
        "focused context close action should be reduced in AppState"
    );
    assert!(
        source.contains("OpenFocusedTimeline"),
        "focused context open should emit a focused timeline subscription effect"
    );
}

#[test]
fn state_machine_docs_describe_the_focused_context_state_machine() {
    let docs = include_str!("../../../docs/architecture/state-machine.md");
    let focused_context_section = docs
        .split("## Focused Context")
        .nth(1)
        .expect("state-machine docs must include a Focused Context section")
        .split("## ")
        .next()
        .expect("Focused Context section should have content");

    for required in [
        "stateDiagram-v2",
        "Closed",
        "Opening",
        "Open",
        "OpenFocusedContext",
        "FocusedContextSubscribed",
        "CloseFocusedContext",
        "OpenFocusedTimeline",
        "TimelineKind::Focused",
        "ready session",
        "selected timeline room",
        "stale subscription signals are ignored",
        "close from `Closed`",
        "focused timelines do not own composer",
        "unsubscribes the previous focused timeline",
    ] {
        assert!(
            focused_context_section.contains(required),
            "Focused Context docs must mention `{required}`"
        );
    }
}

#[test]
fn focused_context_opening_does_not_mutate_the_composer_state() {
    let mut state = ready_selected_room_state();
    let before = state.timeline.composer.clone();

    let effects = reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state.focused_context,
        FocusedContextState::Opening {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        }
    );
    assert_eq!(state.timeline.composer, before);
    assert_eq!(
        effects,
        vec![AppEffect::OpenFocusedTimeline {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        }]
    );

    let effects = reduce(
        &mut state,
        AppAction::FocusedContextSubscribed {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state.focused_context,
        FocusedContextState::Open {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
            is_subscribed: true,
        }
    );
    assert_eq!(state.timeline.composer, before);
    assert_eq!(effects, Vec::new());

    let effects = reduce(&mut state, AppAction::CloseFocusedContext);
    assert_eq!(state.focused_context, FocusedContextState::Closed);
    assert_eq!(state.timeline.composer, before);
    assert_eq!(effects, Vec::new());
}

#[test]
fn focused_context_open_requires_ready_session_and_selected_room() {
    let mut signed_out = ready_selected_room_state();
    signed_out.session = SessionState::SignedOut;
    let effects = reduce(
        &mut signed_out,
        AppAction::OpenFocusedContext {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );
    assert_eq!(signed_out.focused_context, FocusedContextState::Closed);
    assert_eq!(effects, Vec::new());

    let mut different_room = ready_selected_room_state();
    let effects = reduce(
        &mut different_room,
        AppAction::OpenFocusedContext {
            room_id: "!other:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );
    assert_eq!(different_room.focused_context, FocusedContextState::Closed);
    assert_eq!(effects, Vec::new());
}

#[test]
fn stale_focused_context_subscription_signals_are_ignored() {
    let mut state = ready_selected_room_state();
    let _ = reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$wanted:example.invalid".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::FocusedContextSubscribed {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$stale:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state.focused_context,
        FocusedContextState::Opening {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$wanted:example.invalid".to_owned(),
        }
    );
    assert_eq!(effects, Vec::new());
}

#[test]
fn focused_context_close_is_noop_when_closed_or_without_ready_session() {
    let mut closed = ready_selected_room_state();
    let effects = reduce(&mut closed, AppAction::CloseFocusedContext);
    assert_eq!(closed.focused_context, FocusedContextState::Closed);
    assert_eq!(effects, Vec::new());

    let mut not_ready = ready_selected_room_state();
    not_ready.session = SessionState::SignedOut;
    not_ready.focused_context = FocusedContextState::Open {
        room_id: "!room:example.invalid".to_owned(),
        event_id: "$event:example.invalid".to_owned(),
        is_subscribed: true,
    };

    let effects = reduce(&mut not_ready, AppAction::CloseFocusedContext);
    assert_eq!(
        not_ready.focused_context,
        FocusedContextState::Open {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
            is_subscribed: true,
        }
    );
    assert_eq!(effects, Vec::new());
}
