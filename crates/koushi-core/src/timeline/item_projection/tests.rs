use super::super::test_source::item_body;

use std::collections::{BTreeSet, HashSet};

use koushi_state::{
    ComposerDocument, ComposerInline, LiveEventReceipts, LiveReadReceipt, MentionIntent,
    MentionTarget, ReplyQuote, ReplyQuoteState,
};

use matrix_sdk::room::edit::EditedContent;

use matrix_sdk::ruma::events::room::message::{MessageType, TextMessageEventContent};

use matrix_sdk::ruma::events::{
    Mentions, StateEventContentChange, room::name::RoomNameEventContent,
};

use matrix_sdk_ui::timeline::{MembershipChange, ReactionStatus, ReactionsByKeyBySender};

use crate::event_projection::message_actions_for_timeline_item;
use koushi_protocol::command::TimelineCommand;

#[test]
fn live_receipt_summary_compacts_large_reader_input_with_exact_total() {
    let receipts = (0..1_500)
        .map(|index| LiveReadReceipt {
            user_id: format!("@reader-{index}:example.invalid"),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(index),
        })
        .collect();
    let summaries = super::compact_live_receipt_summaries(
        vec![LiveEventReceipts {
            event_id: "$event:example.invalid".into(),
            receipts,
        }],
        None,
    );

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].readers.len(), 3);
    assert_eq!(summaries[0].total_count, 1_500);
}
use koushi_protocol::event::{
    LinkPreview, LinkPreviewState, TimelineFormattedBody, TimelineItemId, TimelineMessageKind,
    TimelineNoticeI18n, TimelineNoticeI18nKey, TimelineSendFailureReason, TimelineSendState,
    TimelineSpoilerSpan, TimelineViewportObservation,
};

use koushi_protocol::failure::TimelineFailureKind;

use koushi_diagnostics::DiagnosticValue;

use matrix_sdk::ruma::events::room::message::{
    EmoteMessageEventContent, NoticeMessageEventContent,
};
use matrix_sdk::ruma::{OwnedUserId, uint};
use matrix_sdk_ui::timeline::ReactionInfo;

use super::super::diagnostics::timeline_item_diagnostic_event;
use super::{
    apply_ignored_sender_suppression, composer_document_from_event_json,
    edited_content_for_edit_target, edited_document_content_for_edit_target,
    has_user_visible_content, link_ranges_for_message_projection,
    megolm_message_index_from_original_json, membership_change_projection,
    message_edit_target_token, message_projection_from_msgtype, msgtype_carries_editable_caption,
    project_local_megolm_rotation_reason, reaction_groups_from_sdk,
    reply_quote_from_message_projection, reset_loading_link_previews_to_pending,
    room_name_notice_projection, state_event_notice_body, state_event_notice_projection,
    timeline_item_can_edit, timeline_item_can_react, timeline_item_can_redact,
    timeline_item_should_be_hidden, validate_cancel_send, validate_redact_reaction,
    validate_retry_send, validate_send_reaction, visible_missing_reply_detail_event_ids,
};

use super::super::test_support::{fake_rid, room_key, timeline_item};

#[test]
fn extracts_megolm_message_index_from_encrypted_event_source() {
    let mut session =
        vodozemac::megolm::GroupSession::new(vodozemac::megolm::SessionConfig::version_1());
    session.encrypt("index zero");
    session.encrypt("index one");
    let ciphertext = session.encrypt("index two").to_base64();
    let event = serde_json::json!({
            "type": "m.room.encrypted",
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": ciphertext,
        }
    });

    assert_eq!(megolm_message_index_from_original_json(&event), Some(2));
}

#[test]
fn omits_megolm_message_index_for_unencrypted_or_invalid_source() {
    let unencrypted = serde_json::json!({
            "type": "m.room.message",
            "content": { "body": "hello", "msgtype": "m.text" }
    });
    let invalid = serde_json::json!({
            "type": "m.room.encrypted",
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "not-megolm",
        }
    });

    assert_eq!(megolm_message_index_from_original_json(&unencrypted), None);
    assert_eq!(megolm_message_index_from_original_json(&invalid), None);
}

#[test]
fn ignored_sender_suppression_preserves_divider_and_restores_event() {
    let mut divider = timeline_item("$divider:test", None, "@ignored:test", false);
    divider.id = TimelineItemId::Synthetic {
        synthetic_id: "date-divider:test".to_owned(),
    };
    let mut event = timeline_item("$ignored:test", Some("body"), "@ignored:test", false);
    let ignored = BTreeSet::from(["@ignored:test".to_owned()]);

    apply_ignored_sender_suppression(&mut divider, &ignored);
    apply_ignored_sender_suppression(&mut event, &ignored);
    assert!(
        !divider.is_hidden,
        "ignoring a sender must not hide a date divider"
    );
    assert!(event.is_hidden);

    apply_ignored_sender_suppression(&mut divider, &BTreeSet::new());
    apply_ignored_sender_suppression(&mut event, &BTreeSet::new());
    assert!(
        !divider.is_hidden,
        "unignore must leave the date divider visible"
    );
    assert!(!event.is_hidden, "unignore must restore eligible content");
}

#[test]
fn formatted_only_content_is_renderable_for_shared_eligibility() {
    let mut item = timeline_item("$formatted:test", None, "@sender:test", false);
    item.formatted = Some(TimelineFormattedBody {
        html: "<b>formatted</b>".to_owned(),
        plain_text: "formatted".to_owned(),
        code_blocks: Vec::new(),
    });

    assert!(has_user_visible_content(&item));
}

