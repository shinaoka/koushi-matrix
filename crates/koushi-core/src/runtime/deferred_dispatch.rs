//! #1060: AppActor → AccountActor dispatches produced while committing an
//! action batch must never wait for AccountActor mailbox capacity. An awaited
//! send there holds the AppActor loop, so every later command, including a
//! purely local room selection, queues behind a full mailbox.
//!
//! Only latest-wins or generation-guarded dispatches are deferred here, where
//! a late or coalesced delivery cannot change the outcome. Each kind keeps at
//! most one held value, and every dispatch of a kind goes through this module
//! so a held value can never be delivered after a newer one:
//!
//! - Activity resolution (`ResolveActivity` / `CancelActivityResolution`):
//!   one latest-wins slot. `Resolving` is published before dispatch, so live
//!   updates never restart a generation, and every settlement is fenced by
//!   generation. A cancel supersedes a held resolve and a resolve supersedes
//!   a held cancel (the AccountActor aborts a running task on either). A held
//!   resolve whose generation is no longer current is dropped.
//! - A Space-children reload (`LoadSpaceChildren`, from a live leave): the
//!   reducer already put the slice in `Loading` under its generation, which
//!   fences every result. A newer reload supersedes a held one, and one whose
//!   Space or generation is no longer current is dropped.
//! - The search-crawler lane (`RebuildSearchIndex`,
//!   `InvalidateSearchCrawlerCache`, `NotifySearchCrawlerRoomsAvailable`):
//!   ordered. While anything is held, later lane messages join the held lane
//!   instead of overtaking it. Rebuild and invalidate are idempotent flags
//!   delivered before the notification. A rebuild changes no settings, so a
//!   held notification stays and follows it. An invalidate (a caption or
//!   filename opt-out) supersedes a held earlier notification, because its
//!   reducer also emits the follow-up notification with the new settings. A
//!   newer notification replaces a held one. The lane is fenced to the
//!   session that produced it.
//!
//! - Settings policies: at most three latest-value messages (read receipts,
//!   display, link previews), fenced to the session. A newer policy replaces
//!   any undelivered values; loading settings must not stall local navigation.
//!
//! The run loop waits for one mailbox slot only while something is deferred,
//! so free capacity can never spin it.

use koushi_protocol::SessionKeyId;
use koushi_protocol::command::RoomCommand;
use koushi_protocol::ids::RequestId;
use koushi_state::{
    ActivityResolutionState, ActivityState, AppAction, OperationFailureKind, SpaceChildrenLoadState,
};
use tokio::sync::mpsc;

use super::{ActionBatchOrigin, AppActor};
use crate::account::AccountMessage;
use crate::activity_resolution::ActivityResolutionRequest;

/// The held Activity-resolution dispatch.
pub(super) enum DeferredActivity {
    /// A started generation whose `ResolveActivity` waits for capacity.
    Resolve {
        generation: u64,
        unresolved_room_count: u32,
        requests: Vec<ActivityResolutionRequest>,
    },
    /// Activity closed; any running resolution must stop.
    Cancel,
}

/// A reducer-admitted Space-children reload waiting for capacity.
pub(super) struct DeferredSpaceChildrenReload {
    request_id: RequestId,
    space_id: String,
    generation: u64,
}

/// Search-crawler room availability for the crawler.
pub(super) struct CrawlerRooms {
    pub(super) room_ids: Vec<String>,
    pub(super) latest_event_ids: std::collections::BTreeMap<String, String>,
    pub(super) settings: koushi_state::SearchCrawlerSettings,
}

/// One search-crawler lane message.
pub(super) enum CrawlerDispatch {
    Rebuild,
    Invalidate,
    Notify(CrawlerRooms),
}

/// The held search-crawler lane, delivered rebuild → invalidate → notify.
struct DeferredCrawlerLane {
    session_key: Option<SessionKeyId>,
    rebuild: bool,
    invalidate: bool,
    notify: Option<CrawlerRooms>,
}

impl DeferredCrawlerLane {
    fn new(session_key: Option<SessionKeyId>) -> Self {
        Self {
            session_key,
            rebuild: false,
            invalidate: false,
            notify: None,
        }
    }

