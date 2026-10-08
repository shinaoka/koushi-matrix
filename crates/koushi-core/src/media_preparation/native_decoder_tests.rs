use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicUsize, Ordering},
};

use image::GenericImageView;

use super::*;
use crate::native_image_decoder::{
    DecodeLimits, DecodedRgbaImage, NativeImageDecodeError, NativeStillImageDecoder,
};

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../koushi-media/tests/fixtures/heif/opaque.heic"
));

/// Fake platform decoder. Its output dimensions deliberately differ from the
/// 64x64 fixture so every assertion proves which decoder produced the pixels.
struct FakeDecoder {
    result: Result<(u32, u32), NativeImageDecodeError>,
    calls: AtomicUsize,
    limits: StdMutex<Vec<DecodeLimits>>,
}

impl FakeDecoder {
    fn ready(width: u32, height: u32) -> Arc<Self> {
        Self::with_result(Ok((width, height)))
    }

    fn failing(error: NativeImageDecodeError) -> Arc<Self> {
        Self::with_result(Err(error))
    }

    fn with_result(result: Result<(u32, u32), NativeImageDecodeError>) -> Arc<Self> {
        Arc::new(Self {
            result,
            calls: AtomicUsize::new(0),
            limits: StdMutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl NativeStillImageDecoder for FakeDecoder {
    fn backend(&self) -> &'static str {
        "fake"
    }

    fn decode(
        &self,
        _source: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodedRgbaImage, NativeImageDecodeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.limits.lock().unwrap().push(limits);
        let (width, height) = self.result?;
        let pixels = (0..width * height)
            .flat_map(|index| [(index % 200) as u8, 90, 30, 255])
            .collect();
        Ok(DecodedRgbaImage::from_straight_rgba(
            width,
            height,
            width as usize * 4,
            pixels,
            limits,
        )?)
    }
}

fn target() -> ComposerTarget {
    ComposerTarget::Main {
        room_id: "!room:example.invalid".to_owned(),
    }
}

fn input(id: &str, bytes: Vec<u8>) -> StageUploadBytesInput {
    StageUploadBytesInput {
        staged_id: id.to_owned(),
        position: 1,
        filename: "camera.heic".to_owned(),
        mime_type: String::new(),
        bytes,
    }
}

/// The licensed opaque fixture followed by a top-level `free` box that names
/// an Apple HDR gain-map auxiliary image. This is not a real gain map: it
/// reproduces, on any platform, the content condition the pure HEIF path
/// rejects for gain-map photos (#1147) while the primary image stays valid.
fn gain_map_marked_heif() -> Vec<u8> {
    let marker = b"urn:com:apple:photo:2020:aux:hdrgainmap";
    let mut bytes = FIXTURE.to_vec();
    bytes.extend_from_slice(&(8 + marker.len() as u32).to_be_bytes());
    bytes.extend_from_slice(b"free");
    bytes.extend_from_slice(marker);
    bytes
}

fn registry_with(decoder: &Arc<FakeDecoder>) -> MediaPreparationRegistry {
    MediaPreparationRegistry::with_native_image_decoder(Some(
        Arc::clone(decoder) as Arc<dyn NativeStillImageDecoder>
    ))
}

fn prepare(
    registry: &mut MediaPreparationRegistry,
    item: StageUploadBytesInput,
) -> StagedUploadItem {
    registry
        .prepare_items(
            &target(),
            vec![item],
            ImageUploadCompressionPolicy::default(),
        )
        .pop()
        .expect("one staged item")
}

#[test]
fn native_decoder_prepares_heif_initial_and_lazy_outputs() {
    let decoder = FakeDecoder::ready(48, 32);
    let mut registry = registry_with(&decoder);
    let item = prepare(&mut registry, input("native", FIXTURE.to_vec()));

    assert_eq!(decoder.calls(), 1);
    assert_eq!(
        decoder.limits.lock().unwrap().as_slice(),
        &[DecodeLimits::default()]
    );
    assert_eq!(
        item.kind,
        StagedUploadKind::Image {
            width: Some(48),
            height: Some(32)
        }
    );
    assert_eq!(item.mime_type, "image/jpeg");
    let StagedUploadPreparation::Ready {
        variants, selected, ..
    } = &item.preparation
    else {
        panic!("native decode should publish Ready");
    };
    assert_eq!(
        *selected,
        StagedUploadOutputSelection {
            resize: StagedUploadResizeChoice::Original,
            format: StagedUploadFormatChoice::Jpeg,
        }
    );
    let original = variants
        .iter()
        .find(|variant| variant.variant_id == "original-keep")
        .expect("exact original choice");
    assert_eq!(original.mime_type, "image/heic");
    assert_eq!(original.filename, "camera.heic");
    assert_eq!((original.width, original.height), (Some(48), Some(32)));
    assert_eq!(
        registry
            .variant_bytes(&target(), "native", "original-keep")
            .expect("original bytes"),
        FIXTURE
    );

    let converted = registry
        .selected_upload(&target(), "native")
        .expect("default converted output");
    assert_eq!(converted.descriptor.mime_type, "image/jpeg");
    assert_eq!(
        image::load_from_memory(&converted.bytes)
            .unwrap()
            .dimensions(),
        (48, 32)
    );

    let source = registry
        .source_input(&target(), "native")
        .expect("retained source");
    for (selection, mime, dimensions) in [
        (
            StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Half,
                format: StagedUploadFormatChoice::Png,
            },
            "image/png",
            (24, 16),
        ),
        (
            StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Quarter,
                format: StagedUploadFormatChoice::Webp,
            },
            "image/webp",
            (12, 8),
        ),
        (
            StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Eighth,
                format: StagedUploadFormatChoice::Keep,
            },
            "image/jpeg",
            (6, 4),
        ),
    ] {
        let (descriptor, bytes) = MediaPreparationRegistry::encode_output(
            &source,
            selection,
            ImageUploadCompressionPolicy::default(),
            registry.native_image_decoder(),
        )
        .expect("lazy native output");
        assert_eq!(
            descriptor.variant_id,
            MediaPreparationRegistry::output_identity(selection)
        );
        assert_eq!(descriptor.mime_type, mime);
        assert_eq!(descriptor.byte_count, bytes.len() as u64);
        assert_eq!(
            (descriptor.width, descriptor.height),
            (Some(u64::from(dimensions.0)), Some(u64::from(dimensions.1)))
        );
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().dimensions(),
            dimensions
        );
    }
    assert_eq!(decoder.calls(), 4, "lazy outputs use the same decoder");
}

