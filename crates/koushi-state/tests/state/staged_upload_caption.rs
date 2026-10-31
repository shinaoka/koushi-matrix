//! #1204: whether a prepared-upload send consumes the composer draft.

use koushi_state::{
    ComposerDocument, ComposerInline, StagedUploadItem, StagedUploadKind, StagedUploadPreparation,
    staged_upload_send_consumes_composer_draft,
};

fn staged_item(caption: Option<ComposerDocument>) -> StagedUploadItem {
    StagedUploadItem {
        staged_id: "staged".to_owned(),
        room_id: "!room:example.invalid".to_owned(),
        position: 1,
        filename: "photo.png".to_owned(),
        mime_type: "image/png".to_owned(),
        byte_count: 128,
        kind: StagedUploadKind::Image {
            width: Some(8),
            height: Some(8),
        },
        caption,
        compression_choice: koushi_state::StagedUploadCompressionChoice::NotApplicable,
        preparation: StagedUploadPreparation::Ready {
            variants: Vec::new(),
            selected: Default::default(),
            pending: None,
            generation: 0,
        },
    }
}

fn document(text: &str) -> ComposerDocument {
    ComposerDocument::new(vec![ComposerInline::Text {
        text: text.to_owned(),
    }])
}

#[test]
fn a_single_attachment_carrying_the_draft_consumes_it() {
    let caption = document("holiday photo");
    assert!(staged_upload_send_consumes_composer_draft(
        &[staged_item(Some(caption.clone()))],
        Some(&caption)
    ));
}

#[test]
fn an_edited_or_absent_caption_keeps_the_draft() {
    let draft = document("holiday photo");
    // The user edited the caption after the seed.
    assert!(!staged_upload_send_consumes_composer_draft(
        &[staged_item(Some(document("holiday photo #2")))],
        Some(&draft)
    ));
    // No caption at all.
    assert!(!staged_upload_send_consumes_composer_draft(
        &[staged_item(None)],
        Some(&draft)
    ));
}

#[test]
fn a_whitespace_only_or_absent_draft_is_not_a_caption() {
    let caption = document("   ");
    assert!(!staged_upload_send_consumes_composer_draft(
        &[staged_item(Some(caption.clone()))],
        Some(&caption)
    ));
    assert!(!staged_upload_send_consumes_composer_draft(
        &[staged_item(Some(document("holiday photo")))],
        None
    ));
}

#[test]
fn any_other_item_count_keeps_the_draft() {
    let caption = document("holiday photo");
    assert!(!staged_upload_send_consumes_composer_draft(
        &[
            staged_item(Some(caption.clone())),
            staged_item(Some(caption.clone()))
        ],
        Some(&caption)
    ));
    assert!(!staged_upload_send_consumes_composer_draft(
        &[],
        Some(&caption)
    ));
}