    fn push(&mut self, dispatch: CrawlerDispatch) {
        match dispatch {
            // A rebuild changes no crawler settings: a held notification is
            // still the newest and follows the rebuild (#1060 review).
            CrawlerDispatch::Rebuild => self.rebuild = true,
            CrawlerDispatch::Invalidate => {
                self.invalidate = true;
                self.notify = None;
            }
            CrawlerDispatch::Notify(rooms) => self.notify = Some(rooms),
        }
    }

    fn is_empty(&self) -> bool {
        !self.rebuild && !self.invalidate && self.notify.is_none()
    }

    /// The next message in lane order.
    fn pop(&mut self) -> Option<AccountMessage> {
        if std::mem::take(&mut self.rebuild) {
            return Some(AccountMessage::RebuildSearchIndex);
        }
        if std::mem::take(&mut self.invalidate) {
            return Some(AccountMessage::InvalidateSearchCrawlerCache);
        }
        self.notify.take().map(crawler_message_of_rooms)
    }
}

fn crawler_message(dispatch: CrawlerDispatch) -> AccountMessage {
    match dispatch {
        CrawlerDispatch::Rebuild => AccountMessage::RebuildSearchIndex,
        CrawlerDispatch::Invalidate => AccountMessage::InvalidateSearchCrawlerCache,
        CrawlerDispatch::Notify(rooms) => crawler_message_of_rooms(rooms),
    }
}

fn crawler_message_of_rooms(rooms: CrawlerRooms) -> AccountMessage {
    AccountMessage::NotifySearchCrawlerRoomsAvailable {
        room_ids: rooms.room_ids,
        latest_event_ids: rooms.latest_event_ids,
        settings: rooms.settings,
    }
}

fn crawler_dispatch_of_message(message: AccountMessage) -> CrawlerDispatch {
    match message {
        AccountMessage::RebuildSearchIndex => CrawlerDispatch::Rebuild,
        AccountMessage::InvalidateSearchCrawlerCache => CrawlerDispatch::Invalidate,
        AccountMessage::NotifySearchCrawlerRoomsAvailable {
            room_ids,
            latest_event_ids,
            settings,
        } => CrawlerDispatch::Notify(CrawlerRooms {
            room_ids,
            latest_event_ids,
            settings,
        }),
        _ => unreachable!("try_send returns the crawler-lane message it was given"),
    }
}

#[derive(Clone)]
enum SettingsPolicyMessage {
    ReadReceipts(bool),
    Display {
        thread_root_order: koushi_state::TimelineThreadRootOrder,
        hide_redacted: bool,
    },
    LinkPreviews {
        unencrypted: bool,
        encrypted: bool,
        overrides: std::collections::BTreeMap<String, bool>,
    },
}

impl SettingsPolicyMessage {
    fn into_message(self) -> AccountMessage {
        match self {
            Self::ReadReceipts(send_read_receipts) => {
                AccountMessage::ReadStatePolicyChanged { send_read_receipts }
            }
            Self::Display {
                thread_root_order,
                hide_redacted,
            } => AccountMessage::DisplayPolicyChanged {
                thread_root_order,
                hide_redacted,
            },
            Self::LinkPreviews {
                unencrypted,
                encrypted,
                overrides,
            } => AccountMessage::TimelineCommand(
                koushi_protocol::command::TimelineCommand::BroadcastLinkPreviewPolicy {
                    unencrypted_global_enabled: unencrypted,
                    encrypted_global_enabled: encrypted,
                    room_overrides: overrides,
                },
            ),
        }
    }
}

/// At most one held dispatch per kind.
#[derive(Default)]
pub(super) struct DeferredAccountDispatch {
    activity: Option<DeferredActivity>,
    space_children_reload: Option<DeferredSpaceChildrenReload>,
    crawler: Option<DeferredCrawlerLane>,
    settings_policy: Option<(
        Option<SessionKeyId>,
        std::collections::VecDeque<SettingsPolicyMessage>,
    )>,
    /// Test-only causal fence: after each dispatch decision, the held state.
    #[cfg(test)]
    pub(super) observer: Option<mpsc::UnboundedSender<DeferredDispatchObservation>>,
}

