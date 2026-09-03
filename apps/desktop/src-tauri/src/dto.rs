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

use koushi_protocol::{CoreCommandAdmission, StateDelta, VersionedAppStateSnapshot};
use koushi_state::{
    AccountManagementCapabilities, AccountManagementState, AccountManagementUrl, ActivityState,
    AppError, AppState, AuthDiscoveryState, BasicOperationState, CjkTextPolicyState, ComposerState,
    CurrentSessionStatusState, DeviceCleanupState, DirectoryState, DisplayPlatform, E2eeTrustState,
    FilesViewState, FocusedContextState, InvitePreview, InviteWorkflowState,
    LinkPreviewSettingsState, LiveSignalsState, LocalEncryptionState, LocaleDisplayProfile,
    MentionCandidatesState, NativeAttentionCapabilities, NativeAttentionState, NavigationState,
    ProfileState, ProvisionalPhase, QrLoginState, RoomInteractionState, RoomListProjection,
    RoomManagementState, RoomNotificationSettings, RoomPreferencesState, RoomSummary,
    SearchCrawlerState, SearchMatchField, SearchMatchKind, SearchResult, SearchScope, SearchState,
    SecureBackupGateState, SessionLockReason, SessionState, SettingsState, SidebarModel,
    SoftLogoutReauthState, SpaceMembersState, SpaceSummary, StagedUploadItem, SyncState,
    ThreadAttentionState, ThreadPaneState, ThreadsListState, TimelinePaneState,
    TypographyDisplayProfile, VerificationGateRejectReason, VerificationGateState,
    VerificationMethod, native_attention_capabilities_for_platform, resolve_locale_display_profile,
    resolve_typography_display_profile,
};
use serde::{Deserialize, Serialize};

/// The snapshot returned only by initial, settlement-resync, and gap-resync reads.
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
        // Use the same account facts the delta path uses
        // (`koushi_core::state_delta`). Composing from rooms and spaces alone
        // silently dropped mute filtering and the invite count, so a full
        // snapshot and a delta reported different Home badge values for the
        // same state.
        let sidebar = koushi_state::compose_sidebar_for_state(&state);
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

pub const STATE_UPDATE_PROTOCOL_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendCommandAdmission {
    protocol_version: u8,
    admitted_generation: u64,
}

