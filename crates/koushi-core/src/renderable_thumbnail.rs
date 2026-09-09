use std::{
    collections::{HashMap, VecDeque},
    fmt, fs,
    hash::{DefaultHasher, Hasher},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_media::image_kind;
use koushi_state::AvatarThumbnailState;

pub(crate) const MAX_RENDERABLE_THUMBNAIL_ENTRIES: usize = 256;
pub(crate) const MAX_RENDERABLE_THUMBNAIL_BYTES: usize = 32 * 1024 * 1024;
const MAX_THUMBNAIL_LEASE_BYTES: usize = 96 * 1024 * 1024;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderableThumbnailStoreError {
    TooLarge { byte_count: usize, max_bytes: usize },
}

impl fmt::Display for RenderableThumbnailStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { .. } => {
                formatter.write_str("renderable thumbnail exceeds cache bound")
            }
        }
    }
}

impl std::error::Error for RenderableThumbnailStoreError {}

struct RenderableThumbnailEntry {
    bytes: Vec<u8>,
    mime_type: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThumbnailLeaseError {
    Unavailable,
    Capacity,
}

/// Private byte ownership, not permission to access a retired or foreign scope.
#[derive(Clone)]
pub(crate) struct RenderableThumbnailLease(Arc<ChargedThumbnail>);

struct ChargedThumbnail {
    source_ref: String,
    entry: Arc<RenderableThumbnailEntry>,
    usage: Arc<Mutex<usize>>,
}

impl Drop for ChargedThumbnail {
    fn drop(&mut self) {
        *self.usage.lock().expect("thumbnail lease budget poisoned") -= self.entry.bytes.len();
    }
}

impl RenderableThumbnailLease {
    pub(crate) fn source_ref(&self) -> &str {
        &self.0.source_ref
    }

    pub(crate) fn control_bytes(&self) -> usize {
        std::mem::size_of::<ChargedThumbnail>() + self.0.source_ref.len()
    }

    pub(crate) fn thumbnail_state(&self) -> AvatarThumbnailState {
        AvatarThumbnailState::Ready {
            source_ref: self.0.source_ref.clone(),
            width: None,
            height: None,
            mime_type: Some(self.0.entry.mime_type.clone()),
        }
    }

