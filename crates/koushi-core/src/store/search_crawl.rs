//! Durable, generation-tagged search-crawl commitments.
//!
//! The index crawl is bounded and resumable in memory, but a restart used to
//! forget which rooms had already been committed, so every startup began the
//! same full-history crawl again. This module persists one small encrypted file
//! per account holding, for each crawled room, the boundary event id and the
//! counters the room row reports.
//!
//! Only identifiers and counters are stored — room ids, boundary event ids and
//! counts, never message text.

use super::{CoreFailure, StoreActor};
use koushi_protocol::SessionKeyId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const SEARCH_CRAWL_FILE_MAGIC: &[u8] = b"KOUSHI-SEARCH-CRAWL-V1\0";

/// Version of the durable search-crawl contract.
///
/// Bump when an index or extraction change means rooms committed under the old
/// behavior must be crawled again. Progress recorded under another version is
/// ignored and dropped on the next save.
pub(crate) const SEARCH_CRAWL_BACKEND_VERSION: u32 = 1;

/// Committed crawls for one account.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SearchCrawlProgress {
    backend_version: u32,
    rooms: BTreeMap<String, CommittedRoomCrawl>,
}

/// What a completed crawl of one room committed to the index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct CommittedRoomCrawl {
    /// Latest event id when the crawl completed. A room that later gains events
    /// uses this as the catch-up boundary (#996).
    pub latest_event_id: Option<String>,
    pub processed: u64,
    pub indexed: u64,
}

impl SearchCrawlProgress {
    pub(crate) fn new() -> Self {
        Self {
            backend_version: SEARCH_CRAWL_BACKEND_VERSION,
            rooms: BTreeMap::new(),
        }
    }

    /// Rooms already committed for the current backend version.
    ///
    /// Anything recorded under another version is left out, so a contract change
    /// re-crawls those rooms instead of trusting a stale commitment.
    pub(crate) fn committed_rooms(&self) -> BTreeMap<String, CommittedRoomCrawl> {
        if self.backend_version == SEARCH_CRAWL_BACKEND_VERSION {
            self.rooms.clone()
        } else {
            BTreeMap::new()
        }
    }

    pub(crate) fn commit(&mut self, room_id: String, crawl: CommittedRoomCrawl) {
        if self.backend_version != SEARCH_CRAWL_BACKEND_VERSION {
            self.backend_version = SEARCH_CRAWL_BACKEND_VERSION;
            self.rooms.clear();
        }
        self.rooms.insert(room_id, crawl);
    }

    pub(crate) fn forget(&mut self, room_id: &str) {
        self.rooms.remove(room_id);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}

impl StoreActor {
    /// Load the account's committed crawls.
    ///
    /// A missing file is an account that has never crawled, which is an empty
    /// commit set rather than a failure; anything unreadable or undecryptable is
    /// reported as `StoreUnavailable` so callers can decide to fail open.
    pub(crate) fn load_search_crawl_progress(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<SearchCrawlProgress, CoreFailure> {
        let path = self.account_search_crawl_path(key_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SearchCrawlProgress::new());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };

        decrypt_search_crawl_payload(&self.load_unlock_secret(key_id)?, &bytes)
    }

    pub(crate) fn save_search_crawl_progress(
        &self,
        key_id: &SessionKeyId,
        progress: &SearchCrawlProgress,
    ) -> Result<(), CoreFailure> {
        let path = self.account_search_crawl_path(key_id);
        if progress.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(CoreFailure::StoreUnavailable),
            };
        }

        let payload =
            encrypt_search_crawl_payload(&self.load_or_create_unlock_secret(key_id)?, progress)?;
        koushi_store::atomic_replace_file(&path, &payload, false)
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    /// Path of the account's committed-crawl file.
    pub(crate) fn account_search_crawl_path(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("search")
            .join("crawl.v1.enc")
    }
}

fn encrypt_search_crawl_payload(
    secret: &koushi_key::LocalUnlockSecret,
    progress: &SearchCrawlProgress,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(progress).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_search_crawl_key();
    koushi_store::encrypt_envelope(
        SEARCH_CRAWL_FILE_MAGIC,
        key.as_bytes(),
        &plaintext,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)
}

fn decrypt_search_crawl_payload(
    secret: &koushi_key::LocalUnlockSecret,
    payload: &[u8],
) -> Result<SearchCrawlProgress, CoreFailure> {
    let key = secret.derive_search_crawl_key();
    let plaintext = koushi_store::decrypt_envelope(
        SEARCH_CRAWL_FILE_MAGIC,
        key.as_bytes(),
        payload,
        usize::MAX,
    )
    .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

#[cfg(test)]
mod tests;
