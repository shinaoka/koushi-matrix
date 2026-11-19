//! #1969: the room media browser.
//!
//! The gallery used to be derived only from the main timeline's resident
//! window, so media older than the loaded messages never appeared and the
//! image viewer's previous/next stopped there. Like Element X's "Media and
//! files", the room's main `TimelineActor` lazily starts a second, media-only
//! SDK timeline (live focus plus a message-type `event_filter`) and pages it
//! backwards on demand. The SDK filter only gates new items; redactions and
//! edits still aggregate onto existing rows.
//!
//! The browser task owns that SDK timeline, its diff stream, and the projected
//! accumulator advanced only by that stream. It posts rows and page outcomes
//! back to the actor, which owns the `MediaGalleryPagination` state machine
//! and is the sole publisher of `MediaGalleryUpdated`.

use std::collections::HashMap;
use std::sync::Arc;

use eyeball_im::Vector;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use koushi_state::{MediaGalleryPagination, TimelineMediaGalleryItem};
use matrix_sdk::ruma::events::room::message::MessageType;
use matrix_sdk::ruma::events::{AnyMessageLikeEventContent, AnySyncTimelineEvent};
use matrix_sdk::ruma::room_version_rules::RoomVersionRules;
use matrix_sdk_ui::timeline::{
    TimelineFocus, TimelineItem as SdkTimelineItem, TimelineItemKind, default_event_filter,
};
use tokio::sync::mpsc;

use crate::executor;
use koushi_protocol::ids::TimelineKey;

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorMessage};
use super::item_projection::{private_media_entry_from_content, sdk_item_to_timeline_item};
use super::media::{PrivateMediaEntry, media_gallery_item_from_timeline_item};
use super::relay::{koushi_timeline_builder, koushi_timeline_subscribe};
// END GENERATED SIBLING IMPORTS

/// Events requested from the shared room event cache per page. A page may hold
/// no media at all; the GUI asks again while the gallery end stays in view.
pub(super) const MEDIA_BROWSER_PAGE_EVENTS: u16 = 100;

/// A projected media row: the public gallery DTO plus its private source,
/// which may hold encrypted media keys and must never be logged.
#[derive(Clone)]
pub(super) struct MediaBrowserRow {
    pub(super) item: TimelineMediaGalleryItem,
    pub(super) source: PrivateMediaEntry,
}

/// What the browser task reports to its actor.
pub(super) enum MediaBrowserUpdate {
    /// The full current set of media rows, oldest first.
    Rows(Vec<MediaBrowserRow>),
    PageFinished {
        reached_start: bool,
    },
    PageFailed,
    /// The browser could not start, or its SDK stream ended.
    Stopped,
}

impl std::fmt::Debug for MediaBrowserUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rows(rows) => write!(formatter, "Rows({} media row(s))", rows.len()),
            Self::PageFinished { reached_start } => formatter
                .debug_struct("PageFinished")
                .field("reached_start", reached_start)
                .finish(),
            Self::PageFailed => formatter.write_str("PageFailed"),
            Self::Stopped => formatter.write_str("Stopped"),
        }
    }
}

/// The actor-side handle of a running media browser.
pub(super) struct MediaBrowser {
    pub(super) serial: u64,
    paginate_tx: mpsc::Sender<()>,
    task: executor::JoinHandle<()>,
    pub(super) rows: Vec<MediaBrowserRow>,
}

