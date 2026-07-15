//! Data-transfer objects: Rust → TypeScript serialization contract.
//!
//! `FrontendDesktopSnapshot` is built from `AppStateSnapshot` (the core state
//! projection). Timeline items and thread messages are REMOVED from the
//! snapshot in Phase 7; they flow as `CoreEvent::Timeline` diffs over
//! `koushi-desktop://event`. The TS types.ts contract keeps `timeline` and
//! `thread` fields for backward compat; the adapter now always sends `[]` /
//! `null` and the React timeline store populates them from events.
//!
//! References: overview.md "Async rule 4" — timeline items never in AppState.

use std::collections::BTreeMap;

use koushi_core::StateDelta;
use koushi_state::{
    AccountManagementCapabilities, AccountManagementState, ActivityState, AppError, AppState,
    AuthDiscoveryState, BasicOperationState, CjkTextPolicyState, ComposerState,
    DeviceSessionListState, DirectoryState, DisplayPlatform, E2eeTrustState, FilesViewState,
    FocusedContextState, InvitePreview, InviteWorkflowState, LinkPreviewSettingsState,
    LiveSignalsState, LocalEncryptionState, LocaleDisplayProfile, NativeAttentionCapabilities,
    NativeAttentionState, NavigationState, ProfileState, ProvisionalPhase, QrLoginState,
    RoomInteractionState, RoomListProjection, RoomManagementState, RoomNotificationSettings,
    RoomPreferencesState, RoomSummary, SearchCrawlerState, SearchMatchField, SearchMatchKind,
    SearchResult, SearchScope, SearchState, SessionState, SettingsState, SidebarModel,
    SoftLogoutReauthState, SpaceSummary, StagedUploadItem, SyncMode, SyncState,
    ThreadAttentionState, ThreadPaneState, ThreadsListState, TimelinePaneState,
    TypographyDisplayProfile, VerificationGateRejectReason, VerificationGateState,
    VerificationMethod, native_attention_capabilities_for_platform, resolve_locale_display_profile,
    resolve_typography_display_profile,
};
use serde::{Deserialize, Serialize};

/// The snapshot returned by all Tauri commands.
///
/// `timeline` and `thread` are always empty / `None` in Phase 7; timeline
/// items flow as `TimelineEvent` diffs over `koushi-desktop://event`.
#[derive(Clone, Debug, Serialize)]
pub struct FrontendDesktopSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_generation: Option<u64>,
    pub state: FrontendAppState,
    pub sidebar: SidebarModel,
    /// Always empty in Phase 7; timeline items flow as diffs.
    pub timeline: Vec<()>,
    /// Always None in Phase 7; thread flow as events.
    pub thread: Option<()>,
}

impl From<AppState> for FrontendDesktopSnapshot {
    fn from(state: AppState) -> Self {
        let sidebar = koushi_state::compose_sidebar(
            state.navigation.active_space_id.as_deref(),
            &state.spaces,
            &state.rooms,
        );
        Self {
            state_generation: None,
            state: state.into(),
            sidebar,
            timeline: Vec::new(),
            thread: None,
        }
    }
}

