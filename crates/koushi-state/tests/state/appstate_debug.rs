//! #1985: `AppState` is the app's general diagnostic dump, so it renders the
//! account's shape — kinds and counts — and never the identities, text, drafts
//! or previews it reaches. `SessionInfo` and `ThreadsListItem` are printed
//! directly by other paths too, so they redact on their own.

use koushi_state::{
    AppError, AppState, BasicOperationState, ComposerMode, ComposerState, FilesViewState,
    PendingComposerSendKind, SessionAuthenticationMethod, SessionInfo, SessionState, SyncState,
    ThreadOpenIntent, ThreadPaneState, ThreadRootAttention, ThreadsListItem, ThreadsListState,
    TimelinePaneState,
};

const PRIVATE_HOMESERVER: &str = "https://private-homeserver.invalid";
const PRIVATE_USER: &str = "@private-user:private-homeserver.invalid";
const PRIVATE_DEVICE: &str = "PRIVATE-DEVICE";
const PRIVATE_ROOM: &str = "!private-room:private-homeserver.invalid";
const PRIVATE_ROOT: &str = "$private-root:private-homeserver.invalid";
const PRIVATE_LATEST: &str = "$private-latest:private-homeserver.invalid";
const PRIVATE_SPACE: &str = "!private-space:private-homeserver.invalid";
const PRIVATE_BODY: &str = "private body text";

fn private_values() -> [&'static str; 8] {
    [
        PRIVATE_HOMESERVER,
        PRIVATE_SPACE,
        PRIVATE_USER,
        PRIVATE_DEVICE,
        PRIVATE_ROOM,
        PRIVATE_ROOT,
        PRIVATE_LATEST,
        PRIVATE_BODY,
    ]
}

fn assert_redacted(debug: &str) {
    for raw in private_values() {
        assert!(!debug.contains(raw), "raw {raw} leaked: {debug}");
    }
    for forbidden in [":private-homeserver.invalid", "Private Sender", "mxc://"] {
        assert!(!debug.contains(forbidden), "leaked {forbidden}: {debug}");
    }
}

fn session() -> SessionState {
    SessionState::Ready(SessionInfo {
        homeserver: PRIVATE_HOMESERVER.to_owned(),
        user_id: PRIVATE_USER.to_owned(),
        device_id: PRIVATE_DEVICE.to_owned(),
        authentication_method: SessionAuthenticationMethod::Password,
    })
}

fn thread_item() -> ThreadsListItem {
    ThreadsListItem {
        room_id: PRIVATE_ROOM.to_owned(),
        root_event_id: PRIVATE_ROOT.to_owned(),
        root_sender: PRIVATE_USER.to_owned(),
        root_sender_label: Some("Private Sender".to_owned()),
        root_body_preview: Some(PRIVATE_BODY.to_owned()),
        root_timestamp_ms: Some(1_800_000_000_000),
        latest_event_id: Some(PRIVATE_LATEST.to_owned()),
        latest_sender: Some(PRIVATE_USER.to_owned()),
        latest_sender_label: Some("Private Sender".to_owned()),
        latest_body_preview: Some(PRIVATE_BODY.to_owned()),
        latest_timestamp_ms: Some(1_800_000_000_000),
        reply_count: 2,
        unread_count: 1,
    }
}

#[test]
fn appstate_debug_renders_shape_instead_of_identities_and_text() {
    let mut state = AppState {
        session: session(),
        ..AppState::default()
    };
    state.navigation.active_room_id = Some(PRIVATE_ROOM.to_owned());
    state.navigation.space_order = vec![PRIVATE_ROOM.to_owned()];
    state.thread_unread.insert(
        (PRIVATE_ROOM.to_owned(), PRIVATE_ROOT.to_owned()),
        ThreadRootAttention {
            unread: 1,
            highlight: 0,
        },
    );
    state.threads_list = ThreadsListState::Open {
        room_id: PRIVATE_ROOM.to_owned(),
        request_id: 1,
        items: vec![thread_item()],
        is_paginating: false,
        end_reached: true,
    };
    state
        .settings
        .values
        .sidebar
        .scope_preferences
        .insert(PRIVATE_SPACE.to_owned(), Default::default());
    state.errors = vec![AppError::new("private_code", PRIVATE_BODY, true)];

    let debug = format!("{state:?}");
    assert_redacted(&debug);
    // The shape stays, because that is what a failing test needs to read.
    assert!(debug.contains("threads_list_items: 1"), "{debug}");
    assert!(debug.contains("errors: 1"), "{debug}");
    assert!(debug.contains("thread_unread: 1"), "{debug}");
    assert!(debug.contains("session: Ready(SessionInfo"), "{debug}");
}

#[test]
fn session_info_debug_redacts_the_account_and_device() {
    let debug = format!("{:?}", session());
    assert_redacted(&debug);
    assert!(debug.contains("homeserver: \"Homeserver(..)\""), "{debug}");
    assert!(debug.contains("user_id: \"UserId(..)\""), "{debug}");
    assert!(debug.contains("device_id: \"DeviceId(..)\""), "{debug}");
    assert!(debug.contains("authentication_method: Password"), "{debug}");
}

