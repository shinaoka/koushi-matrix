use crate::MatrixClientSession;
use futures_util::{StreamExt as _, pin_mut};
use matrix_sdk_search::error::IndexError;
use std::{
    collections::{HashSet, VecDeque},
    fmt,
    path::{Path, PathBuf},
};
use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct MatrixSearchIndexStoreConfig {
    path: PathBuf,
    key: MatrixSearchIndexKey,
}

impl MatrixSearchIndexStoreConfig {
    pub fn new(path: impl Into<PathBuf>, key: MatrixSearchIndexKey) -> Self {
        Self {
            path: path.into(),
            key,
        }
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub(super) fn as_sdk_store_kind(&self) -> matrix_sdk::search_index::SearchIndexStoreKind {
        matrix_sdk::search_index::SearchIndexStoreKind::encrypted_directory_ngram(
            self.path.clone(),
            self.key.expose_key().to_owned(),
            2,
            4,
        )
        .expect("desktop ngram search bounds should be valid")
    }
}

impl fmt::Debug for MatrixSearchIndexStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSearchIndexStoreConfig")
            .field("path", &self.path)
            .field("key", &self.key)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MatrixSearchIndexKey {
    key: Zeroizing<String>,
}

impl MatrixSearchIndexKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: Zeroizing::new(key.into()),
        }
    }

    fn expose_key(&self) -> &str {
        self.key.as_str()
    }
}

impl fmt::Debug for MatrixSearchIndexKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatrixSearchIndexKey(..)")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixSearchCandidate {
    pub room_id: String,
    pub event_id: String,
    pub score_millis: u32,
}

/// Opaque paging cursor over the persistent literal index, newest first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixSearchCursor(matrix_sdk_search::index::SearchCursor);

impl MatrixSearchCursor {
    /// Milliseconds since the Unix epoch of the boundary event.
    pub fn timestamp_millis(&self) -> i64 {
        self.0.timestamp_millis
    }

    /// Event id of the boundary event.
    pub fn event_id(&self) -> &str {
        self.0.event_id.as_str()
    }
}

/// One page of literal search candidates from the persistent index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLiteralCandidate {
    pub room_id: String,
    /// The event id the index holds for this match. It may be an edit event id;
    /// resolution reports the original message identity.
    pub event_id: String,
    /// Milliseconds since the Unix epoch of the indexed entry, as the index
    /// ordered it. Together with `event_id` this is the key the candidate scan
    /// is ordered by, which is not necessarily the resolved content's timestamp.
    pub timestamp_millis: i64,
}

/// Current visible content resolved from the local event cache, edits and
/// redactions applied.
#[derive(Clone, Eq, PartialEq)]
pub struct MatrixResolvedMessage {
    pub event_id: String,
    pub current_event_id: String,
    pub sender: String,
    pub timestamp_ms: Option<u64>,
    /// Visible text: the body of a text message, or a media caption.
    pub body: Option<String>,
    /// Filename of a media message; never logged.
    pub attachment_filename: Option<String>,
}

