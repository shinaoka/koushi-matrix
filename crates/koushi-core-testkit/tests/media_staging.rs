mod support;

use koushi_core::CoreRuntime;
use koushi_core::media_preparation::StageUploadBytesInput;
use koushi_core::media_staging::{
    MAX_MEDIA_STAGING_BATCH_BYTES, MAX_MEDIA_STAGING_BATCH_SIZE, MediaStagingError,
};
use koushi_state::{
    AppAction, ComposerDocument, ComposerInline, ComposerTarget, ImageUploadCompressionMode,
    RoomSummary, StagedUploadCompressionChoice, StagedUploadFormatChoice, StagedUploadPreparation,
    StagedUploadResizeChoice,
};

const ROOM_ID: &str = "!media-staging:example.invalid";

fn target() -> ComposerTarget {
    ComposerTarget::Main {
        room_id: ROOM_ID.to_owned(),
    }
}

fn item(id: &str, bytes: &[u8]) -> StageUploadBytesInput {
    item_at(id, 1, bytes)
}

fn item_at(id: &str, position: u64, bytes: &[u8]) -> StageUploadBytesInput {
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position,
        filename: "fixture.bin".to_owned(),
        mime_type: " application/octet-stream ".to_owned(),
        bytes: bytes.to_vec(),
    }
}

fn png_item(id: &str, position: u64) -> StageUploadBytesInput {
    // Synthetic 1x1 PNG; the bytes are test-only and contain no user data.
    let bytes = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position,
        filename: "fixture.png".to_owned(),
        mime_type: "image/png".to_owned(),
        bytes,
    }
}

async fn ready_runtime() -> (CoreRuntime, koushi_core::CoreConnection) {
    let runtime = CoreRuntime::start_with_event_capacity(64);
    let mut connection = runtime.attach();
    let mut actions = support::restore_ready_actions();
    actions.extend([
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![RoomSummary {
                room_id: ROOM_ID.to_owned(),
                ..support::room_summary(ROOM_ID)
            }],
        },
        AppAction::SelectRoom {
            room_id: ROOM_ID.to_owned(),
        },
    ]);
    runtime.inject_actions(actions).await;
    support::wait_for_state_event(&mut connection, |state| {
        state.timeline.room_id.as_deref() == Some(ROOM_ID)
    })
    .await;
    (runtime, connection)
}

#[test]
fn media_staging_limits_are_named_and_checked_before_preparation() {
    assert_eq!(MAX_MEDIA_STAGING_BATCH_BYTES, 128 * 1024 * 1024);
    assert_eq!(MAX_MEDIA_STAGING_BATCH_SIZE, 16);
}

#[tokio::test]
async fn staging_publishes_preparing_then_ready_and_normalizes_mime() {
    let (_runtime, mut connection) = ready_runtime().await;
    let before = connection.versioned_snapshot();
    let snapshot = connection
        .stage_upload_bytes(target(), vec![item("one", b"bytes")])
        .await
        .expect("staging should settle");
    let snapshot_generation = snapshot;
    let snapshot = connection.versioned_snapshot();
    assert_eq!(snapshot.generation, snapshot_generation);
    assert!(snapshot.generation > before.generation);
    let staged = &snapshot.state.timeline.staged_uploads;
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].mime_type, "application/octet-stream");
    assert!(matches!(
        staged[0].preparation,
        koushi_state::StagedUploadPreparation::Ready { .. }
    ));
}

#[tokio::test]
async fn duplicate_ids_and_overflow_are_rejected_without_state_change() {
    let (runtime, mut connection) = ready_runtime().await;
    let before = connection.versioned_snapshot();
    let duplicate = runtime.media_staging().stage_upload_bytes(
        &mut connection,
        target(),
        vec![item("same", b"a"), item_at("same", 2, b"b")],
    );
    assert!(matches!(
        duplicate.await,
        Err(MediaStagingError::DuplicateStagedId)
    ));
    let too_many = (0..=MAX_MEDIA_STAGING_BATCH_SIZE)
        .map(|index| item(&index.to_string(), b"x"))
        .collect();
    assert!(matches!(
        runtime
            .media_staging()
            .stage_upload_bytes(&mut connection, target(), too_many,)
            .await,
        Err(MediaStagingError::BatchTooLarge)
    ));
    assert_eq!(connection.versioned_snapshot(), before);
}

