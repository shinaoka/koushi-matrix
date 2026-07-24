use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::composer_shortcuts::FormattedMessageDraft;
use crate::submission::{ComposerSubmissionTarget, ComposerTarget, SubmissionId};
use crate::{ComposerDraftRevision, ComposerDraftRevisionError};

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
    pub caption: Option<FormattedMessageDraft>,
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
        variants: Vec<PreparedUploadVariant>,
        selected_variant_id: String,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreparedUploadFormat {
    Original,
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreparedUploadVariant {
    pub variant_id: String,
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
        caption: Option<FormattedMessageDraft>,
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

    pub fn select_variant(
        &mut self,
        target: &ComposerTarget,
        staged_id: &str,
        variant_id: &str,
    ) -> Option<StagedUploadItem> {
        let item = self
            .items
            .get_mut(&(target.clone(), staged_id.to_owned()))?;
        let StagedUploadPreparation::Ready {
            variants,
            selected_variant_id,
        } = &mut item.preparation
        else {
            return None;
        };
        let selected = variants
            .iter()
            .find(|variant| variant.variant_id == variant_id)?
            .clone();
        *selected_variant_id = selected.variant_id;
        item.filename = selected.filename;
        item.mime_type = selected.mime_type;
        item.byte_count = selected.byte_count;
        item.kind = StagedUploadKind::Image {
            width: selected.width,
            height: selected.height,
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
        send_at_ms: u64,
        handle: ScheduledSendHandle,
    ) -> Option<ScheduledSendItem> {
        let item = self.items.get_mut(scheduled_id)?;
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

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerDraftStore {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub rooms: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub threads: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
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
        if let Some(draft) = self.rooms.get(room_id) {
            composer.draft = draft.clone();
        }
        composer.draft_revision = self.room_revision(room_id);
        composer.last_accepted_clear_revision = self
            .room_last_accepted_clear_revisions
            .get(room_id)
            .copied()
            .unwrap_or_default();
        composer
    }

    pub fn set_room_draft(&mut self, room_id: String, draft: String) {
        let Ok(revision) = ComposerDraftRevision::checked_successor(
            self.room_revision(&room_id),
            ComposerDraftRevision::ZERO,
        ) else {
            return;
        };
        let _ = self.apply_room_draft(room_id, draft, revision);
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
        draft: String,
        revision: ComposerDraftRevision,
    ) -> Result<bool, ComposerDraftRevisionError> {
        if revision <= self.room_revision(&room_id) {
            return Ok(false);
        }
        self.room_revisions.insert(room_id.clone(), revision);
        if draft.is_empty() {
            self.rooms.remove(&room_id);
            self.touch_quiescent_room(&room_id);
        } else {
            self.rooms.insert(room_id.clone(), draft);
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

    pub fn clear_room_draft(&mut self, room_id: &str) {
        self.rooms.remove(room_id);
        self.room_revisions.remove(room_id);
        self.room_last_accepted_clear_revisions.remove(room_id);
        self.remove_room_from_lru(room_id);
    }

    pub fn composer_for_thread(&self, room_id: &str, root_event_id: &str) -> ComposerState {
        let mut composer = ComposerState::default();
        if let Some(draft) = self
            .threads
            .get(room_id)
            .and_then(|room_threads| room_threads.get(root_event_id))
        {
            composer.draft = draft.clone();
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

    pub fn set_thread_draft(&mut self, room_id: String, root_event_id: String, draft: String) {
        let Ok(revision) = ComposerDraftRevision::checked_successor(
            self.thread_revision(&room_id, &root_event_id),
            ComposerDraftRevision::ZERO,
        ) else {
            return;
        };
        let _ = self.apply_thread_draft(room_id, root_event_id, draft, revision);
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
        draft: String,
        revision: ComposerDraftRevision,
    ) -> Result<bool, ComposerDraftRevisionError> {
        if revision <= self.thread_revision(&room_id, &root_event_id) {
            return Ok(false);
        }
        self.thread_revisions
            .entry(room_id.clone())
            .or_default()
            .insert(root_event_id.clone(), revision);
        if draft.is_empty() {
            self.remove_thread_content(&room_id, &root_event_id);
            self.touch_quiescent_thread(&room_id, &root_event_id);
        } else {
            self.threads
                .entry(room_id.clone())
                .or_default()
                .insert(root_event_id.clone(), draft);
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
                && !target_is_protected(
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
                    && !target_is_protected(
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
                    && !target_is_protected(
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
                    && !target_is_protected(
                        protection,
                        &ComposerTarget::Thread {
                            room_id: room_id.clone(),
                            root_event_id: root_event_id.clone(),
                        },
                    )
            })
            .collect::<Vec<_>>();
        self.quiescent_thread_lru.extend(missing_threads);

        while self.quiescent_room_lru.len() > MAX_LIVE_COMPOSER_ROOM_TOMBSTONES {
            let Some(room_id) = self.quiescent_room_lru.pop_front() else {
                break;
            };
            let target = ComposerTarget::Main {
                room_id: room_id.clone(),
            };
            if !self.rooms.contains_key(&room_id) && !target_is_protected(protection, &target) {
                self.room_revisions.remove(&room_id);
                self.room_last_accepted_clear_revisions.remove(&room_id);
            }
        }
        while self.quiescent_thread_lru.len() > MAX_LIVE_COMPOSER_THREAD_TOMBSTONES {
            let Some((room_id, root_event_id)) = self.quiescent_thread_lru.pop_front() else {
                break;
            };
            let target = ComposerTarget::Thread {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            };
            if !self
                .threads
                .get(&room_id)
                .is_some_and(|threads| threads.contains_key(&root_event_id))
                && !target_is_protected(protection, &target)
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

    pub fn bounded_for_persistence(&self) -> Self {
        let mut room_ids = self
            .rooms
            .keys()
            .cloned()
            .take(MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT)
            .collect::<Vec<_>>();
        if room_ids.len() < MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT {
            let content_room_ids = room_ids.iter().cloned().collect::<BTreeSet<_>>();
            room_ids.extend(
                self.room_revisions
                    .keys()
                    .filter(|room_id| !content_room_ids.contains(*room_id))
                    .take(MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT - room_ids.len())
                    .cloned(),
            );
        }
        let rooms = room_ids
            .iter()
            .filter_map(|room_id| {
                self.rooms.get(room_id).map(|draft| {
                    (
                        room_id.clone(),
                        truncate_utf8_bytes(draft, MAX_PERSISTED_COMPOSER_DRAFT_BYTES),
                    )
                })
            })
            .collect();
        let room_revisions: std::collections::BTreeMap<_, _> = room_ids
            .iter()
            .filter_map(|room_id| {
                self.room_revisions
                    .get(room_id)
                    .map(|revision| (room_id.clone(), *revision))
            })
            .collect();

        let mut thread_targets = self
            .threads
            .iter()
            .flat_map(|(room_id, room_threads)| {
                room_threads
                    .keys()
                    .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
            })
            .take(MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT)
            .collect::<Vec<_>>();
        if thread_targets.len() < MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT {
            let content_thread_targets = thread_targets.iter().cloned().collect::<BTreeSet<_>>();
            thread_targets.extend(
                self.thread_revisions
                    .iter()
                    .flat_map(|(room_id, room_threads)| {
                        room_threads
                            .keys()
                            .map(|root_event_id| (room_id.clone(), root_event_id.clone()))
                    })
                    .filter(|target| !content_thread_targets.contains(target))
                    .take(MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT - thread_targets.len()),
            );
        }
        let mut threads = std::collections::BTreeMap::new();
        let mut thread_revisions: std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<String, ComposerDraftRevision>,
        > = std::collections::BTreeMap::new();
        for (room_id, root_event_id) in thread_targets {
            if let Some(draft) = self
                .threads
                .get(&room_id)
                .and_then(|room_threads| room_threads.get(&root_event_id))
            {
                threads
                    .entry(room_id.clone())
                    .or_insert_with(std::collections::BTreeMap::new)
                    .insert(
                        root_event_id.clone(),
                        truncate_utf8_bytes(draft, MAX_PERSISTED_COMPOSER_DRAFT_BYTES),
                    );
            }
            if let Some(revision) = self
                .thread_revisions
                .get(&room_id)
                .and_then(|room_threads| room_threads.get(&root_event_id))
            {
                thread_revisions
                    .entry(room_id)
                    .or_insert_with(std::collections::BTreeMap::new)
                    .insert(root_event_id, *revision);
            }
        }

        let quiescent_room_lru = self
            .quiescent_room_lru
            .iter()
            .filter(|room_id| room_revisions.contains_key(*room_id))
            .cloned()
            .collect();
        let quiescent_thread_lru = self
            .quiescent_thread_lru
            .iter()
            .filter(|(room_id, root_event_id)| {
                thread_revisions
                    .get(room_id)
                    .is_some_and(|revisions| revisions.contains_key(root_event_id))
            })
            .cloned()
            .collect();

        Self {
            rooms,
            threads,
            room_revisions,
            thread_revisions,
            room_last_accepted_clear_revisions: self
                .room_last_accepted_clear_revisions
                .iter()
                .filter(|(room_id, _)| room_ids.contains(room_id))
                .map(|(room_id, revision)| (room_id.clone(), *revision))
                .collect(),
            thread_last_accepted_clear_revisions: self
                .thread_last_accepted_clear_revisions
                .iter()
                .filter_map(|(room_id, revisions)| {
                    let retained = revisions
                        .iter()
                        .filter(|(root_event_id, _)| {
                            self.thread_revisions
                                .get(room_id)
                                .is_some_and(|known| known.contains_key(*root_event_id))
                        })
                        .map(|(root_event_id, revision)| (root_event_id.clone(), *revision))
                        .collect::<std::collections::BTreeMap<_, _>>();
                    (!retained.is_empty()).then(|| (room_id.clone(), retained))
                })
                .collect(),
            quiescent_room_lru,
            quiescent_thread_lru,
        }
    }
}

fn target_is_protected(protection: &ComposerDraftProtection, target: &ComposerTarget) -> bool {
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

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
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
