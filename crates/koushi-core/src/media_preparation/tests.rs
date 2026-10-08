use super::*;
use image::GenericImageView;

fn target(root: Option<&str>) -> ComposerTarget {
    match root {
        Some(root_event_id) => ComposerTarget::Thread {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
        None => ComposerTarget::Main {
            room_id: "!room:example.invalid".to_owned(),
        },
    }
}

fn file(id: &str, bytes: &[u8]) -> StageUploadBytesInput {
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position: 1,
        filename: "private.pdf".to_owned(),
        mime_type: "application/pdf".to_owned(),
        bytes: bytes.to_vec(),
    }
}

/// Decodable PNG fixture for output-preparation tests.
fn png_input(id: &str, width: u32, height: u32) -> StageUploadBytesInput {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    let image = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 251) as u8, (y % 239) as u8, 120, 255])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode fixture");
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position: 1,
        filename: "shot.png".to_owned(),
        mime_type: "image/png".to_owned(),
        bytes: bytes.into_inner(),
    }
}

fn heif_input(id: &str) -> StageUploadBytesInput {
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position: 1,
        filename: "camera.png".to_owned(),
        mime_type: "image/png".to_owned(),
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../koushi-media/tests/fixtures/heif/opaque.heic"
        ))
        .to_vec(),
    }
}

#[test]
fn heif_staging_defaults_to_jpeg_but_retains_exact_original_bytes() {
    let target = target(None);
    let mut registry = MediaPreparationRegistry::default();
    let source = heif_input("heif-1");
    let original_bytes = source.bytes.clone();
    let item = registry
        .prepare_items(
            &target,
            vec![source],
            ImageUploadCompressionPolicy::default(),
        )
        .pop()
        .expect("one staged HEIF image");

    assert!(matches!(
        item.kind,
        StagedUploadKind::Image {
            width: Some(64),
            height: Some(64)
        }
    ));
    let StagedUploadPreparation::Ready {
        variants, selected, ..
    } = &item.preparation
    else {
        panic!("HEIF should expose converted image choices");
    };
    assert_eq!(
        *selected,
        StagedUploadOutputSelection {
            resize: StagedUploadResizeChoice::Original,
            format: StagedUploadFormatChoice::Jpeg
        }
    );
    assert!(variants.iter().any(|variant| {
        variant.resize == StagedUploadResizeChoice::Original
            && variant.format_choice == StagedUploadFormatChoice::Keep
            && variant.mime_type == "image/heic"
    }));

    let converted = registry
        .selected_upload(&target, "heif-1")
        .expect("default converted bytes");
    assert_eq!(converted.descriptor.mime_type, "image/jpeg");
    assert_eq!(converted.descriptor.width, Some(64));
    assert_eq!(converted.descriptor.height, Some(64));
    assert_eq!(
        converted.bytes.len() as u64,
        converted.descriptor.byte_count
    );
    assert_eq!(
        image::load_from_memory(&converted.bytes)
            .unwrap()
            .dimensions(),
        (64, 64)
    );

    let source = registry
        .source_input(&target, "heif-1")
        .expect("source bytes retained for lazy output");
    let (resized_keep, resized_bytes) = MediaPreparationRegistry::encode_output(
        &source,
        StagedUploadOutputSelection {
            resize: StagedUploadResizeChoice::Half,
            format: StagedUploadFormatChoice::Keep,
        },
        ImageUploadCompressionPolicy::default(),
        None,
    )
    .expect("resized HEIF Keep should use the compatible JPEG path");
    assert_eq!(resized_keep.variant_id, "half-keep");
    assert_eq!(resized_keep.mime_type, "image/jpeg");
    assert_eq!(resized_keep.width, Some(32));
    assert_eq!(resized_keep.height, Some(32));
    assert_eq!(resized_keep.byte_count, resized_bytes.len() as u64);

    let original = registry
        .use_original(&target, "heif-1")
        .expect("the exact source remains selectable");
    assert_eq!(original.mime_type, "image/heic");
    assert_eq!(
        registry
            .selected_upload(&target, "heif-1")
            .expect("original bytes")
            .bytes,
        original_bytes
    );
}

