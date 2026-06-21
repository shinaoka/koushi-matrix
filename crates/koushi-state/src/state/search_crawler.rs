//! Search history crawler state types — #77.
//!
//! Moved here from `state.rs` per the #87-alignment principle: new cohesive
//! units live in their own files, re-exported from the parent module for
//! backward compatibility with existing callers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Crawler state keyed by room id.
///
/// `Debug` is manually implemented to emit only counts and coarse states —
/// room ids are Matrix identifiers and must not appear in logs
/// (REPOSITORY_RULES privacy: no room IDs in Debug output).
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchCrawlerState {
    pub rooms: BTreeMap<String, SearchCrawlerRoomState>,
    pub last_active: Option<SearchCrawlerLastActive>,
}

impl std::fmt::Debug for SearchCrawlerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Emit per-state counts only; omit room ids.
        let mut idle = 0u32;
        let mut queued = 0u32;
        let mut running = 0u32;
        let mut completed = 0u32;
        let mut failed = 0u32;
        for state in self.rooms.values() {
            match state {
                SearchCrawlerRoomState::Idle => idle += 1,
                SearchCrawlerRoomState::Queued => queued += 1,
                SearchCrawlerRoomState::Running { .. } => running += 1,
                SearchCrawlerRoomState::Completed { .. } => completed += 1,
                SearchCrawlerRoomState::Failed { .. } => failed += 1,
            }
        }
        f.debug_struct("SearchCrawlerState")
            .field("idle", &idle)
            .field("queued", &queued)
            .field("running", &running)
            .field("completed", &completed)
            .field("failed", &failed)
            .field(
                "last_active",
                &self.last_active.as_ref().map(|_| "Some(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchCrawlerLastActive {
    pub room_id: String,
    pub updated_at_ms: u64,
    pub status: SearchCrawlerLastActiveStatus,
    pub processed: u64,
    pub indexed: u64,
}

impl std::fmt::Debug for SearchCrawlerLastActive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchCrawlerLastActive")
            .field("room_id", &"RoomId(..)")
            .field("updated_at_ms", &"Timestamp(..)")
            .field("status", &self.status)
            .field("processed", &self.processed)
            .field("indexed", &self.indexed)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchCrawlerLastActiveStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SearchCrawlerRoomState {
    Idle,
    Queued,
    Running {
        processed: u64,
        indexed: u64,
    },
    Completed {
        indexed: u64,
    },
    /// Failure carries only a coarse kind — no raw SDK error text crosses
    /// the Tauri/TypeScript boundary (privacy rule).
    Failed {
        #[serde(rename = "failureKind")]
        kind: SearchCrawlerFailureKind,
    },
}

impl Default for SearchCrawlerRoomState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchCrawlerFailureKind {
    RoomNotFound,
    Sdk,
    Decryption,
    IndexUnavailable,
}

/// User-visible speed control for the background crawler.
/// `Standard` is the persisted default; `Fast` has 0 ms inter-batch delay
/// and is intended for QA or explicit user opt-in only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchCrawlerSpeed {
    #[default]
    Standard,
    Fast,
    Slow,
    Paused,
}

/// Persisted settings that control the search history crawler.
/// Stored as `settings/settings.json` → `search_crawler`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchCrawlerSettings {
    #[serde(default)]
    pub speed: SearchCrawlerSpeed,
    #[serde(default = "crate::state::default_true")]
    pub include_media_captions: bool,
    #[serde(default = "crate::state::default_true")]
    pub include_filenames: bool,
}

impl Default for SearchCrawlerSettings {
    fn default() -> Self {
        Self {
            speed: SearchCrawlerSpeed::default(),
            include_media_captions: true,
            include_filenames: true,
        }
    }
}
