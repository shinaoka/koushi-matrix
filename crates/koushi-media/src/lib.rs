mod decoded;
#[cfg(test)]
mod decoded_tests;
mod image_kind;
mod thumbnail;
#[cfg(test)]
mod thumbnail_tests;

pub use decoded::{DecodeLimits, DecodedRgbaImage, DecodedRgbaImageError};
pub use image_kind::{ImageKind, image_kind};
pub use thumbnail::thumbnail_jpeg;

use std::io::Cursor;

use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat, ImageReader,
    Limits, RgbaImage,
    codecs::{jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder},
    imageops::FilterType,
};

const MAX_DECODED_DIMENSION: u32 = 16_384;
const MAX_DECODED_ALLOCATION: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImagePreparationPolicy {
    pub target_long_edge: u32,
    pub quality_percent: u8,
}

impl Default for ImagePreparationPolicy {
    fn default() -> Self {
        Self {
            target_long_edge: 2048,
            quality_percent: 82,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedImageFormat {
    Png,
    Jpeg,
    WebP,
    Gif,
    Heif,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedImageVariant {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub format: PreparedImageFormat,
    pub bytes: Vec<u8>,
    pub dimensions: (u32, u32),
    pub metadata_stripped: bool,
    pub thumbnail_refreshed: bool,
    pub recommended: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ImagePreparationError {
    #[error("empty image source")]
    Empty,
    #[error("image decoding failed")]
    Decode,
    #[error("image encoding failed")]
    Encode,
}

/// Linear scale applied to both dimensions of the source image.
///
/// This is independent of the output encoding: `Original` preserves the source
/// dimensions, while [`ImageOutputFormat::Keep`] preserves the source encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageResizeScale {
    Original,
    Half,
    Quarter,
    Eighth,
}

impl ImageResizeScale {
    /// Divisor applied to each dimension.
    pub const fn divisor(self) -> u32 {
        match self {
            Self::Original => 1,
            Self::Half => 2,
            Self::Quarter => 4,
            Self::Eighth => 8,
        }
    }

    /// Stable token used in cache identities and diagnostics.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Half => "half",
            Self::Quarter => "quarter",
            Self::Eighth => "eighth",
        }
    }
}

/// Requested output encoding. `Keep` preserves the source encoding, including
/// the original bytes for an unscaled, non-HEIF image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageOutputFormat {
    Keep,
    Png,
    Jpeg,
    WebP,
}

impl ImageOutputFormat {
    /// Stable token used in cache identities and diagnostics.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::WebP => "webp",
        }
    }
}

/// One independently chosen output: a linear resize plus an encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageOutputRequest {
    pub resize: ImageResizeScale,
    pub format: ImageOutputFormat,
}

impl ImageOutputRequest {
    /// Cache identity for this exact combination.
    pub fn identity(&self) -> String {
        format!("{}-{}", self.resize.token(), self.format.token())
    }
}

