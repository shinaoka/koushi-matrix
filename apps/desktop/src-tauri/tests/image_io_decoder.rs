//! macOS ImageIO still-image decoder (#1147). These exercise the real platform
//! decoder and therefore run only on macOS.
//!
//! The acceptance fixtures are encoded at test time by ImageIO itself, so they
//! are genuine Apple HEIC output rather than files from another encoder. When
//! a runner cannot encode HEIC the tests fail and say so; they never pass
//! silently.
#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::fmt::Write as _;
use std::sync::Arc;

use koushi_core::media_preparation::{MediaPreparationRegistry, StageUploadBytesInput};
use koushi_core::native_image_decoder::DecodeLimits;
use koushi_core::{NativeImageDecodeError, NativeStillImageDecoder};
use koushi_desktop::image_io_decoder::{ImageIoStillImageDecoder, sdr_decode_request};
use koushi_state::{
    ComposerTarget, ImageUploadCompressionPolicy, StagedUploadFormatChoice, StagedUploadKind,
    StagedUploadOutputSelection, StagedUploadPreparation, StagedUploadResizeChoice,
};
use objc2_core_foundation::{
    CFCopyTypeIDDescription, CFData, CFDictionary, CFGetTypeID, CFMutableData, CFNumber, CFString,
    CFType,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_image_io::{CGImageDestination, CGImageSource, kCGImagePropertyOrientation};

/// The repository's licensed 64x64 opaque HEIC fixture (libheif output).
const LICENSED_FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../crates/koushi-media/tests/fixtures/heif/opaque.heic"
));

/// Synthetic 1x1 PNG; test-only bytes with no user data.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// Encode an opaque synthetic gradient as HEIC with ImageIO. `orientation` is
/// the EXIF/TIFF orientation tag stored in the container.
fn apple_heic(width: usize, height: usize, orientation: i32) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let (x, y) = (index % width, index / width);
        pixel.copy_from_slice(&[(x * 255 / width) as u8, (y * 255 / height) as u8, 96, 255]);
    }
    // SAFETY: Core Graphics exports this name on every supported macOS version.
    let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).expect("sRGB");
    // SAFETY: `pixels` holds `width * 4 * height` bytes and outlives `context`.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast::<c_void>(),
            width,
            height,
            8,
            width * 4,
            Some(&*srgb),
            CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0,
        )
    }
    .expect("bitmap context");
    let image = CGBitmapContextCreateImage(Some(&*context)).expect("synthetic CGImage");
    drop(context);

    let encoded = CFMutableData::new(None, 0).expect("mutable data");
    let heic = CFString::from_static_str("public.heic");
    // SAFETY: `encoded` and `heic` are valid; no options.
    let Some(destination) = (unsafe { CGImageDestination::with_data(&encoded, &heic, 1, None) })
    else {
        panic!(
            "this runner's ImageIO cannot create a public.heic destination \
             (HEVC encoding unavailable); the native decoder is unverified here"
        );
    };
    let orientation = CFNumber::new_i32(orientation);
    let orientation: &CFType = &orientation;
    // SAFETY: ImageIO exports this key on every supported macOS version.
    let properties = CFDictionary::<CFString, CFType>::from_slices(
        &[unsafe { kCGImagePropertyOrientation }],
        &[orientation],
    );
    // SAFETY: the image and the CFString-keyed properties are valid.
    unsafe { destination.add_image(&image, Some(properties.as_opaque())) };
    // SAFETY: `destination` is valid.
    assert!(
        unsafe { destination.finalize() },
        "ImageIO could not finalize a public.heic image on this runner"
    );
    let bytes = encoded.to_vec();
    assert!(!bytes.is_empty(), "ImageIO produced an empty HEIC");
    bytes
}

/// Stage-by-stage report of how ImageIO sees `bytes`, attached to failures so
/// one CI run shows which decoder precondition does not hold.
fn diagnose(label: &str, bytes: &[u8]) -> String {
    let mut report = format!("[{label}] {} byte(s)", bytes.len());
    let data = CFData::from_bytes(bytes);
    // SAFETY: no options.
    let Some(source) = (unsafe { CGImageSource::with_data(&data, None) }) else {
        return report + "; CGImageSourceCreateWithData returned NULL";
    };
    // SAFETY: `source` is valid for every query below.
    unsafe {
        let source_type = source.r#type().map(|value| value.to_string());
        let primary = source.primary_image_index();
        let _ = write!(
            report,
            "; type={source_type:?} count={} primary={primary} status={:?} status_at_primary={:?}",
            source.count(),
            source.status(),
            source.status_at_index(primary),
        );
        match source.properties_at_index(primary, None) {
            None => report += "; properties_at_index=NULL",
            Some(properties) => {
                let (keys, values) = properties.cast_unchecked::<CFString, CFType>().to_vecs();
                for (key, value) in keys.iter().zip(values) {
                    let kind = CFCopyTypeIDDescription(CFGetTypeID(Some(&value)))
                        .map(|kind| kind.to_string())
                        .unwrap_or_default();
                    let shown = if kind == "CFNumber" || kind == "CFString" || kind == "CFBoolean" {
                        format!("{:?}", &*value)
                    } else {
                        "..".to_owned()
                    };
                    let _ = write!(report, "; {key}<{kind}>={shown}");
                }
            }
        }
        match source.image_at_index(primary, None) {
            None => report += "; CGImageSourceCreateImageAtIndex=NULL",
            Some(image) => {
                let _ = write!(
                    report,
                    "; image_at_index={}x{}",
                    CGImage::width(Some(&*image)),
                    CGImage::height(Some(&*image))
                );
            }
        }
    }
    report
}

