use std::collections::{BTreeMap, HashSet};

use koushi_state::{
    AttachmentFilter, AttachmentKind, AttachmentResult, AttachmentScope, AttachmentSort,
    SearchResult, normalize_cjk_search_text,
};
use serde::{Deserialize, Serialize};

use crate::SensitiveString;

pub fn cjk_search_query_variants(query: &str) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut variants = vec![query.to_owned()];
    let normalized = normalize_cjk_search_text(query);
    if !normalized.is_empty() && !variants.iter().any(|variant| variant == &normalized) {
        variants.push(normalized);
    }
    variants
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentDocument {
    pub kind: AttachmentKind,
    pub msgtype: String,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub source_mxc: String,
    pub thumbnail_mxc: Option<String>,
    pub filename: SensitiveString,
    pub thread_root: Option<String>,
    pub encrypted: bool,
    pub encryption_version: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub is_edited: bool,
}

impl std::fmt::Debug for AttachmentDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentDocument")
            .field("kind", &self.kind)
            .field("msgtype", &self.msgtype)
            .field("mimetype", &self.mimetype)
            .field("size", &self.size)
            .field("source_mxc", &"MxcUri(..)")
            .field(
                "thumbnail_mxc",
                &self.thumbnail_mxc.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("filename", &"AttachmentFilename(..)")
            .field(
                "thread_root",
                &self.thread_root.as_ref().map(|_| "EventId(..)"),
            )
            .field("encrypted", &self.encrypted)
            .field("encryption_version", &self.encryption_version)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("is_edited", &self.is_edited)
            .finish()
    }
}

#[derive(Default)]
pub struct SearchDocumentStore {
    documents: BTreeMap<String, SearchableEvent>,
    applied_edits: BTreeMap<String, SearchEdit>,
    pending_edits: BTreeMap<String, Vec<SearchEdit>>,
    /// Maps edit_event_id → original_event_id.
    ///
    /// The SDK's ngram index indexes edit events under the edit event_id (via
    /// `RoomIndexOperation::Edit` which removes the original and adds the edit
    /// event). This alias map lets `verify_candidate` resolve an edit_event_id
    /// back to the original document so verification succeeds.
    edit_aliases: BTreeMap<String, String>,
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
        // Register the alias so that when the SDK ngram index returns the
        // edit event_id as a candidate, verify_candidate can resolve it to
        // the original document.
        self.edit_aliases
            .insert(edit.edit_event_id.clone(), edit.target_event_id.clone());

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
        // Also remove any aliases pointing to this event.
        self.edit_aliases.retain(|_, target| target != event_id);
    }

    pub fn verify_candidate(
        &self,
        candidate: SearchCandidate,
        query: &str,
    ) -> Option<SearchResult> {
        // If the candidate event_id is an edit event alias, resolve to the
        // original document (the SDK's ngram index uses edit event_ids after
        // RoomIndexOperation::Edit removes the original and adds the edit).
        let resolved_event_id = self
            .edit_aliases
            .get(&candidate.event_id)
            .cloned()
            .unwrap_or_else(|| candidate.event_id.clone());

        let resolved_candidate = SearchCandidate {
            room_id: candidate.room_id.clone(),
            event_id: resolved_event_id.clone(),
            score_millis: candidate.score_millis,
        };

        let event = self.resolved_event(&resolved_event_id)?;
        crate::verify::verify_candidate(&resolved_candidate, &event, query)
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

            if let Some(attachment) = &edit.attachment {
                event.attachment = Some(attachment.clone());
            }
        }

        Some(event)
    }

    pub fn attachments(
        &self,
        scope: &AttachmentScope,
        filter: &AttachmentFilter,
        sort: AttachmentSort,
    ) -> Vec<AttachmentResult> {
        let allowed_rooms: Option<HashSet<&str>> = match scope {
            AttachmentScope::Account => None,
            AttachmentScope::Room { room_id } => Some(std::iter::once(room_id.as_str()).collect()),
            AttachmentScope::Space { child_room_ids, .. } => Some(
                child_room_ids
                    .iter()
                    .map(|room_id| room_id.as_str())
                    .collect(),
            ),
        };

        let query_variants = filter
            .filename_query
            .as_ref()
            .map(|query| crate::cjk_search_query_variants(query));

        let mut results: Vec<AttachmentResult> = self
            .documents
            .values()
            .filter(|event| {
                if let Some(rooms) = &allowed_rooms {
                    return rooms.contains(event.room_id.as_str());
                }
                true
            })
            .filter_map(|event| self.resolved_event(&event.event_id))
            .filter_map(|event| {
                let attachment = event.attachment.as_ref()?;

                if !filter.kinds.is_empty() && !filter.kinds.contains(&attachment.kind) {
                    return None;
                }

                if let Some(variants) = &query_variants {
                    let filename_lower = attachment.filename.as_str().to_lowercase();
                    if !variants
                        .iter()
                        .any(|variant| filename_lower.contains(&variant.to_lowercase()))
                    {
                        return None;
                    }
                }

                Some(AttachmentResult {
                    room_id: event.room_id.clone(),
                    event_id: event.event_id.clone(),
                    sender: event.sender.clone(),
                    timestamp_ms: event.timestamp_ms,
                    kind: attachment.kind.clone(),
                    filename: attachment.filename.as_str().to_owned(),
                    mimetype: attachment.mimetype.clone(),
                    size: attachment.size,
                    source_mxc: attachment.source_mxc.clone(),
                    thumbnail_mxc: attachment.thumbnail_mxc.clone(),
                    thread_root: attachment.thread_root.clone(),
                    encrypted: attachment.encrypted,
                    encryption_version: attachment.encryption_version.clone(),
                    width: attachment.width,
                    height: attachment.height,
                    is_edited: attachment.is_edited,
                })
            })
            .collect();

        match sort {
            AttachmentSort::NewestFirst => {
                results.sort_by(|left, right| right.timestamp_ms.cmp(&left.timestamp_ms));
            }
            AttachmentSort::OldestFirst => {
                results.sort_by(|left, right| left.timestamp_ms.cmp(&right.timestamp_ms));
            }
            AttachmentSort::Sender => {
                results.sort_by(|left, right| left.sender.cmp(&right.sender));
            }
            AttachmentSort::Filename => {
                results.sort_by(|left, right| left.filename.cmp(&right.filename));
            }
        }

        results
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchableEvent {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: Option<SensitiveString>,
    pub attachment_filename: Option<SensitiveString>,
    pub attachment: Option<AttachmentDocument>,
}

impl std::fmt::Debug for SearchableEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchableEvent")
            .field("room_id", &"RoomId(..)")
            .field("event_id", &"EventId(..)")
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
            .field("attachment", &self.attachment)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchCandidate {
    pub room_id: String,
    pub event_id: String,
    pub score_millis: u32,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchEdit {
    pub edit_event_id: String,
    pub target_event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub body: Option<SensitiveString>,
    pub attachment_filename: Option<SensitiveString>,
    pub attachment: Option<AttachmentDocument>,
}

impl std::fmt::Debug for SearchEdit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchEdit")
            .field("edit_event_id", &"EventId(..)")
            .field("target_event_id", &"EventId(..)")
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
            .field("attachment", &self.attachment)
            .finish()
    }
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