impl FrontendDesktopSnapshot {
    pub fn from_versioned(state: AppState, generation: u64) -> Self {
        let mut snapshot = Self::from(state);
        snapshot.state_generation = Some(generation);
        snapshot
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendDesktopSnapshotDelta {
    pub generation: u64,
    pub changed: FrontendDesktopSnapshotChangedSlices,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FrontendDesktopSnapshotChangedSlices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<FrontendAppStateChangedSlices>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<SidebarModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<Vec<()>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<Option<()>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendAppStateChangedSlices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<FrontendDomainStateChangedSlices>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<FrontendUiStateChangedSlices>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FrontendDomainStateChangedSlices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<FrontendSessionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthDiscoveryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_sessions: Option<DeviceSessionListState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_management: Option<AccountManagementState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_management_capabilities: Option<AccountManagementCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_logout_reauth: Option<SoftLogoutReauthState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_login: Option<QrLoginState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_preview_settings: Option<LinkPreviewSettingsState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_preferences: Option<RoomPreferencesState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale_profile: Option<LocaleDisplayProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typography_profile: Option<TypographyDisplayProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<FrontendSyncState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_mode: Option<SyncMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spaces: Option<Vec<SpaceSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<RoomSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invites: Option<Vec<InvitePreview>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_workflow: Option<InviteWorkflowState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_notification_settings:
        Option<std::collections::HashMap<String, RoomNotificationSettings>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_interactions: Option<BTreeMap<String, RoomInteractionState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<DirectoryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_management: Option<RoomManagementState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<ActivityState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_attention: Option<ThreadAttentionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<FrontendSearchState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_crawler: Option<SearchCrawlerState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_signals: Option<LiveSignalsState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e2ee_trust: Option<E2eeTrustState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_encryption: Option<LocalEncryptionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_attention: Option<NativeAttentionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cjk_text_policy: Option<CjkTextPolicyState>,
}

impl FrontendDomainStateChangedSlices {
    fn is_empty(&self) -> bool {
        self.session.is_none()
            && self.auth.is_none()
            && self.device_sessions.is_none()
            && self.account_management.is_none()
            && self.account_management_capabilities.is_none()
            && self.soft_logout_reauth.is_none()
            && self.qr_login.is_none()
            && self.settings.is_none()
            && self.link_preview_settings.is_none()
            && self.room_preferences.is_none()
            && self.locale_profile.is_none()
            && self.typography_profile.is_none()
            && self.profile.is_none()
            && self.sync.is_none()
            && self.sync_mode.is_none()
            && self.spaces.is_none()
            && self.rooms.is_none()
            && self.invites.is_none()
            && self.invite_workflow.is_none()
            && self.room_notification_settings.is_none()
            && self.room_interactions.is_none()
            && self.directory.is_none()
            && self.room_management.is_none()
            && self.activity.is_none()
            && self.thread_attention.is_none()
            && self.search.is_none()
            && self.search_crawler.is_none()
            && self.live_signals.is_none()
            && self.e2ee_trust.is_none()
            && self.local_encryption.is_none()
            && self.native_attention.is_none()
            && self.cjk_text_policy.is_none()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FrontendUiStateChangedSlices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<NavigationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_list: Option<RoomListProjection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<TimelinePaneState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<FrontendThreadPaneState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused_context: Option<FocusedContextState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_view: Option<FilesViewState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads_list: Option<ThreadsListState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basic_operation: Option<BasicOperationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<AppError>>,
}

impl FrontendUiStateChangedSlices {
    fn is_empty(&self) -> bool {
        self.navigation.is_none()
            && self.room_list.is_none()
            && self.timeline.is_none()
            && self.thread.is_none()
            && self.focused_context.is_none()
            && self.files_view.is_none()
            && self.threads_list.is_none()
            && self.basic_operation.is_none()
            && self.errors.is_none()
    }
}

impl From<StateDelta> for FrontendDesktopSnapshotDelta {
    fn from(delta: StateDelta) -> Self {
        let platform = frontend_display_platform();
        let changed = delta.changed;
        let mut domain = FrontendDomainStateChangedSlices::default();
        let mut ui = FrontendUiStateChangedSlices::default();

        domain.session = changed.session.map(Into::into);
        domain.auth = changed.auth;
        domain.device_sessions = changed.device_sessions;
        domain.account_management = changed.account_management;
        domain.account_management_capabilities = changed.account_management_capabilities;
        domain.soft_logout_reauth = changed.soft_logout_reauth;
        domain.qr_login = changed.qr_login;
        if let Some(settings) = changed.settings {
            domain.locale_profile = Some(resolve_locale_display_profile(
                &settings.values.locale,
                platform,
            ));
            domain.typography_profile = Some(resolve_typography_display_profile(
                &settings.values.typography,
                platform,
            ));
            domain.settings = Some(settings);
        }
        domain.link_preview_settings = changed.link_preview_settings;
        domain.room_preferences = changed.room_preferences;
        domain.profile = changed.profile;
        domain.sync = changed.sync.map(Into::into);
        domain.sync_mode = changed.sync_mode;
        domain.spaces = changed.spaces;
        domain.rooms = changed.rooms;
        domain.invites = changed.invites;
        domain.invite_workflow = changed.invite_workflow;
        domain.room_notification_settings = changed.room_notification_settings;
        domain.room_interactions = changed.room_interactions;
        domain.directory = changed.directory;
        domain.room_management = changed.room_management;
        domain.activity = changed.activity;
        domain.thread_attention = changed.thread_attention;
        domain.search = changed.search.map(Into::into);
        domain.search_crawler = changed.search_crawler;
        domain.live_signals = changed.live_signals;
        domain.e2ee_trust = changed.e2ee_trust;
        domain.local_encryption = changed.local_encryption;
        if let Some(mut native_attention) = changed.native_attention {
            if native_attention.summary.capabilities == NativeAttentionCapabilities::default() {
                native_attention.summary.capabilities =
                    native_attention_capabilities_for_platform(platform);
            }
            domain.native_attention = Some(native_attention);
        }
        domain.cjk_text_policy = changed.cjk_text_policy;

        ui.navigation = changed.navigation;
        ui.room_list = changed.room_list;
        ui.timeline = changed.timeline;
        ui.thread = changed.thread.map(Into::into);
        ui.focused_context = changed.focused_context;
        ui.files_view = changed.files_view;
        ui.threads_list = changed.threads_list;
        ui.basic_operation = changed.basic_operation;
        ui.errors = changed.errors;

        let state = if domain.is_empty() && ui.is_empty() {
            None
        } else {
            Some(FrontendAppStateChangedSlices {
                schema_version: Some(SNAPSHOT_SCHEMA_VERSION),
                domain: (!domain.is_empty()).then_some(domain),
                ui: (!ui.is_empty()).then_some(ui),
            })
        };

        Self {
            generation: delta.generation,
            changed: FrontendDesktopSnapshotChangedSlices {
                state,
                sidebar: changed.sidebar,
                timeline: None,
                thread: None,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendAppState {
    /// IPC snapshot contract version. v2 introduced the domain/ui sectioning
    /// (#87 Phase 4). The renderer asserts this so a stale flat (v1) snapshot or
    /// a mismatched Rust/TS build fails loudly instead of reading `undefined`.
    pub schema_version: u32,
    pub domain: FrontendDomainState,
    pub ui: FrontendUiState,
}

/// Matrix/product state — Rust-owned, reusable by a future mobile shell.
#[derive(Clone, Debug, Serialize)]
pub struct FrontendDomainState {
    pub session: FrontendSessionState,
    pub auth: AuthDiscoveryState,
    pub device_sessions: DeviceSessionListState,
    pub account_management: AccountManagementState,
    pub account_management_capabilities: AccountManagementCapabilities,
    pub soft_logout_reauth: SoftLogoutReauthState,
    pub qr_login: QrLoginState,
    pub settings: SettingsState,
    pub link_preview_settings: LinkPreviewSettingsState,
    pub room_preferences: RoomPreferencesState,
    pub locale_profile: LocaleDisplayProfile,
    pub typography_profile: TypographyDisplayProfile,
    pub profile: ProfileState,
    pub sync: FrontendSyncState,
    pub sync_mode: SyncMode,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub invites: Vec<InvitePreview>,
    pub invite_workflow: InviteWorkflowState,
    pub room_notification_settings: std::collections::HashMap<String, RoomNotificationSettings>,
    pub room_interactions: BTreeMap<String, RoomInteractionState>,
    pub directory: DirectoryState,
    pub room_management: RoomManagementState,
    pub activity: ActivityState,
    pub thread_attention: ThreadAttentionState,
    pub search: FrontendSearchState,
    pub search_crawler: SearchCrawlerState,
    pub live_signals: LiveSignalsState,
    pub e2ee_trust: E2eeTrustState,
    pub local_encryption: LocalEncryptionState,
    pub native_attention: NativeAttentionState,
    pub cjk_text_policy: CjkTextPolicyState,
}

/// Desktop presentation / view / navigation state.
#[derive(Clone, Debug, Serialize)]
pub struct FrontendUiState {
    pub navigation: NavigationState,
    pub room_list: RoomListProjection,
    pub timeline: TimelinePaneState,
    pub thread: FrontendThreadPaneState,
    pub focused_context: FocusedContextState,
    pub files_view: FilesViewState,
    pub threads_list: ThreadsListState,
    pub basic_operation: BasicOperationState,
    pub errors: Vec<AppError>,
}

impl From<AppState> for FrontendAppState {
    fn from(state: AppState) -> Self {
        frontend_app_state_for_platform(state, frontend_display_platform())
    }
}

fn frontend_app_state_for_platform(state: AppState, platform: DisplayPlatform) -> FrontendAppState {
    let locale_profile = resolve_locale_display_profile(&state.settings.values.locale, platform);
    let typography_profile =
        resolve_typography_display_profile(&state.settings.values.typography, platform);
    let mut native_attention = state.native_attention;
    if native_attention.summary.capabilities == NativeAttentionCapabilities::default() {
        native_attention.summary.capabilities =
            native_attention_capabilities_for_platform(platform);
    }
    FrontendAppState {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        domain: FrontendDomainState {
            session: state.session.into(),
            auth: state.auth,
            device_sessions: state.device_sessions,
            account_management: state.account_management,
            account_management_capabilities: state.account_management_capabilities,
            soft_logout_reauth: state.soft_logout_reauth,
            qr_login: state.qr_login,
            settings: state.settings,
            link_preview_settings: state.link_preview_settings,
            room_preferences: state.room_preferences,
            locale_profile,
            typography_profile,
            profile: state.profile,
            sync: state.sync.into(),
            sync_mode: state.sync_mode,
            spaces: state.spaces,
            rooms: state.rooms,
            invites: state.invites,
            invite_workflow: state.invite_workflow,
            room_notification_settings: state.room_notification_settings,
            room_interactions: state.room_interactions,
            directory: state.directory,
            room_management: state.room_management,
            activity: state.activity,
            thread_attention: state.thread_attention,
            search: state.search.into(),
            search_crawler: state.search_crawler,
            live_signals: state.live_signals,
            e2ee_trust: state.e2ee_trust,
            local_encryption: state.local_encryption,
            native_attention,
            cjk_text_policy: state.cjk_text_policy,
        },
        ui: FrontendUiState {
            navigation: state.navigation,
            room_list: state.room_list,
            timeline: state.timeline,
            thread: state.thread.into(),
            focused_context: state.focused_context,
            files_view: state.files_view,
            threads_list: state.threads_list,
            basic_operation: state.basic_operation,
            errors: state.errors,
        },
    }
}

/// IPC snapshot contract version. Bumped to 2 by #87 Phase 4 (domain/ui sectioning).
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;

fn frontend_display_platform() -> DisplayPlatform {
    #[cfg(target_os = "macos")]
    {
        DisplayPlatform::Macos
    }
    #[cfg(target_os = "windows")]
    {
        DisplayPlatform::Windows
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        DisplayPlatform::Linux
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSessionState {
    SignedOut,
    Restoring,
    SwitchingAccount {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    Authenticating {
        homeserver: String,
        attempt_id: FrontendLoginAttemptId,
    },
    Provisional {
        homeserver: String,
        user_id: String,
        device_id: String,
        phase: ProvisionalPhase,
    },
    AwaitingVerification {
        homeserver: String,
        user_id: String,
        device_id: String,
        gate: VerificationGateState,
    },
    Verifying {
        homeserver: String,
        user_id: String,
        device_id: String,
        gate: VerificationGateState,
        method: VerificationMethod,
        flow_id: u64,
        sas_emojis: Vec<koushi_state::SasEmoji>,
    },
    AwaitingBootstrapConfirmation {
        homeserver: String,
        user_id: String,
        device_id: String,
        gate: VerificationGateState,
        flow_id: u64,
        destination_written: bool,
    },
    Rejecting {
        homeserver: String,
        user_id: String,
        device_id: String,
        reason: VerificationGateRejectReason,
    },
    Ready {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    Locked {
        homeserver: String,
        user_id: String,
        device_id: String,
    },
    LoggingOut,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FrontendLoginAttemptId {
    pub connection_id: u64,
    pub sequence: u64,
}

impl From<SessionState> for FrontendSessionState {
    fn from(session: SessionState) -> Self {
        match session {
            SessionState::SignedOut => Self::SignedOut,
            SessionState::Restoring => Self::Restoring,
            SessionState::SwitchingAccount { info } => Self::SwitchingAccount {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::Authenticating {
                homeserver,
                attempt_id,
            } => Self::Authenticating {
                homeserver,
                attempt_id: FrontendLoginAttemptId {
                    connection_id: attempt_id.connection_id(),
                    sequence: attempt_id.sequence(),
                },
            },
            SessionState::Provisional { info, phase } => Self::Provisional {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                phase,
            },
            SessionState::AwaitingVerification { info, gate } => Self::AwaitingVerification {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                gate,
            },
            SessionState::Verifying {
                info,
                gate,
                method,
                flow_id,
                sas_emojis,
            } => Self::Verifying {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                gate,
                method,
                flow_id,
                sas_emojis,
            },
            SessionState::AwaitingBootstrapConfirmation {
                info,
                gate,
                flow_id,
                destination_written,
            } => Self::AwaitingBootstrapConfirmation {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                gate,
                flow_id,
                destination_written,
            },
            SessionState::Rejecting { info, reason } => Self::Rejecting {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                reason,
            },
            SessionState::Ready(info) => Self::Ready {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::Locked(info) => Self::Locked {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
            },
            SessionState::LoggingOut => Self::LoggingOut,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum FrontendSyncState {
    Name(&'static str),
    Failed { failed: String },
    Reconnecting { reconnecting: String },
}

impl From<SyncState> for FrontendSyncState {
    fn from(sync: SyncState) -> Self {
        match sync {
            SyncState::Stopped => Self::Name("stopped"),
            SyncState::Starting => Self::Name("starting"),
            SyncState::Running => Self::Name("running"),
            SyncState::Failed { reason } => Self::Failed { failed: reason },
            SyncState::Reconnecting { reason } => Self::Reconnecting {
                reconnecting: reason,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendThreadPaneState {
    Closed,
    Opening {
        room_id: String,
        root_event_id: String,
    },
    Open {
        room_id: String,
        root_event_id: String,
        is_subscribed: bool,
        composer: ComposerState,
        staged_uploads: Vec<StagedUploadItem>,
    },
}

impl From<ThreadPaneState> for FrontendThreadPaneState {
    fn from(thread: ThreadPaneState) -> Self {
        match thread {
            ThreadPaneState::Closed => Self::Closed,
            ThreadPaneState::Opening {
                room_id,
                root_event_id,
            } => Self::Opening {
                room_id,
                root_event_id,
            },
            ThreadPaneState::Open {
                room_id,
                root_event_id,
                is_subscribed,
                composer,
                staged_uploads,
            } => Self::Open {
                room_id,
                root_event_id,
                is_subscribed,
                composer,
                staged_uploads,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendSearchState {
    Closed,
    Editing {
        query: String,
        scope: SearchScopeKind,
    },
    Searching {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
    },
    Results {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        results: Vec<FrontendSearchResult>,
    },
    Failed {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        message: String,
    },
}

impl From<SearchState> for FrontendSearchState {
    fn from(search: SearchState) -> Self {
        match search {
            SearchState::Closed => Self::Closed,
            SearchState::Editing { query, scope } => Self::Editing {
                query,
                scope: scope.into(),
            },
            SearchState::Searching {
                request_id,
                query,
                scope,
            } => Self::Searching {
                request_id,
                query,
                scope: scope.into(),
            },
            SearchState::Results {
                request_id,
                query,
                scope,
                results,
            } => Self::Results {
                request_id,
                query,
                scope: scope.into(),
                results: results.into_iter().map(Into::into).collect(),
            },
            SearchState::Failed {
                request_id,
                query,
                scope,
                message,
            } => Self::Failed {
                request_id,
                query,
                scope: scope.into(),
                message,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScopeKind {
    CurrentRoom,
    CurrentSpace,
    Dms,
    AllRooms,
}

impl SearchScopeKind {
    #[allow(dead_code)]
    pub fn resolve(self, state: &AppState) -> SearchScope {
        match self {
            Self::CurrentRoom => state
                .navigation
                .active_room_id
                .as_ref()
                .map(|room_id| SearchScope::CurrentRoom {
                    room_id: room_id.clone(),
                })
                .unwrap_or(SearchScope::AllRooms),
            Self::CurrentSpace => state
                .navigation
                .active_space_id
                .as_ref()
                .map(|space_id| SearchScope::CurrentSpace {
                    space_id: space_id.clone(),
                })
                .unwrap_or(SearchScope::AllRooms),
            Self::Dms => SearchScope::Dms,
            Self::AllRooms => SearchScope::AllRooms,
        }
    }
}

impl From<SearchScope> for SearchScopeKind {
    fn from(scope: SearchScope) -> Self {
        match scope {
            SearchScope::CurrentRoom { .. } => Self::CurrentRoom,
            SearchScope::CurrentSpace { .. } => Self::CurrentSpace,
            SearchScope::Dms => Self::Dms,
            SearchScope::AllRooms => Self::AllRooms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendSearchResult {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub score_millis: u32,
    pub snippet: String,
    pub match_field: FrontendSearchMatchField,
    pub highlights: Vec<koushi_state::TextRange>,
    pub match_kind: FrontendSearchMatchKind,
}

impl From<SearchResult> for FrontendSearchResult {
    fn from(result: SearchResult) -> Self {
        Self {
            room_id: result.room_id,
            event_id: result.event_id,
            sender: result.sender,
            timestamp_ms: result.timestamp_ms,
            score_millis: result.score_millis,
            snippet: result.snippet,
            match_field: result.match_field.into(),
            highlights: result.highlights,
            match_kind: result.match_kind.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendSearchMatchField {
    MessageBody,
    AttachmentFileName,
}

impl From<SearchMatchField> for FrontendSearchMatchField {
    fn from(field: SearchMatchField) -> Self {
        match field {
            SearchMatchField::MessageBody => Self::MessageBody,
            SearchMatchField::AttachmentFileName => Self::AttachmentFileName,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendSearchMatchKind {
    Exact,
}

impl From<SearchMatchKind> for FrontendSearchMatchKind {
    fn from(kind: SearchMatchKind) -> Self {
        match kind {
            SearchMatchKind::Exact => Self::Exact,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FrontendDesktopSnapshot, FrontendSyncState, frontend_display_platform};
    use koushi_state::{
        AppState, AvatarImage, AvatarThumbnailState, EmojiPreference, FontPreference,
        InvitePreview, LocaleSettings, OwnProfile, RoomSummary, RoomTags, SessionInfo,
        SessionState, SpaceSummary, SyncState, TextDirectionPreference, TypographySettings,
        UserProfile, native_attention_capabilities_for_platform,
    };

    fn booted_app_state() -> AppState {
        AppState {
            session: SessionState::Ready(SessionInfo {
                homeserver: "https://matrix.org".to_owned(),
                user_id: "@user:matrix.org".to_owned(),
                device_id: "DEVICE".to_owned(),
            }),
            sync: SyncState::Running,
            ..AppState::default()
        }
    }

    #[test]
    fn frontend_snapshot_serializes_to_the_typescript_contract() {
        let state = booted_app_state();
        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(value["state"]["domain"]["session"]["kind"], json!("ready"));
        assert_eq!(
            value["state"]["domain"]["session"]["homeserver"],
            json!("https://matrix.org")
        );
        assert_eq!(value["state"]["domain"]["sync"], json!("running"));
        // invites must be present even when empty; React must not synthesize
        // invite state outside the Rust-owned state machine.
        assert_eq!(value["state"]["domain"]["invites"], json!([]));
        // Core Batch A skeletons must be present in the real Tauri DTO, not
        // only in browser fakes.
        assert_eq!(value["state"]["domain"]["room_interactions"], json!({}));
        assert_eq!(
            value["state"]["domain"]["device_sessions"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["account_management"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["account_management_capabilities"]["change_password"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["domain"]["soft_logout_reauth"]["kind"],
            json!("idle")
        );
        assert_eq!(value["state"]["domain"]["qr_login"]["kind"], json!("idle"));
        assert_eq!(
            value["state"]["domain"]["directory"]["query"]["kind"],
            json!("closed")
        );
        assert_eq!(
            value["state"]["domain"]["directory"]["join"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["room_management"]["selected_room_id"],
            json!(null)
        );
        assert_eq!(
            value["state"]["domain"]["room_management"]["operation"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["activity"]["kind"],
            json!("closed")
        );
        // Phase 7: timeline is always [] (items flow as diffs)
        assert_eq!(value["timeline"], json!([]));
        // Phase 7: the legacy top-level thread is always null...
        assert_eq!(value["thread"], json!(null));
        // ...product thread state lives in state.thread (default Closed). The UI
        // reads the open/closed decision from here, not the legacy placeholder.
        assert_eq!(value["state"]["ui"]["thread"]["kind"], json!("closed"));
        assert_eq!(
            value["state"]["domain"]["thread_attention"]["kind"],
            json!("closed")
        );
        // focused_context must be present (default Closed) so the UI can drive
        // the focused search context view from the Rust-owned state machine.
        assert_eq!(
            value["state"]["ui"]["focused_context"]["kind"],
            json!("closed")
        );
        // basic_operation must be present (default Idle) so the UI can read
        // snapshot.state.basic_operation.kind without crashing.
        assert_eq!(
            value["state"]["ui"]["basic_operation"]["kind"],
            json!("idle")
        );
        // sync_mode must be present so the UI can render the Rust-owned sync
        // backend/capability state (sliding sync vs legacy) without inference.
        assert_eq!(
            value["state"]["domain"]["sync_mode"]["kind"],
            json!("unsupported")
        );
        // room_list must be present so the UI renders the Rust-owned filtered
        // room-list projection instead of computing filters locally.
        assert_eq!(
            value["state"]["ui"]["room_list"]["active_filter"]["kind"],
            json!("rooms")
        );
        assert_eq!(value["state"]["ui"]["room_list"]["items"], json!([]));
        // live_signals must be present so Phase B GUI renders Rust-owned live
        // signal state without inventing receipts, typing, or presence locally.
        assert_eq!(value["state"]["domain"]["live_signals"]["rooms"], json!({}));
        assert_eq!(
            value["state"]["domain"]["live_signals"]["presence"],
            json!({})
        );
        // e2ee_trust must be present (default private-data-free unknowns) so
        // later GUI work consumes the Rust-owned trust state machine.
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["verification"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["cross_signing"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["key_backup"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["identity_reset"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["key_management"]["room_key_export"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["key_management"]["room_key_import"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["key_management"]["secure_backup_setup"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["e2ee_trust"]["key_management"]["passphrase_change"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["local_encryption"]["kind"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["domain"]["native_attention"]["dispatch"]["kind"],
            json!("idle")
        );
        assert_eq!(
            value["state"]["domain"]["native_attention"]["summary"]["capabilities"],
            serde_json::to_value(native_attention_capabilities_for_platform(
                frontend_display_platform()
            ))
            .expect("capability profile serializes")
        );
        assert_eq!(
            value["state"]["domain"]["cjk_text_policy"]["japanese_catalog"]["catalog_locale"],
            json!("en")
        );
        assert_eq!(
            value["state"]["domain"]["cjk_text_policy"]["normalization"]["form"],
            json!("nfkc")
        );
        assert_eq!(
            value["state"]["domain"]["cjk_text_policy"]["collation"]["locale"],
            json!("ja")
        );
        // settings must be present so React can consume Rust-owned product
        // preferences instead of owning theme/locale/shortcut state.
        assert_eq!(
            value["state"]["domain"]["settings"]["values"]["appearance"]["theme"],
            json!("system")
        );
        assert_eq!(
            value["state"]["domain"]["settings"]["values"]["keyboard"]["composer_send_shortcut"],
            json!("enter")
        );
        assert_eq!(
            value["state"]["domain"]["settings"]["values"]["notifications"],
            json!({
                "desktop_notifications": true,
                "sound": true,
                "badges": true,
                "send_read_receipts": true,
                "send_typing_notifications": true
            })
        );
        // room_notification_settings must be present (default empty) so the UI
        // renders per-room notification modes from Rust-owned state.
        assert_eq!(
            value["state"]["domain"]["room_notification_settings"],
            json!({})
        );
        assert_eq!(
            value["state"]["domain"]["settings"]["values"]["media"]["image_upload_compression"],
            json!("ask")
        );
        assert_eq!(
            value["state"]["domain"]["settings"]["values"]["media"]["image_upload_compression_policy"],
            json!({
                "threshold_bytes": 1048576,
                "threshold_long_edge": 2560,
                "target_long_edge": 2048,
                "quality_percent": 82
            })
        );
        assert_eq!(
            value["state"]["domain"]["settings"]["persistence"]["kind"],
            json!("idle")
        );
        // locale_profile must be present so React applies root lang/dir and
        // catalog selection from Rust-owned settings/profile resolution.
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["lang"],
            json!("en")
        );
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["dir"],
            json!("ltr")
        );
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["catalog_locale"],
            json!("en")
        );
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["pseudo_locale"],
            json!("none")
        );
        // typography_profile must be present so React applies font and emoji
        // behavior from Rust-owned settings/profile resolution.
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["font"],
            json!("system")
        );
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["emoji"],
            json!("system")
        );
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["font_asset"],
            json!("systemFallback")
        );
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["emoji_asset"],
            json!("systemFallback")
        );
        // profile must be present so React displays and submits profile updates
        // from the Rust-owned profile state machine, never local component state.
        assert_eq!(
            value["state"]["domain"]["profile"]["own"]["display_name"],
            json!(null)
        );
        assert_eq!(
            value["state"]["domain"]["profile"]["own"]["avatar"],
            json!(null)
        );
        assert_eq!(value["state"]["domain"]["profile"]["users"], json!({}));
        assert_eq!(
            value["state"]["domain"]["profile"]["update"]["kind"],
            json!("idle")
        );
        // composer.mode must be present (default Plain) for the same reason.
        assert_eq!(
            value["state"]["ui"]["timeline"]["composer"]["mode"],
            json!("Plain")
        );
        // The keyed draft backing store can contain non-visible unsent message
        // bodies. It stays Rust/core-internal; the webview receives only the
        // selected room/thread active composer.
        assert_eq!(value["state"]["domain"]["composer_drafts"], json!(null));
        // Scheduled-send backing state follows the same privacy boundary:
        // the full queue can contain future message bodies for non-visible
        // rooms, so only the selected timeline projection is serialized.
        assert_eq!(value["state"]["domain"]["scheduled_sends"], json!(null));
        // Upload staging and media-gallery backing stores follow the same
        // selected-room projection boundary. Hidden room filenames, captions,
        // and MXC URIs must not leak through the root AppState DTO.
        assert_eq!(value["state"]["domain"]["upload_staging"], json!(null));
        assert_eq!(value["state"]["domain"]["media_gallery"], json!(null));
        assert_eq!(
            value["state"]["ui"]["timeline"]["scheduled_send_capability"],
            json!("unknown")
        );
        assert_eq!(
            value["state"]["ui"]["timeline"]["scheduled_sends"],
            json!([])
        );
        assert_eq!(
            value["state"]["ui"]["timeline"]["staged_uploads"],
            json!([])
        );
        assert_eq!(value["state"]["ui"]["timeline"]["media_gallery"], json!([]));
    }

    #[test]
    fn frontend_snapshot_serializes_invite_previews() {
        let mut state = booted_app_state();
        state.invites.push(InvitePreview {
            room_id: "!invite:matrix.org".to_owned(),
            display_name: "Project invite".to_owned(),
            avatar: None,
            topic: Some("Project topic".to_owned()),
            inviter_display_name: Some("Inviter".to_owned()),
            inviter_user_id: Some("@inviter:matrix.org".to_owned()),
            is_dm: true,
        });

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["domain"]["invites"],
            json!([
                {
                    "room_id": "!invite:matrix.org",
                    "display_name": "Project invite",
                    "avatar": null,
                    "topic": "Project topic",
                    "inviter_display_name": "Inviter",
                    "inviter_user_id": "@inviter:matrix.org",
                    "is_dm": true
                }
            ])
        );
    }

    #[test]
    fn frontend_snapshot_can_carry_state_delta_generation_for_reset_recovery() {
        let state = booted_app_state();
        let value = serde_json::to_value(FrontendDesktopSnapshot::from_versioned(state, 7))
            .expect("versioned snapshot should serialize");

        assert_eq!(value["state_generation"], json!(7));
        assert_eq!(value["state"]["domain"]["session"]["kind"], json!("ready"));
    }

    #[test]
    fn frontend_snapshot_serializes_profile_and_summary_avatars() {
        let mut state = booted_app_state();
        let ready_avatar = AvatarImage {
            mxc_uri: "mxc://matrix.org/avatar".to_owned(),
            thumbnail: AvatarThumbnailState::Ready {
                source_url: "asset://avatar".to_owned(),
                width: Some(64),
                height: Some(64),
                mime_type: Some("image/png".to_owned()),
            },
        };
        let room_avatar = AvatarImage {
            mxc_uri: "mxc://matrix.org/room".to_owned(),
            thumbnail: AvatarThumbnailState::NotRequested,
        };
        state.profile.own = OwnProfile {
            display_name: Some("Alice".to_owned()),
            avatar: Some(ready_avatar.clone()),
        };
        state.profile.users.insert(
            "@bob:matrix.org".to_owned(),
            UserProfile {
                user_id: "@bob:matrix.org".to_owned(),
                display_name: Some("Bob".to_owned()),
                display_label: "Bob".to_owned(),
                original_display_label: "Bob".to_owned(),
                mention_search_terms: vec!["Bob".to_owned(), "@bob:matrix.org".to_owned()],
                avatar: Some(ready_avatar),
            },
        );
        state.spaces.push(SpaceSummary {
            space_id: "!space:matrix.org".to_owned(),
            display_name: "Space".to_owned(),
            avatar: Some(room_avatar.clone()),
            child_room_ids: vec![],
        });
        state.rooms.push(RoomSummary {
            room_id: "!room:matrix.org".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: Some(room_avatar),
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 2,
            notification_count: 2,
            highlight_count: 1,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec![],
            dm_space_ids: vec![],
            is_encrypted: false,
            joined_members: 0,
        });

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["domain"]["profile"]["own"],
            json!({
                "display_name": "Alice",
                "avatar": {
                    "mxc_uri": "mxc://matrix.org/avatar",
                    "thumbnail": {
                        "kind": "ready",
                        "source_url": "asset://avatar",
                        "width": 64,
                        "height": 64,
                        "mime_type": "image/png"
                    }
                }
            })
        );
        assert_eq!(
            value["state"]["domain"]["profile"]["users"]["@bob:matrix.org"]["avatar"]["thumbnail"]
                ["kind"],
            json!("ready")
        );
        assert_eq!(
            value["state"]["domain"]["profile"]["users"]["@bob:matrix.org"]["original_display_label"],
            json!("Bob")
        );
        assert_eq!(
            value["state"]["domain"]["spaces"][0]["avatar"],
            json!({
                "mxc_uri": "mxc://matrix.org/room",
                "thumbnail": { "kind": "notRequested" }
            })
        );
        assert_eq!(
            value["state"]["domain"]["rooms"][0]["avatar"],
            json!({
                "mxc_uri": "mxc://matrix.org/room",
                "thumbnail": { "kind": "notRequested" }
            })
        );
        assert_eq!(
            value["state"]["domain"]["rooms"][0]["original_display_label"],
            json!("Room")
        );
        assert_eq!(
            value["state"]["domain"]["rooms"][0]["dm_space_ids"],
            json!([])
        );
        assert_eq!(
            value["sidebar"]["account_home"]["highlight_count"],
            json!(1)
        );
        assert_eq!(
            value["sidebar"]["space_rooms"][0]["highlight_count"],
            json!(1)
        );
        assert_eq!(value["sidebar"]["space_highlight_count"], json!(1));
    }

    #[test]
    fn frontend_snapshot_locale_profile_follows_rust_owned_locale_settings() {
        let mut state = booted_app_state();
        state.settings.values.locale = LocaleSettings {
            language_tag: Some("ar-XB".to_owned()),
            text_direction: TextDirectionPreference::Auto,
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["domain"]["locale_profile"]["lang"],
            json!("ar-XB")
        );
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["dir"],
            json!("rtl")
        );
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["catalog_locale"],
            json!("pseudo")
        );
        assert_eq!(
            value["state"]["domain"]["locale_profile"]["pseudo_locale"],
            json!("bidi")
        );
        assert_ne!(
            value["state"]["domain"]["locale_profile"]["modifier_labels"]["primary"],
            json!(null)
        );
    }

    #[test]
    fn frontend_snapshot_typography_profile_follows_rust_owned_typography_settings() {
        let mut state = booted_app_state();
        state.settings.values.typography = TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["domain"]["typography_profile"]["font"],
            json!("inter")
        );
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["emoji"],
            json!("twemojiColr")
        );
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["font_asset"],
            json!("bundledPreferred")
        );
        assert_eq!(
            value["state"]["domain"]["typography_profile"]["emoji_asset"],
            json!("bundledPreferred")
        );
        assert_ne!(
            value["state"]["domain"]["typography_profile"]["platform"],
            json!(null)
        );
    }

    #[test]
    fn frontend_snapshot_serializes_verification_gate() {
        let state = AppState {
            session: SessionState::AwaitingVerification {
                info: SessionInfo {
                    homeserver: "https://matrix.org".to_owned(),
                    user_id: "@user:matrix.org".to_owned(),
                    device_id: "DEVICE".to_owned(),
                },
                gate: koushi_state::VerificationGateState {
                    methods: vec![
                        koushi_state::VerificationMethodCapability::RecoveryKey,
                        koushi_state::VerificationMethodCapability::SecurityPhrase,
                    ],
                    account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
                    failure: None,
                },
            },
            sync: SyncState::Running,
            ..AppState::default()
        };

        let value = serde_json::to_value(FrontendDesktopSnapshot::from(state))
            .expect("snapshot should serialize");

        assert_eq!(
            value["state"]["domain"]["session"]["kind"],
            json!("awaitingVerification")
        );
        assert_eq!(
            value["state"]["domain"]["session"]["gate"]["methods"],
            json!(["recoveryKey", "securityPhrase"])
        );
        assert_eq!(value["state"]["domain"]["sync"], json!("running"));
    }

    #[test]
    fn frontend_session_gate_variants_are_private_safe_json() {
        let info = SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: "@private:example.invalid".into(),
            device_id: "PRIVATEDEVICE".into(),
        };
        let gate = koushi_state::VerificationGateState {
            methods: vec![
                koushi_state::VerificationMethodCapability::ExistingDeviceSas,
                koushi_state::VerificationMethodCapability::Bootstrap,
            ],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: Some(koushi_state::VerificationGateFailureKind::Network),
        };
        let sessions = [
            SessionState::Provisional {
                info: info.clone(),
                phase: koushi_state::ProvisionalPhase::RecheckingTrust {
                    failure: Some(koushi_state::VerificationGateFailureKind::Timeout),
                },
            },
            SessionState::AwaitingVerification {
                info: info.clone(),
                gate: gate.clone(),
            },
            SessionState::Verifying {
                info: info.clone(),
                gate: gate.clone(),
                method: koushi_state::VerificationMethod::ExistingDeviceSas,
                flow_id: 7,
                sas_emojis: vec![
                    koushi_state::SasEmoji {
                        symbol: "🐶".into(),
                        description: "dog".into()
                    };
                    7
                ],
            },
            SessionState::AwaitingBootstrapConfirmation {
                info: info.clone(),
                gate: gate.clone(),
                flow_id: 8,
                destination_written: true,
            },
            SessionState::Rejecting {
                info,
                reason: koushi_state::VerificationGateRejectReason::UserRejected,
            },
        ];
        let expected = [
            "provisional",
            "awaitingVerification",
            "verifying",
            "awaitingBootstrapConfirmation",
            "rejecting",
        ];
        for (session, kind) in sessions.into_iter().zip(expected) {
            let value =
                serde_json::to_value(super::FrontendSessionState::from(session)).expect("gate DTO");
            assert_eq!(value["kind"], kind);
            let wire = value.to_string();
            assert!(!wire.contains("secret"));
            assert!(!wire.contains("destination_path"));
            assert!(!wire.contains("target_user"));
        }
    }

    #[test]
    fn frontend_sync_state_serializes_failed_and_reconnecting() {
        assert_eq!(
            serde_json::to_value(FrontendSyncState::from(SyncState::Failed {
                reason: "limited network".to_owned(),
            }))
            .expect("failed sync should serialize"),
            json!({ "failed": "limited network" })
        );
        assert_eq!(
            serde_json::to_value(FrontendSyncState::from(SyncState::Reconnecting {
                reason: "limited network".to_owned(),
            }))
            .expect("reconnecting sync should serialize"),
            json!({ "reconnecting": "limited network" })
        );
    }

    /// Characterization / golden test for the complete `FrontendAppState` DTO wire shape.
    ///
    /// Purpose: lock in the exact JSON serialization of `FrontendDesktopSnapshot` so any
    /// later Phase 2 (file splits) or Phase 4 (domain/ui DTO reorg) that silently drops,
    /// renames, or reorders a field is caught immediately.
    ///
    /// The golden artifact lives at `tests/golden/frontend_app_state.json`.
    ///
    /// To regenerate: `UPDATE_GOLDEN=1 cargo test -p koushi-desktop frontend_app_state_golden_matches_maximally_populated_state`
    ///
    /// When to regenerate: ONLY after an intentional, reviewed DTO change (Phase 4 etc.).
    /// A failing golden test with no intentional change signals an accidental field loss —
    /// investigate before regenerating.
    #[test]
    fn frontend_app_state_golden_matches_maximally_populated_state() {
        use koushi_state::{
            ActivityMarkReadState, ActivityRow, ActivityState, ActivityStream, ActivityTab,
            AttachmentFilter, AttachmentKind, AttachmentResult, AttachmentScope, AttachmentSort,
            AvatarImage, AvatarThumbnailState, BasicOperationState, CrossSigningStatus,
            DirectoryJoinState, DirectoryQuery, DirectoryQueryState, DirectoryRoomSummary,
            DirectoryState, E2eeKeyManagementState, E2eeTrustState, FilesViewState,
            FocusedContextState, IdentityResetState, InvitePreview, KeyBackupStatus,
            LiveSignalsState, LocalEncryptionState, MediaTransferProgress,
            NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
            NativeAttentionDispatchState, NativeAttentionState, NativeAttentionSummary,
            NavigationState, OwnProfile, PinOp, PinOperationState, PinnedEvent, RoomAttentionKind,
            RoomHistoryVisibility, RoomInteractionState, RoomJoinRule, RoomLiveSignals,
            RoomManagementOperationState, RoomManagementState, RoomMemberRole, RoomMemberSummary,
            RoomNotificationSettings, RoomPermissionFacts, RoomSettingsSnapshot, RoomSummary,
            RoomTags, SearchMatchField, SearchMatchKind, SearchResult, SearchScope, SearchState,
            SessionInfo, SessionState, SpaceSummary, SyncState, TextRange, ThreadAttentionState,
            ThreadsListItem, ThreadsListState, TimelineMediaDownloadState, TimelinePaneState,
            UserProfile, VerificationFlowState,
        };
        use std::collections::BTreeMap;

        // Construct a maximally-populated AppState. Every section gets at least one
        // non-default field so that Phase 2/4 refactors cannot silently drop it.
        // All identifiers are synthetic (example.invalid / fixture pattern).
        let session_info = SessionInfo {
            homeserver: "https://matrix.example.invalid".to_owned(),
            user_id: "@fixture:example.invalid".to_owned(),
            device_id: "FIXTURE_DEVICE".to_owned(),
        };
        let avatar = AvatarImage {
            mxc_uri: "mxc://example.invalid/fixture-avatar".to_owned(),
            thumbnail: AvatarThumbnailState::Ready {
                source_url: "asset://fixture-avatar".to_owned(),
                width: Some(64),
                height: Some(64),
                mime_type: Some("image/png".to_owned()),
            },
        };

        let mut state = AppState {
            session: SessionState::Ready(session_info.clone()),
            sync: SyncState::Running,
            ..AppState::default()
        };

        // profile — own + one cached user
        state.profile.own = OwnProfile {
            display_name: Some("Fixture User".to_owned()),
            avatar: Some(avatar.clone()),
        };
        state.profile.users.insert(
            "@other:example.invalid".to_owned(),
            UserProfile {
                user_id: "@other:example.invalid".to_owned(),
                display_name: Some("Other Fixture".to_owned()),
                display_label: "Other Fixture".to_owned(),
                original_display_label: "Other Fixture".to_owned(),
                mention_search_terms: vec!["other".to_owned()],
                avatar: Some(avatar.clone()),
            },
        );

        // spaces + rooms
        state.spaces.push(SpaceSummary {
            space_id: "!space:example.invalid".to_owned(),
            display_name: "Fixture Space".to_owned(),
            avatar: None,
            child_room_ids: vec!["!room:example.invalid".to_owned()],
        });
        state.rooms.push(RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Fixture Room".to_owned(),
            display_label: "Fixture Room".to_owned(),
            original_display_label: "Fixture Room".to_owned(),
            avatar: Some(avatar.clone()),
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 2,
            highlight_count: 1,
            marked_unread: false,
            recency_stamp: Some(1_000_000),
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["!space:example.invalid".to_owned()],
            dm_space_ids: vec![],
            is_encrypted: true,
            joined_members: 4,
        });

        // invites
        state.invites.push(InvitePreview {
            room_id: "!invite:example.invalid".to_owned(),
            display_name: "Fixture Invite".to_owned(),
            avatar: None,
            topic: Some("Fixture invite topic".to_owned()),
            inviter_display_name: Some("Inviter".to_owned()),
            inviter_user_id: Some("@inviter:example.invalid".to_owned()),
            is_dm: false,
        });

        // navigation — active room + space
        state.navigation = NavigationState {
            active_room_id: Some("!room:example.invalid".to_owned()),
            active_space_id: Some("!space:example.invalid".to_owned()),
            space_order: vec!["!space:example.invalid".to_owned()],
            last_room_by_space_id: BTreeMap::new(),
            room_scroll_anchors: BTreeMap::new(),
            main_timeline_anchor: None,
        };

        // room_interactions
        state.room_interactions.insert(
            "!room:example.invalid".to_owned(),
            RoomInteractionState {
                pinned_events: vec![PinnedEvent {
                    event_id: "$pinned:example.invalid".to_owned(),
                    sender: Some("@fixture:example.invalid".to_owned()),
                    body_preview: Some("Pinned fixture message".to_owned()),
                    redacted: false,
                }],
                pin_operation: PinOperationState::Pending {
                    request_id: 42,
                    room_id: "!room:example.invalid".to_owned(),
                    event_id: "$pinned:example.invalid".to_owned(),
                    op: PinOp::Pin,
                },
            },
        );

        // room_notification_settings
        state.room_notification_settings.insert(
            "!room:example.invalid".to_owned(),
            RoomNotificationSettings::default(),
        );

        // room_management — with settings snapshot
        state.room_management = RoomManagementState {
            selected_room_id: Some("!room:example.invalid".to_owned()),
            settings: Some(RoomSettingsSnapshot {
                room_id: "!room:example.invalid".to_owned(),
                name: Some("Fixture Room".to_owned()),
                topic: Some("Fixture room topic".to_owned()),
                avatar_url: Some("mxc://example.invalid/room-avatar".to_owned()),
                canonical_alias: None,
                alternate_aliases: Vec::new(),
                share_link: None,
                join_rule: RoomJoinRule::Invite,
                history_visibility: RoomHistoryVisibility::Shared,
                permissions: RoomPermissionFacts {
                    can_edit_settings: true,
                    can_edit_roles: true,
                    can_kick: true,
                    can_ban: true,
                    can_unban: true,
                },
                members: vec![RoomMemberSummary {
                    user_id: "@fixture:example.invalid".to_owned(),
                    display_name: Some("Fixture User".to_owned()),
                    display_label: "Fixture User".to_owned(),
                    original_display_label: "Fixture User".to_owned(),
                    avatar_url: None,
                    power_level: Some(100),
                    role: RoomMemberRole::Administrator,
                    user_trust: None,
                }],
            }),
            operation: RoomManagementOperationState::Idle,
        };

        // activity — open with populated streams
        state.activity = ActivityState::Open {
            active_tab: ActivityTab::Recent,
            recent: ActivityStream {
                rows: vec![ActivityRow {
                    kind: koushi_state::ActivityRowKind::Event,
                    room_id: "!room:example.invalid".to_owned(),
                    event_id: Some("$act:example.invalid".to_owned()),
                    room_label: "Fixture Room".to_owned(),
                    sender_label: Some("Fixture User".to_owned()),
                    preview: Some("Activity preview".to_owned()),
                    timestamp_ms: 500_000,
                    unread: false,
                    highlight: false,
                    ..Default::default()
                }],
                next_batch: None,
                resolution: Default::default(),
            },
            unread: ActivityStream {
                rows: vec![ActivityRow::room_unread_placeholder(
                    "!placeholder:example.invalid".to_owned(),
                    "Placeholder Room".to_owned(),
                    499_000,
                    true,
                )],
                next_batch: None,
                resolution: Default::default(),
            },
            mark_read: ActivityMarkReadState::Idle,
        };

        // timeline — composer + media_downloads populated
        let mut composer = koushi_state::ComposerState::default();
        composer
            .accepted_submission_ids
            .push_back(koushi_state::SubmissionId::new("accepted-contract"));
        state.timeline = TimelinePaneState {
            room_id: Some("!room:example.invalid".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer,
            submission_registry: koushi_state::ComposerSubmissionRegistry {
                accepted_submission_ids: [koushi_state::SubmissionId::new("global-accepted")]
                    .into_iter()
                    .collect(),
                settled_submission_ids: [koushi_state::SubmissionId::new("global-settled")]
                    .into_iter()
                    .collect(),
                active_submissions: Default::default(),
            },
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: {
                let mut m = BTreeMap::new();
                m.insert(
                    "$media:example.invalid".to_owned(),
                    TimelineMediaDownloadState::Pending {
                        progress: Some(MediaTransferProgress {
                            current: 10,
                            total: 100,
                        }),
                    },
                );
                m
            },
            continuity: koushi_state::TimelineContinuityState::FailedIncomplete {
                generation: 7,
                gap_count: 2,
                batches_processed: 3,
                failure_kind: koushi_state::TimelineGapRepairFailureKind::Sdk,
            },
        };

        // live_signals — one room entry
        state.live_signals = LiveSignalsState {
            rooms: {
                let mut m = BTreeMap::new();
                m.insert(
                    "!room:example.invalid".to_owned(),
                    RoomLiveSignals {
                        receipts_by_event: BTreeMap::new(),
                        fully_read_event_id: Some("$read:example.invalid".to_owned()),
                        typing_user_ids: vec!["@other:example.invalid".to_owned()],
                    },
                );
                m
            },
            presence: {
                let mut m = BTreeMap::new();
                m.insert(
                    "@fixture:example.invalid".to_owned(),
                    koushi_state::PresenceKind::Online,
                );
                m
            },
        };

        // e2ee_trust — non-default fields
        state.e2ee_trust = E2eeTrustState {
            verification: VerificationFlowState::Idle,
            cross_signing: CrossSigningStatus::Trusted,
            key_backup: KeyBackupStatus::Enabled {
                version: "v1".to_owned(),
            },
            identity_reset: IdentityResetState::Idle,
            key_management: E2eeKeyManagementState::default(),
            devices: Vec::new(),
        };

        // local_encryption
        state.local_encryption = LocalEncryptionState::Healthy;

        // native_attention — non-default capabilities
        state.native_attention = NativeAttentionState {
            summary: NativeAttentionSummary {
                unread_count: 5,
                highlight_count: 2,
                badge_count: 5,
                candidate: Some(NativeAttentionCandidate {
                    room_display_name: "Fixture Room".to_owned(),
                    kind: RoomAttentionKind::Message,
                    unread_count: 1,
                    highlight_count: 1,
                }),
                capabilities: NativeAttentionCapabilities {
                    notifications: NativeAttentionCapability::Available,
                    badge: NativeAttentionCapability::Available,
                    overlay_icon: NativeAttentionCapability::Unavailable,
                    sound: NativeAttentionCapability::Available,
                    tray: NativeAttentionCapability::Unknown,
                    activation: NativeAttentionCapability::Available,
                },
            },
            dispatch: NativeAttentionDispatchState::Idle,
        };

        // directory — Results with one entry + Joining join state
        state.directory = DirectoryState {
            query: DirectoryQueryState::Results {
                request_id: 7,
                query: DirectoryQuery {
                    term: Some("fixture".to_owned()),
                    server_name: None,
                    limit: Some(20),
                    since: None,
                },
                rooms: vec![DirectoryRoomSummary {
                    room_id: "!dir:example.invalid".to_owned(),
                    canonical_alias: Some("#fixture:example.invalid".to_owned()),
                    name: "Fixture Public Room".to_owned(),
                    topic: Some("Fixture topic".to_owned()),
                    avatar_url: None,
                    joined_members: 42,
                    world_readable: true,
                    guest_can_join: false,
                }],
                next_batch: None,
            },
            join: DirectoryJoinState::Joining {
                request_id: 8,
                alias: "#fixture:example.invalid".to_owned(),
                via_server: None,
            },
        };

        // focused_context — Open referencing a synthetic event
        state.focused_context = FocusedContextState::Open {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$focused:example.invalid".to_owned(),
            is_subscribed: true,
        };

        // search — Results with one entry
        state.search = SearchState::Results {
            request_id: 9,
            query: "fixture query".to_owned(),
            scope: SearchScope::AllRooms,
            results: vec![SearchResult {
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$search:example.invalid".to_owned(),
                sender: "@fixture:example.invalid".to_owned(),
                timestamp_ms: 600_000,
                score_millis: 950,
                snippet: "Fixture search snippet".to_owned(),
                match_field: SearchMatchField::MessageBody,
                highlights: vec![TextRange {
                    start_utf16: 8,
                    end_utf16: 14,
                }],
                match_kind: SearchMatchKind::Exact,
            }],
        };

        // files_view — Open with one attachment entry
        state.files_view = FilesViewState::Open {
            request_id: 10,
            scope: AttachmentScope::Room {
                room_id: "!room:example.invalid".to_owned(),
            },
            filter: AttachmentFilter {
                kinds: vec![AttachmentKind::Image],
                filename_query: None,
            },
            sort: AttachmentSort::NewestFirst,
            items: vec![AttachmentResult {
                event_id: "$attach:example.invalid".to_owned(),
                filename: "fixture.png".to_owned(),
                kind: AttachmentKind::Image,
                mimetype: Some("image/png".to_owned()),
                room_id: "!room:example.invalid".to_owned(),
                sender: "@fixture:example.invalid".to_owned(),
                size: Some(1024),
                source_mxc: "mxc://example.invalid/attach".to_owned(),
                thumbnail_mxc: None,
                timestamp_ms: 700_000,
                thread_root: None,
                encrypted: false,
                encryption_version: None,
                width: Some(128),
                height: Some(128),
                is_edited: false,
            }],
            selected_event_id: Some("$attach:example.invalid".to_owned()),
        };

        // threads_list — Open with one thread row
        state.threads_list = ThreadsListState::Open {
            room_id: "!room:example.invalid".to_owned(),
            request_id: 11,
            items: vec![ThreadsListItem {
                root_event_id: "$thread-root:example.invalid".to_owned(),
                root_sender: "@fixture:example.invalid".to_owned(),
                root_sender_label: Some("Fixture User".to_owned()),
                root_body_preview: Some("Thread root preview".to_owned()),
                root_timestamp_ms: Some(800_000),
                latest_event_id: Some("$thread-reply:example.invalid".to_owned()),
                latest_sender: Some("@other:example.invalid".to_owned()),
                latest_sender_label: Some("Other Fixture".to_owned()),
                latest_body_preview: Some("Latest reply preview".to_owned()),
                latest_timestamp_ms: Some(810_000),
                reply_count: 3,
            }],
            is_paginating: false,
            end_reached: false,
        };

        // thread_attention — Tracking with non-zero counts
        state.thread_attention = ThreadAttentionState::Tracking {
            room_id: "!room:example.invalid".to_owned(),
            root_event_id: "$thread-root:example.invalid".to_owned(),
            notification_count: 4,
            highlight_count: 1,
            live_event_marker_count: 2,
        };

        // basic_operation — non-default (creating room)
        state.basic_operation = BasicOperationState::CreatingRoom {
            request_id: 1,
            name: "Fixture New Room".to_owned(),
        };

        // Serialize
        let sidebar = koushi_state::compose_sidebar(
            state.navigation.active_space_id.as_deref(),
            &state.spaces,
            &state.rooms,
        );
        let value = serde_json::to_value(FrontendDesktopSnapshot {
            state_generation: None,
            state: super::frontend_app_state_for_platform(
                state,
                koushi_state::DisplayPlatform::Linux,
            ),
            sidebar,
            timeline: Vec::new(),
            thread: None,
        })
        .expect("maximally-populated state should serialize to JSON");

        let golden_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/frontend_app_state.json"
        );

        if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
            let pretty = serde_json::to_string_pretty(&value).expect("should format golden JSON");
            std::fs::create_dir_all(std::path::Path::new(golden_path).parent().unwrap())
                .expect("golden directory should be creatable");
            std::fs::write(golden_path, pretty).expect("golden artifact should be writable");
            return;
        }

        let golden_bytes = std::fs::read(golden_path).unwrap_or_else(|_| {
            panic!(
                "golden artifact not found at {golden_path}. \
                Run with UPDATE_GOLDEN=1 to generate it."
            )
        });
        let golden: serde_json::Value =
            serde_json::from_slice(&golden_bytes).expect("golden artifact must be valid JSON");

        assert_eq!(
            value, golden,
            "FrontendAppState wire shape changed — if intentional, regenerate with UPDATE_GOLDEN=1"
        );
    }
}