#[tokio::test]
async fn caption_survives_preparation_and_replacement() {
    let (runtime, mut connection) = ready_runtime().await;
    let staged = runtime
        .media_staging()
        .stage_upload_bytes(&mut connection, target(), vec![item("one", b"bytes")])
        .await
        .expect("staging should settle");
    let staged_generation = staged;
    let staged = connection.versioned_snapshot();
    assert_eq!(staged.generation, staged_generation);
    let caption = ComposerDocument::new(vec![ComposerInline::Text {
        text: "synthetic caption".to_owned(),
    }]);
    let captioned = runtime
        .media_staging()
        .update_caption(
            &mut connection,
            target(),
            "one".to_owned(),
            Some(caption.clone()),
        )
        .await
        .expect("caption should settle");
    let captioned_generation = captioned;
    let captioned = connection.versioned_snapshot();
    assert_eq!(captioned.generation, captioned_generation);
    assert_eq!(
        captioned.state.timeline.staged_uploads[0].caption,
        Some(caption)
    );
    assert!(captioned.generation > staged.generation);
}

#[tokio::test]
async fn empty_preparation_is_a_typed_failure_and_clear_releases_bytes() {
    let (runtime, mut connection) = ready_runtime().await;
    let snapshot = runtime
        .media_staging()
        .stage_upload_bytes(&mut connection, target(), vec![item("empty", b"")])
        .await
        .expect("empty input settles as a failed item");
    let snapshot_generation = snapshot;
    let snapshot = connection.versioned_snapshot();
    assert_eq!(snapshot.generation, snapshot_generation);
    assert!(matches!(
        snapshot.state.timeline.staged_uploads[0].preparation,
        koushi_state::StagedUploadPreparation::Failed { .. }
    ));
    let cleared_generation = runtime
        .media_staging()
        .clear(&mut connection, target())
        .await
        .expect("clear should settle");
    assert_eq!(connection.state_generation(), cleared_generation);
    let stats = runtime.media_preparation().stats().await;
    assert_eq!(stats.source_count, 0);
    assert_eq!(stats.variant_count, 0);
    assert_eq!(stats.source_bytes, 0);
    assert_eq!(stats.variant_bytes, 0);
}

#[tokio::test]
async fn select_retry_original_and_compression_are_targeted_operations() {
    let (runtime, mut connection) = ready_runtime().await;
    runtime
        .media_staging()
        .stage_upload_bytes(&mut connection, target(), vec![png_item("image", 1)])
        .await
        .expect("image staging should settle");

    let selected = connection
        .select_staged_upload_output(
            target(),
            "image".to_owned(),
            koushi_state::StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Half,
                format: StagedUploadFormatChoice::Jpeg,
            },
        )
        .await
        .expect("selection should prepare and settle");
    let selected_generation = selected;
    let selected = connection.versioned_snapshot();
    assert_eq!(selected.generation, selected_generation);
    let selected_item = &selected.state.timeline.staged_uploads[0];
    assert!(matches!(
        selected_item.preparation,
        StagedUploadPreparation::Ready { pending: None, .. }
    ));
    assert_eq!(selected_item.mime_type, "image/jpeg");

    let compressed = connection
        .update_staged_upload_compression(
            target(),
            "image".to_owned(),
            StagedUploadCompressionChoice::Compressed {
                mode: ImageUploadCompressionMode::Always,
            },
        )
        .await
        .expect("compression choice should settle");
    let compressed_generation = compressed;
    let compressed = connection.versioned_snapshot();
    assert_eq!(compressed.generation, compressed_generation);
    assert_eq!(
        compressed.state.timeline.staged_uploads[0].compression_choice,
        StagedUploadCompressionChoice::Compressed {
            mode: ImageUploadCompressionMode::Always,
        }
    );

    let original = connection
        .use_original_staged_upload(target(), "image".to_owned())
        .await
        .expect("original adoption should settle");
    let original_generation = original;
    let original = connection.versioned_snapshot();
    assert_eq!(original.generation, original_generation);
    assert_eq!(
        original.state.timeline.staged_uploads[0].mime_type,
        "image/png"
    );

    let failed = connection
        .stage_upload_bytes(target(), vec![item_at("failed", 2, b"")])
        .await;
    assert!(failed.is_ok());
    let retry = connection
        .retry_staged_upload_preparation(target(), "failed".to_owned())
        .await
        .expect("retry should settle even when the source remains invalid");
    let generation = retry;
    let retry = connection.versioned_snapshot();
    assert_eq!(retry.generation, generation);
    assert!(matches!(
        retry
            .state
            .timeline
            .staged_uploads
            .iter()
            .find(|item| item.staged_id == "failed")
            .unwrap()
            .preparation,
        StagedUploadPreparation::Failed { .. }
    ));
    assert!(matches!(
        connection
            .retry_staged_upload_preparation(target(), "missing".to_owned())
            .await,
        Err(MediaStagingError::MissingStagedItem)
    ));
}

