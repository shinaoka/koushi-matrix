//! Platform still-image decoding port for media preparation.
//!
//! Core owns when a source is decoded and what happens to the result; an
//! adapter may inject a platform decoder (for example ImageIO on macOS) that
//! turns source bytes into normalized pixels. No native handle crosses this
//! boundary: the decoder returns owned pixels or a typed failure.

pub use koushi_media::{DecodeLimits, DecodedRgbaImage, DecodedRgbaImageError};

/// Why a platform decoder produced no pixels. Variants are categories only;
/// raw platform errors never cross the boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum NativeImageDecodeError {
    /// The platform recognizes the source but cannot convert it faithfully,
    /// for example HDR content without an SDR decode on this OS version.
    #[error("the platform decoder does not support this image")]
    Unsupported,
    /// The platform decoder could not be used at all.
    #[error("the platform decoder is unavailable")]
    Unavailable,
    /// The source is not a decodable still image.
    #[error("the image is malformed")]
    Malformed,
    /// The primary image exceeds the requested decode limits.
    #[error("the image exceeds its decode limits")]
    TooLarge,
}

impl NativeImageDecodeError {
    /// Stable diagnostic token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::Malformed => "malformed",
            Self::TooLarge => "too_large",
        }
    }
}

impl From<DecodedRgbaImageError> for NativeImageDecodeError {
    fn from(error: DecodedRgbaImageError) -> Self {
        match error {
            DecodedRgbaImageError::TooLarge => Self::TooLarge,
            DecodedRgbaImageError::InvalidLayout => Self::Malformed,
        }
    }
}

/// Decodes the primary still image of an HEIF/HEIC source.
///
/// Implementations must check `limits` against the primary image's dimensions
/// before allocating its pixels, decode only the primary still (never
/// sequences, auxiliary or depth images), apply its orientation, and return
/// SDR sRGB pixels as a [`DecodedRgbaImage`]. They run on the media
/// preparation worker, never on a UI thread, and must release every platform
/// object on all paths.
pub trait NativeStillImageDecoder: Send + Sync {
    /// Fixed backend name for diagnostics.
    fn backend(&self) -> &'static str;

    fn decode(
        &self,
        source: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodedRgbaImage, NativeImageDecodeError>;
}
