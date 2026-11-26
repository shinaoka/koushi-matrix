//! The exact, durable mention feed behind `wait_for_mentions` (#2110).
//!
//! The sampled `wait_for_events` feed polls room summaries, coalesces posts and
//! forgets everything on restart, which loses mentions by construction. This
//! feed is owned by the core instead. One row is appended per event that
//! mentions the account, never coalesced, with a per-account sequence number as
//! the cursor. Each row is committed to the account's encrypted store before it
//! becomes visible to a reader, so a mention received while no client was
//! connected, or across a restart, is still delivered.
//!
//! Detection lives in the sync path that already sees every event: the SDK has
//! evaluated the account's push rules by then and hands the handler the
//! resulting actions. The SDK's *notification* handler is deliberately not used,
//! because it is skipped for a muted room, a room whose rule suppresses the
//! notification, and the owner's do-not-disturb choice; those settings exist for
//! a human, and an agent account must still see its mentions. The handler here
//! runs for every timeline event, takes the highlight fact from the push actions
//! and falls back to the event's own `m.mentions` when a mute hides it.
//!
//! Who may appear is decided here, before the commit: a mention from a sender
//! outside the allow-list is never stored, so no MCP or CLI client can read it
//! or widen the list. See [`koushi_state::MentionAllowedSenders`] for the
//! derived default (the other signed-in accounts).

mod intake;
mod registry;

pub use intake::MentionIntake;
pub use registry::MentionFeedRegistry;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// How many rows a feed retains before dropping the oldest.
pub const MAX_ENTRIES: usize = 1000;
const CURSOR_VERSION: &str = "mf1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MentionKind {
    /// `m.mentions` or a keyword/user-name highlight.
    Mention,
    /// A message in a direct chat.
    Dm,
    /// An invitation to a room.
    Invite,
}

impl MentionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::Dm => "dm",
            Self::Invite => "invite",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionEntry {
    pub seq: u64,
    pub room_id: String,
    /// Absent for an invite, which is stripped state without an event id.
    pub event_id: Option<String>,
    pub sender: String,
    pub timestamp_ms: u64,
    pub thread_root: Option<String>,
    pub kind: MentionKind,
}

/// What the intake offers before the feed assigns a sequence number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentionCandidate {
    pub room_id: String,
    pub event_id: Option<String>,
    pub sender: String,
    pub timestamp_ms: u64,
    pub thread_root: Option<String>,
    pub kind: MentionKind,
}

/// The persisted shape. `generation` names this log: a cursor binds to it, so a
/// cursor from another process, another account, or a log that was deleted and
/// recreated is refused instead of silently continuing at the wrong place.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionLogData {
    pub generation: String,
    /// Events older than this never enter a log created later than they were:
    /// the first sync after the feature appears replays history, which is not
    /// news.
    pub created_at_ms: u64,
    pub next_seq: u64,
    /// Highest evicted seq. A cursor below it has missed a row.
    pub evicted_through: u64,
    pub entries: Vec<MentionEntry>,
}

impl MentionLogData {
    pub fn fresh(generation: String, created_at_ms: u64) -> Self {
        Self {
            generation,
            created_at_ms,
            next_seq: 1,
            evicted_through: 0,
            entries: Vec::new(),
        }
    }
}

/// Why a cursor cannot continue the feed. The token leads the error message so
/// a client can branch on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorRejection {
    Malformed,
    /// Issued by another process, another account, or a recreated log.
    Foreign,
    Future,
    Expired,
}

impl CursorRejection {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Malformed => "malformed_cursor",
            Self::Foreign => "foreign_cursor",
            Self::Future => "future_cursor",
            Self::Expired => "expired_cursor",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::Malformed => "the cursor was not issued by wait_for_mentions",
            Self::Foreign => {
                "the cursor belongs to another account or to a mention log that no longer exists"
            }
            Self::Future => "the cursor is ahead of the newest mention",
            Self::Expired => "mentions after the cursor were dropped from the retained window",
        }
    }
}

#[derive(Clone, Debug)]
pub struct MentionPage {
    pub entries: Vec<MentionEntry>,
    /// Pass back to continue. Present even when `entries` is empty.
    pub cursor: String,
}

type Persist = dyn Fn(&MentionLogData) -> bool + Send + Sync;

struct Inner {
    data: Mutex<MentionLogData>,
    notify: Notify,
    /// Serializes writers so commits land in sequence order.
    commit: tokio::sync::Mutex<()>,
    persist: Arc<Persist>,
}

#[derive(Clone)]
pub struct MentionFeed {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for MentionFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MentionFeed")
    }
}