#[tokio::test]
async fn thread_target_isolated_from_main_target() {
    let (runtime, mut connection) = ready_runtime().await;
    let root_event_id = "$root:example.invalid";
    runtime
        .inject_actions(vec![
            AppAction::OpenThread {
                room_id: ROOM_ID.to_owned(),
                root_event_id: root_event_id.to_owned(),
                intent: koushi_state::ThreadOpenIntent::NewThreadDraft,
            },
            AppAction::ThreadSubscribed {
                room_id: ROOM_ID.to_owned(),
                root_event_id: root_event_id.to_owned(),
            },
        ])
        .await;
    support::wait_for_state_event(&mut connection, |state| {
        matches!(
            &state.thread,
            koushi_state::ThreadPaneState::Open { root_event_id, .. }
                if root_event_id == "$root:example.invalid"
        )
    })
    .await;
    let thread = ComposerTarget::Thread {
        room_id: ROOM_ID.to_owned(),
        root_event_id: root_event_id.to_owned(),
    };
    let snapshot = connection
        .stage_upload_bytes(thread, vec![item("thread-item", b"thread")])
        .await
        .expect("thread staging should settle");
    let snapshot_generation = snapshot;
    let snapshot = connection.versioned_snapshot();
    assert_eq!(snapshot.generation, snapshot_generation);
    assert!(snapshot.state.timeline.staged_uploads.is_empty());
    assert!(matches!(
        snapshot.state.thread,
        koushi_state::ThreadPaneState::Open { ref staged_uploads, .. }
            if staged_uploads.iter().any(|item| item.staged_id == "thread-item")
    ));
}

