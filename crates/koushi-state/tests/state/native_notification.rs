//! Rust-owned desktop notification text and navigation target.
//!
//! These tests drive the attention projection directly: the notification
//! payload must describe the same room/event the selected candidate came from,
//! must keep message content out unless the device-local setting is on, and
//! must never be built for a suppressed candidate.

use koushi_state::{
    AppAction, AppState, NativeAttentionCandidate, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionObservationKind, NativeAttentionProjectionInput,
    RoomAttentionKind, RoomLatestEventSummary, RoomNotificationMode, RoomSummary, RoomTags,
    SessionInfo, SessionState, native_attention_projection_from_rooms, reduce,
};
use std::collections::{BTreeSet, HashMap};

fn capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

fn room(room_id: &str, is_dm: bool, unread: u64, highlight: u64) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: unread,
        notification_count: unread,
        highlight_count: highlight,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(42),
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

fn latest_event(event_id: &str, sender: &str, preview: &str) -> RoomLatestEventSummary {
    RoomLatestEventSummary {
        event_id: event_id.to_owned(),
        relation_type: None,
        relation_event_id: None,
        thread_root_event_id: None,
        sender_id: Some("@sender:example.invalid".to_owned()),
        sender_label: Some(sender.to_owned()),
        sender_avatar: None,
        preview: Some(preview.to_owned()),
        message_text: None,
        timestamp_ms: 42,
        is_redacted: false,
    }
}

#[test]
fn server_only_notification_count_does_not_raise_a_desktop_notification() {
    let mut phantom = room("!room:example.invalid", false, 0, 0);
    phantom.notification_count = 3;
    phantom.latest_event = Some(latest_event(
        "$old:example.invalid",
        "Sender",
        "Old message",
    ));

    let state = project(
        &[phantom],
        false,
        NativeAttentionObservationKind::Live,
        None,
    );
    assert_eq!(state.summary.badge_count, 0);
    assert!(state.notification.is_none());
}

#[test]
fn unread_count_without_a_verified_latest_event_does_not_raise_a_desktop_notification() {
    let unread = room("!room:example.invalid", false, 2, 0);
    let state = project(&[unread], false, NativeAttentionObservationKind::Live, None);
    assert_eq!(state.summary.badge_count, 2);
    assert!(state.notification.is_none());
}

#[test]
fn server_highlights_and_unmatched_latest_event_do_not_enter_a_desktop_alert() {
    let mut unread = room("!room:example.invalid", false, 1, 9);
    unread.notification_count = 9;
    unread.latest_event = Some(latest_event(
        "$unrelated:example.invalid",
        "Sender",
        "Unrelated content",
    ));
    let state = project(&[unread], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("room-level unread alert");
    assert_eq!(payload.title, "Message in Room");
    assert_eq!(payload.body, "1 unread");
    assert_eq!(payload.target.event_id, None);
    assert_eq!(payload.target.thread_root_event_id, None);
}

#[test]
fn server_highlight_growth_does_not_replay_an_old_room_notification() {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@viewer:example.invalid".to_owned(),
            device_id: "VIEWER".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    let mut first = room("!room:example.invalid", false, 1, 0);
    first.latest_event = Some(latest_event("$old:example.invalid", "Sender", "Old"));
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![first.clone()],
        },
    );

    first.notification_count = 9;
    first.highlight_count = 9;
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![first],
        },
    );
    assert_eq!(state.native_attention.summary.badge_count, 1);
    assert!(state.native_attention.notification.is_none());
}

fn project(
    rooms: &[RoomSummary],
    message_previews: bool,
    observation: NativeAttentionObservationKind,
    previous_candidate: Option<&NativeAttentionCandidate>,
) -> koushi_state::NativeAttentionState {
    native_attention_projection_from_rooms(NativeAttentionProjectionInput {
        rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &HashMap::new(),
        ignored_user_ids: &BTreeSet::new(),
        window_focused: false,
        observation,
        previous_candidate,
        message_previews,
        capabilities: capabilities(),
        locale: koushi_state::CatalogLocale::En,
    })
    .state
}

