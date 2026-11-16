use super::super::tests::unread_diagnostic_room;
use super::*;
use koushi_state::{RoomLatestEventSummary, SessionInfo, SessionState};

#[test]
fn room_list_applied_records_through_real_reducer_with_trace_env_unset() {
    let child = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .args([
        "--exact",
        "runtime::reducer_support::tests::room_list_applied_records_without_trace_environment",
        "--ignored",
        "--nocapture",
    ])
    .env_remove("KOUSHI_UNREAD_TRACE")
    .status()
    .expect("env-unset room-list diagnostic child should start");
    assert!(
        child.success(),
        "env-unset diagnostic child failed: {child}"
    );
}

#[test]
#[ignore]
fn room_list_applied_records_without_trace_environment() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    assert!(std::env::var_os("KOUSHI_UNREAD_TRACE").is_none());
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    let private_room_id = "!private-room:example.invalid";

    reduce_with_unread_diagnostics(
        &mut state,
        &mut unread_trace::UnreadTraceLedger::default(),
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![unread_diagnostic_room(private_room_id)],
        },
    );

    assert_eq!(state.rooms.len(), 1, "the real reducer path should run");
    let event = koushi_diagnostics::snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "core.unread" && record.event.stage == "room_list_applied"
        })
        .expect("room-list applied metrics should be collected without an env switch")
        .event;
    assert_eq!(
        event
            .fields
            .iter()
            .map(|field| (field.key, field.value.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("unread", koushi_diagnostics::DiagnosticValue::Count(3)),
            (
                "notifications",
                koushi_diagnostics::DiagnosticValue::Count(2),
            ),
            ("highlights", koushi_diagnostics::DiagnosticValue::Count(1)),
            (
                "marked_unread",
                koushi_diagnostics::DiagnosticValue::Boolean(true),
            ),
            (
                "notification_mode",
                koushi_diagnostics::DiagnosticValue::Token("unknown"),
            ),
            (
                "display_count",
                koushi_diagnostics::DiagnosticValue::Count(3)
            ),
            (
                "has_unread_content",
                koushi_diagnostics::DiagnosticValue::Boolean(true),
            ),
            (
                "is_attention_highlighted",
                koushi_diagnostics::DiagnosticValue::Boolean(true),
            ),
            (
                "has_unread_mention",
                koushi_diagnostics::DiagnosticValue::Boolean(true),
            ),
            (
                "is_muted",
                koushi_diagnostics::DiagnosticValue::Boolean(false),
            ),
            (
                "latest_event_present",
                koushi_diagnostics::DiagnosticValue::Boolean(false),
            ),
        ]
    );
    assert!(
        !serde_json::to_string(&event)
            .unwrap()
            .contains(private_room_id)
    );
}

#[test]
fn native_attention_recomputed_diagnostic_records_private_safe_fields() {
    let child = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .args([
            "--exact",
            "runtime::reducer_support::tests::native_attention_recomputed_diagnostic_records_private_safe_fields_child",
            "--ignored",
            "--nocapture",
    ])
    .status()
    .expect("native-attention diagnostic child should start");
    assert!(
        child.success(),
        "native-attention diagnostic child failed: {child}"
    );
}

#[test]
#[ignore]
fn native_attention_recomputed_diagnostic_records_private_safe_fields_child() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    let private_room_id = "!private-native-attention:example.invalid";
    let private_event_id = "$private-event:example.invalid";
    let private_user_id = "@private-sender:example.invalid";
    let private_room_label = "Private native attention room";
    let private_message = "Private native attention body";
    let mut room = unread_diagnostic_room(private_room_id);
    room.display_name = private_room_label.to_owned();
    room.display_label = private_room_label.to_owned();
    room.original_display_label = private_room_label.to_owned();
    room.unread_count = 0;
    room.notification_count = 0;
    room.highlight_count = 0;
    room.marked_unread = false;
    room.latest_event = Some(RoomLatestEventSummary {
        event_id: private_event_id.to_owned(),
        relation_type: None,
        relation_event_id: None,
        thread_root_event_id: None,
        sender_id: Some(private_user_id.to_owned()),
        sender_label: Some("Private sender".to_owned()),
        sender_avatar: None,
        preview: Some(private_message.to_owned()),
        message_text: None,
        timestamp_ms: 42,
        is_redacted: false,
    });
    reduce_with_unread_diagnostics(
        &mut state,
        &mut unread_trace::UnreadTraceLedger::default(),
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room.clone()],
        },
    );
    reduce_with_unread_diagnostics(
        &mut state,
        &mut unread_trace::UnreadTraceLedger::default(),
        AppAction::NativeWindowFocusChanged {
            focused: false,
            observation_generation: 1,
        },
    );

    room.unread_count = 1;
    room.notification_count = 1;
    room.recency_stamp = Some(43);
    room.latest_event
        .as_mut()
        .expect("latest event")
        .timestamp_ms = 43;
    reduce_with_unread_diagnostics(
        &mut state,
        &mut unread_trace::UnreadTraceLedger::default(),
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![room],
        },
    );

    let event = koushi_diagnostics::snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "native.attention" && record.event.stage == "recomputed"
        })
        .expect("native-attention recomputation should be diagnosed")
        .event;
    assert_eq!(
        event
            .fields
            .iter()
            .map(|field| (field.key, field.value.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                "observation",
                koushi_diagnostics::DiagnosticValue::Token("live"),
            ),
            (
                "unread_count",
                koushi_diagnostics::DiagnosticValue::Count(1),
            ),
            (
                "notification_count",
                koushi_diagnostics::DiagnosticValue::Count(1),
            ),
            ("badge_count", koushi_diagnostics::DiagnosticValue::Count(1),),
            (
                "badge_source",
                koushi_diagnostics::DiagnosticValue::Token("raw_unread_messages"),
            ),
            (
                "badge_room_count",
                koushi_diagnostics::DiagnosticValue::Count(1),
            ),
            (
                "badge_excluded_room_count",
                koushi_diagnostics::DiagnosticValue::Count(0),
            ),
            (
                "candidate",
                koushi_diagnostics::DiagnosticValue::Token("message"),
            ),
            (
                "suppression",
                koushi_diagnostics::DiagnosticValue::Token("none"),
            ),
            (
                "window_focused",
                koushi_diagnostics::DiagnosticValue::Boolean(false),
            ),
            (
                "active_room_match",
                koushi_diagnostics::DiagnosticValue::Boolean(true),
            ),
        ]
    );
    let serialized = serde_json::to_string(&event).unwrap();
    for private_value in [
        private_room_id,
        private_event_id,
        private_user_id,
        private_room_label,
        private_message,
    ] {
        assert!(
            !serialized.contains(private_value),
            "diagnostic leaked private value: {private_value}"
        );
    }
}
