use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::composer_shortcuts::FormattedMessageDraft;
use crate::submission::{ComposerSubmissionTarget, SubmissionId};

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

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UploadStagingStore {
    pub items: std::collections::BTreeMap<String, StagedUploadItem>,
}

impl UploadStagingStore {
    pub fn items_for_room(&self, room_id: &str) -> Vec<StagedUploadItem> {
        let mut items = self
            .items
            .values()
            .filter(|item| item.room_id == room_id)
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.staged_id.cmp(&right.staged_id))
        });
        items
    }

    pub fn replace_room_items(&mut self, room_id: &str, items: Vec<StagedUploadItem>) {
        self.items.retain(|_, item| item.room_id != room_id);
        for item in items.into_iter().filter(|item| item.room_id == room_id) {
            self.items.insert(item.staged_id.clone(), item);
        }
    }

    pub fn update_caption(
        &mut self,
        staged_id: &str,
        caption: Option<FormattedMessageDraft>,
    ) -> Option<StagedUploadItem> {
        let item = self.items.get_mut(staged_id)?;
        item.caption = caption;
        Some(item.clone())
    }

    pub fn update_compression_choice(
        &mut self,
        staged_id: &str,
        compression_choice: StagedUploadCompressionChoice,
    ) -> Option<StagedUploadItem> {
        let item = self.items.get_mut(staged_id)?;
        item.compression_choice = compression_choice;
        Some(item.clone())
    }

    pub fn clear_room(&mut self, room_id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|_, item| item.room_id != room_id);
        self.items.len() != before
    }

    pub fn retain_rooms(&mut self, room_ids: &BTreeSet<String>) {
        self.items
            .retain(|_, item| room_ids.contains(item.room_id.as_str()));
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
}

pub const MAX_PERSISTED_COMPOSER_DRAFT_BYTES: usize = 16 * 1024;
pub const MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT: usize = 128;
pub const MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT: usize = 256;

impl ComposerDraftStore {
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty() && self.threads.is_empty()
    }

    pub fn composer_for_room(&self, room_id: &str) -> ComposerState {
        let mut composer = ComposerState::default();
        if let Some(draft) = self.rooms.get(room_id) {
            composer.draft = draft.clone();
        }
        composer
    }

    pub fn set_room_draft(&mut self, room_id: String, draft: String) {
        if draft.is_empty() {
            self.rooms.remove(&room_id);
        } else {
            self.rooms.insert(room_id, draft);
        }
    }

    pub fn clear_room_draft(&mut self, room_id: &str) {
        self.rooms.remove(room_id);
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
        composer
    }

    pub fn set_thread_draft(&mut self, room_id: String, root_event_id: String, draft: String) {
        if draft.is_empty() {
            self.clear_thread_draft(&room_id, &root_event_id);
            return;
        }

        self.threads
            .entry(room_id)
            .or_default()
            .insert(root_event_id, draft);
    }

    pub fn clear_thread_draft(&mut self, room_id: &str, root_event_id: &str) {
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

    pub fn retain_rooms(&mut self, room_ids: &BTreeSet<String>) {
        self.rooms.retain(|room_id, _| room_ids.contains(room_id));
        self.threads
            .retain(|room_id, room_threads| room_ids.contains(room_id) && !room_threads.is_empty());
    }

    pub fn bounded_for_persistence(&self) -> Self {
        let rooms = self
            .rooms
            .iter()
            .take(MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT)
            .map(|(room_id, draft)| {
                (
                    room_id.clone(),
                    truncate_utf8_bytes(draft, MAX_PERSISTED_COMPOSER_DRAFT_BYTES),
                )
            })
            .collect();

        let mut remaining_threads = MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT;
        let mut threads = std::collections::BTreeMap::new();
        for (room_id, room_threads) in &self.threads {
            if remaining_threads == 0 {
                break;
            }
            let mut bounded_room_threads = std::collections::BTreeMap::new();
            for (root_event_id, draft) in room_threads.iter().take(remaining_threads) {
                bounded_room_threads.insert(
                    root_event_id.clone(),
                    truncate_utf8_bytes(draft, MAX_PERSISTED_COMPOSER_DRAFT_BYTES),
                );
            }
            remaining_threads = remaining_threads.saturating_sub(bounded_room_threads.len());
            if !bounded_room_threads.is_empty() {
                threads.insert(room_id.clone(), bounded_room_threads);
            }
        }

        Self { rooms, threads }
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
