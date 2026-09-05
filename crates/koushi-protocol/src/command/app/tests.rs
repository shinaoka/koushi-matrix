use super::super::{CoreCommand, test_support::fake_rid};
use super::*;
use koushi_state::{
    ImageUploadCompressionMode, NativeAttentionCandidate, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionDispatchState, NativeAttentionState,
    NativeAttentionSummary, NativeAttentionSuppressionReason, RoomAttentionKind, ThreadOpenIntent,
};

#[test]
fn navigate_to_event_is_correlated_and_redacts_identifiers() {
    let request_id = fake_rid(76);
    let command = AppCommand::NavigateToEvent {
        request_id,
        room_id: "!private-room:example.invalid".to_owned(),
        event_id: "$private-event:example.invalid".to_owned(),
        source: koushi_state::EventNavigationSource::Activity,
        missing_target_policy: EventNavigationMissingTargetPolicy::LiveFallback,
    };

    assert_eq!(CoreCommand::App(command).request_id(), request_id);
    let debug = format!(
        "{:?}",
        AppCommand::NavigateToEvent {
            request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            event_id: "$private-event:example.invalid".to_owned(),
            source: koushi_state::EventNavigationSource::Activity,
            missing_target_policy: EventNavigationMissingTargetPolicy::LiveFallback,
        }
    );
    assert!(debug.contains("Activity"), "{debug}");
    assert!(debug.contains("LiveFallback"), "{debug}");
    assert_eq!(
        serde_json::to_string(&EventNavigationSource::Activity).unwrap(),
        "\"activity\""
    );
    assert_eq!(
        serde_json::to_string(&EventNavigationMissingTargetPolicy::LiveFallback).unwrap(),
        "\"liveFallback\""
    );
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    assert!(!debug.contains("$private-event:example.invalid"), "{debug}");
}

#[test]
fn open_thread_command_retains_typed_intent_and_redacts_identifiers() {
    let request_id = fake_rid(75);
    let command = AppCommand::OpenThread {
        request_id,
        room_id: "!private-room:example.invalid".to_owned(),
        root_event_id: "$private-root:example.invalid".to_owned(),
        intent: ThreadOpenIntent::NewThreadDraft,
    };

    assert_eq!(CoreCommand::App(command).request_id(), request_id);
    let debug = format!(
        "{:?}",
        AppCommand::OpenThread {
            request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            root_event_id: "$private-root:example.invalid".to_owned(),
            intent: ThreadOpenIntent::NewThreadDraft,
        }
    );
    assert!(debug.contains("NewThreadDraft"), "{debug}");
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    assert!(!debug.contains("$private-root:example.invalid"), "{debug}");
}

#[test]
fn set_room_url_preview_override_debug_redacts_room_id() {
    let command = AppCommand::SetRoomUrlPreviewOverride {
        request_id: fake_rid(14),
        room_id: "!room:example.invalid".to_owned(),
        enabled: false,
    };
    let debug = format!("{command:?}");
    assert!(debug.contains("SetRoomUrlPreviewOverride"), "{debug}");
    assert!(debug.contains("RoomId(..)"), "{debug}");
    assert!(debug.contains("enabled"), "{debug}");
    assert!(!debug.contains("!room:example.invalid"), "{debug}");
}

