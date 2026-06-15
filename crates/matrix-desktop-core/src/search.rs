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
//! The `SearchDocumentStore` (from `matrix-desktop-search`) is our in-process
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
//! `Query` commands emit `SearchFailed { kind: IndexUnavailable }`. The actor
//! never falls back to a plaintext index (Security Model).
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

use std::collections::HashMap;
use std::sync::Arc;

use matrix_desktop_sdk::MatrixClientSession;
use matrix_desktop_search::{
    SearchCandidate, SearchDocumentStore, SearchEdit, SearchableEvent, SensitiveString,
    cjk_search_query_variants,
};
use matrix_desktop_state::AppAction;
use tokio::sync::{broadcast, mpsc};

use crate::command::{SearchCommand, SearchScope};
use crate::event::{CoreEvent, SearchEvent, SearchResultItem};
use crate::executor;
use crate::failure::{CoreFailure, SearchFailureKind};
use crate::ids::RequestId;

/// Maximum number of candidates requested from the SDK ngram index.
/// Verification filters this down; the final result set may be smaller.
const SEARCH_CANDIDATE_LIMIT: usize = 50;
const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";

/// Search index mutation queue capacity (canon, overview.md: 512).
pub const SEARCH_INDEX_MUTATION_QUEUE: usize = 512;

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
    },
    /// A message was edited. Update the document store.
    Edit {
        edit_event_id: String,
        target_event_id: String,
        sender: String,
        timestamp_ms: u64,
        body: Option<String>,
        attachment_filename: Option<String>,
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
enum SearchActorMessage {
    /// A `SearchCommand::Query` from the command boundary.
    Query {
        request_id: RequestId,
        query: String,
        scope: SearchScope,
    },
    Shutdown,
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
            },
        };
        self.tx.send(msg).await.is_ok()
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(SearchActorMessage::Shutdown).await;
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
}

impl SearchActor {
    /// Spawn the actor and return its handle.
    pub fn spawn(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
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
        };

        // Spawn the actor task.
        executor::spawn(actor.run(index_rx));

        SearchActorHandle { tx, index_tx }
    }

    async fn run(mut self, mut index_rx: mpsc::Receiver<SearchIndexMessage>) {
        loop {
            tokio::select! {
                biased;
                msg = self.msg_rx.recv() => {
                    let Some(msg) = msg else { break };
                    match msg {
                        SearchActorMessage::Shutdown => break,
                        SearchActorMessage::Query { request_id, query, scope } => {
                            self.handle_query(request_id, &query, scope).await;
                        }
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

    async fn handle_query(&self, request_id: RequestId, query: &str, scope: SearchScope) {
        let query = query.trim();
        if query.trim().is_empty() {
            self.emit_search_succeeded(request_id, Vec::new()).await;
            self.emit(CoreEvent::Search(SearchEvent::Results {
                request_id,
                results: Vec::new(),
            }));
            return;
        }

        let mut candidates_by_key: HashMap<
            (String, String),
            matrix_desktop_sdk::MatrixSearchCandidate,
        > = HashMap::new();
        for query_variant in cjk_search_query_variants(query) {
            let candidates = matrix_desktop_sdk::search_message_candidates(
                &self.session,
                &query_variant,
                SEARCH_CANDIDATE_LIMIT,
            )
            .await;

            let candidates = match candidates {
                Ok(c) => c,
                Err(_) => {
                    self.emit_search_failed(request_id, SEARCH_UNAVAILABLE_MESSAGE)
                        .await;
                    self.emit_failure(
                        request_id,
                        CoreFailure::SearchFailed {
                            kind: SearchFailureKind::IndexUnavailable,
                        },
                    );
                    return;
                }
            };

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

        let mut candidates = candidates_by_key.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .score_millis
                .cmp(&left.score_millis)
                .then_with(|| left.room_id.cmp(&right.room_id))
                .then_with(|| left.event_id.cmp(&right.event_id))
        });

        // Apply scope filter.
        let candidates = match &scope {
            SearchScope::Global => candidates,
            SearchScope::Room { room_id } => candidates
                .into_iter()
                .filter(|c| &c.room_id == room_id)
                .collect(),
        };

        // Verify each candidate against the document store's canonical text.
        // Candidates that fail verification are silently dropped (Security Model).
        let mut projected_results = Vec::new();
        let mut compact_results = Vec::new();
        for sdk_candidate in candidates {
            let candidate = SearchCandidate {
                room_id: sdk_candidate.room_id.clone(),
                event_id: sdk_candidate.event_id.clone(),
                score_millis: sdk_candidate.score_millis,
            };
            let Some(result) = self.document_store.verify_candidate(candidate, query) else {
                continue;
            };
            compact_results.push(SearchResultItem {
                room_id: result.room_id.clone(),
                event_id: result.event_id.clone(),
                snippet: result.snippet.clone(),
            });
            projected_results.push(result);
        }

        self.emit_search_succeeded(request_id, projected_results)
            .await;
        self.emit(CoreEvent::Search(SearchEvent::Results {
            request_id,
            results: compact_results,
        }));
    }

    async fn emit_search_succeeded(
        &self,
        request_id: RequestId,
        results: Vec<matrix_desktop_state::SearchResult>,
    ) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchSucceeded {
                request_id: request_id.sequence,
                results,
            }])
            .await;
    }

    async fn emit_search_failed(&self, request_id: RequestId, message: &str) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
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

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use matrix_desktop_search::{
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
            scope: SearchScope::Global,
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
        };
        let debug = format!("{msg:?}");
        assert!(
            !debug.contains("private-edited-content"),
            "body must not appear in Debug: {debug}"
        );
    }

    // --- SearchResultItem in SearchEvent allows snippet (visible UI state) ---

    #[test]
    fn search_result_item_snippet_is_visible() {
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
        // Snippets in SearchEvent results are allowed visible UI state.
        let debug = format!("{event:?}");
        assert!(
            debug.contains("检索目标消息"),
            "snippet is allowed in SearchEvent Debug (visible UI state): {debug}"
        );
    }
}
