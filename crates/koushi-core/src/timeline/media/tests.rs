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
        thumbnail_mimetype: None,
        thumbnail_width: None,
        thumbnail_height: None,
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

fn quote_media_entry(mxc: &str) -> super::PrivateMediaEntry {
    super::PrivateMediaEntry {
        source: matrix_sdk::ruma::events::room::MediaSource::Plain(
            matrix_sdk::ruma::OwnedMxcUri::from(mxc),
        ),
        thumbnail_source: None,
        mimetype: Some("image/png".to_owned()),
        size: 42,
        width: Some(640),
        height: Some(480),
        thumbnail_mimetype: None,
        thumbnail_width: None,
        thumbnail_height: None,
    }
}

#[tokio::test]
async fn one_reply_quote_thumbnail_fetch_is_shared_by_every_reply_to_the_picture() {
    let mut tasks = std::collections::HashMap::new();
    let picture = "$picture:example.invalid";

    assert!(!super::reply_quote_thumbnail_fetch_is_pending(
        &tasks, picture
    ));
    // The first reply starts the fetch; the second reply to the same picture
    // sees the worker in the task map and coalesces into it.
    tasks.insert(picture.to_owned(), super::executor::spawn(async {}));
    assert!(super::reply_quote_thumbnail_fetch_is_pending(
        &tasks, picture
    ));

    tasks.remove(picture);
    assert!(!super::reply_quote_thumbnail_fetch_is_pending(
        &tasks, picture
    ));
}

#[test]
fn a_resident_or_off_window_original_resolves_to_its_private_source() {
    use koushi_state::{ReplyQuote, ReplyQuoteState};

    let resident = "$resident:example.invalid";
    let off_window = "$off-window:example.invalid";
    let mut media_sources = std::collections::HashMap::new();
    media_sources.insert(
        resident.to_owned(),
        quote_media_entry("mxc://test/resident"),
    );

    let mut hydration = super::super::reply_quote_hydration::ReplyQuoteHydration::default();
    let quote = ReplyQuote {
        event_id: off_window.to_owned(),
        sender: Some("@alice:example.invalid".to_owned()),
        sender_label: None,
        body_preview: None,
        formatted: None,
        mentioned_user_ids: Vec::new(),
        media: Some(koushi_state::ReplyQuoteMedia {
            kind: koushi_state::ReplyQuoteMediaKind::Image,
            animated: false,
            width: Some(640),
            height: Some(480),
            has_thumbnail_source: false,
        }),
        permalink: None,
        can_open: false,
        state: ReplyQuoteState::Ready,
    };
    hydration.learn_with_media(quote, Some(quote_media_entry("mxc://test/off-window")));

    for (event_id, expected) in [
        (resident, "mxc://test/resident"),
        (off_window, "mxc://test/off-window"),
    ] {
        let entry = super::reply_quote_media_source(&media_sources, &hydration, event_id)
            .expect("private source");
        let matrix_sdk::ruma::events::room::MediaSource::Plain(uri) = entry.source else {
            panic!("expected a plain media source");
        };
        assert_eq!(uri.as_str(), expected);
    }
    assert!(
        super::reply_quote_media_source(&media_sources, &hydration, "$missing:example.invalid")
            .is_none()
    );
}

#[tokio::test]
async fn an_already_cached_full_file_answers_the_quote_thumbnail_without_a_fetch() {
    use koushi_protocol::command::MediaDownloadSelection;

    let data_dir = tempfile::tempdir().expect("data dir");
    let entry = quote_media_entry("mxc://test/cached");
    let room_id = "!room:example.invalid";
    let event_id = "$cached:example.invalid";
    let room_hash = super::sanitize_matrix_id_for_path(room_id);
    let event_hash = super::sanitize_matrix_id_for_path(event_id);
    let (source_ref, path) = super::managed_media_download_entry(
        data_dir.path(),
        &room_hash,
        &event_hash,
        &MediaDownloadSelection::File,
    )
    .expect("cache entry");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("cache dir");
    std::fs::write(&path, b"binaryjpegfullimagedata").expect("cache file");

    let ready =
        super::reply_quote_thumbnail_from_cached_file(&entry, data_dir.path(), room_id, event_id)
            .await
            .expect("cached full file");

    assert_eq!(ready.source_ref, source_ref);
    assert!(!ready.source_ref.contains("96x96"));
    assert_eq!(ready.byte_count, b"binaryjpegfullimagedata".len() as u64);
    assert_eq!(ready.width, Some(640));
    assert_eq!(ready.height, Some(480));
}

