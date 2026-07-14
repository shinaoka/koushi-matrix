use koushi_state::{
    AppAction, AppState, ComposerTarget, FormattedMessageDraft, ImageUploadCompressionMode,
    MentionIntent, RoomSummary, RoomTags, SessionInfo, SessionState, StagedUploadCompressionChoice,
    StagedUploadItem, StagedUploadKind, TimelineMediaGalleryItem, TimelineMediaGalleryMedia,
    TimelineMediaGallerySource, TimelineMediaGalleryThumbnail, TimelineMediaKind, UiEvent,
    UploadStagingStore, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn room(room_id: &str) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn selected_room_state(room_id: &str) -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room("room-a"), room("room-b")],
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: room_id.to_owned(),
        },
    );
    state
}

fn caption(body: &str) -> FormattedMessageDraft {
    FormattedMessageDraft {
        plain_body: body.to_owned(),
        formatted_body: None,
        mentions: MentionIntent::default(),
    }
}

fn staged_file(id: &str, room_id: &str, position: u64) -> StagedUploadItem {
    StagedUploadItem {
        staged_id: id.to_owned(),
        room_id: room_id.to_owned(),
        position,
        filename: format!("{id}.txt"),
        mime_type: "text/plain".to_owned(),
        byte_count: 128,
        kind: StagedUploadKind::File,
        caption: Some(caption("private caption")),
        compression_choice: StagedUploadCompressionChoice::NotApplicable,
        preparation: Default::default(),
    }
}

fn gallery_item(event_id: &str, room_id: &str, timestamp_ms: u64) -> TimelineMediaGalleryItem {
    TimelineMediaGalleryItem {
        event_id: event_id.to_owned(),
        room_id: room_id.to_owned(),
        sender: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        timestamp_ms,
        media: TimelineMediaGalleryMedia {
            kind: TimelineMediaKind::Image,
            filename: "private-image.png".to_owned(),
            source: TimelineMediaGallerySource {
                mxc_uri: "mxc://example.invalid/private-image".to_owned(),
                encrypted: true,
                encryption_version: Some("v2".to_owned()),
            },
            mimetype: Some("image/png".to_owned()),
            size: Some(2048),
            width: Some(640),
            height: Some(480),
            thumbnail: Some(TimelineMediaGalleryThumbnail {
                source: TimelineMediaGallerySource {
                    mxc_uri: "mxc://example.invalid/private-thumb".to_owned(),
                    encrypted: true,
                    encryption_version: Some("v2".to_owned()),
                },
                mimetype: Some("image/png".to_owned()),
                size: Some(512),
                width: Some(160),
                height: Some(120),
            }),
        },
    }
}

#[test]
fn upload_staging_tracks_multiple_files_for_selected_room_only() {
    let mut state = selected_room_state("room-a");

    let effects = reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: ComposerTarget::Main {
                room_id: "room-a".to_owned(),
            },
            items: vec![
                staged_file("stage-2", "room-a", 2),
                staged_file("stage-1", "room-a", 1),
            ],
        },
    );

    assert_eq!(state.timeline.staged_uploads.len(), 2);
    assert_eq!(state.timeline.staged_uploads[0].staged_id, "stage-1");
    assert_eq!(state.timeline.staged_uploads[1].staged_id, "stage-2");

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: ComposerTarget::Main {
                room_id: "room-b".to_owned(),
            },
            items: vec![staged_file("stage-b", "room-b", 1)],
        },
    );

    assert_eq!(
        effects,
        vec![koushi_state::AppEffect::EmitUiEvent(
            UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }
        )]
    );
    assert_eq!(state.timeline.staged_uploads.len(), 1);
    assert_eq!(state.timeline.staged_uploads[0].staged_id, "stage-b");
}

#[test]
fn upload_staging_updates_caption_and_compression_choice() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: ComposerTarget::Main {
                room_id: "room-a".to_owned(),
            },
            items: vec![StagedUploadItem {
                kind: StagedUploadKind::Image {
                    width: Some(4000),
                    height: Some(3000),
                },
                compression_choice: StagedUploadCompressionChoice::Original,
                ..staged_file("stage-1", "room-a", 1)
            }],
        },
    );

    reduce(
        &mut state,
        AppAction::UploadStagingCaptionChanged {
            target: ComposerTarget::Main {
                room_id: "room-a".to_owned(),
            },
            staged_id: "stage-1".to_owned(),
            caption: Some(caption("updated caption")),
        },
    );
    reduce(
        &mut state,
        AppAction::UploadStagingCompressionChanged {
            target: ComposerTarget::Main {
                room_id: "room-a".to_owned(),
            },
            staged_id: "stage-1".to_owned(),
            compression_choice: StagedUploadCompressionChoice::Ask,
        },
    );

    let staged = &state.timeline.staged_uploads[0];
    assert_eq!(
        staged.caption.as_ref().unwrap().plain_body,
        "updated caption"
    );
    assert_eq!(
        staged.compression_choice,
        StagedUploadCompressionChoice::Ask
    );

    reduce(
        &mut state,
        AppAction::UploadStagingCompressionChanged {
            target: ComposerTarget::Main {
                room_id: "room-a".to_owned(),
            },
            staged_id: "stage-1".to_owned(),
            compression_choice: StagedUploadCompressionChoice::Compressed {
                mode: ImageUploadCompressionMode::Always,
            },
        },
    );
    assert_eq!(
        state.timeline.staged_uploads[0].compression_choice,
        StagedUploadCompressionChoice::Compressed {
            mode: ImageUploadCompressionMode::Always
        }
    );
}