#[test]
fn threads_list_item_debug_redacts_the_row_and_its_previews() {
    let item = thread_item();
    let debug = format!("{item:?}");
    assert_redacted(&debug);
    for placeholder in ["RoomId(..)", "EventId(..)", "UserId(..)", "MessageBody(..)"] {
        assert!(
            debug.contains(placeholder),
            "missing {placeholder}: {debug}"
        );
    }
    assert!(debug.contains("reply_count: 2"), "{debug}");
    assert!(debug.contains("unread_count: 1"), "{debug}");
}

#[test]
fn authenticating_session_debug_redacts_the_typed_homeserver() {
    let state = SessionState::Authenticating {
        homeserver: PRIVATE_HOMESERVER.to_owned(),
        attempt_id: koushi_state::LoginAttemptId::new(4, 9),
    };
    let debug = format!("{state:?}");
    assert_redacted(&debug);
    assert!(debug.contains("homeserver: \"Homeserver(..)\""), "{debug}");
}

#[test]
fn sync_state_debug_redacts_the_transport_reason() {
    for state in [
        SyncState::Failed {
            reason: "@alice:example.invalid leaked-secret".to_owned(),
        },
        SyncState::Reconnecting {
            reason: "@alice:example.invalid leaked-secret".to_owned(),
        },
    ] {
        let debug = format!("{state:?}");
        assert!(!debug.contains("alice"), "{debug}");
        assert!(!debug.contains("leaked-secret"), "{debug}");
        assert!(debug.contains("Reason(..)"), "{debug}");
    }
}

#[test]
fn composer_debug_reports_the_draft_length_instead_of_its_text() {
    let composer = ComposerState {
        draft: PRIVATE_BODY.to_owned(),
        ..ComposerState::default()
    };
    let debug = format!("{composer:?}");
    assert_redacted(&debug);
    assert!(debug.contains("draft_len: "), "{debug}");
}

#[test]
fn panes_debug_redact_their_room_root_and_composer_text() {
    let timeline = TimelinePaneState {
        room_id: Some(PRIVATE_ROOM.to_owned()),
        composer: ComposerState {
            draft: PRIVATE_BODY.to_owned(),
            ..ComposerState::default()
        },
        ..TimelinePaneState::default()
    };
    let debug = format!("{timeline:?}");
    assert_redacted(&debug);
    assert!(debug.contains("RoomId(..)"), "{debug}");

    let thread = ThreadPaneState::Open {
        room_id: PRIVATE_ROOM.to_owned(),
        root_event_id: PRIVATE_ROOT.to_owned(),
        intent: koushi_state::ThreadOpenIntent::ExistingThread,
        is_subscribed: true,
        composer: ComposerState {
            draft: PRIVATE_BODY.to_owned(),
            ..ComposerState::default()
        },
        staged_uploads: Vec::new(),
    };
    let debug = format!("{thread:?}");
    assert_redacted(&debug);
    assert!(
        debug.contains("RoomId(..)") && debug.contains("EventId(..)"),
        "{debug}"
    );
}

#[test]
fn a_reply_composer_debug_names_the_mode_instead_of_the_event_it_replies_to() {
    let composer = ComposerState {
        pending_send_kind: Some(PendingComposerSendKind::Reply {
            in_reply_to_event_id: PRIVATE_ROOT.to_owned(),
        }),
        mode: ComposerMode::Reply {
            in_reply_to_event_id: PRIVATE_ROOT.to_owned(),
        },
        ..ComposerState::default()
    };
    let debug = format!("{composer:?}");
    assert_redacted(&debug);
    assert_eq!(debug.matches("Reply").count(), 2, "{debug}");
}

#[test]
fn a_pinned_reply_thread_pane_debug_names_the_intent_instead_of_its_event() {
    let thread = ThreadPaneState::Open {
        room_id: PRIVATE_ROOM.to_owned(),
        root_event_id: PRIVATE_ROOT.to_owned(),
        intent: ThreadOpenIntent::PinnedReply {
            event_id: PRIVATE_LATEST.to_owned(),
        },
        is_subscribed: true,
        composer: ComposerState::default(),
        staged_uploads: Vec::new(),
    };
    let debug = format!("{thread:?}");
    assert_redacted(&debug);
    assert!(debug.contains("PinnedReply"), "{debug}");
}

#[test]
fn in_flight_operations_debug_name_their_kind_not_the_typed_name_or_rooms() {
    for state in [
        BasicOperationState::CreatingRoom {
            request_id: 7,
            name: "Private Room Name".to_owned(),
        },
        BasicOperationState::CreatingSpace {
            request_id: 7,
            name: "Private Room Name".to_owned(),
        },
        BasicOperationState::LinkingSpaceChild {
            request_id: 7,
            space_id: PRIVATE_SPACE.to_owned(),
            child_room_id: PRIVATE_ROOM.to_owned(),
        },
    ] {
        let debug = format!("{state:?}");
        assert_redacted(&debug);
        assert!(!debug.contains("Private Room Name"), "{debug}");
    }
    let failed = FilesViewState::Failed {
        request_id: 7,
        scope: koushi_state::AttachmentScope::Room {
            room_id: PRIVATE_ROOM.to_owned(),
        },
        filter: koushi_state::AttachmentFilter::default(),
        sort: koushi_state::AttachmentSort::NewestFirst,
        message: PRIVATE_BODY.to_owned(),
    };
    let debug = format!("{failed:?}");
    assert_redacted(&debug);
    assert!(debug.contains("has_message: true"), "{debug}");
}
