use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hasher},
    path::Path,
    sync::{Mutex, OnceLock},
};

use crate::cached_image::cached_image_kind;
use koushi_state::AvatarThumbnailState;

const RENDERABLE_THUMBNAIL_SCHEME: &str = "koushi-thumbnail://localhost/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderableThumbnailKind {
    Avatar,
    LinkPreview,
}

impl RenderableThumbnailKind {
    fn path_segment(self) -> &'static str {
        match self {
            Self::Avatar => "avatar",
            Self::LinkPreview => "link-preview",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderableThumbnailContent {
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
}

#[derive(Clone)]
struct RenderableThumbnailEntry {
    bytes: Vec<u8>,
    mime_type: String,
}

#[derive(Default)]
struct RenderableThumbnailCache {
    // Ready protocol URLs are stored in AppState; keep bytes until the session
    // cache is explicitly cleared so those projections remain reconstructible.
    entries: HashMap<String, RenderableThumbnailEntry>,
}

impl RenderableThumbnailCache {
    fn insert(
        &mut self,
        cache_key: String,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> RenderableThumbnailEntry {
        let entry = RenderableThumbnailEntry { bytes, mime_type };
        self.entries.insert(cache_key, entry.clone());
        entry
    }

    fn get(&mut self, cache_key: &str) -> Option<RenderableThumbnailContent> {
        let entry = self.entries.get(cache_key)?.clone();
        Some(RenderableThumbnailContent {
            bytes: entry.bytes,
            mime_type: Some(entry.mime_type),
        })
    }
}

fn renderable_thumbnail_cache() -> &'static Mutex<RenderableThumbnailCache> {
    static CACHE: OnceLock<Mutex<RenderableThumbnailCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RenderableThumbnailCache::default()))
}

fn mime_type_from_bytes(bytes: &[u8]) -> String {
    cached_image_kind(bytes)
        .map(|kind| kind.mime_type.to_owned())
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

fn renderable_thumbnail_cache_key(kind: RenderableThumbnailKind, source: &str) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(kind.path_segment().as_bytes());
    hasher.write(source.as_bytes());
    format!("{}/{:016x}", kind.path_segment(), hasher.finish())
}

fn renderable_thumbnail_source_url(cache_key: &str) -> String {
    format!("{RENDERABLE_THUMBNAIL_SCHEME}{cache_key}")
}

fn renderable_thumbnail_cache_key_from_path(path: &str) -> Option<String> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let mut segments = trimmed.split('/');
    let kind = segments.next()?;
    let key = segments.next()?;
    if key.is_empty() || segments.next().is_some() {
        return None;
    }

    match kind {
        "avatar" | "link-preview" => Some(format!("{kind}/{key}")),
        _ => None,
    }
}

pub fn store_renderable_thumbnail(
    kind: RenderableThumbnailKind,
    source: &str,
    bytes: Vec<u8>,
) -> AvatarThumbnailState {
    let mime_type = mime_type_from_bytes(&bytes);
    let cache_key = renderable_thumbnail_cache_key(kind, source);
    {
        let mut cache = renderable_thumbnail_cache()
            .lock()
            .expect("renderable thumbnail cache should not be poisoned");
        cache.insert(cache_key.clone(), bytes, mime_type.clone());
    }

    AvatarThumbnailState::Ready {
        source_url: renderable_thumbnail_source_url(&cache_key),
        width: None,
        height: None,
        mime_type: Some(mime_type),
    }
}

pub fn lookup_renderable_thumbnail(path: &str) -> Option<RenderableThumbnailContent> {
    let cache_key = renderable_thumbnail_cache_key_from_path(path)?;
    let mut cache = renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned");
    cache.get(&cache_key)
}

pub fn clear_renderable_thumbnail_cache() {
    let mut cache = renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned");
    *cache = RenderableThumbnailCache::default();
}