/// Encode exactly one requested output.
///
/// The returned dimensions and bytes always describe the same image, so a
/// caller can report the size that will be uploaded instead of an estimate.
/// Encoding is deterministic for a given request, which is what makes the
/// request's [`ImageOutputRequest::identity`] usable as a cache key.
pub fn prepare_image_output(
    source: &[u8],
    filename: &str,
    request: ImageOutputRequest,
    policy: &ImagePreparationPolicy,
) -> Result<PreparedImageVariant, ImagePreparationError> {
    if source.is_empty() {
        return Err(ImagePreparationError::Empty);
    }
    let heif = probe_heif(source);
    let guessed = image::guess_format(source).ok();
    let source_format = if heif.is_some() {
        PreparedImageFormat::Heif
    } else {
        prepared_format(guessed)
    };
    let decodable = match source_format {
        PreparedImageFormat::Heif => true,
        PreparedImageFormat::Png | PreparedImageFormat::Jpeg | PreparedImageFormat::WebP => {
            !animated_webp(source) && !animated_png(source)
        }
        PreparedImageFormat::Gif | PreparedImageFormat::Other => false,
    };
    if !decodable {
        return Err(ImagePreparationError::Decode);
    }

    // An unscaled Keep output does not need a pixel buffer. Probe dimensions
    // under the same limits as the decoder and retain the exact source bytes;
    // this avoids allocating a full RGBA image merely to upload it unchanged.
    // HEIF intentionally stays on its existing decode path because it is
    // decode-only in this crate.
    if request.resize == ImageResizeScale::Original
        && request.format == ImageOutputFormat::Keep
        && matches!(
            source_format,
            PreparedImageFormat::Png | PreparedImageFormat::Jpeg | PreparedImageFormat::WebP
        )
    {
        let dimensions =
            read_dimensions_with_limits(source, guessed.expect("recognized non-HEIF image format"))
                .map_err(|_| ImagePreparationError::Decode)?;
        return Ok(PreparedImageVariant {
            id: request.identity(),
            filename: normalized_filename(filename, extension(source_format, None)),
            mime_type: actual_mime(source_format).to_owned(),
            format: source_format,
            bytes: source.to_vec(),
            dimensions,
            metadata_stripped: false,
            thumbnail_refreshed: false,
            recommended: false,
        });
    }

    let decoded = match source_format {
        PreparedImageFormat::Heif => decode_heif(source, heif.expect("recognized HEIF"))?,
        _ => decode_with_limits(source, guessed.expect("recognized image format"))
            .map_err(|_| ImagePreparationError::Decode)?,
    };
    let target_format = match request.format {
        ImageOutputFormat::Keep => source_format,
        ImageOutputFormat::Png => PreparedImageFormat::Png,
        ImageOutputFormat::Jpeg => PreparedImageFormat::Jpeg,
        ImageOutputFormat::WebP => PreparedImageFormat::WebP,
    };
    let scaled = scale_linearly(&decoded, request.resize);
    let mut variant = encoded_variant(
        &request.identity(),
        filename,
        target_format,
        &scaled,
        policy.quality_percent,
    )?;
    variant.recommended = false;
    Ok(variant)
}

/// Encode exactly one requested output from pixels a platform decoder already
/// produced.
///
/// `format` must be a concrete encoding: pixels carry no source encoding, so
/// [`ImageOutputFormat::Keep`] is rejected and callers resolve it first. The
/// returned variant strips all source metadata, exactly like
/// [`prepare_image_output`] does for its decoded sources.
pub fn prepare_decoded_image_output(
    decoded: DecodedRgbaImage,
    filename: &str,
    request: ImageOutputRequest,
    policy: &ImagePreparationPolicy,
) -> Result<PreparedImageVariant, ImagePreparationError> {
    let target_format = match request.format {
        ImageOutputFormat::Keep => return Err(ImagePreparationError::Encode),
        ImageOutputFormat::Png => PreparedImageFormat::Png,
        ImageOutputFormat::Jpeg => PreparedImageFormat::Jpeg,
        ImageOutputFormat::WebP => PreparedImageFormat::WebP,
    };
    let (width, height) = (decoded.width(), decoded.height());
    let image = DynamicImage::ImageRgba8(
        RgbaImage::from_raw(width, height, decoded.into_pixels())
            .ok_or(ImagePreparationError::Decode)?,
    );
    // Avoid copying a full-resolution buffer for the unscaled output.
    let scaled = if request.resize == ImageResizeScale::Original {
        image
    } else {
        scale_linearly(&image, request.resize)
    };
    encoded_variant(
        &request.identity(),
        filename,
        target_format,
        &scaled,
        policy.quality_percent,
    )
}

/// Halve each dimension per scale step, never below one pixel.
fn scale_linearly(image: &DynamicImage, resize: ImageResizeScale) -> DynamicImage {
    let divisor = resize.divisor();
    if divisor <= 1 {
        return image.clone();
    }
    let (width, height) = image.dimensions();
    let target_width = (width / divisor).max(1);
    let target_height = (height / divisor).max(1);
    if (target_width, target_height) == (width, height) {
        return image.clone();
    }
    image.resize_exact(target_width, target_height, FilterType::Lanczos3)
}