/// Test-only view of what is held after one dispatch decision.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DeferredDispatchObservation {
    pub(super) event: &'static str,
    pub(super) space_children_reload: Option<u64>,
    pub(super) crawler_lane: Vec<String>,
}

impl DeferredAccountDispatch {
    pub(super) fn is_pending(&self) -> bool {
        self.activity.is_some()
            || self.space_children_reload.is_some()
            || self.crawler.is_some()
            || self.settings_policy.is_some()
    }

    /// Test-only: start with a crawler notification already held.
    #[cfg(test)]
    pub(super) fn hold_crawler_notification_for_test(
        &mut self,
        session_key: Option<SessionKeyId>,
        settings: koushi_state::SearchCrawlerSettings,
    ) {
        let mut lane = DeferredCrawlerLane::new(session_key);
        lane.push(CrawlerDispatch::Notify(CrawlerRooms {
            room_ids: Vec::new(),
            latest_event_ids: Default::default(),
            settings,
        }));
        self.crawler = Some(lane);
    }

    #[cfg(test)]
    pub(super) fn observe(&self, event: &'static str) {
        let Some(observer) = &self.observer else {
            return;
        };
        let crawler_lane = self
            .crawler
            .as_ref()
            .map(|lane| {
                let mut entries = Vec::new();
                if lane.rebuild {
                    entries.push("rebuild".to_owned());
                }
                if lane.invalidate {
                    entries.push("invalidate".to_owned());
                }
                if let Some(rooms) = &lane.notify {
                    entries.push(crawler_settings_label(&rooms.settings));
                }
                entries
            })
            .unwrap_or_default();
        let _ = observer.send(DeferredDispatchObservation {
            event,
            space_children_reload: self
                .space_children_reload
                .as_ref()
                .map(|reload| reload.generation),
            crawler_lane,
        });
    }
}

/// Test-only label of a crawler notification's settings.
#[cfg(test)]
pub(super) fn crawler_settings_label(settings: &koushi_state::SearchCrawlerSettings) -> String {
    format!(
        "notify:{:?}:captions={}:filenames={}",
        settings.speed, settings.include_media_captions, settings.include_filenames
    )
}

/// Whether a generation-guarded dispatch reached the mailbox (now or
/// deferred), or the AccountActor is gone and it must be settled as failed.
pub(super) enum GuardedDispatch {
    SentOrDeferred,
    Closed,
}

impl AppActor {
    pub(super) fn dispatch_settings_policy(&mut self) {
        let values = &self.state.settings.values;
        let mut messages = std::collections::VecDeque::from([
            SettingsPolicyMessage::ReadReceipts(values.notifications.send_read_receipts),
            SettingsPolicyMessage::Display {
                thread_root_order: values.timeline.thread_root_order,
                hide_redacted: values.display.hide_redacted,
            },
        ]);
        if self.current_account_key().is_some() {
            messages.push_back(SettingsPolicyMessage::LinkPreviews {
                unencrypted: values.display.url_previews_enabled,
                encrypted: values.display.encrypted_url_previews_enabled,
                overrides: self.state.link_preview_settings.room_overrides.clone(),
            });
        }
        self.deferred_account_dispatch.settings_policy = None;
        while let Some(message) = messages.pop_front() {
            if let Err(unsent) = self.account_actor.try_send(message.clone().into_message()) {
                if let mpsc::error::TrySendError::Full(_) = *unsent {
                    messages.push_front(message);
                    self.deferred_account_dispatch.settings_policy =
                        Some((super::account_settings_session_key(&self.state), messages));
                }
                break;
            }
        }
    }

    pub(super) fn dispatch_activity_resolution(
        &mut self,
        generation: u64,
        unresolved_room_count: u32,
        requests: Vec<ActivityResolutionRequest>,
    ) -> GuardedDispatch {
        // A newer generation supersedes any held resolve or cancel; nothing
        // held may reach the AccountActor after this one.
        self.deferred_account_dispatch.activity = None;
        let Err(unsent) = self
            .account_actor
            .try_send(AccountMessage::ResolveActivity {
                generation,
                requests,
            })
        else {
            return GuardedDispatch::SentOrDeferred;
        };
        match *unsent {
            mpsc::error::TrySendError::Full(AccountMessage::ResolveActivity {
                requests, ..
            }) => {
                self.deferred_account_dispatch.activity = Some(DeferredActivity::Resolve {
                    generation,
                    unresolved_room_count,
                    requests,
                });
                GuardedDispatch::SentOrDeferred
            }
            mpsc::error::TrySendError::Closed(_) => GuardedDispatch::Closed,
            mpsc::error::TrySendError::Full(_) => {
                unreachable!("try_send returns the ResolveActivity it was given")
            }
        }
    }