pub fn cleanup_legacy_plaintext_thumbnail_dirs(data_dir: &Path) -> std::io::Result<()> {
    for dir in [
        data_dir.join("avatar_thumbnails"),
        data_dir.join("link_preview_thumbnails"),
    ] {
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex as TestMutex, MutexGuard, OnceLock as TestOnceLock};

    fn cache_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: TestOnceLock<TestMutex<()>> = TestOnceLock::new();
        LOCK.get_or_init(|| TestMutex::new(()))
            .lock()
            .expect("renderable thumbnail cache test lock should not be poisoned")
    }

    #[test]
    fn stores_avatar_and_link_preview_thumbnails_in_memory_with_protocol_urls() {
        let _guard = cache_test_lock();
        clear_renderable_thumbnail_cache();

        let avatar = store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            "mxc://example.test/avatar",
            b"avatar-bytes".to_vec(),
        );
        let link_preview = store_renderable_thumbnail(
            RenderableThumbnailKind::LinkPreview,
            "https://example.test/page",
            b"preview-bytes".to_vec(),
        );

        let AvatarThumbnailState::Ready {
            source_url,
            mime_type,
            ..
        } = avatar
        else {
            panic!("avatar thumbnail should be ready");
        };
        assert!(source_url.starts_with("koushi-thumbnail://localhost/avatar/"));
        assert_eq!(mime_type.as_deref(), Some("application/octet-stream"));

        let AvatarThumbnailState::Ready {
            source_url,
            mime_type,
            ..
        } = link_preview
        else {
            panic!("link-preview thumbnail should be ready");
        };
        assert!(source_url.starts_with("koushi-thumbnail://localhost/link-preview/"));
        assert_eq!(mime_type.as_deref(), Some("application/octet-stream"));
    }

    #[test]
    fn lookup_renderable_thumbnail_returns_bytes_for_protocol_path() {
        let _guard = cache_test_lock();
        clear_renderable_thumbnail_cache();

        let ready = store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            "mxc://example.test/lookup",
            b"lookup-bytes".to_vec(),
        );
        let AvatarThumbnailState::Ready { source_url, .. } = ready else {
            panic!("thumbnail should be ready");
        };

        let path = source_url
            .strip_prefix("koushi-thumbnail://localhost")
            .expect("protocol url should have localhost authority");
        let content = lookup_renderable_thumbnail(path).expect("thumbnail should be cached");
        assert_eq!(content.bytes, b"lookup-bytes");
        assert_eq!(
            content.mime_type.as_deref(),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn ready_thumbnail_protocol_urls_survive_session_cache_churn() {
        let _guard = cache_test_lock();
        clear_renderable_thumbnail_cache();

        let ready = store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            "mxc://example.test/pinned",
            b"pinned-bytes".to_vec(),
        );
        let AvatarThumbnailState::Ready { source_url, .. } = ready else {
            panic!("thumbnail should be ready");
        };
        let path = source_url
            .strip_prefix("koushi-thumbnail://localhost")
            .expect("protocol url should have localhost authority");

        for index in 0..=128 {
            let source = format!("mxc://example.test/churn/{index}");
            let bytes = format!("bytes-{index}").into_bytes();
            let _ = store_renderable_thumbnail(RenderableThumbnailKind::Avatar, &source, bytes);
        }

        let content = lookup_renderable_thumbnail(path)
            .expect("Ready thumbnail URL must remain reconstructible until session clear");
        assert_eq!(content.bytes, b"pinned-bytes");
    }

    #[test]
    fn clear_renderable_thumbnail_cache_drops_previous_session_bytes() {
        let _guard = cache_test_lock();
        clear_renderable_thumbnail_cache();

        let ready = store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            "mxc://example.test/session-scoped",
            b"session-bytes".to_vec(),
        );
        let AvatarThumbnailState::Ready { source_url, .. } = ready else {
            panic!("thumbnail should be ready");
        };
        let path = source_url
            .strip_prefix("koushi-thumbnail://localhost")
            .expect("protocol url should have localhost authority");

        assert!(lookup_renderable_thumbnail(path).is_some());

        clear_renderable_thumbnail_cache();

        assert!(lookup_renderable_thumbnail(path).is_none());
    }

    #[test]
    fn cleanup_legacy_plaintext_thumbnail_dirs_preserves_media_downloads() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path();

        fs::create_dir_all(data_dir.join("avatar_thumbnails")).expect("seed avatar dir");
        fs::write(
            data_dir.join("avatar_thumbnails").join("thumb.bin"),
            b"avatar",
        )
        .expect("seed avatar file");
        fs::create_dir_all(data_dir.join("link_preview_thumbnails")).expect("seed preview dir");
        fs::write(
            data_dir.join("link_preview_thumbnails").join("preview.bin"),
            b"preview",
        )
        .expect("seed preview file");
        fs::create_dir_all(data_dir.join("media_downloads")).expect("seed media dir");
        fs::write(
            data_dir.join("media_downloads").join("download.bin"),
            b"download",
        )
        .expect("seed download file");

        cleanup_legacy_plaintext_thumbnail_dirs(data_dir).expect("cleanup should succeed");

        assert!(!data_dir.join("avatar_thumbnails").exists());
        assert!(!data_dir.join("link_preview_thumbnails").exists());
        assert!(data_dir.join("media_downloads").exists());
        assert_eq!(
            fs::read(data_dir.join("media_downloads").join("download.bin")).expect("media file"),
            b"download"
        );
    }

    #[test]
    fn avatar_and_preview_thumbnail_helpers_do_not_use_legacy_plaintext_paths() {
        let account_source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/account.rs"));
        let account_body = account_source
            .split("async fn download_avatar_thumbnail")
            .nth(1)
            .expect("avatar helper");
        let account_body = account_body
            .split("fn classify_profile_error")
            .next()
            .expect("avatar helper body");
        assert!(account_body.contains("get_media_content"));
        assert!(account_body.contains("true,"));
        assert!(!account_body.contains("avatar_thumbnails"));
        assert!(!account_body.contains("file://"));

        let link_preview_source =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/link_preview.rs"));
        let link_preview_body = link_preview_source
            .split("async fn download_preview_image")
            .nth(1)
            .expect("preview helper");
        let link_preview_body = link_preview_body
            .split("#[cfg(test)]")
            .next()
            .expect("preview helper body");
        assert!(link_preview_body.contains("get_media_content"));
        assert!(link_preview_body.contains("false,"));
        assert!(!link_preview_body.contains("link_preview_thumbnails"));
        assert!(!link_preview_body.contains("file://"));
    }
}
