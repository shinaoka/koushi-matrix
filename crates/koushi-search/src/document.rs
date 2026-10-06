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
    /// event_id -> the edit whose content the row currently holds.
    applied_edits: BTreeMap<String, AppliedEdit>,
    /// event_id -> the redacted edits of that row, so a replay of any of them
    /// cannot come back. Bounded per row by [`RETIRED_EDITS_PER_ROW`].
    retired_edits: BTreeMap<String, Vec<String>>,
    /// event_id -> the newest canonical observation accepted for that row.
    ///
    /// The timeline projection stamps every message it produces with a
    /// monotonically increasing observation value, so a canonical message can be
    /// ordered against the one the row already holds without relying on the
    /// edit's own timestamp: a stale observation of an older state is refused,
    /// while a rollback (the SDK promoting an older surviving edit) applies.
    canonical_observations: BTreeMap<String, u64>,
    /// Edits that arrived before their target.
    pending_edits: BTreeMap<String, Vec<PendingEdit>>,
}

/// The edit whose content a row currently holds.
///
/// Kept so a replayed or out-of-order message/edit cannot regress the row: the
/// canonical timeline projection always carries the current visible state, while
/// a history crawl can replay an older version of the same message.
///
/// The derived `Ord` is the ordering: the edit's own event time and id decide
/// (the crawler pages newest first, so a later page can still carry an older
/// edit), and `canonical` only breaks a tie, where the timeline projection's
/// current visible state wins. A later history edit therefore still beats an
/// older canonical one; an edit rollback reaches a row by redacting the applied
/// edit, not by sending an older timestamp.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AppliedEdit {
    timestamp_ms: u64,
    canonical: bool,
    edit_event_id: String,
    /// The canonical observation this edit was applied under; `0` when a crawl
    /// applied it.
    observation: u64,
}

/// Identity of the edit whose content a message carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchEditKey {
    pub edit_event_id: String,
    pub timestamp_ms: u64,
}

impl SearchEditKey {
    pub fn new(edit_event_id: impl Into<String>, timestamp_ms: u64) -> Self {
        Self {
            edit_event_id: edit_event_id.into(),
            timestamp_ms,
        }
    }
}

impl AppliedEdit {
    fn from_key(key: &SearchEditKey, canonical: bool, observation: u64) -> Self {
        Self {
            timestamp_ms: key.timestamp_ms,
            canonical,
            edit_event_id: key.edit_event_id.clone(),
            observation,
        }
    }
}

/// How many redacted edit ids one row remembers. A crawl replays a bounded
/// number of versions, so the newest handful is enough to refuse a replay.
const RETIRED_EDITS_PER_ROW: usize = 8;

