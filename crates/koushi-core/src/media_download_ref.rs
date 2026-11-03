//! Opaque references for downloaded timeline media (#1619, #1770).
//!
//! A download writes a managed cache file under
//! `<account data dir>/media-downloads/<room-hash>/`. Core state and events name
//! that entry with an opaque ref, never with a filesystem path:
//!
//! - `download/<room-hash>/<event-hash>` — the original file.
//! - `download/<room-hash>/<event-hash>/<width>x<height>` — a thumbnail of that
//!   event, so the two selections never share an entry (#1770).
//!
//! The platform adapter resolves the ref against the account cache roots it
//! already owns; paths stay out of Core state, events, commands, diagnostics,
//! and the webview.

use std::path::{Path, PathBuf};

pub const MEDIA_DOWNLOAD_REF_PREFIX: &str = "download";

/// Separates the requested thumbnail size into width and height.
const THUMBNAIL_SIZE_SEPARATOR: char = 'x';

/// Build the opaque ref for one event's downloaded original (#1619).
pub fn media_download_ref(room_hash: &str, event_hash: &str) -> String {
    format!("{MEDIA_DOWNLOAD_REF_PREFIX}/{room_hash}/{event_hash}")
}

/// Build the opaque ref for one event's downloaded thumbnail (#1770).
///
/// The requested size is part of the key, so a thumbnail keeps its own entry
/// instead of the original's and two sizes keep their own instead of each
/// other's. It keys the *selection*: an event that carries its own thumbnail
/// source serves those bytes for any requested size, so the component is the
/// request, not a promise about the stored pixels. `None` when a size does not
/// fit the ref grammar, which only a caller bug can produce.
pub fn media_download_thumbnail_ref(
    room_hash: &str,
    event_hash: &str,
    width: u64,
    height: u64,
) -> Option<String> {
    let size = format!("{width}{THUMBNAIL_SIZE_SEPARATOR}{height}");
    validated_size_component(&size)?;
    Some(format!(
        "{MEDIA_DOWNLOAD_REF_PREFIX}/{room_hash}/{event_hash}/{size}"
    ))
}

/// Resolve a ref inside one account's media cache root.
///
/// Refuses anything that is not `download/<hash>/<hash>` or
/// `download/<hash>/<hash>/<width>x<height>`, so a renderer-supplied value
/// cannot escape the managed cache directory.
pub fn media_download_cache_path(cache_root: &Path, source_ref: &str) -> Option<PathBuf> {
    let mut parts = source_ref.split('/');
    if parts.next()? != MEDIA_DOWNLOAD_REF_PREFIX {
        return None;
    }
    let room_hash = parts.next()?;
    let event_hash = parts.next()?;
    let thumbnail_size = parts.next();
    if parts.next().is_some() || !is_path_hash(room_hash) || !is_path_hash(event_hash) {
        return None;
    }
    let file_name = match thumbnail_size {
        None => format!("{event_hash}.bin"),
        Some(size) => format!("{event_hash}-{}.bin", validated_size_component(size)?),
    };
    Some(cache_root.join(room_hash).join(file_name))
}

/// `sanitize_matrix_id_for_path` emits up to 16 decimal digits.
fn is_path_hash(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16 && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// Accept only `<hash>x<hash>`: the same digits-only rule as the other
/// components, so a size can never add a path segment or a `..`.
fn validated_size_component(size: &str) -> Option<&str> {
    let (width, height) = size.split_once(THUMBNAIL_SIZE_SEPARATOR)?;
    (is_path_hash(width) && is_path_hash(height)).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_resolves_a_ref_inside_the_cache_root() {
        let root = Path::new("/cache/media-downloads");
        let source_ref = media_download_ref("1234567890123456", "6543210987654321");
        assert_eq!(source_ref, "download/1234567890123456/6543210987654321");
        assert_eq!(
            media_download_cache_path(root, &source_ref),
            Some(root.join("1234567890123456").join("6543210987654321.bin"))
        );
    }

    /// #1770: the two selections of one event must resolve to distinct files.
    #[test]
    fn a_thumbnail_resolves_beside_the_original_not_onto_it() {
        let root = Path::new("/cache/media-downloads");
        let file = media_download_ref("1234567890123456", "6543210987654321");
        let thumbnail =
            media_download_thumbnail_ref("1234567890123456", "6543210987654321", 800, 800)
                .expect("an 800x800 thumbnail is nameable");
        assert_eq!(
            thumbnail,
            "download/1234567890123456/6543210987654321/800x800"
        );
        assert_eq!(
            media_download_cache_path(root, &thumbnail),
            Some(
                root.join("1234567890123456")
                    .join("6543210987654321-800x800.bin")
            )
        );
        assert_ne!(
            media_download_cache_path(root, &file),
            media_download_cache_path(root, &thumbnail)
        );
    }

    #[test]
    fn rejects_refs_that_could_escape_the_cache_root() {
        let root = Path::new("/cache/media-downloads");
        for source_ref in [
            "",
            "download",
            "download/1",
            "download/1/2/3",
            "download/../../etc/passwd/.bin",
            "download/../1",
            "download/1/..",
            "download/12345678901234567/1",
            "download/abcdef/123",
            "download/12 3/456",
            "asset://download/1/2",
            "/download/1/2",
            "download/1/2\u{0}",
            "download/1/2/",
            "download/1/2/x",
            "download/1/2/800",
            "download/1/2/800x",
            "download/1/2/x800",
            "download/1/2/800x800/1",
            "download/1/2/800x800x1",
            "download/1/2/12345678901234567x1",
            "download/1/2/../../etc/passwd",
            "download/1/2/8 0x800",
        ] {
            assert_eq!(
                media_download_cache_path(root, source_ref),
                None,
                "{source_ref:?} must not resolve"
            );
        }
    }
}
