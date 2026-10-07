//! SearchActor: encrypted ngram-index search with canonical-text verification.
//!
//! ## Ownership
//! One `SearchActor` per account, owned by `AccountActor`. The actor starts
//! when a store-backed session is established and stops before sync in the
//! ordered shutdown (canon, overview.md Async rule 12 step 3: timelines →
//! search → sync).
//!
//! ## Indexing pipeline
//! The SDK client is built with a `MatrixSearchIndexStoreConfig` (encrypted
//! ngram index; configured by `StoreActor::account_search_index_config`).
//! The SDK's sync loop feeds the ngram index automatically as events arrive.
//!
//! The `SearchDocumentStore` (from `koushi-search`) holds attachment metadata
//! for the Files view only; it retains no message body and no edit text.
//! Timeline diffs still arrive via an `mpsc` channel (`SearchIndexMessage`)
//! forwarded from the `TimelineManagerActor`/`TimelineActor`.
//!
//! ## Query pipeline (overview.md Security Model — Search)
//! `SearchCommand::Query` → `MatrixLiteralSearchPager` over the persistent
//! ngram index (literal, newest-first, offset-free) → resolve each candidate's
//! current content from the encrypted event cache (edits and redactions
//! applied) → verify with `koushi_search::verify_candidate()` → emit
//! `SearchEvent::Results`. Candidates that fail verification (false positives,
//! stale index entries) are silently dropped — never surfaced as results.
//!
//! ## Fail-closed
//! If the search index key cannot be derived (credential store unreachable),
//! `Query` commands emit `SearchFailed { kind: IndexUnavailable }`. Query
//! parser and internal SDK failures keep separate coarse kinds. The actor never
//! falls back to a plaintext index (Security Model).
//!
//! ## Document-level mutations (overview.md Async rule 4, Security Model Search)
//! The SDK ngram index is fed by sync and crawl automatically; these paths only
//! maintain the Files view's attachment metadata.
//! - **Upsert**: a message carrying an attachment is recorded as a Files row;
//!   messages without one are not stored.
//! - **Edit**: `SearchDocumentStore::upsert_edit` updates the row's filename or
//!   attachment and marks it edited. Edit text is never retained.
//! - **Redact**: `SearchDocumentStore::redact` removes the row.
//! - **Unresolved replacement** (edit before original): held as a pending edit
//!   in `SearchDocumentStore` until the attachment row arrives.
//!
//! ## Debug redaction
//! Search queries and snippets must not appear in Debug of internal messages
//! (they can appear in `SearchEvent::Results` payloads — those are visible UI
//! state). `SearchActorMessage::Query` redacts the query in Debug.

mod attachment_admission;
#[cfg(test)]
mod history_scale;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_search::{
    AttachmentDocument, SearchCandidate, SearchDocumentStore, SearchEdit, SearchEditKey,
    SearchRoomFilter, SearchableEvent, SensitiveString, cjk_search_query_variants,
};
use koushi_state::{
    AppAction, AttachmentFilter, AttachmentScope, AttachmentSort, SearchCrawlerSettings,
    SearchCrawlerSpeed,
};
use tokio::sync::{broadcast, mpsc};

use crate::account_work::AccountWorkScheduler;
use crate::command_policy::{SEARCH_UNAVAILABLE_MESSAGE, search_scope_to_state};

use crate::executor;
use crate::search_crawler::{HistoryCrawlCheckpoint, HistoryCrawlPageResult};
use koushi_protocol::command::{SearchCommand, SearchScope};
use koushi_protocol::event::{CoreEvent, SearchEvent, SearchResultItem};
use koushi_protocol::failure::{CoreFailure, SearchFailureKind};
use koushi_protocol::ids::RequestId;

/// Maximum number of candidates requested from the SDK ngram index.
/// Verification filters this down; the final result set may be smaller.
const SEARCH_CANDIDATE_LIMIT: usize = 50;
/// Candidates requested per index page while verifying a query variant.
const SEARCH_CANDIDATE_PAGE: usize = 50;
/// Upper bound on candidates examined for one query variant, so a query whose
/// matches mostly fail verification still terminates with bounded work.
const SEARCH_CANDIDATE_SCAN_BUDGET: usize = 500;
/// Search index mutation queue capacity (canon, overview.md: 512).
pub const SEARCH_INDEX_MUTATION_QUEUE: usize = 512;
const SEARCH_ACTOR_SHUTDOWN_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const SEARCH_ACTOR_SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Automatic history crawling is held off for this long after the crawler first
/// has work, so it does not contend with user-visible pagination during the
/// startup window. Crawler timing is Rust-owned (not a user setting). The
/// maintainer confirmed a ~1 minute delay is fully acceptable (#123).
const CRAWLER_STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

fn search_scope_trace_label(scope: &SearchScope) -> &'static str {
    match scope {
        SearchScope::AllRooms => "all_rooms",
        SearchScope::CurrentRoom { .. } => "current_room",
        SearchScope::CurrentSpace { .. } => "current_space",
    }
}

fn search_room_filter_debug(filter: &SearchRoomFilter) -> (&'static str, usize) {
    match filter {
        SearchRoomFilter::AllRooms => ("all_rooms", 0),
        SearchRoomFilter::OnlyRooms(room_ids) => ("only_rooms", room_ids.len()),
    }
}

fn trace_search_start(
    request_id: RequestId,
    scope: &SearchScope,
    queued_ms: u128,
    query_bytes: usize,
    query_chars: usize,
    variants: usize,
    normalized_diff: bool,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "start")
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::token(
                "scope",
                search_scope_trace_label(scope),
            ))
            .field(DiagnosticField::milliseconds("queued", queued_ms))
            .field(DiagnosticField::count("query_bytes", query_bytes as u64))
            .field(DiagnosticField::count("query_chars", query_chars as u64))
            .field(DiagnosticField::count("variants", variants as u64))
            .field(DiagnosticField::boolean("normalized_diff", normalized_diff)),
    );
}

fn search_verify_diagnostic_event(
    request_id: RequestId,
    sdk_total_ms: u128,
    project_ms: u128,
    verification: &IndexCandidateVerification,
) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "verify")
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ))
        .field(DiagnosticField::count(
            "candidates_in_scope",
            verification.in_scope as u64,
        ))
        .field(DiagnosticField::count(
            "rooms",
            verification.rooms.len() as u64,
        ))
        .field(DiagnosticField::count(
            "cache_resolved",
            verification.resolved as u64,
        ))
        .field(DiagnosticField::count(
            "verified",
            verification.verified as u64,
        ))
        .field(DiagnosticField::milliseconds("sdk_total_ms", sdk_total_ms))
        .field(DiagnosticField::milliseconds("project_ms", project_ms))
}

// ---------------------------------------------------------------------------
// Public message type (forwarded from TimelineActor)
// ---------------------------------------------------------------------------

/// Timeline-side events forwarded to `SearchActor` for document-store
/// maintenance. Sent over the internal mpsc from the `TimelineManagerActor`.
///
/// The body fields in Upsert/Edit carry visible message text; they must not
/// appear in log output. `Debug` is manually implemented to redact them.
pub enum SearchIndexMessage {
    /// A visible message arrived (new or late decrypt). Index it.
    Upsert {
        room_id: String,
        event_id: String,
        sender: String,
        timestamp_ms: u64,
        body: Option<String>,
        attachment_filename: Option<String>,
        attachment: Option<AttachmentDocument>,
        /// True when the timeline projection sends this: it always carries the
        /// message's current visible content. A history crawl sends `false`, and
        /// the store then keeps an attachment an edit already produced.
        canonical: bool,
        /// The edit that produced this content, when the message is edited, so
        /// the content and the edit that follows are one guarded update.
        edit: Option<SearchEditKey>,
    },
    /// A message was edited. Update the document store.
    Edit {
        room_id: String,
        edit_event_id: String,
        target_event_id: String,
        sender: String,
        timestamp_ms: u64,
        body: Option<String>,
        attachment_filename: Option<String>,
        attachment: Option<AttachmentDocument>,
        /// See [`SearchIndexMessage::Upsert::canonical`]. A canonical edit also
        /// outranks a history edit, so an edit rollback applies.
        canonical: bool,
    },
    /// A message was redacted. Remove it from the document store.
    Redact { event_id: String },
}