#[test]
fn gain_map_heif_is_rejected_by_the_pure_path_but_ready_with_a_native_decoder() {
    let mut pure = MediaPreparationRegistry::default();
    let rejected = prepare(&mut pure, input("gain-map", gain_map_marked_heif()));
    assert_eq!(rejected.kind, StagedUploadKind::File);
    assert_eq!(
        rejected.preparation,
        StagedUploadPreparation::Failed {
            failure_kind: MediaPreparationFailureKind::Decode,
            can_use_original: true,
        }
    );

    let decoder = FakeDecoder::ready(40, 30);
    let mut native = registry_with(&decoder);
    let ready = prepare(&mut native, input("gain-map", gain_map_marked_heif()));
    assert_eq!(
        ready.kind,
        StagedUploadKind::Image {
            width: Some(40),
            height: Some(30)
        }
    );
    assert!(matches!(
        ready.preparation,
        StagedUploadPreparation::Ready { .. }
    ));
}

#[test]
fn native_decode_failure_keeps_the_original_file_fallback() {
    for (error, failure_kind) in [
        (
            NativeImageDecodeError::Unsupported,
            MediaPreparationFailureKind::Unsupported,
        ),
        (
            NativeImageDecodeError::Unavailable,
            MediaPreparationFailureKind::Decode,
        ),
        (
            NativeImageDecodeError::Malformed,
            MediaPreparationFailureKind::Decode,
        ),
        (
            NativeImageDecodeError::TooLarge,
            MediaPreparationFailureKind::Decode,
        ),
    ] {
        let decoder = FakeDecoder::failing(error);
        let mut registry = registry_with(&decoder);
        let item = prepare(&mut registry, input("fails", FIXTURE.to_vec()));
        assert_eq!(item.kind, StagedUploadKind::File, "{error:?}");
        assert_eq!(item.mime_type, "image/heic");
        assert_eq!(
            item.preparation,
            StagedUploadPreparation::Failed {
                failure_kind,
                can_use_original: true,
            },
            "{error:?}"
        );
        // No converted output is reachable without a successful decode.
        assert!(registry.selected_upload(&target(), "fails").is_none());

        let source = registry
            .source_input(&target(), "fails")
            .expect("retained source");
        assert!(
            MediaPreparationRegistry::encode_output(
                &source,
                StagedUploadOutputSelection {
                    resize: StagedUploadResizeChoice::Half,
                    format: StagedUploadFormatChoice::Jpeg,
                },
                ImageUploadCompressionPolicy::default(),
                registry.native_image_decoder(),
            )
            .is_err()
        );

        let original = registry
            .use_original(&target(), "fails")
            .expect("original fallback");
        assert_eq!(original.mime_type, "image/heic");
        assert_eq!(
            registry
                .selected_upload(&target(), "fails")
                .expect("original upload")
                .bytes,
            FIXTURE
        );
    }
}

