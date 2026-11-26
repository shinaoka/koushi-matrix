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

/// Previews go through the protocol; everything else is saved rather than
/// rendered, so only renderable types need a token. #2096 added the audio types
/// the inline player needs, and text is not here at all: a text preview reads a
/// bounded string through a command, never a URL.
const MIME_TOKENS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("m4v", "video/mp4"),
    ("mp3", "audio/mpeg"),
    ("m4a", "audio/mp4"),
    ("aac", "audio/aac"),
    ("wav", "audio/wav"),
    ("flac", "audio/flac"),
    ("ogg", "audio/ogg"),
    ("opus", "audio/ogg"),
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
    let total = bytes.len() as u64;
    // #2096: a media element expects to seek. Answering only whole bodies makes
    // AVFoundation and WebView2 refuse to start a <video> or <audio> at all, so
    // one byte range is honoured here. Anything the parser does not understand
    // falls back to the whole body, which RFC 9110 allows.
    let wanted = requested_range(request, total);
    let (status, body, content_range) = match wanted {
        None => (StatusCode::OK, bytes, None),
        Some(Ok((start, end))) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[start as usize..=end as usize].to_vec(),
            Some(format!("bytes {start}-{end}/{total}")),
        ),
        Some(Err(())) => {
            let mut response = error_response(StatusCode::RANGE_NOT_SATISFIABLE);
            if let Ok(value) = tauri::http::HeaderValue::from_str(&format!("bytes */{total}")) {
                response
                    .headers_mut()
                    .insert(tauri::http::header::CONTENT_RANGE, value);
            }
            return response;
        }
    };

    let mut response = Response::builder()
        .status(status)
        .header(tauri::http::header::CONTENT_TYPE, mime_type)
        // Decrypted downloaded bytes stay out of the WebView HTTP cache, like
        // the thumbnail protocol.
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .header(tauri::http::header::ACCEPT_RANGES, "bytes")
        .header("X-Content-Type-Options", "nosniff");
    if let Some(range) = content_range {
        response = response.header(tauri::http::header::CONTENT_RANGE, range);
    }
    response.body(body).expect("media download response")
}

/// `None` when the whole body should be sent, `Some(Ok((start, end)))` for a
/// satisfiable range, `Some(Err(()))` when the range cannot be satisfied.
fn requested_range(request: &Request<Vec<u8>>, total: u64) -> Option<Result<(u64, u64), ()>> {
    let header = request
        .headers()
        .get(tauri::http::header::RANGE)?
        .to_str()
        .ok()?;
    let spec = header.trim().strip_prefix("bytes=")?.trim();
    // Multi-range responses would need a multipart body; one range or none.
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    if total == 0 {
        return Some(Err(()));
    }
    match (start.trim(), end.trim()) {
        // `bytes=-n`: the last n bytes.
        ("", suffix) => {
            let suffix: u64 = suffix.parse().ok()?;
            if suffix == 0 {
                return Some(Err(()));
            }
            Some(Ok((total.saturating_sub(suffix), total - 1)))
        }
        (from, to) => {
            let from: u64 = from.parse().ok()?;
            if from >= total {
                return Some(Err(()));
            }
            let to = if to.is_empty() {
                total - 1
            } else {
                to.parse::<u64>().ok()?.min(total - 1)
            };
            if to < from {
                return Some(Err(()));
            }
            Some(Ok((from, to)))
        }
    }
}

fn error_response(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Vec::new())
        .expect("media download error response")
}

