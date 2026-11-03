use koushi_state::{AppAction, TimelineMediaKind as GalleryTimelineMediaKind};

use matrix_sdk::attachment::AttachmentInfo;

use koushi_protocol::command::{UploadMediaKind, UploadMediaRequest};
use koushi_protocol::event::{TimelineDiff, TimelineItemId, TimelineMediaKind};

use matrix_sdk::ruma::uint;

use koushi_protocol::command::{
    ImageUploadCompressionPolicy, ImageUploadCompressionState, ImageUploadDimensions,
    ImageUploadVariantInfo, ImageUploadVariantKind,
};

use super::super::display_projection::apply_timeline_diffs_to_items;
use super::super::item_projection::attachment_info_for_upload;
use super::super::test_support::{room_key, timeline_item, timeline_media_item};
use super::{authoritative_media_gallery_replacement, media_gallery_items_from_timeline_items};

fn media_entry() -> super::PrivateMediaEntry {
    super::PrivateMediaEntry {
        source: matrix_sdk::ruma::events::room::MediaSource::Plain(
            matrix_sdk::ruma::OwnedMxcUri::from("mxc://test/media"),
        ),
        thumbnail_source: None,
        mimetype: None,
        size: 0,
        width: None,
        height: None,
    }
}

#[test]
fn media_gallery_projection_keeps_event_media_newest_first() {
    let mut transaction_media = timeline_media_item(
        "$local:test",
        "@me:test",
        None,
        3,
        "local.png",
        TimelineMediaKind::Image,
    );
    transaction_media.id = TimelineItemId::Transaction {
        transaction_id: "txn-local".to_owned(),
    };
    let items = vec![
        timeline_media_item(
            "$old:test",
            "@alice:test",
            Some("Alice"),
            1,
            "old.png",
            TimelineMediaKind::Image,
        ),
        timeline_item("$text:test", Some("text"), "@bob:test", false),
        transaction_media,
        timeline_media_item(
            "$new:test",
            "@carol:test",
            Some("Carol"),
            2,
            "new.png",
            TimelineMediaKind::Image,
        ),
    ];

    let gallery = media_gallery_items_from_timeline_items(&room_key(), &items);

    assert_eq!(gallery.len(), 2);
    assert_eq!(gallery[0].event_id, "$new:test");
    assert_eq!(gallery[0].sender.as_deref(), Some("@carol:test"));
    assert_eq!(gallery[0].sender_label.as_deref(), Some("Carol"));
    assert_eq!(gallery[0].timestamp_ms, 2);
    assert_eq!(gallery[0].media.kind, GalleryTimelineMediaKind::Image);
    assert_eq!(gallery[0].media.filename, "new.png");
    assert!(gallery[0].media.source.encrypted);
    assert_eq!(
        gallery[0].media.thumbnail.as_ref().map(|thumb| thumb.width),
        Some(Some(160))
    );
    assert_eq!(gallery[1].event_id, "$old:test");
}

#[test]
fn media_gallery_projection_recomputes_after_timeline_diffs() {
    let mut items = vec![
        timeline_media_item(
            "$old:test",
            "@alice:test",
            None,
            1,
            "old.png",
            TimelineMediaKind::Image,
        ),
        timeline_media_item(
            "$new:test",
            "@bob:test",
            None,
            2,
            "new.png",
            TimelineMediaKind::Image,
        ),
    ];

    apply_timeline_diffs_to_items(&mut items, &[TimelineDiff::Remove { index: 1 }]);
    let gallery = media_gallery_items_from_timeline_items(&room_key(), &items);
    assert_eq!(gallery.len(), 1);
    assert_eq!(gallery[0].event_id, "$old:test");

    apply_timeline_diffs_to_items(&mut items, &[TimelineDiff::Reset { items: Vec::new() }]);
    assert!(media_gallery_items_from_timeline_items(&room_key(), &items).is_empty());
}

#[test]
fn relay_overflow_authoritative_snapshot_replaces_media_gallery_and_emits_action() {
    let key = room_key();
    let old_navigation = vec![timeline_media_item(
        "$old:test",
        "@alice:test",
        None,
        1,
        "old.png",
        TimelineMediaKind::Image,
    )];
    let new_navigation = vec![timeline_media_item(
        "$new:test",
        "@bob:test",
        None,
        2,
        "new.png",
        TimelineMediaKind::Image,
    )];
    let old_gallery = media_gallery_items_from_timeline_items(&key, &old_navigation);
    let replacement = authoritative_media_gallery_replacement(&key, &old_gallery, &new_navigation)
        .expect("changed authoritative snapshot must emit gallery replacement");
    assert_eq!(replacement.items.len(), 1);
    assert_eq!(replacement.items[0].event_id, "$new:test");
    assert!(matches!(
        replacement.action,
        AppAction::MediaGalleryUpdated { items, .. }
            if items.len() == 1 && items[0].event_id == "$new:test"
    ));
}

#[test]
fn media_source_reconciliation_drops_removed_events_but_keeps_downloads() {
    let mut media_sources = std::collections::HashMap::new();
    for event_id in ["$resident:test", "$removed:test", "$downloading:test"] {
        media_sources.insert(event_id.to_owned(), media_entry());
    }
    let resident: std::collections::HashSet<String> =
        ["$resident:test".to_owned()].into_iter().collect();
    let downloads: std::collections::HashSet<String> =
        ["$downloading:test".to_owned()].into_iter().collect();

    super::retain_resident_media_sources(&mut media_sources, &resident, &downloads);

    assert!(media_sources.contains_key("$resident:test"));
    assert!(media_sources.contains_key("$downloading:test"));
    assert!(!media_sources.contains_key("$removed:test"));
}