impl fmt::Debug for MatrixResolvedMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixResolvedMessage")
            .field("event_id", &"EventId(..)")
            .field("current_event_id", &"EventId(..)")
            .field("sender", &"UserId(..)")
            .field("timestamp_ms", &self.timestamp_ms)
            .field("body", &self.body.as_ref().map(|_| "MessageBody(..)"))
            .field(
                "attachment_filename",
                &self
                    .attachment_filename
                    .as_ref()
                    .map(|_| "AttachmentFilename(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixSearchScope {
    AllRooms,
    CurrentRoom { room_id: String },
    RoomSet { room_ids: Vec<String> },
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MatrixSearchError {
    #[error("Matrix search index unavailable")]
    IndexUnavailable,
    #[error("Matrix search query failed")]
    Query,
    #[error("Matrix search internal failure")]
    Internal,
}

pub fn search_message_candidates_blocking(
    session: &MatrixClientSession,
    query: &str,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|_| MatrixSearchError::Internal)?;

    runtime.block_on(search_message_candidates(session, query, limit))
}

pub async fn search_message_candidates(
    session: &MatrixClientSession,
    query: &str,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    search_message_candidates_scoped(session, query, MatrixSearchScope::AllRooms, limit).await
}

pub async fn search_message_candidates_scoped(
    session: &MatrixClientSession,
    query: &str,
    scope: MatrixSearchScope,
    limit: usize,
) -> Result<Vec<MatrixSearchCandidate>, MatrixSearchError> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    match scope {
        MatrixSearchScope::CurrentRoom { room_id } => {
            let room_id =
                matrix_sdk::ruma::RoomId::parse(&room_id).map_err(|_| MatrixSearchError::Query)?;
            let Some(room) = session.client().get_room(&room_id) else {
                return Ok(Vec::new());
            };
            let iterator = room.search_messages(query.to_owned());
            pin_mut!(iterator);
            let Some(candidates) = iterator
                .next()
                .await
                .transpose()
                .map_err(|error| matrix_search_error_from_index(&error))?
            else {
                return Ok(Vec::new());
            };

            return Ok(candidates
                .into_iter()
                .take(limit)
                .enumerate()
                .map(|(index, (_score, event_id))| MatrixSearchCandidate {
                    room_id: room_id.to_string(),
                    event_id: event_id.to_string(),
                    score_millis: 1_000_u32.saturating_sub(index as u32),
                })
                .collect());
        }
        MatrixSearchScope::AllRooms | MatrixSearchScope::RoomSet { .. } => {}
    }

    let builder = session.client().search_messages(query.to_owned());
    let iterator = builder.build();
    pin_mut!(iterator);
    let Some(candidates) = iterator
        .next()
        .await
        .transpose()
        .map_err(|error| matrix_search_error_from_index(&error))?
    else {
        return Ok(Vec::new());
    };

    let mut candidates = candidates
        .into_iter()
        .take(limit)
        .enumerate()
        .map(
            |(index, (room_id, _score, event_id))| MatrixSearchCandidate {
                room_id: room_id.to_string(),
                event_id: event_id.to_string(),
                score_millis: 1_000_u32.saturating_sub(index as u32),
            },
        )
        .collect::<Vec<_>>();
    if let MatrixSearchScope::RoomSet { room_ids } = scope {
        candidates.retain(|candidate| room_ids.iter().any(|room_id| room_id == &candidate.room_id));
    }
    Ok(candidates)
}

fn matrix_search_error_from_index(error: &IndexError) -> MatrixSearchError {
    match error {
        IndexError::OpenDirectoryError(_) | IndexError::IO(_) => {
            MatrixSearchError::IndexUnavailable
        }
        IndexError::QueryParserError(_) => MatrixSearchError::Query,
        // A literal query that tokenizes to nothing: callers must fall back.
        IndexError::EmptyMessage => MatrixSearchError::Query,
        IndexError::TantivyError(_)
        | IndexError::IndexSchemaError(_)
        | IndexError::IndexWriteError(_)
        | IndexError::MessageTypeNotSupported
        | IndexError::CannotIndexRedactedMessage => MatrixSearchError::Internal,
    }
}

/// Fetch one bounded, newest-first page of cursors for a single room.
async fn fetch_literal_page(
    session: &MatrixClientSession,
    query: &str,
    room_id: &str,
    limit: usize,
    cursor: Option<MatrixSearchCursor>,
) -> Result<Vec<MatrixSearchCursor>, MatrixSearchError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixSearchError::Query)?;
    let Some(room) = session.client().get_room(&room_id) else {
        return Ok(Vec::new());
    };

    let page = room
        .search_literal_page(query, limit, cursor.map(|cursor| cursor.0))
        .await
        .map_err(|error| matrix_search_error_from_index(&error))?;

    Ok(page.into_iter().map(MatrixSearchCursor).collect())
}

/// One room's buffered position while merging literal index pages.
struct MatrixLiteralRoomStream {
    room_id: String,
    /// Exclusive upper bound for this room's next page.
    cursor: Option<MatrixSearchCursor>,
    /// Candidates already fetched for this room, newest first.
    buffered: VecDeque<MatrixLiteralCandidate>,
    /// Set once the room returned a page shorter than the page size.
    exhausted: bool,
}

/// Pages literal index matches across a search scope, newest first, without
/// offsets.
///
/// The ngram index is per room, so a scoped search keeps one bounded page per
/// room and refills a room only when its buffer drains. Buffered memory stays
/// bounded by `page_size * rooms`, independent of history depth, and skipping
/// no offset keeps Tantivy from collecting hits it discards.
pub struct MatrixLiteralSearchPager {
    query: String,
    page_size: usize,
    rooms: Vec<MatrixLiteralRoomStream>,
}

impl MatrixLiteralSearchPager {
    /// Build a pager over `scope`, newest match first.
    pub fn new(
        session: &MatrixClientSession,
        query: &str,
        scope: &MatrixSearchScope,
        page_size: usize,
    ) -> Self {
        Self {
            query: query.to_owned(),
            page_size: page_size.max(1),
            rooms: scope_room_ids(session, scope)
                .into_iter()
                .map(|room_id| MatrixLiteralRoomStream {
                    room_id,
                    cursor: None,
                    buffered: VecDeque::new(),
                    exhausted: false,
                })
                .collect(),
        }
    }

