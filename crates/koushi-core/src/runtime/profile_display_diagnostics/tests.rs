//! Sibling test module for read-receipt profile-resolution diagnostics.
//!
//! Issue #1255: the inline module reached the 200-line inline test ceiling, so
//! it lives here. All identifiers are synthetic (`example.invalid`).

use super::super::tests::unread_diagnostic_room;
use super::*;
use koushi_state::{LiveEventReceipts, LiveReadReceipt, SessionInfo};

#[test]
fn read_receipt_profile_diagnostic_reports_child_room_profile_cache_miss() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let room_id = "!child:example.invalid";
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@own:example.invalid".to_owned(),
            device_id: "OWN".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    let mut room = unread_diagnostic_room(room_id);
    room.parent_space_ids = vec!["!space:example.invalid".to_owned()];
    state.rooms.push(room);

    let action = AppAction::LiveRoomReceiptsWindowReconciled {
        room_id: room_id.to_owned(),
        scope: koushi_state::ReceiptScope::Main,
        scoped_event_ids: Vec::new(),
        receipts_by_event: vec![LiveEventReceipts {
            event_id: "$event".to_owned(),
            receipts: vec![LiveReadReceipt {
                user_id: "@child-only:example.invalid".to_owned(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(42),
            }],
        }],
    };

    let event = live_receipt_profile_diagnostic_event(&state, &action)
        .expect("receipt diagnostics should be emitted");
    assert_eq!(event.source, "core.read_receipt_profile");
    assert_eq!(event.stage, "resolution");
    let field = |key| {
        event
            .fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| &field.value)
    };
    assert_eq!(
        field("profile_cache_miss_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(1))
    );
    assert_eq!(
        field("room_in_space"),
        Some(&koushi_diagnostics::DiagnosticValue::Boolean(true))
    );
    assert_eq!(
        field("lookup_scope"),
        Some(&koushi_diagnostics::DiagnosticValue::Token(
            "global_profile_cache"
        ))
    );
    assert_eq!(
        field("unresolved_reason"),
        Some(&koushi_diagnostics::DiagnosticValue::Token(
            "profile_cache_miss"
        ))
    );
}

#[test]
fn profile_resolution_diagnostic_counts_actual_resolution_sources() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let room_id = "!resolution-room:example.invalid";
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@own:example.invalid".to_owned(),
            device_id: "OWN".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    let profile = |user_id: &str, display_name: &str| UserProfile {
        user_id: user_id.to_owned(),
        display_name: Some(display_name.to_owned()),
        display_label: String::new(),
        original_display_label: String::new(),
        mention_search_terms: Vec::new(),
        avatar: None,
    };
    state.profile.local_aliases.insert(
        "@alias:example.invalid".to_owned(),
        "Private alias".to_owned(),
    );
    state.profile.users.insert(
        "@cached:example.invalid".to_owned(),
        profile("@cached:example.invalid", "Cached label"),
    );
    state
        .profile
        .room_users
        .entry(room_id.to_owned())
        .or_default()
        .insert(
            "@room:example.invalid".to_owned(),
            profile("@room:example.invalid", "Room label"),
        );

    let receipt = |user_id: &str, display_name: Option<&str>| LiveReadReceipt {
        user_id: user_id.to_owned(),
        display_name: display_name.map(ToOwned::to_owned),
        original_display_label: String::new(),
        avatar: None,
        timestamp_ms: Some(42),
    };
    let action = AppAction::LiveRoomReceiptsWindowReconciled {
        room_id: room_id.to_owned(),
        scope: koushi_state::ReceiptScope::Main,
        scoped_event_ids: Vec::new(),
        receipts_by_event: vec![
            LiveEventReceipts {
                event_id: "$alias-event:example.invalid".to_owned(),
                receipts: vec![receipt("@alias:example.invalid", None)],
            },
            LiveEventReceipts {
                event_id: "$room-event:example.invalid".to_owned(),
                receipts: vec![receipt("@room:example.invalid", None)],
            },
            LiveEventReceipts {
                event_id: "$payload-event:example.invalid".to_owned(),
                receipts: vec![receipt("@payload:example.invalid", Some("Payload label"))],
            },
            LiveEventReceipts {
                event_id: "$cache-event:example.invalid".to_owned(),
                receipts: vec![receipt("@cached:example.invalid", None)],
            },
            LiveEventReceipts {
                event_id: "$unknown-event:example.invalid".to_owned(),
                receipts: vec![receipt("@unknown:example.invalid", None)],
            },
        ],
    };

    let event = profile_resolution_diagnostic_event(&state, &action)
        .expect("profile resolution diagnostics should be emitted");
    let field = |key| {
        event
            .fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| &field.value)
    };
    assert_eq!(
        field("input_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(5))
    );
    assert_eq!(
        field("output_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(5))
    );
    assert_eq!(
        field("local_alias_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(1))
    );
    assert_eq!(
        field("relevant_room_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(1))
    );
    assert_eq!(
        field("payload_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(1))
    );
    assert_eq!(
        field("global_cache_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(1))
    );
    assert_eq!(
        field("unresolved_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(1))
    );
    assert_eq!(
        field("cache_stale_hit_status"),
        Some(&koushi_diagnostics::DiagnosticValue::Token("not_tracked"))
    );

    let encoded = serde_json::to_string(&event).expect("diagnostic should serialize");
    for forbidden in [
        "@alias:example.invalid",
        "Private alias",
        "mxc://example.invalid/avatar",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "diagnostic leaked {forbidden}"
        );
    }
}