    /// Stop any running Activity resolution. Supersedes a held resolve.
    pub(super) fn dispatch_cancel_activity_resolution(&mut self) {
        self.deferred_account_dispatch.activity = None;
        if let Err(unsent) = self
            .account_actor
            .try_send(AccountMessage::CancelActivityResolution)
            && let mpsc::error::TrySendError::Full(_) = *unsent
        {
            self.deferred_account_dispatch.activity = Some(DeferredActivity::Cancel);
        }
        // A closed mailbox has no resolution left to cancel.
    }

    pub(super) fn dispatch_space_children_reload(
        &mut self,
        request_id: RequestId,
        space_id: String,
        generation: u64,
    ) -> GuardedDispatch {
        // The reducer bumped the generation for this reload, so an older held
        // one is already superseded.
        self.deferred_account_dispatch.space_children_reload = None;
        let Err(unsent) = self.account_actor.try_send(AccountMessage::RoomCommand(
            RoomCommand::LoadSpaceChildren {
                request_id,
                space_id,
                generation,
            },
        )) else {
            return GuardedDispatch::SentOrDeferred;
        };
        match *unsent {
            mpsc::error::TrySendError::Full(AccountMessage::RoomCommand(
                RoomCommand::LoadSpaceChildren {
                    request_id,
                    space_id,
                    generation,
                },
            )) => {
                self.deferred_account_dispatch.space_children_reload =
                    Some(DeferredSpaceChildrenReload {
                        request_id,
                        space_id,
                        generation,
                    });
                #[cfg(test)]
                self.deferred_account_dispatch
                    .observe("space_children_reload");
                GuardedDispatch::SentOrDeferred
            }
            mpsc::error::TrySendError::Closed(_) => GuardedDispatch::Closed,
            mpsc::error::TrySendError::Full(_) => {
                unreachable!("try_send returns the LoadSpaceChildren it was given")
            }
        }
    }

    /// A user reload of the same Space and generation is being sent now; a
    /// held live-leave reload would only repeat the same `/hierarchy` request.
    pub(super) fn drop_deferred_space_children_reload(&mut self, space_id: &str, generation: u64) {
        if self
            .deferred_account_dispatch
            .space_children_reload
            .as_ref()
            .is_some_and(|reload| reload.space_id == space_id && reload.generation == generation)
        {
            self.deferred_account_dispatch.space_children_reload = None;
        }
        #[cfg(test)]
        self.deferred_account_dispatch
            .observe("user_space_children_reload");
    }

    /// Send one search-crawler lane message, or hold it behind the lane.
    pub(super) fn dispatch_crawler(&mut self, dispatch: CrawlerDispatch) {
        let session_key = super::navigation::navigation_session_key(&self.state);
        if let Some(lane) = self.deferred_account_dispatch.crawler.as_mut() {
            // Something is held: this message must not overtake it. Another
            // session's held lane is discarded rather than merged.
            if lane.session_key != session_key {
                *lane = DeferredCrawlerLane::new(session_key);
            }
            lane.push(dispatch);
            #[cfg(test)]
            self.deferred_account_dispatch.observe("crawler");
            return;
        }
        if let Err(unsent) = self.account_actor.try_send(crawler_message(dispatch))
            && let mpsc::error::TrySendError::Full(message) = *unsent
        {
            let mut lane = DeferredCrawlerLane::new(session_key);
            lane.push(crawler_dispatch_of_message(message));
            self.deferred_account_dispatch.crawler = Some(lane);
        }
        // A closed mailbox has no crawler to notify; there is nothing to settle.
        #[cfg(test)]
        self.deferred_account_dispatch.observe("crawler");
    }