fn decode(
    bytes: &[u8],
) -> Result<koushi_core::native_image_decoder::DecodedRgbaImage, NativeImageDecodeError> {
    ImageIoStillImageDecoder.decode(bytes, DecodeLimits::default())
}

#[test]
fn apple_encoded_heic_decodes_to_opaque_srgb_pixels() {
    let heic = apple_heic(64, 40, 1);
    let decoded = decode(&heic)
        .unwrap_or_else(|error| panic!("{error:?}: {}", diagnose("apple 64x40", &heic)));
    assert_eq!(ImageIoStillImageDecoder.backend(), "imageio");
    assert_eq!((decoded.width(), decoded.height()), (64, 40));
    assert_eq!(decoded.pixels().len(), 64 * 40 * 4);
    assert!(
        decoded
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 255),
        "an opaque photo stays opaque after un-premultiplying"
    );
    // The gradient runs from dark at the top-left to bright red and green at
    // the bottom-right, so the pixels kept their orientation.
    let first = &decoded.pixels()[..4];
    let last = &decoded.pixels()[decoded.pixels().len() - 4..];
    assert!(first[0] < 40 && first[1] < 40, "{first:?}");
    assert!(last[0] > 200 && last[1] > 200, "{last:?}");
}

#[test]
fn orientation_is_applied_before_reporting_dimensions() {
    let heic = apple_heic(64, 40, 6);
    let decoded = decode(&heic)
        .unwrap_or_else(|error| panic!("{error:?}: {}", diagnose("apple rotated", &heic)));
    assert_eq!((decoded.width(), decoded.height()), (40, 64));
}

#[test]
fn licensed_libheif_fixture_has_an_explicit_outcome() {
    // The libheif-made fixture is not the native acceptance fixture (#1147);
    // record how ImageIO treats it and require a decode or a typed failure.
    match decode(LICENSED_FIXTURE) {
        Ok(decoded) => assert_eq!((decoded.width(), decoded.height()), (64, 64)),
        Err(error) => {
            eprintln!("{error:?}: {}", diagnose("licensed", LICENSED_FIXTURE));
            assert!(
                matches!(
                    error,
                    NativeImageDecodeError::Unsupported | NativeImageDecodeError::Malformed
                ),
                "{error:?}"
            );
        }
    }
}

#[test]
fn limits_are_enforced_before_decoding() {
    let heic = apple_heic(64, 40, 1);
    let limits = DecodeLimits {
        max_dimension: 32,
        ..DecodeLimits::default()
    };
    assert_eq!(
        ImageIoStillImageDecoder.decode(&heic, limits),
        Err(NativeImageDecodeError::TooLarge),
        "{}",
        diagnose("apple limits", &heic)
    );
}

#[test]
fn malformed_and_non_heif_sources_are_typed_failures() {
    let heic = apple_heic(64, 40, 1);
    assert_eq!(decode(&[]), Err(NativeImageDecodeError::Malformed));
    assert_eq!(
        decode(b"not an image at all"),
        Err(NativeImageDecodeError::Malformed)
    );
    let truncated = &heic[..heic.len() / 3];
    let outcome = decode(truncated);
    assert!(
        matches!(
            outcome,
            Err(NativeImageDecodeError::Malformed | NativeImageDecodeError::Unsupported)
        ),
        "{outcome:?}: {}",
        diagnose("truncated", truncated)
    );
    assert_eq!(decode(PNG), Err(NativeImageDecodeError::Unsupported));
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
    let heic = apple_heic(64, 40, 1);
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
                bytes: heic.clone(),
            }],
            ImageUploadCompressionPolicy::default(),
        )
        .pop()
        .expect("one staged item");
    assert_eq!(
        item.kind,
        StagedUploadKind::Image {
            width: Some(64),
            height: Some(40)
        },
        "{}",
        diagnose("core", &heic)
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
        heic
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
        assert_eq!((descriptor.width, descriptor.height), (Some(32), Some(20)));
        assert_eq!(descriptor.byte_count, bytes.len() as u64);
    }
}