#[test]
fn previews_setting_never_claims_an_unverified_event_or_content() {
    let mut dm = room("!dm:example.invalid", true, 2, 0);
    dm.latest_event = Some(latest_event(
        "$secret:example.invalid",
        "Alice",
        "private body",
    ));

    for previews in [false, true] {
        let state = project(
            &[dm.clone()],
            previews,
            NativeAttentionObservationKind::Live,
            None,
        );
        let payload = state
            .notification
            .as_ref()
            .expect("room-level notification");
        assert_eq!(payload.title, "Direct message in Room");
        assert_eq!(payload.body, "2 unread");
        assert_eq!(payload.target.room_id, "!dm:example.invalid");
        assert_eq!(payload.target.event_id, None);
        assert_eq!(payload.target.thread_root_event_id, None);
        assert_eq!(label(&state), "Room");
    }
}

#[test]
fn missing_room_history_does_not_claim_a_new_message() {
    let mut room = room("!room:example.invalid", false, 1, 0);
    room.latest_event = None;

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    assert!(state.notification.is_none());
}

#[test]
fn suppressed_candidates_never_produce_notification_text() {
    let mut room = room("!room:example.invalid", false, 2, 0);
    room.latest_event = Some(latest_event(
        "$event:example.invalid",
        "Alice",
        "private body",
    ));

    for observation in [
        NativeAttentionObservationKind::InitialSync,
        NativeAttentionObservationKind::Backfill,
        NativeAttentionObservationKind::SelfEvent,
    ] {
        let state = project(&[room.clone()], true, observation, None);
        assert_eq!(state.summary.candidate, None, "observation {observation:?}");
        assert!(state.notification.is_none(), "observation {observation:?}");
    }

    // Deduplication: an unchanged candidate is suppressed and must not produce
    // a second banner payload.
    let candidate = project(
        &[room.clone()],
        true,
        NativeAttentionObservationKind::Live,
        None,
    )
    .summary
    .candidate
    .expect("candidate");
    let duplicate = project(
        &[room],
        true,
        NativeAttentionObservationKind::Live,
        Some(&candidate),
    );
    assert_eq!(duplicate.summary.candidate, None);
    assert!(duplicate.notification.is_none());
}

#[test]
fn muted_rooms_produce_no_notification_text() {
    let mut room = room("!room:example.invalid", false, 2, 0);
    room.latest_event = Some(latest_event(
        "$event:example.invalid",
        "Alice",
        "private body",
    ));

    let modes = HashMap::from([(
        "!room:example.invalid".to_owned(),
        RoomNotificationMode::Mute,
    )]);
    let state = native_attention_projection_from_rooms(NativeAttentionProjectionInput {
        rooms: &[room],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &modes,
        ignored_user_ids: &BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        message_previews: true,
        capabilities: capabilities(),
        locale: koushi_state::CatalogLocale::En,
    })
    .state;

    assert_eq!(state.summary.candidate, None);
    assert!(state.notification.is_none());
}

#[test]
fn notification_debug_redacts_the_body_and_the_target() {
    let mut room = room("!room:example.invalid", false, 1, 0);
    room.latest_event = Some(latest_event(
        "$event:example.invalid",
        "Alice",
        "private body",
    ));

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let rendered = format!("{:?}", state);

    assert!(!rendered.contains("private body"));
    assert!(!rendered.contains("!room:example.invalid"));
    assert!(!rendered.contains("$event:example.invalid"));
    assert!(!rendered.contains("Mention in Room"));
    assert!(rendered.contains("NotificationBody(..)"));
}

fn label(state: &koushi_state::NativeAttentionState) -> String {
    state
        .summary
        .candidate
        .as_ref()
        .map(|candidate| candidate.room_display_name.clone())
        .expect("candidate room label")
}

