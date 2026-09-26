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
//! The `SearchDocumentStore` (from `koushi-search`) is our in-process
//! verification layer: it mirrors the visible canonical text for every indexed
//! event. Timeline diffs arrive via an `mpsc` channel (`SearchIndexMessage`)
//! forwarded from the `TimelineManagerActor`/`TimelineActor`.
//!
//! ## Query pipeline (overview.md Security Model — Search)
//! `SearchCommand::Query` → SDK `client.search_messages()` → candidate list
//! → verify each against `SearchDocumentStore::verify_candidate()` → emit
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
//! - **Upsert**: a new or updated visible message is indexed into the document
//!   store. The SDK ngram index is fed by sync automatically.
//! - **Edit**: `SearchDocumentStore::upsert_edit` updates only the affected
//!   document. Old terms are no longer verified against the canonical text, so
//!   they drop out of results naturally.
//! - **Redact**: `SearchDocumentStore::redact` removes the document; candidates
//!   for that event will no longer verify.
//! - **Unresolved replacement** (edit before original): stored as a pending edit
//!   in `SearchDocumentStore`; not indexed as a standalone message (canon).
//!
//! ## Debug redaction
//! Search queries and snippets must not appear in Debug of internal messages
//! (they can appear in `SearchEvent::Results` payloads — those are visible UI
//! state). `SearchActorMessage::Query` redacts the query in Debug.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_search::{
    AttachmentDocument, SearchCandidate, SearchDocumentStore, SearchEdit, SearchRoomFilter,
    SearchableEvent, SensitiveString, cjk_search_query_variants,
};
use koushi_state::{
    AppAction, AttachmentFilter, AttachmentScope, AttachmentSort, SearchCrawlerSettings,
    SearchCrawlerSpeed,
};
use tokio::sync::{broadcast, mpsc};

use crate::account_work::AccountWorkScheduler;
use crate::command_policy::search_scope_to_state;
use crate::executor;
use crate::search_crawler::{HistoryCrawlCheckpoint, HistoryCrawlPageResult};
use koushi_protocol::command::{SearchCommand, SearchScope};
use koushi_protocol::event::{CoreEvent, SearchEvent, SearchResultItem};
use koushi_protocol::failure::SearchFailureKind;
use koushi_protocol::ids::RequestId;

/// Maximum number of candidates requested from the SDK ngram index.
/// Verification filters this down; the final result set may be smaller.
const SEARCH_CANDIDATE_LIMIT: usize = 50;
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
    sdk_unique: usize,
    sdk_rooms: usize,
    store_docs: usize,
    sdk_total_ms: u128,
    project_ms: u128,
    stats: &koushi_search::SearchWithCandidatesStats,
) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "verify")
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ))
        .field(DiagnosticField::count("sdk_unique", sdk_unique as u64))
        .field(DiagnosticField::count("sdk_rooms", sdk_rooms as u64))
        .field(DiagnosticField::count(
            "sdk_in_scope",
            stats.sdk_candidates_in_scope as u64,
        ))
        .field(DiagnosticField::count(
            "verified_sdk",
            stats.verified_sdk_count as u64,
        ))
        .field(DiagnosticField::count("store_docs", store_docs as u64))
        .field(DiagnosticField::count(
            "scan_visited",
            stats.scan.documents_visited as u64,
        ))
        .field(DiagnosticField::count(
            "scan_in_scope",
            stats.scan.documents_in_scope as u64,
        ))
        .field(DiagnosticField::count(
            "scan_matches",
            stats.scan.matches_before_limit as u64,
        ))
        .field(DiagnosticField::count(
            "scan_returned",
            stats.scan.returned as u64,
        ))
        .field(DiagnosticField::milliseconds("sdk_total_ms", sdk_total_ms))
        .field(DiagnosticField::milliseconds("project_ms", project_ms))
        .field(DiagnosticField::milliseconds(
            "scan_ms",
            stats.scan_elapsed_ms,
        ))
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
    },
    /// A message was edited. Update the document store.
    Edit {
        edit_event_id: String,
        target_event_id: String,
        sender: String,
        timestamp_ms: u64,
        body: Option<String>,
        attachment_filename: Option<String>,
        attachment: Option<AttachmentDocument>,
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
    room_filter: SearchRoomFilter,
    candidates: Result<Vec<SearchCandidate>, SearchFailureKind>,
    sdk_total_ms: u128,
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
    /// event_id -> room_id for indexed documents. Lets `IndexUpdated` carry the
    /// room id for edits, whose `SearchIndexMessage::Edit` payload only names
    /// the target event id. These are app-owned visible-state identifiers
    /// (never bodies), so retaining them here does not leak secrets.
    indexed_rooms: HashMap<String, String>,
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
}