#[test]
fn media_gallery_projection_is_ordered_and_room_scoped() {
    let mut state = selected_room_state("room-a");

    reduce(
        &mut state,
        AppAction::MediaGalleryUpdated {
            room_id: "room-a".to_owned(),
            items: vec![
                gallery_item("$old", "room-a", 1_900_000_000_000),
                gallery_item("$new", "room-a", 1_900_000_060_000),
            ],
        },
    );
    reduce(
        &mut state,
        AppAction::MediaGalleryUpdated {
            room_id: "room-b".to_owned(),
            items: vec![gallery_item("$other", "room-b", 1_900_000_090_000)],
        },
    );

    assert_eq!(state.timeline.media_gallery.len(), 2);
    assert_eq!(state.timeline.media_gallery[0].event_id, "$new");
    assert_eq!(state.timeline.media_gallery[1].event_id, "$old");

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    assert_eq!(state.timeline.media_gallery.len(), 1);
    assert_eq!(state.timeline.media_gallery[0].event_id, "$other");
}

#[test]
fn upload_staging_store_is_not_serialized_but_selected_projection_is() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: ComposerTarget::Main {
                room_id: "room-a".to_owned(),
            },
            items: vec![staged_file("stage-1", "room-a", 1)],
        },
    );
    reduce(
        &mut state,
        AppAction::MediaGalleryUpdated {
            room_id: "room-a".to_owned(),
            items: vec![gallery_item("$media", "room-a", 1_900_000_000_000)],
        },
    );

    let serialized = serde_json::to_value(&state).expect("serialize app state");
    assert!(serialized.get("upload_staging").is_none());
    assert_eq!(
        serialized["timeline"]["staged_uploads"][0]["staged_id"],
        "stage-1"
    );
    assert_eq!(
        serialized["timeline"]["media_gallery"][0]["event_id"],
        "$media"
    );
}

#[test]
fn upload_staging_debug_redacts_private_names_captions_and_mxc() {
    let item = staged_file("stage-private", "!private-room:example.invalid", 1);
    let debug = format!("{item:?}");
    assert!(debug.contains("StagedUploadItem"), "{debug}");
    assert!(debug.contains("stage-private"), "{debug}");
    assert!(!debug.contains("stage-private.txt"), "{debug}");
    assert!(!debug.contains("private caption"), "{debug}");
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");

    let gallery = gallery_item("$media", "!private-room:example.invalid", 1_900_000_000_000);
    let debug = format!("{gallery:?}");
    assert!(debug.contains("TimelineMediaGalleryItem"), "{debug}");
    assert!(debug.contains("$media"), "{debug}");
    assert!(!debug.contains("private-image.png"), "{debug}");
    assert!(
        !debug.contains("mxc://example.invalid/private-image"),
        "{debug}"
    );
    assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
}

#[test]
fn upload_staging_store_retain_rooms_drops_orphaned_private_staging() {
    let mut store = UploadStagingStore::default();
    store.replace_room_items("room-a", vec![staged_file("stage-a", "room-a", 1)]);
    store.replace_room_items("room-b", vec![staged_file("stage-b", "room-b", 1)]);

    store.retain_rooms(&["room-a".to_owned()].into_iter().collect());

    assert_eq!(store.items_for_room("room-a").len(), 1);
    assert!(store.items_for_room("room-b").is_empty());
}

#[test]
fn upload_staging_isolated_by_main_and_thread_composer_target() {
    let main = ComposerTarget::Main {
        room_id: "room-a".to_owned(),
    };
    let thread = ComposerTarget::Thread {
        room_id: "room-a".to_owned(),
        root_event_id: "$root".to_owned(),
    };
    let mut store = UploadStagingStore::default();

    store.replace_target_items(main.clone(), vec![staged_file("shared", "room-a", 1)]);
    store.replace_target_items(thread.clone(), vec![staged_file("shared", "room-a", 1)]);

    assert_eq!(store.items_for_target(&main).len(), 1);
    assert_eq!(store.items_for_target(&thread).len(), 1);
    assert!(store.clear_target(&thread));
    assert_eq!(store.items_for_target(&main).len(), 1);
    assert!(store.items_for_target(&thread).is_empty());
}

#[test]
fn stale_target_cannot_change_another_composer_staged_item() {
    let mut state = selected_room_state("room-a");
    let main = ComposerTarget::Main {
        room_id: "room-a".to_owned(),
    };
    let stale_thread = ComposerTarget::Thread {
        room_id: "room-a".to_owned(),
        root_event_id: "$closed".to_owned(),
    };
    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: main,
            items: vec![staged_file("stage-1", "room-a", 1)],
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::UploadStagingCaptionChanged {
            target: stale_thread,
            staged_id: "stage-1".to_owned(),
            caption: Some(caption("must not cross targets")),
        },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.timeline.staged_uploads[0]
            .caption
            .as_ref()
            .expect("caption")
            .plain_body,
        "private caption"
    );
}

#[test]
fn open_thread_projects_only_its_staging_and_close_clears_it() {
    let mut state = selected_room_state("room-a");
    reduce(
        &mut state,
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::ThreadSubscribed {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
        },
    );
    let target = ComposerTarget::Thread {
        room_id: "room-a".to_owned(),
        root_event_id: "$root".to_owned(),
    };

    reduce(
        &mut state,
        AppAction::UploadStagingChanged {
            target: target.clone(),
            items: vec![staged_file("thread-stage", "room-a", 1)],
        },
    );

    let koushi_state::ThreadPaneState::Open { staged_uploads, .. } = &state.thread else {
        panic!("thread should be open");
    };
    assert_eq!(staged_uploads[0].staged_id, "thread-stage");
    assert!(state.timeline.staged_uploads.is_empty());

    reduce(&mut state, AppAction::CloseThread);
    assert!(state.upload_staging.items_for_target(&target).is_empty());
}