/// An edit waiting for its original message.
struct PendingEdit {
    edit: SearchEdit,
    canonical: bool,
    observation: u64,
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
        self.applied_edits.clear();
        self.retired_edits.clear();
        self.canonical_observations.clear();
        self.pending_edits.clear();
    }

    /// Record a message's attachment metadata.
    ///
    /// `canonical` marks the timeline projection, which always carries the
    /// message's current visible content; a history crawl reports what its crawl
    /// saw, which may be an older version of the same message. `edit` names the
    /// edit that produced the content when the message is edited.
    pub fn upsert_message(
        &mut self,
        mut event: SearchableEvent,
        canonical: bool,
        edit: Option<SearchEditKey>,
        observation: u64,
    ) {
        if event.attachment.is_none() {
            // Nothing to show in the Files view, and search does not read this
            // store, so retaining the message would only cost memory.
            return;
        }
        if !self.admits_message(&event.event_id, canonical, observation, edit.as_ref()) {
            return;
        }
        // A redacted edit must not come back through the content half of its
        // pair either: the redaction dropped the metadata it produced, and this
        // content is that same metadata.
        if let Some(key) = &edit
            && self.is_retired_edit(&event.event_id, &key.edit_event_id)
        {
            return;
        }
        retain_attachment_metadata(&mut event);

        let event_id = event.event_id.clone();
        self.documents.insert(event_id.clone(), event);

        if let Some(key) = edit {
            self.applied_edits.insert(
                event_id.clone(),
                AppliedEdit::from_key(&key, canonical, observation),
            );
        }
        // A keyless upsert says nothing about an edit the row already holds: it
        // can be a queued observation from before that edit existed, so only a
        // redaction (which retires the edit) clears the row's edit.

        if let Some(pending) = self.pending_edits.remove(&event_id) {
            for pending in pending {
                self.apply_edit_if_newer(&pending.edit, pending.canonical, pending.observation);
            }
        }
    }

    /// Whether an incoming message may replace what the row already holds.
    ///
    /// A stamped canonical (timeline) message is ordered by the observation the
    /// projection gave it, so a stale observation cannot undo a newer edit
    /// another producer applied while a rollback still applies. Anything
    /// unstamped (`observation == 0`, a history crawl or a caller that does not
    /// stamp) falls back to the edit-time rules, and a message's original never
    /// replaces the attachment an applied edit produced.
    fn admits_message(
        &mut self,
        event_id: &str,
        canonical: bool,
        observation: u64,
        edit: Option<&SearchEditKey>,
    ) -> bool {
        if canonical && observation > 0 {
            return self.admits_observation(event_id, observation);
        }

        let applied = self.applied_edits.get(event_id).cloned();
        match (edit, &applied) {
            (None, Some(_)) => false,
            (Some(key), Some(applied))
                if *applied >= AppliedEdit::from_key(key, canonical, observation) =>
            {
                false
            }
            _ => true,
        }
    }

    /// Apply the observation ordering for one row, recording the accepted value.
    fn admits_observation(&mut self, event_id: &str, observation: u64) -> bool {
        match self.canonical_observations.get(event_id).copied() {
            Some(last) if observation <= last => false,
            _ => {
                self.canonical_observations
                    .insert(event_id.to_owned(), observation);
                true
            }
        }
    }

    pub fn upsert_edit(&mut self, mut edit: SearchEdit, canonical: bool, observation: u64) {
        // Edit text is never retained, here or while the edit is pending.
        edit.body = None;
        if self.is_retired_edit(&edit.target_event_id, &edit.edit_event_id) {
            // The edit was redacted; it must not come back through a replay.
            return;
        }
        // The edit body's own content is dropped, but the edit event id and
        // timestamp still mark the attachment as edited.

        if self.documents.contains_key(&edit.target_event_id) {
            self.apply_edit_if_newer(&edit, canonical, observation);
        } else if edit.attachment.is_some() || edit.attachment_filename.is_some() {
            // A pending edit is only useful for a row that will carry an
            // attachment; a body-only edit cannot change one, and holding it
            // would grow with message history the store otherwise never keeps.
            self.pending_edits
                .entry(edit.target_event_id.clone())
                .or_default()
                .push(PendingEdit {
                    edit,
                    canonical,
                    observation,
                });
        }
    }

    /// Whether an incoming edit may replace what the row already holds.
    fn admits_edit(
        &mut self,
        target_event_id: &str,
        canonical: bool,
        observation: u64,
        edit: &SearchEdit,
    ) -> bool {
        if canonical && observation > 0 {
            return self.admits_observation(target_event_id, observation);
        }

        let incoming = AppliedEdit::from_key(
            &SearchEditKey::new(edit.edit_event_id.clone(), edit.timestamp_ms),
            canonical,
            observation,
        );
        self.applied_edits
            .get(target_event_id)
            .is_none_or(|applied| *applied < incoming)
    }

    /// Apply one edit unless the row already holds the same or a newer one.
    fn apply_edit_if_newer(
        &mut self,
        edit: &SearchEdit,
        canonical: bool,
        observation: u64,
    ) -> bool {
        if !self.admits_edit(&edit.target_event_id, canonical, observation, edit) {
            return false;
        }
        let incoming = AppliedEdit::from_key(
            &SearchEditKey::new(edit.edit_event_id.clone(), edit.timestamp_ms),
            canonical,
            observation,
        );
        if let Some(event) = self.documents.get_mut(&edit.target_event_id) {
            apply_edit(event, edit);
        }
        self.applied_edits
            .insert(edit.target_event_id.clone(), incoming);
        true
    }

    /// Whether this row already refused a replay of this redacted edit.
    fn is_retired_edit(&self, target_event_id: &str, edit_event_id: &str) -> bool {
        self.retired_edits
            .get(target_event_id)
            .is_some_and(|retired| retired.iter().any(|id| id == edit_event_id))
    }

    fn retire_edit(&mut self, target_event_id: &str, edit_event_id: &str) {
        let retired = self
            .retired_edits
            .entry(target_event_id.to_owned())
            .or_default();
        retired.push(edit_event_id.to_owned());
        if retired.len() > RETIRED_EDITS_PER_ROW {
            retired.remove(0);
        }
    }

    /// Remove a message, or retire a redacted edit.
    ///
    /// A redacted edit is no longer visible, so a row that holds it stops
    /// holding it (keyed by the edit event id, not the target id): the next
    /// message for that target -- a history replay of the original, or the
    /// canonical projection's current content -- can then set the row. Without
    /// this, an applied rename would pin the row against every later message.
    pub fn redact(&mut self, event_id: &str) {
        // A redacted edit is no longer visible, so it is retired and its content
        // is dropped: the row's attachment metadata came from that edit, and
        // nothing here can rebuild the version it replaced. The next message for
        // the target -- a replay of the original, or the canonical projection's
        // current content -- sets the row again.
        let mut affected: Vec<String> = self
            .applied_edits
            .iter()
            .filter(|(_, applied)| applied.edit_event_id == event_id)
            .map(|(target, _)| target.clone())
            .collect();
        affected.extend(
            self.pending_edits
                .iter()
                .filter(|(_, pending)| {
                    pending
                        .iter()
                        .any(|pending| pending.edit.edit_event_id == event_id)
                })
                .map(|(target, _)| target.clone()),
        );
        for target in affected {
            self.retire_edit(&target, event_id);
            self.documents.remove(&target);
            self.applied_edits.remove(&target);
            self.pending_edits.remove(&target);
        }

        self.retired_edits.remove(event_id);
        self.documents.remove(event_id);
        self.applied_edits.remove(event_id);
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
