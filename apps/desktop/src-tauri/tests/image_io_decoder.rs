//! macOS ImageIO still-image decoder (#1147). These exercise the real platform
//! decoder and therefore run only on macOS.
#![cfg(target_os = "macos")]

use std::sync::Arc;

use koushi_core::media_preparation::{MediaPreparationRegistry, StageUploadBytesInput};
use koushi_core::native_image_decoder::DecodeLimits;
use koushi_core::{NativeImageDecodeError, NativeStillImageDecoder};
use koushi_desktop::image_io_decoder::{ImageIoStillImageDecoder, sdr_decode_request};
use koushi_state::{
    ComposerTarget, ImageUploadCompressionPolicy, StagedUploadFormatChoice, StagedUploadKind,
    StagedUploadOutputSelection, StagedUploadPreparation, StagedUploadResizeChoice,
};

/// The repository's licensed 64x64 opaque HEIC fixture.
const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../crates/koushi-media/tests/fixtures/heif/opaque.heic"
));

/// Synthetic 1x1 PNG; test-only bytes with no user data.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn heic_fixture_decodes_to_opaque_srgb_pixels() {
    let decoder = ImageIoStillImageDecoder;
    assert_eq!(decoder.backend(), "imageio");
    let decoded = decoder
        .decode(FIXTURE, DecodeLimits::default())
        .expect("ImageIO decodes the licensed HEIC fixture");
    assert_eq!((decoded.width(), decoded.height()), (64, 64));
    assert_eq!(decoded.pixels().len(), 64 * 64 * 4);
    assert!(
        decoded
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 255),
        "an opaque photo stays opaque after un-premultiplying"
    );
}

#[test]
fn limits_are_enforced_before_decoding() {
    let limits = DecodeLimits {
        max_dimension: 32,
        ..DecodeLimits::default()
    };
    assert_eq!(
        ImageIoStillImageDecoder.decode(FIXTURE, limits),
        Err(NativeImageDecodeError::TooLarge)
    );
}

#[test]
fn malformed_and_non_heif_sources_are_typed_failures() {
    let decoder = ImageIoStillImageDecoder;
    assert_eq!(
        decoder.decode(&[], DecodeLimits::default()),
        Err(NativeImageDecodeError::Malformed)
    );
    assert_eq!(
        decoder.decode(b"not an image at all", DecodeLimits::default()),
        Err(NativeImageDecodeError::Malformed)
    );
    assert_eq!(
        decoder.decode(&FIXTURE[..FIXTURE.len() / 3], DecodeLimits::default()),
        Err(NativeImageDecodeError::Malformed)
    );
    assert_eq!(
        decoder.decode(PNG, DecodeLimits::default()),
        Err(NativeImageDecodeError::Unsupported)
    );
}

#[test]
fn sdr_decode_request_is_resolved_exactly_on_macos_14_and_later() {
    let version = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .expect("sw_vers");
    let version = String::from_utf8(version.stdout).expect("utf-8 version");
    let major: u32 = version
        .trim()
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .expect("major version");
    assert_eq!(sdr_decode_request().is_some(), major >= 14, "macOS {major}");
}

#[test]
fn core_preparation_uses_imageio_for_initial_and_lazy_outputs() {
    let target = ComposerTarget::Main {
        room_id: "!room:example.invalid".to_owned(),
    };
    let mut registry = MediaPreparationRegistry::with_native_image_decoder(Some(Arc::new(
        ImageIoStillImageDecoder,
    )));
    let item = registry
        .prepare_items(
            &target,
            vec![StageUploadBytesInput {
                staged_id: "heic".to_owned(),
                position: 1,
                filename: "IMG_0001.HEIC".to_owned(),
                mime_type: String::new(),
                bytes: FIXTURE.to_vec(),
            }],
            ImageUploadCompressionPolicy::default(),
        )
        .pop()
        .expect("one staged item");
    assert_eq!(
        item.kind,
        StagedUploadKind::Image {
            width: Some(64),
            height: Some(64)
        }
    );
    assert_eq!(item.mime_type, "image/jpeg");
    assert!(matches!(
        item.preparation,
        StagedUploadPreparation::Ready { .. }
    ));
    assert_eq!(
        registry
            .variant_bytes(&target, "heic", "original-keep")
            .expect("exact original"),
        FIXTURE
    );

    let source = registry.source_input(&target, "heic").expect("source");
    for (format, mime) in [
        (StagedUploadFormatChoice::Png, "image/png"),
        (StagedUploadFormatChoice::Webp, "image/webp"),
        (StagedUploadFormatChoice::Keep, "image/jpeg"),
    ] {
        let (descriptor, bytes) = MediaPreparationRegistry::encode_output(
            &source,
            StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Half,
                format,
            },
            ImageUploadCompressionPolicy::default(),
            registry.native_image_decoder(),
        )
        .expect("lazy ImageIO output");
        assert_eq!(descriptor.mime_type, mime);
        assert_eq!((descriptor.width, descriptor.height), (Some(32), Some(32)));
        assert_eq!(descriptor.byte_count, bytes.len() as u64);
    }
}