// Redact body/filename from Debug — they are visible UI state but must not
// leak into internal log strings (spec: "SendText and EditText redact body
// in Debug and errors"; same principle applies here).
impl std::fmt::Debug for SearchIndexMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upsert {
                room_id, event_id, ..
            } => f
                .debug_struct("SearchIndexMessage::Upsert")
                .field("room_id", room_id)
                .field("event_id", event_id)
                .field("body", &"MessageBody(..)")
                .field("attachment", &"Attachment(..)")
                .finish(),
            Self::Edit {
                edit_event_id,
                target_event_id,
                ..
            } => f
                .debug_struct("SearchIndexMessage::Edit")
                .field("edit_event_id", edit_event_id)
                .field("target_event_id", target_event_id)
                .field("body", &"MessageBody(..)")
                .field("attachment", &"Attachment(..)")
                .finish(),
            Self::Redact { event_id } => f
                .debug_struct("SearchIndexMessage::Redact")
                .field("event_id", event_id)
                .finish(),
        }
    }
}

// ---------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------

/// Latest-wins snapshot of the joined rooms the crawler may index, from
/// `AppEffect::NotifySearchCrawlerRoomsAvailable`.
pub struct CrawlerRoomsNotification {
    pub room_ids: Vec<String>,
    /// Room id to the room's latest event id, for rooms that have one. A
    /// completed room whose latest event changed gets a catch-up crawl (#996).
    pub latest_event_ids: std::collections::BTreeMap<String, String>,
    pub settings: SearchCrawlerSettings,
}

impl std::fmt::Debug for CrawlerRoomsNotification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrawlerRoomsNotification")
            .field("room_count", &self.room_ids.len())
            .field("latest_count", &self.latest_event_ids.len())
            .field("settings", &self.settings)
            .finish()
    }
}

/// Messages routed to the `SearchActor`.
pub(crate) enum SearchActorMessage {
    /// A `SearchCommand::Query` from the command boundary.
    Query {
        request_id: RequestId,
        query: String,
        scope: SearchScope,
        room_filter: SearchRoomFilter,
        /// The account's content policy at submission. Adopting it here keeps
        /// verification on the same policy the state accepted the query under,
        /// even when the crawler notification that also carries it is deferred.
        content_policy: Option<SearchCrawlerSettings>,
        enqueued_at: Instant,
    },
    /// A `SearchCommand::Attachments` from the command boundary.
    Attachments {
        request_id: RequestId,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    StartHistoryCrawl {
        request_id: RequestId,
        room_id: String,
        settings: SearchCrawlerSettings,
    },
    StopHistoryCrawl {
        request_id: RequestId,
        room_id: String,
    },
    /// Notify the SearchActor that the set of joined rooms has changed.
    /// The actor starts an idempotent background crawl for each newly-observed
    /// room when `settings.speed != Paused` and the room is not already
    /// `Running` or `Completed`.
    RoomsAvailable(CrawlerRoomsNotification),
    /// Content-indexing settings changed (include_media_captions or
    /// include_filenames toggled). The actor must drop all rooms from
    /// `completed_rooms` so the next `RoomsAvailable` notification re-crawls
    /// them with the updated settings.
    InvalidateCrawlerCache,
    /// Clear the in-memory search document store and crawl queues so joined
    /// rooms can be indexed from scratch.
    RebuildIndex,
    Shutdown,
}

fn coalesce_contiguous_pending_queries(
    mut message: SearchActorMessage,
    pending: &mut VecDeque<SearchActorMessage>,
) -> (SearchActorMessage, usize) {
    debug_assert!(matches!(message, SearchActorMessage::Query { .. }));
    let mut dropped_queries = 0;

    while matches!(pending.front(), Some(SearchActorMessage::Query { .. })) {
        if let Some(next_query) = pending.pop_front() {
            message = next_query;
            dropped_queries += 1;
        }
    }

    (message, dropped_queries)
}

struct SearchSdkQueryResult {
    generation: u64,
    request_id: RequestId,
    query: String,
    scope: SearchScope,
    /// The filter the query was scoped with, so a re-verification under a new
    /// content policy can resume the same query.
    room_filter: SearchRoomFilter,
    /// The account's content policy generation when this query started.
    content_policy_generation: u64,
    projection: Result<SearchProjection, SearchFailureKind>,
    sdk_total_ms: u128,
}

/// Verified outcome of one SDK query task.
struct SearchProjection {
    results: Vec<koushi_state::SearchResult>,
    verification: IndexCandidateVerification,
    /// Milliseconds the task spent resolving and verifying candidates.
    project_ms: u128,
}

// Redact query text in Debug (queries may contain message content).
impl std::fmt::Debug for SearchActorMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Query {
                request_id,
                scope,
                room_filter,
                ..
            } => f
                .debug_struct("SearchActorMessage::Query")
                .field("request_id", request_id)
                .field("query", &"SearchQuery(..)")
                .field("scope", scope)
                .field("room_filter", &search_room_filter_debug(room_filter))
                .finish(),
            Self::Attachments {
                request_id,
                scope,
                filter,
                sort,
            } => f
                .debug_struct("SearchActorMessage::Attachments")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .finish(),
            Self::StartHistoryCrawl {
                request_id,
                room_id: _,
                settings,
            } => f
                .debug_struct("SearchActorMessage::StartHistoryCrawl")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("settings", settings)
                .finish(),
            Self::StopHistoryCrawl {
                request_id,
                room_id: _,
            } => f
                .debug_struct("SearchActorMessage::StopHistoryCrawl")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RoomsAvailable(notification) => f
                .debug_struct("SearchActorMessage::RoomsAvailable")
                .field("room_count", &notification.room_ids.len())
                .field("latest_count", &notification.latest_event_ids.len())
                .field("settings", &notification.settings)
                .finish(),
            Self::InvalidateCrawlerCache => {
                write!(f, "SearchActorMessage::InvalidateCrawlerCache")
            }
            Self::RebuildIndex => write!(f, "SearchActorMessage::RebuildIndex"),
            Self::Shutdown => write!(f, "SearchActorMessage::Shutdown"),
        }
    }
}

/// What the actor remembers about a room whose crawl completed this session.
#[derive(Clone)]
struct CompletedHistoryCrawl {
    /// Latest event id when the completed crawl started; a catch-up crawl
    /// stops at this event (#996).
    latest_event_id: Option<String>,
    processed: u64,
    indexed: u64,
}

/// Handle to the `SearchActor` background task.
pub struct SearchActorHandle {
    tx: mpsc::Sender<SearchActorMessage>,
    /// Channel for forwarding timeline mutations (SearchIndexMessage).
    /// Cloned and handed to TimelineManagerActor on creation.
    index_tx: mpsc::Sender<SearchIndexMessage>,
    task: Option<executor::JoinHandle<()>>,
}

impl SearchActorHandle {
    pub async fn send_command(&self, command: SearchCommand) -> bool {
        self.send_query_command(command, None).await
    }

    /// Send a command, carrying the account's content policy when the caller has
    /// the authoritative one.
    pub async fn send_query_command(
        &self,
        command: SearchCommand,
        content_policy: Option<SearchCrawlerSettings>,
    ) -> bool {
        let msg = match command {
            SearchCommand::Query {
                request_id,
                query,
                scope,
                room_filter,
            } => SearchActorMessage::Query {
                request_id,
                query,
                scope,
                room_filter,
                content_policy,
                enqueued_at: Instant::now(),
            },
            SearchCommand::Attachments {
                request_id,
                scope,
                filter,
                sort,
            } => SearchActorMessage::Attachments {
                request_id,
                scope,
                filter,
                sort,
            },
            SearchCommand::StartHistoryCrawl {
                request_id,
                room_id,
                settings,
            } => SearchActorMessage::StartHistoryCrawl {
                request_id,
                room_id,
                settings,
            },
            SearchCommand::StopHistoryCrawl {
                request_id,
                room_id,
            } => SearchActorMessage::StopHistoryCrawl {
                request_id,
                room_id,
            },
        };
        self.tx.send(msg).await.is_ok()
    }

    pub async fn shutdown(self) -> bool {
        self.shutdown_with_timeouts(
            SEARCH_ACTOR_SHUTDOWN_SEND_TIMEOUT,
            SEARCH_ACTOR_SHUTDOWN_JOIN_TIMEOUT,
        )
        .await
    }

