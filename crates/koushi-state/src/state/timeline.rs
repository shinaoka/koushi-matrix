use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
};

use serde::{Deserialize, Deserializer, Serialize};

use crate::submission::{ComposerSubmissionTarget, ComposerTarget, SubmissionId};
use crate::{ComposerDocument, ComposerDraftRevision, ComposerDraftRevisionError};

use super::composer_draft::{
    ComposerDraftProtection, MAX_LIVE_COMPOSER_ROOM_TOMBSTONES, MAX_LIVE_COMPOSER_THREAD_TOMBSTONES,
};
use super::media_download::TimelineMediaDownloadState;
use super::settings::ImageUploadCompressionMode;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelinePaneState {
    pub room_id: Option<String>,
    pub is_subscribed: bool,
    pub is_paginating_backwards: bool,
    pub composer: ComposerState,
    #[serde(default)]
    pub submission_registry: ComposerSubmissionRegistry,
    pub scheduled_send_capability: ScheduledSendCapability,
    pub scheduled_sends: Vec<ScheduledSendItem>,
    pub staged_uploads: Vec<StagedUploadItem>,
    pub media_gallery: Vec<TimelineMediaGalleryItem>,
    #[serde(default)]
    pub media_downloads: std::collections::BTreeMap<String, TimelineMediaDownloadState>,
    #[serde(default)]
    pub continuity: TimelineContinuityState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineGapRepairFailureKind {
    Network,
    Timeout,
    Sdk,
    Cancelled,
    UnsupportedAnchor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineContinuityInspection {
    Unknown,
    Gapped { gap_count: u32 },
    Complete,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimelineContinuityState {
    #[default]
    Unknown,
    Inspecting {
        generation: u64,
        known_gap_count: u32,
    },
    Healthy {
        generation: u64,
        authoritative_start: bool,
    },
    Incomplete {
        generation: u64,
        gap_count: u32,
    },
    Repairing {
        generation: u64,
        gap_count: u32,
        batches_processed: u32,
        minimum_batch_id: Option<u64>,
    },
    FailedIncomplete {
        generation: u64,
        gap_count: u32,
        batches_processed: u32,
        failure_kind: TimelineGapRepairFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerSubmissionRegistry {
    pub accepted_submission_ids: VecDeque<SubmissionId>,
    #[serde(default)]
    pub queued_submission_ids: VecDeque<SubmissionId>,
    pub settled_submission_ids: VecDeque<SubmissionId>,
    #[serde(skip)]
    pub active_submissions: VecDeque<ComposerSubmissionRecord>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerSubmissionRecord {
    pub submission_id: SubmissionId,
    pub transaction_id: String,
    pub target: ComposerSubmissionTarget,
}

impl fmt::Debug for ComposerSubmissionRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ComposerSubmissionRecord(..)")
    }
}

impl ComposerSubmissionRegistry {
    pub(crate) fn remember_queued(&mut self, id: SubmissionId) {
        remember_bounded_id(&mut self.queued_submission_ids, id);
    }

    pub(crate) fn remember_accepted(
        &mut self,
        id: SubmissionId,
        transaction_id: String,
        target: ComposerSubmissionTarget,
    ) {
        if !self.accepted_submission_ids.contains(&id) {
            self.accepted_submission_ids.push_back(id.clone());
            self.active_submissions.push_back(ComposerSubmissionRecord {
                submission_id: id,
                transaction_id,
                target,
            });
        }
    }

    pub(crate) fn active_matches(
        &self,
        id: &SubmissionId,
        transaction_id: &str,
        target: &ComposerSubmissionTarget,
    ) -> bool {
        self.active_submissions.iter().any(|active| {
            &active.submission_id == id
                && active.transaction_id == transaction_id
                && &active.target == target
        })
    }

    pub(crate) fn remember_settled(&mut self, id: SubmissionId) {
        self.accepted_submission_ids.retain(|active| active != &id);
        self.queued_submission_ids.retain(|queued| queued != &id);
        self.active_submissions
            .retain(|active| active.submission_id != id);
        remember_bounded_id(&mut self.settled_submission_ids, id);
    }
}

fn remember_bounded_id(ids: &mut VecDeque<SubmissionId>, id: SubmissionId) {
    if ids.contains(&id) {
        return;
    }
    while ids.len() >= MAX_ACCEPTED_SUBMISSION_TOMBSTONES {
        ids.pop_front();
    }
    ids.push_back(id);
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct StagedUploadItem {
    pub staged_id: String,
    pub room_id: String,
    pub position: u64,
    pub filename: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub kind: StagedUploadKind,
    pub caption: Option<ComposerDocument>,
    pub compression_choice: StagedUploadCompressionChoice,
    #[serde(default)]
    pub preparation: StagedUploadPreparation,
}

impl fmt::Debug for StagedUploadItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedUploadItem")
            .field("staged_id", &self.staged_id)
            .field("room_id", &"RoomId(..)")
            .field("position", &self.position)
            .field("filename", &"MediaFilename(..)")
            .field("mime_type", &self.mime_type)
            .field("byte_count", &self.byte_count)
            .field("kind", &self.kind)
            .field(
                "caption",
                &self.caption.as_ref().map(|_| "MediaCaption(..)"),
            )
            .field("compression_choice", &self.compression_choice)
            .field("preparation", &self.preparation)
            .finish()
    }
}

/// Attachments may be sent once every item has a prepared output and none is
/// still recompressing (#500): the bytes that upload are the ones the UI shows.
/// The empty list is vacuously sendable; callers reject an empty staging list
/// separately when that matters.
/// Whether a prepared-upload send moves the composer draft into the message (#1204).
///
/// True when the send submits exactly one attachment whose caption *is* the submitted
/// draft document: the caption was seeded from that text, so the send carries it as
/// the message caption and the composer must not keep a copy. Content identity is the
/// policy — an edited caption, a newer draft, a whitespace-only document or any other
/// item count leaves the draft alone (#1130).
pub fn staged_upload_send_consumes_composer_draft(
    items: &[StagedUploadItem],
    draft_document: Option<&ComposerDocument>,
) -> bool {
    let [item] = items else {
        return false;
    };
    let Some(draft_document) =
        draft_document.filter(|document| !document.plain_body().trim().is_empty())
    else {
        return false;
    };
    item.caption.as_ref() == Some(draft_document)
}

pub fn staged_uploads_are_sendable(items: &[StagedUploadItem]) -> bool {
    items.iter().all(|item| {
        matches!(
            item.preparation,
            StagedUploadPreparation::Ready { pending: None, .. }
        )
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StagedUploadKind {
    Image {
        width: Option<u64>,
        height: Option<u64>,
    },
    File,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StagedUploadCompressionChoice {
    NotApplicable,
    Ask,
    Original,
    Compressed { mode: ImageUploadCompressionMode },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StagedUploadPreparation {
    #[default]
    Preparing,
    Ready {
        /// Completed combinations, reused immediately when re-selected.
        variants: Vec<PreparedUploadVariant>,
        /// The single owner of "which output will be uploaded".
        selected: StagedUploadOutputSelection,
        /// Set while `selected` has no prepared output yet.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pending: Option<StagedUploadOutputSelection>,
        /// Fences stale encode results so the latest selection wins.
        #[serde(default)]
        generation: u64,
    },
    Failed {
        failure_kind: MediaPreparationFailureKind,
        can_use_original: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaPreparationFailureKind {
    Empty,
    Unsupported,
    Decode,
    Encode,
    MissingPreparedBytes,
}

/// Actual encoding of a prepared output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedUploadFormat {
    Original,
    Png,
    Jpeg,
    Webp,
}

/// Linear scale the user chose for the upload, applied to both dimensions.
///
/// Independent of the encoding: `Original` preserves the source dimensions,
/// while [`StagedUploadFormatChoice::Keep`] preserves the source encoding.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StagedUploadResizeChoice {
    #[default]
    Original,
    Half,
    Quarter,
    Eighth,
}

/// Encoding the user chose for the upload. `Keep` preserves the source format;
/// for decode-only HEIF, only the unscaled Original/Keep pair is exact and a
/// resized Keep request is prepared as a compatible JPEG.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StagedUploadFormatChoice {
    #[default]
    Keep,
    Png,
    Jpeg,
    Webp,
}

/// The two independent axes that identify one prepared output.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedUploadOutputSelection {
    pub resize: StagedUploadResizeChoice,
    pub format: StagedUploadFormatChoice,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreparedUploadVariant {
    pub variant_id: String,
    /// Resize axis this output was prepared for.
    #[serde(default)]
    pub resize: StagedUploadResizeChoice,
    /// Format axis this output was prepared for, as chosen (not as resolved).
    #[serde(default)]
    pub format_choice: StagedUploadFormatChoice,
    pub filename: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub format: PreparedUploadFormat,
    pub savings_percent: i64,
    pub metadata_stripped: bool,
    pub thumbnail_refreshed: bool,
}

impl fmt::Debug for PreparedUploadVariant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedUploadVariant")
            .field("variant_id", &"PreparedVariantId(..)")
            .field("resize", &self.resize)
            .field("format_choice", &self.format_choice)
            .field("filename", &"MediaFilename(..)")
            .field("mime_type", &self.mime_type)
            .field("byte_count", &self.byte_count)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("savings_percent", &self.savings_percent)
            .field("metadata_stripped", &self.metadata_stripped)
            .field("thumbnail_refreshed", &self.thumbnail_refreshed)
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UploadStagingStore {
    pub items: std::collections::BTreeMap<(ComposerTarget, String), StagedUploadItem>,
}

/// Build the item that adopts a completed encode, or `None` when the result is
/// stale.
///
/// The fence lives here, not in the transport layer: a caller that ran the
/// encode outside the store still cannot let a slow result overwrite the output
/// the user is currently waiting for.
pub fn staged_upload_item_with_completed_output(
    item: &StagedUploadItem,
    prepared: PreparedUploadVariant,
    completed_generation: u64,
) -> Option<StagedUploadItem> {
    let StagedUploadPreparation::Ready {
        variants,
        selected,
        generation,
        ..
    } = &item.preparation
    else {
        return None;
    };
    if completed_generation != *generation
        || prepared.resize != selected.resize
        || prepared.format_choice != selected.format
    {
        return None;
    }
    let mut variants = variants.clone();
    variants.retain(|variant| {
        variant.resize != prepared.resize || variant.format_choice != prepared.format_choice
    });
    variants.push(prepared.clone());
    let mut next = item.clone();
    next.filename = prepared.filename.clone();
    next.mime_type = prepared.mime_type.clone();
    next.byte_count = prepared.byte_count;
    next.kind = StagedUploadKind::Image {
        width: prepared.width,
        height: prepared.height,
    };
    next.preparation = StagedUploadPreparation::Ready {
        variants,
        selected: *selected,
        pending: None,
        generation: *generation,
    };
    Some(next)
}

impl UploadStagingStore {
    pub fn items_for_target(&self, target: &ComposerTarget) -> Vec<StagedUploadItem> {
        let mut items = self
            .items
            .iter()
            .filter(|((item_target, _), _)| item_target == target)
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.staged_id.cmp(&right.staged_id))
        });
        items
    }

    pub fn items_for_room(&self, room_id: &str) -> Vec<StagedUploadItem> {
        self.items_for_target(&ComposerTarget::Main {
            room_id: room_id.to_owned(),
        })
    }

    pub fn replace_target_items(&mut self, target: ComposerTarget, items: Vec<StagedUploadItem>) {
        self.items
            .retain(|(item_target, _), _| item_target != &target);
        let target_room_id = target.room_id();
        for item in items
            .into_iter()
            .filter(|item| item.room_id == target_room_id)
        {
            self.items
                .insert((target.clone(), item.staged_id.clone()), item);
        }
    }

    pub fn replace_room_items(&mut self, room_id: &str, items: Vec<StagedUploadItem>) {
        self.replace_target_items(
            ComposerTarget::Main {
                room_id: room_id.to_owned(),
            },
            items,
        );
    }

    pub fn update_caption(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        caption: Option<ComposerDocument>,
    ) -> Option<StagedUploadItem> {
        let item = self
            .items
            .get_mut(&(target.clone(), staged_id.to_owned()))?;
        item.caption = caption;
        Some(item.clone())
    }

    pub fn update_compression_choice(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        compression_choice: StagedUploadCompressionChoice,
    ) -> Option<StagedUploadItem> {
        let item = self
            .items
            .get_mut(&(target.clone(), staged_id.to_owned()))?;
        item.compression_choice = compression_choice;
        Some(item.clone())
    }

    /// Choose one output by its resize/format pair.
    ///
    /// A pair that is already prepared is adopted immediately and describes the
    /// bytes that will be uploaded. A pair that is not prepared becomes
    /// `pending` under a fresh generation, so the last completed output keeps
    /// describing the upload until the new one lands and the latest selection
    /// wins over any encode still in flight.
    pub fn select_output(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        selection: StagedUploadOutputSelection,
    ) -> Option<StagedUploadItem> {
        let item = self
            .items
            .get_mut(&(target.clone(), staged_id.to_owned()))?;
        let StagedUploadPreparation::Ready {
            variants,
            selected,
            pending,
            generation,
        } = &mut item.preparation
        else {
            return None;
        };
        *selected = selection;
        match variants
            .iter()
            .find(|variant| {
                variant.resize == selection.resize && variant.format_choice == selection.format
            })
            .cloned()
        {
            Some(prepared) => {
                *pending = None;
                item.filename = prepared.filename;
                item.mime_type = prepared.mime_type;
                item.byte_count = prepared.byte_count;
                item.kind = StagedUploadKind::Image {
                    width: prepared.width,
                    height: prepared.height,
                };
            }
            None => {
                *pending = Some(selection);
                *generation = generation.saturating_add(1);
            }
        }
        Some(item.clone())
    }

    /// Adopt a completed encode, unless a newer selection superseded it.
    ///
    /// Returns `None` for a stale generation so a slow encode can never
    /// overwrite the output the user is currently waiting for.
    pub fn complete_output(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        prepared: PreparedUploadVariant,
        completed_generation: u64,
    ) -> Option<StagedUploadItem> {
        let item = self
            .items
            .get_mut(&(target.clone(), staged_id.to_owned()))?;
        let StagedUploadPreparation::Ready {
            variants,
            selected,
            pending,
            generation,
        } = &mut item.preparation
        else {
            return None;
        };
        if completed_generation != *generation {
            return None;
        }
        let matches_selection =
            prepared.resize == selected.resize && prepared.format_choice == selected.format;
        if !matches_selection {
            return None;
        }
        variants.retain(|variant| {
            variant.resize != prepared.resize || variant.format_choice != prepared.format_choice
        });
        variants.push(prepared.clone());
        *pending = None;
        item.filename = prepared.filename;
        item.mime_type = prepared.mime_type;
        item.byte_count = prepared.byte_count;
        item.kind = StagedUploadKind::Image {
            width: prepared.width,
            height: prepared.height,
        };
        Some(item.clone())
    }

    pub fn clear_target(&mut self, target: &ComposerTarget) -> bool {
        let before = self.items.len();
        self.items
            .retain(|(item_target, _), _| item_target != target);
        self.items.len() != before
    }

    pub fn clear_room(&mut self, room_id: &str) -> bool {
        self.clear_target(&ComposerTarget::Main {
            room_id: room_id.to_owned(),
        })
    }

    pub fn clear_thread_targets_for_room(&mut self, room_id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|(target, _), _| {
            !matches!(
                target,
                ComposerTarget::Thread {
                    room_id: target_room_id,
                    ..
                } if target_room_id == room_id
            )
        });
        self.items.len() != before
    }

    pub fn retain_rooms(&mut self, room_ids: &BTreeSet<String>) {
        self.items
            .retain(|(target, _), _| room_ids.contains(target.room_id()));
    }
}

impl fmt::Debug for UploadStagingStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadStagingStore")
            .field(
                "items",
                &format_args!("{} staged upload(s)", self.items.len()),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMediaGalleryItem {
    pub event_id: String,
    pub room_id: String,
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    pub timestamp_ms: u64,
    pub media: TimelineMediaGalleryMedia,
}

impl fmt::Debug for TimelineMediaGalleryItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMediaGalleryItem")
            .field("event_id", &self.event_id)
            .field("room_id", &"RoomId(..)")
            .field("sender", &self.sender.as_ref().map(|_| "UserId(..)"))
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field("timestamp_ms", &"Timestamp(..)")
            .field("media", &self.media)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMediaGalleryMedia {
    pub kind: TimelineMediaKind,
    pub filename: String,
    pub source: TimelineMediaGallerySource,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub thumbnail: Option<TimelineMediaGalleryThumbnail>,
}

impl fmt::Debug for TimelineMediaGalleryMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMediaGalleryMedia")
            .field("kind", &self.kind)
            .field("filename", &"MediaFilename(..)")
            .field("source", &self.source)
            .field("mimetype", &self.mimetype)
            .field("size", &self.size)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("thumbnail", &self.thumbnail)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineMediaKind {
    Image,
    File,
    Audio,
    Video,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMediaGallerySource {
    pub mxc_uri: String,
    pub encrypted: bool,
    pub encryption_version: Option<String>,
}

impl fmt::Debug for TimelineMediaGallerySource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMediaGallerySource")
            .field("mxc_uri", &"MxcUri(..)")
            .field("encrypted", &self.encrypted)
            .field("encryption_version", &self.encryption_version)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMediaGalleryThumbnail {
    pub source: TimelineMediaGallerySource,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaGalleryStore {
    pub rooms: std::collections::BTreeMap<String, Vec<TimelineMediaGalleryItem>>,
}

impl MediaGalleryStore {
    pub fn items_for_room(&self, room_id: &str) -> Vec<TimelineMediaGalleryItem> {
        let mut items = self.rooms.get(room_id).cloned().unwrap_or_default();
        items.sort_by(|left, right| {
            right
                .timestamp_ms
                .cmp(&left.timestamp_ms)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        items
    }

    pub fn replace_room_items(&mut self, room_id: &str, items: Vec<TimelineMediaGalleryItem>) {
        let mut items = items
            .into_iter()
            .filter(|item| item.room_id == room_id)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .timestamp_ms
                .cmp(&left.timestamp_ms)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        if items.is_empty() {
            self.rooms.remove(room_id);
        } else {
            self.rooms.insert(room_id.to_owned(), items);
        }
    }

    pub fn retain_rooms(&mut self, room_ids: &BTreeSet<String>) {
        self.rooms.retain(|room_id, _| room_ids.contains(room_id));
    }
}

impl fmt::Debug for MediaGalleryStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let item_count = self.rooms.values().map(Vec::len).sum::<usize>();
        formatter
            .debug_struct("MediaGalleryStore")
            .field("rooms", &format_args!("{} room(s)", self.rooms.len()))
            .field("items", &format_args!("{item_count} media gallery item(s)"))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledSendItem {
    pub scheduled_id: String,
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_event_id: Option<String>,
    pub body: String,
    pub send_at_ms: u64,
    pub handle: ScheduledSendHandle,
    #[serde(skip)]
    pub is_dispatching: bool,
}

impl fmt::Debug for ScheduledSendItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScheduledSendItem")
            .field("scheduled_id", &self.scheduled_id)
            .field("room_id", &"RoomId(..)")
            .field(
                "thread_root_event_id",
                &self.thread_root_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field("body", &"MessageBody(..)")
            .field("send_at_ms", &"Timestamp(..)")
            .field("handle", &self.handle)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScheduledSendHandle {
    Local,
    Server { delay_id: String },
}

impl fmt::Debug for ScheduledSendHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => formatter.write_str("Local"),
            Self::Server { .. } => formatter
                .debug_struct("Server")
                .field("delay_id", &"DelayedEventHandle(..)")
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledSendCapability {
    #[default]
    Unknown,
    ServerDelayedEvents,
    LocalFallback,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduledSendStore {
    pub capability: ScheduledSendCapability,
    pub items: std::collections::BTreeMap<String, ScheduledSendItem>,
}

impl ScheduledSendStore {
    pub fn items_for_room(&self, room_id: &str) -> Vec<ScheduledSendItem> {
        let mut items = self
            .items
            .values()
            .filter(|item| item.room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.send_at_ms
                .cmp(&right.send_at_ms)
                .then_with(|| left.scheduled_id.cmp(&right.scheduled_id))
        });
        items
    }

    pub fn insert(&mut self, item: ScheduledSendItem) {
        self.items.insert(item.scheduled_id.clone(), item);
    }

    pub fn remove(&mut self, scheduled_id: &str) -> Option<ScheduledSendItem> {
        self.items.remove(scheduled_id)
    }

    pub fn reschedule(
        &mut self,
        scheduled_id: &str,
        body: String,
        send_at_ms: u64,
        handle: ScheduledSendHandle,
    ) -> Option<ScheduledSendItem> {
        let item = self.items.get_mut(scheduled_id)?;
        item.body = body;
        item.send_at_ms = send_at_ms;
        item.handle = handle;
        item.is_dispatching = false;
        Some(item.clone())
    }

    pub fn start_local_dispatch(&mut self, scheduled_id: &str) -> Option<ScheduledSendItem> {
        let item = self.items.get_mut(scheduled_id)?;
        if !matches!(item.handle, ScheduledSendHandle::Local) {
            return None;
        }
        item.is_dispatching = true;
        Some(item.clone())
    }

    pub fn retry_local_dispatch(
        &mut self,
        scheduled_id: &str,
        retry_at_ms: u64,
    ) -> Option<ScheduledSendItem> {
        let item = self.items.get_mut(scheduled_id)?;
        if !matches!(item.handle, ScheduledSendHandle::Local) {
            return None;
        }
        item.is_dispatching = false;
        item.send_at_ms = retry_at_ms;
        Some(item.clone())
    }

    pub fn next_due(&self, now_ms: u64) -> Option<ScheduledSendItem> {
        self.items
            .values()
            .filter(|item| item.send_at_ms <= now_ms)
            .min_by(|left, right| {
                left.send_at_ms
                    .cmp(&right.send_at_ms)
                    .then_with(|| left.scheduled_id.cmp(&right.scheduled_id))
            })
            .cloned()
    }

    pub fn next_local_due(&self, now_ms: u64) -> Option<ScheduledSendItem> {
        self.items
            .values()
            .filter(|item| matches!(item.handle, ScheduledSendHandle::Local))
            .filter(|item| !item.is_dispatching)
            .filter(|item| item.send_at_ms <= now_ms)
            .min_by(|left, right| {
                left.send_at_ms
                    .cmp(&right.send_at_ms)
                    .then_with(|| left.scheduled_id.cmp(&right.scheduled_id))
            })
            .cloned()
    }

    pub fn next_send_at_ms(&self) -> Option<u64> {
        self.items.values().map(|item| item.send_at_ms).min()
    }

    pub fn next_local_send_at_ms(&self) -> Option<u64> {
        self.items
            .values()
            .filter(|item| matches!(item.handle, ScheduledSendHandle::Local))
            .filter(|item| !item.is_dispatching)
            .map(|item| item.send_at_ms)
            .min()
    }

    pub fn retain_rooms(&mut self, room_ids: &BTreeSet<String>) {
        self.items
            .retain(|_, item| room_ids.contains(item.room_id.as_str()));
    }
}

impl fmt::Debug for ScheduledSendStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScheduledSendStore")
            .field("capability", &self.capability)
            .field(
                "items",
                &format_args!("{} scheduled send(s)", self.items.len()),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ComposerDraftPersistenceEntry {
    pub content: Option<ComposerDocument>,
    pub revision: ComposerDraftRevision,
    pub last_accepted_clear_revision: ComposerDraftRevision,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct ComposerDraftPersistenceProjection {
    pub rooms: BTreeMap<String, ComposerDraftPersistenceEntry>,
    pub threads: BTreeMap<String, BTreeMap<String, ComposerDraftPersistenceEntry>>,
    pub quiescent_room_order: Vec<String>,
    pub quiescent_thread_order: Vec<(String, String)>,
    pub protected_empty_rooms: Vec<String>,
    pub protected_empty_threads: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerDraftPersistenceImportError {
    InvalidProjection,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerDraftStore {
    #[serde(
        default,
        skip_serializing_if = "std::collections::BTreeMap::is_empty",
        deserialize_with = "deserialize_room_documents"
    )]
    pub rooms: std::collections::BTreeMap<String, ComposerDocument>,
    #[serde(
        default,
        skip_serializing_if = "std::collections::BTreeMap::is_empty",
        deserialize_with = "deserialize_thread_documents"
    )]
    pub threads:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, ComposerDocument>>,
    /// Monotonic causal fences. Empty-draft entries are retained so an accepted
    /// send remains newer than a delayed pre-acceptance write.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub room_revisions: std::collections::BTreeMap<String, ComposerDraftRevision>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thread_revisions: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, ComposerDraftRevision>,
    >,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub room_last_accepted_clear_revisions:
        std::collections::BTreeMap<String, ComposerDraftRevision>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thread_last_accepted_clear_revisions: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, ComposerDraftRevision>,
    >,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    quiescent_room_lru: VecDeque<String>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    quiescent_thread_lru: VecDeque<(String, String)>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ComposerDocumentWire {
    Plain(String),
    Structured(ComposerDocument),
}

impl ComposerDocumentWire {
    fn into_document(self) -> ComposerDocument {
        match self {
            Self::Plain(text) => ComposerDocument::from_plain_text(text),
            Self::Structured(document) => document,
        }
    }
}

fn deserialize_room_documents<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, ComposerDocument>, D::Error>
where
    D: Deserializer<'de>,
{
    BTreeMap::<String, ComposerDocumentWire>::deserialize(deserializer).map(|rooms| {
        rooms
            .into_iter()
            .map(|(room_id, document)| (room_id, document.into_document()))
            .collect()
    })
}

fn deserialize_thread_documents<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeMap<String, ComposerDocument>>, D::Error>
where
    D: Deserializer<'de>,
{
    BTreeMap::<String, BTreeMap<String, ComposerDocumentWire>>::deserialize(deserializer).map(
        |rooms| {
            rooms
                .into_iter()
                .map(|(room_id, threads)| {
                    (
                        room_id,
                        threads
                            .into_iter()
                            .map(|(root_event_id, document)| {
                                (root_event_id, document.into_document())
                            })
                            .collect(),
                    )
                })
                .collect()
        },
    )
}

pub const MAX_PERSISTED_COMPOSER_DRAFT_BYTES: usize = 16 * 1024;
pub const MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT: usize = 128;
pub const MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT: usize = 256;

impl ComposerDraftStore {
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
            && self.threads.is_empty()
            && self.room_revisions.is_empty()
            && self.thread_revisions.is_empty()
            && self.room_last_accepted_clear_revisions.is_empty()
            && self.thread_last_accepted_clear_revisions.is_empty()
            && self.quiescent_room_lru.is_empty()
            && self.quiescent_thread_lru.is_empty()
    }

    pub fn composer_for_room(&self, room_id: &str) -> ComposerState {
        let mut composer = ComposerState::default();
        if let Some(document) = self.rooms.get(room_id) {
            composer.document = document.clone();
            composer.draft = document.plain_body();
        }
        composer.draft_revision = self.room_revision(room_id);
        composer.last_accepted_clear_revision = self
            .room_last_accepted_clear_revisions
            .get(room_id)
            .copied()
            .unwrap_or_default();
        composer
    }

    pub fn set_room_draft(&mut self, room_id: String, document: impl Into<ComposerDocument>) {
        let document = document.into();
        let Ok(revision) = ComposerDraftRevision::checked_successor(
            self.room_revision(&room_id),
            ComposerDraftRevision::ZERO,
        ) else {
            return;
        };
        let _ = self.apply_room_draft(room_id, document, revision);
    }

    pub fn room_revision(&self, room_id: &str) -> ComposerDraftRevision {
        self.room_revisions
            .get(room_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn apply_room_draft(
        &mut self,
        room_id: String,
        document: impl Into<ComposerDocument>,
        revision: ComposerDraftRevision,
    ) -> Result<bool, ComposerDraftRevisionError> {
        let document = document.into();
        if revision <= self.room_revision(&room_id) {
            return Ok(false);
        }
        self.room_revisions.insert(room_id.clone(), revision);
        if document.is_empty() {
            self.rooms.remove(&room_id);
            self.touch_quiescent_room(&room_id);
        } else {
            self.rooms.insert(room_id.clone(), document);
            self.remove_room_from_lru(&room_id);
        }
        Ok(true)
    }

    pub fn advance_room_revision(
        &mut self,
        room_id: &str,
        submitted_revision: ComposerDraftRevision,
    ) -> Result<ComposerDraftRevision, ComposerDraftRevisionError> {
        let current_revision = self.room_revision(room_id);
        let revision =
            ComposerDraftRevision::checked_successor(current_revision, submitted_revision)?;
        if current_revision <= submitted_revision {
            self.rooms.remove(room_id);
            self.room_last_accepted_clear_revisions
                .insert(room_id.to_owned(), revision);
        }
        self.room_revisions.insert(room_id.to_owned(), revision);
        if self.rooms.contains_key(room_id) {
            self.remove_room_from_lru(room_id);
        } else {
            self.touch_quiescent_room(room_id);
        }
        Ok(revision)
    }

    /// #1130: settle a room's composer draft revision without clearing its
    /// content. A staged-attachment send settles the draft it never consumed, so
    /// text the user still has to send stays available while the revision,
    /// tombstones and LRU accounting stay exactly as an accepted send leaves
    /// them.
    pub fn settle_room_revision(
        &mut self,
        room_id: &str,
        submitted_revision: ComposerDraftRevision,
    ) -> Result<ComposerDraftRevision, ComposerDraftRevisionError> {
        let revision = ComposerDraftRevision::checked_successor(
            self.room_revision(room_id),
            submitted_revision,
        )?;
        self.room_revisions.insert(room_id.to_owned(), revision);
        if self.rooms.contains_key(room_id) {
            self.remove_room_from_lru(room_id);
        } else {
            self.touch_quiescent_room(room_id);
        }
        Ok(revision)
    }

    /// #1130: thread-target counterpart of [`Self::settle_room_revision`].
    pub fn settle_thread_revision(
        &mut self,
        room_id: &str,
        root_event_id: &str,
        submitted_revision: ComposerDraftRevision,
    ) -> Result<ComposerDraftRevision, ComposerDraftRevisionError> {
        let revision = ComposerDraftRevision::checked_successor(
            self.thread_revision(room_id, root_event_id),
            submitted_revision,
        )?;
        self.thread_revisions
            .entry(room_id.to_owned())
            .or_default()
            .insert(root_event_id.to_owned(), revision);
        if self
            .threads
            .get(room_id)
            .is_some_and(|threads| threads.contains_key(root_event_id))
        {
            self.remove_thread_from_lru(room_id, root_event_id);
        } else {
            self.touch_quiescent_thread(room_id, root_event_id);
        }
        Ok(revision)
    }

    pub fn clear_room_draft(&mut self, room_id: &str) {
        self.rooms.remove(room_id);
        self.room_revisions.remove(room_id);
        self.room_last_accepted_clear_revisions.remove(room_id);
        self.remove_room_from_lru(room_id);
    }

    pub fn composer_for_thread(&self, room_id: &str, root_event_id: &str) -> ComposerState {
        let mut composer = ComposerState::default();
        if let Some(document) = self
            .threads
            .get(room_id)
            .and_then(|room_threads| room_threads.get(root_event_id))
        {
            composer.document = document.clone();
            composer.draft = document.plain_body();
        }
        composer.draft_revision = self.thread_revision(room_id, root_event_id);
        composer.last_accepted_clear_revision = self
            .thread_last_accepted_clear_revisions
            .get(room_id)
            .and_then(|room_threads| room_threads.get(root_event_id))
            .copied()
            .unwrap_or_default();
        composer
    }

    pub fn set_thread_draft(
        &mut self,
        room_id: String,
        root_event_id: String,
        document: impl Into<ComposerDocument>,
    ) {
        let document = document.into();
        let Ok(revision) = ComposerDraftRevision::checked_successor(
            self.thread_revision(&room_id, &root_event_id),
            ComposerDraftRevision::ZERO,
        ) else {
            return;
        };
        let _ = self.apply_thread_draft(room_id, root_event_id, document, revision);
    }

    pub fn thread_revision(&self, room_id: &str, root_event_id: &str) -> ComposerDraftRevision {
        self.thread_revisions
            .get(room_id)
            .and_then(|room_threads| room_threads.get(root_event_id))
            .copied()
            .unwrap_or_default()
    }

    pub fn apply_thread_draft(
        &mut self,
        room_id: String,
        root_event_id: String,
        document: impl Into<ComposerDocument>,
        revision: ComposerDraftRevision,
    ) -> Result<bool, ComposerDraftRevisionError> {
        let document = document.into();
        if revision <= self.thread_revision(&room_id, &root_event_id) {
            return Ok(false);
        }
        self.thread_revisions
            .entry(room_id.clone())
            .or_default()
            .insert(root_event_id.clone(), revision);
        if document.is_empty() {
            self.remove_thread_content(&room_id, &root_event_id);
            self.touch_quiescent_thread(&room_id, &root_event_id);
        } else {
            self.threads
                .entry(room_id.clone())
                .or_default()
                .insert(root_event_id.clone(), document);
            self.remove_thread_from_lru(&room_id, &root_event_id);
        }
        Ok(true)
    }

    pub fn advance_thread_revision(
        &mut self,
        room_id: &str,
        root_event_id: &str,
        submitted_revision: ComposerDraftRevision,
    ) -> Result<ComposerDraftRevision, ComposerDraftRevisionError> {
        let current_revision = self.thread_revision(room_id, root_event_id);
        let revision =
            ComposerDraftRevision::checked_successor(current_revision, submitted_revision)?;
        if current_revision <= submitted_revision {
            self.remove_thread_content(room_id, root_event_id);
            self.thread_last_accepted_clear_revisions
                .entry(room_id.to_owned())
                .or_default()
                .insert(root_event_id.to_owned(), revision);
        }
        self.thread_revisions
            .entry(room_id.to_owned())
            .or_default()
            .insert(root_event_id.to_owned(), revision);
        if self
            .threads
            .get(room_id)
            .is_some_and(|threads| threads.contains_key(root_event_id))
        {
            self.remove_thread_from_lru(room_id, root_event_id);
        } else {
            self.touch_quiescent_thread(room_id, root_event_id);
        }
        Ok(revision)
    }

    pub fn clear_thread_draft(&mut self, room_id: &str, root_event_id: &str) {
        self.remove_thread_content(room_id, root_event_id);
        let should_remove_room = if let Some(room_threads) = self.thread_revisions.get_mut(room_id)
        {
            room_threads.remove(root_event_id);
            room_threads.is_empty()
        } else {
            false
        };
        if should_remove_room {
            self.thread_revisions.remove(room_id);
        }
        let should_remove_clear_room = if let Some(room_threads) =
            self.thread_last_accepted_clear_revisions.get_mut(room_id)
        {
            room_threads.remove(root_event_id);
            room_threads.is_empty()
        } else {
            false
        };
        if should_remove_clear_room {
            self.thread_last_accepted_clear_revisions.remove(room_id);
        }
        self.remove_thread_from_lru(room_id, root_event_id);
    }

    fn remove_thread_content(&mut self, room_id: &str, root_event_id: &str) {
        let should_remove_room = if let Some(room_threads) = self.threads.get_mut(room_id) {
            room_threads.remove(root_event_id);
            room_threads.is_empty()
        } else {
            false
        };
        if should_remove_room {
            self.threads.remove(room_id);
        }
    }

    pub fn reconcile_lifecycle(&mut self, protection: &ComposerDraftProtection) {
        self.quiescent_room_lru.retain(|room_id| {
            self.room_revisions.contains_key(room_id)
                && !self.rooms.contains_key(room_id)
                && !target_is_touch_protected(
                    protection,
                    &ComposerTarget::Main {
                        room_id: room_id.clone(),
                    },
                )
        });
        self.quiescent_thread_lru
            .retain(|(room_id, root_event_id)| {
                self.thread_revisions
                    .get(room_id)
                    .is_some_and(|revisions| revisions.contains_key(root_event_id))
                    && !self
                        .threads
                        .get(room_id)
                        .is_some_and(|threads| threads.contains_key(root_event_id))
                    && !target_is_touch_protected(
                        protection,
                        &ComposerTarget::Thread {
                            room_id: room_id.clone(),
                            root_event_id: root_event_id.clone(),
                        },
                    )
            });

        let missing_rooms = self
            .room_revisions
            .keys()
            .filter(|room_id| {
                !self.rooms.contains_key(*room_id)
                    && !self.quiescent_room_lru.contains(*room_id)
                    && !target_is_touch_protected(
                        protection,
                        &ComposerTarget::Main {
                            room_id: (*room_id).clone(),
                        },
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        self.quiescent_room_lru.extend(missing_rooms);

        let missing_threads = self
            .thread_revisions
            .iter()
            .flat_map(|(room_id, revisions)| {
                revisions
                    .keys()
                    .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
            })
            .filter(|(room_id, root_event_id)| {
                !self
                    .threads
                    .get(room_id)
                    .is_some_and(|threads| threads.contains_key(root_event_id))
                    && !self
                        .quiescent_thread_lru
                        .contains(&(room_id.clone(), root_event_id.clone()))
                    && !target_is_touch_protected(
                        protection,
                        &ComposerTarget::Thread {
                            room_id: room_id.clone(),
                            root_event_id: root_event_id.clone(),
                        },
                    )
            })
            .collect::<Vec<_>>();
        self.quiescent_thread_lru.extend(missing_threads);

        while self
            .quiescent_room_lru
            .iter()
            .filter(|room_id| {
                !target_is_protected(
                    protection,
                    &ComposerTarget::Main {
                        room_id: (*room_id).clone(),
                    },
                )
            })
            .count()
            > MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
        {
            let Some(index) = self.quiescent_room_lru.iter().position(|room_id| {
                !target_is_protected(
                    protection,
                    &ComposerTarget::Main {
                        room_id: room_id.clone(),
                    },
                )
            }) else {
                break;
            };
            let Some(room_id) = self.quiescent_room_lru.remove(index) else {
                break;
            };
            if !self.rooms.contains_key(&room_id) {
                self.room_revisions.remove(&room_id);
                self.room_last_accepted_clear_revisions.remove(&room_id);
            }
        }
        while self
            .quiescent_thread_lru
            .iter()
            .filter(|(room_id, root_event_id)| {
                !target_is_protected(
                    protection,
                    &ComposerTarget::Thread {
                        room_id: room_id.clone(),
                        root_event_id: root_event_id.clone(),
                    },
                )
            })
            .count()
            > MAX_LIVE_COMPOSER_THREAD_TOMBSTONES
        {
            let Some(index) =
                self.quiescent_thread_lru
                    .iter()
                    .position(|(room_id, root_event_id)| {
                        !target_is_protected(
                            protection,
                            &ComposerTarget::Thread {
                                room_id: room_id.clone(),
                                root_event_id: root_event_id.clone(),
                            },
                        )
                    })
            else {
                break;
            };
            let Some((room_id, root_event_id)) = self.quiescent_thread_lru.remove(index) else {
                break;
            };
            if !self
                .threads
                .get(&room_id)
                .is_some_and(|threads| threads.contains_key(&root_event_id))
            {
                remove_nested_entry(&mut self.thread_revisions, &room_id, &root_event_id);
                remove_nested_entry(
                    &mut self.thread_last_accepted_clear_revisions,
                    &room_id,
                    &root_event_id,
                );
            }
        }
    }

    pub fn quiescent_room_tombstone_count(&self) -> usize {
        self.room_revisions
            .keys()
            .filter(|room_id| !self.rooms.contains_key(*room_id))
            .count()
    }

    pub fn quiescent_thread_tombstone_count(&self) -> usize {
        self.thread_revisions
            .iter()
            .map(|(room_id, revisions)| {
                revisions
                    .keys()
                    .filter(|root_event_id| {
                        !self
                            .threads
                            .get(room_id)
                            .is_some_and(|threads| threads.contains_key(*root_event_id))
                    })
                    .count()
            })
            .sum()
    }

    fn touch_quiescent_room(&mut self, room_id: &str) {
        self.remove_room_from_lru(room_id);
        self.quiescent_room_lru.push_back(room_id.to_owned());
    }

    fn remove_room_from_lru(&mut self, room_id: &str) {
        self.quiescent_room_lru
            .retain(|candidate| candidate != room_id);
    }

    fn touch_quiescent_thread(&mut self, room_id: &str, root_event_id: &str) {
        self.remove_thread_from_lru(room_id, root_event_id);
        self.quiescent_thread_lru
            .push_back((room_id.to_owned(), root_event_id.to_owned()));
    }

    fn remove_thread_from_lru(&mut self, room_id: &str, root_event_id: &str) {
        self.quiescent_thread_lru
            .retain(|candidate| candidate != &(room_id.to_owned(), root_event_id.to_owned()));
    }

    pub fn retain_rooms(&mut self, room_ids: &BTreeSet<String>) {
        self.rooms.retain(|room_id, _| room_ids.contains(room_id));
        self.threads
            .retain(|room_id, room_threads| room_ids.contains(room_id) && !room_threads.is_empty());
        self.room_revisions
            .retain(|room_id, _| room_ids.contains(room_id));
        self.thread_revisions
            .retain(|room_id, revisions| room_ids.contains(room_id) && !revisions.is_empty());
        self.room_last_accepted_clear_revisions
            .retain(|room_id, _| room_ids.contains(room_id));
        self.thread_last_accepted_clear_revisions
            .retain(|room_id, revisions| room_ids.contains(room_id) && !revisions.is_empty());
        self.quiescent_room_lru
            .retain(|room_id| room_ids.contains(room_id));
        self.quiescent_thread_lru
            .retain(|(room_id, _)| room_ids.contains(room_id));
    }

    pub fn persisted_projection(
        &self,
        protection: &ComposerDraftProtection,
    ) -> ComposerDraftPersistenceProjection {
        let touch_protected_targets = protection
            .active
            .iter()
            .chain(&protection.leased)
            .cloned()
            .collect::<BTreeSet<_>>();
        let protected_rooms = touch_protected_targets
            .iter()
            .filter_map(|target| match target {
                ComposerTarget::Main { room_id } => Some(room_id.clone()),
                ComposerTarget::Thread { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let protected_threads = touch_protected_targets
            .iter()
            .filter_map(|target| match target {
                ComposerTarget::Main { .. } => None,
                ComposerTarget::Thread {
                    room_id,
                    root_event_id,
                } => Some((room_id.clone(), root_event_id.clone())),
            })
            .collect::<BTreeSet<_>>();
        let store_pending_rooms = protection
            .store_pending
            .iter()
            .filter_map(|target| match target {
                ComposerTarget::Main { room_id } => Some(room_id.clone()),
                ComposerTarget::Thread { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let store_pending_threads = protection
            .store_pending
            .iter()
            .filter_map(|target| match target {
                ComposerTarget::Main { .. } => None,
                ComposerTarget::Thread {
                    room_id,
                    root_event_id,
                } => Some((room_id.clone(), root_event_id.clone())),
            })
            .collect::<BTreeSet<_>>();

        let all_room_ids = self
            .rooms
            .keys()
            .chain(self.room_revisions.keys())
            .chain(self.room_last_accepted_clear_revisions.keys())
            .cloned()
            .chain(protected_rooms.iter().cloned())
            .chain(store_pending_rooms.iter().cloned())
            .collect::<BTreeSet<_>>();
        let room_entry = |room_id: &str| ComposerDraftPersistenceEntry {
            content: self
                .rooms
                .get(room_id)
                .filter(|content| !content.is_empty())
                .map(|content| {
                    content.truncated_to_plain_bytes(MAX_PERSISTED_COMPOSER_DRAFT_BYTES)
                }),
            revision: self.room_revision(room_id),
            last_accepted_clear_revision: self
                .room_last_accepted_clear_revisions
                .get(room_id)
                .copied()
                .unwrap_or_default(),
        };
        let room_has_content = |room_id: &str| {
            self.rooms
                .get(room_id)
                .is_some_and(|content| !content.is_empty())
        };
        let mut quiescent_room_order = Vec::new();
        let mut seen_rooms = BTreeSet::new();
        for room_id in &self.quiescent_room_lru {
            if all_room_ids.contains(room_id)
                && !room_has_content(room_id)
                && !protected_rooms.contains(room_id)
                && seen_rooms.insert(room_id.clone())
            {
                quiescent_room_order.push(room_id.clone());
            }
        }
        for room_id in &all_room_ids {
            if !room_has_content(room_id)
                && !protected_rooms.contains(room_id)
                && seen_rooms.insert(room_id.clone())
            {
                quiescent_room_order.push(room_id.clone());
            }
        }
        retain_newest_eligible(
            &mut quiescent_room_order,
            MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT,
            |room_id| store_pending_rooms.contains(room_id),
        );
        let protected_empty_rooms = protected_rooms
            .iter()
            .filter(|room_id| !room_has_content(room_id))
            .cloned()
            .collect::<Vec<_>>();
        let retained_rooms = self
            .rooms
            .iter()
            .filter(|&(_room_id, content)| !content.is_empty())
            .map(|(room_id, _content)| room_id.clone())
            .chain(quiescent_room_order.iter().cloned())
            .chain(protected_empty_rooms.iter().cloned())
            .collect::<BTreeSet<_>>();
        let rooms = retained_rooms
            .into_iter()
            .map(|room_id| {
                let entry = room_entry(&room_id);
                (room_id, entry)
            })
            .collect();

        let all_thread_targets = self
            .threads
            .iter()
            .flat_map(|(room_id, threads)| {
                threads
                    .keys()
                    .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
            })
            .chain(
                self.thread_revisions
                    .iter()
                    .flat_map(|(room_id, revisions)| {
                        revisions
                            .keys()
                            .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
                    }),
            )
            .chain(self.thread_last_accepted_clear_revisions.iter().flat_map(
                |(room_id, revisions)| {
                    revisions
                        .keys()
                        .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
                },
            ))
            .chain(protected_threads.iter().cloned())
            .chain(store_pending_threads.iter().cloned())
            .collect::<BTreeSet<_>>();
        let thread_has_content = |room_id: &str, root_event_id: &str| {
            self.threads
                .get(room_id)
                .and_then(|threads| threads.get(root_event_id))
                .is_some_and(|content| !content.is_empty())
        };
        let thread_entry = |room_id: &str, root_event_id: &str| ComposerDraftPersistenceEntry {
            content: self
                .threads
                .get(room_id)
                .and_then(|threads| threads.get(root_event_id))
                .filter(|content| !content.is_empty())
                .map(|content| {
                    content.truncated_to_plain_bytes(MAX_PERSISTED_COMPOSER_DRAFT_BYTES)
                }),
            revision: self.thread_revision(room_id, root_event_id),
            last_accepted_clear_revision: self
                .thread_last_accepted_clear_revisions
                .get(room_id)
                .and_then(|threads| threads.get(root_event_id))
                .copied()
                .unwrap_or_default(),
        };
        let mut quiescent_thread_order = Vec::new();
        let mut seen_threads = BTreeSet::new();
        for target @ (room_id, root_event_id) in &self.quiescent_thread_lru {
            if all_thread_targets.contains(target)
                && !thread_has_content(room_id, root_event_id)
                && !protected_threads.contains(target)
                && seen_threads.insert(target.clone())
            {
                quiescent_thread_order.push(target.clone());
            }
        }
        for target @ (room_id, root_event_id) in &all_thread_targets {
            if !thread_has_content(room_id, root_event_id)
                && !protected_threads.contains(target)
                && seen_threads.insert(target.clone())
            {
                quiescent_thread_order.push(target.clone());
            }
        }
        retain_newest_eligible(
            &mut quiescent_thread_order,
            MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT,
            |target| store_pending_threads.contains(target),
        );
        let protected_empty_threads = protected_threads
            .iter()
            .filter(|(room_id, root_event_id)| !thread_has_content(room_id, root_event_id))
            .cloned()
            .collect::<Vec<_>>();
        let retained_threads = self
            .threads
            .iter()
            .flat_map(|(room_id, threads)| {
                threads
                    .iter()
                    .filter(|(_, content)| !content.is_empty())
                    .map(|(root_event_id, _)| (room_id.clone(), root_event_id.clone()))
            })
            .chain(quiescent_thread_order.iter().cloned())
            .chain(protected_empty_threads.iter().cloned())
            .collect::<BTreeSet<_>>();
        let mut threads = BTreeMap::<String, BTreeMap<String, _>>::new();
        for (room_id, root_event_id) in retained_threads {
            threads.entry(room_id.clone()).or_default().insert(
                root_event_id.clone(),
                thread_entry(&room_id, &root_event_id),
            );
        }

        ComposerDraftPersistenceProjection {
            rooms,
            threads,
            quiescent_room_order,
            quiescent_thread_order,
            protected_empty_rooms,
            protected_empty_threads,
        }
    }

    pub fn from_persisted_projection(
        mut projection: ComposerDraftPersistenceProjection,
    ) -> Result<Self, ComposerDraftPersistenceImportError> {
        validate_persisted_projection(&projection)?;

        projection.protected_empty_rooms.sort();
        projection.protected_empty_threads.sort();
        projection
            .quiescent_room_order
            .extend(projection.protected_empty_rooms);
        projection
            .quiescent_thread_order
            .extend(projection.protected_empty_threads);
        retain_newest(
            &mut projection.quiescent_room_order,
            MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT,
        );
        retain_newest(
            &mut projection.quiescent_thread_order,
            MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT,
        );
        let retained_empty_rooms = projection
            .quiescent_room_order
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let retained_empty_threads = projection
            .quiescent_thread_order
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();

        let mut drafts = Self::default();
        for (room_id, entry) in projection.rooms {
            if let Some(content) = entry.content {
                drafts.rooms.insert(
                    room_id.clone(),
                    content.truncated_to_plain_bytes(MAX_PERSISTED_COMPOSER_DRAFT_BYTES),
                );
            } else if !retained_empty_rooms.contains(&room_id) {
                continue;
            }
            drafts
                .room_revisions
                .insert(room_id.clone(), entry.revision);
            if !entry.last_accepted_clear_revision.is_zero() {
                drafts
                    .room_last_accepted_clear_revisions
                    .insert(room_id, entry.last_accepted_clear_revision);
            }
        }
        for (room_id, room_threads) in projection.threads {
            for (root_event_id, entry) in room_threads {
                let target = (room_id.clone(), root_event_id.clone());
                if let Some(content) = entry.content {
                    drafts.threads.entry(room_id.clone()).or_default().insert(
                        root_event_id.clone(),
                        content.truncated_to_plain_bytes(MAX_PERSISTED_COMPOSER_DRAFT_BYTES),
                    );
                } else if !retained_empty_threads.contains(&target) {
                    continue;
                }
                drafts
                    .thread_revisions
                    .entry(room_id.clone())
                    .or_default()
                    .insert(root_event_id.clone(), entry.revision);
                if !entry.last_accepted_clear_revision.is_zero() {
                    drafts
                        .thread_last_accepted_clear_revisions
                        .entry(room_id.clone())
                        .or_default()
                        .insert(root_event_id, entry.last_accepted_clear_revision);
                }
            }
        }
        drafts.quiescent_room_lru = projection.quiescent_room_order.into();
        drafts.quiescent_thread_lru = projection.quiescent_thread_order.into();
        Ok(drafts)
    }
}

fn retain_newest<T>(items: &mut Vec<T>, maximum: usize) {
    if items.len() > maximum {
        items.drain(..items.len() - maximum);
    }
}

fn retain_newest_eligible<T>(
    items: &mut Vec<T>,
    maximum: usize,
    is_protected: impl Fn(&T) -> bool,
) {
    while items.iter().filter(|item| !is_protected(item)).count() > maximum {
        let Some(index) = items.iter().position(|item| !is_protected(item)) else {
            break;
        };
        items.remove(index);
    }
}

fn validate_persisted_projection(
    projection: &ComposerDraftPersistenceProjection,
) -> Result<(), ComposerDraftPersistenceImportError> {
    let invalid = || ComposerDraftPersistenceImportError::InvalidProjection;
    let empty_rooms = projection
        .rooms
        .iter()
        .filter_map(|(room_id, entry)| {
            if entry.last_accepted_clear_revision > entry.revision
                || entry
                    .content
                    .as_ref()
                    .is_some_and(ComposerDocument::is_empty)
            {
                return None;
            }
            entry.content.is_none().then(|| room_id.clone())
        })
        .collect::<BTreeSet<_>>();
    if projection.rooms.values().any(|entry| {
        entry.last_accepted_clear_revision > entry.revision
            || entry
                .content
                .as_ref()
                .is_some_and(ComposerDocument::is_empty)
    }) {
        return Err(invalid());
    }
    let quiescent_rooms = projection
        .quiescent_room_order
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let protected_rooms = projection
        .protected_empty_rooms
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if quiescent_rooms.len() != projection.quiescent_room_order.len()
        || protected_rooms.len() != projection.protected_empty_rooms.len()
        || !quiescent_rooms.is_disjoint(&protected_rooms)
        || quiescent_rooms
            .union(&protected_rooms)
            .cloned()
            .collect::<BTreeSet<_>>()
            != empty_rooms
    {
        return Err(invalid());
    }

    let mut empty_threads = BTreeSet::new();
    for (room_id, room_threads) in &projection.threads {
        if room_threads.is_empty() {
            return Err(invalid());
        }
        for (root_event_id, entry) in room_threads {
            if entry.last_accepted_clear_revision > entry.revision
                || entry
                    .content
                    .as_ref()
                    .is_some_and(ComposerDocument::is_empty)
            {
                return Err(invalid());
            }
            if entry.content.is_none() {
                empty_threads.insert((room_id.clone(), root_event_id.clone()));
            }
        }
    }
    let quiescent_threads = projection
        .quiescent_thread_order
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let protected_threads = projection
        .protected_empty_threads
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if quiescent_threads.len() != projection.quiescent_thread_order.len()
        || protected_threads.len() != projection.protected_empty_threads.len()
        || !quiescent_threads.is_disjoint(&protected_threads)
        || quiescent_threads
            .union(&protected_threads)
            .cloned()
            .collect::<BTreeSet<_>>()
            != empty_threads
    {
        return Err(invalid());
    }
    Ok(())
}

fn target_is_protected(protection: &ComposerDraftProtection, target: &ComposerTarget) -> bool {
    target_is_touch_protected(protection, target) || protection.store_pending.contains(target)
}

fn target_is_touch_protected(
    protection: &ComposerDraftProtection,
    target: &ComposerTarget,
) -> bool {
    protection.active.contains(target) || protection.leased.contains(target)
}

fn remove_nested_entry<T>(
    values: &mut std::collections::BTreeMap<String, std::collections::BTreeMap<String, T>>,
    room_id: &str,
    root_event_id: &str,
) {
    let remove_room = if let Some(room_values) = values.get_mut(room_id) {
        room_values.remove(root_event_id);
        room_values.is_empty()
    } else {
        false
    };
    if remove_room {
        values.remove(room_id);
    }
}

impl fmt::Debug for ComposerDraftStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let thread_count: usize = self
            .threads
            .values()
            .map(std::collections::BTreeMap::len)
            .sum();
        formatter
            .debug_struct("ComposerDraftStore")
            .field("rooms", &format_args!("{} room draft(s)", self.rooms.len()))
            .field("threads", &format_args!("{thread_count} thread draft(s)"))
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerState {
    #[serde(default)]
    pub accepted_submission_ids: VecDeque<SubmissionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_submission_id: Option<SubmissionId>,
    pub pending_transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_send_kind: Option<PendingComposerSendKind>,
    #[serde(default)]
    pub draft_revision: ComposerDraftRevision,
    #[serde(default)]
    pub last_accepted_clear_revision: ComposerDraftRevision,
    pub draft: String,
    #[serde(default)]
    pub document: ComposerDocument,
    pub mode: ComposerMode,
}

pub(crate) const MAX_ACCEPTED_SUBMISSION_TOMBSTONES: usize = 128;

impl ComposerState {
    pub(crate) fn remember_accepted_submission(&mut self, submission_id: SubmissionId) {
        while self.accepted_submission_ids.len() >= MAX_ACCEPTED_SUBMISSION_TOMBSTONES {
            self.accepted_submission_ids.pop_front();
        }
        self.accepted_submission_ids.push_back(submission_id);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PendingComposerSendKind {
    Plain,
    Reply { in_reply_to_event_id: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ComposerMode {
    #[default]
    Plain,
    Reply {
        in_reply_to_event_id: String,
    },
}

#[cfg(test)]
mod staged_upload_tests {
    use super::*;

    fn item(preparation: StagedUploadPreparation) -> StagedUploadItem {
        StagedUploadItem {
            staged_id: "s1".to_owned(),
            room_id: "!room:example.test".to_owned(),
            position: 0,
            filename: "fixture.png".to_owned(),
            mime_type: "image/png".to_owned(),
            byte_count: 10,
            kind: StagedUploadKind::File,
            caption: None,
            compression_choice: StagedUploadCompressionChoice::NotApplicable,
            preparation,
        }
    }

    fn ready(pending: Option<StagedUploadOutputSelection>) -> StagedUploadPreparation {
        StagedUploadPreparation::Ready {
            variants: Vec::new(),
            selected: StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Original,
                format: StagedUploadFormatChoice::Keep,
            },
            pending,
            generation: 0,
        }
    }

    #[test]
    fn staged_uploads_are_sendable_requires_ready_without_pending() {
        assert!(staged_uploads_are_sendable(&[item(ready(None))]));
        assert!(!staged_uploads_are_sendable(&[item(ready(Some(
            StagedUploadOutputSelection {
                resize: StagedUploadResizeChoice::Half,
                format: StagedUploadFormatChoice::Keep,
            }
        )))]));
        assert!(!staged_uploads_are_sendable(&[item(
            StagedUploadPreparation::Preparing
        )]));
        assert!(!staged_uploads_are_sendable(&[item(
            StagedUploadPreparation::Failed {
                failure_kind: MediaPreparationFailureKind::Empty,
                can_use_original: false,
            }
        )]));
        // One non-sendable item blocks the whole list.
        assert!(!staged_uploads_are_sendable(&[
            item(ready(None)),
            item(StagedUploadPreparation::Preparing)
        ]));
    }

    #[test]
    fn empty_staging_list_is_vacuously_sendable() {
        assert!(staged_uploads_are_sendable(&[]));
    }
}
