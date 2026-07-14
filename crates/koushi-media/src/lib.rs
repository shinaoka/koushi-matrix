use std::io::Cursor;

use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat, ImageReader,
    Limits,
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
    #[error("image encoding failed")]
    Encode,
}

pub fn prepare_image_variants(
    source: &[u8],
    filename: &str,
    declared_mime: &str,
    policy: &ImagePreparationPolicy,
) -> Result<Vec<PreparedImageVariant>, ImagePreparationError> {
    if source.is_empty() {
        return Err(ImagePreparationError::Empty);
    }

    let guessed = image::guess_format(source).ok();
    let format = prepared_format(guessed);
    let _ = declared_mime;
    let mime_type = actual_mime(format);
    let decoded = match format {
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
        filename: normalized_filename(filename, extension(format)),
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
        PreparedImageFormat::Gif | PreparedImageFormat::Other => {
            return Err(ImagePreparationError::Encode);
        }
    }
    Ok(PreparedImageVariant {
        id: id.to_owned(),
        filename: normalized_filename(source_filename, extension(format)),
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
        PreparedImageFormat::Other => "application/octet-stream",
    }
}

fn extension(format: PreparedImageFormat) -> &'static str {
    match format {
        PreparedImageFormat::Png => "png",
        PreparedImageFormat::Jpeg => "jpg",
        PreparedImageFormat::WebP => "webp",
        PreparedImageFormat::Gif => "gif",
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
