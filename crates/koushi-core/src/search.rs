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
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_search::{
    AttachmentDocument, SearchCandidate, SearchDocumentStore, SearchEdit, SearchableEvent,
    SensitiveString, cjk_search_query_variants,
};
use koushi_state::{
    AppAction, AttachmentFilter, AttachmentScope, AttachmentSort, SearchCrawlerSettings,
    SearchCrawlerSpeed,
};
use tokio::sync::{broadcast, mpsc};

use crate::command::{SearchCommand, SearchScope};
use crate::event::{CoreEvent, SearchEvent, SearchResultItem};
use crate::executor;
use crate::failure::{CoreFailure, SearchFailureKind};
use crate::ids::RequestId;
use crate::messages_backpressure::MessagesBackpressure;
use crate::search_crawler::{HistoryCrawlCheckpoint, HistoryCrawlPageResult};

/// Maximum number of candidates requested from the SDK ngram index.
/// Verification filters this down; the final result set may be smaller.
const SEARCH_CANDIDATE_LIMIT: usize = 50;
const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";
const ENV_SEARCH_TRACE: &str = "KOUSHI_SEARCH_TRACE";

/// Search index mutation queue capacity (canon, overview.md: 512).
pub const SEARCH_INDEX_MUTATION_QUEUE: usize = 512;

/// Automatic history crawling is held off for this long after the crawler first
/// has work, so it does not contend with user-visible pagination during the
/// startup window. Crawler timing is Rust-owned (not a user setting). The
/// maintainer confirmed a ~1 minute delay is fully acceptable (#123).
const CRAWLER_STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

fn state_search_scope(scope: &SearchScope) -> koushi_state::SearchScope {
    match scope {
        SearchScope::AllRooms => koushi_state::SearchScope::AllRooms,
        SearchScope::CurrentRoom { room_id } => koushi_state::SearchScope::CurrentRoom {
            room_id: room_id.clone(),
        },
        SearchScope::CurrentSpace { space_id } => koushi_state::SearchScope::CurrentSpace {
            space_id: space_id.clone(),
        },
        SearchScope::Dms => koushi_state::SearchScope::Dms,
    }
}

fn search_trace_enabled() -> bool {
    std::env::var_os(ENV_SEARCH_TRACE).is_some()
}

fn search_scope_trace_label(scope: &SearchScope) -> &'static str {
    match scope {
        SearchScope::AllRooms => "all_rooms",
        SearchScope::CurrentRoom { .. } => "current_room",
        SearchScope::CurrentSpace { .. } => "current_space",
        SearchScope::Dms => "dms",
    }
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
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

/// Messages routed to the `SearchActor`.
pub(crate) enum SearchActorMessage {
    /// A `SearchCommand::Query` from the command boundary.
    Query {
        request_id: RequestId,
        query: String,
        scope: SearchScope,
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
    RoomsAvailable {
        room_ids: Vec<String>,
        settings: SearchCrawlerSettings,
    },
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

// Redact query text in Debug (queries may contain message content).
impl std::fmt::Debug for SearchActorMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Query {
                request_id, scope, ..
            } => f
                .debug_struct("SearchActorMessage::Query")
                .field("request_id", request_id)
                .field("query", &"SearchQuery(..)")
                .field("scope", scope)
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
            Self::RoomsAvailable { room_ids, settings } => f
                .debug_struct("SearchActorMessage::RoomsAvailable")
                .field("room_count", &room_ids.len())
                .field("settings", settings)
                .finish(),
            Self::InvalidateCrawlerCache => {
                write!(f, "SearchActorMessage::InvalidateCrawlerCache")
            }
            Self::RebuildIndex => write!(f, "SearchActorMessage::RebuildIndex"),
            Self::Shutdown => write!(f, "SearchActorMessage::Shutdown"),
        }
    }
}

/// Handle to the `SearchActor` background task.
pub struct SearchActorHandle {
    tx: mpsc::Sender<SearchActorMessage>,
    /// Channel for forwarding timeline mutations (SearchIndexMessage).
    /// Cloned and handed to TimelineManagerActor on creation.
    index_tx: mpsc::Sender<SearchIndexMessage>,
}

