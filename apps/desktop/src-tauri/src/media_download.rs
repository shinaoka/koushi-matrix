//! Platform-side resolution of opaque timeline media download refs (#1619).
//!
//! Core keeps the managed cache file and hands React only
//! `download/<room-hash>/<event-hash>` (the original) or
//! `download/<room-hash>/<event-hash>/<width>x<height>` (a thumbnail, #1770).
//! This adapter maps such a ref to the cache file for saving and serves image
//! bytes through the `koushi-media` protocol. Local paths never cross into Core
//! state, events, commands, or the webview.

use std::path::{Path, PathBuf};

use tauri::http::{Request, Response, StatusCode};

/// Image previews go through the protocol; everything else is saved rather
/// than rendered, so only image types need a token.
const MIME_TOKENS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
];

/// A decoded preview cannot use more than this; refuse to buffer larger files
/// into the process. Saving such a file still works.
const MAX_PROTOCOL_BODY_BYTES: u64 = 128 * 1024 * 1024;

fn mime_for_token(token: &str) -> Option<&'static str> {
    MIME_TOKENS
        .iter()
        .find(|(candidate, _)| *candidate == token)
        .map(|(_, mime)| *mime)
}

pub(crate) fn protocol_response(
    request: &Request<Vec<u8>>,
    cache_roots: &[PathBuf],
) -> Response<Vec<u8>> {
    let path = request.uri().path();
    let Some((token, source_ref)) = parse_protocol_path(path) else {
        return error_response(StatusCode::NOT_FOUND);
    };
    let Some(mime_type) = mime_for_token(token) else {
        return error_response(StatusCode::NOT_FOUND);
    };
    let Some(file) = resolve_managed_file(cache_roots, &source_ref) else {
        return error_response(StatusCode::NOT_FOUND);
    };
    let Ok(metadata) = std::fs::metadata(&file) else {
        return error_response(StatusCode::NOT_FOUND);
    };
    if metadata.len() > MAX_PROTOCOL_BODY_BYTES {
        return error_response(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let Ok(bytes) = std::fs::read(&file) else {
        return error_response(StatusCode::NOT_FOUND);
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(tauri::http::header::CONTENT_TYPE, mime_type)
        // Decrypted downloaded bytes stay out of the WebView HTTP cache, like
        // the thumbnail protocol.
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(bytes)
        .expect("media download response")
}

fn error_response(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Vec::new())
        .expect("media download error response")
}

/// `/download/<token>/<room-hash>/<event-hash>` -> `(token, source_ref)`.
fn parse_protocol_path(path: &str) -> Option<(&str, String)> {
    let mut parts = path.trim_start_matches('/').split('/');
    if parts.next()? != "download" {
        return None;
    }
    let token = parts.next()?;
    let room_hash = parts.next()?;
    let event_hash = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((
        token,
        koushi_core::media_download_ref::media_download_ref(room_hash, event_hash),
    ))
}

/// Find the ref in one of the account cache roots, refusing symlinked escapes.
fn resolve_managed_file(cache_roots: &[PathBuf], source_ref: &str) -> Option<PathBuf> {
    for root in cache_roots {
        let Some(candidate) =
            koushi_core::media_download_ref::media_download_cache_path(root, source_ref)
        else {
            continue;
        };
        let Ok(canonical_root) = root.canonicalize() else {
            continue;
        };
        let Ok(canonical_file) = candidate.canonicalize() else {
            continue;
        };
        if canonical_file.starts_with(&canonical_root) && canonical_file.is_file() {
            return Some(canonical_file);
        }
    }
    None
}

/// Resolve a renderer-supplied ref for the save flow.
pub(crate) fn cache_path_for_tab(cache_root: &Path, source_ref: &str) -> Option<PathBuf> {
    let path =
        koushi_core::media_download_ref::media_download_cache_path(cache_root, source_ref.trim())?;
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str) -> Request<Vec<u8>> {
        Request::builder()
            .uri(format!("koushi-media://localhost{path}"))
            .body(Vec::new())
            .expect("request")
    }

    fn cache_with_file(file_name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("media-downloads");
        std::fs::create_dir_all(root.join("1234567890123456")).expect("room dir");
        std::fs::write(root.join("1234567890123456").join(file_name), bytes).expect("cache file");
        (temp, root)
    }

    #[test]
    fn serves_an_image_from_the_managed_cache() {
        let (temp, root) = cache_with_file("6543210987654321.bin", b"\x89PNG\r\n\x1a\nrest");
        let response = protocol_response(
            &request("/download/png/1234567890123456/6543210987654321"),
            std::slice::from_ref(&root),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(tauri::http::header::CONTENT_TYPE),
            Some(&"image/png".parse().expect("header value"))
        );
        assert_eq!(
            response.headers().get("X-Content-Type-Options"),
            Some(&"nosniff".parse().expect("header value"))
        );
        assert!(response.body().starts_with(b"\x89PNG"));
        drop(temp);
    }

    #[test]
    fn rejects_unknown_tokens_and_traversal_refs() {
        let (temp, root) = cache_with_file("6543210987654321.bin", b"x");
        for path in [
            "/download/tiff/1234567890123456/6543210987654321",
            "/download/png/1234567890123456/6543210987654321/extra",
            "/download/png/../../etc/passwd",
            "/download/png/1234567890123456/..",
            "/asset/png/1234567890123456/6543210987654321",
        ] {
            let response = protocol_response(&request(path), std::slice::from_ref(&root));
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert!(response.body().is_empty());
        }
        drop(temp);
    }

    /// #1770: a thumbnail ref is its own managed entry, and the save flow
    /// resolves it through the same rule the writer used.
    #[test]
    fn resolves_a_thumbnail_ref_to_its_own_cache_file() {
        let (temp, root) = cache_with_file("6543210987654321.bin", b"original");
        let thumbnail = root
            .join("1234567890123456")
            .join("6543210987654321-800x800.bin");
        std::fs::write(&thumbnail, b"thumbnail").expect("thumbnail file");

        assert_eq!(
            cache_path_for_tab(&root, "download/1234567890123456/6543210987654321/800x800"),
            Some(thumbnail)
        );
        assert_eq!(
            cache_path_for_tab(&root, "download/1234567890123456/6543210987654321"),
            Some(root.join("1234567890123456").join("6543210987654321.bin"))
        );
        // The `koushi-media` protocol still serves only the original: no GUI
        // path requests a thumbnail download (#1769 owns that surface).
        let response = protocol_response(
            &request("/download/png/1234567890123456/6543210987654321/800x800"),
            std::slice::from_ref(&root),
        );
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        drop(temp);
    }

    #[test]
    fn rejects_a_symlink_that_escapes_the_cache_root() {
        let (temp, root) = cache_with_file("6543210987654321.bin", b"x");
        let outside = temp.path().join("outside.bin");
        std::fs::write(&outside, b"secret").expect("outside file");
        let link = root.join("1234567890123456").join("1111111111111111.bin");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, &link).expect("symlink");
            let response = protocol_response(
                &request("/download/png/1234567890123456/1111111111111111"),
                std::slice::from_ref(&root),
            );
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        #[cfg(not(unix))]
        {
            // Windows junctions and symlinks need privileges; the hosted
            // Windows gate covers those assumptions instead.
            assert!(!link.exists());
            drop(outside);
        }
        drop(temp);
    }
}
