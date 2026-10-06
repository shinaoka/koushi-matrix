use std::collections::{BTreeMap, HashSet};

use koushi_state::{
    AttachmentFilter, AttachmentKind, AttachmentResult, AttachmentScope, AttachmentSort,
    normalize_cjk_search_text,
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

/// Attachment metadata for the messages koushi has observed.
///
/// Search no longer reads this store. Candidates come from the persistent ngram
/// index and are verified against the encrypted event cache, so no message body
/// and no edit text is retained here; the Files view is the only reader, and it
/// needs attachment metadata only. Messages without an attachment are therefore
/// not stored at all.
#[derive(Default)]
pub struct SearchDocumentStore {
    /// event_id -> visible metadata of a message that carries an attachment.
    documents: BTreeMap<String, SearchableEvent>,
    /// Edits that arrived before their target, oldest first.
    pending_edits: BTreeMap<String, Vec<SearchEdit>>,
}

impl SearchDocumentStore {
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Room id of an indexed document, if it is currently resident.
    ///
    /// The document map is the single owner of indexed-identifier state; callers
    /// (`SearchActor`) must not keep a parallel event_id -> room_id map (#1150).
    pub fn room_id_of(&self, event_id: &str) -> Option<&str> {
        self.documents
            .get(event_id)
            .map(|event| event.room_id.as_str())
    }

    /// Whether an event id is currently resident in the store.
    pub fn contains(&self, event_id: &str) -> bool {
        self.documents.contains_key(event_id)
    }

    pub fn pending_edit_count(&self) -> usize {
        self.pending_edits.values().map(Vec::len).sum()
    }

    /// Bytes of message text this store currently retains.
    ///
    /// Search reads bodies from the encrypted event cache on demand, so this is
    /// a memory probe for the #1150 budget evidence rather than a feature; it
    /// stays zero however deep the indexed history grows.
    pub fn resident_body_bytes(&self) -> usize {
        self.documents
            .values()
            .map(|event| event.body.as_ref().map_or(0, |body| body.as_str().len()))
            .sum()
    }

    pub fn clear(&mut self) {
        self.documents.clear();
        self.pending_edits.clear();
    }

    pub fn upsert_message(&mut self, mut event: SearchableEvent) {
        if event.attachment.is_none() {
            // Nothing to show in the Files view, and search does not read this
            // store, so retaining the message would only cost memory.
            return;
        }
        retain_attachment_metadata(&mut event);

        let event_id = event.event_id.clone();
        self.documents.insert(event_id.clone(), event);

        if let Some(edits) = self.pending_edits.remove(&event_id)
            && let Some(latest) = latest_edit(edits)
            && let Some(stored) = self.documents.get_mut(&event_id)
        {
            apply_edit(stored, &latest);
        }
    }

    pub fn upsert_edit(&mut self, mut edit: SearchEdit) {
        // Edit text is never retained, here or while the edit is pending.
        edit.body = None;
        // The edit body's own content is dropped, but the edit event id and
        // timestamp still mark the attachment as edited.

        if let Some(event) = self.documents.get_mut(&edit.target_event_id) {
            apply_edit(event, &edit);
        } else {
            self.pending_edits
                .entry(edit.target_event_id.clone())
                .or_default()
                .push(edit);
        }
    }

    pub fn redact(&mut self, event_id: &str) {
        self.documents.remove(event_id);
        self.pending_edits.remove(event_id);
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
            .filter_map(|event| {
                let attachment = event.attachment.as_ref()?;

                if !filter.kinds.is_empty() && !filter.kinds.contains(&attachment.kind) {
                    return None;
                }

                if let Some(variants) = &query_variants {
                    let filename = normalize_cjk_search_text(attachment.filename.as_str());
                    if !variants
                        .iter()
                        .any(|variant| filename.contains(&normalize_cjk_search_text(variant)))
                    {
                        return None;
                    }
                }

                Some(AttachmentResult {
                    room_id: event.room_id.clone(),
                    event_id: event.event_id.clone(),
                    sender: event.sender.clone(),
                    sender_label: None,
                    timestamp_ms: event.timestamp_ms,
                    kind: attachment.kind,
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
                results.sort_by_key(|result| std::cmp::Reverse(result.timestamp_ms));
            }
            AttachmentSort::OldestFirst => {
                results.sort_by_key(|left| left.timestamp_ms);
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

/// Drop everything the Files view does not need.
///
/// The filename lives on the attachment, which is what the Files view reads; a
/// caller that only set the standalone `attachment_filename` still gets a usable
/// one copied across.
fn retain_attachment_metadata(event: &mut SearchableEvent) {
    if let (Some(attachment), Some(filename)) =
        (event.attachment.as_mut(), &event.attachment_filename)
        && attachment.filename.as_str().is_empty()
    {
        attachment.filename = filename.clone();
    }

    event.body = None;
    event.attachment_filename = None;
}

/// Apply an edit to the attachment metadata it can affect.
///
/// An edit may rename the file, replace the attachment, or only change the
/// caption; every case marks the row as edited, as the timeline does.
fn apply_edit(event: &mut SearchableEvent, edit: &SearchEdit) {
    let Some(attachment) = event.attachment.as_mut() else {
        return;
    };

    if let Some(filename) = &edit.attachment_filename {
        attachment.filename = filename.clone();
    }
    if let Some(replacement) = &edit.attachment {
        *attachment = replacement.clone();
    }
    attachment.is_edited = true;
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
