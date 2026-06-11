use std::collections::BTreeMap;

use matrix_desktop_state::SearchResult;
use serde::{Deserialize, Serialize};

use crate::SensitiveString;

#[derive(Default)]
pub struct SearchDocumentStore {
    documents: BTreeMap<String, SearchableEvent>,
    applied_edits: BTreeMap<String, SearchEdit>,
    pending_edits: BTreeMap<String, Vec<SearchEdit>>,
}

impl SearchDocumentStore {
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    pub fn pending_edit_count(&self) -> usize {
        self.pending_edits.values().map(Vec::len).sum()
    }

    pub fn upsert_message(&mut self, event: SearchableEvent) {
        let event_id = event.event_id.clone();
        self.documents.insert(event_id.clone(), event);

        if let Some(edits) = self.pending_edits.remove(&event_id)
            && let Some(latest_edit) = latest_edit(edits)
        {
            self.applied_edits.insert(event_id, latest_edit);
        }
    }

    pub fn upsert_edit(&mut self, edit: SearchEdit) {
        if self.documents.contains_key(&edit.target_event_id) {
            self.apply_edit(edit);
        } else {
            self.pending_edits
                .entry(edit.target_event_id.clone())
                .or_default()
                .push(edit);
        }
    }

    pub fn redact(&mut self, event_id: &str) {
        self.documents.remove(event_id);
        self.applied_edits.remove(event_id);
        self.pending_edits.remove(event_id);
    }

    pub fn verify_candidate(
        &self,
        candidate: SearchCandidate,
        query: &str,
    ) -> Option<SearchResult> {
        let event = self.resolved_event(&candidate.event_id)?;
        crate::verify::verify_candidate(&candidate, &event, query)
    }

    fn apply_edit(&mut self, edit: SearchEdit) {
        let target_event_id = edit.target_event_id.clone();
        match self.applied_edits.get(&target_event_id) {
            Some(current) if !edit_is_newer(&edit, current) => {}
            _ => {
                self.applied_edits.insert(target_event_id, edit);
            }
        }
    }

    fn resolved_event(&self, event_id: &str) -> Option<SearchableEvent> {
        let mut event = self.documents.get(event_id)?.clone();

        if let Some(edit) = self.applied_edits.get(event_id) {
            if let Some(body) = &edit.body {
                event.body = Some(body.clone());
            }

            if let Some(attachment_filename) = &edit.attachment_filename {
                event.attachment_filename = Some(attachment_filename.clone());
            }
        }

        Some(event)
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchEdit {
    pub edit_event_id: String,
    pub target_event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: Option<SensitiveString>,
    pub attachment_filename: Option<SensitiveString>,
}

fn latest_edit(edits: Vec<SearchEdit>) -> Option<SearchEdit> {
    edits.into_iter().max_by(|left, right| {
        (left.timestamp_ms, left.edit_event_id.as_str())
            .cmp(&(right.timestamp_ms, right.edit_event_id.as_str()))
    })
}

fn edit_is_newer(candidate: &SearchEdit, current: &SearchEdit) -> bool {
    (candidate.timestamp_ms, candidate.edit_event_id.as_str())
        > (current.timestamp_ms, current.edit_event_id.as_str())
}