    /// Whether open Activity still waits on this resolution generation.
    fn activity_resolution_is_current(&self, generation: u64) -> bool {
        matches!(
            &self.state.activity,
            ActivityState::Open { unread, .. }
                if matches!(
                    unread.resolution,
                    ActivityResolutionState::Resolving { generation: current, .. }
                        if current == generation
                )
        )
    }

    /// Whether the Space-children slice still waits on this reload.
    fn space_children_reload_is_current(&self, reload: &DeferredSpaceChildrenReload) -> bool {
        let children = &self.state.space_children;
        children.selected_space_id.as_deref() == Some(reload.space_id.as_str())
            && children.generation == reload.generation
            && children.load == SpaceChildrenLoadState::Loading
    }

    /// The next held message still worth delivering, most important first.
    /// Stale held values found on the way are dropped.
    fn next_deferred_message(&mut self) -> Option<AccountMessage> {
        if let Some((session_key, mut messages)) =
            self.deferred_account_dispatch.settings_policy.take()
            && session_key == super::account_settings_session_key(&self.state)
        {
            let message = messages
                .pop_front()
                .map(SettingsPolicyMessage::into_message);
            if !messages.is_empty() {
                self.deferred_account_dispatch.settings_policy = Some((session_key, messages));
            }
            return message;
        }
        match self.deferred_account_dispatch.activity.take() {
            Some(DeferredActivity::Resolve {
                generation,
                requests,
                ..
            }) if self.activity_resolution_is_current(generation) => {
                return Some(AccountMessage::ResolveActivity {
                    generation,
                    requests,
                });
            }
            Some(DeferredActivity::Cancel) => {
                return Some(AccountMessage::CancelActivityResolution);
            }
            // A superseded generation, or one whose Activity closed.
            Some(DeferredActivity::Resolve { .. }) | None => {}
        }
        if let Some(reload) = self.deferred_account_dispatch.space_children_reload.take()
            && self.space_children_reload_is_current(&reload)
        {
            return Some(AccountMessage::RoomCommand(
                RoomCommand::LoadSpaceChildren {
                    request_id: reload.request_id,
                    space_id: reload.space_id,
                    generation: reload.generation,
                },
            ));
        }
        let mut lane = self.deferred_account_dispatch.crawler.take()?;
        // Another session's crawler lane must not reach this session.
        if lane.session_key != super::navigation::navigation_session_key(&self.state) {
            return None;
        }
        let message = lane.pop();
        if !lane.is_empty() {
            self.deferred_account_dispatch.crawler = Some(lane);
        }
        message
    }

    /// Spend one reserved mailbox slot on the most important deferred
    /// dispatch, or settle what can no longer be delivered.
    pub(super) async fn deliver_deferred_account_dispatch(
        &mut self,
        permit: Result<mpsc::OwnedPermit<AccountMessage>, mpsc::error::SendError<()>>,
    ) {
        let Ok(permit) = permit else {
            // The AccountActor is gone. The crawler lane and a cancel need no
            // settlement; a started resolution becomes failed and retryable,
            // exactly as an AccountActor without a session reports it, and a
            // reload fails while the cached children remain.
            self.deferred_account_dispatch.crawler = None;
            self.deferred_account_dispatch.settings_policy = None;
            let mut failures = Vec::new();
            if let Some(DeferredActivity::Resolve {
                generation,
                unresolved_room_count,
                ..
            }) = self.deferred_account_dispatch.activity.take()
            {
                failures.push(AppAction::ActivityResolutionFailed {
                    generation,
                    unresolved_room_count,
                    kind: OperationFailureKind::Sdk,
                });
            }
            if let Some(reload) = self.deferred_account_dispatch.space_children_reload.take() {
                failures.push(AppAction::SpaceChildrenLoadFailed {
                    space_id: reload.space_id,
                    generation: reload.generation,
                    failure: OperationFailureKind::Sdk,
                });
            }
            if !failures.is_empty() {
                Box::pin(self.commit_action_batch(failures, ActionBatchOrigin::Actor)).await;
            }
            return;
        };
        // Stale held values are skipped until one is worth the slot.
        while self.deferred_account_dispatch.is_pending() {
            if let Some(message) = self.next_deferred_message() {
                permit.send(message);
                return;
            }
        }
    }
}
