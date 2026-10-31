//! Local-file upload preparation for out-of-band senders (#1533).
//!
//! The MCP bridge hands Core absolute local paths; Core checks, reads, and
//! prepares them into upload requests without touching composer state.

use std::path::PathBuf;

use koushi_core::local_upload::{
    LocalUploadFailure, MAX_LOCAL_UPLOAD_FILE_BYTES, MAX_LOCAL_UPLOAD_FILES, check_local_paths,
    prepare_local_uploads,
};
use koushi_protocol::command::UploadMediaKind;
use koushi_state::{ComposerDocument, ComposerFormattingOptions, ImageUploadCompressionPolicy};

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode png");
    bytes
}

async fn prepare(
    paths: Vec<PathBuf>,
    caption: Option<&str>,
) -> Result<Vec<koushi_protocol::command::UploadMediaRequest>, LocalUploadFailure> {
    prepare_local_uploads(
        paths,
        caption.map(ComposerDocument::from_plain_text),
        ComposerFormattingOptions::default(),
        ImageUploadCompressionPolicy::default(),
        None,
    )
    .await
}

#[test]
fn relative_paths_are_refused() {
    assert_eq!(
        check_local_paths(&[PathBuf::from("fixture_notes.txt")]),
        Err(LocalUploadFailure::NotAbsolute)
    );
}

#[test]
fn an_empty_or_oversized_batch_is_refused() {
    assert_eq!(check_local_paths(&[]), Err(LocalUploadFailure::NoFiles));
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fixture_notes.txt");
    std::fs::write(&path, b"synthetic").expect("write");
    let many = vec![path; MAX_LOCAL_UPLOAD_FILES + 1];
    assert_eq!(
        check_local_paths(&many),
        Err(LocalUploadFailure::TooManyFiles)
    );
}

#[test]
fn directories_and_missing_files_are_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        check_local_paths(&[dir.path().to_path_buf()]),
        Err(LocalUploadFailure::NotARegularFile)
    );
    assert_eq!(
        check_local_paths(&[dir.path().join("missing.txt")]),
        Err(LocalUploadFailure::Unreadable)
    );
}

#[test]
fn an_oversized_file_is_refused_before_reading() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fixture_large.bin");
    let file = std::fs::File::create(&path).expect("create");
    file.set_len(MAX_LOCAL_UPLOAD_FILE_BYTES + 1)
        .expect("sparse length");
    assert_eq!(
        check_local_paths(&[path]),
        Err(LocalUploadFailure::TooLarge)
    );
}

#[tokio::test]
async fn an_empty_file_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fixture_empty.txt");
    std::fs::write(&path, b"").expect("write");
    assert_eq!(
        prepare(vec![path], None).await.err(),
        Some(LocalUploadFailure::Empty)
    );
}

#[tokio::test]
async fn a_text_file_becomes_a_file_upload_with_the_caption() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fixture_notes.txt");
    std::fs::write(&path, b"synthetic notes").expect("write");
    let requests = prepare(vec![path], Some("Synthetic caption"))
        .await
        .expect("prepared");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.filename, "fixture_notes.txt");
    assert_eq!(request.mime_type, "text/plain");
    assert_eq!(request.bytes, b"synthetic notes");
    assert!(matches!(request.kind, UploadMediaKind::File));
    assert_eq!(
        request
            .caption
            .as_ref()
            .map(|caption| caption.plain_body.as_str()),
        Some("Synthetic caption")
    );
}

#[tokio::test]
async fn an_image_is_sniffed_and_sent_untouched_with_dimensions() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A misleading extension: the bytes decide the type.
    let image_path = dir.path().join("fixture_chart.bin");
    let png = png_bytes(4, 3);
    std::fs::write(&image_path, &png).expect("write");
    let other_path = dir.path().join("fixture_budget.csv");
    std::fs::write(&other_path, b"a,b\n1,2\n").expect("write");

    let requests = prepare(vec![image_path, other_path], Some("Synthetic caption"))
        .await
        .expect("prepared");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].mime_type, "image/png");
    assert_eq!(requests[0].bytes, png, "the original bytes are uploaded");
    assert!(matches!(
        requests[0].kind,
        UploadMediaKind::Image {
            width: Some(4),
            height: Some(3)
        }
    ));
    assert!(
        requests[0].caption.is_some(),
        "the body captions the first file"
    );
    assert_eq!(requests[1].mime_type, "text/csv");
    assert!(requests[1].caption.is_none(), "and only the first file");
}