#[test]
fn local_megolm_reason_is_exact_and_missing_evidence_is_unavailable() {
    use koushi_protocol::event::TimelineMegolmSessionReason as Projected;
    use koushi_sdk::MatrixRoomKeyRotationReason as Sdk;

    assert_eq!(
        project_local_megolm_rotation_reason(false, Some(Sdk::ExpiredTime)),
        None
    );
    assert_eq!(
        project_local_megolm_rotation_reason(true, None),
        Some(Projected::NotRetained)
    );
    for (sdk, expected) in [
        (Sdk::Initial, Projected::Initial),
        (Sdk::ExpiredTime, Projected::ExpiredTime),
        (Sdk::ExpiredMessageCount, Projected::ExpiredMessageCount),
        (
            Sdk::MembershipOrDeviceChange,
            Projected::MembershipOrDeviceChange,
        ),
        (
            Sdk::EncryptionSettingsChanged,
            Projected::EncryptionSettingsChanged,
        ),
        (Sdk::ExplicitDiscard, Projected::ExplicitDiscard),
        (Sdk::FullMemberListReload, Projected::FullMemberListReload),
        (Sdk::RoomSubscription, Projected::RoomSubscription),
        (Sdk::LimitedSyncResponse, Projected::LimitedSyncResponse),
        (Sdk::KeyShareFailure, Projected::KeyShareFailure),
        (Sdk::StoreMissing, Projected::StoreMissing),
        (Sdk::Invalidated, Projected::Invalidated),
        (Sdk::Unknown, Projected::Unknown),
    ] {
        assert_eq!(
            project_local_megolm_rotation_reason(true, Some(sdk)),
            Some(expected)
        );
    }
}

#[test]
fn visible_missing_reply_detail_event_ids_only_returns_visible_unrequested_missing_replies() {
    let mut before = timeline_item("$before:test", Some("before"), "@alice:test", false);
    before.reply_quote = Some(ReplyQuote {
        event_id: "$root-before:test".to_owned(),
        sender: None,
        sender_label: None,
        body_preview: None,
        formatted: None,
        state: ReplyQuoteState::Missing,
    });
    let first_visible = timeline_item("$first-visible:test", Some("first"), "@alice:test", false);
    let mut missing = timeline_item("$missing:test", Some("missing"), "@alice:test", false);
    missing.reply_quote = Some(ReplyQuote {
        event_id: "$root-missing:test".to_owned(),
        sender: None,
        sender_label: None,
        body_preview: None,
        formatted: None,
        state: ReplyQuoteState::Missing,
    });
    let mut ready = timeline_item("$ready:test", Some("ready"), "@alice:test", false);
    ready.reply_quote = Some(ReplyQuote {
        event_id: "$root-ready:test".to_owned(),
        sender: Some("@bob:test".to_owned()),
        sender_label: None,
        body_preview: Some("loaded".to_owned()),
        formatted: None,
        state: ReplyQuoteState::Ready,
    });
    let mut already_requested = timeline_item(
        "$already-requested:test",
        Some("already"),
        "@alice:test",
        false,
    );
    already_requested.reply_quote = Some(ReplyQuote {
        event_id: "$root-already:test".to_owned(),
        sender: None,
        sender_label: None,
        body_preview: None,
        formatted: None,
        state: ReplyQuoteState::Missing,
    });
    let mut after = timeline_item("$after:test", Some("after"), "@alice:test", false);
    after.reply_quote = Some(ReplyQuote {
        event_id: "$root-after:test".to_owned(),
        sender: None,
        sender_label: None,
        body_preview: None,
        formatted: None,
        state: ReplyQuoteState::Missing,
    });

    let items = vec![
        before,
        first_visible,
        missing,
        ready,
        already_requested,
        after,
    ];
    let requested = HashSet::from(["$already-requested:test".to_owned()]);

    let event_ids = visible_missing_reply_detail_event_ids(
        &items,
        &TimelineViewportObservation {
            first_visible_event_id: Some("$first-visible:test".to_owned()),
            last_visible_event_id: Some("$already-requested:test".to_owned()),
            visible_gap_ids: Vec::new(),
            at_bottom: false,
        },
        &requested,
    );

    assert_eq!(event_ids, vec!["$missing:test".to_owned()]);
}

fn reaction_groups_fixture() -> ReactionsByKeyBySender {
    let mut reactions = ReactionsByKeyBySender::default();
    let thumbs = reactions.entry("👍".to_owned()).or_default();
    thumbs.insert(
        OwnedUserId::try_from("@me:test").expect("user id"),
        ReactionInfo {
            timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(1)),
            status: ReactionStatus::RemoteToRemote(
                matrix_sdk::ruma::OwnedEventId::try_from("$reaction:me").expect("event id"),
            ),
        },
    );
    thumbs.insert(
        OwnedUserId::try_from("@alice:test").expect("user id"),
        ReactionInfo {
            timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(2)),
            status: ReactionStatus::LocalToRemote(None),
        },
    );
    thumbs.insert(
        OwnedUserId::try_from("@bob:test").expect("user id"),
        ReactionInfo {
            timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(3)),
            status: ReactionStatus::LocalToRemote(None),
        },
    );
    thumbs.insert(
        OwnedUserId::try_from("@carol:test").expect("user id"),
        ReactionInfo {
            timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(4)),
            status: ReactionStatus::LocalToRemote(None),
        },
    );

    reactions
}

