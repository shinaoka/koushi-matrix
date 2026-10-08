mod support;

use std::time::Duration;

use koushi_core::CoreRuntime;
use koushi_core::media_preparation::StageUploadBytesInput;
use koushi_core::runtime::IsolatedStores;
use koushi_state::{
    AppAction, ComposerTarget, RoomSummary, StagedUploadFormatChoice, StagedUploadOutputSelection,
    StagedUploadResizeChoice,
};

const ROOM_ID: &str = "!media-staging-b2:example.invalid";

fn target() -> ComposerTarget {
    ComposerTarget::Main {
        room_id: ROOM_ID.to_owned(),
    }
}

fn image(id: &str) -> StageUploadBytesInput {
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position: 1,
        filename: "fixture.png".to_owned(),
        mime_type: "image/png".to_owned(),
        bytes: vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ],
    }
}

async fn ready_runtime() -> (CoreRuntime, koushi_core::CoreConnection, IsolatedStores) {
    let (runtime, _stores) = CoreRuntime::start_isolated_with_event_capacity(64);
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
    (runtime, connection, _stores)
}

#[tokio::test]
async fn prepared_preview_is_core_owned_and_target_fenced() {
    let (runtime, mut connection, _stores) = ready_runtime().await;
    let staged = connection
        .stage_upload_bytes(target(), vec![image("preview")], None)
        .await
        .expect("image should stage");
    let staged_generation = staged;
    let staged = connection.versioned_snapshot();
    assert_eq!(staged.generation, staged_generation);
    let variant_id = match &staged.state.timeline.staged_uploads[0].preparation {
        koushi_state::StagedUploadPreparation::Ready { variants, .. } => {
            variants[0].variant_id.clone()
        }
        preparation => panic!("unexpected preparation: {preparation:?}"),
    };
    let bytes = connection
        .prepared_upload_preview(target(), "preview".to_owned(), variant_id)
        .await
        .expect("preview bytes should be available");
    assert!(!bytes.is_empty());

    let inactive = connection
        .prepared_upload_preview(
            ComposerTarget::Main {
                room_id: "!other:example.invalid".to_owned(),
            },
            "preview".to_owned(),
            "original".to_owned(),
        )
        .await;
    assert!(inactive.is_err());
    drop(runtime);
}

#[tokio::test]
async fn same_target_preparation_admission_is_serialized() {
    let (runtime, mut connection, _stores) = ready_runtime().await;
    connection
        .stage_upload_bytes(target(), vec![image("race")], None)
        .await
        .expect("image should stage");
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
                StagedUploadOutputSelection {
                    resize: StagedUploadResizeChoice::Half,
                    format: StagedUploadFormatChoice::Jpeg,
                },
            )
            .await
    });
    barrier.wait_started().await;

    let service = runtime.media_staging().clone();
    let mut second_connection = runtime.attach();
    let mut second = tokio::spawn(async move {
        service
            .select_staged_upload_output(
                &mut second_connection,
                target(),
                "race".to_owned(),
                StagedUploadOutputSelection {
                    resize: StagedUploadResizeChoice::Quarter,
                    format: StagedUploadFormatChoice::Webp,
                },
            )
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut second)
            .await
            .is_err()
    );
    barrier.release();
    first.await.expect("first task").expect("first selection");
    second
        .await
        .expect("second task")
        .expect("second selection");
    drop(runtime);
}

#[tokio::test]
async fn prepared_send_rejects_before_upload_when_account_or_target_fence_fails() {
    let (runtime, mut connection, _stores) = ready_runtime().await;
    let generation = connection
        .begin_composer_draft_renderer_generation()
        .expect("renderer generation");
    let account = koushi_protocol::SessionKeyId {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    };
    let lease = connection
        .acquire_composer_draft_lease(
            generation,
            koushi_core::composer_draft_lifecycle::ComposerDraftScope {
                account: account.clone(),
                target: target(),
            },
        )
        .expect("composer lease");
    let result = connection
        .send_prepared_uploads(
            account,
            generation,
            lease,
            target(),
            koushi_state::ComposerDraftRevision::default(),
            None,
        )
        .await;
    assert!(result.is_err());
    drop(runtime);
}

/// The repository's licensed opaque HEIC fixture.
const HEIF_FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../koushi-media/tests/fixtures/heif/opaque.heic"
));

/// #1147: the preview surface must render an Original+Keep HEIF selection
/// without WebView HEIC support, while the selection itself keeps the exact
/// source bytes for the upload.
#[tokio::test]
async fn heif_original_preview_is_not_the_raw_heic_payload() {
    let (runtime, mut connection, _stores) = ready_runtime().await;
    connection
        .stage_upload_bytes(
            target(),
            vec![StageUploadBytesInput {
                staged_id: "heif-preview".to_owned(),
                position: 1,
                filename: "camera.HEIC".to_owned(),
                mime_type: String::new(),
                bytes: HEIF_FIXTURE.to_vec(),
            }],
            None,
        )
        .await
        .expect("HEIC staging should settle");
    connection
        .select_staged_upload_output(
            target(),
            "heif-preview".to_owned(),
            StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Original,
                format: StagedUploadFormatChoice::Keep,
            },
        )
        .await
        .expect("Original+Keep should be selectable");

    let snapshot = connection.versioned_snapshot();
    let item = snapshot
        .state
        .timeline
        .staged_uploads
        .iter()
        .find(|item| item.staged_id == "heif-preview")
        .expect("staged HEIC");
    // The selection still describes the exact original payload.
    assert_eq!(item.mime_type, "image/heic");
    assert_eq!(item.byte_count, HEIF_FIXTURE.len() as u64);
    let koushi_state::StagedUploadPreparation::Ready { variants, .. } = &item.preparation else {
        panic!("HEIC should be ready");
    };
    let original_keep = variants
        .iter()
        .find(|variant| {
            variant.resize == StagedUploadResizeChoice::Original
                && variant.format_choice == StagedUploadFormatChoice::Keep
        })
        .expect("Original+Keep variant")
        .variant_id
        .clone();

    let preview = connection
        .prepared_upload_preview(target(), "heif-preview".to_owned(), original_keep)
        .await
        .expect("preview bytes should be available");
    assert_ne!(
        preview, HEIF_FIXTURE,
        "the preview must not hand the raw HEIC payload to the WebView"
    );
    assert_eq!(
        preview[..3],
        [0xFF, 0xD8, 0xFF],
        "the preview must be a renderable image"
    );
    drop(runtime);
}