    pub(crate) fn content(&self) -> RenderableThumbnailContent {
        RenderableThumbnailContent {
            bytes: self.0.entry.bytes.clone(),
            mime_type: Some(self.0.entry.mime_type.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderableThumbnailCacheStats {
    pub entry_count: usize,
    pub retained_bytes: usize,
    pub high_water_entry_count: usize,
    pub high_water_bytes: usize,
    pub eviction_count: u64,
    pub clear_count: u64,
    pub oversize_rejection_count: u64,
}

#[derive(Default)]
struct RenderableThumbnailCache {
    // Opaque references are stored in AppState while their bytes remain in this
    // count-and-byte-bounded LRU. Access refreshes recency; session clear drops
    // all retained bytes.
    entries: HashMap<String, Arc<RenderableThumbnailEntry>>,
    lease_bytes: Arc<Mutex<usize>>,
    // Oldest at the front, most recently accessed at the back. The bound is
    // deliberately larger than the existing 129-entry session churn contract.
    lru: VecDeque<String>,
    retained_bytes: usize,
    high_water_entry_count: usize,
    high_water_bytes: usize,
    eviction_count: u64,
    clear_count: u64,
    oversize_rejection_count: u64,
}

impl RenderableThumbnailCache {
    fn insert(
        &mut self,
        cache_key: String,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<(), RenderableThumbnailStoreError> {
        if bytes.len() > MAX_RENDERABLE_THUMBNAIL_BYTES {
            self.oversize_rejection_count = self.oversize_rejection_count.saturating_add(1);
            record(
                DiagnosticEvent::new(DiagnosticLevel::Warn, "core.renderable_thumbnail", "reject")
                    .field(DiagnosticField::token("reason", "oversize"))
                    .field(DiagnosticField::count("byte_count", bytes.len() as u64))
                    .field(DiagnosticField::count(
                        "max_bytes",
                        MAX_RENDERABLE_THUMBNAIL_BYTES as u64,
                    ))
                    .field(DiagnosticField::count(
                        "oversize_rejection_count",
                        self.oversize_rejection_count,
                    )),
            );
            return Err(RenderableThumbnailStoreError::TooLarge {
                byte_count: bytes.len(),
                max_bytes: MAX_RENDERABLE_THUMBNAIL_BYTES,
            });
        }
        let entry = RenderableThumbnailEntry { bytes, mime_type };
        if let Some(previous) = self.entries.remove(&cache_key) {
            self.retained_bytes = self.retained_bytes.saturating_sub(previous.bytes.len());
            self.remove_from_lru(&cache_key);
        }
        self.retained_bytes = self.retained_bytes.saturating_add(entry.bytes.len());
        self.entries.insert(cache_key.clone(), Arc::new(entry));
        self.lru.push_back(cache_key);
        self.update_high_water();
        self.evict_if_needed();
        Ok(())
    }

    fn get(&mut self, cache_key: &str) -> Option<RenderableThumbnailContent> {
        let entry = self.entries.get(cache_key)?.clone();
        self.touch(cache_key);
        Some(RenderableThumbnailContent {
            bytes: entry.bytes.clone(),
            mime_type: Some(entry.mime_type.clone()),
        })
    }

    fn lease(&mut self, cache_key: &str) -> Result<RenderableThumbnailLease, ThumbnailLeaseError> {
        let entry = self
            .entries
            .get(cache_key)
            .ok_or(ThumbnailLeaseError::Unavailable)?
            .clone();
        let mut usage = self
            .lease_bytes
            .lock()
            .expect("thumbnail lease budget poisoned");
        let total = usage
            .checked_add(entry.bytes.len())
            .filter(|total| *total <= MAX_THUMBNAIL_LEASE_BYTES)
            .ok_or(ThumbnailLeaseError::Capacity)?;
        *usage = total;
        drop(usage);
        self.touch(cache_key);
        Ok(RenderableThumbnailLease(Arc::new(ChargedThumbnail {
            source_ref: cache_key.into(),
            entry,
            usage: self.lease_bytes.clone(),
        })))
    }

    fn stats(&self) -> RenderableThumbnailCacheStats {
        RenderableThumbnailCacheStats {
            entry_count: self.entries.len(),
            retained_bytes: self.retained_bytes,
            high_water_entry_count: self.high_water_entry_count,
            high_water_bytes: self.high_water_bytes,
            eviction_count: self.eviction_count,
            clear_count: self.clear_count,
            oversize_rejection_count: self.oversize_rejection_count,
        }
    }

    fn clear(&mut self) {
        let removed_entries = self.entries.len();
        let removed_bytes = self.retained_bytes;
        self.entries.clear();
        self.lru.clear();
        self.retained_bytes = 0;
        self.clear_count = self.clear_count.saturating_add(1);
        if removed_entries > 0 || removed_bytes > 0 {
            record(
                DiagnosticEvent::new(DiagnosticLevel::Debug, "core.renderable_thumbnail", "clear")
                    .field(DiagnosticField::count(
                        "removed_entries",
                        removed_entries as u64,
                    ))
                    .field(DiagnosticField::count(
                        "removed_bytes",
                        removed_bytes as u64,
                    ))
                    .field(DiagnosticField::count("clear_count", self.clear_count))
                    .field(DiagnosticField::count(
                        "high_water_entries",
                        self.high_water_entry_count as u64,
                    ))
                    .field(DiagnosticField::count(
                        "high_water_bytes",
                        self.high_water_bytes as u64,
                    )),
            );
        }
    }

    fn touch(&mut self, cache_key: &str) {
        self.remove_from_lru(cache_key);
        self.lru.push_back(cache_key.to_owned());
    }

    fn remove_from_lru(&mut self, cache_key: &str) {
        self.lru.retain(|key| key != cache_key);
    }

    fn update_high_water(&mut self) {
        self.high_water_entry_count = self.high_water_entry_count.max(self.entries.len());
        self.high_water_bytes = self.high_water_bytes.max(self.retained_bytes);
    }

    fn evict_if_needed(&mut self) {
        let mut evicted_entries = 0usize;
        let mut evicted_bytes = 0usize;
        while self.entries.len() > MAX_RENDERABLE_THUMBNAIL_ENTRIES
            || self.retained_bytes > MAX_RENDERABLE_THUMBNAIL_BYTES
        {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            let Some(entry) = self.entries.remove(&oldest) else {
                continue;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(entry.bytes.len());
            evicted_entries += 1;
            evicted_bytes = evicted_bytes.saturating_add(entry.bytes.len());
            self.eviction_count = self.eviction_count.saturating_add(1);
        }
        if evicted_entries > 0 {
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Debug,
                    "core.renderable_thumbnail",
                    "eviction",
                )
                .field(DiagnosticField::count(
                    "evicted_entries",
                    evicted_entries as u64,
                ))
                .field(DiagnosticField::count(
                    "evicted_bytes",
                    evicted_bytes as u64,
                ))
                .field(DiagnosticField::count("entries", self.entries.len() as u64))
                .field(DiagnosticField::count("bytes", self.retained_bytes as u64))
                .field(DiagnosticField::count(
                    "eviction_count",
                    self.eviction_count,
                )),
            );
        }
    }
}

fn renderable_thumbnail_cache() -> &'static Mutex<RenderableThumbnailCache> {
    static CACHE: OnceLock<Mutex<RenderableThumbnailCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RenderableThumbnailCache::default()))
}

fn mime_type_from_bytes(bytes: &[u8]) -> String {
    image_kind(bytes)
        .map(|kind| kind.mime_type.to_owned())
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

pub(crate) fn renderable_thumbnail_cache_key(
    kind: RenderableThumbnailKind,
    source: &str,
) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(kind.path_segment().as_bytes());
    hasher.write(source.as_bytes());
    format!("{}/{:016x}", kind.path_segment(), hasher.finish())
}

fn validated_renderable_thumbnail_ref(source_ref: &str) -> Option<&str> {
    let (kind, hash) = source_ref.split_once('/')?;
    if !matches!(kind, "avatar" | "link-preview")
        || hash.len() != 16
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(source_ref)
}

pub fn store_renderable_thumbnail(
    kind: RenderableThumbnailKind,
    source: &str,
    bytes: Vec<u8>,
) -> Result<AvatarThumbnailState, RenderableThumbnailStoreError> {
    let mime_type = mime_type_from_bytes(&bytes);
    let cache_key = renderable_thumbnail_cache_key(kind, source);
    {
        let mut cache = renderable_thumbnail_cache()
            .lock()
            .expect("renderable thumbnail cache should not be poisoned");
        cache.insert(cache_key.clone(), bytes, mime_type.clone())?;
    }

    Ok(AvatarThumbnailState::Ready {
        source_ref: cache_key,
        width: None,
        height: None,
        mime_type: Some(mime_type),
    })
}

pub(crate) fn lease_renderable_thumbnail(
    source_ref: &str,
) -> Result<RenderableThumbnailLease, ThumbnailLeaseError> {
    let cache_key =
        validated_renderable_thumbnail_ref(source_ref).ok_or(ThumbnailLeaseError::Unavailable)?;
    renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned")
        .lease(cache_key)
}

/// Check availability without cloning image bytes or reserving a resource lease.
pub(crate) fn is_renderable_thumbnail_cached(source_ref: &str) -> bool {
    let Some(cache_key) = validated_renderable_thumbnail_ref(source_ref) else {
        return false;
    };
    let mut cache = renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned");
    if !cache.entries.contains_key(cache_key) {
        return false;
    }
    cache.touch(cache_key);
    true
}

pub fn lookup_renderable_thumbnail(source_ref: &str) -> Option<RenderableThumbnailContent> {
    let cache_key = validated_renderable_thumbnail_ref(source_ref)?;
    let mut cache = renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned");
    cache.get(&cache_key)
}

pub fn clear_renderable_thumbnail_cache() {
    let mut cache = renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned");
    cache.clear();
}

#[cfg(test)]
pub(crate) fn test_cache_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("renderable thumbnail cache test lock should not be poisoned")
}

pub fn renderable_thumbnail_cache_stats() -> RenderableThumbnailCacheStats {
    renderable_thumbnail_cache()
        .lock()
        .expect("renderable thumbnail cache should not be poisoned")
        .stats()
}

pub fn renderable_thumbnail_summary_event(stats: RenderableThumbnailCacheStats) -> DiagnosticEvent {
    DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.renderable_thumbnail",
        "summary",
    )
    .field(DiagnosticField::token(
        "policy",
        "count_and_byte_bounded_lru",
    ))
    .field(DiagnosticField::count(
        "entry_count",
        stats.entry_count as u64,
    ))
    .field(DiagnosticField::count(
        "retained_bytes",
        stats.retained_bytes as u64,
    ))
    .field(DiagnosticField::count(
        "high_water_entry_count",
        stats.high_water_entry_count as u64,
    ))
    .field(DiagnosticField::count(
        "high_water_bytes",
        stats.high_water_bytes as u64,
    ))
    .field(DiagnosticField::count(
        "eviction_count",
        stats.eviction_count,
    ))
    .field(DiagnosticField::count("clear_count", stats.clear_count))
    .field(DiagnosticField::count(
        "oversize_rejection_count",
        stats.oversize_rejection_count,
    ))
}

pub fn record_renderable_thumbnail_summary(stats: RenderableThumbnailCacheStats) {
    record(renderable_thumbnail_summary_event(stats));
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
mod tests;