    /// Fetch the next globally newest `limit` candidates, refilling rooms as
    /// their buffers drain.
    ///
    /// Returns fewer than `limit` candidates only when every room in scope is
    /// exhausted.
    pub async fn next_page(
        &mut self,
        session: &MatrixClientSession,
        limit: usize,
    ) -> Result<Vec<MatrixLiteralCandidate>, MatrixSearchError> {
        let Self {
            query,
            page_size,
            rooms,
        } = self;
        let page_size = *page_size;
        let mut page = Vec::with_capacity(limit);

        while page.len() < limit {
            for room in rooms.iter_mut() {
                if !room.buffered.is_empty() || room.exhausted {
                    continue;
                }
                let fetched = fetch_literal_page(
                    session,
                    query.as_str(),
                    &room.room_id,
                    page_size,
                    room.cursor.clone(),
                )
                .await?;
                if fetched.len() < page_size {
                    room.exhausted = true;
                }
                room.cursor = fetched.last().cloned();
                room.buffered = fetched
                    .into_iter()
                    .map(|cursor| MatrixLiteralCandidate {
                        room_id: room.room_id.clone(),
                        event_id: cursor.event_id().to_owned(),
                        timestamp_millis: cursor.timestamp_millis(),
                    })
                    .collect();
            }

            let Some(index) = newest_buffered_room(rooms) else {
                break;
            };
            let hit = rooms[index]
                .buffered
                .pop_front()
                .expect("the chosen room has a buffered candidate");
            page.push(hit);
        }

        Ok(page)
    }
}

/// Rooms in `scope`, in a stable order.
fn scope_room_ids(session: &MatrixClientSession, scope: &MatrixSearchScope) -> Vec<String> {
    match scope {
        MatrixSearchScope::CurrentRoom { room_id } => vec![room_id.clone()],
        MatrixSearchScope::RoomSet { room_ids } => room_ids.clone(),
        MatrixSearchScope::AllRooms => session
            .client()
            .rooms()
            .into_iter()
            .map(|room| room.room_id().to_string())
            .collect(),
    }
}

/// Index of the room holding the globally newest buffered candidate.
///
/// The index pages by descending `(timestamp, event_id)`, so the same order
/// picks the next match across rooms.
fn newest_buffered_room(rooms: &[MatrixLiteralRoomStream]) -> Option<usize> {
    rooms
        .iter()
        .enumerate()
        .filter_map(|(index, room)| room.buffered.front().map(|hit| (index, hit)))
        .max_by(|(_, left), (_, right)| {
            left.timestamp_millis
                .cmp(&right.timestamp_millis)
                .then_with(|| left.event_id.cmp(&right.event_id))
        })
        .map(|(index, _)| index)
}

/// Inspect explicit cached redactions, including those whose targets are absent.
/// Missing events are not redaction evidence. The batch uses one SDK cache guard
/// and makes no network request.
///
/// # Errors
/// Returns `Query` for invalid identifiers or an unknown room, and `Internal`
/// when the SDK cannot read or decode its encrypted event cache.
pub async fn cached_redacted_event_ids(
    session: &MatrixClientSession,
    room_id: &str,
    event_ids: &[String],
) -> Result<HashSet<String>, MatrixSearchError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixSearchError::Query)?;
    let event_ids = event_ids
        .iter()
        .map(matrix_sdk::ruma::EventId::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| MatrixSearchError::Query)?;
    let room = session
        .client()
        .get_room(&room_id)
        .ok_or(MatrixSearchError::Query)?;
    let (cache, _handles) = room
        .event_cache()
        .await
        .map_err(|_| MatrixSearchError::Internal)?;
    let redacted = cache
        .redacted_event_ids(&event_ids)
        .await
        .map_err(|_| MatrixSearchError::Internal)?;
    Ok(redacted.into_iter().map(|id| id.to_string()).collect())
}

/// Resolve a message to its current visible content, reading only the local
/// event cache (no network). Returns `None` when it is missing or redacted.
pub async fn resolve_cached_message(
    session: &MatrixClientSession,
    room_id: &str,
    event_id: &str,
) -> Result<Option<MatrixResolvedMessage>, MatrixSearchError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixSearchError::Query)?;
    let event_id =
        matrix_sdk::ruma::EventId::parse(event_id).map_err(|_| MatrixSearchError::Query)?;
    let Some(room) = session.client().get_room(&room_id) else {
        return Ok(None);
    };

    let resolved = room
        .resolve_cached_message(&event_id)
        .await
        .map_err(|_| MatrixSearchError::Internal)?;

    Ok(resolved.map(|message| MatrixResolvedMessage {
        event_id: message.event_id.to_string(),
        current_event_id: message.current_event_id.to_string(),
        sender: message.sender.to_string(),
        timestamp_ms: message.timestamp_millis,
        body: message.body,
        attachment_filename: message.attachment_filename,
    }))
}