#[test]
fn completed_download_releases_media_source_only_when_the_event_left_the_window() {
    let mut media_sources = std::collections::HashMap::new();
    media_sources.insert("$gone:test".to_owned(), media_entry());
    media_sources.insert("$resident:test".to_owned(), media_entry());
    let items = vec![timeline_item(
        "$resident:test",
        Some("text"),
        "@bob:test",
        false,
    )];

    super::release_removed_media_source(&mut media_sources, &items, "$gone:test");
    assert!(!media_sources.contains_key("$gone:test"));

    super::release_removed_media_source(&mut media_sources, &items, "$resident:test");
    assert!(media_sources.contains_key("$resident:test"));
}

#[test]
fn sdk_media_diffs_that_can_remove_entries_are_classified() {
    use std::sync::Arc;

    use eyeball_im::VectorDiff;
    use matrix_sdk_ui::timeline::TimelineItem as SdkTimelineItem;

    type Diff = VectorDiff<Arc<SdkTimelineItem>>;

    assert!(super::sdk_media_diff_removes_items(&Diff::PopFront));
    assert!(super::sdk_media_diff_removes_items(&Diff::PopBack));
    assert!(super::sdk_media_diff_removes_items(&Diff::Remove {
        index: 0
    }));
    assert!(super::sdk_media_diff_removes_items(&Diff::Truncate {
        length: 0
    }));
    assert!(!super::sdk_media_diff_removes_items(&Diff::Clear));
}

#[test]
fn attachment_info_for_image_upload_uses_selected_variant_metadata() {
    let request = UploadMediaRequest {
        filename: "private-screenshot.jpg".to_owned(),
        mime_type: "image/jpeg".to_owned(),
        bytes: vec![1, 2, 3, 4],
        kind: UploadMediaKind::Image {
            width: Some(1200),
            height: Some(900),
        },
        compression: Some(ImageUploadCompressionState {
            mode: koushi_state::ImageUploadCompressionMode::Always,
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
                byte_count: 4,
                dimensions: Some(ImageUploadDimensions {
                    width: 1200,
                    height: 900,
                }),
            },
            selected_variant: ImageUploadVariantKind::Compressed,
            skipped_small_image: false,
            metadata_stripped: true,
            thumbnail_refreshed: true,
        }),
        thumbnail: None,
        caption: None,
    };

    match attachment_info_for_upload(&request) {
        AttachmentInfo::Image(info) => {
            assert_eq!(info.width, Some(uint!(1200)));
            assert_eq!(info.height, Some(uint!(900)));
            assert_eq!(info.size, Some(uint!(4)));
        }
        other => panic!("expected image info, got {other:?}"),
    }
}

/// Download one event's media exactly as the actor does, for the cache-key test.
async fn download_cached(
    session: &std::sync::Arc<koushi_sdk::MatrixClientSession>,
    data_dir: &std::path::Path,
    selection: &koushi_protocol::command::MediaDownloadSelection,
) -> super::MediaDownloadReady {
    let entry = media_entry();
    let request = super::media_request_for_download(&entry, selection).expect("supported request");
    match super::TimelineActor::download_media_for(
        session.clone(),
        Some(data_dir.to_path_buf()),
        "!room:example.invalid".to_owned(),
        "$media:example.invalid".to_owned(),
        entry,
        selection,
        request,
    )
    .await
    {
        super::MediaDownloadOutcome::Ready(ready) => ready,
        super::MediaDownloadOutcome::Failed(kind) => panic!("download failed: {kind:?}"),
    }
}

/// #1770: the managed entry must name what was downloaded, not only the event.
/// A thumbnail request and a `File` request for one event are different bytes,
/// so whichever ran first must never answer the other.
#[tokio::test]
async fn a_thumbnail_download_and_a_file_download_of_one_event_stay_separate() {
    use koushi_protocol::command::MediaDownloadSelection;
    use matrix_sdk::ruma::api::client::media::get_content_thumbnail::v3::Method;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    server.mock_authed_media_download().ok_image().mount().await;
    server
        .mock_authed_media_thumbnail(Method::Scale, 800, 800, false)
        .ok()
        .mount()
        .await;
    let session = std::sync::Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let thumbnail = MediaDownloadSelection::Thumbnail {
        width: 800,
        height: 800,
    };
    let file = MediaDownloadSelection::File;

    // Either order: the second download must not be answered from the first.
    for thumbnail_first in [true, false] {
        let data_dir = tempfile::tempdir().expect("data dir");
        let (first, second) = if thumbnail_first {
            (&thumbnail, &file)
        } else {
            (&file, &thumbnail)
        };
        let first_ready = download_cached(&session, data_dir.path(), first).await;
        let second_ready = download_cached(&session, data_dir.path(), second).await;
        let (file_ready, thumbnail_ready) = if thumbnail_first {
            (second_ready, first_ready)
        } else {
            (first_ready, second_ready)
        };

        assert_ne!(
            file_ready.source_ref, thumbnail_ready.source_ref,
            "thumbnail_first={thumbnail_first}: both selections must name distinct cache entries"
        );
        assert_ne!(
            file_ready.path, thumbnail_ready.path,
            "thumbnail_first={thumbnail_first}: both selections must write distinct files"
        );
        assert_eq!(
            std::fs::read(&file_ready.path).expect("file bytes"),
            b"binaryjpegfullimagedata",
            "thumbnail_first={thumbnail_first}: the File entry must hold the original bytes"
        );
        assert_eq!(
            std::fs::read(&thumbnail_ready.path).expect("thumbnail bytes"),
            b"binaryjpegthumbnaildata",
            "thumbnail_first={thumbnail_first}: the thumbnail entry must hold the thumbnail"
        );
    }
}
