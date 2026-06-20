#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CachedImageKind {
    pub(crate) extension: &'static str,
    pub(crate) mime_type: &'static str,
}

pub(crate) fn cached_image_kind(bytes: &[u8]) -> Option<CachedImageKind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(CachedImageKind {
            extension: "png",
            mime_type: "image/png",
        });
    }
    if bytes.len() >= 3 && bytes[0..3] == [0xff, 0xd8, 0xff] {
        return Some(CachedImageKind {
            extension: "jpg",
            mime_type: "image/jpeg",
        });
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(CachedImageKind {
            extension: "gif",
            mime_type: "image/gif",
        });
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(CachedImageKind {
            extension: "webp",
            mime_type: "image/webp",
        });
    }
    if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis")
    {
        return Some(CachedImageKind {
            extension: "avif",
            mime_type: "image/avif",
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_image_kind_detects_png_for_webview_content_type() {
        let kind = cached_image_kind(b"\x89PNG\r\n\x1a\nrest").expect("png image kind");

        assert_eq!(kind.extension, "png");
        assert_eq!(kind.mime_type, "image/png");
    }

    #[test]
    fn cached_image_kind_detects_jpeg_for_webview_content_type() {
        let kind = cached_image_kind(b"\xff\xd8\xff\xe0rest").expect("jpeg image kind");

        assert_eq!(kind.extension, "jpg");
        assert_eq!(kind.mime_type, "image/jpeg");
    }

    #[test]
    fn cached_image_kind_rejects_unknown_bytes() {
        assert_eq!(cached_image_kind(b"not an image"), None);
    }
}