#[test]
fn editable_document_uses_formatted_links_for_duplicate_mention_identity() {
    let document = composer_document_from_event_json(&serde_json::json!({
                "content": {
                    "body": "**hello** @Same @Same typed @Same",
                    "format": "org.matrix.custom.html",
                    "formatted_body": "<strong>hello</strong> <a href=\"https://matrix.to/#/%40alice%3Aexample.test\">@Same</a> <a href=\"https://matrix.to/#/%40bob%3Aexample.test\">@Same</a> typed @Same",
                    "m.mentions": {
                        "user_ids": ["@alice:example.test", "@bob:example.test"]
                }
            }
        }))
        .expect("structured document");

    assert_eq!(document.plain_body(), "**hello** @Same @Same typed @Same");
    assert_eq!(
        document.mention_intent().user_ids(),
        vec!["@alice:example.test", "@bob:example.test"]
    );
    assert!(
        matches!(document.inlines.last(), Some(ComposerInline::Text { text }) if text.ends_with("typed @Same"))
    );
}

#[test]
fn editable_document_keeps_room_link_identity_without_user_mentions_metadata() {
    let document = composer_document_from_event_json(&serde_json::json!({
                "content": {
                    "body": "visit **@Project**",
                    "format": "org.matrix.custom.html",
                    "formatted_body": "visit <strong><a href=\"https://matrix.to/#/%23project%3Aexample.test\">@Project</a></strong>"
            }
        }))
        .expect("structured room mention");

    assert_eq!(document.plain_body(), "visit **@Project**");
    assert!(matches!(
        &document.inlines[1],
        ComposerInline::Mention {
            target: MentionTarget::Room { room_id, .. },
            ..
        } if room_id == "#project:example.test"
    ));
}

#[test]
fn editable_document_rejects_unsafe_links_even_when_labels_match() {
    let document = composer_document_from_event_json(&serde_json::json!({
            "content": {
                "body": "@Same",
                "format": "org.matrix.custom.html",
                "formatted_body": "<a href=\"http://matrix.to/#/%40alice%3Aexample.test\">@Same</a>",
                "m.mentions": { "user_ids": ["@alice:example.test"] }
        }
    }));

    assert!(document.is_none());
}

#[test]
fn message_projection_carries_msgtype_and_plain_spoiler_spans() {
    let projection = message_projection_from_msgtype(
        &MessageType::Notice(NoticeMessageEventContent::plain("keep ||secret|| hidden")),
        "keep ||secret|| hidden",
    );

    assert_eq!(projection.message_kind, TimelineMessageKind::Notice);
    assert_eq!(projection.body.as_deref(), Some("keep secret hidden"));
    assert_eq!(
        projection.spoiler_spans,
        vec![TimelineSpoilerSpan {
            start_utf16: 5,
            end_utf16: 11,
            reason: None,
        }]
    );
}

#[test]
fn membership_change_projection_is_a_supported_notice() {
    let projection =
        membership_change_projection("Alice", Some(MembershipChange::InvitationAccepted));

    assert_eq!(projection.message_kind, TimelineMessageKind::Notice);
    assert_eq!(projection.body.as_deref(), Some("Alice joined the room"));
    assert_eq!(projection.body_is_user_content, false);
    assert!(
        !projection
            .body
            .as_deref()
            .unwrap_or_default()
            .contains("Unsupported event: m.room.member")
    );
}

#[test]
fn pinned_events_projection_is_a_supported_notice() {
    assert_eq!(
        state_event_notice_body("m.room.pinned_events").as_ref(),
        "updated pinned messages"
    );
    assert_eq!(
        state_event_notice_body("m.room.create").as_ref(),
        "created the room"
    );
    assert_eq!(
        state_event_notice_body("m.room.power_levels").as_ref(),
        "updated room permissions"
    );
    assert_eq!(
        state_event_notice_body("m.room.guest_access").as_ref(),
        "updated guest access"
    );
    assert_eq!(
        state_event_notice_body("m.room.encryption").as_ref(),
        "enabled room encryption"
    );
    assert_eq!(
        state_event_notice_body("m.space.parent").as_ref(),
        "updated the parent space"
    );
    assert_eq!(
        state_event_notice_body("m.room.join_rules").as_ref(),
        "updated join rules"
    );
    assert_eq!(
        state_event_notice_body("m.room.history_visibility").as_ref(),
        "updated history visibility"
    );
    assert_eq!(
        state_event_notice_body("m.room.topic").as_ref(),
        "Unsupported event: m.room.topic"
    );
}

#[test]
fn supported_state_event_notices_carry_i18n_keys() {
    let projection = state_event_notice_projection("m.room.power_levels");

    assert_eq!(projection.body.as_deref(), Some("updated room permissions"));
    assert_eq!(
        projection.notice_i18n,
        Some(TimelineNoticeI18n {
            key: TimelineNoticeI18nKey::RoomPowerLevels,
            old_name: None,
            new_name: None,
        })
    );
    assert_eq!(projection.body_is_user_content, false);
}

fn original_room_name_change(
    name: &str,
    previous_name: Option<&str>,
) -> StateEventContentChange<RoomNameEventContent> {
    StateEventContentChange::Original {
        content: RoomNameEventContent::new(name.to_owned()),
        prev_content: previous_name.map(|previous_name| {
            serde_json::from_value(serde_json::json!({ "name": previous_name }))
                .expect("previous room name should deserialize")
        }),
    }
}

#[test]
fn room_name_notice_projects_initial_name_as_structured_set_notice() {
    let projection = room_name_notice_projection(&original_room_name_change("研究室 🧪", None));

    assert_eq!(
        projection.body.as_deref(),
        Some("set the room name to 研究室 🧪")
    );
    assert_eq!(
        projection.notice_i18n,
        Some(TimelineNoticeI18n {
            key: TimelineNoticeI18nKey::RoomNameSet,
            old_name: None,
            new_name: Some("研究室 🧪".to_owned()),
        })
    );
    assert_eq!(projection.message_kind, TimelineMessageKind::Notice);
    assert!(!projection.body_is_user_content);
}