#[tokio::test]
async fn positions_are_nonzero_unique_and_second_batches_settle_in_order() {
    let (runtime, mut connection) = ready_runtime().await;
    assert!(matches!(
        connection
            .stage_upload_bytes(target(), vec![item_at("zero", 0, b"x")])
            .await,
        Err(MediaStagingError::InvalidPosition)
    ));
    assert!(matches!(
        connection
            .stage_upload_bytes(target(), vec![item_at("a", 1, b"a"), item_at("b", 1, b"b")])
            .await,
        Err(MediaStagingError::InvalidPosition)
    ));

    runtime
        .media_staging()
        .stage_upload_bytes(&mut connection, target(), vec![item_at("one", 1, b"one")])
        .await
        .expect("first batch should settle");
    let second = connection
        .stage_upload_bytes(
            target(),
            vec![item_at("two", 2, b"two"), item_at("three", 3, b"three")],
        )
        .await
        .expect("second batch should settle");
    let second_generation = second;
    let second = connection.versioned_snapshot();
    assert_eq!(second.generation, second_generation);
    let items = &second.state.timeline.staged_uploads;
    assert_eq!(
        items
            .iter()
            .map(|item| item.staged_id.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
    assert_eq!(
        items.iter().map(|item| item.position).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[tokio::test]
async fn invalid_selection_is_immediate_and_missing_select_is_typed() {
    let (runtime, mut connection) = ready_runtime().await;
    runtime
        .media_staging()
        .stage_upload_bytes(&mut connection, target(), vec![item("file", b"file")])
        .await
        .expect("file staging should settle");
    let selection = koushi_state::StagedUploadOutputSelection {
        resize: StagedUploadResizeChoice::Half,
        format: StagedUploadFormatChoice::Jpeg,
    };
    assert!(matches!(
        connection
            .select_staged_upload_output(target(), "file".to_owned(), selection)
            .await,
        Err(MediaStagingError::InvalidSelection)
    ));
    assert!(matches!(
        connection
            .select_staged_upload_output(target(), "missing".to_owned(), selection)
            .await,
        Err(MediaStagingError::MissingStagedItem)
    ));
}

#[tokio::test]
async fn blocked_preparation_preserves_caption_and_releases_removed_bytes() {
    let (runtime, mut connection) = ready_runtime().await;
    let mut barrier = runtime
        .media_staging()
        .install_preparation_barrier_for_testing();
    let service = runtime.media_staging().clone();
    let mut staging_connection = runtime.attach();
    let task = tokio::spawn(async move {
        service
            .stage_upload_bytes(
                &mut staging_connection,
                target(),
                vec![item("captioned", b"bytes")],
            )
            .await
    });
    barrier.wait_started().await;
    let caption = ComposerDocument::new(vec![ComposerInline::Text {
        text: "caption while preparing".to_owned(),
    }]);
    runtime
        .inject_actions(vec![AppAction::UploadStagingCaptionChanged {
            target: target(),
            staged_id: "captioned".to_owned(),
            caption: Some(caption.clone()),
        }])
        .await;
    support::wait_for_state_event(&mut connection, |state| {
        state.timeline.staged_uploads[0].caption.as_ref() == Some(&caption)
    })
    .await;
    barrier.release();
    let settled = task.await.unwrap().expect("staging should settle");
    let settled_generation = settled;
    let settled = connection.versioned_snapshot();
    assert_eq!(settled.generation, settled_generation);
    assert_eq!(
        settled.state.timeline.staged_uploads[0].caption,
        Some(caption)
    );

    let mut remove_barrier = runtime
        .media_staging()
        .install_preparation_barrier_for_testing();
    let service = runtime.media_staging().clone();
    let mut staging_connection = runtime.attach();
    let task = tokio::spawn(async move {
        service
            .stage_upload_bytes(
                &mut staging_connection,
                target(),
                vec![item_at("removed", 2, b"bytes")],
            )
            .await
    });
    remove_barrier.wait_started().await;
    runtime
        .inject_actions(vec![AppAction::UploadStagingCleared { target: target() }])
        .await;
    support::wait_for_state_event(&mut connection, |state| {
        state.timeline.staged_uploads.is_empty()
    })
    .await;
    remove_barrier.release();
    assert!(matches!(task.await.unwrap(), Err(MediaStagingError::Stale)));
    let stats = runtime.media_preparation().stats().await;
    assert_eq!(
        (stats.source_count, stats.variant_count, stats.source_bytes),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn stale_account_and_replaced_target_do_not_publish_prepared_items() {
    let (runtime, mut connection) = ready_runtime().await;
    let mut barrier = runtime
        .media_staging()
        .install_preparation_barrier_for_testing();
    let service = runtime.media_staging().clone();
    let mut staging_connection = runtime.attach();
    let task = tokio::spawn(async move {
        service
            .stage_upload_bytes(
                &mut staging_connection,
                target(),
                vec![item("stale", b"bytes")],
            )
            .await
    });
    barrier.wait_started().await;
    runtime
        .inject_actions(vec![AppAction::LogoutRequested, AppAction::LogoutFinished])
        .await;
    support::wait_for_state_event(&mut connection, |state| {
        matches!(state.session, koushi_state::SessionState::SignedOut)
    })
    .await;
    barrier.release();
    assert!(matches!(task.await.unwrap(), Err(MediaStagingError::Stale)));
    assert_eq!(runtime.media_preparation().stats().await.source_count, 0);
}

#[tokio::test]
async fn selection_generation_race_is_latest_wins_and_stale_is_explicit() {
    let (runtime, mut connection) = ready_runtime().await;
    runtime
        .media_staging()
        .stage_upload_bytes(&mut connection, target(), vec![png_item("race", 1)])
        .await
        .expect("initial image should settle");
    let mut barrier = runtime
        .media_staging()
        .install_preparation_barrier_for_testing();
    let service = runtime.media_staging().clone();
    let mut first_connection = runtime.attach();
    let first = tokio::spawn(async move {
        service
            .select_staged_upload_output(
                &mut first_connection,
                target(),
                "race".to_owned(),
                koushi_state::StagedUploadOutputSelection {
                    resize: StagedUploadResizeChoice::Half,
                    format: StagedUploadFormatChoice::Jpeg,
                },
            )
            .await
    });
    barrier.wait_started().await;
    let latest_selection = koushi_state::StagedUploadOutputSelection {
        resize: StagedUploadResizeChoice::Quarter,
        format: StagedUploadFormatChoice::Webp,
    };
    runtime
        .inject_actions(vec![AppAction::UploadStagingOutputSelected {
            target: target(),
            staged_id: "race".to_owned(),
            selection: latest_selection,
        }])
        .await;
    support::wait_for_state_event(&mut connection, |state| {
        matches!(
            state.timeline.staged_uploads[0].preparation,
            StagedUploadPreparation::Ready {
                pending: Some(selection),
                ..
            } if selection == latest_selection
        )
    })
    .await;
    barrier.release();
    assert!(matches!(
        first.await.unwrap(),
        Err(MediaStagingError::Stale)
    ));
    assert!(matches!(
        connection.snapshot().timeline.staged_uploads[0].preparation,
        StagedUploadPreparation::Ready {
            pending: Some(selection),
            ..
        } if selection == latest_selection
    ));
}

#[tokio::test]
async fn settings_change_fences_blocked_preparation() {
    let (runtime, connection) = ready_runtime().await;
    let mut barrier = runtime
        .media_staging()
        .install_preparation_barrier_for_testing();
    let service = runtime.media_staging().clone();
    let mut staging_connection = runtime.attach();
    let task = tokio::spawn(async move {
        service
            .stage_upload_bytes(
                &mut staging_connection,
                target(),
                vec![item("policy", b"bytes")],
            )
            .await
    });
    barrier.wait_started().await;
    let mut settings = connection.snapshot().settings.values;
    settings
        .media
        .image_upload_compression_policy
        .target_long_edge += 1;
    runtime
        .inject_actions(vec![AppAction::SettingsLoaded { values: settings }])
        .await;
    barrier.release();
    assert!(matches!(task.await.unwrap(), Err(MediaStagingError::Stale)));
}