impl SearchActorHandle {
    pub async fn send_command(&self, command: SearchCommand) -> bool {
        let msg = match command {
            SearchCommand::Query {
                request_id,
                query,
                scope,
            } => SearchActorMessage::Query {
                request_id,
                query,
                scope,
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

    pub async fn shutdown(&self) {
        let _ = self.tx.send(SearchActorMessage::Shutdown).await;
    }

    /// Try to notify the actor that the set of joined rooms has changed.
    /// Room availability is latest-wins background state; callers that get a
    /// full inbox keep the returned payload and retry later instead of blocking
    /// user-visible commands on crawler work.
    pub fn try_notify_rooms_available(
        &self,
        room_ids: Vec<String>,
        settings: SearchCrawlerSettings,
    ) -> Result<(), (Vec<String>, SearchCrawlerSettings)> {
        match self
            .tx
            .try_send(SearchActorMessage::RoomsAvailable { room_ids, settings })
        {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(
                SearchActorMessage::RoomsAvailable { room_ids, settings },
            )) => Err((room_ids, settings)),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(
                SearchActorMessage::RoomsAvailable { room_ids, settings },
            )) => Err((room_ids, settings)),
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
    messages_backpressure: MessagesBackpressure,
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
    /// Room ids whose history has been fully crawled at least once. Used by
    /// `handle_rooms_available` to skip idempotent auto-start for completed
    /// rooms.
    completed_rooms: HashSet<String>,
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
        messages_backpressure: MessagesBackpressure,
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
            messages_backpressure,
            crawl_queue: VecDeque::new(),
            queued_crawl_rooms: HashSet::new(),
            available_crawl_rooms: HashSet::new(),
            active_crawl_page: None,
            active_crawl_checkpoint: None,
            completed_rooms: HashSet::new(),
            crawl_settings_generation: 0,
            crawl_delay_elapsed: false,
            crawl_delay_timer: None,
        };

        // Spawn the actor task.
        executor::spawn(actor.run(index_rx));

        SearchActorHandle { tx, index_tx }
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
                msg = self.msg_rx.recv() => {
                    let Some(msg) = msg else { break };
                    if !self.handle_actor_message(msg).await {
                        break;
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
                self.stop_all_history_crawls().await;
                false
            }
            SearchActorMessage::Query {
                request_id,
                query,
                scope,
                enqueued_at,
            } => {
                self.drain_available_actor_messages();
                let (latest_query, dropped_queries) = coalesce_contiguous_pending_queries(
                    SearchActorMessage::Query {
                        request_id,
                        query,
                        scope,
                        enqueued_at,
                    },
                    &mut self.deferred_messages,
                );
                if let SearchActorMessage::Query {
                    request_id,
                    query,
                    scope,
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
                    if search_trace_enabled() && dropped_queries > 0 {
                        eprintln!(
                            "koushi.search stage=coalesce request={} dropped_queries={} deferred_messages={}",
                            request_id.sequence,
                            dropped_queries,
                            self.deferred_messages.len()
                        );
                    }
                    self.handle_query(request_id, &query, scope, enqueued_at)
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
            SearchActorMessage::RoomsAvailable { room_ids, settings } => {
                self.handle_rooms_available(room_ids, settings).await;
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
        &self,
        request_id: RequestId,
        query: &str,
        scope: SearchScope,
        enqueued_at: Instant,
    ) {
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

        let trace = search_trace_enabled();
        let query_started = Instant::now();
        let queued_ms = enqueued_at.elapsed().as_millis();
        let variants = cjk_search_query_variants(query);
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "start")
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token(
                    "scope",
                    search_scope_trace_label(&scope),
                ))
                .field(DiagnosticField::milliseconds("queued", queued_ms))
                .field(DiagnosticField::count("variants", variants.len() as u64))
                .field(DiagnosticField::boolean(
                    "normalized_diff",
                    variants.iter().any(|variant| variant != query),
                )),
        );
        if trace {
            eprintln!(
                "koushi.search stage=start request={} scope={} queued_ms={} query_bytes={} query_chars={} variants={} normalized_diff={}",
                request_id.sequence,
                search_scope_trace_label(&scope),
                queued_ms,
                query.len(),
                query.chars().count(),
                variants.len(),
                variants.iter().any(|variant| variant != query)
            );
        }

        let sdk_started = Instant::now();
        let mut candidates_by_key: HashMap<(String, String), koushi_sdk::MatrixSearchCandidate> =
            HashMap::new();
        for (variant_index, query_variant) in variants.iter().enumerate() {
            let variant_started = Instant::now();
            let candidates = koushi_sdk::search_message_candidates(
                &self.session,
                query_variant,
                SEARCH_CANDIDATE_LIMIT,
            )
            .await;

            let candidates = match candidates {
                Ok(c) => c,
                Err(error) => {
                    let kind = classify_matrix_search_error(&error);
                    self.emit_search_failed(
                        request_id,
                        query,
                        &scope,
                        search_failure_message(kind),
                    )
                    .await;
                    self.emit_failure(request_id, CoreFailure::SearchFailed { kind });
                    return;
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
                        query_variant == query,
                    ))
                    .field(DiagnosticField::count(
                        "candidates",
                        candidates.len() as u64,
                    ))
                    .field(DiagnosticField::milliseconds("duration", elapsed_ms)),
            );
            if trace {
                eprintln!(
                    "koushi.search stage=sdk_variant request={} variant={} raw={} candidates={} elapsed_ms={}",
                    request_id.sequence,
                    variant_index,
                    query_variant == query,
                    candidates.len(),
                    elapsed_ms
                );
            }

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
        let sdk_total_ms = sdk_started.elapsed().as_millis();

        let sdk_candidates = candidates_by_key
            .into_values()
            .map(|candidate| SearchCandidate {
                room_id: candidate.room_id,
                event_id: candidate.event_id,
                score_millis: candidate.score_millis,
            })
            .collect::<Vec<_>>();

        let room_filter = match &scope {
            SearchScope::CurrentRoom { room_id } => Some(room_id.as_str()),
            SearchScope::AllRooms | SearchScope::CurrentSpace { .. } | SearchScope::Dms => None,
        };
        let sdk_room_count = {
            sdk_candidates
                .iter()
                .map(|candidate| candidate.room_id.as_str())
                .collect::<HashSet<_>>()
                .len()
        };

        // #162: the SDK ngram index is an accelerator, not the authority. Union
        // the index candidates with a direct scan of the document store so any
        // message koushi has indexed (crawled history, live, CJK, short
        // queries) is found even when the sync-fed ngram index does not surface
        // it. Verification against canonical text still drops false positives.
        let projection_started = Instant::now();
        let projection = self.document_store.search_with_candidates_with_stats(
            query,
            room_filter,
            &sdk_candidates,
            SEARCH_CANDIDATE_LIMIT,
        );
        let projection_elapsed_ms = projection_started.elapsed().as_millis();
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "verify")
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::count(
                    "sdk_unique",
                    sdk_candidates.len() as u64,
                ))
                .field(DiagnosticField::count("sdk_rooms", sdk_room_count as u64))
                .field(DiagnosticField::count(
                    "sdk_in_scope",
                    projection.stats.sdk_candidates_in_scope as u64,
                ))
                .field(DiagnosticField::count(
                    "verified_sdk",
                    projection.stats.verified_sdk_count as u64,
                ))
                .field(DiagnosticField::count(
                    "store_docs",
                    self.document_store.document_count() as u64,
                ))
                .field(DiagnosticField::milliseconds(
                    "duration",
                    projection_elapsed_ms,
                )),
        );
        if trace {
            eprintln!(
                "koushi.search stage=verify request={} sdk_unique={} sdk_rooms={} sdk_in_scope={} verified_sdk={} store_docs={} scan_visited={} scan_in_scope={} scan_matches={} scan_returned={} sdk_total_ms={} project_ms={} scan_ms={}",
                request_id.sequence,
                sdk_candidates.len(),
                sdk_room_count,
                projection.stats.sdk_candidates_in_scope,
                projection.stats.verified_sdk_count,
                self.document_store.document_count(),
                projection.stats.scan.documents_visited,
                projection.stats.scan.documents_in_scope,
                projection.stats.scan.matches_before_limit,
                projection.stats.scan.returned,
                sdk_total_ms,
                projection_elapsed_ms,
                projection.stats.scan_elapsed_ms
            );
        }
        let projected_results = projection.results;
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "core.search", "finish")
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
                .field(DiagnosticField::milliseconds(
                    "duration",
                    query_started.elapsed().as_millis(),
                )),
        );
        let compact_results = projected_results
            .iter()
            .map(|result| SearchResultItem {
                room_id: result.room_id.clone(),
                event_id: result.event_id.clone(),
                snippet: result.snippet.clone(),
            })
            .collect();
        if trace {
            let result_room_count = projected_results
                .iter()
                .map(|result| result.room_id.as_str())
                .collect::<HashSet<_>>()
                .len();
            eprintln!(
                "koushi.search stage=finish request={} results={} result_rooms={} total_ms={}",
                request_id.sequence,
                projected_results.len(),
                result_room_count,
                query_started.elapsed().as_millis()
            );
        }

        self.emit_search_succeeded(request_id, query, &scope, projected_results)
            .await;
        self.emit(CoreEvent::Search(SearchEvent::Results {
            request_id,
            results: compact_results,
        }));
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
                scope: state_search_scope(scope),
                results,
            }])
            .await;
    }

    async fn emit_search_failed(
        &self,
        request_id: RequestId,
        query: &str,
        scope: &SearchScope,
        message: &str,
    ) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
                query: query.to_owned(),
                scope: state_search_scope(scope),
                message: message.to_owned(),
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
        self.remove_history_crawl_room(&room_id);
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
        self.remove_history_crawl_room(&room_id);
        self.emit_history_crawl_stopped(room_id).await;
    }

    /// Auto-start idempotent background crawls when the account observes a new
    /// set of joined rooms.
    ///
    /// This mirrors Element's Seshat crawler shape: maintain a checkpoint queue
    /// and process one `/messages` page at a time. If a page has a continuation
    /// token, push_back(next_checkpoint) so other rooms get a turn first.
    async fn handle_rooms_available(
        &mut self,
        room_ids: Vec<String>,
        settings: SearchCrawlerSettings,
    ) {
        self.available_crawl_rooms = room_ids.iter().cloned().collect();

        if settings.speed == SearchCrawlerSpeed::Paused {
            self.stop_all_history_crawls().await;
            return;
        }

        let mut stopped_room_ids = self.retain_history_crawl_rooms();
        if let Some(room_id) = self.abort_active_history_crawl_if_retired() {
            stopped_room_ids.push(room_id);
        }
        for room_id in stopped_room_ids {
            self.emit_history_crawl_stopped(room_id).await;
        }

        for room_id in room_ids {
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
                    timestamp_ms: current_epoch_ms(),
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

    fn history_crawl_room_is_known(&self, room_id: &str) -> bool {
        self.completed_rooms.contains(room_id)
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
        let Some(checkpoint) = self.crawl_queue.pop_front() else {
            return;
        };
        self.queued_crawl_rooms.remove(&checkpoint.room_id);
        if !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id) {
            self.start_next_history_crawl_page();
            return;
        }
        let handle = crate::search_crawler::spawn_history_crawl_page(
            self.session.clone(),
            self.messages_backpressure.clone(),
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
                        timestamp_ms: current_epoch_ms(),
                    }])
                    .await;
                if completed {
                    self.completed_rooms.insert(checkpoint.room_id.clone());
                    let _ = self
                        .action_tx
                        .send(vec![AppAction::HistoryCrawlCompleted {
                            room_id: checkpoint.room_id.clone(),
                            indexed: checkpoint.indexed,
                            timestamp_ms: current_epoch_ms(),
                        }])
                        .await;
                    self.emit(CoreEvent::Search(SearchEvent::HistoryCrawlCompleted {
                        room_id: checkpoint.room_id,
                        indexed: checkpoint.indexed,
                    }));
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
                        timestamp_ms: current_epoch_ms(),
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
        for room_id in &self.completed_rooms {
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
            .retain(|room_id| self.available_crawl_rooms.contains(room_id));
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

    fn abort_active_history_crawl_if_retired(&mut self) -> Option<String> {
        let retired_room_id = self
            .active_crawl_checkpoint
            .as_ref()
            .filter(|checkpoint| {
                !checkpoint.manual && !self.available_crawl_rooms.contains(&checkpoint.room_id)
            })
            .map(|checkpoint| checkpoint.room_id.clone());
        if retired_room_id.is_some() {
            if let Some(handle) = self.active_crawl_page.take() {
                handle.abort();
            }
            self.active_crawl_checkpoint = None;
        }
        retired_room_id
    }

    fn remove_history_crawl_room(&mut self, room_id: &str) {
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
                handle.abort();
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
            handle.abort();
        }
        self.active_crawl_checkpoint = None;
        if let Some(timer) = self.crawl_delay_timer.take() {
            timer.abort();
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

    fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }
}

fn classify_matrix_search_error(error: &koushi_sdk::MatrixSearchError) -> SearchFailureKind {
    match error {
        koushi_sdk::MatrixSearchError::IndexUnavailable => SearchFailureKind::IndexUnavailable,
        koushi_sdk::MatrixSearchError::Query => SearchFailureKind::Query,
        koushi_sdk::MatrixSearchError::Internal => SearchFailureKind::Internal,
    }
}

fn search_failure_message(kind: SearchFailureKind) -> &'static str {
    match kind {
        SearchFailureKind::IndexUnavailable => SEARCH_UNAVAILABLE_MESSAGE,
        SearchFailureKind::Query => "search query failed",
        SearchFailureKind::Internal => "search internal failure",
    }
}

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use koushi_search::{
        SearchCandidate, SearchDocumentStore, SearchEdit, SearchableEvent, SensitiveString,
    };

    // Helper constructors
    fn make_event(room_id: &str, event_id: &str, body: &str) -> SearchableEvent {
        SearchableEvent {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            sender: "@alice:test".to_owned(),
            timestamp_ms: 1000,
            body: Some(SensitiveString::new(body.to_owned())),
            attachment_filename: None,
            attachment: None,
        }
    }

    fn make_candidate(room_id: &str, event_id: &str) -> SearchCandidate {
        SearchCandidate {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            score_millis: 900,
        }
    }

    fn make_edit(target: &str, new_body: &str) -> SearchEdit {
        SearchEdit {
            edit_event_id: format!("{target}_edit"),
            target_event_id: target.to_owned(),
            sender: "@alice:test".to_owned(),
            timestamp_ms: 2000,
            body: Some(SensitiveString::new(new_body.to_owned())),
            attachment_filename: None,
            attachment: None,
        }
    }

    // --- Candidate verification rejects index false positives ---

    #[test]
    fn verify_candidate_rejects_false_positive() {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(make_event("!r:test", "$e1", "hello world"));
        // Candidate for a different event not in the store — must reject.
        let candidate = make_candidate("!r:test", "$not_indexed");
        assert!(
            store.verify_candidate(candidate, "hello").is_none(),
            "candidate for unindexed event must not verify"
        );
    }

    #[test]
    fn verify_candidate_rejects_stale_query() {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(make_event("!r:test", "$e1", "hello world"));
        let candidate = make_candidate("!r:test", "$e1");
        // Query doesn't appear in the body — false positive.
        assert!(
            store
                .verify_candidate(candidate, "foobar_not_present")
                .is_none(),
            "candidate must not verify against a query not in the body"
        );
    }

    #[test]
    fn verify_candidate_accepts_exact_match() {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(make_event("!r:test", "$e1", "検索対象メッセージ test body"));
        let candidate = make_candidate("!r:test", "$e1");
        assert!(
            store.verify_candidate(candidate, "検索対象").is_some(),
            "CJK substring must verify"
        );
    }

    // --- Edit mutation removes old terms and finds new ---

    #[test]
    fn edit_removes_old_body_and_indexes_new() {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(make_event("!r:test", "$e1", "original text"));

        // Verify old text matches before edit.
        let candidate_before = make_candidate("!r:test", "$e1");
        assert!(
            store
                .verify_candidate(candidate_before, "original")
                .is_some(),
            "original body must verify before edit"
        );

        // Apply edit.
        store.upsert_edit(make_edit("$e1", "replacement text"));

        // Old query must no longer verify.
        let candidate_after_old = make_candidate("!r:test", "$e1");
        assert!(
            store
                .verify_candidate(candidate_after_old, "original")
                .is_none(),
            "old body must not verify after edit"
        );

        // New query must verify.
        let candidate_after_new = make_candidate("!r:test", "$e1");
        assert!(
            store
                .verify_candidate(candidate_after_new, "replacement")
                .is_some(),
            "new body must verify after edit"
        );
    }

    // --- Redaction removes document ---

    #[test]
    fn redaction_removes_document_from_store() {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(make_event("!r:test", "$e1", "secret content"));

        let candidate_before = make_candidate("!r:test", "$e1");
        assert!(
            store.verify_candidate(candidate_before, "secret").is_some(),
            "must verify before redaction"
        );

        store.redact("$e1");

        let candidate_after = make_candidate("!r:test", "$e1");
        assert!(
            store.verify_candidate(candidate_after, "secret").is_none(),
            "must not verify after redaction"
        );
        assert_eq!(store.document_count(), 0, "document count must drop to 0");
    }

    #[test]
    fn clear_removes_documents_edits_pending_edits_and_aliases() {
        let mut store = SearchDocumentStore::default();
        store.upsert_message(make_event("!r:test", "$e1", "original content"));
        store.upsert_edit(make_edit("$e1", "edited content"));
        store.upsert_edit(make_edit("$missing", "pending edit"));

        assert_eq!(store.document_count(), 1);
        assert_eq!(store.pending_edit_count(), 1);
        assert!(
            store
                .verify_candidate(make_candidate("!r:test", "$e1_edit"), "edited")
                .is_some(),
            "edit alias must verify before clear"
        );

        store.clear();

        assert_eq!(store.document_count(), 0);
        assert_eq!(store.pending_edit_count(), 0);
        assert!(
            store
                .verify_candidate(make_candidate("!r:test", "$e1"), "edited")
                .is_none(),
            "cleared document must not verify"
        );
        assert!(
            store
                .verify_candidate(make_candidate("!r:test", "$e1_edit"), "edited")
                .is_none(),
            "cleared edit alias must not verify"
        );
        assert!(
            store
                .verify_candidate(make_candidate("!r:test", "$missing"), "pending")
                .is_none(),
            "cleared pending edit must not verify"
        );
    }

    // --- Unresolved replacement not indexed as standalone ---

    #[test]
    fn unresolved_replacement_not_indexed_as_standalone() {
        let mut store = SearchDocumentStore::default();
        // Arrive edit BEFORE original — should be a pending edit, not a standalone message.
        store.upsert_edit(make_edit("$original", "edited content"));

        // The pending edit must NOT be reachable as a standalone document.
        assert_eq!(
            store.document_count(),
            0,
            "edit before original must not appear as a document"
        );
        assert_eq!(
            store.pending_edit_count(),
            1,
            "edit before original must be pending"
        );

        // Querying for edited content must return nothing (no candidate to verify).
        let candidate = make_candidate("!r:test", "$original");
        assert!(
            store.verify_candidate(candidate, "edited").is_none(),
            "unresolved replacement must not be searchable"
        );
    }

    #[test]
    fn unresolved_replacement_resolves_when_original_arrives() {
        let mut store = SearchDocumentStore::default();
        store.upsert_edit(make_edit("$original", "edited content"));

        // Now original arrives — document_store should apply the pending edit.
        store.upsert_message(make_event("!r:test", "$original", "original content"));

        // Pending edit must have resolved.
        assert_eq!(store.pending_edit_count(), 0, "pending edit must resolve");

        // "edited content" must verify; "original content" must not.
        let c1 = make_candidate("!r:test", "$original");
        assert!(
            store.verify_candidate(c1, "edited").is_some(),
            "resolved edit body must be searchable"
        );
        let c2 = make_candidate("!r:test", "$original");
        assert!(
            store.verify_candidate(c2, "original content").is_none(),
            "superseded original body must not verify"
        );
    }

    // --- Failure kinds ---

    #[test]
    fn search_failure_kind_is_copy_eq() {
        use crate::failure::SearchFailureKind;
        let k1 = SearchFailureKind::IndexUnavailable;
        let k2 = k1;
        assert_eq!(k1, k2);
        let _ = SearchFailureKind::Query;
        let _ = SearchFailureKind::Internal;
    }

    #[test]
    fn search_query_failures_are_classified_from_sdk_error() {
        let source = include_str!("search.rs");
        let query_handler = source
            .split("async fn handle_query")
            .nth(1)
            .expect("query handler should exist")
            .split("async fn handle_index_message")
            .next()
            .expect("index handler should follow query handler");

        assert!(
            source.contains("fn classify_matrix_search_error"),
            "search failures must have a boundary classifier"
        );
        assert!(
            query_handler.contains("classify_matrix_search_error(&error)"),
            "query failures must classify the SDK search error before emitting CoreFailure"
        );
        assert!(
            !query_handler.contains("kind: SearchFailureKind::IndexUnavailable"),
            "query failures must not hardcode every SDK error as IndexUnavailable"
        );
    }

    // --- Debug redaction ---

    #[test]
    fn search_command_query_redacts_query_in_debug() {
        use crate::command::{SearchCommand, SearchScope};
        use crate::ids::{RequestId, RuntimeConnectionId};
        let cmd = SearchCommand::Query {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 1,
            },
            query: "super-secret-search-query".to_owned(),
            scope: SearchScope::AllRooms,
        };
        let debug = format!("{cmd:?}");
        assert!(
            !debug.contains("super-secret-search-query"),
            "query must not appear in Debug: {debug}"
        );
        assert!(
            debug.contains("SearchQuery(..)"),
            "redacted placeholder must appear in Debug: {debug}"
        );
    }

    #[test]
    fn search_index_message_upsert_redacts_body_in_debug() {
        let msg = super::SearchIndexMessage::Upsert {
            room_id: "!r:test".to_owned(),
            event_id: "$e:test".to_owned(),
            sender: "@a:test".to_owned(),
            timestamp_ms: 1000,
            body: Some("very-private-message-body".to_owned()),
            attachment_filename: None,
            attachment: None,
        };
        let debug = format!("{msg:?}");
        assert!(
            !debug.contains("very-private-message-body"),
            "body must not appear in Debug: {debug}"
        );
    }

    #[test]
    fn search_index_message_edit_redacts_body_in_debug() {
        let msg = super::SearchIndexMessage::Edit {
            edit_event_id: "$edit:test".to_owned(),
            target_event_id: "$orig:test".to_owned(),
            sender: "@a:test".to_owned(),
            timestamp_ms: 2000,
            body: Some("private-edited-content".to_owned()),
            attachment_filename: None,
            attachment: None,
        };
        let debug = format!("{msg:?}");
        assert!(
            !debug.contains("private-edited-content"),
            "body must not appear in Debug: {debug}"
        );
    }

    // --- SearchResultItem in SearchEvent redacts snippets from Debug ---

    #[test]
    fn search_result_item_snippet_is_redacted_from_debug() {
        use crate::event::{SearchEvent, SearchResultItem};
        use crate::ids::{RequestId, RuntimeConnectionId};
        let result = SearchResultItem {
            room_id: "!r:test".to_owned(),
            event_id: "$e:test".to_owned(),
            snippet: "检索目标消息 found here".to_owned(),
        };
        let event = SearchEvent::Results {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 2,
            },
            results: vec![result],
        };
        let debug = format!("{event:?}");
        assert!(
            !debug.contains("检索目标消息"),
            "snippet must not appear in SearchEvent Debug: {debug}"
        );
        assert!(
            !debug.contains("!r:test") && !debug.contains("$e:test"),
            "Matrix identifiers must not appear in SearchEvent Debug: {debug}"
        );
        assert!(
            debug.contains("result_count"),
            "redacted Debug should keep structural counts: {debug}"
        );
    }

    #[test]
    fn empty_query_is_not_special_cased_in_runtime() {
        let runtime_source = include_str!("runtime.rs");
        let search_source = include_str!("search.rs");

        assert!(
            !runtime_source.contains("is_empty_query"),
            "runtime should not special-case empty search queries"
        );
        assert!(
            !runtime_source.contains("results: Vec::new()"),
            "runtime should not locally settle empty search results"
        );
        assert!(
            search_source.contains("query.trim().is_empty()"),
            "search actor should own empty-query handling"
        );
        assert!(
            search_source.contains("CoreEvent::Search(SearchEvent::Results"),
            "search actor should still emit search results events"
        );
    }

    #[test]
    fn contiguous_pending_queries_coalesce_to_latest_without_crossing_non_query_messages() {
        use std::collections::VecDeque;
        use std::time::Instant;

        use crate::command::SearchScope;
        use crate::ids::{RequestId, RuntimeConnectionId};

        fn query(sequence: u64) -> super::SearchActorMessage {
            super::SearchActorMessage::Query {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence,
                },
                query: format!("q{sequence}"),
                scope: SearchScope::AllRooms,
                enqueued_at: Instant::now(),
            }
        }

        fn query_sequence(message: &super::SearchActorMessage) -> u64 {
            match message {
                super::SearchActorMessage::Query { request_id, .. } => request_id.sequence,
                other => panic!("expected query message, got {other:?}"),
            }
        }

        let mut pending = VecDeque::from([
            query(2),
            query(3),
            super::SearchActorMessage::RebuildIndex,
            query(5),
        ]);

        let (latest, dropped) = super::coalesce_contiguous_pending_queries(query(1), &mut pending);

        assert_eq!(query_sequence(&latest), 3);
        assert_eq!(dropped, 2);
        assert!(matches!(
            pending.front(),
            Some(super::SearchActorMessage::RebuildIndex)
        ));
        assert_eq!(
            query_sequence(pending.get(1).expect("query after rebuild")),
            5
        );
    }

    #[test]
    fn search_actor_crawler_uses_element_style_round_robin_checkpoints() {
        let source = include_str!("search.rs");

        assert!(
            source.contains(concat!("VecDeque", "<", "HistoryCrawlCheckpoint", ">")),
            "crawler must keep an actor-owned checkpoint queue"
        );
        assert!(
            source.contains(concat!("start", "_next", "_history", "_crawl", "_page")),
            "crawler must start one bounded /messages page at a time"
        );
        assert!(
            source.contains(concat!("push", "_back", "(", "next", "_checkpoint", ")")),
            "unfinished rooms must be requeued behind other checkpoints"
        );
    }

    #[test]
    fn search_actor_prunes_crawler_queue_when_joined_rooms_change() {
        let source = include_str!("search.rs");

        assert!(
            source.contains(concat!("retain", "_history", "_crawl", "_rooms")),
            "RoomsAvailable must prune queued/completed rooms against the current joined set"
        );
        assert!(
            source.contains(concat!(
                "abort", "_active", "_history", "_crawl", "_if", "_retired"
            )),
            "an in-flight page for a room that disappeared must be aborted before it can reinsert state"
        );
    }

    #[test]
    fn search_actor_history_crawler_uses_account_wide_messages_backpressure() {
        let source = include_str!("search.rs");
        let start_page_source = source
            .split(concat!("fn start", "_next", "_history", "_crawl", "_page"))
            .nth(1)
            .and_then(|section| {
                section
                    .split(concat!(
                        "async fn handle",
                        "_history",
                        "_crawl",
                        "_page",
                        "_result"
                    ))
                    .next()
            })
            .expect("crawler page starter should exist");

        assert!(
            source.contains("MessagesBackpressure"),
            "SearchActor must carry the shared account-wide /messages backpressure handle"
        );
        assert!(
            start_page_source.contains("messages_backpressure.clone()"),
            "each search crawler page must receive the shared /messages backpressure handle"
        );
    }

    #[test]
    fn search_actor_room_availability_notifications_have_nonblocking_entrypoint() {
        let source = include_str!("search.rs");
        let handle_impl = source
            .split("impl SearchActorHandle")
            .nth(1)
            .expect("SearchActorHandle impl")
            .split("// ---------------------------------------------------------------------------")
            .next()
            .expect("SearchActorHandle methods");

        assert!(
            handle_impl.contains("pub fn try_notify_rooms_available"),
            "background crawler room availability must have a nonblocking latest-wins delivery path"
        );
        assert!(
            handle_impl.contains(".try_send(SearchActorMessage::RoomsAvailable"),
            "nonblocking crawler notification delivery must use try_send"
        );
        assert!(
            !handle_impl.contains("TrySendError::Closed(_)) => Ok(())"),
            "closed SearchActor delivery must not be reported as success"
        );
    }

    #[test]
    fn search_crawler_lifecycle_projects_actor_owned_stop_settles() {
        let source = include_str!("search.rs");
        let start_handler = source
            .split("async fn handle_start_history_crawl")
            .nth(1)
            .expect("start handler should exist")
            .split("async fn handle_stop_history_crawl")
            .next()
            .expect("stop handler should follow start handler");
        let stop_handler = source
            .split("async fn handle_stop_history_crawl")
            .nth(1)
            .expect("stop handler should exist")
            .split("/// Auto-start")
            .next()
            .expect("rooms-available handler should follow stop handler");
        let rooms_available_handler = source
            .split("async fn handle_rooms_available")
            .nth(1)
            .expect("rooms available handler should exist")
            .split("async fn enqueue_history_crawl")
            .next()
            .expect("enqueue helper should follow rooms available handler");
        let retain_helper = source
            .split("fn retain_history_crawl_rooms")
            .nth(1)
            .expect("retain helper should exist")
            .split("fn abort_active_history_crawl_if_retired")
            .next()
            .expect("active abort helper should follow retain helper");

        assert!(
            start_handler.contains("settings.speed == SearchCrawlerSpeed::Paused")
                && start_handler.contains("self.emit_history_crawl_stopped(room_id).await"),
            "manual crawl start while paused must not leave a Queued room forever"
        );
        assert!(
            stop_handler.contains("self.emit_history_crawl_stopped(room_id).await"),
            "manual stop must settle the Rust-owned crawler room state"
        );
        assert!(
            rooms_available_handler.contains("self.stop_all_history_crawls().await")
                && rooms_available_handler
                    .contains("self.emit_history_crawl_stopped(room_id).await"),
            "pause and prune must emit stopped projections from the owning actor"
        );
        assert!(
            retain_helper.contains("-> Vec<String>"),
            "pruning must return retired room ids so AppState is settled"
        );
    }

    #[test]
    fn preempted_crawl_page_is_requeued() {
        let source = include_str!("search.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        let handler = production
            .split("fn handle_history_crawl_page_result")
            .nth(1)
            .and_then(|s| s.split("\n    fn ").next())
            .expect("handle_history_crawl_page_result should exist");
        assert!(
            handler.contains("HistoryCrawlPageResult::Preempted"),
            "the result handler must handle Preempted"
        );
        assert!(
            handler.contains("push_front"),
            "a preempted checkpoint must be re-queued at the front (no history lost)"
        );
    }

    #[test]
    fn automatic_crawl_starts_are_delayed_at_startup() {
        let source = include_str!("search.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        assert!(
            production.contains("CRAWLER_STARTUP_DELAY"),
            "there must be a crawler startup-delay constant"
        );
        let starter = production
            .split("fn start_next_history_crawl_page")
            .nth(1)
            .and_then(|s| s.split("\n    fn ").next())
            .or_else(|| production.split("fn start_next_history_crawl_page").nth(1))
            .expect("start_next_history_crawl_page should exist");
        assert!(
            starter.contains("crawl_delay_elapsed"),
            "automatic crawl-page starts must be gated on the startup delay"
        );
        assert!(
            starter.contains("manual"),
            "manual crawls must bypass the startup delay"
        );
    }
}
