use super::super::{CoreCommand, test_support::fake_rid};
use super::*;
use crate::ids::AccountKey;
use koushi_state::{ImageUploadCompressionMode, MentionIntent, MentionTarget};

fn test_session_key() -> crate::SessionKeyId {
    crate::SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@a:test".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

#[test]
fn send_text_debug_redacts_body_and_mentions() {
    let command = TimelineCommand::SendText {
        request_id: fake_rid(6),
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
        transaction_id: "txn-text".to_owned(),
        document: ComposerDocument::new(vec![
            koushi_state::ComposerInline::Text {
                text: "secret text body ".to_owned(),
            },
            koushi_state::ComposerInline::Mention {
                target: MentionTarget::User {
                    user_id: "@alice:example.test".to_owned(),
                    display_label: "Alice".to_owned(),
                },
                display_label: "Alice".to_owned(),
            },
        ]),
    };

    let debug = format!("{command:?}");
    assert!(debug.contains("SendText"), "{debug}");
    assert!(!debug.contains("txn-text"), "{debug}");
    assert!(debug.contains("TimelineKey(..)"), "{debug}");
    assert!(debug.contains("TransactionId(..)"), "{debug}");
    assert!(!debug.contains("!room:test"), "{debug}");
    assert!(!debug.contains("secret text body"), "{debug}");
    assert!(!debug.contains("@alice:example.test"), "{debug}");
    assert!(!debug.contains("Alice"), "{debug}");
}

#[test]
fn send_reply_debug_redacts_body_and_event_ids() {
    let command = TimelineCommand::SendReply {
        request_id: fake_rid(7),
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
        transaction_id: "txn-reply".to_owned(),
        in_reply_to_event_id: "$event:test".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("secret reply body".to_owned()),
    };

    let debug = format!("{command:?}");
    assert!(debug.contains("SendReply"), "{debug}");
    assert!(!debug.contains("txn-reply"), "{debug}");
    assert!(debug.contains("TimelineKey(..)"), "{debug}");
    assert!(debug.contains("TransactionId(..)"), "{debug}");
    assert!(!debug.contains("!room:test"), "{debug}");
    assert!(!debug.contains("secret reply body"), "{debug}");
    assert!(!debug.contains("$event:test"), "{debug}");
}

#[test]
fn forward_message_debug_redacts_source_destination_and_transaction() {
    let request_id = fake_rid(71);
    let command = TimelineCommand::ForwardMessage {
        request_id,
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
        source_event_id: "$source-event:test".to_owned(),
        destination_room_id: "!destination-room:test".to_owned(),
        transaction_id: "txn-forward-private".to_owned(),
    };

    assert_eq!(CoreCommand::Timeline(command).request_id(), request_id);

    let command = TimelineCommand::ForwardMessage {
        request_id,
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
        source_event_id: "$source-event:test".to_owned(),
        destination_room_id: "!destination-room:test".to_owned(),
        transaction_id: "txn-forward-private".to_owned(),
    };
    let debug = format!("{command:?}");
    assert!(debug.contains("ForwardMessage"), "{debug}");
    assert!(debug.contains("TimelineKey(..)"), "{debug}");
    assert!(debug.contains("EventId(..)"), "{debug}");
    assert!(debug.contains("RoomId(..)"), "{debug}");
    assert!(debug.contains("TransactionId(..)"), "{debug}");
    assert!(!debug.contains("@a:test"), "{debug}");
    assert!(!debug.contains("!source-room:test"), "{debug}");
    assert!(!debug.contains("$source-event:test"), "{debug}");
    assert!(!debug.contains("!destination-room:test"), "{debug}");
    assert!(!debug.contains("txn-forward-private"), "{debug}");
}

#[test]
fn load_message_source_debug_redacts_timeline_key_and_event_id() {
    let request_id = fake_rid(72);
    let command = TimelineCommand::LoadMessageSource {
        request_id,
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
        event_id: "$source-event:test".to_owned(),
    };

    assert_eq!(CoreCommand::Timeline(command).request_id(), request_id);

    let command = TimelineCommand::LoadMessageSource {
        request_id,
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!source-room:test"),
        event_id: "$source-event:test".to_owned(),
    };
    let debug = format!("{command:?}");
    assert!(debug.contains("LoadMessageSource"), "{debug}");
    assert!(debug.contains("TimelineKey(..)"), "{debug}");
    assert!(debug.contains("EventId(..)"), "{debug}");
    assert!(!debug.contains("@a:test"), "{debug}");
    assert!(!debug.contains("!source-room:test"), "{debug}");
    assert!(!debug.contains("$source-event:test"), "{debug}");
}

#[test]
fn upload_media_debug_redacts_filename_caption_and_bytes() {
    let dimensions = ImageUploadDimensions {
        width: 1200,
        height: 900,
    };
    let compression = ImageUploadCompressionState {
        mode: ImageUploadCompressionMode::Always,
        policy: ImageUploadCompressionPolicy::default(),
        original: ImageUploadVariantInfo {
            mime_type: "image/jpeg".to_owned(),
            byte_count: 3_200_000,
            dimensions: Some(ImageUploadDimensions {
                width: 4032,
                height: 3024,
            }),
        },
        selected: ImageUploadVariantInfo {
            mime_type: "image/jpeg".to_owned(),
            byte_count: 128_000,
            dimensions: Some(dimensions),
        },
        selected_variant: ImageUploadVariantKind::Compressed,
        skipped_small_image: false,
        metadata_stripped: true,
        thumbnail_refreshed: true,
    };
    let command = TimelineCommand::UploadAndSendMedia {
        request_id: fake_rid(8),
        expected_account: test_session_key(),
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test"),
        transaction_id: "txn-media".to_owned(),
        request: UploadMediaRequest {
            filename: "private-fixture-name.png".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: vec![1, 2, 3, 4],
            kind: UploadMediaKind::Image {
                width: Some(2),
                height: Some(2),
            },
            compression: Some(compression),
            thumbnail: Some(UploadMediaThumbnail {
                mime_type: "image/jpeg".to_owned(),
                bytes: vec![9, 8, 7, 6],
                width: 320,
                height: 240,
            }),
            caption: Some(koushi_state::build_formatted_message_draft(
                "private caption",
                MentionIntent::default(),
            )),
        },
    };

    let debug = format!("{command:?}");
    assert!(debug.contains("UploadAndSendMedia"), "{debug}");
    assert!(!debug.contains("txn-media"), "{debug}");
    assert!(debug.contains("TimelineKey(..)"), "{debug}");
    assert!(debug.contains("TransactionId(..)"), "{debug}");
    assert!(!debug.contains("!room:test"), "{debug}");
    assert!(debug.contains("image/png"), "{debug}");
    assert!(debug.contains("Compressed"), "{debug}");
    assert!(debug.contains("thumbnail"), "{debug}");
    assert!(!debug.contains("private-fixture-name.png"), "{debug}");
    assert!(!debug.contains("private caption"), "{debug}");
    assert!(!debug.contains("1, 2, 3, 4"), "{debug}");
    assert!(!debug.contains("9, 8, 7, 6"), "{debug}");
}

#[test]
fn image_upload_compression_policy_preserves_aspect_ratio_and_skips_small_images() {
    let policy = ImageUploadCompressionPolicy::default();

    assert_eq!(policy.threshold_bytes, 1_048_576);
    assert_eq!(policy.threshold_long_edge, 2560);
    assert_eq!(policy.target_long_edge, 2048);
    assert_eq!(policy.quality_percent, 82);
    assert_eq!(
        policy.target_dimensions_for(ImageUploadDimensions {
            width: 4032,
            height: 3024
        }),
        ImageUploadDimensions {
            width: 2048,
            height: 1536
        }
    );
    assert_eq!(
        policy.target_dimensions_for(ImageUploadDimensions {
            width: 1024,
            height: 768
        }),
        ImageUploadDimensions {
            width: 1024,
            height: 768
        }
    );

    let small = ImageUploadVariantInfo {
        mime_type: "image/png".to_owned(),
        byte_count: 64_000,
        dimensions: Some(ImageUploadDimensions {
            width: 800,
            height: 600,
        }),
    };
    let large_by_size = ImageUploadVariantInfo {
        mime_type: "image/png".to_owned(),
        byte_count: 2_000_000,
        dimensions: Some(ImageUploadDimensions {
            width: 800,
            height: 600,
        }),
    };
    let large_by_dimension = ImageUploadVariantInfo {
        mime_type: "image/png".to_owned(),
        byte_count: 64_000,
        dimensions: Some(ImageUploadDimensions {
            width: 4096,
            height: 512,
        }),
    };

    assert!(policy.should_skip(&small));
    assert!(!policy.should_skip(&large_by_size));
    assert!(!policy.should_skip(&large_by_dimension));
}

#[test]
fn retry_and_cancel_send_debug_redacts_timeline_key_and_transaction_id() {
    let key = TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test");
    let retry = TimelineCommand::RetrySend {
        request_id: fake_rid(9),
        key: key.clone(),
        transaction_id: "txn-private".to_owned(),
    };
    let cancel = TimelineCommand::CancelSend {
        request_id: fake_rid(10),
        key,
        transaction_id: "txn-private".to_owned(),
    };

    for debug in [format!("{retry:?}"), format!("{cancel:?}")] {
        assert!(!debug.contains("!room:test"), "{debug}");
        assert!(!debug.contains("@a:test"), "{debug}");
        assert!(!debug.contains("txn-private"), "{debug}");
        assert!(debug.contains("TransactionId(..)"), "{debug}");
    }
}

#[test]
fn submit_text_and_reply_debug_redact_timeline_key_and_transaction_id() {
    let key = TimelineKey::room(AccountKey("@a:test".to_owned()), "!room:test");
    let submit_text = TimelineCommand::SubmitText {
        request_id: fake_rid(11),
        expected_account: test_session_key(),
        submission_id: koushi_state::SubmissionId::new("submission"),
        key: key.clone(),
        transaction_id: "txn-submit-private".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("secret submit body".to_owned()),
        draft_revision: 1.into(),
    };
    let submit_reply = TimelineCommand::SubmitReply {
        request_id: fake_rid(12),
        expected_account: test_session_key(),
        submission_id: koushi_state::SubmissionId::new("submission"),
        key,
        transaction_id: "txn-submit-private".to_owned(),
        in_reply_to_event_id: "$event:test".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("secret submit body".to_owned()),
        draft_revision: 1.into(),
    };

    for (debug, variant) in [
        (format!("{submit_text:?}"), "SubmitText"),
        (format!("{submit_reply:?}"), "SubmitReply"),
    ] {
        assert!(debug.contains(variant), "{debug}");
        assert!(!debug.contains("txn-submit-private"), "{debug}");
        assert!(debug.contains("TimelineKey(..)"), "{debug}");
        assert!(debug.contains("TransactionId(..)"), "{debug}");
        assert!(!debug.contains("!room:test"), "{debug}");
        assert!(!debug.contains("@a:test"), "{debug}");
        assert!(!debug.contains("$event:test"), "{debug}");
        assert!(!debug.contains("secret submit body"), "{debug}");
    }
}
