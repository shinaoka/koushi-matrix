//! Rust-owned desktop notification text and navigation target.
//!
//! These tests drive the attention projection directly: the notification
//! payload must describe the same room/event the selected candidate came from,
//! must keep message content out unless the device-local setting is on, and
//! must never be built for a suppressed candidate.

use koushi_state::{
    NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomAttentionKind,
    RoomLatestEventSummary, RoomNotificationMode, RoomSummary, RoomTags,
    native_attention_projection_from_rooms,
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
        timestamp_ms: 42,
        is_redacted: false,
    }
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
    })
    .state
}

#[test]
fn previews_off_keeps_message_content_out_of_the_notification() {
    // The same triggering event serves both runs, so the only difference is the
    // device-local setting.
    let mut dm = room("!dm:example.invalid", true, 2, 0);
    dm.latest_event = Some(latest_event(
        "$secret:example.invalid",
        "Alice",
        "private body",
    ));

    let without_previews = project(
        &[dm.clone()],
        false,
        NativeAttentionObservationKind::Live,
        None,
    );
    let payload = without_previews
        .notification
        .clone()
        .expect("notification payload for the selected candidate");

    assert_eq!(payload.title, "Direct message in Room");
    assert_eq!(payload.body, "2 unread");
    assert!(!payload.body.contains("private body"));
    assert!(!payload.title.contains("private body"));
    assert_eq!(payload.target.room_id, "!dm:example.invalid");
    assert_eq!(
        payload.target.event_id.as_deref(),
        Some("$secret:example.invalid")
    );
    assert_eq!(label(&without_previews), "Room");
}

#[test]
fn previews_on_shows_the_triggering_events_content_and_target() {
    let mut room = room("!room:example.invalid", false, 3, 1);
    room.latest_event = Some(latest_event(
        "$mention:example.invalid",
        "Alice",
        "are you around?",
    ));

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("notification payload");

    assert_eq!(payload.title, "Mention in Room");
    assert_eq!(payload.body, "Alice: are you around?");
    assert_eq!(payload.target.room_id, "!room:example.invalid");
    assert_eq!(
        payload.target.event_id.as_deref(),
        Some("$mention:example.invalid")
    );
    assert_eq!(payload.target.thread_root_event_id, None);
}

#[test]
fn previews_on_keeps_the_thread_root_of_the_triggering_reply() {
    let mut room = room("!room:example.invalid", false, 1, 0);
    let mut event = latest_event("$reply:example.invalid", "Bob", "thread reply");
    event.thread_root_event_id = Some("$root:example.invalid".to_owned());
    room.latest_event = Some(event);

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("notification payload");

    assert_eq!(
        payload.target.event_id.as_deref(),
        Some("$reply:example.invalid")
    );
    assert_eq!(
        payload.target.thread_root_event_id.as_deref(),
        Some("$root:example.invalid")
    );
}

#[test]
fn previews_on_uses_the_projected_fallback_text_for_unavailable_content() {
    for (preview, expected) in [
        (
            "Unable to decrypt message",
            "Alice: Unable to decrypt message",
        ),
        ("Message deleted", "Alice: Message deleted"),
        ("m.sticker", "Alice: m.sticker"),
    ] {
        let mut room = room("!room:example.invalid", false, 1, 0);
        room.is_encrypted = true;
        room.latest_event = Some(latest_event("$event:example.invalid", "Alice", preview));

        let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
        let payload = state.notification.expect("notification payload");

        assert_eq!(payload.body, expected);
    }
}

#[test]
fn previews_on_falls_back_to_counts_when_no_preview_exists() {
    let mut room = room("!room:example.invalid", false, 4, 0);
    room.latest_event = Some(RoomLatestEventSummary {
        preview: None,
        ..latest_event("$event:example.invalid", "Alice", "unused")
    });

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("notification payload");

    assert_eq!(payload.body, "4 unread");
    assert_eq!(
        payload.target.event_id.as_deref(),
        Some("$event:example.invalid")
    );
}

#[test]
fn previews_on_collapses_whitespace_and_bounds_the_body() {
    let mut room = room("!room:example.invalid", false, 1, 0);
    let long = "x".repeat(400);
    room.latest_event = Some(latest_event(
        "$event:example.invalid",
        "Alice",
        &format!("first\n\n  second {long}"),
    ));

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("notification payload");

    assert!(payload.body.starts_with("Alice: first second "));
    assert_eq!(payload.body.chars().count(), 201);
    assert!(payload.body.ends_with('…'));
}

#[test]
fn previews_on_bounds_multibyte_previews_on_character_boundaries() {
    let mut room = room("!room:example.invalid", false, 1, 0);
    room.latest_event = Some(latest_event(
        "$event:example.invalid",
        "アリス",
        &"あ".repeat(300),
    ));

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("notification payload");

    assert_eq!(payload.body.chars().count(), 201);
    assert!(payload.body.ends_with('…'));
}

#[test]
fn missing_room_history_still_opens_the_room() {
    let mut room = room("!room:example.invalid", false, 1, 0);
    room.latest_event = None;

    let state = project(&[room], true, NativeAttentionObservationKind::Live, None);
    let payload = state.notification.expect("notification payload");

    assert_eq!(payload.target.room_id, "!room:example.invalid");
    assert_eq!(payload.target.event_id, None);
    assert_eq!(payload.target.thread_root_event_id, None);
    assert_eq!(payload.body, "1 unread");
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
        (RoomAttentionKind::Mention, "Mention in Room"),
        (RoomAttentionKind::Dm, "Direct message in Room"),
        (RoomAttentionKind::Message, "Message in Room"),
    ] {
        let room = if kind == RoomAttentionKind::Dm {
            room("!dm:example.invalid", true, 1, 0)
        } else if kind == RoomAttentionKind::Mention {
            room("!room:example.invalid", false, 1, 1)
        } else {
            room("!room:example.invalid", false, 1, 0)
        };
        let state = project(&[room], false, NativeAttentionObservationKind::Live, None);
        let payload = state.notification.expect("notification payload");
        assert_eq!(payload.title, expected);
    }
}