pub fn prepare_image_variants(
    source: &[u8],
    filename: &str,
    _declared_mime: &str,
    policy: &ImagePreparationPolicy,
) -> Result<Vec<PreparedImageVariant>, ImagePreparationError> {
    if source.is_empty() {
        return Err(ImagePreparationError::Empty);
    }

    let heif = probe_heif(source);
    let guessed = image::guess_format(source).ok();
    let format = if heif.is_some() {
        PreparedImageFormat::Heif
    } else {
        prepared_format(guessed)
    };
    let mime_type = heif
        .map(|probe| probe.mime_type)
        .unwrap_or_else(|| actual_mime(format));
    let decoded = match format {
        PreparedImageFormat::Heif => decode_heif(source, heif.expect("recognized HEIF")).ok(),
        PreparedImageFormat::Png | PreparedImageFormat::Jpeg | PreparedImageFormat::WebP
            if !animated_webp(source) && !animated_png(source) =>
        {
            decode_with_limits(source, guessed.expect("recognized image format")).ok()
        }
        _ => None,
    };
    let dimensions = decoded
        .as_ref()
        .map(GenericImageView::dimensions)
        .unwrap_or((0, 0));
    let mut variants = vec![PreparedImageVariant {
        id: "original".to_owned(),
        filename: normalized_filename(filename, extension(format, Some(mime_type))),
        mime_type: mime_type.to_owned(),
        format,
        bytes: source.to_vec(),
        dimensions,
        metadata_stripped: false,
        thumbnail_refreshed: false,
        recommended: false,
    }];

    let Some(decoded) = decoded else {
        variants[0].recommended = true;
        return Ok(variants);
    };
    let resized = resize_to_long_edge(&decoded, policy.target_long_edge.max(1));
    match format {
        PreparedImageFormat::Png => {
            variants.push(encoded_variant(
                "resized-png",
                filename,
                PreparedImageFormat::Png,
                &resized,
                policy.quality_percent,
            )?);
            variants.push(encoded_variant(
                "webp",
                filename,
                PreparedImageFormat::WebP,
                &resized,
                policy.quality_percent,
            )?);
        }
        PreparedImageFormat::Jpeg => {
            variants.push(encoded_variant(
                "resized-jpeg",
                filename,
                PreparedImageFormat::Jpeg,
                &resized,
                policy.quality_percent,
            )?);
            variants.push(encoded_variant(
                "webp",
                filename,
                PreparedImageFormat::WebP,
                &resized,
                policy.quality_percent,
            )?);
        }
        PreparedImageFormat::WebP => variants.push(encoded_variant(
            "resized-webp",
            filename,
            PreparedImageFormat::WebP,
            &resized,
            policy.quality_percent,
        )?),
        PreparedImageFormat::Heif => {
            variants.push(encoded_variant(
                "resized-jpeg",
                filename,
                PreparedImageFormat::Jpeg,
                &resized,
                policy.quality_percent,
            )?);
            variants.push(encoded_variant(
                "resized-webp",
                filename,
                PreparedImageFormat::WebP,
                &resized,
                policy.quality_percent,
            )?);
            variants.push(encoded_variant(
                "resized-png",
                filename,
                PreparedImageFormat::Png,
                &resized,
                policy.quality_percent,
            )?);
        }
        PreparedImageFormat::Gif | PreparedImageFormat::Other => {}
    }

    let original_len = source.len();
    let recommended_index = variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| variant.bytes.len() <= original_len)
        .min_by_key(|(_, variant)| variant.bytes.len())
        .map(|(index, _)| index)
        .unwrap_or(0);
    variants[recommended_index].recommended = true;
    Ok(variants)
}

fn encoded_variant(
    id: &str,
    source_filename: &str,
    format: PreparedImageFormat,
    image: &DynamicImage,
    quality_percent: u8,
) -> Result<PreparedImageVariant, ImagePreparationError> {
    let (width, height) = image.dimensions();
    let mut bytes = Vec::new();
    match format {
        PreparedImageFormat::Png => {
            let rgba = image.to_rgba8();
            PngEncoder::new(&mut bytes)
                .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
                .map_err(|_| ImagePreparationError::Encode)?;
        }
        PreparedImageFormat::Jpeg => {
            let rgb = image.to_rgb8();
            JpegEncoder::new_with_quality(&mut bytes, quality_percent.clamp(1, 100))
                .write_image(&rgb, width, height, ExtendedColorType::Rgb8)
                .map_err(|_| ImagePreparationError::Encode)?;
        }
        PreparedImageFormat::WebP => {
            let rgba = image.to_rgba8();
            WebPEncoder::new_lossless(&mut bytes)
                .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
                .map_err(|_| ImagePreparationError::Encode)?;
        }
        PreparedImageFormat::Gif | PreparedImageFormat::Heif | PreparedImageFormat::Other => {
            return Err(ImagePreparationError::Encode);
        }
    }
    Ok(PreparedImageVariant {
        id: id.to_owned(),
        filename: normalized_filename(source_filename, extension(format, None)),
        mime_type: actual_mime(format).to_owned(),
        format,
        bytes,
        dimensions: (width, height),
        metadata_stripped: true,
        thumbnail_refreshed: true,
        recommended: false,
    })
}