#[test]
fn room_name_notice_projects_old_and_new_names_for_change() {
    let projection =
        room_name_notice_projection(&original_room_name_change("<新しい部屋>", Some("Old room")));

    assert_eq!(
        projection.body.as_deref(),
        Some("changed the room name from Old room to <新しい部屋>")
    );
    assert_eq!(
        projection.notice_i18n,
        Some(TimelineNoticeI18n {
            key: TimelineNoticeI18nKey::RoomNameChanged,
            old_name: Some("Old room".to_owned()),
            new_name: Some("<新しい部屋>".to_owned()),
        })
    );
}

#[test]
fn room_name_notice_projects_empty_name_as_removal() {
    let projection =
        room_name_notice_projection(&original_room_name_change("   ", Some("Old room")));

    assert_eq!(projection.body.as_deref(), Some("removed the room name"));
    assert_eq!(
        projection.notice_i18n,
        Some(TimelineNoticeI18n {
            key: TimelineNoticeI18nKey::RoomNameRemoved,
            old_name: None,
            new_name: None,
        })
    );
}

#[test]
fn room_name_notice_uses_set_wording_for_identical_names() {
    let projection =
        room_name_notice_projection(&original_room_name_change("Same room", Some("Same room")));

    assert_eq!(
        projection.notice_i18n.as_ref().map(|notice| notice.key),
        Some(TimelineNoticeI18nKey::RoomNameSet)
    );
}

#[test]
fn room_name_notice_projects_redacted_content_as_safe_generic_notice() {
    let redacted = StateEventContentChange::Redacted(
        serde_json::from_value(serde_json::json!({}))
            .expect("redacted room name should deserialize"),
    );
    let projection = room_name_notice_projection(&redacted);

    assert_eq!(projection.body.as_deref(), Some("changed the room name"));
    assert_eq!(
        projection.notice_i18n,
        Some(TimelineNoticeI18n {
            key: TimelineNoticeI18nKey::RoomNameChangedGeneric,
            old_name: None,
            new_name: None,
        })
    );
    assert!(!projection.body.unwrap_or_default().contains("m.room.name"));
}

#[test]
fn message_projection_extracts_formatted_spoiler_spans_with_reason() {
    let msgtype = MessageType::Emote(EmoteMessageEventContent::html(
        "plain fallback",
        r#"keep <span data-mx-spoiler="because">secret</span> hidden"#,
    ));

    let projection = message_projection_from_msgtype(&msgtype, "plain fallback");

    assert_eq!(projection.message_kind, TimelineMessageKind::Emote);
    assert_eq!(
        projection.spoiler_spans,
        vec![TimelineSpoilerSpan {
            start_utf16: 5,
            end_utf16: 11,
            reason: Some("because".to_owned()),
        }]
    );
}

#[test]
fn message_projection_sanitizes_formatted_html_and_extracts_code_blocks() {
    let msgtype = MessageType::Text(TextMessageEventContent::html(
        "plain fallback",
        r#"<strong>ok</strong><script>alert(1)</script><a href="javascript:alert(1)">bad</a><a href="https://example.invalid/path">safe</a><pre><code class="language-rust ignored">fn main() {}</code></pre>"#,
    ));

    let projection = message_projection_from_msgtype(&msgtype, "plain fallback");
    let formatted = projection
        .formatted
        .expect("html formatted_body should project to a Rust-owned render model");

    assert!(formatted.html.contains("<strong>ok</strong>"));
    assert!(!formatted.html.contains("<script"));
    assert!(!formatted.html.contains("alert(1)"));
    assert!(!formatted.html.contains("javascript:"));
    assert!(formatted.html.contains("https://example.invalid/path"));
    assert_eq!(formatted.plain_text, "okbadsafefn main() {}");
    assert_eq!(formatted.code_blocks.len(), 1);
    assert_eq!(formatted.code_blocks[0].language.as_deref(), Some("rust"));
    assert_eq!(formatted.code_blocks[0].body, "fn main() {}");
}

#[test]
fn formatted_message_link_ranges_use_formatted_plain_text_basis() {
    let msgtype = MessageType::Text(TextMessageEventContent::html(
        "fallback without url",
        r#"<strong>Visit https://example.invalid/path</strong>"#,
    ));

    let projection = message_projection_from_msgtype(&msgtype, "fallback without url");
    let ranges = link_ranges_for_message_projection(
        projection.body.as_deref(),
        projection.formatted.as_ref(),
    );

    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].url, "https://example.invalid/path");
    assert_eq!(ranges[0].start_utf16, "Visit ".encode_utf16().count());
    assert_eq!(
        ranges[0].end_utf16,
        "Visit https://example.invalid/path".encode_utf16().count()
    );
}

#[test]
fn message_projection_keeps_allowed_formatted_blocks_and_spoilers() {
    let msgtype = MessageType::Emote(EmoteMessageEventContent::html(
        "plain fallback",
        r#"<blockquote>quote</blockquote><ul><li>one</li></ul><span data-mx-spoiler="reason">secret</span>"#,
    ));

    let projection = message_projection_from_msgtype(&msgtype, "plain fallback");
    let formatted = projection
        .formatted
        .expect("allowed formatted_body should project to a render model");

    assert!(formatted.html.contains("<blockquote>quote</blockquote>"));
    assert!(formatted.html.contains("<ul><li>one</li></ul>"));
    assert!(formatted.html.contains("data-mx-spoiler=\"reason\""));
    assert!(formatted.html.contains(">secret<"));
}