    async fn shutdown_with_timeouts(
        mut self,
        send_timeout: Duration,
        join_timeout: Duration,
    ) -> bool {
        let sent = matches!(
            executor::timeout(send_timeout, self.tx.send(SearchActorMessage::Shutdown)).await,
            Ok(Ok(()))
        );
        let Some(mut task) = self.task.take() else {
            return sent;
        };
        if sent && executor::timeout(join_timeout, &mut task).await.is_ok() {
            return true;
        }
        task.abort();
        let _ = task.await;
        false
    }

    /// Try to notify the actor that the set of joined rooms has changed.
    /// Room availability is latest-wins background state; callers that get a
    /// full inbox keep the returned payload and retry later instead of blocking
    /// user-visible commands on crawler work.
    pub fn try_notify_rooms_available(
        &self,
        notification: CrawlerRoomsNotification,
    ) -> Result<(), CrawlerRoomsNotification> {
        match self
            .tx
            .try_send(SearchActorMessage::RoomsAvailable(notification))
        {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(
                SearchActorMessage::RoomsAvailable(notification),
            )) => Err(notification),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(
                SearchActorMessage::RoomsAvailable(notification),
            )) => Err(notification),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                unreachable!("try_notify_rooms_available only sends RoomsAvailable messages")
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                unreachable!("try_notify_rooms_available only sends RoomsAvailable messages")
            }
        }
    }

    /// Invalidate the actor's completed-room cache because content-indexing
    /// settings changed.  The actor drops all rooms from `completed_rooms` so
    /// the next `RoomsAvailable` notification triggers re-crawls.
    ///
    /// Uses `send` (not `try_send`) for reliable delivery.
    pub async fn invalidate_crawler_cache(&self) {
        let _ = self
            .tx
            .send(SearchActorMessage::InvalidateCrawlerCache)
            .await;
    }

    /// Clear indexed search documents and crawler progress in the actor.
    pub async fn rebuild_search_index(&self) {
        let _ = self.tx.send(SearchActorMessage::RebuildIndex).await;
    }

    /// Return a sender for forwarding timeline mutations (indexable events).
    /// The `TimelineManagerActor` holds this sender and forwards diffs here.
    pub fn index_sender(&self) -> mpsc::Sender<SearchIndexMessage> {
        self.index_tx.clone()
    }
}

impl Drop for SearchActorHandle {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// Actor
// ---------------------------------------------------------------------------

pub(crate) struct SearchActor {
    session: Arc<MatrixClientSession>,
    document_store: SearchDocumentStore,
    // Body-free retries are bounded by the mutation queue. A single completed
    // crawl page waits separately; no next page starts while either is pending.
    attachment_retries: VecDeque<SearchIndexMessage>,
    queued_crawl_index: VecDeque<SearchIndexMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    msg_rx: mpsc::Receiver<SearchActorMessage>,
    /// Read-ahead actor messages drained from `msg_rx` while coalescing a burst
    /// of pending search queries. Non-query messages stay in order here.
    deferred_messages: VecDeque<SearchActorMessage>,
    /// Current search generation. Every submitted query increments this and any
    /// SDK supplement finishing with an older generation is dropped in actor.
    active_query_generation: u64,
    active_sdk_search: Option<executor::JoinHandle<SearchSdkQueryResult>>,
    account_work: AccountWorkScheduler,
    /// Element-style checkpoint queue for history crawling. The actor starts
    /// exactly one bounded `/messages` page at a time; unfinished rooms are
    /// pushed to the back so other rooms get a turn before the next page.
    crawl_queue: VecDeque<HistoryCrawlCheckpoint>,
    /// Room ids currently present in `crawl_queue`.
    queued_crawl_rooms: HashSet<String>,
    /// Current joined-room set from the latest `RoomsAvailable` notification.
    /// Manual probes are allowed outside this set; auto crawls are pruned
    /// against it whenever room membership changes.
    available_crawl_rooms: HashSet<String>,
    /// Active one-page crawl task.
    active_crawl_page: Option<executor::JoinHandle<HistoryCrawlPageResult>>,
    /// Checkpoint currently owned by `active_crawl_page`. Kept separately so a
    /// room-list update can abort a page whose room disappeared before the task
    /// returns stale progress.
    active_crawl_checkpoint: Option<HistoryCrawlCheckpoint>,
    /// Rooms whose history has been fully crawled at least once this session.
    /// `handle_rooms_available` skips their auto-start unless their latest
    /// event changed since completion, which queues a catch-up (#996).
    completed_rooms: HashMap<String, CompletedHistoryCrawl>,
    /// Room id to latest event id from the newest `RoomsAvailable` snapshot.
    latest_event_ids: std::collections::BTreeMap<String, String>,
    /// Monotonically increasing generation counter. Incremented each time
    /// content-indexing settings change (via `InvalidateCrawlerCache`). Every
    /// queued checkpoint records the current generation, and stale page results
    /// are discarded before they can update the index or reducer state.
    crawl_settings_generation: u64,
    /// True once the startup delay has elapsed (automatic crawls may start).
    crawl_delay_elapsed: bool,
    /// One-shot startup-delay timer; its completion is awaited in `run`.
    crawl_delay_timer: Option<executor::JoinHandle<()>>,
    /// Bumped whenever the account's content policy changes.
    ///
    /// A query verifies candidates with the policy captured when it started, so
    /// its result is only publishable while this still matches: otherwise it
    /// could surface a caption or filename the account has since opted out of.
    content_policy_generation: u64,
    /// Content-indexing settings the verifier must apply.
    ///
    /// The crawler honours these when it indexes, but the persistent index also
    /// receives events from sync, so verification is the enforcement point: a
    /// query must never match a caption or filename the account opted out of.
    /// Seeded restrictively until the account's own settings arrive with the
    /// first `RoomsAvailable` notification, so an opted-out value can never be
    /// exposed in the window before that.
    crawler_settings: SearchCrawlerSettings,
}

/// Outcome of verifying index candidates against the event cache.
#[derive(Default)]
struct IndexCandidateVerification {
    /// Candidates inside the Rust-resolved scope filter.
    in_scope: usize,
    /// Rooms the examined candidates came from.
    rooms: HashSet<String>,
    /// Candidates whose current content was available in the cache.
    resolved: usize,
    /// Candidates that matched the query, in pager order and deduplicated by
    /// resolved identity.
    verified: usize,
    results: Vec<VerifiedCandidate>,
}

/// The `(timestamp, event_id)` key the persistent index pages a room's matches
/// by, descending.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IndexOrderKey {
    timestamp_millis: i64,
    event_id: String,
}

/// A verified candidate together with the key its candidate scan was ordered by.
///
/// Verification filters candidates, so the newest results are the newest *by
/// this key*: the scan can only have seen candidates at or below it. The
/// resolved content's timestamp (an edit's, for example) need not agree with it,
/// so selection must not re-order by that timestamp before capping the results.
struct VerifiedCandidate {
    index_key: IndexOrderKey,
    result: koushi_state::SearchResult,
}

impl SearchActor {
    fn new(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        msg_rx: mpsc::Receiver<SearchActorMessage>,
        account_work: AccountWorkScheduler,
    ) -> Self {
        Self {
            session,
            document_store: SearchDocumentStore::default(),
            attachment_retries: VecDeque::new(),
            queued_crawl_index: VecDeque::new(),
            action_tx,
            event_tx,
            msg_rx,
            deferred_messages: VecDeque::new(),
            active_query_generation: 0,
            active_sdk_search: None,
            account_work,
            crawl_queue: VecDeque::new(),
            queued_crawl_rooms: HashSet::new(),
            available_crawl_rooms: HashSet::new(),
            active_crawl_page: None,
            active_crawl_checkpoint: None,
            completed_rooms: HashMap::new(),
            latest_event_ids: std::collections::BTreeMap::new(),
            crawl_settings_generation: 0,
            crawl_delay_elapsed: false,
            crawl_delay_timer: None,
            content_policy_generation: 0,
            crawler_settings: restricted_crawler_settings(),
        }
    }