/// `/download/<token>/<room-hash>/<event-hash>[/<width>x<height>]` ->
/// `(token, source_ref)`. The optional size names the quote-thumbnail entry
/// (#1769) through the same core ref grammar as the original (#1770).
fn parse_protocol_path(path: &str) -> Option<(&str, String)> {
    let mut parts = path.trim_start_matches('/').split('/');
    if parts.next()? != "download" {
        return None;
    }
    let token = parts.next()?;
    let room_hash = parts.next()?;
    let event_hash = parts.next()?;
    let source_ref = match parts.next() {
        None => koushi_core::media_download_ref::media_download_ref(room_hash, event_hash),
        Some(size) => {
            let (width, height) = size.split_once('x')?;
            let width = width.parse().ok()?;
            let height = height.parse().ok()?;
            koushi_core::media_download_ref::media_download_thumbnail_ref(
                room_hash, event_hash, width, height,
            )?
        }
    };
    if parts.next().is_some() {
        return None;
    }
    Some((token, source_ref))
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

/// Largest file an inline text preview will read (#2096).
///
/// The card decides from the DTO's byte size, so a file above this is never
/// previewed and never fetched: the attachment keeps the plain download card.
/// The command enforces the same limit so a stale size cannot make the renderer
/// read an unbounded file.
pub(crate) const MAX_TEXT_PREVIEW_BYTES: u64 = 256 * 1024;

/// Read a downloaded text attachment for an inline preview (#2096).
///
/// The bytes never leave Core's managed cache for anything but this bounded
/// string: no path is returned, and the file is refused rather than truncated so
/// a preview never shows a document that looks whole but is not.
pub(crate) fn read_managed_text(
    cache_roots: &[PathBuf],
    source_ref: &str,
    max_bytes: u64,
) -> Result<String, TextPreviewError> {
    let Some(file) = resolve_managed_file(cache_roots, source_ref.trim()) else {
        return Err(TextPreviewError::Unavailable);
    };
    let Ok(metadata) = std::fs::metadata(&file) else {
        return Err(TextPreviewError::Unavailable);
    };
    if metadata.len() > max_bytes {
        return Err(TextPreviewError::TooLarge);
    }
    let Ok(bytes) = std::fs::read(&file) else {
        return Err(TextPreviewError::Unavailable);
    };
    String::from_utf8(bytes).map_err(|_| TextPreviewError::NotText)
}

/// Read a downloaded Markdown attachment and render it for the document view (#2097).
///
/// It goes through [`read_managed_text`], so the 256 KB cap, the UTF-8 check and
/// the confinement to the managed cache are the stage-1 text preview's own; only
/// the rendering (and its sanitizing) is new, and it lives in Core.
pub(crate) fn read_managed_markdown(
    cache_roots: &[PathBuf],
    source_ref: &str,
    max_bytes: u64,
) -> Result<koushi_protocol::event::TimelineFormattedBody, TextPreviewError> {
    read_managed_text(cache_roots, source_ref, max_bytes)
        .map(|text| koushi_core::timeline::render_markdown_document(&text))
}

/// Why a text attachment cannot be previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextPreviewError {
    Unavailable,
    TooLarge,
    NotText,
}

impl TextPreviewError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "media download reference is unavailable",
            Self::TooLarge => "media file is too large to preview",
            Self::NotText => "media file is not text",
        }
    }
}

/// Largest PDF the viewer will read (#2096 stage 2), and the one place the
/// number lives on the Rust side; the renderer mirrors it for its card.
///
/// The file is refused above this rather than truncated: a truncated PDF can
/// parse and draw as a shorter, wrong document.
pub(crate) const MAX_PDF_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;

/// How far into the file `%PDF-` may start; readers tolerate leading junk.
const PDF_HEADER_SEARCH_BYTES: usize = 1024;

/// Read a downloaded PDF's bytes for the in-app viewer (#2096 stage 2).
///
/// Same resolution as [`read_managed_text`]: the opaque ref is resolved inside
/// the account's managed cache and no path is returned. A body without a PDF
/// header is refused here so the renderer's parser never sees arbitrary bytes.
pub(crate) fn read_managed_pdf(
    cache_roots: &[PathBuf],
    source_ref: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, PdfPreviewError> {
    let Some(file) = resolve_managed_file(cache_roots, source_ref.trim()) else {
        return Err(PdfPreviewError::Unavailable);
    };
    let Ok(metadata) = std::fs::metadata(&file) else {
        return Err(PdfPreviewError::Unavailable);
    };
    if metadata.len() > max_bytes {
        return Err(PdfPreviewError::TooLarge);
    }
    let Ok(bytes) = std::fs::read(&file) else {
        return Err(PdfPreviewError::Unavailable);
    };
    // The file may have grown between the metadata check and the read.
    if bytes.len() as u64 > max_bytes {
        return Err(PdfPreviewError::TooLarge);
    }
    let head = &bytes[..bytes.len().min(PDF_HEADER_SEARCH_BYTES)];
    if !head.windows(5).any(|window| window == b"%PDF-") {
        return Err(PdfPreviewError::NotPdf);
    }
    Ok(bytes)
}

/// Why a PDF attachment cannot be previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PdfPreviewError {
    Unavailable,
    TooLarge,
    NotPdf,
}

impl PdfPreviewError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "media download reference is unavailable",
            Self::TooLarge => "media file is too large to preview",
            Self::NotPdf => "media file is not a PDF",
        }
    }
}

/// Resolve a renderer-supplied ref for the save flow.
pub(crate) fn cache_path_for_tab(cache_root: &Path, source_ref: &str) -> Option<PathBuf> {
    let path =
        koushi_core::media_download_ref::media_download_cache_path(cache_root, source_ref.trim())?;
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests;