#[test]
fn reply_quote_projection_retains_sanitized_formatted_body() {
    let msgtype = MessageType::Text(TextMessageEventContent::html(
        "plain fallback",
        r#"<ul><li>one</li><li>two</li></ul><script>bad()</script><pre><code class="language-rust">fn main() {}</code></pre>"#,
    ));

    let projection = message_projection_from_msgtype(&msgtype, "plain fallback");
    let quote = reply_quote_from_message_projection(
        "$root:example.invalid",
        Some("@bob:example.invalid".to_owned()),
        Some(projection),
    );

    assert_eq!(quote.state, ReplyQuoteState::Ready);
    let formatted = quote.formatted.expect("formatted quote body");
    assert!(formatted.html.contains("<ul><li>one</li><li>two</li></ul>"));
    assert!(!formatted.html.contains("<script"));
    assert_eq!(formatted.code_blocks.len(), 1);
    assert_eq!(formatted.code_blocks[0].language.as_deref(), Some("rust"));
    assert_eq!(formatted.code_blocks[0].body, "fn main() {}");
}

#[test]
fn captionless_media_projections_can_reply_and_keep_filename_reply_quotes() {
    for msgtype in media_msgtype_fixtures() {
        let projection = message_projection_from_msgtype(&msgtype, "ignored fallback body");
        let filename = projection
            .media
            .as_ref()
            .expect("media fixture projects media")
            .filename
            .clone();

        assert!(
            projection.body.is_none(),
            "media fixture must be captionless"
        );

        let actions = message_actions_for_timeline_item(
            "!room:test",
            &TimelineItemId::Event {
                event_id: "$captionless-media:test".to_owned(),
            },
            projection.body.as_deref(),
            projection.media.is_some(),
            false,
        );
        assert!(actions.can_reply, "{filename} should be replyable");

        let quote =
            reply_quote_from_message_projection("$captionless-media:test", None, Some(projection));
        assert_eq!(quote.body_preview.as_deref(), Some(filename.as_str()));
    }
}

#[test]
fn message_projection_falls_back_to_plain_body_when_formatted_body_is_empty() {
    let msgtype = MessageType::Text(TextMessageEventContent::html("plain fallback", "   "));

    let projection = message_projection_from_msgtype(&msgtype, "plain fallback");

    assert_eq!(projection.body.as_deref(), Some("plain fallback"));
    assert!(projection.formatted.is_none());
}

#[test]
fn message_projection_falls_back_to_plain_body_when_formatted_body_has_only_markup() {
    let msgtype = MessageType::Text(TextMessageEventContent::html(
        "plain fallback",
        "<p><br /></p>",
    ));

    let projection = message_projection_from_msgtype(&msgtype, "plain fallback");

    assert_eq!(projection.body.as_deref(), Some("plain fallback"));
    assert!(projection.formatted.is_none());
}

#[test]
fn user_visible_content_includes_formatted_body() {
    let mut item = timeline_item("$formatted:test", None, "@alice:test", false);
    item.formatted = Some(koushi_protocol::event::TimelineFormattedBody {
        html: "<strong>visible</strong>".to_owned(),
        plain_text: "visible".to_owned(),
        code_blocks: Vec::new(),
    });

    assert!(has_user_visible_content(&item));
}

#[test]
fn bodyless_event_backed_items_are_hidden_unless_redacted() {
    assert!(timeline_item_should_be_hidden(false, false));
    assert!(!timeline_item_should_be_hidden(true, false));
    assert!(!timeline_item_should_be_hidden(false, true));
}

