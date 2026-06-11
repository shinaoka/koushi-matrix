use std::collections::BTreeMap;

use matrix_desktop_state::SearchResult;
use serde::{Deserialize, Serialize};

use crate::SensitiveString;

#[derive(Default)]
pub struct SearchDocumentStore {
    documents: BTreeMap<String, SearchableEvent>,
}

impl SearchDocumentStore {
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    pub fn upsert_message(&mut self, event: SearchableEvent) {
        self.documents.insert(event.event_id.clone(), event);
    }

    pub fn verify_candidate(
        &self,
        candidate: SearchCandidate,
        query: &str,
    ) -> Option<SearchResult> {
        let event = self.documents.get(&candidate.event_id)?;
        crate::verify::verify_candidate(&candidate, event, query)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchableEvent {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: Option<SensitiveString>,
    pub attachment_filename: Option<SensitiveString>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchCandidate {
    pub room_id: String,
    pub event_id: String,
    pub score_millis: u32,
}

pub struct SearchEdit;
