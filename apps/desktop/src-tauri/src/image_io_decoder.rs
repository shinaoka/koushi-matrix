//! macOS still-image decoding through ImageIO and Core Graphics (#1147).
//!
//! Core injects this decoder into HEIF media preparation. It decodes only the
//! primary still, applies its orientation, converts HDR/gain-map content to
//! SDR where the OS supports that request, and renders into an 8-bit sRGB
//! bitmap. Every CF object is owned by a `CFRetained`, so it is released on
//! all paths; only owned pixels or a typed category leave this module.

use std::ffi::{CStr, c_char, c_void};

use koushi_core::native_image_decoder::{DecodeLimits, DecodedRgbaImage};
use koushi_core::{NativeImageDecodeError, NativeStillImageDecoder};
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_image_io::{
    CGImageSource, CGImageSourceStatus, kCGImageAuxiliaryDataTypeHDRGainMap,
    kCGImagePropertyPixelHeight, kCGImagePropertyPixelWidth,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache, kCGImageSourceThumbnailMaxPixelSize,
};

/// Fixed backend name reported in diagnostics.
const BACKEND: &str = "imageio";

/// The only ImageIO source types accepted as a single HEIF still. Image
/// sequences (`public.heics`) and every other container are unsupported.
const STILL_HEIF_TYPES: [&str; 2] = ["public.heic", "public.heif"];

/// ImageIO/Core Graphics decoder injected into Core media preparation.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImageIoStillImageDecoder;

impl NativeStillImageDecoder for ImageIoStillImageDecoder {
    fn backend(&self) -> &'static str {
        BACKEND
    }

    fn decode(
        &self,
        source: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodedRgbaImage, NativeImageDecodeError> {
        // Preparation runs on a worker thread without an enclosing pool;
        // drain anything ImageIO autoreleases before returning.
        objc2::rc::autoreleasepool(|_| decode_primary_still(source, limits))
    }
}

fn decode_primary_still(
    source: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedRgbaImage, NativeImageDecodeError> {
    if source.is_empty() {
        return Err(NativeImageDecodeError::Malformed);
    }
    let yes: &CFType = CFBoolean::new(true);
    let no: &CFType = CFBoolean::new(false);
    let data = CFData::from_bytes(source);
    // SAFETY: ImageIO exports this key on every supported macOS version.
    let no_cache = cf_dictionary(&[(unsafe { kCGImageSourceShouldCache }, no)]);
    // SAFETY: the options dictionary maps CFString keys to CF values.
    let image_source = unsafe { CGImageSource::with_data(&data, Some(no_cache.as_opaque())) }
        .ok_or(NativeImageDecodeError::Malformed)?;
    // SAFETY: `image_source` is a valid source for the duration of the call.
    let source_type = unsafe { image_source.r#type() }
        .map(|identifier| identifier.to_string())
        .ok_or(NativeImageDecodeError::Malformed)?;
    if !STILL_HEIF_TYPES.contains(&source_type.as_str()) {
        return Err(NativeImageDecodeError::Unsupported);
    }
    // SAFETY: as above.
    if unsafe { image_source.count() } == 0 {
        return Err(NativeImageDecodeError::Malformed);
    }
    // SAFETY: as above; available since macOS 10.14.
    let primary = unsafe { image_source.primary_image_index() };
    // A truncated or corrupt container must not yield a partially decoded
    // image.
    // SAFETY: as above.
    if unsafe { image_source.status_at_index(primary) } != CGImageSourceStatus::StatusComplete {
        return Err(NativeImageDecodeError::Malformed);
    }

    // Bound the decode by the container's declared dimensions before any
    // pixel buffer exists.
    let (width, height) = primary_dimensions(&image_source, primary)?;
    if !limits.admits(width, height) {
        return Err(NativeImageDecodeError::TooLarge);
    }

    let sdr_request = sdr_decode_request();
    if sdr_request.is_none() && has_hdr_gain_map(&image_source, primary) {
        // Without the macOS 14 SDR request, the base image of a gain-map
        // photo cannot be tone mapped faithfully; keep the original instead.
        return Err(NativeImageDecodeError::Unsupported);
    }

    let max_pixel_size = CFNumber::new_i64(i64::try_from(width.max(height)).unwrap_or(i64::MAX));
    let max_pixel_size: &CFType = &max_pixel_size;
    // SAFETY: ImageIO exports these keys on every supported macOS version.
    let mut entries: Vec<(&CFString, &CFType)> = unsafe {
        vec![
            (kCGImageSourceCreateThumbnailFromImageAlways, yes),
            (kCGImageSourceCreateThumbnailWithTransform, yes),
            (kCGImageSourceThumbnailMaxPixelSize, max_pixel_size),
            (kCGImageSourceShouldCache, no),
        ]
    };
    if let Some((request_key, to_sdr)) = sdr_request {
        entries.push((request_key, to_sdr));
    }
    let options = cf_dictionary(&entries);
    // SAFETY: the options dictionary maps CFString keys to CF values.
    let image = unsafe { image_source.thumbnail_at_index(primary, Some(options.as_opaque())) }
        .ok_or(NativeImageDecodeError::Malformed)?;

    if sdr_request.is_none()
        && CGImage::color_space(Some(&*image)).is_some_and(|space| space.uses_itur_2100_tf())
    {
        // PQ/HLG content that this OS cannot convert to SDR.
        return Err(NativeImageDecodeError::Unsupported);
    }
    render_srgb(&image, limits)
}

/// Draw `image` into a tightly packed 8-bit sRGB RGBA bitmap.
fn render_srgb(
    image: &CGImage,
    limits: DecodeLimits,
) -> Result<DecodedRgbaImage, NativeImageDecodeError> {
    // The transform may have swapped the declared dimensions; re-check the
    // dimensions actually allocated.
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    if !limits.admits(width as u64, height as u64) {
        return Err(NativeImageDecodeError::TooLarge);
    }
    let stride = width * 4;
    let mut pixels = vec![0u8; stride * height];
    // SAFETY: Core Graphics exports this name on every supported macOS version.
    let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or(NativeImageDecodeError::Unavailable)?;
    // Core Graphics only renders 8-bit RGBA with premultiplied alpha; the
    // straight-alpha conversion happens below. Byte order is R, G, B, A.
    let bitmap_info = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
    // SAFETY: `pixels` holds `stride * height` bytes and outlives `context`,
    // which is dropped before the buffer is read.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast::<c_void>(),
            width,
            height,
            8,
            stride,
            Some(&*srgb),
            bitmap_info,
        )
    }
    .ok_or(NativeImageDecodeError::Unavailable)?;
    let bounds = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&*context), bounds, Some(image));
    drop(context);
    let width = u32::try_from(width).map_err(|_| NativeImageDecodeError::TooLarge)?;
    let height = u32::try_from(height).map_err(|_| NativeImageDecodeError::TooLarge)?;
    Ok(DecodedRgbaImage::from_premultiplied_rgba(
        width, height, stride, pixels, limits,
    )?)
}

