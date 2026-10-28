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

use std::{borrow::Cow, io::Cursor};

use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat,
    ImageReader, Limits, RgbaImage,
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
    /// Decode-only source: WeChat serves a BMP under every image MIME it
    /// offers (#1630). Its Keep output is PNG.
    Bmp,
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
    /// The source format is deliberately not decoded (GIF, animated WebP/PNG,
    /// unrecognized). Distinct from [`Self::Decode`] so a caller can keep the
    /// bytes verbatim without also masking a decoder guard rejection (#1396).
    #[error("image format is not supported")]
    UnsupportedFormat,
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
        PreparedImageFormat::Heif | PreparedImageFormat::Bmp => true,
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

    let (decoded, icc_profile) = match source_format {
        PreparedImageFormat::Heif => (decode_heif(source, heif.expect("recognized HEIF"))?, None),
        _ => decode_with_limits(source, guessed.expect("recognized image format"))
            .map_err(|_| ImagePreparationError::Decode)?,
    };
    let target_format = match request.format {
        // A bitmap is not an upload format; keeping it means lossless PNG.
        ImageOutputFormat::Keep if source_format == PreparedImageFormat::Bmp => {
            PreparedImageFormat::Png
        }
        ImageOutputFormat::Keep => source_format,
        ImageOutputFormat::Png => PreparedImageFormat::Png,
        ImageOutputFormat::Jpeg => PreparedImageFormat::Jpeg,
        ImageOutputFormat::WebP => PreparedImageFormat::WebP,
    };
    let scaled = scale_linearly(&decoded, request.resize, target_format);
    let mut variant = encoded_variant(
        &request.identity(),
        filename,
        target_format,
        &scaled,
        policy.quality_percent,
        icc_profile.as_deref(),
    )?;
    variant.recommended = false;
    Ok(variant)
}

/// Encode one output fitted to a maximum long edge with a concrete encoding.
///
/// Unlike [`prepare_image_output`], whose resize is a linear fraction of the
/// source, the size here is a pixel budget, which is what a display slot needs.
/// The source is never upscaled: an image already within `target_long_edge`
/// keeps its own dimensions. Non-decodable content (GIF, animated WebP/PNG)
/// is reported as [`ImagePreparationError::UnsupportedFormat`] so the caller can
/// decide its fallback, while a decoder guard rejection stays
/// [`ImagePreparationError::Decode`] and must not become a pass-through.
pub fn prepare_image_output_fitted(
    source: &[u8],
    filename: &str,
    target_long_edge: u32,
    format: ImageOutputFormat,
    policy: &ImagePreparationPolicy,
) -> Result<PreparedImageVariant, ImagePreparationError> {
    if source.is_empty() {
        return Err(ImagePreparationError::Empty);
    }
    let target_format = match format {
        // Pixels carry no source encoding, so `Keep` cannot be resolved here.
        ImageOutputFormat::Keep => return Err(ImagePreparationError::Encode),
        ImageOutputFormat::Png => PreparedImageFormat::Png,
        ImageOutputFormat::Jpeg => PreparedImageFormat::Jpeg,
        ImageOutputFormat::WebP => PreparedImageFormat::WebP,
    };
    let heif = probe_heif(source);
    let guessed = image::guess_format(source).ok();
    let source_format = if heif.is_some() {
        PreparedImageFormat::Heif
    } else {
        prepared_format(guessed)
    };
    let decodable = match source_format {
        PreparedImageFormat::Heif | PreparedImageFormat::Bmp => true,
        PreparedImageFormat::Png | PreparedImageFormat::Jpeg | PreparedImageFormat::WebP => {
            !animated_webp(source) && !animated_png(source)
        }
        PreparedImageFormat::Gif | PreparedImageFormat::Other => false,
    };
    if !decodable {
        return Err(ImagePreparationError::UnsupportedFormat);
    }
    let (decoded, icc_profile) = match source_format {
        PreparedImageFormat::Heif => (decode_heif(source, heif.expect("recognized HEIF"))?, None),
        _ => decode_with_limits(source, guessed.expect("recognized image format"))
            .map_err(|_| ImagePreparationError::Decode)?,
    };
    let long_edge = target_long_edge.max(1);
    let scaled = resize_to_long_edge(&decoded, long_edge, target_format);
    let mut variant = encoded_variant(
        &format!("fit-{long_edge}-{}", format.token()),
        filename,
        target_format,
        &scaled,
        policy.quality_percent,
        icc_profile.as_deref(),
    )?;
    variant.recommended = false;
    Ok(variant)
}
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
        scale_linearly(&image, request.resize, target_format)
    };
    encoded_variant(
        &request.identity(),
        filename,
        target_format,
        &scaled,
        policy.quality_percent,
        None,
    )
}