fn resize_to_long_edge(image: &DynamicImage, target_long_edge: u32) -> DynamicImage {
    let (width, height) = image.dimensions();
    let long_edge = width.max(height);
    if long_edge <= target_long_edge {
        return image.clone();
    }
    let scale = target_long_edge as f64 / long_edge as f64;
    let target_width = ((width as f64 * scale).round() as u32).max(1);
    let target_height = ((height as f64 * scale).round() as u32).max(1);
    image.resize_exact(target_width, target_height, FilterType::Lanczos3)
}

fn prepared_format(format: Option<ImageFormat>) -> PreparedImageFormat {
    match format {
        Some(ImageFormat::Png) => PreparedImageFormat::Png,
        Some(ImageFormat::Jpeg) => PreparedImageFormat::Jpeg,
        Some(ImageFormat::WebP) => PreparedImageFormat::WebP,
        Some(ImageFormat::Gif) => PreparedImageFormat::Gif,
        _ => PreparedImageFormat::Other,
    }
}

fn actual_mime(format: PreparedImageFormat) -> &'static str {
    match format {
        PreparedImageFormat::Png => "image/png",
        PreparedImageFormat::Jpeg => "image/jpeg",
        PreparedImageFormat::WebP => "image/webp",
        PreparedImageFormat::Gif => "image/gif",
        PreparedImageFormat::Heif => "image/heif",
        PreparedImageFormat::Other => "application/octet-stream",
    }
}

fn extension(format: PreparedImageFormat, mime_type: Option<&str>) -> &'static str {
    match format {
        PreparedImageFormat::Png => "png",
        PreparedImageFormat::Jpeg => "jpg",
        PreparedImageFormat::WebP => "webp",
        PreparedImageFormat::Gif => "gif",
        PreparedImageFormat::Heif => {
            if mime_type == Some("image/heic") {
                "heic"
            } else {
                "heif"
            }
        }
        PreparedImageFormat::Other => "bin",
    }
}

fn normalized_filename(filename: &str, extension: &str) -> String {
    let filename = filename.trim();
    if filename.is_empty() {
        return format!("attachment.{extension}");
    }
    match filename.rfind('.') {
        Some(index) if index > 0 => format!("{}.{}", &filename[..index], extension),
        _ => format!("{filename}.{extension}"),
    }
}

fn animated_webp(source: &[u8]) -> bool {
    source.windows(4).any(|window| window == b"ANIM")
}

fn animated_png(source: &[u8]) -> bool {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !source.starts_with(PNG_SIGNATURE) {
        return false;
    }
    let mut offset = PNG_SIGNATURE.len();
    while let Some(header) = source.get(offset..offset.saturating_add(8)) {
        let length = u32::from_be_bytes(header[..4].try_into().expect("four-byte chunk length"));
        if &header[4..8] == b"acTL" {
            return true;
        }
        let Some(next) = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length as usize))
        else {
            return false;
        };
        if next > source.len() {
            return false;
        }
        offset = next;
    }
    false
}

fn decode_with_limits(
    source: &[u8],
    format: ImageFormat,
) -> Result<DynamicImage, image::ImageError> {
    let mut reader = ImageReader::with_format(Cursor::new(source), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODED_DIMENSION);
    limits.max_image_height = Some(MAX_DECODED_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_ALLOCATION);
    reader.limits(limits);
    reader.decode()
}

fn read_dimensions_with_limits(
    source: &[u8],
    format: ImageFormat,
) -> Result<(u32, u32), image::ImageError> {
    let mut reader = ImageReader::with_format(Cursor::new(source), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODED_DIMENSION);
    limits.max_image_height = Some(MAX_DECODED_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_ALLOCATION);
    reader.limits(limits);
    let dimensions = reader.into_dimensions()?;
    validate_decoded_dimensions(dimensions)
}