#[test]
fn activity_commands_debug_redacts_targets_and_carry_request_ids() {
    use koushi_state::{ActivityMarkReadTarget, ActivityTab};

    let set_tab_request_id = fake_rid(21);
    let set_tab = AppCommand::SetActivityTab {
        request_id: set_tab_request_id,
        tab: ActivityTab::Unread,
    };
    let paginate_request_id = fake_rid(22);
    let paginate = AppCommand::PaginateActivity {
        request_id: paginate_request_id,
        tab: ActivityTab::Recent,
        cursor: Some("private-page-token".to_owned()),
    };
    let mark_request_id = fake_rid(23);
    let mark = AppCommand::MarkActivityRead {
        request_id: mark_request_id,
        target: ActivityMarkReadTarget::Room {
            room_id: "!private-room:example.invalid".to_owned(),
            up_to_event_id: "$private-event:example.invalid".to_owned(),
        },
    };

    assert_eq!(CoreCommand::App(set_tab).request_id(), set_tab_request_id);
    assert_eq!(CoreCommand::App(paginate).request_id(), paginate_request_id);
    assert_eq!(
        CoreCommand::App(AppCommand::MarkActivityRead {
            request_id: mark_request_id,
            target: ActivityMarkReadTarget::All,
        })
        .request_id(),
        mark_request_id
    );

    for debug in [
        format!(
            "{:?}",
            AppCommand::PaginateActivity {
                request_id: fake_rid(24),
                tab: ActivityTab::Unread,
                cursor: Some("private-page-token".to_owned()),
            }
        ),
        format!("{mark:?}"),
    ] {
        assert!(!debug.contains("private-page-token"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
        assert!(!debug.contains("$private-event:example.invalid"), "{debug}");
    }
}

#[test]
fn upload_staging_commands_are_correlated_and_redact_debug() {
    use koushi_state::StagedUploadKind;

    let set_request_id = fake_rid(24);
    let update_caption_request_id = fake_rid(25);
    let update_compression_request_id = fake_rid(26);
    let clear_request_id = fake_rid(27);
    let target = koushi_state::ComposerTarget::Main {
        room_id: "!private-room:example.invalid".to_owned(),
    };
    let set = AppCommand::SetUploadStaging {
        request_id: set_request_id,
        target: target.clone(),
        items: vec![StagedUploadItem {
            staged_id: "private-staged-id".to_owned(),
            room_id: "!private-room:example.invalid".to_owned(),
            position: 1,
            filename: "private-image.png".to_owned(),
            mime_type: "image/png".to_owned(),
            byte_count: 99,
            kind: StagedUploadKind::Image {
                width: Some(4),
                height: Some(2),
            },
            caption: Some(ComposerDocument::from_plain_text("private staged caption")),
            compression_choice: StagedUploadCompressionChoice::Original,
            preparation: Default::default(),
        }],
    };
    let update_caption = AppCommand::UpdateStagedUploadCaption {
        request_id: update_caption_request_id,
        target: target.clone(),
        staged_id: "private-staged-id".to_owned(),
        caption: Some(ComposerDocument::from_plain_text("private staged caption")),
    };
    let update_compression = AppCommand::UpdateStagedUploadCompression {
        request_id: update_compression_request_id,
        target: target.clone(),
        staged_id: "private-staged-id".to_owned(),
        compression_choice: StagedUploadCompressionChoice::Compressed {
            mode: ImageUploadCompressionMode::Always,
        },
    };
    let clear = AppCommand::ClearUploadStaging {
        request_id: clear_request_id,
        target: target.clone(),
    };

    assert_eq!(CoreCommand::App(set).request_id(), set_request_id);
    for debug in [
        format!("{update_caption:?}"),
        format!("{update_compression:?}"),
        format!("{clear:?}"),
        format!(
            "{:?}",
            AppCommand::SetUploadStaging {
                request_id: set_request_id,
                target,
                items: vec![StagedUploadItem {
                    staged_id: "private-staged-id".to_owned(),
                    room_id: "!private-room:example.invalid".to_owned(),
                    position: 1,
                    filename: "private-image.png".to_owned(),
                    mime_type: "image/png".to_owned(),
                    byte_count: 99,
                    kind: StagedUploadKind::File,
                    caption: None,
                    compression_choice: StagedUploadCompressionChoice::NotApplicable,
                    preparation: Default::default(),
                }],
            }
        ),
    ] {
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
        assert!(!debug.contains("private-staged-id"), "{debug}");
        assert!(!debug.contains("private-image.png"), "{debug}");
        assert!(!debug.contains("private staged caption"), "{debug}");
    }
}

#[test]
fn open_timeline_at_timestamp_is_correlated_and_redacts_debug() {
    let request_id = fake_rid(28);
    let command = AppCommand::OpenTimelineAtTimestamp {
        request_id,
        room_id: "!private-room:example.invalid".to_owned(),
        timestamp_ms: 1_718_000_000_000,
    };

    assert_eq!(CoreCommand::App(command).request_id(), request_id);
    let debug = format!(
        "{:?}",
        AppCommand::OpenTimelineAtTimestamp {
            request_id,
            room_id: "!private-room:example.invalid".to_owned(),
            timestamp_ms: 1_718_000_000_000,
        }
    );
    assert!(debug.contains("RoomId(..)"), "{debug}");
    assert!(debug.contains("Timestamp(..)"), "{debug}");
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    assert!(!debug.contains("1718000000000"), "{debug}");
}

#[test]
fn focused_projection_command_redacts_matrix_identifiers() {
    let debug = format!(
        "{:?}",
        AppCommand::OpenAnchoredTimeline {
            request_id: fake_rid(29),
            room_id: "!private-room:example.invalid".to_owned(),
            event_id: "$private-event:example.invalid".to_owned(),
            allow_live_fallback: false,
        }
    );
    assert!(debug.contains("RoomId(..)"), "{debug}");
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    assert!(!debug.contains("$private-event:example.invalid"), "{debug}");
}

#[test]
fn native_attention_command_debug_redacts_candidate_labels() {
    let command = AppCommand::UpdateNativeAttentionState {
        request_id: fake_rid(27),
        attention: NativeAttentionState {
            summary: NativeAttentionSummary {
                unread_count: 4,
                highlight_count: 1,
                badge_count: 4,
                candidate: Some(NativeAttentionCandidate {
                    room_display_name: "Private Room Name".to_owned(),
                    kind: RoomAttentionKind::Mention,
                    unread_count: 4,
                    highlight_count: 1,
                }),
                capabilities: NativeAttentionCapabilities {
                    notifications: NativeAttentionCapability::Available,
                    badge: NativeAttentionCapability::Available,
                    overlay_icon: NativeAttentionCapability::Unknown,
                    sound: NativeAttentionCapability::Unknown,
                    tray: NativeAttentionCapability::Unavailable,
                    activation: NativeAttentionCapability::Unknown,
                },
            },
            dispatch: NativeAttentionDispatchState::Suppressed {
                reason: NativeAttentionSuppressionReason::WindowFocused,
            },
        },
    };

    let debug = format!("{command:?}");

    assert!(debug.contains("UpdateNativeAttentionState"), "{debug}");
    assert!(debug.contains("unread_count"), "{debug}");
    assert!(debug.contains("suppressed"), "{debug}");
    assert!(!debug.contains("Private Room Name"), "{debug}");
}

#[test]
fn observe_native_window_focus_command_is_correlated_and_private_safe() {
    let request_id = fake_rid(28);
    let command = AppCommand::ObserveNativeWindowFocus {
        request_id,
        focused: false,
        observation_generation: 7,
    };

    assert_eq!(
        CoreCommand::App(command).request_id(),
        request_id,
        "focus observation must preserve command correlation"
    );
    let debug = format!(
        "{:?}",
        AppCommand::ObserveNativeWindowFocus {
            request_id,
            focused: false,
            observation_generation: 7,
        }
    );
    assert!(debug.contains("ObserveNativeWindowFocus"), "{debug}");
    assert!(debug.contains("focused: false"), "{debug}");
    assert!(debug.contains("observation_generation: 7"), "{debug}");
    assert!(!debug.contains("room_id"), "{debug}");
    assert!(!debug.contains("event_id"), "{debug}");
    assert!(!debug.contains("user_id"), "{debug}");
}