impl Drop for MediaBrowser {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MediaBrowser {
    pub(super) fn source(&self, event_id: &str) -> Option<&PrivateMediaEntry> {
        self.rows
            .iter()
            .find(|row| row.item.event_id == event_id)
            .map(|row| &row.source)
    }
}

/// Element X's media filter: image, video, file and audio room messages.
/// An event only reaches this filter once the shared event cache has decrypted
/// it, so undecryptable media is dropped here; a late decryption re-adds an
/// event only when it was already inside this timeline's window. Media that
/// arrived undecryptable outside the window stays absent until the browser
/// restarts, which is Element X's behaviour as well.
fn media_event_filter(event: &AnySyncTimelineEvent, rules: &RoomVersionRules) -> bool {
    if !default_event_filter(event, rules) {
        return false;
    }
    let AnySyncTimelineEvent::MessageLike(message) = event else {
        return false;
    };
    matches!(
        message.original_content(),
        Some(AnyMessageLikeEventContent::RoomMessage(content))
            if matches!(
                content.msgtype,
                MessageType::Image(_)
                    | MessageType::Video(_)
                    | MessageType::File(_)
                    | MessageType::Audio(_)
            )
    )
}

/// Project one media-timeline item; anything without displayable media
/// (date dividers, undecryptable or redacted events) is `None`.
pub(super) fn media_browser_row(
    key: &TimelineKey,
    room_id: &str,
    item: &Arc<SdkTimelineItem>,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
) -> Option<MediaBrowserRow> {
    let TimelineItemKind::Event(event_item) = item.kind() else {
        return None;
    };
    let source = private_media_entry_from_content(event_item.content())?;
    let projected = sdk_item_to_timeline_item(key, item, own_user_id);
    let item = media_gallery_item_from_timeline_item(room_id, &projected)?;
    Some(MediaBrowserRow { item, source })
}

fn present_rows(rows: &Vector<Option<MediaBrowserRow>>) -> Vec<MediaBrowserRow> {
    rows.iter().flatten().cloned().collect()
}

async fn post(
    actor_tx: &mpsc::Sender<TimelineActorMessage>,
    serial: u64,
    update: MediaBrowserUpdate,
) -> bool {
    actor_tx
        .send(TimelineActorMessage::MediaBrowserUpdate { serial, update })
        .await
        .is_ok()
}

async fn run_media_browser(
    room: matrix_sdk::Room,
    key: TimelineKey,
    own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
    serial: u64,
    actor_tx: mpsc::Sender<TimelineActorMessage>,
    mut paginate_rx: mpsc::Receiver<()>,
) {
    let room_id = key.room_id().to_owned();
    let timeline = match koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: false,
        },
    )
    .event_filter(media_event_filter)
    .build()
    .await
    {
        Ok(timeline) => Arc::new(timeline),
        Err(_) => {
            post(&actor_tx, serial, MediaBrowserUpdate::Stopped).await;
            return;
        }
    };
    let (initial, stream) = koushi_timeline_subscribe(timeline.clone()).await;
    let own = own_user_id.as_deref();
    let mut rows: Vector<Option<MediaBrowserRow>> = initial
        .iter()
        .map(|item| media_browser_row(&key, &room_id, item, own))
        .collect();
    if !post(
        &actor_tx,
        serial,
        MediaBrowserUpdate::Rows(present_rows(&rows)),
    )
    .await
    {
        return;
    }

    let mut stream = std::pin::pin!(stream);
    let mut page: Option<BoxFuture<'static, Result<bool, ()>>> = None;
    loop {
        let update = tokio::select! {
            batch = stream.next() => {
                let Some(batch) = batch else {
                    post(&actor_tx, serial, MediaBrowserUpdate::Stopped).await;
                    return;
                };
                for diff in batch {
                    diff.map(|item| media_browser_row(&key, &room_id, &item, own))
                        .apply(&mut rows);
                }
                MediaBrowserUpdate::Rows(present_rows(&rows))
            }
            request = paginate_rx.recv(), if page.is_none() => {
                if request.is_none() {
                    return;
                }
                let timeline = timeline.clone();
                page = Some(Box::pin(async move {
                    timeline
                        .paginate_backwards(MEDIA_BROWSER_PAGE_EVENTS)
                        .await
                        .map_err(|_| ())
                }));
                continue;
            }
            outcome = async { page.as_mut().expect("guarded by is_some").await }, if page.is_some() => {
                page = None;
                match outcome {
                    Ok(reached_start) => MediaBrowserUpdate::PageFinished { reached_start },
                    Err(()) => MediaBrowserUpdate::PageFailed,
                }
            }
        };
        if !post(&actor_tx, serial, update).await {
            return;
        }
    }
}

/// The next state after a `PaginateMediaGallery` request, and whether a page
/// must be requested from the browser. `None` means the request is absorbed:
/// a page is already in flight, or the room start was reached.
pub(super) fn media_gallery_pagination_on_request(
    current: MediaGalleryPagination,
) -> Option<MediaGalleryPagination> {
    match current {
        MediaGalleryPagination::Idle | MediaGalleryPagination::Failed => {
            Some(MediaGalleryPagination::Loading)
        }
        MediaGalleryPagination::Loading | MediaGalleryPagination::ReachedStart => None,
    }
}

