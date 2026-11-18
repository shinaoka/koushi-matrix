use koushi_state::{
    MediaGalleryPagination, TimelineMediaGalleryItem, TimelineMediaGalleryMedia,
    TimelineMediaGallerySource, TimelineMediaKind,
};
use matrix_sdk::ruma::events::AnySyncTimelineEvent;
use matrix_sdk::ruma::room_version_rules::RoomVersionRules;
use matrix_sdk::ruma::serde::Raw;
use serde_json::json;

use super::{
    MediaBrowserUpdate, media_event_filter, media_gallery_pagination_on_request,
    media_gallery_pagination_on_update, merge_media_gallery_items,
};

fn gallery_item(event_id: &str, timestamp_ms: u64) -> TimelineMediaGalleryItem {
    TimelineMediaGalleryItem {
        event_id: event_id.to_owned(),
        room_id: "!room:example.invalid".to_owned(),
        sender: None,
        sender_label: None,
        timestamp_ms,
        media: TimelineMediaGalleryMedia {
            kind: TimelineMediaKind::Image,
            filename: "fixture.png".to_owned(),
            source: TimelineMediaGallerySource {
                mxc_uri: "mxc://example.invalid/fixture".to_owned(),
                encrypted: false,
                encryption_version: None,
            },
            mimetype: Some("image/png".to_owned()),
            size: None,
            width: None,
            height: None,
            thumbnail: None,
        },
    }
}

fn sync_event(content: serde_json::Value, event_type: &str) -> AnySyncTimelineEvent {
    Raw::new(&json!({
        "type": event_type,
        "event_id": "$event:example.invalid",
        "sender": "@member:example.invalid",
        "origin_server_ts": 1_900_000_000_000_u64,
        "content": content,
    }))
    .expect("raw event")
    .cast_unchecked::<AnySyncTimelineEvent>()
    .deserialize()
    .expect("sync event")
}

fn rules() -> RoomVersionRules {
    matrix_sdk::ruma::RoomVersionId::V10
        .rules()
        .expect("v10 rules")
}

#[test]
fn media_filter_admits_media_messages_and_rejects_text_and_state() {
    for msgtype in ["m.image", "m.video", "m.file", "m.audio"] {
        let event = sync_event(
            json!({ "msgtype": msgtype, "body": "fixture", "url": "mxc://example.invalid/m" }),
            "m.room.message",
        );
        assert!(media_event_filter(&event, &rules()), "{msgtype}");
    }
    let text = sync_event(
        json!({ "msgtype": "m.text", "body": "hello" }),
        "m.room.message",
    );
    assert!(!media_event_filter(&text, &rules()));
    let reaction = sync_event(
        json!({ "m.relates_to": { "rel_type": "m.annotation", "event_id": "$x:example.invalid", "key": "+" } }),
        "m.reaction",
    );
    assert!(!media_event_filter(&reaction, &rules()));
}

#[test]
fn pagination_request_starts_from_idle_or_failed_and_coalesces_otherwise() {
    assert_eq!(
        media_gallery_pagination_on_request(MediaGalleryPagination::Idle),
        Some(MediaGalleryPagination::Loading)
    );
    assert_eq!(
        media_gallery_pagination_on_request(MediaGalleryPagination::Failed),
        Some(MediaGalleryPagination::Loading)
    );
    assert_eq!(
        media_gallery_pagination_on_request(MediaGalleryPagination::Loading),
        None,
        "a duplicate request while a page is in flight coalesces"
    );
    assert_eq!(
        media_gallery_pagination_on_request(MediaGalleryPagination::ReachedStart),
        None,
        "the room start is terminal"
    );
}

#[test]
fn pagination_settles_from_page_outcomes_and_browser_stop() {
    let loading = MediaGalleryPagination::Loading;
    assert_eq!(
        media_gallery_pagination_on_update(loading, &MediaBrowserUpdate::Rows(Vec::new())),
        None
    );
    assert_eq!(
        media_gallery_pagination_on_update(
            loading,
            &MediaBrowserUpdate::PageFinished {
                reached_start: false
            }
        ),
        Some(MediaGalleryPagination::Idle)
    );
    assert_eq!(
        media_gallery_pagination_on_update(
            loading,
            &MediaBrowserUpdate::PageFinished {
                reached_start: true
            }
        ),
        Some(MediaGalleryPagination::ReachedStart)
    );
    assert_eq!(
        media_gallery_pagination_on_update(loading, &MediaBrowserUpdate::PageFailed),
        Some(MediaGalleryPagination::Failed)
    );
    assert_eq!(
        media_gallery_pagination_on_update(loading, &MediaBrowserUpdate::Stopped),
        Some(MediaGalleryPagination::Failed),
        "a browser that stops mid-page must not leave the gallery loading forever"
    );
    assert_eq!(
        media_gallery_pagination_on_update(
            MediaGalleryPagination::ReachedStart,
            &MediaBrowserUpdate::Stopped
        ),
        None
    );
}

#[test]
fn browser_rows_extend_the_window_gallery_beyond_resident_messages() {
    let window = vec![gallery_item("$recent", 30), gallery_item("$middle", 20)];
    let browser = vec![
        gallery_item("$oldest", 10),
        gallery_item("$middle", 20),
        gallery_item("$recent", 30),
    ];

    let merged = merge_media_gallery_items(window.clone(), browser);
    let ids = merged
        .iter()
        .map(|item| item.event_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["$recent", "$middle", "$oldest"]);

    assert_eq!(
        merge_media_gallery_items(window.clone(), Vec::new()),
        window,
        "without a browser the gallery is the window projection unchanged"
    );
}
