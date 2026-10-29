use super::*;
use crate::commands::contracts::fake_request_id;
use koushi_protocol::{AccountKey, CoreCommand, PaginationDirection, TimelineCommand};
use koushi_state::ComposerDocument;

#[test]
fn build_subscribe_focused_timeline_command_routes_to_focused_timeline_kind() {
    let account_key = AccountKey("@alice:example.org".to_owned());
    let command = build_subscribe_focused_timeline_command(
        fake_request_id(21),
        account_key.clone(),
        "!room:example.org".to_owned(),
        "$event".to_owned(),
    );

    match command {
        CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id,
            key,
            initial_backfill,
        }) => {
            assert_eq!(request_id, fake_request_id(21));
            assert_eq!(key.account_key, account_key);
            assert_eq!(
                initial_backfill,
                koushi_protocol::command::InitialBackfillPolicy::Disabled
            );
            assert_eq!(
                key.kind,
                koushi_protocol::TimelineKind::Focused {
                    room_id: "!room:example.org".to_owned(),
                    event_id: "$event".to_owned(),
                }
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn hide_link_preview_tauri_command_contract_is_present() {
    let request_id = koushi_protocol::RequestId {
        connection_id: koushi_protocol::RuntimeConnectionId(1),
        sequence: 1,
    };
    let command = build_hide_link_preview_command(
        request_id,
        AccountKey("@u:example.test".to_owned()),
        "!room:example.test".to_owned(),
        "$event:example.test".to_owned(),
    );
    assert!(matches!(
        command,
        Some(CoreCommand::Timeline(
            TimelineCommand::HideLinkPreview { .. }
        ))
    ));
}

#[test]
fn load_link_previews_tauri_command_contract_is_present() {
    let request_id = koushi_protocol::RequestId {
        connection_id: koushi_protocol::RuntimeConnectionId(1),
        sequence: 1,
    };
    let command = build_load_link_previews_command(
        request_id,
        AccountKey("@u:example.test".to_owned()),
        "!room:example.test".to_owned(),
        "$event:example.test".to_owned(),
    );
    assert!(matches!(
        command,
        Some(CoreCommand::Timeline(
            TimelineCommand::LoadLinkPreviews { .. }
        ))
    ));
}

#[test]
fn tauri_command_routes_blank_message_bodies_return_no_command() {
    let account_key = AccountKey("@alice:example.org".to_owned());
    let room_id = "!room:example.org".to_owned();

    assert!(
        build_send_text_command(
            fake_request_id(14),
            account_key.clone(),
            room_id.clone(),
            "desktop-14".to_owned(),
            ComposerDocument::from_plain_text("   "),
        )
        .is_none()
    );
    assert!(
        build_edit_message_command(
            fake_request_id(15),
            account_key,
            room_id,
            "$event".to_owned(),
            ComposerDocument::from_plain_text("\n\t "),
        )
        .is_none()
    );
    assert!(
        build_download_media_command(
            fake_request_id(18),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            "\n\t ".to_owned(),
        )
        .is_none()
    );
    assert!(
        build_send_thread_reply_command(
            fake_request_id(16),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            "$root".to_owned(),
            "desktop-16".to_owned(),
            ComposerDocument::from_plain_text("\n\t "),
        )
        .is_none()
    );
}

#[test]
fn thread_timeline_backwards_pagination_builder_targets_thread_key() {
    let account_key = AccountKey("@alice:example.org".to_owned());
    let room_id = "!room:example.org".to_owned();
    let root_event_id = "$thread-root".to_owned();

    match build_paginate_thread_timeline_backwards_command(
        fake_request_id(22),
        account_key.clone(),
        room_id.clone(),
        root_event_id.clone(),
    ) {
        CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key,
            direction,
            event_count,
        }) => {
            assert_eq!(request_id, fake_request_id(22));
            assert_eq!(key.account_key, account_key);
            assert_eq!(
                key.kind,
                koushi_protocol::TimelineKind::Thread {
                    room_id,
                    root_event_id,
                }
            );
            assert_eq!(direction, PaginationDirection::Backward);
            assert_eq!(event_count, 100);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn focused_timeline_pagination_builder_targets_focused_key_in_both_directions() {
    let account_key = AccountKey("@alice:example.org".to_owned());
    let kind = koushi_protocol::TimelineKind::Focused {
        room_id: "!room:example.org".to_owned(),
        event_id: "$anchor".to_owned(),
    };

    for expected_direction in [PaginationDirection::Forward, PaginationDirection::Backward] {
        match build_paginate_timeline_command(
            fake_request_id(23),
            account_key.clone(),
            kind.clone(),
            expected_direction,
        ) {
            CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(key.account_key, account_key);
                assert_eq!(key.kind, kind);
                assert_eq!(direction, expected_direction);
                assert_eq!(event_count, 100);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
