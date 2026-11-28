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
//! outside the allow-list is never stored, so it never becomes a feed row. This
//! guards the feed only; the room's own content stays readable through the
//! other read tools. See [`koushi_state::MentionAllowedSenders`] for the
//! derived default (the other signed-in accounts).

mod intake;
mod registry;

pub use intake::MentionIntake;
pub use registry::MentionFeedRegistry;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// How many rows a feed retains before dropping the oldest.
pub const MAX_ENTRIES: usize = 1000;
/// How long a row is retained. A cursor that points before a dropped row (by
/// count or by age) is `expired_cursor`: the client must resync.
pub const MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// Persist attempts for one row before the feed is marked unhealthy.
const PERSIST_ATTEMPTS: usize = 3;
const PERSIST_RETRY_DELAY: Duration = Duration::from_millis(100);
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

/// What a message is, recorded at intake from the decoded event (#2132 §7).
/// A closed set: no filename, body, caption or bytes ever enter the feed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Text,
    Notice,
    Emote,
    Image,
    File,
    Audio,
    Video,
    Location,
    Other,
}

impl MessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Notice => "notice",
            Self::Emote => "emote",
            Self::Image => "image",
            Self::File => "file",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Location => "location",
            Self::Other => "other",
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
    /// Absent for an invite and for a row written before this field existed.
    #[serde(default)]
    pub message_kind: Option<MessageKind>,
    /// The sender's declared `info.mimetype`: a hint for triage, never a
    /// guarantee and never used to decide what is served.
    #[serde(default)]
    pub mime_type: Option<String>,
    /// The sender's declared `info.size`: a hint, like `mime_type`.
    #[serde(default)]
    pub size_bytes: Option<u64>,
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
    /// Absent for an invite and for a row written before this field existed.
    pub message_kind: Option<MessageKind>,
    /// The sender's declared `info.mimetype`: a hint for triage, never a
    /// guarantee and never used to decide what is served.
    pub mime_type: Option<String>,
    /// The sender's declared `info.size`: a hint, like `mime_type`.
    pub size_bytes: Option<u64>,
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
    /// `evicted_through` including rows that are too old to be served at
    /// `now_ms`, so a reader never sees a row the retention window dropped.
    pub fn evicted_at(&self, now_ms: u64) -> u64 {
        let cutoff = now_ms.saturating_sub(MAX_AGE_MS);
        self.entries
            .iter()
            .filter(|entry| entry.timestamp_ms < cutoff)
            .map(|entry| entry.seq)
            .max()
            .map_or(self.evicted_through, |seq| seq.max(self.evicted_through))
    }

    /// Drops rows beyond the count cap or the age cap.
    fn prune(&mut self, now_ms: u64) {
        let evicted = self.evicted_at(now_ms);
        self.entries.retain(|entry| entry.seq > evicted);
        self.evicted_through = evicted;
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            if let Some(last) = self.entries.drain(..excess).next_back() {
                self.evicted_through = self.evicted_through.max(last.seq);
            }
        }
    }

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
    /// Serializes writers so commits land in sequence order. Owned guards move
    /// into the detached commit task.
    commit: Arc<tokio::sync::Mutex<()>>,
    persist: Arc<Persist>,
    /// Cleared when a row could not be committed: the cursor would no longer
    /// describe what the feed delivered, so readers are refused.
    healthy: AtomicBool,
}

/// What `record` did with a candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOutcome {
    Recorded,
    /// The event was already a row.
    Duplicate,
    /// The row could not be committed; the feed is now unhealthy.
    Failed,
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

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