/// #305 regression guard: after a lazily encoded pair is selected, the send
/// path must still resolve prepared bytes. `send_prepared_uploads` fails
/// outright when `selected_upload` returns `None`.
#[test]
fn selecting_a_lazily_encoded_output_keeps_prepared_upload_bytes_available() {
    let target = target(None);
    let mut registry = MediaPreparationRegistry::default();
    let staged = registry.prepare_items(
        &target,
        vec![png_input("staged-1", 64, 32)],
        ImageUploadCompressionPolicy::default(),
    );
    let item = staged.first().expect("one staged image").clone();
    assert!(
        registry.selected_upload(&target, "staged-1").is_some(),
        "staging must leave prepared bytes available"
    );

    let selection = StagedUploadOutputSelection {
        resize: StagedUploadResizeChoice::Half,
        format: StagedUploadFormatChoice::Keep,
    };
    let variant_id = MediaPreparationRegistry::output_identity(selection);
    assert!(
        !registry.select_variant(&target, "staged-1", &variant_id),
        "the newly requested pair must not be cached yet"
    );

    let source = registry
        .source_input(&target, "staged-1")
        .expect("the source is retained for lazy encoding");
    let (descriptor, bytes) = MediaPreparationRegistry::encode_output(
        &source,
        selection,
        ImageUploadCompressionPolicy::default(),
        None,
    )
    .expect("the requested pair must encode");
    registry.insert_prepared_output(&target, "staged-1", descriptor.clone(), bytes);

    let prepared = registry
        .selected_upload(&target, "staged-1")
        .expect("send must still resolve prepared bytes after a lazy encode");
    assert_eq!(prepared.descriptor.variant_id, variant_id);
    assert_eq!(prepared.descriptor.width, Some(32));
    assert_eq!(prepared.descriptor.height, Some(16));
    assert_eq!(
        prepared.bytes.len() as u64,
        prepared.descriptor.byte_count,
        "the reported byte count must describe the bytes that upload"
    );

    // Drive the same order production uses: the selection reaches state
    // first, then the encode is adopted under the generation state handed
    // out at selection time.
    let mut store = koushi_state::UploadStagingStore::default();
    store
        .items
        .insert((target.clone(), "staged-1".to_owned()), item);
    let selected_item = store
        .select_output(&target, "staged-1", selection)
        .expect("selecting an unprepared pair must be accepted");
    let (pending, generation) = match &selected_item.preparation {
        StagedUploadPreparation::Ready {
            pending,
            generation,
            ..
        } => (*pending, *generation),
        other => panic!("staged image must stay ready, got {other:?}"),
    };
    assert_eq!(
        pending,
        Some(selection),
        "an unprepared pair must be reported as pending"
    );

    let adopted = store
        .complete_output(&target, "staged-1", descriptor, generation)
        .expect("the completed output must be adopted");
    assert_eq!(
        adopted.byte_count, prepared.descriptor.byte_count,
        "state and registry must describe the same output"
    );
    match &adopted.preparation {
        StagedUploadPreparation::Ready {
            pending, variants, ..
        } => {
            assert!(pending.is_none(), "adopting the output must clear pending");
            assert_eq!(variants.len(), 2, "both outputs stay cached for reuse");
        }
        other => panic!("staged image must stay ready, got {other:?}"),
    }
}

#[test]
fn registry_isolates_equal_ids_by_target_and_clears_bytes() {
    let mut registry = MediaPreparationRegistry::default();
    let main = target(None);
    let thread = target(Some("$root"));
    let policy = ImageUploadCompressionPolicy::default();
    registry.prepare_target(&main, vec![file("shared", b"main")], policy);
    registry.prepare_target(&thread, vec![file("shared", b"thread")], policy);

    let retained = registry.stats();
    assert_eq!(retained.source_count, 2);
    assert_eq!(retained.source_bytes, b"main".len() + b"thread".len());
    assert_eq!(retained.variant_count, 2);
    assert_eq!(retained.source_backed_variant_count, 2);
    assert_eq!(retained.variant_bytes, 0);

    assert_eq!(
        registry.selected_upload(&main, "shared").unwrap().bytes,
        b"main"
    );
    assert_eq!(
        registry.selected_upload(&thread, "shared").unwrap().bytes,
        b"thread"
    );
    assert_eq!(
        registry.variant_bytes(&main, "shared", "original").unwrap(),
        b"main"
    );
    registry.clear_target(&thread);
    assert!(registry.selected_upload(&thread, "shared").is_none());
    assert_eq!(
        registry.selected_upload(&main, "shared").unwrap().bytes,
        b"main"
    );
    let after_thread_clear = registry.stats();
    assert_eq!(after_thread_clear.source_count, 1);
    assert_eq!(after_thread_clear.variant_count, 1);
    registry.remove_item(&main, "shared");
    let after_remove = registry.stats();
    assert_eq!(after_remove.source_count, 0);
    assert_eq!(after_remove.source_bytes, 0);
    assert_eq!(after_remove.variant_count, 0);
    assert_eq!(after_remove.variant_bytes, 0);
    assert_eq!(after_remove.high_water_source_count, 2);
    assert_eq!(after_remove.high_water_variant_count, 2);
}