#[tokio::test]
async fn a_reply_quote_thumbnail_downloads_at_96_by_96() {
    use koushi_protocol::command::MediaDownloadSelection;
    use matrix_sdk::ruma::api::client::media::get_content_thumbnail::v3::Method;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    server
        .mock_authed_media_thumbnail(Method::Scale, 96, 96, false)
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
    let data_dir = tempfile::tempdir().expect("data dir");
    let selection = MediaDownloadSelection::Thumbnail {
        width: 96,
        height: 96,
    };

    let ready = download_cached(&session, data_dir.path(), &selection).await;

    assert_eq!(
        ready.source_ref,
        super::media_download_thumbnail_ref(
            &super::sanitize_matrix_id_for_path("!room:example.invalid"),
            &super::sanitize_matrix_id_for_path("$media:example.invalid"),
            96,
            96,
        )
        .expect("thumbnail ref")
    );
    assert_eq!(
        std::fs::read(&ready.path).expect("thumbnail bytes"),
        b"binaryjpegthumbnaildata"
    );
}

#[test]
fn an_encrypted_thumbnail_request_keeps_the_encrypted_source_for_the_decrypt_path() {
    use koushi_protocol::command::MediaDownloadSelection;
    use matrix_sdk::ruma::events::room::{EncryptedFile, MediaSource};

    let file: EncryptedFile = serde_json::from_value(serde_json::json!({
        "url": "mxc://test/encrypted",
        "key": {
            "kty": "oct",
            "key_ops": ["encrypt", "decrypt"],
            "alg": "A256CTR",
            "k": "b50ACIv6LMn9AfMCFD1POJI_UAFWIclxAN1kWrEO2X8",
            "ext": true
        },
        "iv": "AK1wyzigZtQAAAABAAAAKK",
        "hashes": { "sha256": "/NogKqW5bz/m8xHgFiH5haFGjCNVmUIPLzfvOhHdrxY" },
        "v": "v2"
    }))
    .expect("encrypted file");
    let entry = super::PrivateMediaEntry {
        source: MediaSource::Encrypted(Box::new(file)),
        thumbnail_source: None,
        mimetype: Some("image/png".to_owned()),
        size: 42,
        width: None,
        height: None,
        thumbnail_mimetype: None,
        thumbnail_width: None,
        thumbnail_height: None,
    };
    let request = super::media_request_for_download(
        &entry,
        &MediaDownloadSelection::Thumbnail {
            width: 96,
            height: 96,
        },
    )
    .expect("supported request");

    assert!(matches!(request.source, MediaSource::Encrypted(_)));
    assert!(matches!(
        request.format,
        matrix_sdk::media::MediaFormat::Thumbnail(_)
    ));
}

#[test]
fn a_thumbnail_selection_reports_the_thumbnail_metadata_not_the_video_file_metadata() {
    use koushi_protocol::command::MediaDownloadSelection;

    let mut entry = quote_media_entry("mxc://test/video");
    entry.mimetype = Some("video/mp4".to_owned());
    entry.width = Some(1280);
    entry.height = Some(720);
    entry.thumbnail_source = Some(matrix_sdk::ruma::events::room::MediaSource::Plain(
        matrix_sdk::ruma::OwnedMxcUri::from("mxc://test/thumb"),
    ));
    entry.thumbnail_mimetype = Some("image/jpeg".to_owned());
    entry.thumbnail_width = Some(320);
    entry.thumbnail_height = Some(180);

    assert_eq!(
        super::media_download_selection_metadata(
            &entry,
            &MediaDownloadSelection::Thumbnail {
                width: 96,
                height: 96,
            },
        ),
        (Some("image/jpeg".to_owned()), Some(320), Some(180))
    );
    assert_eq!(
        super::media_download_selection_metadata(&entry, &MediaDownloadSelection::File),
        (Some("video/mp4".to_owned()), Some(1280), Some(720))
    );
}

#[tokio::test]
async fn a_cached_video_original_is_not_reused_as_a_quote_thumbnail() {
    use koushi_protocol::command::MediaDownloadSelection;

    let data_dir = tempfile::tempdir().expect("data dir");
    let mut entry = quote_media_entry("mxc://test/video-cached");
    entry.mimetype = Some("video/mp4".to_owned());
    let room_id = "!room:example.invalid";
    let event_id = "$video-cached:example.invalid";
    let room_hash = super::sanitize_matrix_id_for_path(room_id);
    let event_hash = super::sanitize_matrix_id_for_path(event_id);
    let (_, path) = super::managed_media_download_entry(
        data_dir.path(),
        &room_hash,
        &event_hash,
        &MediaDownloadSelection::File,
    )
    .expect("cache entry");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("cache dir");
    std::fs::write(&path, b"mp4bytes").expect("cache file");

    assert!(
        super::reply_quote_thumbnail_from_cached_file(&entry, data_dir.path(), room_id, event_id,)
            .await
            .is_none()
    );
}