#[test]
fn timeline_item_structured_fields_match_private_legacy_semantics() {
    let key = room_key();
    let mut item = timeline_item("$private-event:test", Some("   "), "   ", true);
    item.timestamp_ms = Some(1_783_076_820_000);
    item.thread_root = Some("   ".to_owned());
    item.in_reply_to_event_id = Some("   ".to_owned());
    item.formatted = Some(koushi_protocol::event::TimelineFormattedBody {
        html: "<br>".to_owned(),
        plain_text: "   ".to_owned(),
        code_blocks: Vec::new(),
    });

    let event = timeline_item_diagnostic_event("initial", &key, "item", Some(7), &item);

    assert_eq!(
        event
            .fields
            .iter()
            .map(|field| (field.key, field.value.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("kind", DiagnosticValue::Token("item")),
            ("timeline", DiagnosticValue::Token("room")),
            ("id_kind", DiagnosticValue::Token("event")),
            ("count", DiagnosticValue::Count(1)),
            ("index", DiagnosticValue::Count(7)),
            ("index_present", DiagnosticValue::Boolean(true)),
            (
                "timestamp_minute",
                DiagnosticValue::Count(1_783_076_820_000 / 60_000),
            ),
            ("timestamp_present", DiagnosticValue::Boolean(true)),
            ("sender_present", DiagnosticValue::Boolean(false)),
            ("hidden", DiagnosticValue::Boolean(true)),
            ("thread_root_present", DiagnosticValue::Boolean(false)),
            ("reply_present", DiagnosticValue::Boolean(false)),
            ("body_present", DiagnosticValue::Boolean(false)),
            ("formatted_present", DiagnosticValue::Boolean(false)),
            ("media_present", DiagnosticValue::Boolean(false)),
            ("redacted", DiagnosticValue::Boolean(false)),
            ("unable_to_decrypt", DiagnosticValue::Boolean(false)),
            ("send_state_present", DiagnosticValue::Boolean(false)),
        ]
    );
    let serialized = serde_json::to_string(&event).expect("diagnostic event serializes");
    for private_value in ["$private-event:test", "!r:test"] {
        assert!(!serialized.contains(private_value));
    }
}

#[test]
fn send_operation_guards_allow_retry_and_cancel_only_from_outbound_states() {
    assert_eq!(
        validate_retry_send(Some(&TimelineSendState::NotSent {
            reason: TimelineSendFailureReason::Recoverable,
        })),
        Ok(())
    );
    assert_eq!(
        validate_retry_send(Some(&TimelineSendState::Sending)),
        Err(TimelineFailureKind::InvalidSendState)
    );
    assert_eq!(
        validate_retry_send(Some(&TimelineSendState::Sent)),
        Err(TimelineFailureKind::InvalidSendState)
    );
    assert_eq!(
        validate_cancel_send(Some(&TimelineSendState::Sending)),
        Ok(())
    );
    assert_eq!(
        validate_cancel_send(Some(&TimelineSendState::NotSent {
            reason: TimelineSendFailureReason::Unrecoverable,
        })),
        Ok(())
    );
    assert_eq!(
        validate_cancel_send(Some(&TimelineSendState::Sent)),
        Err(TimelineFailureKind::InvalidSendState)
    );
    assert_eq!(
        validate_cancel_send(None),
        Err(TimelineFailureKind::InvalidSendTarget)
    );
}

#[test]
fn reaction_groups_project_my_sender_and_remote_event_id() {
    let own_user_id = OwnedUserId::try_from("@me:test").expect("user id");
    let groups = reaction_groups_from_sdk(&reaction_groups_fixture(), Some(own_user_id.as_ref()));

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].key, "👍");
    assert_eq!(groups[0].count, 4);
    assert!(groups[0].reacted_by_me);
    assert_eq!(
        groups[0].my_reaction_event_id.as_deref(),
        Some("$reaction:me")
    );
    assert_eq!(
        groups[0]
            .sender_preview
            .iter()
            .map(|sender| sender.user_id.as_str())
            .collect::<Vec<_>>(),
        vec!["@me:test", "@alice:test", "@bob:test"]
    );
}

#[test]
fn reaction_groups_count_unique_senders_after_sdk_deduplication() {
    let mut reactions = ReactionsByKeyBySender::default();
    let thumbs = reactions.entry("👍".to_owned()).or_default();
    let alice = OwnedUserId::try_from("@alice:test").expect("user id");
    thumbs.insert(
        alice.clone(),
        ReactionInfo {
            timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(1)),
            status: ReactionStatus::RemoteToRemote(
                matrix_sdk::ruma::OwnedEventId::try_from("$reaction:old").expect("event id"),
            ),
        },
    );
    thumbs.insert(
        alice,
        ReactionInfo {
            timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(2)),
            status: ReactionStatus::RemoteToRemote(
                matrix_sdk::ruma::OwnedEventId::try_from("$reaction:new").expect("event id"),
            ),
        },
    );

    let groups = reaction_groups_from_sdk(&reactions, None);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].count, 1);
    assert_eq!(
        groups[0]
            .sender_preview
            .iter()
            .map(|sender| sender.user_id.as_str())
            .collect::<Vec<_>>(),
        vec!["@alice:test"]
    );
}

#[test]
fn reaction_groups_follow_sdk_redaction_removal() {
    let mut reactions = reaction_groups_fixture();
    reactions
        .get_mut("👍")
        .expect("thumbs reaction")
        .shift_remove(&OwnedUserId::try_from("@me:test").expect("user id"));
    let own_user_id = OwnedUserId::try_from("@me:test").expect("user id");

    let groups = reaction_groups_from_sdk(&reactions, Some(own_user_id.as_ref()));

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].count, 3);
    assert!(!groups[0].reacted_by_me);
    assert_eq!(groups[0].my_reaction_event_id, None);
}

#[test]
fn timeline_item_can_react_requires_event_backed_renderable_content() {
    assert!(timeline_item_can_react(true, true, false, true));
    assert!(!timeline_item_can_react(false, true, false, true));
    assert!(!timeline_item_can_react(true, false, false, true));
    assert!(!timeline_item_can_react(true, true, false, false));
    assert!(!timeline_item_can_react(true, true, true, true));
}

#[test]
fn send_reaction_guard_requires_reactable_target_without_existing_own_reaction() {
    assert_eq!(validate_send_reaction(true, None), Ok(()));
    assert_eq!(
        validate_send_reaction(false, None),
        Err(TimelineFailureKind::InvalidReactionTarget)
    );
    assert_eq!(
        validate_send_reaction(true, Some("$reaction:example.test")),
        Err(TimelineFailureKind::InvalidReactionState)
    );
}

#[test]
fn redact_reaction_guard_requires_matching_own_reaction_event() {
    assert_eq!(
        validate_redact_reaction(
            true,
            Some("$reaction:example.test"),
            "$reaction:example.test"
        ),
        Ok(())
    );
    assert_eq!(
        validate_redact_reaction(
            false,
            Some("$reaction:example.test"),
            "$reaction:example.test"
        ),
        Err(TimelineFailureKind::InvalidReactionTarget)
    );
    assert_eq!(
        validate_redact_reaction(true, None, "$reaction:example.test"),
        Err(TimelineFailureKind::InvalidReactionState)
    );
    assert_eq!(
        validate_redact_reaction(
            true,
            Some("$other-reaction:example.test"),
            "$reaction:example.test"
        ),
        Err(TimelineFailureKind::InvalidReactionState)
    );
}

