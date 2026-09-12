const FULL_RANGE_TOPOLOGY_REVISION: u64 = 14_695_981_039_346_656_037;

use super::*;
use koushi_protocol::event::*;
use koushi_protocol::ids::{
    AccountKey, RequestId, RuntimeConnectionId, TimelineGeneration, TimelineKey,
};
use koushi_state::*;
use serde_json::json;

fn fake_rid(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(7),
        sequence,
    }
}
fn timeline_item_fixture(event_id: &str, is_redacted: bool) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: if is_redacted {
            None
        } else {
            Some("visible body".to_owned())
        },
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1),
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: !is_redacted,
        is_redacted,
        is_hidden: false,
        can_redact: !is_redacted,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    }
}

#[test]
fn timeline_gap_id_wire_serializes_and_deserializes_full_range_revision() {
    let gap_id = TimelineGapId {
        topology_revision: FULL_RANGE_TOPOLOGY_REVISION,
        ordinal: 0,
    };

    let encoded = serde_json::to_value(gap_id).expect("timeline gap id serializes");
    assert_eq!(
        encoded,
        json!({
                "topology_revision": "14695981039346656037",
                "ordinal": 0,
        })
    );
    assert_eq!(
        serde_json::from_value::<TimelineGapId>(encoded)
            .expect("canonical decimal-string topology revision deserializes"),
        gap_id
    );
}
#[test]
fn timeline_gap_id_wire_rejects_noncanonical_revision_encodings() {
    for encoded in [
        r#"{"topology_revision":14695981039346656037,"ordinal":0}"#,
        r#"{"topology_revision":"+14695981039346656037","ordinal":0}"#,
        r#"{"topology_revision":" 14695981039346656037","ordinal":0}"#,
        r#"{"topology_revision":"014695981039346656037","ordinal":0}"#,
    ] {
        assert!(
            serde_json::from_str::<TimelineGapId>(encoded).is_err(),
            "noncanonical topology revision must be rejected: {encoded}"
        );
    }
}
#[test]
fn timeline_item_serializes_thread_fields_reactions_and_redaction_affordances() {
    let item = TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: "$event:test".to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("hello".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1_234),
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: Some("$root:test".to_owned()),
        thread_summary: Some(ThreadSummaryDto {
            reply_count: 2,
            latest_event_id: Some("$latest-reply:test".to_owned()),
            latest_sender: Some("@bob:example.invalid".to_owned()),
            latest_sender_label: None,
            latest_body_preview: Some("latest reply".to_owned()),
            latest_timestamp_ms: Some(1_456),
        }),
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: vec![ReactionGroup {
            key: "👍".to_owned(),
            count: 2,
            reacted_by_me: true,
            my_reaction_event_id: Some("$reaction:test".to_owned()),
            sender_preview: vec![ReactionSender {
                user_id: "@alice:example.invalid".to_owned(),
                display_label: Some("Alice".to_owned()),
            }],
        }],
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: true,
        is_edited: true,
        can_edit: true,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    };

    let value = serde_json::to_value(&item).expect("timeline item serializes");

    assert_eq!(
        value["reactions"],
        json!([
            {
                    "key": "👍",
                    "count": 2,
                    "reacted_by_me": true,
                    "my_reaction_event_id": "$reaction:test",
                    "sender_preview": [
                    {
                            "user_id": "@alice:example.invalid",
                            "display_label": "Alice"
                    }
                ]
            }
        ])
    );
    assert_eq!(value["can_react"], json!(true));
    assert_eq!(value["is_redacted"], json!(false));
    assert_eq!(value["can_redact"], json!(true));
    assert_eq!(value["is_edited"], json!(true));
    assert_eq!(value["can_edit"], json!(true));
    assert_eq!(value["thread_root"], json!("$root:test"));
    assert_eq!(
        value["thread_summary"],
        json!({
                "reply_count": 2,
                "latest_event_id": "$latest-reply:test",
                "latest_sender": "@bob:example.invalid",
                "latest_sender_label": null,
                "latest_body_preview": "latest reply",
                "latest_timestamp_ms": 1456
        })
    );
}
#[test]
fn timeline_item_serializes_reply_quote_without_debugging_body() {
    let item = TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: "$reply:test".to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("reply body".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1_234),
        in_reply_to_event_id: Some("$root:test".to_owned()),
        formatted: None,
        reply_quote: Some(koushi_state::ReplyQuote {
            event_id: "$root:test".to_owned(),
            sender: Some("@bob:example.invalid".to_owned()),
            sender_label: None,
            body_preview: Some("quoted body".to_owned()),
            formatted: Some(koushi_state::ReplyQuoteFormattedBody {
                html: "<p>quoted <strong>body</strong></p>".to_owned(),
                plain_text: "quoted body".to_owned(),
                code_blocks: Vec::new(),
            }),
            state: koushi_state::ReplyQuoteState::Ready,
        }),
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: true,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    };

    let value = serde_json::to_value(&item).expect("timeline item serializes");

    assert_eq!(
        value["reply_quote"],
        json!({
                "event_id": "$root:test",
                "sender": "@bob:example.invalid",
                "sender_label": null,
                "body_preview": "quoted body",
                "formatted": {
                    "html": "<p>quoted <strong>body</strong></p>",
                    "plain_text": "quoted body",
                    "code_blocks": []
            },
                "state": "ready"
        })
    );
    let debug = format!("{item:?}");
    assert!(debug.contains("reply_quote"));
    assert!(!debug.contains("quoted body"), "{debug}");
    assert!(!debug.contains("$root:test"), "{debug}");
}
#[test]
fn timeline_item_serializes_formatted_body_without_debugging_content() {
    let item = TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: "$formatted:test".to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("plain fallback".to_owned()),
        notice_i18n: None,
        message_kind: TimelineMessageKind::Emote,
        spoiler_spans: vec![TimelineSpoilerSpan {
            start_utf16: 0,
            end_utf16: 13,
            reason: Some("reason".to_owned()),
        }],
        timestamp_ms: Some(1_234),
        in_reply_to_event_id: None,
        formatted: Some(TimelineFormattedBody {
            html: "<strong>private html</strong><pre><code class=\"language-rust\">private_code()</code></pre>"
                .to_owned(),
            plain_text: "private htmlprivate_code()".to_owned(),
            code_blocks: vec![TimelineCodeBlock {
                language: Some("rust".to_owned()),
                body: "private_code()".to_owned(),
            }],
        }),
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: true,
        is_edited: false,
        can_edit: true,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    };

    let value = serde_json::to_value(&item).expect("timeline item serializes");

    assert_eq!(
        value["formatted"],
        json!({
                "html": "<strong>private html</strong><pre><code class=\"language-rust\">private_code()</code></pre>",
                "plain_text": "private htmlprivate_code()",
                "code_blocks": [
                {
                        "language": "rust",
                        "body": "private_code()"
                }
            ]
        })
    );
    assert_eq!(value["message_kind"], json!("emote"));
    assert_eq!(
        value["spoiler_spans"],
        json!([
            {
                    "start_utf16": 0,
                    "end_utf16": 13,
                    "reason": "reason"
            }
        ])
    );
    let debug = format!("{item:?}");
    assert!(debug.contains("TimelineFormattedBody"));
    assert!(!debug.contains("private html"), "{debug}");
    assert!(!debug.contains("private_code"), "{debug}");
    assert!(!debug.contains("language-rust"), "{debug}");
    assert!(!debug.contains("reason"), "{debug}");
    assert!(!debug.contains("$formatted:test"), "{debug}");
}
#[test]
fn timeline_item_serializes_rust_owned_message_actions() {
    let item = TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: "$event:test".to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("copyable body".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1_234),
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: true,
        is_edited: false,
        can_edit: true,
        actions: message_actions_for_timeline_item(
            "!room:test",
            &TimelineItemId::Event {
                event_id: "$event:test".to_owned(),
            },
            Some("copyable body"),
            false,
            false,
        ),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    };

    let value = serde_json::to_value(&item).expect("timeline item serializes");

    assert_eq!(
        value["actions"],
        json!({
                "can_copy": true,
                "can_forward": true,
                "can_reply": true,
                "can_permalink": true,
                "can_view_source": true,
                "permalink": "https://matrix.to/#/!room%3Atest/%24event%3Atest"
        })
    );
    let debug = format!("{item:?}");
    assert!(debug.contains("actions"), "{debug}");
    assert!(!debug.contains("https://matrix.to"), "{debug}");
    assert!(!debug.contains("$event:test"), "{debug}");
    assert!(!debug.contains("!room:test"), "{debug}");

    let redacted = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Event {
            event_id: "$redacted:test".to_owned(),
        },
        Some("redacted body"),
        true,
        true,
    );
    assert!(!redacted.can_copy);
    assert!(!redacted.can_forward);
    assert!(redacted.can_permalink);
    assert!(redacted.can_view_source);

    let media_without_body = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Event {
            event_id: "$media:test".to_owned(),
        },
        None,
        true,
        false,
    );
    assert!(!media_without_body.can_copy);
    assert!(!media_without_body.can_forward);
    assert!(media_without_body.can_permalink);
    assert!(media_without_body.can_view_source);

    let local_echo = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Transaction {
            transaction_id: "txn:test".to_owned(),
        },
        Some("local echo"),
        false,
        false,
    );
    assert_eq!(local_echo, TimelineMessageActions::default());
}
#[test]
fn message_actions_allow_reply_for_captionless_stable_events() {
    for media_kind in ["file", "image", "audio", "video"] {
        let actions = message_actions_for_timeline_item(
            "!room:test",
            &TimelineItemId::Event {
                event_id: format!("${media_kind}:test"),
            },
            None,
            true,
            false,
        );

        assert!(actions.can_reply, "{media_kind} event should be replyable");
    }

    let redacted = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Event {
            event_id: "$redacted:test".to_owned(),
        },
        None,
        true,
        true,
    );
    assert!(!redacted.can_reply);

    let local_echo = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Transaction {
            transaction_id: "txn:test".to_owned(),
        },
        None,
        true,
        false,
    );
    assert!(!local_echo.can_reply);
}
#[test]
fn message_actions_reject_stable_non_message_events() {
    let actions = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Event {
            event_id: "$state-event:test".to_owned(),
        },
        None,
        false,
        false,
    );

    assert!(!actions.can_reply);
}
#[test]
fn message_actions_allow_reply_for_empty_text_body() {
    let actions = message_actions_for_timeline_item(
        "!room:test",
        &TimelineItemId::Event {
            event_id: "$empty-text:test".to_owned(),
        },
        Some(""),
        false,
        false,
    );

    assert!(actions.can_reply);
}
#[test]
fn message_source_and_forward_events_are_typed_and_redacted_in_debug() {
    let key = TimelineKey::room(AccountKey("@alice:test".to_owned()), "!room:test");
    let source = TimelineMessageSource {
        event_id: "$event:test".to_owned(),
        sender: Some("@alice:test".to_owned()),
        timestamp_ms: Some(1234),
        body: Some("private source body".to_owned()),
        in_reply_to_event_id: Some("$root:test".to_owned()),
        thread_root: Some("$thread:test".to_owned()),
        is_redacted: false,
        is_edited: true,
        has_media: false,
        megolm_session_fingerprint: Some("AbCdEfGhIjKl".to_owned()),
        megolm_message_index: Some(2),
        megolm_session_rotation_reason: Some(TimelineMegolmSessionReason::ExpiredTime),
        original_json: Some(json!({
                "event_id": "$event:test",
                "sender": "@alice:test",
                "type": "m.room.message",
                "content": {
                    "body": "private source body",
                    "msgtype": "m.text"
            },
                "origin_server_ts": 1234
        })),
    };
    let loaded = TimelineEvent::MessageSourceLoaded {
        request_id: fake_rid(30),
        key: key.clone(),
        source: source.clone(),
    };
    let forwarded = TimelineEvent::MessageForwarded {
        request_id: fake_rid(31),
        key,
        destination_room_id: "!destination:test".to_owned(),
        transaction_id: "txn-forward-private".to_owned(),
        event_id: "$forwarded:test".to_owned(),
    };

    let value = serde_json::to_value(&loaded).expect("source event serializes");
    assert_eq!(
        value,
        json!({
                "MessageSourceLoaded": {
                    "request_id": { "connection_id": 7, "sequence": 30 },
                    "key": {
                        "account_key": "@alice:test",
                        "kind": { "Room": { "room_id": "!room:test" } }
                },
                    "source": {
                        "event_id": "$event:test",
                        "sender": "@alice:test",
                        "timestamp_ms": 1234,
                        "body": "private source body",
                        "in_reply_to_event_id": "$root:test",
                        "thread_root": "$thread:test",
                        "is_redacted": false,
                        "is_edited": true,
                        "has_media": false,
                        "megolm_session_fingerprint": "AbCdEfGhIjKl",
                        "megolm_message_index": 2,
                        "megolm_session_rotation_reason": "expiredTime",
                        "original_json": {
                            "content": {
                                "body": "private source body",
                                "msgtype": "m.text"
                        },
                            "event_id": "$event:test",
                            "origin_server_ts": 1234,
                            "sender": "@alice:test",
                            "type": "m.room.message"
                    }
                }
            }
        })
    );

    for debug in [
        format!("{source:?}"),
        format!("{loaded:?}"),
        format!("{forwarded:?}"),
    ] {
        assert!(!debug.contains("private source body"), "{debug}");
        assert!(!debug.contains("$event:test"), "{debug}");
        assert!(!debug.contains("$root:test"), "{debug}");
        assert!(!debug.contains("$thread:test"), "{debug}");
        assert!(!debug.contains("$forwarded:test"), "{debug}");
        assert!(!debug.contains("!destination:test"), "{debug}");
        assert!(!debug.contains("txn-forward-private"), "{debug}");
        assert!(!debug.contains("AbCdEfGhIjKl"), "{debug}");
    }
}
#[test]
fn timeline_item_serializes_outbound_send_state_without_raw_error() {
    let item = TimelineItem {
        request_state: None,
        id: TimelineItemId::Transaction {
            transaction_id: "txn-send-state".to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("hello".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1_234),
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: Some(TimelineSendState::NotSent {
            reason: TimelineSendFailureReason::Recoverable,
        }),
        unable_to_decrypt: None,
        display_metadata: None,
    };

    let value = serde_json::to_value(&item).expect("timeline item serializes");

    assert_eq!(
        value["send_state"],
        json!({
                "kind": "notSent",
                "reason": "recoverable"
        })
    );
    let debug = format!("{item:?}");
    assert!(debug.contains("NotSent"), "{debug}");
    assert!(!debug.contains("hello"), "{debug}");
}
#[test]
fn timeline_item_serializes_media_metadata_without_encryption_secrets() {
    let item = TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: "$media:test".to_owned(),
        },
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("synthetic caption".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1_234),
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: Some(TimelineMedia {
            kind: koushi_protocol::event::TimelineMediaKind::Image,
            filename: "synthetic-image.png".to_owned(),
            source: TimelineMediaSource {
                mxc_uri: "mxc://example.invalid/media".to_owned(),
                encrypted: true,
                encryption_version: Some("v2".to_owned()),
            },
            mimetype: Some("image/png".to_owned()),
            size: Some(68),
            width: Some(2),
            height: Some(2),
            thumbnail: Some(TimelineMediaThumbnail {
                source: TimelineMediaSource {
                    mxc_uri: "mxc://example.invalid/thumb".to_owned(),
                    encrypted: true,
                    encryption_version: Some("v2".to_owned()),
                },
                mimetype: Some("image/png".to_owned()),
                size: Some(32),
                width: Some(1),
                height: Some(1),
            }),
        }),
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: true,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    };

    let value = serde_json::to_value(&item).expect("timeline item serializes");

    assert_eq!(
        value["media"],
        json!({
                "kind": "Image",
                "filename": "synthetic-image.png",
                "source": {
                    "mxc_uri": "mxc://example.invalid/media",
                    "encrypted": true,
                    "encryption_version": "v2"
            },
                "mimetype": "image/png",
                "size": 68,
                "width": 2,
                "height": 2,
                "thumbnail": {
                    "source": {
                        "mxc_uri": "mxc://example.invalid/thumb",
                        "encrypted": true,
                        "encryption_version": "v2"
                },
                    "mimetype": "image/png",
                    "size": 32,
                    "width": 1,
                    "height": 1
            }
        })
    );
    let serialized = serde_json::to_string(&item).expect("timeline item json");
    assert!(!serialized.contains("key"));
    assert!(!serialized.contains("hashes"));

    let debug = format!("{item:?}");
    assert!(!debug.contains("synthetic caption"), "{debug}");
    assert!(!debug.contains("synthetic-image.png"), "{debug}");
    assert!(!debug.contains("mxc://example.invalid"), "{debug}");
    assert!(!debug.contains("$media:test"), "{debug}");
}
#[test]
fn media_timeline_event_debug_redacts_routing_and_media_identifiers() {
    let key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room:example.invalid",
    );
    let event = TimelineEvent::MediaUploadProgress {
        request_id: Some(RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: 7,
        }),
        key,
        transaction_id: "txn-media".to_owned(),
        index: 0,
        progress: MediaTransferProgress {
            current: 4,
            total: 8,
        },
        source: Some(TimelineMediaSource {
            mxc_uri: "mxc://example.invalid/media".to_owned(),
            encrypted: true,
            encryption_version: Some("v2".to_owned()),
        }),
    };

    let debug = format!("{event:?}");
    assert!(debug.contains("MediaUploadProgress"), "{debug}");
    assert!(debug.contains("txn-media"), "{debug}");
    assert!(!debug.contains("!room:example.invalid"), "{debug}");
    assert!(!debug.contains("@alice:example.invalid"), "{debug}");
    assert!(!debug.contains("mxc://example.invalid"), "{debug}");
}
#[test]
fn display_labels_updated_event_serializes_and_redacts_debug() {
    let labels = vec![
        TimelineDisplayLabelUpdate {
            user_id: "@alice:example.invalid".to_owned(),
            display_label: "Alice Alias".to_owned(),
        },
        TimelineDisplayLabelUpdate {
            user_id: "@bob:example.invalid".to_owned(),
            display_label: "Bobby".to_owned(),
        },
    ];
    let event = TimelineEvent::DisplayLabelsUpdated { labels };

    let value = serde_json::to_value(&event).expect("DisplayLabelsUpdated serializes");
    assert_eq!(
        value,
        json!({
                "DisplayLabelsUpdated": {
                    "labels": [
                    { "user_id": "@alice:example.invalid", "display_label": "Alice Alias" },
                    { "user_id": "@bob:example.invalid", "display_label": "Bobby" }
                ]
            }
        })
    );

    let debug = format!("{event:?}");
    assert!(debug.contains("DisplayLabelsUpdated"), "{debug}");
    assert!(!debug.contains("@alice:example.invalid"), "{debug}");
    assert!(!debug.contains("@bob:example.invalid"), "{debug}");
    assert!(!debug.contains("Alice Alias"), "{debug}");
    assert!(!debug.contains("Bobby"), "{debug}");
}
#[test]
fn timeline_items_project_redacted_visibility_from_settings() {
    let mut state = AppState::default();
    state.settings.values.display.hide_redacted = true;
    let key = TimelineKey::room(
        AccountKey("@me:example.invalid".to_owned()),
        "!room:example.invalid",
    );
    let mut event = TimelineEvent::InitialItems {
        request_id: None,
        cause_request_id: None,
        key,
        actor_generation: 0,
        generation: TimelineGeneration(0),
        items: vec![
            timeline_item_fixture("$redacted:example.invalid", true),
            timeline_item_fixture("$visible:example.invalid", false),
        ],
    };

    project_timeline_event_display_labels(&mut event, &state);

    let TimelineEvent::InitialItems { items, .. } = event else {
        panic!("expected InitialItems");
    };
    assert!(items[0].is_redacted);
    assert!(items[0].is_hidden);
    assert!(!items[1].is_redacted);
    assert!(!items[1].is_hidden);
}
#[test]
fn timeline_display_policy_update_serializes_and_redacts_debug() {
    let event = TimelineEvent::DisplayPolicyUpdated {
        hide_redacted: true,
    };

    let value = serde_json::to_value(&event).expect("DisplayPolicyUpdated serializes");
    assert_eq!(
        value,
        json!({
                "DisplayPolicyUpdated": {
                    "hide_redacted": true
            }
        })
    );

    let debug = format!("{event:?}");
    assert!(debug.contains("DisplayPolicyUpdated"), "{debug}");
    assert!(debug.contains("hide_redacted"), "{debug}");
}
#[test]
fn derive_display_label_updates_resolves_from_profile_state() {
    let mut state = AppState::default();
    state.profile.own.display_name = Some("My Name".to_owned());
    state.profile.local_aliases.insert(
        "@alice:example.invalid".to_owned(),
        "Alice Alias".to_owned(),
    );
    state.profile.local_aliases.insert(
        "@bob:example.invalid".to_owned(),
        "".to_owned(), // empty alias = cleared, falls through
    );
    state.profile.users.insert(
        "@carol:example.invalid".to_owned(),
        koushi_state::UserProfile {
            user_id: "@carol:example.invalid".to_owned(),
            display_name: Some("Carol Upstream".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );
    // own user id for resolve_user_display_name own-user fallback
    let own_user_id = Some("@me:example.invalid");

    let updates = derive_display_label_updates_for_user_ids(
        &state.profile,
        own_user_id,
        [
            "@alice:example.invalid",
            "@bob:example.invalid",
            "@carol:example.invalid",
            "@me:example.invalid",
        ],
    );

    // Alice: alias present -> label = alias
    let alice = updates
        .iter()
        .find(|u| u.user_id == "@alice:example.invalid")
        .expect("alice in updates");
    assert_eq!(alice.display_label, "Alice Alias");

    // Bob: alias is empty -> falls through to MXID since no upstream
    let bob = updates
        .iter()
        .find(|u| u.user_id == "@bob:example.invalid")
        .expect("bob in updates");
    assert_eq!(bob.display_label, "@bob:example.invalid");

    // Carol: upstream display_name in users, no alias -> label = upstream
    let carol = updates
        .iter()
        .find(|u| u.user_id == "@carol:example.invalid")
        .expect("carol in updates");
    assert_eq!(carol.display_label, "Carol Upstream");

    // Own user is included when own display_name is set
    let me = updates
        .iter()
        .find(|u| u.user_id == "@me:example.invalid")
        .expect("own user in updates");
    assert_eq!(me.display_label, "My Name");

    for index in 0..1500 {
        let user_id = format!("@unrelated-{index}:example.invalid");
        state.profile.users.insert(
            user_id.clone(),
            koushi_state::UserProfile {
                user_id,
                display_name: Some("Unrelated".to_owned()),
                display_label: "Unrelated".to_owned(),
                original_display_label: "Unrelated".to_owned(),
                mention_search_terms: Vec::new(),
                avatar: None,
            },
        );
    }
    let updates = derive_display_label_updates_for_user_ids(
        &state.profile,
        own_user_id,
        ["@unknown:example.invalid"].into_iter(),
    );
    assert_eq!(
        updates.len(),
        1,
        "unrelated cached users must not be republished"
    );
    let unknown = updates
        .iter()
        .find(|u| u.user_id == "@unknown:example.invalid")
        .expect("additional user id in updates");
    assert_eq!(unknown.display_label, "@unknown:example.invalid");
}
#[test]
fn media_download_events_redact_routing_and_source_url_in_debug() {
    let key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room:example.invalid",
    );
    let completed = TimelineEvent::MediaDownloadCompleted {
        request_id: RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: 7,
        },
        key: key.clone(),
        event_id: "$event:example.invalid".to_owned(),
        source_url: "/data/secret.png".to_owned(),
        byte_count: 1234,
        mimetype: Some("image/png".to_owned()),
        width: Some(640),
        height: Some(480),
    };

    let debug = format!("{completed:?}");
    assert!(debug.contains("MediaDownloadCompleted"), "{debug}");
    assert!(debug.contains("byte_count"), "{debug}");
    assert!(!debug.contains("!room:example.invalid"), "{debug}");
    assert!(!debug.contains("@alice:example.invalid"), "{debug}");
    assert!(!debug.contains("$event:example.invalid"), "{debug}");
    assert!(!debug.contains("/data/secret.png"), "{debug}");

    let failed = TimelineEvent::MediaDownloadFailed {
        request_id: RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: 8,
        },
        key,
        event_id: "$event:example.invalid".to_owned(),
        kind: koushi_protocol::failure::TimelineFailureKind::Network,
    };
    let debug = format!("{failed:?}");
    assert!(debug.contains("MediaDownloadFailed"), "{debug}");
    assert!(!debug.contains("$event:example.invalid"), "{debug}");
}
#[test]
fn media_download_event_serializes_with_camel_case_fields() {
    let key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room:example.invalid",
    );
    let event = TimelineEvent::MediaDownloadCompleted {
        request_id: RequestId {
            connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
            sequence: 7,
        },
        key,
        event_id: "$event:example.invalid".to_owned(),
        source_url: "/data/image.png".to_owned(),
        byte_count: 1234,
        mimetype: Some("image/png".to_owned()),
        width: Some(640),
        height: Some(480),
    };

    let value = serde_json::to_value(&event).expect("MediaDownloadCompleted serializes");
    let completed = value.get("MediaDownloadCompleted").expect("tagged variant");
    assert_eq!(
        completed.get("source_url").and_then(|v| v.as_str()),
        Some("/data/image.png")
    );
    assert_eq!(
        completed.get("byte_count").and_then(|v| v.as_u64()),
        Some(1234)
    );
    assert_eq!(
        completed.get("mimetype").and_then(|v| v.as_str()),
        Some("image/png")
    );
    assert_eq!(completed.get("width").and_then(|v| v.as_u64()), Some(640));
    assert_eq!(completed.get("height").and_then(|v| v.as_u64()), Some(480));
}
#[test]
fn avatar_metadata_events_redact_private_mxc_values() {
    let mut item = timeline_item_fixture("$event:test", false);
    item.sender_avatar = Some(koushi_state::AvatarImage {
        mxc_uri: "mxc://example.invalid/private-avatar".to_owned(),
        thumbnail: koushi_state::AvatarThumbnailState::Ready {
            source_ref: "avatar/0123456789abcdef".to_owned(),
            width: Some(64),
            height: Some(64),
            mime_type: Some("image/png".to_owned()),
        },
    });
    let debug = format!("{:?}", item);
    assert!(
        !debug.contains("mxc://example.invalid/private-avatar"),
        "{debug}"
    );
    assert!(!debug.contains("avatar/0123456789abcdef"), "{debug}");
    assert!(debug.contains("AvatarImage"), "{debug}");
}