/// Index a room's events in the persistent ngram index now, awaiting the commit.
///
/// The SDK's own indexing runs in a background subscriber task that consumes
/// linked-chunk updates and can lag (`RecvError::Lagged`) or fail silently, so a
/// caller that needs an acknowledgement before reporting a successful crawl page
/// must write through here instead: the fork's index guard commits synchronously, so
/// an `Ok` return means the events are persisted. Indexing an event that is
/// already present is a no-op, and the guard's mutex serialises this with the
/// subscriber, so either way `Ok` is a real acknowledgement.
pub async fn index_room_events_now(
    session: &MatrixClientSession,
    room_id: &str,
    events: Vec<matrix_sdk::deserialized_responses::TimelineEvent>,
) -> Result<(), MatrixSearchError> {
    if events.is_empty() {
        return Ok(());
    }

    let room_id = matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| MatrixSearchError::Query)?;
    let Some(room) = session.client().get_room(&room_id) else {
        return Err(MatrixSearchError::Query);
    };
    let (room_cache, _drop_handles) = room
        .event_cache()
        .await
        .map_err(|_| MatrixSearchError::Internal)?;
    let redaction_rules = room.clone_info().room_version_rules_or_default().redaction;

    let client = session.client();
    let mut guard = client.search_index().lock().await;
    guard
        .bulk_handle_timeline_event(events.into_iter(), &room_cache, &room_id, &redaction_rules)
        .await
        .map_err(|error| matrix_search_error_from_index(&error))
}

#[cfg(test)]
mod tests {
    use super::{
        MatrixLiteralCandidate, MatrixLiteralRoomStream, MatrixSearchIndexKey,
        MatrixSearchIndexStoreConfig, newest_buffered_room,
    };

    use std::collections::VecDeque;
    use std::path::PathBuf;

    use matrix_sdk::ruma::{OwnedEventId, event_id};
    #[test]
    fn search_index_store_config_uses_encrypted_ngram_index() {
        let config = MatrixSearchIndexStoreConfig::new(
            PathBuf::from("search-index"),
            MatrixSearchIndexKey::new("synthetic-search-key"),
        );

        let kind = config.as_sdk_store_kind();

        assert!(matches!(
            kind,
            matrix_sdk::search_index::SearchIndexStoreKind::EncryptedDirectoryWithConfig(_, _, _)
        ));
    }

    fn room(room_id: &str, hits: &[(i64, OwnedEventId)]) -> MatrixLiteralRoomStream {
        MatrixLiteralRoomStream {
            room_id: room_id.to_owned(),
            cursor: None,
            exhausted: false,
            buffered: hits
                .iter()
                .map(|(timestamp_millis, event_id)| MatrixLiteralCandidate {
                    room_id: room_id.to_owned(),
                    event_id: event_id.to_string(),
                    timestamp_millis: *timestamp_millis,
                })
                .collect(),
        }
    }

    #[test]
    fn newest_buffered_room_merges_rooms_newest_first() {
        let mut rooms = vec![
            room(
                "!a:localhost",
                &[
                    (100, event_id!("$a1:localhost").to_owned()),
                    (50, event_id!("$a2:localhost").to_owned()),
                ],
            ),
            room(
                "!b:localhost",
                &[
                    (200, event_id!("$b1:localhost").to_owned()),
                    (100, event_id!("$b2:localhost").to_owned()),
                ],
            ),
        ];

        let mut order = Vec::new();
        while let Some(index) = newest_buffered_room(&rooms) {
            order.push(
                rooms[index]
                    .buffered
                    .pop_front()
                    .expect("non-empty")
                    .event_id,
            );
        }

        // Same-timestamp ties break by larger event id, matching the index.
        assert_eq!(
            order,
            [
                "$b1:localhost",
                "$b2:localhost",
                "$a1:localhost",
                "$a2:localhost"
            ]
        );
    }

    #[test]
    fn newest_buffered_room_ignores_drained_rooms() {
        let rooms = vec![MatrixLiteralRoomStream {
            room_id: "!a:localhost".to_owned(),
            cursor: None,
            exhausted: true,
            buffered: VecDeque::new(),
        }];

        assert!(newest_buffered_room(&rooms).is_none());
    }
}