#[test]
fn clear_releases_retained_media_and_keeps_private_free_high_water_stats() {
    let mut registry = MediaPreparationRegistry::default();
    let target = target(None);
    registry.prepare_target(
        &target,
        vec![file("private-stage", b"private bytes")],
        ImageUploadCompressionPolicy::default(),
    );

    registry.clear();

    let stats = registry.stats();
    assert_eq!(stats.source_count, 0);
    assert_eq!(stats.source_bytes, 0);
    assert_eq!(stats.variant_count, 0);
    assert_eq!(stats.variant_bytes, 0);
    assert!(stats.high_water_source_bytes >= b"private bytes".len());
    assert_eq!(stats.high_water_variant_bytes, 0);
    let debug = format!("{registry:?}");
    assert!(!debug.contains("private-stage"));
    assert!(!debug.contains("private bytes"));
}

#[test]
fn original_image_and_heif_variants_reuse_retained_source_bytes() {
    let target = target(None);
    let mut registry = MediaPreparationRegistry::default();
    let png = png_input("png-source", 8, 4);
    let png_bytes = png.bytes.clone();
    let heif = heif_input("heif-source");
    let heif_bytes = heif.bytes.clone();

    registry.prepare_items(
        &target,
        vec![png, heif],
        ImageUploadCompressionPolicy::default(),
    );

    let heif_converted_bytes = registry
        .selected_upload(&target, "heif-source")
        .expect("HEIF JPEG conversion")
        .bytes
        .len();
    let stats = registry.stats();
    assert_eq!(stats.source_backed_variant_count, 2);
    assert_eq!(stats.variant_bytes, heif_converted_bytes);
    assert_eq!(
        registry
            .variant_bytes(&target, "png-source", "original-keep")
            .expect("PNG original bytes"),
        png_bytes
    );
    assert_eq!(
        registry
            .variant_bytes(&target, "heif-source", "original-keep")
            .expect("HEIF original bytes"),
        heif_bytes
    );
}

#[test]
fn empty_input_is_a_typed_failure_and_debug_is_private() {
    let mut registry = MediaPreparationRegistry::default();
    let target = target(None);
    let items = registry.prepare_target(
        &target,
        vec![file("private-stage", b"")],
        ImageUploadCompressionPolicy::default(),
    );
    assert!(matches!(
        items[0].preparation,
        StagedUploadPreparation::Failed {
            failure_kind: MediaPreparationFailureKind::Empty,
            ..
        }
    ));
    let debug = format!("{:?}", file("private-stage", b"private bytes"));
    assert!(!debug.contains("private.pdf"));
    assert!(!debug.contains("private bytes"));
}

#[test]
fn failed_item_can_promote_its_captured_nonempty_source_to_original() {
    let mut registry = MediaPreparationRegistry::default();
    let target = target(Some("$root"));
    registry.sources.insert(
        (target.clone(), "failed".to_owned()),
        StageUploadBytesInput {
            staged_id: "failed".to_owned(),
            position: 2,
            filename: "private.bin".to_owned(),
            mime_type: "application/octet-stream".to_owned(),
            bytes: b"captured source".to_vec(),
        },
    );

    let item = registry
        .use_original(&target, "failed")
        .expect("captured original should be selectable");
    assert!(matches!(
        item.preparation,
        StagedUploadPreparation::Ready { .. }
    ));
    assert_eq!(
        registry.selected_upload(&target, "failed").unwrap().bytes,
        b"captured source"
    );
}

#[test]
fn snapshot_reconcile_retains_only_items_backed_by_staging_state() {
    let mut registry = MediaPreparationRegistry::default();
    let main = target(None);
    let thread = target(Some("$root"));
    let stale_thread = target(Some("$stale"));
    let policy = ImageUploadCompressionPolicy::default();
    let mut staged = Vec::new();
    for (target, id) in [
        (&main, "main"),
        (&thread, "thread"),
        (&stale_thread, "stale"),
    ] {
        let items = registry.prepare_target(target, vec![file(id, id.as_bytes())], policy);
        staged.push(((*target).clone(), items[0].clone()));
    }
    let mut snapshot = koushi_state::AppState::default();
    for (target, item) in staged.into_iter().take(2) {
        snapshot
            .upload_staging
            .items
            .insert((target, item.staged_id.clone()), item);
    }

    registry.reconcile_snapshot(&snapshot);

    assert!(registry.selected_upload(&main, "main").is_some());
    assert!(registry.selected_upload(&thread, "thread").is_some());
    assert!(registry.selected_upload(&stale_thread, "stale").is_none());
    registry.clear_thread_targets();
    assert!(registry.selected_upload(&thread, "thread").is_none());
    assert!(registry.selected_upload(&main, "main").is_some());
    let stats = registry.stats();
    assert_eq!(stats.source_count, 1);
    assert_eq!(stats.variant_count, 1);
}

