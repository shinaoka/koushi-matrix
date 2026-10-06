//! Core media-save policy over a platform filesystem port.
//!
//! The port supplies filesystem operations; this module owns validation,
//! canonical containment, operation ordering, and private-safe error mapping.

use std::path::{Path, PathBuf};

/// Private-safe classification of one filesystem operation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MediaSaveIoError {
    #[error("filesystem object was not found")]
    NotFound,
    #[error("filesystem permission was denied")]
    PermissionDenied,
    #[error("filesystem object already exists")]
    AlreadyExists,
    #[error("filesystem input was invalid")]
    InvalidInput,
    #[error("filesystem operation failed")]
    Other,
}

/// The filesystem operations required by the media-save policy.
///
/// Implementations belong to platform adapters. In particular, Core does not
/// select a filesystem backend or perform filesystem syscalls itself.
pub trait MediaSaveFilesystem: Send + Sync {
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, MediaSaveIoError>;
    fn create_dir_all(&self, path: &Path) -> Result<(), MediaSaveIoError>;
    fn copy(&self, source: &Path, destination: &Path) -> Result<(), MediaSaveIoError>;
}

/// Private-safe policy failure for saving downloaded media.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MediaSaveError {
    #[error("media source is empty")]
    SourceEmpty,
    #[error("media source must be a local cache path")]
    SourceUrl,
    #[error("media source must be an absolute cache path")]
    SourceRelative,
    #[error("media cache is unavailable: {0}")]
    CacheCanonicalize(MediaSaveIoError),
    #[error("media source could not be read: {0}")]
    SourceCanonicalize(MediaSaveIoError),
    #[error("media source is outside the download cache")]
    SourceOutsideCache,
    #[error("media save destination is empty")]
    DestinationEmpty,
    #[error("media save destination must be absolute")]
    DestinationRelative,
    #[error("media save destination parent could not be created: {0}")]
    DestinationParent(MediaSaveIoError),
    #[error("media file could not be saved: {0}")]
    Copy(MediaSaveIoError),
}

/// Replace path separators and Windows-forbidden filename characters.
pub fn safe_media_save_filename(input: &str) -> String {
    let trimmed = input.trim();
    let candidate = if trimmed.is_empty() {
        "download"
    } else {
        trimmed
    };
    candidate
        .chars()
        .map(|character| match character {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect()
}

/// Media kinds the default save name distinguishes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaSaveKind {
    Image,
    File,
}

/// Platform-resolved facts the default save name needs beyond the filename.
pub struct MediaSaveNameFacts<'a> {
    /// Catalog-owned prefix for a generated image name, for example
    /// `Koushi_Image`. The renderer resolves it so product text stays in the
    /// catalog while the naming policy stays here.
    pub local_name_prefix: &'a str,
    /// The item's media kind.
    pub kind: MediaSaveKind,
    /// The event's `origin_server_ts`, when the platform knows it.
    pub timestamp_ms: Option<u64>,
    /// That instant's UTC offset in minutes, as the platform resolved it.
    pub utc_offset_minutes: i32,
}

/// Image stems that identify a client's generic upload name rather than a name a
/// person chose (#1135). Matched after trimming and lowercasing.
const GENERIC_IMAGE_STEMS: [&str; 4] = ["", "download", "image", "screenshot"];
const GENERIC_IMAGE_STEM_PREFIXES: [&str; 2] = ["clipboard", "pasted image"];