impl SearchActor {
    /// Spawn the actor and return its handle.
    pub fn spawn(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        account_work: AccountWorkScheduler,
    ) -> SearchActorHandle {
        let (tx, msg_rx) = mpsc::channel(64);
        let (index_tx, index_rx) = mpsc::channel(SEARCH_INDEX_MUTATION_QUEUE);

        let actor = SearchActor {
            session,
            document_store: SearchDocumentStore::default(),
            indexed_rooms: HashMap::new(),
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
        };

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
                index_msg = index_rx.recv() => {
                    let Some(index_msg) = index_msg else {
                        // Timeline sender dropped — that's fine (e.g. on shutdown).
                        continue;
                    };
                    self.handle_index(index_msg);
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
                enqueued_at,
            } => {
                self.drain_available_actor_messages();
                let (latest_query, dropped_queries) = coalesce_contiguous_pending_queries(
                    SearchActorMessage::Query {
                        request_id,
                        query,
                        scope,
                        room_filter,
                        enqueued_at,
                    },
                    &mut self.deferred_messages,
                );
                if let SearchActorMessage::Query {
                    request_id,
                    query,
                    scope,
                    room_filter,
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
                    self.handle_query(request_id, &query, scope, room_filter, enqueued_at)
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
        enqueued_at: Instant,
    ) {
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
            self.emit_search_succeeded(request_id, query, &scope, Vec::new())
                .await;
            self.emit(CoreEvent::Search(SearchEvent::Results {
                request_id,
                results: Vec::new(),
            }));
            return;
        }

        let query_started = Instant::now();
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

        let projected_results =
            self.project_search_results(request_id, query, &room_filter, &[], 0);
        record_search_finish(
            request_id,
            "local_finish",
            &projected_results,
            query_started.elapsed().as_millis(),
        );
        let compact_results = compact_search_results(&projected_results);
        self.emit_search_succeeded(request_id, query, &scope, projected_results)
            .await;
        self.emit(CoreEvent::Search(SearchEvent::Results {
            request_id,
            results: compact_results,
        }));

        if matches!(&room_filter, SearchRoomFilter::OnlyRooms(room_ids) if room_ids.is_empty()) {
            return;
        }

        let session = self.session.clone();
        let query = query.to_owned();
        let sdk_scope = matrix_sdk_search_scope(&scope, &room_filter);
        self.active_sdk_search = Some(executor::spawn(run_sdk_query(
            session,
            generation,
            request_id,
            query,
            scope,
            room_filter,
            sdk_scope,
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

        let sdk_candidates = match result.candidates {
            Ok(candidates) => candidates,
            Err(kind) => {
                record_search_sdk_failure(result.request_id, kind, result.sdk_total_ms);
                return;
            }
        };
        let projection_started = Instant::now();
        let projected_results = self.project_search_results(
            result.request_id,
            &result.query,
            &result.room_filter,
            &sdk_candidates,
            result.sdk_total_ms,
        );
        record_search_finish(
            result.request_id,
            "finish",
            &projected_results,
            projection_started.elapsed().as_millis() + result.sdk_total_ms,
        );
        let compact_results = compact_search_results(&projected_results);
        self.emit_search_succeeded(
            result.request_id,
            &result.query,
            &result.scope,
            projected_results,
        )
        .await;
        self.emit(CoreEvent::Search(SearchEvent::Results {
            request_id: result.request_id,
            results: compact_results,
        }));
    }

    fn project_search_results(
        &self,
        request_id: RequestId,
        query: &str,
        room_filter: &SearchRoomFilter,
        sdk_candidates: &[SearchCandidate],
        sdk_total_ms: u128,
    ) -> Vec<koushi_state::SearchResult> {
        let sdk_room_count = {
            sdk_candidates
                .iter()
                .map(|candidate| candidate.room_id.as_str())
                .collect::<HashSet<_>>()
                .len()
        };

        // #162/#341: the SDK ngram index is an accelerator, not the authority.
        // The direct document-store scan runs first and with the same
        // Rust-resolved scope filter, so indexed local results are visible even
        // while an SDK supplement is still pending.
        let projection_started = Instant::now();
        let projection = self.document_store.search_with_candidates_with_stats(
            query,
            room_filter,
            sdk_candidates,
            SEARCH_CANDIDATE_LIMIT,
        );
        let projection_elapsed_ms = projection_started.elapsed().as_millis();
        record(search_verify_diagnostic_event(
            request_id,
            sdk_candidates.len(),
            sdk_room_count,
            self.document_store.document_count(),
            sdk_total_ms,
            projection_elapsed_ms,
            &projection.stats,
        ));
        projection.results
    }

    async fn handle_attachments(
        &self,
        request_id: RequestId,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    ) {
        let results = self
            .document_store
            .attachments(&scope, &filter, sort.clone());

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
                query: query.to_owned(),
                scope: search_scope_to_state(scope),
                results,
            }])
            .await;
    }

    fn handle_index(&mut self, msg: SearchIndexMessage) {
        match msg {
            SearchIndexMessage::Upsert {
                room_id,
                event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
            } => {
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
                self.document_store.upsert_message(event);
                self.indexed_rooms
                    .insert(indexed_event_id.clone(), indexed_room_id.clone());
                self.emit(CoreEvent::Search(SearchEvent::IndexUpdated {
                    room_id: indexed_room_id,
                    event_id: indexed_event_id,
                }));
            }
            SearchIndexMessage::Edit {
                edit_event_id,
                target_event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
            } => {
                // The Edit payload only names the target event id; resolve its
                // room id from the indexed-document map so `IndexUpdated` stays
                // honest (no fabricated room id). An edit whose original is not
                // yet indexed is stored as a pending edit and emits no event.
                let edited_room_id = self.indexed_rooms.get(&target_event_id).cloned();
                let edited_event_id = target_event_id.clone();
                let edit = SearchEdit {
                    edit_event_id,
                    target_event_id,
                    sender,
                    timestamp_ms,
                    body: body.map(SensitiveString::new),
                    attachment_filename: attachment_filename.map(SensitiveString::new),
                    attachment,
                };
                self.document_store.upsert_edit(edit);
                if let Some(room_id) = edited_room_id {
                    self.emit(CoreEvent::Search(SearchEvent::IndexUpdated {
                        room_id,
                        event_id: edited_event_id,
                    }));
                }
            }
            SearchIndexMessage::Redact { event_id } => {
                self.indexed_rooms.remove(&event_id);
                self.document_store.redact(&event_id);
            }
        }
    }

    async fn handle_start_history_crawl(
        &mut self,
        request_id: RequestId,
        room_id: String,
        settings: SearchCrawlerSettings,
    ) {
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

        if settings.speed == SearchCrawlerSpeed::Paused {
            self.stop_all_history_crawls().await;
            return;
        }

        let mut stopped_room_ids = self.retain_history_crawl_rooms();
        if let Some(room_id) = self.abort_active_history_crawl_if_retired().await {
            stopped_room_ids.push(room_id);
        }
        for room_id in stopped_room_ids {
            self.emit_history_crawl_stopped(room_id).await;
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
            || self.indexed_rooms.contains_key(latest)
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
        if self.active_crawl_page.is_some() {
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
            } => {
                if checkpoint.settings_generation != self.crawl_settings_generation {
                    return;
                }
                if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
                    return;
                }
                for message in messages {
                    self.handle_index(message);
                }
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
                    self.completed_rooms.insert(
                        checkpoint.room_id.clone(),
                        CompletedHistoryCrawl {
                            latest_event_id: checkpoint.latest_event_id_at_start.clone(),
                            processed: checkpoint.processed,
                            indexed: checkpoint.indexed,
                        },
                    );
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

    fn retain_history_crawl_rooms(&mut self) -> Vec<String> {
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
        self.stop_all_history_crawls().await;
    }

    async fn rebuild_search_index(&mut self) {
        self.document_store.clear();
        self.indexed_rooms.clear();
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

fn compact_search_results(results: &[koushi_state::SearchResult]) -> Vec<SearchResultItem> {
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

async fn run_sdk_query(
    session: Arc<MatrixClientSession>,
    generation: u64,
    request_id: RequestId,
    query: String,
    scope: SearchScope,
    room_filter: SearchRoomFilter,
    sdk_scope: koushi_sdk::MatrixSearchScope,
    variants: Vec<String>,
) -> SearchSdkQueryResult {
    let sdk_started = Instant::now();
    let mut candidates_by_key: HashMap<(String, String), koushi_sdk::MatrixSearchCandidate> =
        HashMap::new();
    for (variant_index, query_variant) in variants.iter().enumerate() {
        let variant_started = Instant::now();
        let candidates = koushi_sdk::search_message_candidates_scoped(
            &session,
            query_variant,
            sdk_scope.clone(),
            SEARCH_CANDIDATE_LIMIT,
        )
        .await;

        let candidates = match candidates {
            Ok(candidates) => candidates,
            Err(error) => {
                return SearchSdkQueryResult {
                    generation,
                    request_id,
                    query,
                    scope,
                    room_filter,
                    candidates: Err(classify_matrix_search_error(&error)),
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
                    candidates.len() as u64,
                ))
                .field(DiagnosticField::milliseconds("duration", elapsed_ms)),
        );

        for candidate in candidates {
            let key = (candidate.room_id.clone(), candidate.event_id.clone());
            candidates_by_key
                .entry(key)
                .and_modify(|current| {
                    if candidate.score_millis > current.score_millis {
                        *current = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
    }

    SearchSdkQueryResult {
        generation,
        request_id,
        query,
        scope,
        room_filter,
        candidates: Ok(candidates_by_key
            .into_values()
            .map(|candidate| SearchCandidate {
                room_id: candidate.room_id,
                event_id: candidate.event_id,
                score_millis: candidate.score_millis,
            })
            .collect()),
        sdk_total_ms: sdk_started.elapsed().as_millis(),
    }
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