#[test]
fn account_change_clears_bytes_even_when_room_ids_match() {
    let mut registry = MediaPreparationRegistry::default();
    let target = target(None);
    let mut snapshot = koushi_state::AppState::default();
    snapshot.timeline.room_id = Some("!room:example.invalid".to_owned());
    snapshot.session = koushi_state::SessionState::Ready(koushi_state::SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@first:example.invalid".to_owned(),
        device_id: "FIRST".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    });
    registry.reconcile_snapshot(&snapshot);
    let items = registry.prepare_target(
        &target,
        vec![file("private", b"first account")],
        ImageUploadCompressionPolicy::default(),
    );
    snapshot
        .upload_staging
        .items
        .insert((target.clone(), "private".to_owned()), items[0].clone());
    assert!(registry.selected_upload(&target, "private").is_some());

    snapshot.session = koushi_state::SessionState::SwitchingAccount {
        info: koushi_state::SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@second:example.invalid".to_owned(),
            device_id: "SECOND".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    };
    registry.reconcile_snapshot(&snapshot);
    assert!(registry.selected_upload(&target, "private").is_some());

    snapshot.session = koushi_state::SessionState::Ready(koushi_state::SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@second:example.invalid".to_owned(),
        device_id: "SECOND".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    });
    registry.reconcile_snapshot(&snapshot);

    assert!(registry.selected_upload(&target, "private").is_none());
    let stats = registry.stats();
    assert_eq!(stats.source_count, 0);
    assert_eq!(stats.variant_count, 0);
}

#[test]
fn detached_batch_merge_preserves_an_overlapping_stage_result() {
    let target = target(None);
    let policy = ImageUploadCompressionPolicy::default();
    let mut committed = MediaPreparationRegistry::default();
    committed.prepare_items(&target, vec![file("first", b"first")], policy);
    let mut later = MediaPreparationRegistry::default();
    later.prepare_items(&target, vec![file("second", b"second")], policy);

    committed.merge_prepared(later);

    assert_eq!(
        committed.selected_upload(&target, "first").unwrap().bytes,
        b"first"
    );
    assert_eq!(
        committed.selected_upload(&target, "second").unwrap().bytes,
        b"second"
    );
}

/// #1147: an Original+Keep HEIF selection uploads the exact source bytes while
/// its preview renders the converted output the same item already caches, so the
/// WebView never has to decode raw HEIC.
#[test]
fn heif_original_preview_renders_converted_bytes_while_upload_stays_exact() {
    let target = target(None);
    let mut registry = MediaPreparationRegistry::default();
    let source = heif_input("heif-preview");
    let original_bytes = source.bytes.clone();
    let item = registry
        .prepare_items(
            &target,
            vec![source],
            ImageUploadCompressionPolicy::default(),
        )
        .pop()
        .expect("one staged HEIF image");
    let StagedUploadPreparation::Ready { variants, .. } = &item.preparation else {
        panic!("HEIF should expose converted image choices");
    };
    let original_keep = variants
        .iter()
        .find(|variant| {
            variant.resize == StagedUploadResizeChoice::Original
                && variant.format_choice == StagedUploadFormatChoice::Keep
        })
        .expect("Original+Keep variant");
    let converted_jpeg = variants
        .iter()
        .find(|variant| {
            variant.resize == StagedUploadResizeChoice::Original
                && variant.format_choice == StagedUploadFormatChoice::Jpeg
        })
        .expect("converted default variant");

    // Upload keeps the exact HEIC payload.
    assert_eq!(
        registry.variant_bytes(&target, "heif-preview", &original_keep.variant_id),
        Some(original_bytes.clone())
    );
    // The preview does not, and is a renderable image: the converted JPEG.
    let preview = registry
        .variant_preview_bytes(&target, "heif-preview", &original_keep.variant_id)
        .expect("original preview bytes");
    assert_ne!(preview, original_bytes);
    assert_eq!(preview[..3], [0xFF, 0xD8, 0xFF]);
    assert_eq!(
        preview,
        registry
            .variant_bytes(&target, "heif-preview", &converted_jpeg.variant_id)
            .expect("converted upload bytes")
    );
    // A decodable variant previews its own upload bytes.
    assert_eq!(
        registry.variant_preview_bytes(&target, "heif-preview", &converted_jpeg.variant_id),
        registry.variant_bytes(&target, "heif-preview", &converted_jpeg.variant_id)
    );

    // The preview is a real image, not merely bytes that are not HEIC.
    let decoded = image::load_from_memory(&preview).expect("preview decodes");
    assert_eq!((decoded.width(), decoded.height()), (64, 64));

    // And the selection the send path resolves still carries the exact source.
    assert!(registry.select_variant(&target, "heif-preview", &original_keep.variant_id));
    assert_eq!(
        registry
            .selected_upload(&target, "heif-preview")
            .expect("Original+Keep is selected")
            .bytes,
        original_bytes
    );
}
