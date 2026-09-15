use crate::MatrixClientSession;
use futures_util::{StreamExt as _, pin_mut};
use matrix_sdk_search::error::IndexError;
use std::{
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
        .map(|(index, (room_id, _score, event_id))| MatrixSearchCandidate {
            room_id: room_id.to_string(),
            event_id: event_id.to_string(),
            score_millis: 1_000_u32.saturating_sub(index as u32),
        })
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
        IndexError::TantivyError(_)
        | IndexError::IndexSchemaError(_)
        | IndexError::IndexWriteError(_)
        | IndexError::MessageTypeNotSupported
        | IndexError::CannotIndexRedactedMessage
        | IndexError::EmptyMessage => MatrixSearchError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::{MatrixSearchIndexKey, MatrixSearchIndexStoreConfig};

    use std::path::PathBuf;
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
}