fn validate_decoded_dimensions(dimensions: (u32, u32)) -> Result<(u32, u32), image::ImageError> {
    let (width, height) = dimensions;
    let decoded_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            image::ImageError::Limits(image::error::LimitError::from_kind(
                image::error::LimitErrorKind::InsufficientMemory,
            ))
        })?;
    if decoded_bytes > MAX_DECODED_ALLOCATION {
        return Err(image::ImageError::Limits(
            image::error::LimitError::from_kind(image::error::LimitErrorKind::InsufficientMemory),
        ));
    }
    Ok(dimensions)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeifProbe {
    mime_type: &'static str,
    dimensions: Option<(u32, u32)>,
}

/// Return the normalized MIME for a recognized HEIF/HEIC `ftyp` box.
pub fn heif_mime_type(source: &[u8]) -> Option<&'static str> {
    probe_heif(source).map(|probe| probe.mime_type)
}

fn probe_heif(source: &[u8]) -> Option<HeifProbe> {
    let size = u32::from_be_bytes(source.get(0..4)?.try_into().ok()?) as usize;
    if size < 16 || size > source.len() || &source[4..8] != b"ftyp" {
        return None;
    }
    let major = source.get(8..12)?;
    let compatible = source.get(16..size).unwrap_or_default();
    let is_heic = [b"heic", b"heix", b"hevc", b"hevx"]
        .iter()
        .any(|brand| major == *brand || compatible.chunks_exact(4).any(|item| item == *brand));
    let is_heif = major == b"mif1" || compatible.chunks_exact(4).any(|item| item == b"mif1");
    if !is_heic && !is_heif {
        return None;
    }
    Some(HeifProbe {
        mime_type: if is_heic { "image/heic" } else { "image/heif" },
        dimensions: find_ispe_dimensions(source),
    })
}

fn find_ispe_dimensions(source: &[u8]) -> Option<(u32, u32)> {
    let mut dimensions: Option<(u32, u32)> = None;
    for (type_offset, window) in source.windows(4).enumerate() {
        if window != b"ispe" || type_offset < 4 {
            continue;
        }
        let box_start = type_offset - 4;
        let Some(size_bytes) = source.get(box_start..type_offset) else {
            continue;
        };
        let Ok(size_bytes) = size_bytes.try_into() else {
            continue;
        };
        let size = u32::from_be_bytes(size_bytes) as usize;
        if size < 20 || box_start.checked_add(size)? > source.len() {
            continue;
        }
        let Some(width_bytes) = source.get(type_offset + 8..type_offset + 12) else {
            continue;
        };
        let Some(height_bytes) = source.get(type_offset + 12..type_offset + 16) else {
            continue;
        };
        let Ok(width_bytes) = width_bytes.try_into() else {
            continue;
        };
        let Ok(height_bytes) = height_bytes.try_into() else {
            continue;
        };
        let width = u32::from_be_bytes(width_bytes);
        let height = u32::from_be_bytes(height_bytes);
        if width > 0 && height > 0 {
            dimensions = Some(match dimensions {
                Some((old_width, old_height)) => (old_width.max(width), old_height.max(height)),
                None => (width, height),
            });
        }
    }
    dimensions
}

fn decode_heif(source: &[u8], probe: HeifProbe) -> Result<DynamicImage, ImagePreparationError> {
    let (width, height) = probe.dimensions.ok_or(ImagePreparationError::Decode)?;
    let allocation = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ImagePreparationError::Decode)?;
    if width > MAX_DECODED_DIMENSION
        || height > MAX_DECODED_DIMENSION
        || allocation > MAX_DECODED_ALLOCATION
    {
        return Err(ImagePreparationError::Decode);
    }
    let decoded = heif_oxide::decode_bytes(source).map_err(|_| ImagePreparationError::Decode)?;
    if decoded
        .color
        .nclx
        .is_some_and(|nclx| matches!(nclx.transfer, 16 | 18))
        || source
            .windows(b"gainmap".len())
            .any(|window| window == b"gainmap")
    {
        return Err(ImagePreparationError::Decode);
    }
    let decoded_allocation = u64::from(decoded.width)
        .checked_mul(u64::from(decoded.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ImagePreparationError::Decode)?;
    if decoded.width > MAX_DECODED_DIMENSION
        || decoded.height > MAX_DECODED_DIMENSION
        || decoded_allocation > MAX_DECODED_ALLOCATION
    {
        return Err(ImagePreparationError::Decode);
    }
    let pixels = decoded.to_rgba8();
    let image = RgbaImage::from_raw(decoded.width, decoded.height, pixels)
        .ok_or(ImagePreparationError::Decode)?;
    Ok(DynamicImage::ImageRgba8(image))
}