/// Halve each dimension per scale step, never below one pixel.
fn scale_linearly(
    image: &DynamicImage,
    resize: ImageResizeScale,
    format: PreparedImageFormat,
) -> DynamicImage {
    let image = jpeg_background(image, format);
    let divisor = resize.divisor();
    if divisor <= 1 {
        return image.into_owned();
    }
    let (width, height) = image.dimensions();
    let target_width = (width / divisor).max(1);
    let target_height = (height / divisor).max(1);
    if (target_width, target_height) == (width, height) {
        return image.into_owned();
    }
    resize_with_alpha(&image, target_width, target_height)
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
        PreparedImageFormat::Heif => decode_heif(source, heif.expect("recognized HEIF"))
            .ok()
            .map(|image| (image, None)),
        PreparedImageFormat::Png | PreparedImageFormat::Jpeg | PreparedImageFormat::WebP
            if !animated_webp(source) && !animated_png(source) =>
        {
            decode_with_limits(source, guessed.expect("recognized image format")).ok()
        }
        _ => None,
    };
    let dimensions = decoded
        .as_ref()
        .map(|(image, _)| image.dimensions())
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

    let Some((decoded, icc_profile)) = decoded else {
        variants[0].recommended = true;
        return Ok(variants);
    };
    let long_edge = policy.target_long_edge.max(1);
    let resized = resize_to_long_edge(&decoded, long_edge, format);
    match format {
        PreparedImageFormat::Png => {
            variants.push(encoded_variant(
                "resized-png",
                filename,
                PreparedImageFormat::Png,
                &resized,
                policy.quality_percent,
                icc_profile.as_deref(),
            )?);
            variants.push(encoded_variant(
                "webp",
                filename,
                PreparedImageFormat::WebP,
                &resized,
                policy.quality_percent,
                icc_profile.as_deref(),
            )?);
        }
        PreparedImageFormat::Jpeg => {
            variants.push(encoded_variant(
                "resized-jpeg",
                filename,
                PreparedImageFormat::Jpeg,
                &resized,
                policy.quality_percent,
                icc_profile.as_deref(),
            )?);
            variants.push(encoded_variant(
                "webp",
                filename,
                PreparedImageFormat::WebP,
                &resized,
                policy.quality_percent,
                icc_profile.as_deref(),
            )?);
        }
        PreparedImageFormat::WebP => variants.push(encoded_variant(
            "resized-webp",
            filename,
            PreparedImageFormat::WebP,
            &resized,
            policy.quality_percent,
            icc_profile.as_deref(),
        )?),
        PreparedImageFormat::Heif => {
            // The alpha-preserving variants below still use `resized`; JPEG
            // must composite the source before its own resampling.
            let jpeg = resize_to_long_edge(&decoded, long_edge, PreparedImageFormat::Jpeg);
            variants.push(encoded_variant(
                "resized-jpeg",
                filename,
                PreparedImageFormat::Jpeg,
                &jpeg,
                policy.quality_percent,
                None,
            )?);
            variants.push(encoded_variant(
                "resized-webp",
                filename,
                PreparedImageFormat::WebP,
                &resized,
                policy.quality_percent,
                None,
            )?);
            variants.push(encoded_variant(
                "resized-png",
                filename,
                PreparedImageFormat::Png,
                &resized,
                policy.quality_percent,
                None,
            )?);
        }
        PreparedImageFormat::Gif | PreparedImageFormat::Bmp | PreparedImageFormat::Other => {}
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
    icc_profile: Option<&[u8]>,
) -> Result<PreparedImageVariant, ImagePreparationError> {
    let (width, height) = image.dimensions();
    let mut bytes = Vec::new();
    match format {
        PreparedImageFormat::Png => {
            let rgba = image.to_rgba8();
            with_color_profile(PngEncoder::new(&mut bytes), icc_profile)?
                .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
                .map_err(|_| ImagePreparationError::Encode)?;
        }
        PreparedImageFormat::Jpeg => {
            let rgb = jpeg_background(image, format).to_rgb8();
            with_color_profile(
                JpegEncoder::new_with_quality(&mut bytes, quality_percent.clamp(1, 100)),
                icc_profile,
            )?
            .write_image(&rgb, width, height, ExtendedColorType::Rgb8)
            .map_err(|_| ImagePreparationError::Encode)?;
        }
        PreparedImageFormat::WebP => {
            let rgba = image.to_rgba8();
            with_color_profile(WebPEncoder::new_lossless(&mut bytes), icc_profile)?
                .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
                .map_err(|_| ImagePreparationError::Encode)?;
        }
        PreparedImageFormat::Gif
        | PreparedImageFormat::Bmp
        | PreparedImageFormat::Heif
        | PreparedImageFormat::Other => {
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

/// Decoded dimensions of `source` without retaining pixels, for content that is
/// stored verbatim because it cannot be re-encoded (#1396). Returns `None` when
/// the format is unrecognized or the header cannot be read within the decode
/// guards.
pub fn probe_image_dimensions(source: &[u8]) -> Option<(u32, u32)> {
    if source.is_empty() {
        return None;
    }
    if let Some(heif) = probe_heif(source) {
        return decode_heif(source, heif)
            .ok()
            .map(|image| (image.width(), image.height()));
    }
    let format = image::guess_format(source).ok()?;
    read_dimensions_with_limits(source, format).ok()
}

/// Resolve JPEG's missing alpha channel before filtering, so invisible RGB
/// cannot bleed into edges. Other encodings keep their alpha untouched.
fn jpeg_background(image: &DynamicImage, format: PreparedImageFormat) -> Cow<'_, DynamicImage> {
    if format != PreparedImageFormat::Jpeg || !image.color().has_alpha() {
        return Cow::Borrowed(image);
    }
    let rgb = image::RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let alpha = u32::from(pixel[3]);
        image::Rgb(std::array::from_fn(|channel| {
            ((u32::from(pixel[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        }))
    });
    Cow::Owned(DynamicImage::ImageRgb8(rgb))
}

/// Filtering straight-alpha RGB would make invisible colors bleed into edges.
fn resize_with_alpha(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    if !image.color().has_alpha() {
        return image.resize_exact(width, height, FilterType::Lanczos3);
    }
    let mut pixels = image.to_rgba8();
    for pixel in pixels.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let mut resized = image::imageops::resize(&pixels, width, height, FilterType::Lanczos3);
    for pixel in resized.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    DynamicImage::ImageRgba8(resized)
}

fn resize_to_long_edge(
    image: &DynamicImage,
    target_long_edge: u32,
    format: PreparedImageFormat,
) -> DynamicImage {
    let image = jpeg_background(image, format);
    let (width, height) = image.dimensions();
    let long_edge = width.max(height);
    if long_edge <= target_long_edge {
        return image.into_owned();
    }
    let scale = target_long_edge as f64 / long_edge as f64;
    let target_width = ((width as f64 * scale).round() as u32).max(1);
    let target_height = ((height as f64 * scale).round() as u32).max(1);
    resize_with_alpha(&image, target_width, target_height)
}

fn prepared_format(format: Option<ImageFormat>) -> PreparedImageFormat {
    match format {
        Some(ImageFormat::Png) => PreparedImageFormat::Png,
        Some(ImageFormat::Jpeg) => PreparedImageFormat::Jpeg,
        Some(ImageFormat::WebP) => PreparedImageFormat::WebP,
        Some(ImageFormat::Gif) => PreparedImageFormat::Gif,
        Some(ImageFormat::Bmp) => PreparedImageFormat::Bmp,
        _ => PreparedImageFormat::Other,
    }
}

fn actual_mime(format: PreparedImageFormat) -> &'static str {
    match format {
        PreparedImageFormat::Png => "image/png",
        PreparedImageFormat::Jpeg => "image/jpeg",
        PreparedImageFormat::WebP => "image/webp",
        PreparedImageFormat::Gif => "image/gif",
        PreparedImageFormat::Bmp => "image/bmp",
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
        PreparedImageFormat::Bmp => "bmp",
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
    if source.len() < 12 || &source[..4] != b"RIFF" || &source[8..12] != b"WEBP" {
        return false;
    }
    let length = u32::from_le_bytes(source[4..8].try_into().expect("four-byte RIFF length"));
    let Some(riff) = (length as usize)
        .checked_add(8)
        .and_then(|end| source.get(..end))
    else {
        return false;
    };
    let mut offset = 12usize;
    while let Some(header) = riff.get(offset..offset.saturating_add(8)) {
        let length =
            u32::from_le_bytes(header[4..8].try_into().expect("four-byte chunk length")) as usize;
        let Some(end) = offset
            .checked_add(8)
            .and_then(|start| start.checked_add(length))
        else {
            return false;
        };
        let Some(body) = riff.get(offset + 8..end) else {
            return false;
        };
        if &header[..4] == b"ANIM"
            || (&header[..4] == b"VP8X" && body.first().is_some_and(|flags| flags & 0x02 != 0))
        {
            return true;
        }
        let Some(next) = end.checked_add(length % 2) else {
            return false;
        };
        offset = next;
    }
    false
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

fn with_color_profile<E: ImageEncoder>(
    mut encoder: E,
    profile: Option<&[u8]>,
) -> Result<E, ImagePreparationError> {
    if let Some(profile) = profile {
        encoder
            .set_icc_profile(profile.to_vec())
            .map_err(|_| ImagePreparationError::Encode)?;
    }
    Ok(encoder)
}

/// Pixel encoders here produce RGB, so a profile for another channel model
/// must use the existing original-file fallback instead of mislabelling RGB.
fn rgb_icc_profile(decoder: &mut impl ImageDecoder) -> image::ImageResult<Option<Vec<u8>>> {
    let profile = decoder.icc_profile()?;
    if profile.as_ref().is_some_and(|profile| {
        profile.get(16..20) != Some(b"RGB ")
            || decoder.original_color_type() == ExtendedColorType::Cmyk8
    }) {
        return Err(image::ImageError::Unsupported(
            image::error::UnsupportedError::from_format_and_kind(
                image::error::ImageFormatHint::Unknown,
                image::error::UnsupportedErrorKind::GenericFeature(
                    "non-RGB color profile".to_owned(),
                ),
            ),
        ));
    }
    Ok(profile)
}

fn decode_with_limits(
    source: &[u8],
    format: ImageFormat,
) -> Result<(DynamicImage, Option<Vec<u8>>), image::ImageError> {
    let mut reader = ImageReader::with_format(Cursor::new(source), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODED_DIMENSION);
    limits.max_image_height = Some(MAX_DECODED_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_ALLOCATION);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let profile = rgb_icc_profile(&mut decoder)?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok((image, profile))
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
    let mut decoder = reader.into_decoder()?;
    let (width, height) = validate_decoded_dimensions(decoder.dimensions())?;
    let orientation = decoder.orientation()?;
    use image::metadata::Orientation;
    Ok(match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    })
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

/// Categorical name of the encoding `source` actually carries, judged by its
/// content rather than any declared MIME. Diagnostics use it to tell a
/// mislabeled attachment from a corrupt one without recording any bytes.
pub fn detected_image_format_token(source: &[u8]) -> &'static str {
    if probe_heif(source).is_some() {
        return "heif";
    }
    match image::guess_format(source) {
        Ok(ImageFormat::Png) => "png",
        Ok(ImageFormat::Jpeg) => "jpeg",
        Ok(ImageFormat::WebP) => "webp",
        Ok(ImageFormat::Gif) => "gif",
        Ok(ImageFormat::Bmp) => "bmp",
        Ok(ImageFormat::Tiff) => "tiff",
        Ok(ImageFormat::Avif) => "avif",
        Ok(_) => "other",
        Err(_) => "unknown",
    }
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