impl FrontendCommandAdmission {
    pub(crate) fn from_core(admission: CoreCommandAdmission) -> Self {
        Self {
            protocol_version: STATE_UPDATE_PROTOCOL_VERSION,
            admitted_generation: admission.admitted_generation,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendCommandResult<T> {
    pub result: T,
    pub settlement: FrontendCommandSettlement,
}

impl<T> FrontendCommandResult<T> {
    pub(crate) fn new(result: T, settlement: FrontendCommandSettlement) -> Self {
        Self { result, settlement }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendCommandSettlement {
    protocol_version: u8,
    published_generation: u64,
}

impl FrontendCommandSettlement {
    pub(crate) fn from_published_generation(published_generation: u64) -> Self {
        Self {
            protocol_version: STATE_UPDATE_PROTOCOL_VERSION,
            published_generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StateUpdateSnapshotReason {
    Initial,
    Gap,
    Lag,
    Settlement,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FrontendStateUpdateEnvelope {
    Delta {
        protocol_version: u8,
        generation: u64,
        changed: FrontendDesktopSnapshotChangedSlices,
    },
    Snapshot {
        protocol_version: u8,
        generation: u64,
        snapshot: FrontendDesktopSnapshot,
        reason: StateUpdateSnapshotReason,
    },
}

impl FrontendStateUpdateEnvelope {
    pub(crate) fn delta(delta: FrontendDesktopSnapshotDelta) -> Self {
        Self::Delta {
            protocol_version: STATE_UPDATE_PROTOCOL_VERSION,
            generation: delta.generation,
            changed: delta.changed,
        }
    }

    pub(crate) fn snapshot(
        snapshot: VersionedAppStateSnapshot,
        reason: StateUpdateSnapshotReason,
    ) -> Self {
        Self::Snapshot {
            protocol_version: STATE_UPDATE_PROTOCOL_VERSION,
            generation: snapshot.generation,
            snapshot: FrontendDesktopSnapshot::from_versioned(snapshot.state, snapshot.generation),
            reason,
        }
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
    pub session_lock_reason: Option<Option<SessionLockReason>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure_backup_gate: Option<SecureBackupGateState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_session_status: Option<CurrentSessionStatusState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_cleanup: Option<DeviceCleanupState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthDiscoveryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_management_url: Option<Option<AccountManagementUrl>>,
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
    pub space_members: Option<SpaceMembersState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<FrontendSyncState>,
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
    pub mention_candidates: Option<MentionCandidatesState>,
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
            && self.session_lock_reason.is_none()
            && self.secure_backup_gate.is_none()
            && self.current_session_status.is_none()
            && self.device_cleanup.is_none()
            && self.auth.is_none()
            && self.account_management_url.is_none()
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
            && self.space_members.is_none()
            && self.sync.is_none()
            && self.spaces.is_none()
            && self.rooms.is_none()
            && self.invites.is_none()
            && self.invite_workflow.is_none()
            && self.room_notification_settings.is_none()
            && self.room_interactions.is_none()
            && self.directory.is_none()
            && self.room_management.is_none()
            && self.mention_candidates.is_none()
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
        domain.session_lock_reason = changed.session_lock_reason;
        domain.secure_backup_gate = changed.secure_backup_gate;
        domain.current_session_status = changed.current_session_status;
        domain.device_cleanup = changed.device_cleanup;
        domain.auth = changed.auth;
        domain.account_management_url = changed.account_management_url;
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
        domain.space_members = changed.space_members;
        domain.sync = changed.sync.map(Into::into);
        domain.spaces = changed.spaces;
        domain.rooms = changed.rooms;
        domain.invites = changed.invites;
        domain.invite_workflow = changed.invite_workflow;
        domain.room_notification_settings = changed.room_notification_settings;
        domain.room_interactions = changed.room_interactions;
        domain.directory = changed.directory;
        domain.room_management = changed.room_management;
        domain.mention_candidates = changed.mention_candidates;
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
                    native_attention_capabilities_for_platform(platform)
                        .with_tray(crate::tray::observed_tray_capability());
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
    /// IPC snapshot contract version. v6 carries opaque renderable-thumbnail
    /// references instead of Core-minted Tauri URLs. The renderer asserts this so a stale snapshot or
    /// a mismatched Rust/TS build fails loudly instead of reading `undefined`.
    pub schema_version: u32,
    pub domain: FrontendDomainState,
    pub ui: FrontendUiState,
}

/// Matrix/product state — Rust-owned, reusable by a future mobile shell.
#[derive(Clone, Debug, Serialize)]
pub struct FrontendDomainState {
    pub session: FrontendSessionState,
    pub session_lock_reason: Option<SessionLockReason>,
    pub secure_backup_gate: SecureBackupGateState,
    pub current_session_status: CurrentSessionStatusState,
    pub device_cleanup: DeviceCleanupState,
    pub auth: AuthDiscoveryState,
    pub account_management_url: Option<AccountManagementUrl>,
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
    pub space_members: SpaceMembersState,
    pub sync: FrontendSyncState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub invites: Vec<InvitePreview>,
    pub invite_workflow: InviteWorkflowState,
    pub room_notification_settings: std::collections::HashMap<String, RoomNotificationSettings>,
    pub room_interactions: BTreeMap<String, RoomInteractionState>,
    pub directory: DirectoryState,
    pub room_management: RoomManagementState,
    pub mention_candidates: MentionCandidatesState,
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
            native_attention_capabilities_for_platform(platform)
                .with_tray(crate::tray::observed_tray_capability());
    }
    FrontendAppState {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        domain: FrontendDomainState {
            session: state.session.into(),
            session_lock_reason: state.session_lock_reason,
            secure_backup_gate: state.secure_backup_gate,
            current_session_status: state.current_session_status,
            device_cleanup: state.device_cleanup,
            auth: state.auth,
            account_management_url: state.account_management_url,
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
            space_members: state.space_members,
            sync: state.sync.into(),
            spaces: state.spaces,
            rooms: state.rooms,
            invites: state.invites,
            invite_workflow: state.invite_workflow,
            room_notification_settings: state.room_notification_settings,
            room_interactions: state.room_interactions,
            directory: state.directory,
            room_management: state.room_management,
            mention_candidates: state.mention_candidates,
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
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 6;

pub(crate) fn frontend_display_platform() -> DisplayPlatform {
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
    CapabilityBlocked {
        homeserver: String,
        user_id: String,
        device_id: String,
        failure: koushi_state::SlidingSyncCapabilityFailureKind,
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
            SessionState::CapabilityBlocked { info, failure } => Self::CapabilityBlocked {
                homeserver: info.homeserver,
                user_id: info.user_id,
                device_id: info.device_id,
                failure,
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
        intent: koushi_state::ThreadOpenIntent,
    },
    Open {
        room_id: String,
        root_event_id: String,
        intent: koushi_state::ThreadOpenIntent,
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
                intent,
            } => Self::Opening {
                room_id,
                root_event_id,
                intent,
            },
            ThreadPaneState::Open {
                room_id,
                root_event_id,
                intent,
                is_subscribed,
                composer,
                staged_uploads,
            } => Self::Open {
                room_id,
                root_event_id,
                intent,
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
    TooShort {
        request_id: u64,
        query: String,
        scope: SearchScopeKind,
        min_chars: u8,
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
            SearchState::TooShort {
                request_id,
                query,
                scope,
                min_chars,
            } => Self::TooShort {
                request_id,
                query,
                scope: scope.into(),
                min_chars,
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
                .unwrap_or_else(|| SearchScope::CurrentRoom {
                    room_id: String::new(),
                }),
            Self::CurrentSpace => state
                .navigation
                .active_space_id
                .as_ref()
                .map(|space_id| SearchScope::CurrentSpace {
                    space_id: space_id.clone(),
                })
                .unwrap_or_else(|| SearchScope::CurrentSpace {
                    space_id: String::new(),
                }),
            Self::AllRooms => SearchScope::AllRooms,
        }
    }
}

impl From<SearchScope> for SearchScopeKind {
    fn from(scope: SearchScope) -> Self {
        match scope {
            SearchScope::CurrentRoom { .. } => Self::CurrentRoom,
            SearchScope::CurrentSpace { .. } => Self::CurrentSpace,
            SearchScope::AllRooms => Self::AllRooms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FrontendSearchResult {
    pub room_id: String,
    pub event_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_label: Option<String>,
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
            context_label: result.context_label,
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
mod tests;