/// Propose the filename a save should start from.
///
/// A meaningful name is kept as-is. A generic image upload name (`image.png`,
/// `Pasted image 1.png`, `clipboard.png`, `screenshot.png`, or no name at all) is
/// replaced with a timestamped one built from the event's local time, so
/// successive saves do not all propose the same path.
pub fn default_media_save_filename(filename: &str, facts: MediaSaveNameFacts<'_>) -> String {
    let sanitized = safe_media_save_filename(filename);
    if facts.kind != MediaSaveKind::Image {
        return sanitized;
    }
    let (stem, extension) = split_media_filename(&sanitized);
    if !is_generic_image_stem(stem) {
        return sanitized;
    }
    let Some(timestamp_ms) = facts.timestamp_ms else {
        return sanitized;
    };
    let Some(stamp) = local_media_save_stamp(timestamp_ms, facts.utc_offset_minutes) else {
        return sanitized;
    };
    let prefix = safe_media_save_filename(facts.local_name_prefix);
    format!("{prefix}_{stamp}{extension}")
}

fn split_media_filename(filename: &str) -> (&str, &str) {
    // A leading dot starts a hidden name, not an extension.
    match filename.rfind('.') {
        Some(index) if index > 0 => (&filename[..index], &filename[index..]),
        _ => (filename, ""),
    }
}

fn is_generic_image_stem(stem: &str) -> bool {
    let normalized = stem.trim().to_lowercase();
    GENERIC_IMAGE_STEMS.contains(&normalized.as_str())
        || GENERIC_IMAGE_STEM_PREFIXES
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
}

/// `YYYYMMDD_HHMMSS` of `timestamp_ms` shifted by the platform's UTC offset. The
/// result is locale-independent on purpose: it is part of a generated filename,
/// not product text.
fn local_media_save_stamp(timestamp_ms: u64, utc_offset_minutes: i32) -> Option<String> {
    let shifted = i64::try_from(timestamp_ms)
        .ok()?
        .checked_add(i64::from(utc_offset_minutes).checked_mul(60_000)?)?;
    let stamp = jiff::Timestamp::from_millisecond(shifted).ok()?;
    Some(
        stamp
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y%m%d_%H%M%S")
            .to_string(),
    )
}

/// Build the default save destination from a safe filename.
pub fn default_media_save_path(filename: &str, downloads: Option<&Path>) -> PathBuf {
    let filename = safe_media_save_filename(filename);
    downloads
        .map(|directory| directory.join(&filename))
        .unwrap_or_else(|| PathBuf::from(filename))
}

/// Validate and copy a downloaded file through the platform filesystem port.
///
/// The source is admitted only after both cache and source paths have been
/// canonicalized and the canonical source remains within the canonical cache
/// root by path component. This rejects sibling-prefix and symlink escapes.
pub fn save_downloaded_media<P: MediaSaveFilesystem>(
    port: &P,
    cache_root: &Path,
    source: &Path,
    destination: &Path,
) -> Result<(), MediaSaveError> {
    validate_source(source)?;
    validate_destination(destination)?;

    let canonical_cache = port
        .canonicalize(cache_root)
        .map_err(MediaSaveError::CacheCanonicalize)?;
    let canonical_source = port
        .canonicalize(source)
        .map_err(MediaSaveError::SourceCanonicalize)?;
    if !canonical_source.starts_with(&canonical_cache) {
        return Err(MediaSaveError::SourceOutsideCache);
    }

    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        port.create_dir_all(parent)
            .map_err(MediaSaveError::DestinationParent)?;
    }
    port.copy(&canonical_source, destination)
        .map_err(MediaSaveError::Copy)
}

fn validate_source(source: &Path) -> Result<(), MediaSaveError> {
    if source.as_os_str().is_empty() {
        return Err(MediaSaveError::SourceEmpty);
    }
    if source.to_string_lossy().contains("://") {
        return Err(MediaSaveError::SourceUrl);
    }
    if !source.is_absolute() {
        return Err(MediaSaveError::SourceRelative);
    }
    Ok(())
}

fn validate_destination(destination: &Path) -> Result<(), MediaSaveError> {
    if destination.as_os_str().is_empty() {
        return Err(MediaSaveError::DestinationEmpty);
    }
    if !destination.is_absolute() {
        return Err(MediaSaveError::DestinationRelative);
    }
    Ok(())
}
