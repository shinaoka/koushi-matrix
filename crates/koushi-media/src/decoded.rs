use crate::{MAX_DECODED_ALLOCATION, MAX_DECODED_DIMENSION};

/// Decoded-dimension and allocation bounds a decoder must enforce before it
/// allocates a full pixel buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    /// Largest accepted width or height, in pixels.
    pub max_dimension: u32,
    /// Largest accepted RGBA8 buffer (`width * height * 4`), in bytes.
    pub max_allocation_bytes: u64,
}

impl Default for DecodeLimits {
    /// The same limits the pure decoders in this crate apply.
    fn default() -> Self {
        Self {
            max_dimension: MAX_DECODED_DIMENSION,
            max_allocation_bytes: MAX_DECODED_ALLOCATION,
        }
    }
}

impl DecodeLimits {
    /// Whether an RGBA8 image of these dimensions fits within the limits.
    pub fn admits(&self, width: u64, height: u64) -> bool {
        width > 0
            && height > 0
            && width <= u64::from(self.max_dimension)
            && height <= u64::from(self.max_dimension)
            && width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(4))
                .is_some_and(|bytes| bytes <= self.max_allocation_bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DecodedRgbaImageError {
    #[error("decoded image exceeds its limits")]
    TooLarge,
    #[error("decoded pixel buffer does not match its layout")]
    InvalidLayout,
}

/// A decoded still image: 8-bit sRGB RGBA with straight (non-premultiplied)
/// alpha, tightly packed rows, and any orientation already applied.
///
/// This is the only pixel shape a platform decoder hands to the encoders here,
/// so it carries no native handles and no source metadata.
#[derive(Clone, Eq, PartialEq)]
pub struct DecodedRgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl std::fmt::Debug for DecodedRgbaImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedRgbaImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("byte_count", &self.pixels.len())
            .finish()
    }
}

impl DecodedRgbaImage {
    /// Adopt straight-alpha RGBA rows of `stride` bytes each.
    pub fn from_straight_rgba(
        width: u32,
        height: u32,
        stride: usize,
        pixels: Vec<u8>,
        limits: DecodeLimits,
    ) -> Result<Self, DecodedRgbaImageError> {
        Self::from_rows(width, height, stride, pixels, limits)
    }

    /// Adopt premultiplied-alpha RGBA rows of `stride` bytes each and convert
    /// them to straight alpha.
    ///
    /// 8-bit RGBA bitmap contexts on some platforms only render premultiplied
    /// alpha; this keeps that representation out of the encoders.
    pub fn from_premultiplied_rgba(
        width: u32,
        height: u32,
        stride: usize,
        pixels: Vec<u8>,
        limits: DecodeLimits,
    ) -> Result<Self, DecodedRgbaImageError> {
        let mut image = Self::from_rows(width, height, stride, pixels, limits)?;
        for pixel in image.pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            if alpha == 0 {
                pixel[..3].fill(0);
            } else if alpha < 255 {
                for channel in &mut pixel[..3] {
                    let straight = (u32::from(*channel) * 255 + alpha / 2) / alpha;
                    *channel = straight.min(255) as u8;
                }
            }
        }
        Ok(image)
    }

    fn from_rows(
        width: u32,
        height: u32,
        stride: usize,
        mut pixels: Vec<u8>,
        limits: DecodeLimits,
    ) -> Result<Self, DecodedRgbaImageError> {
        if !limits.admits(u64::from(width), u64::from(height)) {
            return Err(DecodedRgbaImageError::TooLarge);
        }
        let row_bytes = width as usize * 4;
        let rows = height as usize;
        let required = stride
            .checked_mul(rows - 1)
            .and_then(|bytes| bytes.checked_add(row_bytes))
            .ok_or(DecodedRgbaImageError::InvalidLayout)?;
        if stride < row_bytes || pixels.len() < required {
            return Err(DecodedRgbaImageError::InvalidLayout);
        }
        if stride != row_bytes {
            for row in 1..rows {
                pixels.copy_within(row * stride..row * stride + row_bytes, row * row_bytes);
            }
        }
        pixels.truncate(row_bytes * rows);
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Tightly packed straight-alpha RGBA8 rows.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}