/// Declared pixel dimensions of the image at `index`, before orientation.
fn primary_dimensions(
    image_source: &CGImageSource,
    index: usize,
) -> Result<(u64, u64), NativeImageDecodeError> {
    // SAFETY: no options are passed.
    let properties = unsafe { image_source.properties_at_index(index, None) }
        .ok_or(NativeImageDecodeError::Malformed)?;
    // SAFETY: image property dictionaries have CFString keys.
    let properties = unsafe { properties.cast_unchecked::<CFString, CFType>() };
    let dimension = |key: &CFString| {
        properties
            .get(key)
            .and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64))
            .and_then(|value| u64::try_from(value).ok())
            .ok_or(NativeImageDecodeError::Malformed)
    };
    // SAFETY: ImageIO exports these keys on every supported macOS version.
    let width = dimension(unsafe { kCGImagePropertyPixelWidth })?;
    let height = dimension(unsafe { kCGImagePropertyPixelHeight })?;
    Ok((width, height))
}

/// Whether the image at `index` carries an Apple HDR gain map.
fn has_hdr_gain_map(image_source: &CGImageSource, index: usize) -> bool {
    // SAFETY: the auxiliary type constant is available since macOS 11.
    unsafe { image_source.auxiliary_data_info_at_index(index, kCGImageAuxiliaryDataTypeHDRGainMap) }
        .is_some()
}

/// `kCGImageSourceDecodeRequest` and `kCGImageSourceDecodeToSDR`, which exist
/// only on macOS 14 and later.
///
/// The bindings declare them as ordinary extern statics; referencing those
/// would make the binary fail to launch on macOS 12 and 13, so they are looked
/// up at run time instead.
pub fn sdr_decode_request() -> Option<(&'static CFString, &'static CFType)> {
    let request = exported_cf_string(c"kCGImageSourceDecodeRequest")?;
    let to_sdr: &'static CFType = exported_cf_string(c"kCGImageSourceDecodeToSDR")?;
    Some((request, to_sdr))
}

/// `RTLD_DEFAULT` on Apple platforms.
const RTLD_DEFAULT: *mut c_void = std::ptr::without_provenance_mut(-2isize as usize);

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// Read an exported `const CFStringRef` variable from a loaded framework.
fn exported_cf_string(name: &CStr) -> Option<&'static CFString> {
    // SAFETY: `name` is NUL-terminated; a null result means the symbol is
    // absent on this OS version.
    let symbol = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    // SAFETY: the symbol is a `const CFStringRef` variable holding a constant
    // string that lives for the rest of the process.
    unsafe { (*symbol.cast::<*const CFString>()).as_ref() }
}

fn cf_dictionary(entries: &[(&CFString, &CFType)]) -> CFRetained<CFDictionary<CFString, CFType>> {
    let keys = entries.iter().map(|(key, _)| *key).collect::<Vec<_>>();
    let values = entries.iter().map(|(_, value)| *value).collect::<Vec<_>>();
    CFDictionary::from_slices(&keys, &values)
}
