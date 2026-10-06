use crate::{
    action::{LoginRequest, RecoveryRequest},
    state::{
        AttachmentFilter, AttachmentScope, AttachmentSort, LoginAttemptId, RoomPreferencesState,
        SearchCrawlerSettings, SearchRoomFilter, SearchScope, SessionInfo, SettingsPatch,
        SettingsValues, SlidingSyncAdmissionKind, SlidingSyncAdmissionSource,
        SlidingSyncCapabilityResult, VerificationCancelReason, VerificationMethod,
        VerificationTarget,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    RestoreSession,
    DiscoverLogin {
        homeserver: String,
    },
    ContinueSlidingSyncAdmission {
        account_epoch: u64,
        request_id: u64,
        admission: SlidingSyncAdmissionKind,
        source: SlidingSyncAdmissionSource,
    },
    RetrySlidingSyncCapabilityDiscovery {
        account_epoch: u64,
        blocked_request_id: u64,
        request_id: u64,
    },
    ScheduleSlidingSyncCapabilityRevalidation {
        account_epoch: u64,
    },
    SettleSlidingSyncCapabilityRevalidation {
        account_epoch: u64,
        request_id: u64,
        result: SlidingSyncCapabilityResult,
    },
    Login {
        attempt_id: LoginAttemptId,
        request: LoginRequest,
    },
    CheckCurrentDeviceTrust,
    InspectSecureBackup,
    SyncConnectivityChanged {
        proven: bool,
    },
    RefreshCurrentSessionStatus {
        request_id: u64,
        trigger: crate::SessionStatusRefreshTrigger,
    },
    /// #1009: (re)place the single Core-owned session-status timer. When the
    /// wall clock reaches `due_at_ms` Core projects
    /// `CurrentSessionStatusCheckDue { token, .. }`.
    ArmCurrentSessionStatusCheck {
        token: u64,
        due_at_ms: u64,
    },
    DiscoverVerificationMethods,
    BeginSessionVerification {
        method: VerificationMethod,
        flow_id: u64,
    },
    RejectProvisionalSession,
    RecoverE2ee(RecoveryRequest),
    RequestVerification {
        request_id: u64,
        target: VerificationTarget,
    },
    AcceptVerification {
        request_id: u64,
    },
    ConfirmSasVerification {
        request_id: u64,
    },
    CancelVerification {
        request_id: u64,
        reason: VerificationCancelReason,
    },
    BootstrapCrossSigning {
        request_id: u64,
    },
    EnableKeyBackup {
        request_id: u64,
    },
    RestoreKeyBackup {
        request_id: u64,
        version: Option<String>,
    },
    ResetIdentity {
        request_id: u64,
    },
    PersistSession(SessionInfo),
    PersistSettings {
        request_id: u64,
        values: SettingsValues,
        patch: Box<SettingsPatch>,
    },
    PersistRoomPreferences {
        request_id: u64,
        preferences: RoomPreferencesState,
    },
    StartSync,
    StopSync,
    SubscribeTimeline {
        room_id: String,
    },
    PaginateTimelineBackwards {
        room_id: String,
    },
    SendText {
        room_id: String,
        transaction_id: String,
        body: String,
    },
    OpenThreadTimeline {
        room_id: String,
        root_event_id: String,
        intent: crate::ThreadOpenIntent,
    },
    OpenFocusedTimeline {
        room_id: String,
        event_id: String,
    },
    /// #1037: a main-composer send (text or attachments) was accepted for the
    /// active room. Pending echoes project only into the live Room timeline,
    /// so Core cancels every main-pane navigation for `room_id` that it still
    /// owns and the reducer cannot see (a date jump or event navigation
    /// awaiting its focused projection), releasing its focused timeline.
    CancelPendingMainTimelineNavigation {
        room_id: String,
    },
    SearchMessages {
        request_id: u64,
        query: String,
        scope: SearchScope,
        room_filter: SearchRoomFilter,
        /// The account's content policy at submission.
        ///
        /// The search actor verifies candidates with it, so a query can never
        /// verify with a policy older than the state that accepted the query.
        content_policy: crate::state::SearchCrawlerSettings,
    },
    /// Publish an admitted result set to the event stream.
    ///
    /// The state is the only place that knows whether a result matches the
    /// accepted query and the account's current content policy, so the search
    /// actor asks for publication here instead of publishing on its own.
    PublishSearchResults {
        request_id: u64,
        results: Vec<crate::state::SearchResult>,
    },
    SearchAttachments {
        request_id: u64,
        scope: AttachmentScope,
        filter: AttachmentFilter,
        sort: AttachmentSort,
    },
    SubscribeThreadsList {
        request_id: u64,
        room_id: String,
    },
    SubscribeThreadsListScoped {
        request_id: u64,
        scope: crate::state::ThreadsListScope,
        room_ids: Vec<String>,
    },
    PaginateThreadsList {
        request_id: u64,
        room_id: String,
    },
    UnsubscribeThreadsList,
    /// Tell the `SearchActor` to idempotently start background crawls for
    /// the given rooms.  Emitted when speed transitions from `Paused` to
    /// active, or when a content-indexing setting changes so rooms are
    /// re-crawled with the new settings.
    NotifySearchCrawlerRoomsAvailable {
        room_ids: Vec<String>,
        /// Latest event id of each room that has one in the room list. The
        /// actor re-queues a catch-up crawl for a completed room whose latest
        /// event changed since its crawl completed (#996).
        latest_event_ids: std::collections::BTreeMap<String, String>,
        settings: SearchCrawlerSettings,
    },
    /// Tell the `SearchActor` to drop all rooms from its `completed_rooms`
    /// cache.  Emitted alongside `NotifySearchCrawlerRoomsAvailable` when
    /// content-indexing settings change so the actor re-crawls rooms that
    /// it had previously recorded as done.
    InvalidateSearchCrawlerCache,
    /// Tell the `SearchActor` to clear its in-memory search document store and
    /// crawler queues before a full local search rebuild.
    RebuildSearchIndex,
    /// Issue #1062: reload the selected Space's advertised children under
    /// `generation`. The reducer has already admitted the load (the slice is
    /// `Loading` under this generation), so the runtime only routes the
    /// command to the room actor.
    LoadSpaceChildren {
        space_id: String,
        generation: u64,
    },
    RecordNativeAttentionRecomputed {
        observation: crate::NativeAttentionObservationKind,
        unread_count: u64,
        notification_count: u64,
        badge_count: u64,
        badge_room_count: u64,
        badge_excluded_room_count: u64,
        candidate: Option<crate::RoomAttentionKind>,
        suppression: Option<crate::NativeAttentionSuppressionReason>,
        window_focused: bool,
        active_room_match: bool,
    },
    EmitUiEvent(UiEvent),
}

/// Display-label identities retained by a profile mutation.
///
/// An empty list denotes a profile change that does not change display labels.
/// Debug output includes the count, never the identities.
///
/// ```
/// use koushi_state::ProfileDisplayChange;
/// let change = ProfileDisplayChange { user_ids: vec!["@example:example.invalid".into()] };
/// assert!(!format!("{change:?}").contains("@example"));
/// ```
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ProfileDisplayChange {
    /// Candidates to resolve; publication deduplicates repeated identities.
    pub user_ids: Vec<String>,
}

impl std::fmt::Debug for ProfileDisplayChange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProfileDisplayChange")
            .field("user_count", &self.user_ids.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiEvent {
    SessionChanged,
    AuthChanged,
    SettingsChanged,
    LinkPreviewSettingsChanged,
    ProfileChanged(ProfileDisplayChange),
    RoomListChanged,
    InviteWorkflowChanged,
    FocusedContextChanged,
    SpaceChildrenChanged,
    SpaceMembersChanged,
    TimelineChanged { room_id: String },
    ThreadChanged,
    ThreadsListChanged,
    SearchChanged,
    SearchCrawlerChanged,
    FilesViewChanged,
    HistoryExportChanged,
    LiveSignalsChanged,
    E2eeTrustChanged,
    E2eeKeyManagementChanged,
    AccountManagementChanged,
    AccountManagementCapabilitiesChanged,
    AccountNotificationsChanged,
    ContactSecurityChanged,
    SoftLogoutReauthChanged,
    QrLoginChanged,
    RoomInteractionsChanged,
    DirectoryChanged,
    ActivityChanged,
    RoomManagementChanged,
    MentionCandidatesChanged,
    LocalEncryptionChanged,
    NativeAttentionChanged,
    CjkTextPolicyChanged,
    RoomNotificationSettingsChanged,
    ErrorChanged,
}