#[test]
fn timeline_item_can_redact_requires_own_renderable_event_content() {
    assert!(timeline_item_can_redact(true, true, false, true));
    assert!(!timeline_item_can_redact(false, true, false, true));
    assert!(!timeline_item_can_redact(true, false, false, true));
    assert!(!timeline_item_can_redact(true, true, true, true));
    assert!(!timeline_item_can_redact(true, true, false, false));
}

#[test]
fn timeline_item_can_edit_requires_own_editable_body() {
    assert!(timeline_item_can_edit(true, true, false, true));
    assert!(!timeline_item_can_edit(false, true, false, true));
    assert!(!timeline_item_can_edit(true, false, false, true));
    assert!(!timeline_item_can_edit(true, true, true, true));
    assert!(!timeline_item_can_edit(true, true, false, false));
}

fn media_msgtype_fixtures() -> Vec<MessageType> {
    use matrix_sdk::ruma::events::room::message::{
        AudioMessageEventContent, FileMessageEventContent, ImageMessageEventContent,
        VideoMessageEventContent,
    };
    use matrix_sdk::ruma::owned_mxc_uri;

    vec![
        MessageType::Audio(AudioMessageEventContent::plain(
            "fixture-audio".to_owned(),
            owned_mxc_uri!("mxc://fixture.invalid/audio"),
        )),
        MessageType::File(FileMessageEventContent::plain(
            "fixture-file".to_owned(),
            owned_mxc_uri!("mxc://fixture.invalid/file"),
        )),
        MessageType::Image(ImageMessageEventContent::plain(
            "fixture-image".to_owned(),
            owned_mxc_uri!("mxc://fixture.invalid/image"),
        )),
        MessageType::Video(VideoMessageEventContent::plain(
            "fixture-video".to_owned(),
            owned_mxc_uri!("mxc://fixture.invalid/video"),
        )),
    ]
}

fn non_media_msgtype_fixtures() -> Vec<MessageType> {
    use matrix_sdk::ruma::events::room::message::{
        EmoteMessageEventContent, NoticeMessageEventContent, TextMessageEventContent,
    };

    vec![
        MessageType::Emote(EmoteMessageEventContent::plain("fixture-emote")),
        MessageType::Notice(NoticeMessageEventContent::plain("fixture-notice")),
        MessageType::Text(TextMessageEventContent::plain("fixture-text")),
    ]
}

#[test]
fn edit_replacement_preserves_media_attachment_as_caption() {
    // A text replacement carries no url/file/info/filename, so it silently
    // drops the attachment from the edited event (issue #328).
    for msgtype in media_msgtype_fixtures() {
        let target = message_edit_target_token(Some(&msgtype));
        match edited_content_for_edit_target(
            Some(&msgtype),
            "edited caption",
            &MentionIntent::default(),
        ) {
            EditedContent::MediaCaption {
                caption,
                formatted_caption,
                mentions,
            } => {
                assert_eq!(caption.as_deref(), Some("edited caption"));
                assert!(
                    formatted_caption.is_none(),
                    "{target}: plain caption edit must not add formatting"
                );
                assert!(
                    mentions.is_none(),
                    "{target}: plain caption edit must not add mentions"
                );
            }
            other => panic!("{target}: media edit must preserve the attachment, got {other:?}"),
        }
    }
}

#[test]
fn edit_replacement_preserves_non_media_message_kind() {
    for msgtype in non_media_msgtype_fixtures() {
        let target = message_edit_target_token(Some(&msgtype));
        match edited_content_for_edit_target(
            Some(&msgtype),
            "edited body",
            &MentionIntent::default(),
        ) {
            EditedContent::RoomMessage(content) => match &content.msgtype {
                MessageType::Text(text) => assert_eq!(text.body, "edited body"),
                MessageType::Notice(notice) => assert_eq!(notice.body, "edited body"),
                MessageType::Emote(emote) => assert_eq!(emote.body, "edited body"),
                other => {
                    panic!(
                        "{target}: non-media replacement expected, got {:?}",
                        other.msgtype()
                    )
                }
            },
            other => panic!("{target}: non-media replacement expected, got {other:?}"),
        }
    }
}

#[test]
fn edit_replacement_stays_plain_text_for_unresolved_target() {
    // A target missing from the timeline, or one that is not an
    // m.room.message, keeps the pre-existing text replacement instead of
    // guessing a caption edit the SDK would reject.
    assert!(matches!(
        edited_content_for_edit_target(None, "edited body", &MentionIntent::default()),
        EditedContent::RoomMessage(_)
    ));
}

#[test]
fn structured_edit_preserves_final_mentions_and_formats_text_and_media_captions() {
    let document = ComposerDocument::new(vec![
        ComposerInline::Text {
            text: "hello ".into(),
        },
        ComposerInline::Mention {
            target: MentionTarget::User {
                user_id: "@alice:example.org".into(),
                display_label: "Alice".into(),
            },
            display_label: "Alice".into(),
        },
    ]);
    let text = MessageType::Text(TextMessageEventContent::plain("old"));
    let EditedContent::RoomMessage(content) =
        edited_document_content_for_edit_target(Some(&text), &document)
    else {
        panic!("text edit must stay a room message")
    };
    let MessageType::Text(text) = content.msgtype else {
        panic!("text edit must stay text")
    };
    assert_eq!(text.body, "hello @Alice");
    assert_eq!(
        text.formatted.map(|formatted| formatted.body),
        Some("hello <a href=\"https://matrix.to/#/%40alice%3Aexample.org\">@Alice</a>".into())
    );
    assert_eq!(content.mentions.expect("final mentions").user_ids.len(), 1);

    let media = media_msgtype_fixtures().pop().expect("media fixture");
    let EditedContent::MediaCaption {
        caption,
        formatted_caption,
        mentions,
    } = edited_document_content_for_edit_target(Some(&media), &document)
    else {
        panic!("media edit must remain a caption edit")
    };
    assert_eq!(caption.as_deref(), Some("hello @Alice"));
    assert!(formatted_caption.is_some());
    assert_eq!(mentions.expect("caption mentions").user_ids.len(), 1);
}