/// The state after a browser update; `None` leaves it unchanged.
pub(super) fn media_gallery_pagination_on_update(
    current: MediaGalleryPagination,
    update: &MediaBrowserUpdate,
) -> Option<MediaGalleryPagination> {
    match update {
        MediaBrowserUpdate::Rows(_) => None,
        MediaBrowserUpdate::PageFinished {
            reached_start: true,
        } => Some(MediaGalleryPagination::ReachedStart),
        MediaBrowserUpdate::PageFinished {
            reached_start: false,
        } => Some(MediaGalleryPagination::Idle),
        MediaBrowserUpdate::PageFailed => Some(MediaGalleryPagination::Failed),
        MediaBrowserUpdate::Stopped => match current {
            MediaGalleryPagination::Loading => Some(MediaGalleryPagination::Failed),
            _ => None,
        },
    }
}

impl TimelineActor {
    pub(super) async fn handle_paginate_media_gallery(&mut self) {
        if !matches!(
            self.key.kind,
            koushi_protocol::ids::TimelineKind::Room { .. }
        ) {
            return;
        }
        let Some(next) = media_gallery_pagination_on_request(self.media_gallery_pagination) else {
            return;
        };
        if self.media_browser.is_none() {
            let Some(room) = self.sdk_room_for_key() else {
                self.media_gallery_pagination = MediaGalleryPagination::Failed;
                self.emit_media_gallery_if_changed().await;
                return;
            };
            self.next_media_browser_serial += 1;
            let serial = self.next_media_browser_serial;
            let (paginate_tx, paginate_rx) = mpsc::channel(1);
            let task = executor::spawn(run_media_browser(
                room,
                self.key.clone(),
                self.own_user_id.clone(),
                serial,
                self.msg_tx.clone(),
                paginate_rx,
            ));
            self.media_browser = Some(MediaBrowser {
                serial,
                paginate_tx,
                task,
                rows: Vec::new(),
            });
        }
        let browser = self.media_browser.as_ref().expect("started above");
        // The state machine admits one page at a time, so the capacity-one
        // channel is always empty here; a send can only fail once the task
        // has exited, which its `Stopped` report settles.
        if browser.paginate_tx.send(()).await.is_err() {
            self.media_browser = None;
            self.media_gallery_pagination = MediaGalleryPagination::Failed;
        } else {
            self.media_gallery_pagination = next;
        }
        self.emit_media_gallery_if_changed().await;
    }

    pub(super) async fn handle_media_browser_update(
        &mut self,
        serial: u64,
        update: MediaBrowserUpdate,
    ) {
        if self.media_browser.as_ref().map(|browser| browser.serial) != Some(serial) {
            return;
        }
        if let Some(next) =
            media_gallery_pagination_on_update(self.media_gallery_pagination, &update)
        {
            self.media_gallery_pagination = next;
        }
        match update {
            MediaBrowserUpdate::Rows(rows) => {
                if let Some(browser) = self.media_browser.as_mut() {
                    browser.rows = rows;
                }
            }
            MediaBrowserUpdate::Stopped => {
                self.media_browser = None;
            }
            MediaBrowserUpdate::PageFinished { .. } | MediaBrowserUpdate::PageFailed => {}
        }
        self.emit_media_gallery_if_changed().await;
    }

    /// Private source for a gallery item outside the resident window.
    pub(super) fn media_browser_source(&self, event_id: &str) -> Option<PrivateMediaEntry> {
        self.media_browser
            .as_ref()
            .and_then(|browser| browser.source(event_id))
            .cloned()
    }

    /// Browser rows not already present in the window-derived gallery.
    pub(super) fn media_browser_gallery_items(&self) -> Vec<TimelineMediaGalleryItem> {
        self.media_browser
            .as_ref()
            .map(|browser| browser.rows.iter().map(|row| row.item.clone()).collect())
            .unwrap_or_default()
    }
}

/// Union of window-derived and browser gallery items by event id, newest
/// first. A browser row wins, since it is not bounded by the window.
pub(super) fn merge_media_gallery_items(
    window_items: Vec<TimelineMediaGalleryItem>,
    browser_items: Vec<TimelineMediaGalleryItem>,
) -> Vec<TimelineMediaGalleryItem> {
    if browser_items.is_empty() {
        return window_items;
    }
    let mut by_event: HashMap<String, TimelineMediaGalleryItem> = window_items
        .into_iter()
        .map(|item| (item.event_id.clone(), item))
        .collect();
    for item in browser_items {
        by_event.insert(item.event_id.clone(), item);
    }
    let mut items = by_event.into_values().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    items
}

#[cfg(test)]
mod tests;