impl MentionFeed {
    /// Opens a feed over `loaded` rows (or a fresh log). `persist` commits one
    /// snapshot and reports success; it runs on a blocking thread.
    pub fn open(
        loaded: Option<MentionLogData>,
        now_ms: u64,
        persist: impl Fn(&MentionLogData) -> bool + Send + Sync + 'static,
    ) -> Self {
        let data = loaded.unwrap_or_else(|| MentionLogData::fresh(new_generation(), now_ms));
        Self {
            inner: Arc::new(Inner {
                data: Mutex::new(data),
                notify: Notify::new(),
                commit: tokio::sync::Mutex::new(()),
                persist: Arc::new(persist),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MentionLogData> {
        self.inner
            .data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn created_at_ms(&self) -> u64 {
        self.lock().created_at_ms
    }

    pub fn cursor_for(&self, seq: u64) -> String {
        format!("{CURSOR_VERSION}.{}.{seq}", self.lock().generation)
    }

    /// Appends one row, committing it before it is visible to a reader.
    /// Returns `Some(committed)` when a row was appended (`committed` is false
    /// when the store refused the write: the row is still delivered this run),
    /// or `None` when it was a duplicate of a row already held.
    pub async fn record(&self, candidate: MentionCandidate) -> Option<bool> {
        let _writer = self.inner.commit.lock().await;
        let mut next = {
            let data = self.lock();
            if is_duplicate(&data, &candidate) {
                return None;
            }
            data.clone()
        };
        let seq = next.next_seq;
        next.next_seq += 1;
        next.entries.push(MentionEntry {
            seq,
            room_id: candidate.room_id,
            event_id: candidate.event_id,
            sender: candidate.sender,
            timestamp_ms: candidate.timestamp_ms,
            thread_root: candidate.thread_root,
            kind: candidate.kind,
        });
        if next.entries.len() > MAX_ENTRIES {
            let excess = next.entries.len() - MAX_ENTRIES;
            if let Some(last) = next.entries.drain(..excess).next_back() {
                next.evicted_through = next.evicted_through.max(last.seq);
            }
        }
        let snapshot = next.clone();
        let persist = Arc::clone(&self.inner.persist);
        let committed = crate::executor::spawn_blocking(move || persist(&snapshot))
            .await
            .unwrap_or(false);
        *self.lock() = next;
        self.inner.notify.notify_waiters();
        Some(committed)
    }

    fn resolve(&self, cursor: Option<&str>) -> Result<u64, CursorRejection> {
        let data = self.lock();
        let Some(cursor) = cursor else {
            // No cursor: everything still retained, so a client that starts
            // (or restarts) without saved state still receives what arrived
            // while it was away.
            return Ok(data.evicted_through);
        };
        let mut parts = cursor.split('.');
        let (Some(version), Some(generation), Some(seq), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(CursorRejection::Malformed);
        };
        let seq = seq.parse::<u64>().map_err(|_| CursorRejection::Malformed)?;
        if version != CURSOR_VERSION
            || generation.len() != 16
            || !generation.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CursorRejection::Malformed);
        }
        if generation != data.generation {
            return Err(CursorRejection::Foreign);
        }
        if seq >= data.next_seq {
            return Err(CursorRejection::Future);
        }
        if seq < data.evicted_through {
            return Err(CursorRejection::Expired);
        }
        Ok(seq)
    }

    fn after(&self, seq: u64, limit: usize) -> Vec<MentionEntry> {
        self.lock()
            .entries
            .iter()
            .filter(|entry| entry.seq > seq)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Long-polls: returns the rows after `cursor` (at most `limit`, in order),
    /// waiting up to `timeout` for the first one. There is no sampling: an
    /// append wakes every waiter. The returned cursor is valid even when the
    /// wait times out with nothing.
    pub async fn wait(
        &self,
        cursor: Option<&str>,
        timeout: Duration,
        limit: usize,
    ) -> Result<MentionPage, CursorRejection> {
        let start = self.resolve(cursor)?;
        let limit = limit.max(1);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Register interest before reading so an append in between is not
            // missed.
            let notified = self.inner.notify.notified();
            let entries = self.after(start, limit);
            if let Some(last) = entries.last() {
                let cursor = self.cursor_for(last.seq);
                return Ok(MentionPage { entries, cursor });
            }
            if tokio::time::Instant::now() >= deadline
                || tokio::time::timeout_at(deadline, notified).await.is_err()
            {
                return Ok(MentionPage {
                    entries: Vec::new(),
                    cursor: self.cursor_for(start),
                });
            }
        }
    }
}

/// An event is one row: a replay of the same event (a repeated sync chunk, a
/// restart) never doubles it. An invite has no event id, so a repeat of the
/// same invite from the same sender is the same row.
fn is_duplicate(data: &MentionLogData, candidate: &MentionCandidate) -> bool {
    match &candidate.event_id {
        Some(event_id) => data
            .entries
            .iter()
            .any(|entry| entry.event_id.as_ref() == Some(event_id)),
        None => data
            .entries
            .iter()
            .rev()
            .find(|entry| entry.room_id == candidate.room_id)
            .is_some_and(|entry| entry.kind == candidate.kind && entry.sender == candidate.sender),
    }
}

/// 16 hex digits from the OS random source.
fn new_generation() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // The std hasher is seeded randomly per process; mixed with the clock
        // it still differs between runs.
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0),
        );
        bytes = hasher.finish().to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