#[test]
fn edit_replacement_carries_final_mentions_and_sdk_filters_revision_mentions() {
    use matrix_sdk::ruma::events::room::message::ReplacementMetadata;

    let alice = matrix_sdk::ruma::user_id!("@alice:example.org");
    let bob = matrix_sdk::ruma::user_id!("@bob:example.org");
    let mentions = MentionIntent {
        targets: vec![
            MentionTarget::User {
                user_id: alice.to_string(),
                display_label: "alice".to_owned(),
            },
            MentionTarget::User {
                user_id: bob.to_string(),
                display_label: "bob".to_owned(),
            },
        ],
    };
    let target = MessageType::Text(TextMessageEventContent::plain("old"));
    let edited = match edited_content_for_edit_target(Some(&target), "@alice @bob", &mentions) {
        EditedContent::RoomMessage(content) => content,
        other => panic!("text edit must remain a room message: {other:?}"),
    };
    let original_mentions = Mentions::with_user_ids([alice.to_owned()]);
    let replacement = edited.make_replacement(ReplacementMetadata::new(
        matrix_sdk::ruma::event_id!("$edit:example.org").to_owned(),
        Some(original_mentions),
    ));
    assert_eq!(
        replacement
            .mentions
            .expect("new mention notification set")
            .user_ids
            .into_iter()
            .collect::<Vec<_>>(),
        vec![bob.to_owned()]
    );
    let Some(matrix_sdk::ruma::events::room::message::Relation::Replacement(replacement)) =
        replacement.relates_to
    else {
        panic!("replacement relation must carry final mentions");
    };
    assert_eq!(
        replacement
            .new_content
            .mentions
            .expect("final mention set")
            .user_ids
            .into_iter()
            .collect::<Vec<_>>(),
        vec![alice.to_owned(), bob.to_owned()]
    );

    let removed = match edited_content_for_edit_target(
        Some(&target),
        "no mentions remain",
        &MentionIntent::default(),
    ) {
        EditedContent::RoomMessage(content) => content,
        other => panic!("removed mentions must stay a room message: {other:?}"),
    };
    assert!(removed.mentions.is_none());

    let media = media_msgtype_fixtures().pop().expect("media fixture");
    let media_edit = edited_content_for_edit_target(Some(&media), "@bob", &mentions);
    let EditedContent::MediaCaption {
        mentions: media_mentions,
        ..
    } = media_edit
    else {
        panic!("media edit must remain a caption edit");
    };
    assert_eq!(
        media_mentions
            .expect("media final mentions")
            .user_ids
            .into_iter()
            .collect::<Vec<_>>(),
        vec![alice.to_owned(), bob.to_owned()]
    );
}

#[test]
fn edit_replacement_caption_support_matches_media_projection() {
    // Both sides of this equality are load-bearing: a type projected with
    // TimelineItem.media but edited as text loses its attachment, and a type
    // edited as a caption without media support is rejected by the SDK as an
    // incompatible edit.
    for msgtype in media_msgtype_fixtures()
        .into_iter()
        .chain(non_media_msgtype_fixtures())
    {
        let target = message_edit_target_token(Some(&msgtype));
        let projects_media = message_projection_from_msgtype(&msgtype, "fixture-body")
            .media
            .is_some();
        assert_eq!(
            projects_media,
            msgtype_carries_editable_caption(&msgtype),
            "{target}: media projection and caption-edit support must agree"
        );
    }
}

#[test]
fn timeline_send_command_bodies_are_not_visible_in_debug() {
    // Manager-owned enqueue payloads originate from the public command, so
    // its Debug implementation is the privacy boundary for the send body.
    let cmd = TimelineCommand::SendText {
        request_id: fake_rid(1),
        key: room_key(),
        transaction_id: "txn-vis".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("very-private-body".to_owned()),
    };
    let debug = format!("{cmd:?}");
    assert!(
        !debug.contains("very-private-body"),
        "body leaked in Debug: {debug}"
    );
    assert!(
        debug.contains("txn-vis"),
        "txn_id should be visible: {debug}"
    );
}

#[test]
fn cancelled_link_preview_loads_return_loading_previews_to_pending() {
    let mut item = timeline_item(
        "$link:test",
        Some("https://example.test"),
        "@bob:test",
        false,
    );
    item.link_previews = Some(vec![
        LinkPreview {
            url: "https://example.test/loading".to_owned(),
            title: None,
            description: None,
            image: None,
            state: LinkPreviewState::Loading,
        },
        LinkPreview {
            url: "https://example.test/ready".to_owned(),
            title: Some("ready".to_owned()),
            description: None,
            image: None,
            state: LinkPreviewState::Ready,
        },
    ]);

    assert!(reset_loading_link_previews_to_pending(&mut item));
    let previews = item.link_previews.as_ref().expect("link previews");
    assert_eq!(previews[0].state, LinkPreviewState::Pending);
    assert_eq!(previews[1].state, LinkPreviewState::Ready);
    assert!(!reset_loading_link_previews_to_pending(&mut item));
}
