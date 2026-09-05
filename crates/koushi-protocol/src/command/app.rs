use std::fmt;

use koushi_state::{
    ActivityMarkReadTarget, ActivityTab, AttachmentFilter, AttachmentSort, ComposerDocument,
    ComposerDraftRevision, EventNavigationSource, FilesViewScope, InviteScopeSelection,
    JapaneseCatalogProfile, LocalEncryptionHealth, NativeAttentionDispatchId,
    NativeAttentionSoundOutcome, NativeAttentionState, NavigationPreferenceUpdate, RoomListFilter,
    SettingsPatch, StagedUploadCompressionChoice, StagedUploadItem, TimelineScrollAnchor,
};

use crate::ids::RequestId;

pub use koushi_state::MissingTargetPolicy as EventNavigationMissingTargetPolicy;

pub enum AppCommand {
    Shutdown {
        request_id: RequestId,
    },
    SetComposerReplyTarget {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    CancelComposerReply {
        request_id: RequestId,
    },
    SetComposerDraft {
        request_id: RequestId,
        expected_account: crate::SessionKeyId,
        room_id: String,
        document: ComposerDocument,
        revision: ComposerDraftRevision,
    },
    SetThreadComposerDraft {
        request_id: RequestId,
        expected_account: crate::SessionKeyId,
        room_id: String,
        root_event_id: String,
        document: ComposerDocument,
        revision: ComposerDraftRevision,
    },
    AcceptComposerDraft {
        request_id: RequestId,
        expected_account: crate::SessionKeyId,
        target: koushi_state::ComposerTarget,
        submitted_revision: ComposerDraftRevision,
    },
    SetUploadStaging {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        items: Vec<StagedUploadItem>,
    },
    UpdateStagedUploadCaption {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        caption: Option<ComposerDocument>,
    },
    UpdateStagedUploadCompression {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        compression_choice: StagedUploadCompressionChoice,
    },
    SelectStagedUploadOutput {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
        staged_id: String,
        selection: koushi_state::StagedUploadOutputSelection,
    },
    ClearUploadStaging {
        request_id: RequestId,
        target: koushi_state::ComposerTarget,
    },
    ScheduleSend {
        request_id: RequestId,
        expected_account: crate::SessionKeyId,
        room_id: String,
        thread_root_event_id: Option<String>,
        body: String,
        send_at_ms: u64,
        draft_revision: ComposerDraftRevision,
    },
    CancelScheduledSend {
        request_id: RequestId,
        scheduled_id: String,
    },
    RescheduleScheduledSend {
        request_id: RequestId,
        scheduled_id: String,
        body: String,
        send_at_ms: u64,
    },
    OpenThread {
        request_id: RequestId,
        room_id: String,
        root_event_id: String,
        intent: koushi_state::ThreadOpenIntent,
    },
    CloseThread {
        request_id: RequestId,
    },
    OpenFocusedContext {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    NavigateToEvent {
        request_id: RequestId,
        room_id: String,
        event_id: String,
        source: EventNavigationSource,
        missing_target_policy: EventNavigationMissingTargetPolicy,
    },
    /// Starts a main-pane Focused navigation settled by the matching
    /// actor-owned projection commit.
    OpenAnchoredTimeline {
        request_id: RequestId,
        room_id: String,
        event_id: String,
        allow_live_fallback: bool,
    },
    EnterAnchoredTimeline {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    OpenTimelineAtTimestamp {
        request_id: RequestId,
        room_id: String,
        timestamp_ms: u64,
    },
    RepairRoomTimeline {
        request_id: RequestId,
        room_id: String,
    },
    TimelineScrollAnchorUpdated {
        request_id: RequestId,
        room_id: String,
        anchor: TimelineScrollAnchor,
    },
    CloseFocusedContext {
        request_id: RequestId,
    },
    CloseSearch {
        request_id: RequestId,
    },
    OpenInviteWorkflow {
        request_id: RequestId,
        room_id: String,
    },
    CloseInviteWorkflow {
        request_id: RequestId,
    },
    SearchInviteTargets {
        request_id: RequestId,
        room_id: String,
        query: String,
    },
    SetInviteScope {
        request_id: RequestId,
        room_id: String,
        scope: InviteScopeSelection,
    },
    SelectInviteTarget {
        request_id: RequestId,
        room_id: String,
        user_id: String,
    },
    RemoveInviteTarget {
        request_id: RequestId,
        user_id: String,
    },
    UpdateSettings {
        request_id: RequestId,
        patch: SettingsPatch,
    },
    ImportLegacySettings {
        request_id: RequestId,
        patch: SettingsPatch,
    },
    UpdateNavigationPreference {
        request_id: RequestId,
        update: NavigationPreferenceUpdate,
    },
    RebuildSearchIndex {
        request_id: RequestId,
    },
    SetRoomUrlPreviewOverride {
        request_id: RequestId,
        room_id: String,
        enabled: bool,
    },
    OpenActivity {
        request_id: RequestId,
    },
    CloseActivity {
        request_id: RequestId,
    },
    SetActivityTab {
        request_id: RequestId,
        tab: ActivityTab,
    },
    PaginateActivity {
        request_id: RequestId,
        tab: ActivityTab,
        cursor: Option<String>,
    },
    RetryActivityResolution {
        request_id: RequestId,
    },
    MarkActivityRead {
        request_id: RequestId,
        target: ActivityMarkReadTarget,
    },
    OpenFilesView {
        request_id: RequestId,
        scope: FilesViewScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    CloseFilesView {
        request_id: RequestId,
    },
    OpenThreadsList {
        request_id: RequestId,
        scope: koushi_state::ThreadsListScope,
    },
    CloseThreadsList {
        request_id: RequestId,
    },
    PaginateThreadsList {
        request_id: RequestId,
        scope: koushi_state::ThreadsListScope,
    },
    RecordLocalEncryptionHealth {
        request_id: RequestId,
        health: LocalEncryptionHealth,
    },
    UpdateNativeAttentionState {
        request_id: RequestId,
        attention: NativeAttentionState,
    },
    ObserveNativeWindowFocus {
        request_id: RequestId,
        focused: bool,
        observation_generation: u64,
    },
    StartNativeAttentionDispatch {
        request_id: RequestId,
        dispatch_id: NativeAttentionDispatchId,
    },
    SettleNativeAttentionDispatch {
        request_id: RequestId,
        dispatch_id: NativeAttentionDispatchId,
        outcome: NativeAttentionSoundOutcome,
    },
    UpdateJapaneseCatalogProfile {
        request_id: RequestId,
        profile: JapaneseCatalogProfile,
    },
    SelectRoomListFilter {
        request_id: RequestId,
        filter: RoomListFilter,
    },
}

impl fmt::Debug for AppCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shutdown { request_id } => formatter
                .debug_struct("Shutdown")
                .field("request_id", request_id)
                .finish(),
            Self::SetComposerReplyTarget {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("SetComposerReplyTarget")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::CancelComposerReply { request_id } => formatter
                .debug_struct("CancelComposerReply")
                .field("request_id", request_id)
                .finish(),
            Self::SetComposerDraft {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("SetComposerDraft")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("draft", &"MessageBody(..)")
                .finish(),
            Self::SetThreadComposerDraft {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("SetThreadComposerDraft")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("root_event_id", &"EventId(..)")
                .field("draft", &"MessageBody(..)")
                .finish(),
            Self::AcceptComposerDraft { request_id, .. } => formatter
                .debug_struct("AcceptComposerDraft")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .finish(),
            Self::SetUploadStaging {
                request_id, items, ..
            } => formatter
                .debug_struct("SetUploadStaging")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .field("item_count", &items.len())
                .finish(),
            Self::UpdateStagedUploadCaption { request_id, .. } => formatter
                .debug_struct("UpdateStagedUploadCaption")
                .field("request_id", request_id)
                .field("staged_id", &"StagedUploadId(..)")
                .field("caption", &"MediaCaption(..)")
                .finish(),
            Self::UpdateStagedUploadCompression {
                request_id,
                compression_choice,
                ..
            } => formatter
                .debug_struct("UpdateStagedUploadCompression")
                .field("request_id", request_id)
                .field("staged_id", &"StagedUploadId(..)")
                .field("compression_choice", compression_choice)
                .finish(),
            Self::SelectStagedUploadOutput {
                request_id,
                selection,
                ..
            } => formatter
                .debug_struct("SelectStagedUploadOutput")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .field("staged_id", &"StagedUploadId(..)")
                // The chosen axes are not private data; the filename is.
                .field("selection", selection)
                .finish(),
            Self::ClearUploadStaging { request_id, .. } => formatter
                .debug_struct("ClearUploadStaging")
                .field("request_id", request_id)
                .field("target", &"ComposerTarget(..)")
                .finish(),
            Self::ScheduleSend {
                request_id,
                send_at_ms,
                ..
            } => formatter
                .debug_struct("ScheduleSend")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("body", &"MessageBody(..)")
                .field("send_at_ms", &send_at_ms)
                .finish(),
            Self::CancelScheduledSend {
                request_id,
                scheduled_id,
            } => formatter
                .debug_struct("CancelScheduledSend")
                .field("request_id", request_id)
                .field("scheduled_id", scheduled_id)
                .finish(),
            Self::RescheduleScheduledSend {
                request_id,
                scheduled_id,
                body: _,
                send_at_ms,
            } => formatter
                .debug_struct("RescheduleScheduledSend")
                .field("request_id", request_id)
                .field("scheduled_id", scheduled_id)
                .field("send_at_ms", send_at_ms)
                .finish(),
            Self::OpenThread {
                request_id, intent, ..
            } => formatter
                .debug_struct("OpenThread")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("root_event_id", &"EventId(..)")
                .field("intent", intent)
                .finish(),
            Self::CloseThread { request_id } => formatter
                .debug_struct("CloseThread")
                .field("request_id", request_id)
                .finish(),
            Self::OpenFocusedContext {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("OpenFocusedContext")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::NavigateToEvent {
                request_id,
                source,
                missing_target_policy,
                ..
            } => formatter
                .debug_struct("NavigateToEvent")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .field("source", source)
                .field("missing_target_policy", missing_target_policy)
                .finish(),
            Self::OpenAnchoredTimeline { request_id, .. } => formatter
                .debug_struct("OpenAnchoredTimeline")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::EnterAnchoredTimeline {
                request_id,
                room_id,
                ..
            } => formatter
                .debug_struct("EnterAnchoredTimeline")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::OpenTimelineAtTimestamp { request_id, .. } => formatter
                .debug_struct("OpenTimelineAtTimestamp")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("timestamp_ms", &"Timestamp(..)")
                .finish(),
            Self::RepairRoomTimeline { request_id, .. } => formatter
                .debug_struct("RepairRoomTimeline")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::TimelineScrollAnchorUpdated {
                request_id, anchor, ..
            } => formatter
                .debug_struct("TimelineScrollAnchorUpdated")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("event_id", &"EventId(..)")
                .field("offset_px", &anchor.offset_px)
                .field("updated_at_ms", &anchor.updated_at_ms)
                .finish(),
            Self::CloseFocusedContext { request_id } => formatter
                .debug_struct("CloseFocusedContext")
                .field("request_id", request_id)
                .finish(),
            Self::CloseSearch { request_id } => formatter
                .debug_struct("CloseSearch")
                .field("request_id", request_id)
                .finish(),
            Self::OpenInviteWorkflow { request_id, .. } => formatter
                .debug_struct("OpenInviteWorkflow")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::CloseInviteWorkflow { request_id } => formatter
                .debug_struct("CloseInviteWorkflow")
                .field("request_id", request_id)
                .finish(),
            Self::SearchInviteTargets {
                request_id, query, ..
            } => formatter
                .debug_struct("SearchInviteTargets")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("query_len", &query.len())
                .finish(),
            Self::SetInviteScope {
                request_id, scope, ..
            } => formatter
                .debug_struct("SetInviteScope")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("scope", scope)
                .finish(),
            Self::SelectInviteTarget { request_id, .. } => formatter
                .debug_struct("SelectInviteTarget")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::RemoveInviteTarget { request_id, .. } => formatter
                .debug_struct("RemoveInviteTarget")
                .field("request_id", request_id)
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::UpdateSettings { request_id, patch } => formatter
                .debug_struct("UpdateSettings")
                .field("request_id", request_id)
                .field("patch_fields", &settings_patch_field_names(patch))
                .finish(),
            Self::ImportLegacySettings { request_id, patch } => formatter
                .debug_struct("ImportLegacySettings")
                .field("request_id", request_id)
                .field("patch_fields", &settings_patch_field_names(patch))
                .finish(),
            Self::UpdateNavigationPreference { request_id, update } => formatter
                .debug_struct("UpdateNavigationPreference")
                .field("request_id", request_id)
                .field("update", update)
                .finish(),
            Self::RebuildSearchIndex { request_id } => formatter
                .debug_struct("RebuildSearchIndex")
                .field("request_id", request_id)
                .finish(),
            Self::SetRoomUrlPreviewOverride {
                request_id,
                enabled,
                ..
            } => formatter
                .debug_struct("SetRoomUrlPreviewOverride")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("enabled", enabled)
                .finish(),
            Self::OpenActivity { request_id } => formatter
                .debug_struct("OpenActivity")
                .field("request_id", request_id)
                .finish(),
            Self::CloseActivity { request_id } => formatter
                .debug_struct("CloseActivity")
                .field("request_id", request_id)
                .finish(),
            Self::SetActivityTab { request_id, tab } => formatter
                .debug_struct("SetActivityTab")
                .field("request_id", request_id)
                .field("tab", tab)
                .finish(),
            Self::PaginateActivity {
                request_id,
                tab,
                cursor,
            } => formatter
                .debug_struct("PaginateActivity")
                .field("request_id", request_id)
                .field("tab", tab)
                .field("cursor", &cursor.as_ref().map(|_| "PageToken(..)"))
                .finish(),
            Self::RetryActivityResolution { request_id } => formatter
                .debug_struct("RetryActivityResolution")
                .field("request_id", request_id)
                .finish(),
            Self::MarkActivityRead { request_id, target } => formatter
                .debug_struct("MarkActivityRead")
                .field("request_id", request_id)
                .field("target", target)
                .finish(),
            Self::OpenFilesView {
                request_id,
                scope,
                filter,
                sort,
            } => formatter
                .debug_struct("OpenFilesView")
                .field("request_id", request_id)
                .field("scope", scope)
                .field("filter", filter)
                .field("sort", sort)
                .finish(),
            Self::CloseFilesView { request_id } => formatter
                .debug_struct("CloseFilesView")
                .field("request_id", request_id)
                .finish(),
            Self::OpenThreadsList { request_id, .. } => formatter
                .debug_struct("OpenThreadsList")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::CloseThreadsList { request_id } => formatter
                .debug_struct("CloseThreadsList")
                .field("request_id", request_id)
                .finish(),
            Self::PaginateThreadsList { request_id, .. } => formatter
                .debug_struct("PaginateThreadsList")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RecordLocalEncryptionHealth { request_id, health } => formatter
                .debug_struct("RecordLocalEncryptionHealth")
                .field("request_id", request_id)
                .field("health", health)
                .finish(),
            Self::UpdateNativeAttentionState {
                request_id,
                attention,
            } => formatter
                .debug_struct("UpdateNativeAttentionState")
                .field("request_id", request_id)
                .field("unread_count", &attention.summary.unread_count)
                .field("highlight_count", &attention.summary.highlight_count)
                .field("badge_count", &attention.summary.badge_count)
                .field("dispatch", &attention.dispatch.kind())
                .field(
                    "candidate",
                    &attention
                        .summary
                        .candidate
                        .as_ref()
                        .map(|_| "AttentionCandidate(..)"),
                )
                .finish(),
            Self::ObserveNativeWindowFocus {
                request_id,
                focused,
                observation_generation,
            } => formatter
                .debug_struct("ObserveNativeWindowFocus")
                .field("request_id", request_id)
                .field("focused", focused)
                .field("observation_generation", observation_generation)
                .finish(),
            Self::StartNativeAttentionDispatch {
                request_id,
                dispatch_id,
            } => formatter
                .debug_struct("StartNativeAttentionDispatch")
                .field("request_id", request_id)
                .field("dispatch_id", dispatch_id)
                .finish(),
            Self::SettleNativeAttentionDispatch {
                request_id,
                dispatch_id,
                outcome,
            } => formatter
                .debug_struct("SettleNativeAttentionDispatch")
                .field("request_id", request_id)
                .field("dispatch_id", dispatch_id)
                .field("outcome", outcome)
                .finish(),
            Self::UpdateJapaneseCatalogProfile {
                request_id,
                profile,
            } => formatter
                .debug_struct("UpdateJapaneseCatalogProfile")
                .field("request_id", request_id)
                .field("catalog_locale", &profile.catalog_locale)
                .field("complete", &profile.complete)
                .field("missing_count", &profile.missing_message_ids.len())
                .finish(),
            Self::SelectRoomListFilter { request_id, filter } => formatter
                .debug_struct("SelectRoomListFilter")
                .field("request_id", request_id)
                .field("filter", filter)
                .finish(),
        }
    }
}

fn settings_patch_field_names(patch: &SettingsPatch) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if patch.locale.is_some() {
        fields.push("locale");
    }
    if patch.appearance.is_some() {
        fields.push("appearance");
    }
    if patch.typography.is_some() {
        fields.push("typography");
    }
    if patch.keyboard.is_some() {
        fields.push("keyboard");
    }
    if patch.composer.is_some() {
        fields.push("composer");
    }
    if patch.notifications.is_some() {
        fields.push("notifications");
    }
    if patch.display.is_some() {
        fields.push("display");
    }
    if patch.media.is_some() {
        fields.push("media");
    }
    if patch.timeline.is_some() {
        fields.push("timeline");
    }
    if patch.thread_list_order.is_some() {
        fields.push("thread_list_order");
    }
    if patch.room_list_sort.is_some() {
        fields.push("room_list_sort");
    }
    if patch.search_crawler.is_some() {
        fields.push("search_crawler");
    }
    if patch.sidebar.is_some() {
        fields.push("sidebar");
    }
    fields
}

#[cfg(test)]
mod tests;