    /// Spawn the actor and return its handle.
    pub fn spawn(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        account_work: AccountWorkScheduler,
    ) -> SearchActorHandle {
        let (tx, msg_rx) = mpsc::channel(64);
        let (index_tx, index_rx) = mpsc::channel(SEARCH_INDEX_MUTATION_QUEUE);

        let actor = Self::new(session, action_tx, event_tx, msg_rx, account_work);

        // Spawn the actor task.
        let task = executor::spawn(actor.run(index_rx));

        SearchActorHandle {
            tx,
            index_tx,
            task: Some(task),
        }
    }

    async fn run(mut self, mut index_rx: mpsc::Receiver<SearchIndexMessage>) {
        loop {
            if let Some(msg) = self.deferred_messages.pop_front() {
                if !self.handle_actor_message(msg).await {
                    break;
                }
                continue;
            }

            tokio::select! {
                biased;
                msg = self.msg_rx.recv() => {
                    let Some(msg) = msg else { break };
                    if !self.handle_actor_message(msg).await {
                        break;
                    }
                }
                crawl_result = async {
                    self.active_crawl_page.as_mut().unwrap().await
                }, if self.active_crawl_page.is_some() => {
                    self.active_crawl_page = None;
                    self.active_crawl_checkpoint = None;
                    if let Ok(result) = crawl_result {
                        self.handle_history_crawl_page_result(result).await;
                    }
                    self.start_next_history_crawl_page();
                }
                _ = async {
                    self.crawl_delay_timer.as_mut().unwrap().await.ok();
                }, if self.crawl_delay_timer.is_some() => {
                    self.crawl_delay_timer = None;
                    self.crawl_delay_elapsed = true;
                    self.start_next_history_crawl_page();
                }
                sdk_result = async {
                    self.active_sdk_search.as_mut().unwrap().await
                }, if self.active_sdk_search.is_some() => {
                    self.active_sdk_search = None;
                    if let Ok(result) = sdk_result {
                        self.handle_sdk_query_result(result).await;
                    }
                }
                _ = std::future::ready(()), if !self.queued_crawl_index.is_empty()
                    && self.attachment_retries.len() < SEARCH_INDEX_MUTATION_QUEUE => {
                    let message = self.queued_crawl_index.pop_front().unwrap();
                    self.handle_index(message).await;
                    self.start_next_history_crawl_page();
                }
                index_msg = index_rx.recv(), if self.attachment_retries.len() < SEARCH_INDEX_MUTATION_QUEUE => {
                    let Some(index_msg) = index_msg else {
                        // Timeline sender dropped — that's fine (e.g. on shutdown).
                        continue;
                    };
                    self.handle_index(index_msg).await;
                }
            }
        }
    }