impl MentionFeed {
    /// Opens a feed over `loaded` rows (or a fresh log). `persist` commits one
    /// snapshot and reports success.
    ///
    /// A fresh log is saved before this returns, so its `created_at_ms` (the
    /// history line) and generation survive a restart even if no mention is
    /// ever recorded; `None` when that save fails, because a feed whose
    /// history line moves on every start would drop mentions that arrived while
    /// the app was down. Blocks on the store: call from a blocking context.
    pub fn open(
        loaded: Option<MentionLogData>,
        now_ms: u64,
        persist: impl Fn(&MentionLogData) -> bool + Send + Sync + 'static,
    ) -> Option<Self> {
        let data = match loaded {
            Some(mut data) => {
                data.prune(now_ms);
                data
            }
            None => {
                let data = MentionLogData::fresh(new_generation(), now_ms);
                if !persist(&data) {
                    return None;
                }
                data
            }
        };
        Some(Self {
            inner: Arc::new(Inner {
                data: Mutex::new(data),
                notify: Notify::new(),
                commit: Arc::new(tokio::sync::Mutex::new(())),
                persist: Arc::new(persist),
                healthy: AtomicBool::new(true),
            }),
        })
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

    pub fn is_healthy(&self) -> bool {
        self.inner.healthy.load(Ordering::Acquire)
    }

    /// Names this log: a recreated feed has another one (#2132).
    pub fn generation(&self) -> String {
        self.lock().generation.clone()
    }

    pub fn cursor_for(&self, seq: u64) -> String {
        format!("{CURSOR_VERSION}.{}.{seq}", self.lock().generation)
    }

    /// The cursor that precedes the oldest retained row: continuing from it
    /// delivers everything still retained (#2132, `subscribe after:"retained"`).
    pub fn retained_start_cursor(&self) -> String {
        let floor = self.lock().evicted_at(now_ms());
        self.cursor_for(floor)
    }

    /// The sequence number a cursor names, checking only that this feed issued
    /// it (shape and generation). Unlike a read, it does not refuse a cursor
    /// that has since expired: an acknowledgement of an old row is a no-op, not
    /// a failure (#2132).
    pub fn seq_of(&self, cursor: &str) -> Result<u64, CursorRejection> {
        let data = self.lock();
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
        Ok(seq)
    }

    /// Appends one row, committing it before it is visible to a reader.
    ///
    /// The commit runs in a detached task that also advances memory, so a
    /// caller that is dropped mid-write (an interrupted sync, a shutdown)
    /// cannot leave the disk ahead of memory and reuse a sequence number. A
    /// failed write is retried a bounded number of times and then marks the
    /// feed unhealthy without advancing memory.
    pub async fn record(&self, candidate: MentionCandidate) -> RecordOutcome {
        let feed = self.clone();
        let commit = Arc::clone(&self.inner.commit);
        let task = crate::executor::spawn(async move {
            let _writer = commit.lock_owned().await;
            feed.commit_one(candidate).await
        });
        task.await.unwrap_or(RecordOutcome::Failed)
    }

    async fn commit_one(&self, candidate: MentionCandidate) -> RecordOutcome {
        if !self.is_healthy() {
            return RecordOutcome::Failed;
        }
        let mut next = {
            let data = self.lock();
            if is_duplicate(&data, &candidate) {
                return RecordOutcome::Duplicate;
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
            message_kind: candidate.message_kind,
            mime_type: candidate.mime_type,
            size_bytes: candidate.size_bytes,
        });
        next.prune(now_ms());
        let mut committed = false;
        for attempt in 0..PERSIST_ATTEMPTS {
            let snapshot = next.clone();
            let persist = Arc::clone(&self.inner.persist);
            committed = crate::executor::spawn_blocking(move || persist(&snapshot))
                .await
                .unwrap_or(false);
            if committed {
                break;
            }
            koushi_diagnostics::record(
                koushi_diagnostics::DiagnosticEvent::new(
                    koushi_diagnostics::DiagnosticLevel::Warn,
                    "core.mention_feed",
                    "persist_failed",
                )
                .field(koushi_diagnostics::DiagnosticField::count(
                    "attempt",
                    attempt as u64 + 1,
                )),
            );
            if attempt + 1 < PERSIST_ATTEMPTS {
                crate::executor::sleep(PERSIST_RETRY_DELAY).await;
            }
        }
        if !committed {
            self.inner.healthy.store(false, Ordering::Release);
            koushi_diagnostics::record(koushi_diagnostics::DiagnosticEvent::new(
                koushi_diagnostics::DiagnosticLevel::Error,
                "core.mention_feed",
                "feed_unhealthy",
            ));
            self.inner.notify.notify_waiters();
            return RecordOutcome::Failed;
        }
        *self.lock() = next;
        self.inner.notify.notify_waiters();
        RecordOutcome::Recorded
    }

    fn resolve(&self, cursor: Option<&str>) -> Result<u64, CursorRejection> {
        let data = self.lock();
        let Some(cursor) = cursor else {
            // No cursor: everything still retained, so a client that starts
            // (or restarts) without saved state still receives what arrived
            // while it was away.
            return Ok(data.evicted_at(now_ms()));
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
        if seq < data.evicted_at(now_ms()) {
            return Err(CursorRejection::Expired);
        }
        Ok(seq)
    }

    fn after(&self, seq: u64, limit: usize) -> Vec<MentionEntry> {
        let data = self.lock();
        let floor = seq.max(data.evicted_at(now_ms()));
        data.entries
            .iter()
            .filter(|entry| entry.seq > floor)
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
