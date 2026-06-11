use std::collections::BTreeMap;

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

pub struct SearchCandidate;
pub struct SearchEdit;