    async fn handle_actor_message(&mut self, msg: SearchActorMessage) -> bool {
        match msg {
            SearchActorMessage::Shutdown => {
                if let Some(task) = self.active_sdk_search.take() {
                    abort_and_await_task(task).await;
                }
                self.stop_all_history_crawls().await;
                false
            }
            SearchActorMessage::Query {
                request_id,
                query,
                scope,
                room_filter,
                content_policy,
                enqueued_at,
            } => {
                self.drain_available_actor_messages();
                let (latest_query, dropped_queries) = coalesce_contiguous_pending_queries(
                    SearchActorMessage::Query {
                        request_id,
                        query,
                        scope,
                        room_filter,
                        content_policy,
                        enqueued_at,
                    },
                    &mut self.deferred_messages,
                );
                if let SearchActorMessage::Query {
                    request_id,
                    query,
                    scope,
                    room_filter,
                    content_policy,
                    enqueued_at,
                } = latest_query
                {
                    if dropped_queries > 0 {
                        record(
                            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "coalesce")
                                .field(DiagnosticField::request_id(
                                    "request_id",
                                    request_id.connection_id.0,
                                    request_id.sequence,
                                ))
                                .field(DiagnosticField::count(
                                    "dropped_queries",
                                    dropped_queries as u64,
                                ))
                                .field(DiagnosticField::count(
                                    "deferred_messages",
                                    self.deferred_messages.len() as u64,
                                )),
                        );
                    }
                    self.handle_query(
                        request_id,
                        &query,
                        scope,
                        room_filter,
                        content_policy,
                        enqueued_at,
                    )
                    .await;
                }
                true
            }
            SearchActorMessage::Attachments {
                request_id,
                scope,
                filter,
                sort,
            } => {
                self.handle_attachments(request_id, scope, filter, sort)
                    .await;
                true
            }
            SearchActorMessage::StartHistoryCrawl {
                request_id,
                room_id,
                settings,
            } => {
                self.handle_start_history_crawl(request_id, room_id, settings)
                    .await;
                true
            }
            SearchActorMessage::StopHistoryCrawl {
                request_id,
                room_id,
            } => {
                self.handle_stop_history_crawl(request_id, room_id).await;
                true
            }
            SearchActorMessage::RoomsAvailable(notification) => {
                self.handle_rooms_available(notification).await;
                true
            }
            SearchActorMessage::InvalidateCrawlerCache => {
                // Content-indexing settings changed — bump the generation and
                // drop queued/in-flight checkpoints so the next `RoomsAvailable`
                // notification re-crawls all rooms with the new settings.
                self.crawl_settings_generation = self.crawl_settings_generation.wrapping_add(1);
                self.invalidate_history_crawler_cache().await;
                true
            }
            SearchActorMessage::RebuildIndex => {
                self.rebuild_search_index().await;
                true
            }
        }
    }

    fn drain_available_actor_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            self.deferred_messages.push_back(msg);
        }
    }

    async fn handle_query(
        &mut self,
        request_id: RequestId,
        query: &str,
        scope: SearchScope,
        room_filter: SearchRoomFilter,
        content_policy: Option<SearchCrawlerSettings>,
        enqueued_at: Instant,
    ) {
        // The submission's policy is authoritative for this query, so adopt it
        // before capturing the generation the result will be checked against.
        if let Some(settings) = content_policy {
            self.set_crawler_settings(settings);
        }
        self.active_query_generation = self.active_query_generation.wrapping_add(1);
        let generation = self.active_query_generation;
        if let Some(task) = self.active_sdk_search.take() {
            abort_and_await_task(task).await;
            record_stale_sdk_drop(
                request_id,
                generation.saturating_sub(1),
                generation,
                "aborted",
            );
        }

        let query = query.trim();
        if query.trim().is_empty() {
            // The state admits and publishes the empty result set.
            self.emit_search_succeeded(request_id, query, &scope, Vec::new())
                .await;
            return;
        }

        let queued_ms = enqueued_at.elapsed().as_millis();
        let variants = cjk_search_query_variants(query);
        trace_search_start(
            request_id,
            &scope,
            queued_ms,
            query.len(),
            query.chars().count(),
            variants.len(),
            variants.iter().any(|variant| variant != query),
        );

        // Search is index-first: there is no in-process history to scan, so the
        // first emission is the verified SDK page. Emitting an empty placeholder
        // first would look like a settled empty answer to callers.
        if matches!(&room_filter, SearchRoomFilter::OnlyRooms(room_ids) if room_ids.is_empty()) {
            self.emit_search_succeeded(request_id, query, &scope, Vec::new())
                .await;
            return;
        }

        let session = self.session.clone();
        let query = query.to_owned();
        let sdk_scope = matrix_sdk_search_scope(&scope, &room_filter);
        let settings = self.crawler_settings.clone();
        let content_policy_generation = self.content_policy_generation;
        self.active_sdk_search = Some(executor::spawn(run_sdk_query(
            session,
            generation,
            request_id,
            query,
            scope,
            room_filter,
            sdk_scope,
            settings,
            content_policy_generation,
            variants,
        )));
    }

    async fn handle_sdk_query_result(&mut self, result: SearchSdkQueryResult) {
        if result.generation != self.active_query_generation {
            record_stale_sdk_drop(
                result.request_id,
                result.generation,
                self.active_query_generation,
                "completed",
            );
            return;
        }

        if result.content_policy_generation != self.content_policy_generation {
            // The account's content policy changed while this query ran, so its
            // verification used a policy that is no longer current: publishing it
            // could show a caption or filename the account has opted out of.
            // Verify the same query again under the current policy.
            record(
                DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "policy_changed")
                    .field(DiagnosticField::request_id(
                        "request_id",
                        result.request_id.connection_id.0,
                        result.request_id.sequence,
                    ))
                    .field(DiagnosticField::count(
                        "captured_generation",
                        result.content_policy_generation,
                    ))
                    .field(DiagnosticField::count(
                        "current_generation",
                        self.content_policy_generation,
                    )),
            );
            let SearchSdkQueryResult {
                generation,
                request_id,
                query,
                scope,
                room_filter,
                ..
            } = result;
            let variants = cjk_search_query_variants(&query);
            let sdk_scope = matrix_sdk_search_scope(&scope, &room_filter);
            let settings = self.crawler_settings.clone();
            let content_policy_generation = self.content_policy_generation;
            self.active_sdk_search = Some(executor::spawn(run_sdk_query(
                self.session.clone(),
                generation,
                request_id,
                query,
                scope,
                room_filter,
                sdk_scope,
                settings,
                content_policy_generation,
                variants,
            )));
            return;
        }

        let projection = match result.projection {
            Ok(projection) => projection,
            Err(kind) => {
                record_search_sdk_failure(result.request_id, kind, result.sdk_total_ms);
                self.emit_search_failed(result.request_id, &result.query, &result.scope, kind)
                    .await;
                return;
            }
        };
        record(search_verify_diagnostic_event(
            result.request_id,
            result.sdk_total_ms,
            projection.project_ms,
            &projection.verification,
        ));
        let projected_results = projection.results;
        record_search_finish(
            result.request_id,
            "finish",
            &projected_results,
            result.sdk_total_ms,
        );

        // The state publishes an admitted result set (it is the only owner of
        // the account's content policy and of the accepted query identity), so
        // the actor only asks for it here.
        self.emit_search_succeeded(
            result.request_id,
            &result.query,
            &result.scope,
            projected_results,
        )
        .await;
    }

    async fn handle_attachments(
        &mut self,
        request_id: RequestId,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    ) {
        if !self.reconcile_attachment_redactions().await {
            let _ = self
                .action_tx
                .send(vec![AppAction::FilesViewQueryFailed {
                    request_id: request_id.sequence,
                    message: SEARCH_UNAVAILABLE_MESSAGE.to_owned(),
                }])
                .await;
            self.emit(CoreEvent::Search(SearchEvent::AttachmentsFailed {
                request_id,
                message: SEARCH_UNAVAILABLE_MESSAGE.to_owned(),
            }));
            return;
        }
        self.start_next_history_crawl_page();
        let results = self.document_store.attachments(&scope, &filter, sort);

        let _ = self
            .action_tx
            .send(vec![AppAction::FilesViewQuerySucceeded {
                request_id: request_id.sequence,
                items: results.clone(),
            }])
            .await;

        self.emit(CoreEvent::Search(SearchEvent::AttachmentsResults {
            request_id,
            results,
        }));
    }

    /// Ask the state to admit a result set and publish it.
    async fn emit_search_succeeded(
        &self,
        request_id: RequestId,
        query: &str,
        scope: &SearchScope,
        results: Vec<koushi_state::SearchResult>,
    ) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchSucceeded {
                request_id: request_id.sequence,
                connection_id: request_id.connection_id.0,
                query: query.to_owned(),
                scope: search_scope_to_state(scope),
                results,
            }])
            .await;
    }

    /// Settle a query that could not produce results, so the UI stops waiting.
    async fn emit_search_failed(
        &self,
        request_id: RequestId,
        query: &str,
        scope: &SearchScope,
        kind: SearchFailureKind,
    ) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
                connection_id: request_id.connection_id.0,
                query: query.to_owned(),
                scope: search_scope_to_state(scope),
                message: SEARCH_UNAVAILABLE_MESSAGE.to_owned(),
            }])
            .await;
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure: CoreFailure::SearchFailed { kind },
        });
    }

    /// Whether the account's content policy excludes this attachment metadata.
    ///
    /// The Files view lists attachments by filename, which the account can opt
    /// out of indexing. The timeline projection sends attachment metadata without
    /// consulting the policy, so the policy is applied here, once, for every
    /// producer.
    fn attachment_policy_excludes(
        &self,
        attachment: &Option<AttachmentDocument>,
        attachment_filename: Option<&str>,
    ) -> bool {
        !self.crawler_settings.include_filenames
            && (attachment.is_some() || attachment_filename.is_some())
    }

    /// Apply one index message to the document store.
    ///
    /// Returns the row it changed, for the `IndexUpdated` wake-up, when there is
    /// one. Callers that rebuild rows in bulk (the Files-view refresh) use this
    /// directly so a bulk rebuild does not emit one event per message.
    fn apply_index_message(&mut self, msg: SearchIndexMessage) -> Option<(String, String)> {
        match msg {
            SearchIndexMessage::Upsert {
                room_id,
                event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
                canonical,
                edit,
            } => {
                if self.attachment_policy_excludes(&attachment, attachment_filename.as_deref()) {
                    return None;
                }
                // Capture the visible-state identifiers before the payload is
                // consumed by the document store, so `IndexUpdated` can wake
                // pollers (room/event ids only — never the body).
                let indexed_room_id = room_id.clone();
                let indexed_event_id = event_id.clone();
                let event = SearchableEvent {
                    room_id,
                    event_id,
                    sender,
                    timestamp_ms,
                    body: body.map(SensitiveString::new),
                    attachment_filename: attachment_filename.map(SensitiveString::new),
                    attachment,
                };
                self.document_store.upsert_message(event, canonical, edit);
                Some((indexed_room_id, indexed_event_id))
            }
            SearchIndexMessage::Edit {
                room_id,
                edit_event_id,
                target_event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
                canonical,
            } => {
                if self.attachment_policy_excludes(&attachment, attachment_filename.as_deref()) {
                    return None;
                }
                // The Edit payload only names the target event id; resolve its
                // room id from the document store so `IndexUpdated` stays honest
                // (no fabricated room id). An edit whose original is not yet
                // indexed is stored as a pending edit and emits no event.
                let edited_room_id = self
                    .document_store
                    .room_id_of(&target_event_id)
                    .map(str::to_owned);
                let edited_event_id = target_event_id.clone();
                let edit = SearchEdit {
                    room_id,
                    edit_event_id,
                    target_event_id,
                    sender,
                    timestamp_ms,
                    body: body.map(SensitiveString::new),
                    attachment_filename: attachment_filename.map(SensitiveString::new),
                    attachment,
                };
                self.document_store.upsert_edit(edit, canonical);
                edited_room_id.map(|room_id| (room_id, edited_event_id))
            }
            SearchIndexMessage::Redact { event_id } => {
                self.document_store.redact(&event_id);
                None
            }
        }
    }

    async fn handle_start_history_crawl(
        &mut self,
        request_id: RequestId,
        room_id: String,
        settings: SearchCrawlerSettings,
    ) {
        // The command's settings configure this crawl only. The verifier and the
        // Files projection follow the account's policy, which arrives with a
        // query and with the room-list notification, so a caller-supplied crawl
        // policy cannot widen what a search may match.
        self.remove_history_crawl_room(&room_id).await;
        self.completed_rooms.remove(&room_id);
        if settings.speed == SearchCrawlerSpeed::Paused {
            self.emit_history_crawl_stopped(room_id).await;
            return;
        }
        self.enqueue_history_crawl(
            HistoryCrawlCheckpoint::new(
                room_id,
                settings.clone(),
                self.crawl_settings_generation,
                true,
            ),
            request_id.sequence,
        )
        .await;
        if settings.speed != SearchCrawlerSpeed::Paused {
            self.start_next_history_crawl_page();
        }
    }

    async fn handle_stop_history_crawl(&mut self, _request_id: RequestId, room_id: String) {
        self.remove_history_crawl_room(&room_id).await;
        self.emit_history_crawl_stopped(room_id).await;
    }

    /// Auto-start idempotent background crawls when the account observes a new
    /// set of joined rooms.
    ///
    /// This mirrors Element's Seshat crawler shape: maintain a checkpoint queue
    /// and process one `/messages` page at a time. If a page has a continuation
    /// token, push_back(next_checkpoint) so other rooms get a turn first.
    async fn handle_rooms_available(&mut self, notification: CrawlerRoomsNotification) {
        let CrawlerRoomsNotification {
            room_ids,
            latest_event_ids,
            settings,
        } = notification;
        self.available_crawl_rooms = room_ids.iter().cloned().collect();
        self.latest_event_ids = latest_event_ids;
        // The account's content policy applies to queries even while the
        // crawler is paused, so record it before the speed check.
        self.set_crawler_settings(settings.clone());

        // Membership changes prune the crawl set (and its durable record) even
        // while the crawler is paused; otherwise a departed room stays
        // committed and the restart after it rejoins skips its crawl.
        let mut stopped_room_ids = self.retain_history_crawl_rooms().await;
        if let Some(room_id) = self.abort_active_history_crawl_if_retired().await {
            stopped_room_ids.push(room_id);
        }
        for room_id in stopped_room_ids {
            self.emit_history_crawl_stopped(room_id).await;
        }

        if settings.speed == SearchCrawlerSpeed::Paused {
            self.stop_all_history_crawls().await;
            return;
        }

        for room_id in room_ids {
            if let Some(checkpoint) = self.catch_up_checkpoint(&room_id, &settings) {
                self.enqueue_history_crawl(checkpoint, 0).await;
                continue;
            }
            if self.history_crawl_room_is_known(&room_id) {
                continue;
            }
            self.enqueue_history_crawl(
                HistoryCrawlCheckpoint::new(
                    room_id,
                    settings.clone(),
                    self.crawl_settings_generation,
                    false,
                ),
                0,
            )
            .await;
        }
        self.start_next_history_crawl_page();
    }

    async fn enqueue_history_crawl(&mut self, checkpoint: HistoryCrawlCheckpoint, request_id: u64) {
        if self.queued_crawl_rooms.insert(checkpoint.room_id.clone()) {
            self.crawl_queue.push_back(checkpoint);
            let Some(queued) = self.crawl_queue.back() else {
                return;
            };
            let _ = self
                .action_tx
                .send(vec![AppAction::HistoryCrawlStarted {
                    request_id,
                    room_id: queued.room_id.clone(),
                    timestamp_ms: crate::time::current_epoch_ms(),
                }])
                .await;
        }
    }

    async fn emit_history_crawl_stopped(&self, room_id: String) {
        let _ = self
            .action_tx
            .send(vec![AppAction::HistoryCrawlStopped { room_id }])
            .await;
    }

    /// #996: a completed room whose latest event changed since completion, and
    /// whose latest event is not already indexed (an open room's timeline
    /// indexes live), gets a catch-up crawl bounded by the recorded event.
    fn catch_up_checkpoint(
        &self,
        room_id: &str,
        settings: &SearchCrawlerSettings,
    ) -> Option<HistoryCrawlCheckpoint> {
        let completed = self.completed_rooms.get(room_id)?;
        let latest = self.latest_event_ids.get(room_id)?;
        if completed.latest_event_id.as_deref() == Some(latest.as_str())
            || self.document_store.contains(latest)
            || self.queued_crawl_rooms.contains(room_id)
            || self
                .active_crawl_checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.room_id == room_id)
        {
            return None;
        }
        let checkpoint = match &completed.latest_event_id {
            Some(after_event_id) => HistoryCrawlCheckpoint::catch_up(
                room_id.to_owned(),
                settings.clone(),
                self.crawl_settings_generation,
                after_event_id.clone(),
                completed.processed,
                completed.indexed,
            ),
            // No boundary was known when the room completed (it had no latest
            // event), so the only safe catch-up is a fresh crawl.
            None => HistoryCrawlCheckpoint::new(
                room_id.to_owned(),
                settings.clone(),
                self.crawl_settings_generation,
                false,
            ),
        };
        Some(checkpoint)
    }

    fn history_crawl_room_is_known(&self, room_id: &str) -> bool {
        self.completed_rooms.contains_key(room_id)
            || self.queued_crawl_rooms.contains(room_id)
            || self
                .active_crawl_checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.room_id == room_id)
    }

    fn start_next_history_crawl_page(&mut self) {
        if self.active_crawl_page.is_some()
            || !self.queued_crawl_index.is_empty()
            || !self.attachment_retries.is_empty()
        {
            return;
        }
        // Startup delay: hold AUTOMATIC crawls until the delay elapses; manual
        // (explicit StartHistoryCrawl) checkpoints bypass it.
        if !self.crawl_delay_elapsed {
            // During the startup delay only MANUAL checkpoints may start. If one
            // is queued (even behind automatic work), pull it to the front so the
            // pop below starts it; otherwise arm the delay timer and wait.
            match self.crawl_queue.iter().position(|c| c.manual) {
                Some(pos) => {
                    if let Some(manual) = self.crawl_queue.remove(pos) {
                        self.crawl_queue.push_front(manual);
                    }
                }
                None => {
                    if !self.crawl_queue.is_empty() && self.crawl_delay_timer.is_none() {
                        self.crawl_delay_timer = Some(executor::spawn(async {
                            executor::sleep(CRAWLER_STARTUP_DELAY).await;
                        }));
                    }
                    return;
                }
            }
        }
        let Some(mut checkpoint) = self.crawl_queue.pop_front() else {
            return;
        };
        if checkpoint.first_page {
            checkpoint.latest_event_id_at_start =
                self.latest_event_ids.get(&checkpoint.room_id).cloned();
        }
        self.queued_crawl_rooms.remove(&checkpoint.room_id);
        if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
            self.start_next_history_crawl_page();
            return;
        }
        let handle = crate::search_crawler::spawn_history_crawl_page(
            self.session.clone(),
            self.account_work.clone(),
            checkpoint.clone(),
        );
        self.active_crawl_checkpoint = Some(checkpoint);
        self.active_crawl_page = Some(handle);
    }

    async fn handle_history_crawl_page_result(&mut self, result: HistoryCrawlPageResult) {
        match result {
            HistoryCrawlPageResult::Success {
                checkpoint,
                messages,
                completed,
                work_permit,
            } => {
                if checkpoint.settings_generation != self.crawl_settings_generation {
                    return;
                }
                if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
                    return;
                }
                // The permit covers page/index work, not Files cache admission.
                drop(work_permit);
                self.queue_crawl_messages(messages);
                let _ = self
                    .action_tx
                    .send(vec![AppAction::HistoryCrawlProgress {
                        room_id: checkpoint.room_id.clone(),
                        processed: checkpoint.processed,
                        indexed: checkpoint.indexed,
                        timestamp_ms: crate::time::current_epoch_ms(),
                    }])
                    .await;
                if completed {
                    let crawl = CompletedHistoryCrawl {
                        latest_event_id: checkpoint.latest_event_id_at_start.clone(),
                        processed: checkpoint.processed,
                        indexed: checkpoint.indexed,
                    };
                    self.completed_rooms
                        .insert(checkpoint.room_id.clone(), crawl);
                    let _ = self
                        .action_tx
                        .send(vec![AppAction::HistoryCrawlCompleted {
                            room_id: checkpoint.room_id.clone(),
                            indexed: checkpoint.indexed,
                            timestamp_ms: crate::time::current_epoch_ms(),
                        }])
                        .await;
                    self.emit(CoreEvent::Search(SearchEvent::HistoryCrawlCompleted {
                        room_id: checkpoint.room_id.clone(),
                        indexed: checkpoint.indexed,
                    }));
                    // An event that arrived while this crawl ran has no later
                    // notification to trigger its catch-up; check now.
                    if self.available_crawl_rooms.contains(&checkpoint.room_id)
                        && let Some(catch_up) =
                            self.catch_up_checkpoint(&checkpoint.room_id, &checkpoint.settings)
                    {
                        self.enqueue_history_crawl(catch_up, 0).await;
                    }
                } else {
                    let next_checkpoint = checkpoint;
                    self.queued_crawl_rooms
                        .insert(next_checkpoint.room_id.clone());
                    self.crawl_queue.push_back(next_checkpoint);
                }
            }
            HistoryCrawlPageResult::Failed { checkpoint, kind } => {
                if checkpoint.settings_generation != self.crawl_settings_generation {
                    return;
                }
                if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
                    return;
                }
                let _ = self
                    .action_tx
                    .send(vec![AppAction::HistoryCrawlFailed {
                        room_id: checkpoint.room_id,
                        kind,
                        timestamp_ms: crate::time::current_epoch_ms(),
                    }])
                    .await;
            }
            HistoryCrawlPageResult::Preempted { checkpoint } => {
                if checkpoint.settings_generation != self.crawl_settings_generation {
                    return;
                }
                if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
                    return;
                }
                // No progress was made; retry this checkpoint next. The crawler's
                // next acquire blocks behind the waiting timeline (waiting_timeline),
                // so this does not livelock.
                self.queued_crawl_rooms.insert(checkpoint.room_id.clone());
                self.crawl_queue.push_front(checkpoint);
            }
        }
    }

    /// Adopt the account's content-indexing settings.
    ///
    /// A content-policy change invalidates any query that is verifying under the
    /// previous policy; a speed-only change does not.
    fn set_crawler_settings(&mut self, settings: SearchCrawlerSettings) {
        if content_policy_changed(&self.crawler_settings, &settings) {
            self.content_policy_generation = self.content_policy_generation.wrapping_add(1);
            // Rows admitted under the previous policy are no longer admissible: a
            // filename the account has just opted out of must not stay readable
            // through the Files view until some later message happens to replace
            // it. The next Files query rebuilds them under the new policy.
            self.document_store.clear();
            self.attachment_retries.clear();
            self.queued_crawl_index.clear();
        }
        self.crawler_settings = settings;
    }

    async fn retain_history_crawl_rooms(&mut self) -> Vec<String> {
        let mut stopped_room_ids = std::collections::BTreeSet::new();
        for room_id in self.completed_rooms.keys() {
            if !self.available_crawl_rooms.contains(room_id) {
                stopped_room_ids.insert(room_id.clone());
            }
        }
        for checkpoint in &self.crawl_queue {
            if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
                stopped_room_ids.insert(checkpoint.room_id.clone());
            }
        }
        self.completed_rooms
            .retain(|room_id, _| self.available_crawl_rooms.contains(room_id));
        self.crawl_queue.retain(|checkpoint| {
            checkpoint.manual || self.available_crawl_rooms.contains(&checkpoint.room_id)
        });
        self.queued_crawl_rooms = self
            .crawl_queue
            .iter()
            .map(|checkpoint| checkpoint.room_id.clone())
            .collect();

        stopped_room_ids.into_iter().collect()
    }

    async fn abort_active_history_crawl_if_retired(&mut self) -> Option<String> {
        let retired_room_id = self
            .active_crawl_checkpoint
            .as_ref()
            .filter(|checkpoint| {
                !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id)
            })
            .map(|checkpoint| checkpoint.room_id.clone());
        if retired_room_id.is_some() {
            if let Some(handle) = self.active_crawl_page.take() {
                abort_and_await_task(handle).await;
            }
            self.active_crawl_checkpoint = None;
        }
        retired_room_id
    }

    async fn remove_history_crawl_room(&mut self, room_id: &str) {
        self.crawl_queue
            .retain(|checkpoint| checkpoint.room_id != room_id);
        self.queued_crawl_rooms.remove(room_id);
        self.completed_rooms.remove(room_id);
        let active_matches = self
            .active_crawl_checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.room_id == room_id);
        if active_matches {
            if let Some(handle) = self.active_crawl_page.take() {
                abort_and_await_task(handle).await;
            }
            self.active_crawl_checkpoint = None;
        }
    }

    async fn stop_all_history_crawls(&mut self) {
        let mut stopped_room_ids = std::collections::BTreeSet::new();
        stopped_room_ids.extend(self.queued_crawl_rooms.iter().cloned());
        if let Some(checkpoint) = &self.active_crawl_checkpoint {
            stopped_room_ids.insert(checkpoint.room_id.clone());
        }
        self.crawl_queue.clear();
        self.queued_crawl_rooms.clear();
        if let Some(handle) = self.active_crawl_page.take() {
            abort_and_await_task(handle).await;
        }
        self.active_crawl_checkpoint = None;
        if let Some(timer) = self.crawl_delay_timer.take() {
            abort_and_await_task(timer).await;
        }
        for room_id in stopped_room_ids {
            self.emit_history_crawl_stopped(room_id).await;
        }
    }

    async fn invalidate_history_crawler_cache(&mut self) {
        self.completed_rooms.clear();
        // Attachment rows were built under the old content policy, and the
        // Files view must rebuild them under the new one.
        self.document_store.clear();
        self.attachment_retries.clear();
        self.queued_crawl_index.clear();
        // A settings change that could
        // not be saved is still detected on the next start.
        self.stop_all_history_crawls().await;
    }

    async fn rebuild_search_index(&mut self) {
        self.document_store.clear();
        self.crawl_settings_generation = self.crawl_settings_generation.wrapping_add(1);
        self.invalidate_history_crawler_cache().await;
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }
}