#[test]
fn native_decoder_is_only_used_for_heif_sources() {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    let mut png = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 4, Rgba([1, 2, 3, 255])))
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let decoder = FakeDecoder::ready(1, 1);
    let mut registry = registry_with(&decoder);
    let item = prepare(
        &mut registry,
        StageUploadBytesInput {
            mime_type: "image/png".to_owned(),
            filename: "shot.png".to_owned(),
            ..input("png", png.into_inner())
        },
    );
    assert_eq!(
        item.kind,
        StagedUploadKind::Image {
            width: Some(8),
            height: Some(4)
        }
    );
    let source = registry.source_input(&target(), "png").unwrap();
    MediaPreparationRegistry::encode_output(
        &source,
        StagedUploadOutputSelection {
            resize: StagedUploadResizeChoice::Half,
            format: StagedUploadFormatChoice::Jpeg,
        },
        ImageUploadCompressionPolicy::default(),
        registry.native_image_decoder(),
    )
    .expect("pure path");
    assert_eq!(decoder.calls(), 0);
}

#[test]
fn native_decode_diagnostics_are_categorical() {
    let _guard = koushi_diagnostics::test_support::lock();
    let decoder = FakeDecoder::failing(NativeImageDecodeError::Unsupported);
    let mut registry = registry_with(&decoder);
    prepare(&mut registry, input("diagnostic", FIXTURE.to_vec()));

    let records = koushi_diagnostics::snapshot().records;
    let event = &records
        .iter()
        .rev()
        .find(|record| {
            record.event.source == "core.media_preparation" && record.event.stage == "native_decode"
        })
        .expect("native decode diagnostic")
        .event;
    let token = |key: &str| {
        event.fields.iter().find_map(|field| match field.value {
            koushi_diagnostics::DiagnosticValue::Token(value) if field.key == key => Some(value),
            _ => None,
        })
    };
    assert_eq!(token("backend"), Some("fake"));
    assert_eq!(token("detected"), Some("heic"));
    assert_eq!(token("outcome"), Some("unsupported"));
    assert!(event.fields.iter().any(|field| field.key == "elapsed_ms"));
    let serialized = serde_json::to_string(event).unwrap();
    assert!(!serialized.contains("camera"));
}
