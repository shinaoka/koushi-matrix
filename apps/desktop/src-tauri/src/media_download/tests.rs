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

fn ranged_request(range: &str) -> Request<Vec<u8>> {
    Request::builder()
        .uri("koushi-media://localhost/download/mp4/1234567890123456/6543210987654321")
        .header(tauri::http::header::RANGE, range)
        .body(Vec::new())
        .expect("request")
}

/// #2096: a media element seeks, so one byte range must answer with 206 and
/// the rest with the whole body; AVFoundation refuses a range-less handler.
#[test]
fn answers_byte_ranges_for_media_elements() {
    let (_temp, root) = cache_with_file("6543210987654321.bin", b"0123456789");
    let full = protocol_response(
        &request("/download/mp4/1234567890123456/6543210987654321"),
        std::slice::from_ref(&root),
    );
    assert_eq!(full.status(), StatusCode::OK);
    assert_eq!(
        full.headers()
            .get(tauri::http::header::ACCEPT_RANGES)
            .unwrap(),
        "bytes"
    );
    assert!(
        full.headers()
            .get(tauri::http::header::CONTENT_RANGE)
            .is_none()
    );
    assert_eq!(full.body().as_slice(), b"0123456789");

    let bounded = protocol_response(&ranged_request("bytes=2-4"), std::slice::from_ref(&root));
    assert_eq!(bounded.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(bounded.body().as_slice(), b"234");
    assert_eq!(
        bounded
            .headers()
            .get(tauri::http::header::CONTENT_RANGE)
            .unwrap(),
        "bytes 2-4/10"
    );

    assert_eq!(
        protocol_response(&ranged_request("bytes=7-"), std::slice::from_ref(&root))
            .body()
            .as_slice(),
        b"789"
    );
    assert_eq!(
        protocol_response(&ranged_request("bytes=-3"), std::slice::from_ref(&root))
            .body()
            .as_slice(),
        b"789"
    );
    // An end past the file is clamped, not refused.
    assert_eq!(
        protocol_response(&ranged_request("bytes=8-99"), std::slice::from_ref(&root))
            .body()
            .as_slice(),
        b"89"
    );

    let unsatisfiable =
        protocol_response(&ranged_request("bytes=10-"), std::slice::from_ref(&root));
    assert_eq!(unsatisfiable.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        unsatisfiable
            .headers()
            .get(tauri::http::header::CONTENT_RANGE)
            .unwrap(),
        "bytes */10"
    );

    // Anything the parser does not understand still gets the whole file.
    for ignored in ["bytes=0-1,3-4", "items=0-1", "bytes=nonsense"] {
        let response = protocol_response(&ranged_request(ignored), std::slice::from_ref(&root));
        assert_eq!(response.status(), StatusCode::OK, "{ignored}");
        assert_eq!(response.body().as_slice(), b"0123456789", "{ignored}");
    }
}

/// #2096: an inline text preview reads a bounded string, never a path, and
/// refuses anything it cannot show whole.
#[test]
fn reads_managed_text_within_the_preview_limit() {
    let (temp, root) = cache_with_file("6543210987654321.bin", b"fn main() {}\nsecond\n");
    let text = read_managed_text(
        std::slice::from_ref(&root),
        "download/1234567890123456/6543210987654321",
        MAX_TEXT_PREVIEW_BYTES,
    )
    .expect("text");
    assert_eq!(text, "fn main() {}\nsecond\n");
    // At the limit is still previewable; one byte under is not.
    let size = text.len() as u64;
    assert!(
        read_managed_text(
            std::slice::from_ref(&root),
            "download/1234567890123456/6543210987654321",
            size,
        )
        .is_ok()
    );
    assert_eq!(
        read_managed_text(
            std::slice::from_ref(&root),
            "download/1234567890123456/6543210987654321",
            size - 1,
        ),
        Err(TextPreviewError::TooLarge)
    );
    drop(temp);
}

#[test]
fn refuses_text_previews_it_cannot_show_whole() {
    let (temp, root) = cache_with_file("6543210987654321.bin", &[0xff, 0xfe, 0x00]);
    assert_eq!(
        read_managed_text(
            std::slice::from_ref(&root),
            "download/1234567890123456/6543210987654321",
            MAX_TEXT_PREVIEW_BYTES,
        ),
        Err(TextPreviewError::NotText)
    );
    for refused in [
        "download/1234567890123456/9999999999999999",
        "download/../../etc/passwd",
        "/etc/passwd",
    ] {
        assert_eq!(
            read_managed_text(std::slice::from_ref(&root), refused, MAX_TEXT_PREVIEW_BYTES,),
            Err(TextPreviewError::Unavailable),
            "{refused}",
        );
    }
    assert_eq!(
        TextPreviewError::TooLarge.message(),
        "media file is too large to preview"
    );
    drop(temp);
}

/// #2096 stage 2: a PDF preview reads bytes, never a path, and refuses a file
/// above the cap instead of truncating it (a truncated PDF must never be drawn).
#[test]
fn reads_managed_pdf_bytes_up_to_the_cap_and_refuses_one_byte_over() {
    let body = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF\n";
    let (temp, root) = cache_with_file("6543210987654321.bin", body);
    let source = "download/1234567890123456/6543210987654321";
    let size = body.len() as u64;
    assert_eq!(
        read_managed_pdf(std::slice::from_ref(&root), source, size).expect("at the cap"),
        body
    );
    assert_eq!(
        read_managed_pdf(std::slice::from_ref(&root), source, size - 1),
        Err(PdfPreviewError::TooLarge)
    );
    assert_eq!(MAX_PDF_PREVIEW_BYTES, 32 * 1024 * 1024);
    drop(temp);
}

#[test]
fn malformed_pdf_bodies_are_refused_without_panicking() {
    for body in [
        &b""[..],
        &b"%PD"[..],
        &[0xff, 0x00, 0xfe, 0x25, 0x50][..],
        &b"not a pdf at all"[..],
    ] {
        let (temp, root) = cache_with_file("6543210987654321.bin", body);
        assert_eq!(
            read_managed_pdf(
                std::slice::from_ref(&root),
                "download/1234567890123456/6543210987654321",
                MAX_PDF_PREVIEW_BYTES,
            ),
            Err(PdfPreviewError::NotPdf),
            "{body:?}"
        );
        drop(temp);
    }
    let (temp, root) = cache_with_file("6543210987654321.bin", b"%PDF-1.7");
    for refused in [
        "download/1234567890123456/9999999999999999",
        "download/../../etc/passwd",
        "/etc/passwd",
    ] {
        assert_eq!(
            read_managed_pdf(std::slice::from_ref(&root), refused, MAX_PDF_PREVIEW_BYTES),
            Err(PdfPreviewError::Unavailable),
            "{refused}"
        );
    }
    drop(temp);
}

#[test]
fn renders_a_managed_markdown_file_and_enforces_the_same_cap() {
    // #2097: the document path reads through the stage-1 text reader, so the cap,
    // the UTF-8 check and the managed-cache confinement are shared.
    let (temp, root) = cache_with_file(
        "6543210987654321.bin",
        b"# Plan\n\n<script>alert(1)</script>\n\n```rust\nlet x = 1;\n```\n",
    );
    let source = "download/1234567890123456/6543210987654321";
    let rendered =
        read_managed_markdown(std::slice::from_ref(&root), source, MAX_TEXT_PREVIEW_BYTES)
            .expect("rendered");
    assert!(
        rendered.html.contains("<h1 id=\"md-h0\">Plan</h1>"),
        "{}",
        rendered.html
    );
    assert!(!rendered.html.contains("<script"), "{}", rendered.html);
    assert!(rendered.plain_text.contains("<script>alert(1)</script>"));
    assert_eq!(rendered.code_blocks[0].language.as_deref(), Some("rust"));
    assert_eq!(
        read_managed_markdown(std::slice::from_ref(&root), source, 8).map(|_| ()),
        Err(TextPreviewError::TooLarge)
    );
    assert_eq!(
        read_managed_markdown(
            std::slice::from_ref(&root),
            "download/../../etc/passwd",
            MAX_TEXT_PREVIEW_BYTES
        )
        .map(|_| ()),
        Err(TextPreviewError::Unavailable)
    );
    drop(temp);
}

#[test]
fn serves_the_audio_tokens_the_inline_player_needs() {
    for (token, mime) in [
        ("mp3", "audio/mpeg"),
        ("m4a", "audio/mp4"),
        ("aac", "audio/aac"),
        ("wav", "audio/wav"),
        ("flac", "audio/flac"),
        ("ogg", "audio/ogg"),
        ("opus", "audio/ogg"),
        ("m4v", "video/mp4"),
    ] {
        assert_eq!(mime_for_token(token), Some(mime), "{token}");
    }
    assert_eq!(mime_for_token("txt"), None);
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
    let response = protocol_response(
        &request("/download/png/1234567890123456/6543210987654321/800x800"),
        std::slice::from_ref(&root),
    );
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body(), b"thumbnail");
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