impl Drop for SearchActor {
    fn drop(&mut self) {
        if let Some(task) = &self.active_sdk_search {
            task.abort();
        }
        if let Some(task) = &self.active_crawl_page {
            task.abort();
        }
        if let Some(task) = &self.crawl_delay_timer {
            task.abort();
        }
    }
}

async fn abort_and_await_task<T>(task: executor::JoinHandle<T>) {
    task.abort();
    let _ = task.await;
}

pub(crate) fn compact_search_results(
    results: &[koushi_state::SearchResult],
) -> Vec<SearchResultItem> {
    results
        .iter()
        .map(|result| SearchResultItem {
            room_id: result.room_id.clone(),
            event_id: result.event_id.clone(),
            snippet: result.snippet.clone(),
        })
        .collect()
}

fn record_search_finish(
    request_id: RequestId,
    stage: &'static str,
    projected_results: &[koushi_state::SearchResult],
    duration_ms: u128,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", stage)
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::count(
                "results",
                projected_results.len() as u64,
            ))
            .field(DiagnosticField::count(
                "result_rooms",
                projected_results
                    .iter()
                    .map(|result| result.room_id.as_str())
                    .collect::<HashSet<_>>()
                    .len() as u64,
            ))
            .field(DiagnosticField::milliseconds("duration", duration_ms)),
    );
}