#[test]
fn kind_specific_titles_match_the_notification_kind() {
    for (kind, expected) in [
        (RoomAttentionKind::Dm, "Direct message in Room"),
        (RoomAttentionKind::Message, "Message in Room"),
    ] {
        let mut room = if kind == RoomAttentionKind::Dm {
            room("!dm:example.invalid", true, 1, 0)
        } else {
            room("!room:example.invalid", false, 1, 0)
        };
        room.latest_event = Some(latest_event(
            "$observed:example.invalid",
            "Sender",
            "Message",
        ));
        let state = project(&[room], false, NativeAttentionObservationKind::Live, None);
        let payload = state.notification.expect("notification payload");
        assert_eq!(payload.title, expected);
    }
}

/// #994: the preview switch belongs to each account. Two signed-in accounts that see
/// the same live message each follow their own setting, and only a verified live
/// rise carries text.
#[test]
fn each_account_follows_its_own_preview_setting() {
    fn account(user: &str, message_previews: bool) -> AppState {
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: user.to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            ..AppState::default()
        };
        state.settings.values.notifications.message_previews = message_previews;
        state.native_attention.summary.capabilities = capabilities();
        state.native_attention_context.window_focused = false;
        state
    }
    fn receive(state: &mut AppState) -> Option<String> {
        let mut read = room("!dm:example.invalid", true, 0, 0);
        read.latest_event = Some(latest_event("$old:example.invalid", "Alice", "old"));
        reduce(
            state,
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![read],
            },
        );
        let mut unread = room("!dm:example.invalid", true, 1, 0);
        let mut event = latest_event("$new:example.invalid", "Alice", "preview");
        event.message_text = Some("decrypted private text".to_owned());
        unread.latest_event = Some(event);
        reduce(
            state,
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![unread],
            },
        );
        state
            .native_attention
            .notification
            .as_ref()
            .map(|payload| payload.body.clone())
    }

    let mut previews_on = account("@first:example.invalid", true);
    let mut previews_off = account("@second:example.invalid", false);
    assert_eq!(
        receive(&mut previews_on).as_deref(),
        Some("decrypted private text")
    );
    assert_eq!(receive(&mut previews_off).as_deref(), Some("1 unread"));
}

/// #994: an unread rise alone does not prove which event raised it. The preview
/// shows text only when the latest event changed in that same observation and
/// another user sent it; otherwise the banner keeps the count body.
#[test]
fn a_preview_needs_a_new_latest_event_from_another_user() {
    fn observe(previous_event: &str, event: &str, sender: &str) -> Option<String> {
        let mut state = AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://example.invalid".to_owned(),
                user_id: "@me:example.invalid".to_owned(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            ..AppState::default()
        };
        state.settings.values.notifications.message_previews = true;
        state.native_attention.summary.capabilities = capabilities();
        state.native_attention_context.window_focused = false;
        let mut read = room("!dm:example.invalid", true, 0, 0);
        read.latest_event = Some(latest_event(previous_event, "Alice", "old"));
        reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![read],
            },
        );
        let mut unread = room("!dm:example.invalid", true, 1, 0);
        let mut latest = latest_event(event, "Alice", "preview");
        latest.sender_id = Some(sender.to_owned());
        latest.message_text = Some("decrypted private text".to_owned());
        unread.latest_event = Some(latest);
        reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![unread],
            },
        );
        state
            .native_attention
            .notification
            .as_ref()
            .map(|payload| payload.body.clone())
    }

    assert_eq!(
        observe(
            "$old:example.invalid",
            "$new:example.invalid",
            "@alice:example.invalid"
        )
        .as_deref(),
        Some("decrypted private text")
    );
    // The same latest event: the rise came from something the room list does not name.
    assert_eq!(
        observe(
            "$same:example.invalid",
            "$same:example.invalid",
            "@alice:example.invalid"
        )
        .as_deref(),
        Some("1 unread")
    );
    // The user's own message is never shown back to them as a preview.
    assert_eq!(
        observe(
            "$old:example.invalid",
            "$new:example.invalid",
            "@me:example.invalid"
        )
        .as_deref(),
        Some("1 unread")
    );
}