fn record_stale_sdk_drop(
    request_id: RequestId,
    generation: u64,
    active_generation: u64,
    reason: &'static str,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "stale_sdk_drop")
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::count("generation", generation))
            .field(DiagnosticField::count(
                "active_generation",
                active_generation,
            ))
            .field(DiagnosticField::token("reason", reason)),
    );
}

fn record_search_sdk_failure(request_id: RequestId, kind: SearchFailureKind, duration_ms: u128) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "sdk_failed")
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::token(
                "kind",
                search_failure_trace_label(kind),
            ))
            .field(DiagnosticField::milliseconds("duration", duration_ms)),
    );
}

fn search_failure_trace_label(kind: SearchFailureKind) -> &'static str {
    match kind {
        SearchFailureKind::IndexUnavailable => "index_unavailable",
        SearchFailureKind::Query => "query",
        SearchFailureKind::Internal => "internal",
    }
}

fn matrix_sdk_search_scope(
    scope: &SearchScope,
    room_filter: &SearchRoomFilter,
) -> koushi_sdk::MatrixSearchScope {
    match scope {
        SearchScope::CurrentRoom { room_id } => koushi_sdk::MatrixSearchScope::CurrentRoom {
            room_id: room_id.clone(),
        },
        SearchScope::CurrentSpace { .. } => match room_filter {
            SearchRoomFilter::OnlyRooms(room_ids) => koushi_sdk::MatrixSearchScope::RoomSet {
                room_ids: room_ids.clone(),
            },
            SearchRoomFilter::AllRooms => koushi_sdk::MatrixSearchScope::AllRooms,
        },
        SearchScope::AllRooms => koushi_sdk::MatrixSearchScope::AllRooms,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "actor wiring: independent owned handles moved into one task"
)]
async fn run_sdk_query(
    session: Arc<MatrixClientSession>,
    generation: u64,
    request_id: RequestId,
    query: String,
    scope: SearchScope,
    room_filter: SearchRoomFilter,
    sdk_scope: koushi_sdk::MatrixSearchScope,
    settings: SearchCrawlerSettings,
    content_policy_generation: u64,
    variants: Vec<String>,
) -> SearchSdkQueryResult {
    let sdk_started = Instant::now();
    let mut verification = IndexCandidateVerification::default();
    let mut verified_by_identity: HashMap<(String, String), VerifiedCandidate> = HashMap::new();

    for (variant_index, query_variant) in variants.iter().enumerate() {
        let variant_started = Instant::now();
        let outcome = match verify_literal_candidates(
            &session,
            query_variant,
            &sdk_scope,
            &room_filter,
            &settings,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(kind) => {
                return SearchSdkQueryResult {
                    generation,
                    request_id,
                    query,
                    scope,
                    room_filter,
                    content_policy_generation,
                    projection: Err(kind),
                    sdk_total_ms: sdk_started.elapsed().as_millis(),
                };
            }
        };
        let elapsed_ms = variant_started.elapsed().as_millis();
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "sdk_variant")
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::count("variant", variant_index as u64))
                .field(DiagnosticField::boolean(
                    "raw_variant",
                    query_variant == &query,
                ))
                .field(DiagnosticField::count(
                    "candidates",
                    outcome.in_scope as u64,
                ))
                .field(DiagnosticField::count("verified", outcome.verified as u64))
                .field(DiagnosticField::milliseconds("duration", elapsed_ms)),
        );

        verification.in_scope += outcome.in_scope;
        verification.resolved += outcome.resolved;
        verification.verified += outcome.verified;
        verification.rooms.extend(outcome.rooms);
        for candidate in outcome.results {
            let identity = (
                candidate.result.room_id.clone(),
                candidate.result.event_id.clone(),
            );
            match verified_by_identity.entry(identity) {
                // One message can match more than one query variant; keep the
                // newer index position of the two.
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if entry.get().index_key < candidate.index_key {
                        entry.insert(candidate);
                    }
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(candidate);
                }
            }
        }
    }

    let total_ms = sdk_started.elapsed().as_millis();
    SearchSdkQueryResult {
        generation,
        request_id,
        query,
        scope,
        room_filter,
        content_policy_generation,
        projection: Ok(SearchProjection {
            results: select_newest(verified_by_identity.into_values().collect()),
            verification,
            project_ms: total_ms,
        }),
        sdk_total_ms: total_ms,
    }
}

/// Cap verified candidates at the result limit and order them for display.
///
/// The cap must be applied in the order the candidate scan used -- the index's
/// `(timestamp, event_id)` key, descending -- because that is the only order in
/// which "newest" is knowable: re-ordering by the displayed (resolved) timestamp
/// first can drop a result the scan did see as newer. Callers deduplicate by
/// resolved identity before this point.
fn select_newest(mut candidates: Vec<VerifiedCandidate>) -> Vec<koushi_state::SearchResult> {
    candidates.sort_by(|left, right| {
        right
            .index_key
            .cmp(&left.index_key)
            .then_with(|| right.result.event_id.cmp(&left.result.event_id))
    });
    candidates.truncate(SEARCH_CANDIDATE_LIMIT);
    candidates.sort_by(|left, right| {
        right
            .result
            .timestamp_ms
            .cmp(&left.result.timestamp_ms)
            .then_with(|| right.index_key.cmp(&left.index_key))
    });
    candidates
        .into_iter()
        .map(|candidate| candidate.result)
        .collect()
}

/// Verify one query variant, paging the index until enough candidates match or
/// the scan budget is spent.
///
/// Verification filters candidates, so a single page can under-report when most
/// of it fails to match; refilling keeps the answer complete while the candidate
/// scan stays bounded and no offset is ever used.
async fn verify_literal_candidates(
    session: &Arc<MatrixClientSession>,
    query: &str,
    sdk_scope: &koushi_sdk::MatrixSearchScope,
    room_filter: &SearchRoomFilter,
    settings: &SearchCrawlerSettings,
) -> Result<IndexCandidateVerification, SearchFailureKind> {
    let mut pager =
        koushi_sdk::MatrixLiteralSearchPager::new(session, query, sdk_scope, SEARCH_CANDIDATE_PAGE);
    let mut verification = IndexCandidateVerification::default();
    let mut seen_identities: HashSet<(String, String)> = HashSet::new();

    while verification.results.len() < SEARCH_CANDIDATE_LIMIT
        && verification.in_scope < SEARCH_CANDIDATE_SCAN_BUDGET
    {
        let page = pager
            .next_page(session, SEARCH_CANDIDATE_PAGE)
            .await
            .map_err(|error| classify_matrix_search_error(&error))?;
        if page.is_empty() {
            break;
        }

        for candidate in page {
            if !room_filter.contains(&candidate.room_id) {
                continue;
            }
            verification.in_scope += 1;
            verification.rooms.insert(candidate.room_id.clone());
            // A missing or redacted cached event simply drops out of the page.
            let Ok(Some(resolved)) = koushi_sdk::resolve_cached_message(
                session,
                &candidate.room_id,
                &candidate.event_id,
            )
            .await
            else {
                continue;
            };
            verification.resolved += 1;

            // The index may answer with an edit event id; the resolved reader
            // reports the original identity plus current content. The content
            // policy is applied here: the index also gets events from sync, so
            // this is the only place an opted-out caption or filename can be
            // kept out of results.
            let Some((body, attachment_filename)) = visible_content(
                settings,
                resolved.body.as_deref(),
                resolved.attachment_filename.as_deref(),
            ) else {
                continue;
            };
            let event = SearchableEvent {
                room_id: candidate.room_id.clone(),
                event_id: resolved.event_id.clone(),
                sender: resolved.sender.clone(),
                timestamp_ms: resolved.timestamp_ms.unwrap_or(0),
                body: body.map(SensitiveString::new),
                attachment_filename: attachment_filename.map(SensitiveString::new),
                attachment: None,
            };
            let resolved_candidate = SearchCandidate {
                room_id: candidate.room_id.clone(),
                event_id: resolved.event_id,
                score_millis: 0,
            };
            if let Some(result) =
                koushi_search::verify_candidate(&resolved_candidate, &event, query)
            {
                // An edit event id and its root can both come back from the
                // index. Deduplicate by the resolved identity before the result
                // counts toward the quota, so a duplicate cannot consume a slot
                // that a distinct message needs.
                if !seen_identities.insert((
                    candidate.room_id.clone(),
                    resolved_candidate.event_id.clone(),
                )) {
                    continue;
                }
                verification.verified += 1;
                verification.results.push(VerifiedCandidate {
                    index_key: IndexOrderKey {
                        timestamp_millis: candidate.timestamp_millis,
                        event_id: candidate.event_id,
                    },
                    result,
                });
            }
        }
    }

    Ok(verification)
}

/// Whether a settings change alters what the verifier may match.
///
/// A crawler speed change is not a content-policy change and must not invalidate
/// an in-flight query.
fn content_policy_changed(previous: &SearchCrawlerSettings, next: &SearchCrawlerSettings) -> bool {
    previous.include_media_captions != next.include_media_captions
        || previous.include_filenames != next.include_filenames
}

/// Content settings that expose nothing until the account's own arrive.
///
/// Media captions and filenames are opt-in content for search; before the first
/// `RoomsAvailable` notification carries the account's settings, a query must
/// miss them rather than reveal them.
fn restricted_crawler_settings() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        include_media_captions: false,
        include_filenames: false,
        ..SearchCrawlerSettings::default()
    }
}

/// Project a cache-resolved message onto the content the search policy allows.
///
/// `attachment_filename.is_some()` marks a media message: the SDK resolver fills
/// it for image/video/audio/file and never for text-like messages
/// (`resolved_text` in the fork's `search_index`). This mirrors the crawler's
/// own projection (`search_crawler_project_message_content`) so a query can
/// never match text the account opted out of indexing. Returns `None` when the
/// policy leaves nothing to match.
fn visible_content(
    settings: &SearchCrawlerSettings,
    body: Option<&str>,
    attachment_filename: Option<&str>,
) -> Option<(Option<String>, Option<String>)> {
    let Some(filename) = attachment_filename else {
        // A text-like message keeps its body; the policy governs media only.
        return body.map(|body| (Some(body.to_owned()), None));
    };
    let caption = settings
        .include_media_captions
        .then(|| body.map(str::to_owned))
        .flatten();
    let filename = settings.include_filenames.then(|| filename.to_owned());
    (caption.is_some() || filename.is_some()).then_some((caption, filename))
}

fn classify_matrix_search_error(error: &koushi_sdk::MatrixSearchError) -> SearchFailureKind {
    match error {
        koushi_sdk::MatrixSearchError::IndexUnavailable => SearchFailureKind::IndexUnavailable,
        koushi_sdk::MatrixSearchError::Query => SearchFailureKind::Query,
        koushi_sdk::MatrixSearchError::Internal => SearchFailureKind::Internal,
    }
}

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
