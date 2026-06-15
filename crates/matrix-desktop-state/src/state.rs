use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub session: SessionState,
    pub auth: AuthDiscoveryState,
    pub settings: SettingsState,
    pub profile: ProfileState,
    pub sync: SyncState,
    pub navigation: NavigationState,
    pub spaces: Vec<SpaceSummary>,
    pub rooms: Vec<RoomSummary>,
    pub invites: Vec<InvitePreview>,
    pub timeline: TimelinePaneState,
    pub thread: ThreadPaneState,
    pub focused_context: FocusedContextState,
    pub search: SearchState,
    pub basic_operation: BasicOperationState,
    pub live_signals: LiveSignalsState,
    pub e2ee_trust: E2eeTrustState,
    pub errors: Vec<AppError>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::SignedOut,
            auth: AuthDiscoveryState::Unknown,
            settings: SettingsState::default(),
            profile: ProfileState::default(),
            sync: SyncState::Stopped,
            navigation: NavigationState::default(),
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
            timeline: TimelinePaneState::default(),
            thread: ThreadPaneState::Closed,
            focused_context: FocusedContextState::Closed,
            search: SearchState::Closed,
            basic_operation: BasicOperationState::Idle,
            live_signals: LiveSignalsState::default(),
            e2ee_trust: E2eeTrustState::default(),
            errors: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsState {
    pub values: SettingsValues,
    pub persistence: SettingsPersistenceState,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            values: SettingsValues::default(),
            persistence: SettingsPersistenceState::Idle,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsValues {
    pub locale: LocaleSettings,
    pub appearance: AppearanceSettings,
    pub typography: TypographySettings,
    pub keyboard: KeyboardSettings,
}

impl SettingsValues {
    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        if let Some(locale) = patch.locale {
            self.locale = locale;
        }
        if let Some(appearance) = patch.appearance {
            self.appearance = appearance;
        }
        if let Some(typography) = patch.typography {
            self.typography = typography;
        }
        if let Some(keyboard) = patch.keyboard {
            self.keyboard = keyboard;
        }
    }
}

impl Default for SettingsValues {
    fn default() -> Self {
        Self {
            locale: LocaleSettings::default(),
            appearance: AppearanceSettings::default(),
            typography: TypographySettings::default(),
            keyboard: KeyboardSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocaleSettings {
    pub language_tag: Option<String>,
    pub text_direction: TextDirectionPreference,
}

impl Default for LocaleSettings {
    fn default() -> Self {
        Self {
            language_tag: None,
            text_direction: TextDirectionPreference::Auto,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextDirectionPreference {
    Auto,
    Ltr,
    Rtl,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppearanceSettings {
    pub theme: ThemePreference,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypographySettings {
    pub font: FontPreference,
    pub emoji: EmojiPreference,
}

impl Default for TypographySettings {
    fn default() -> Self {
        Self {
            font: FontPreference::System,
            emoji: EmojiPreference::System,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontPreference {
    System,
    Inter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmojiPreference {
    System,
    TwemojiColr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KeyboardSettings {
    pub composer_send_shortcut: ComposerSendShortcut,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self {
            composer_send_shortcut: ComposerSendShortcut::Enter,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposerSendShortcut {
    Enter,
    ModEnter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SettingsPersistenceState {
    Idle,
    Saving { request_id: u64 },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatch {
    pub locale: Option<LocaleSettings>,
    pub appearance: Option<AppearanceSettings>,
    pub typography: Option<TypographySettings>,
    pub keyboard: Option<KeyboardSettings>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthDiscoveryState {
    Unknown,
    Discovering {
        homeserver: String,
    },
    Ready {
        homeserver: String,
        flows: Vec<LoginFlow>,
    },
    Failed {
        homeserver: String,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoginFlow {
    pub kind: LoginFlowKind,
    pub delegated_oidc_compatibility: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginFlowKind {
    Password,
    Sso,
    Token,
    Unknown(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionState {
    SignedOut,
    Restoring,
    SwitchingAccount {
        info: SessionInfo,
    },
    Authenticating {
        homeserver: String,
    },
    NeedsRecovery {
        info: SessionInfo,
        methods: Vec<RecoveryMethod>,
    },
    Recovering {
        info: SessionInfo,
        methods: Vec<RecoveryMethod>,
    },
    Ready(SessionInfo),
    Locked(SessionInfo),
    LoggingOut,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryMethod {
    RecoveryKey,
    SecurityPhrase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum E2eeRecoveryState {
    Unknown,
    Enabled,
    Disabled,
    Incomplete,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct E2eeTrustState {
    pub verification: VerificationFlowState,
    pub cross_signing: CrossSigningStatus,
    pub key_backup: KeyBackupStatus,
    pub identity_reset: IdentityResetState,
    pub devices: Vec<DeviceTrustSummary>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VerificationFlowState {
    #[default]
    Idle,
    Requested {
        request_id: u64,
        target: VerificationTarget,
    },
    Accepted {
        request_id: u64,
        target: VerificationTarget,
    },
    SasPresented {
        request_id: u64,
        target: VerificationTarget,
        emojis: Vec<SasEmoji>,
    },
    Confirming {
        request_id: u64,
        target: VerificationTarget,
        emojis: Vec<SasEmoji>,
    },
    Done {
        request_id: u64,
        target: VerificationTarget,
    },
    Failed {
        request_id: u64,
        target: VerificationTarget,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerificationTarget {
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SasEmoji {
    pub symbol: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CrossSigningStatus {
    #[default]
    Unknown,
    Missing,
    Bootstrapping {
        request_id: u64,
    },
    Trusted,
    NotTrusted,
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KeyBackupStatus {
    #[default]
    Unknown,
    Disabled,
    Enabling {
        request_id: u64,
    },
    Enabled {
        version: String,
    },
    Restoring {
        request_id: u64,
        version: Option<String>,
        restored_rooms: u64,
        total_rooms: Option<u64>,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IdentityResetState {
    #[default]
    Idle,
    Resetting {
        request_id: u64,
    },
    AwaitingAuth {
        request_id: u64,
        auth_type: IdentityResetAuthType,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdentityResetAuthType {
    Uiaa,
    #[serde(rename = "oauth")]
    OAuth,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceTrustSummary {
    pub user_id: String,
    pub device_id: String,
    pub trust_level: DeviceTrustLevel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceTrustLevel {
    Unknown,
    Unverified,
    Verified,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustOperationFailureKind {
    Cancelled,
    Mismatch,
    Network,
    Forbidden,
    Timeout,
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationCancelReason {
    User,
    Mismatch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileState {
    pub own: OwnProfile,
    pub users: BTreeMap<String, UserProfile>,
    pub update: ProfileUpdateState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnProfile {
    pub display_name: Option<String>,
    pub avatar: Option<AvatarImage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar: Option<AvatarImage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AvatarImage {
    pub mxc_uri: String,
    pub thumbnail: AvatarThumbnailState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AvatarThumbnailState {
    #[default]
    NotRequested,
    Loading {
        request_id: u64,
    },
    Ready {
        source_url: String,
        width: Option<u64>,
        height: Option<u64>,
        mime_type: Option<String>,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: AvatarThumbnailFailureKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AvatarThumbnailFailureKind {
    Network,
    Forbidden,
    Unsupported,
    Sdk,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProfileUpdateState {
    #[default]
    Idle,
    SettingDisplayName {
        request_id: u64,
        display_name: Option<String>,
    },
    SettingAvatar {
        request_id: u64,
        mime_type: String,
        byte_count: u64,
    },
}

impl ProfileUpdateState {
    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::SettingDisplayName { request_id, .. }
            | Self::SettingAvatar { request_id, .. } => Some(*request_id),
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProfileUpdateRequest {
    SetDisplayName { display_name: Option<String> },
    SetAvatar { mime_type: String, byte_count: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncState {
    Stopped,
    Starting,
    Running,
    Failed { reason: String },
    Reconnecting { reason: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationState {
    pub active_space_id: Option<String>,
    pub active_room_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub space_id: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub child_room_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub is_dm: bool,
    #[serde(default)]
    pub tags: RoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub parent_space_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomTags {
    pub favourite: Option<RoomTagInfo>,
    pub low_priority: Option<RoomTagInfo>,
}

impl RoomTags {
    pub fn set(&mut self, tag: RoomTagKind, info: RoomTagInfo) {
        match tag {
            RoomTagKind::Favourite => {
                self.favourite = Some(info);
                self.low_priority = None;
            }
            RoomTagKind::LowPriority => {
                self.low_priority = Some(info);
                self.favourite = None;
            }
        }
    }

    pub fn remove(&mut self, tag: RoomTagKind) {
        match tag {
            RoomTagKind::Favourite => self.favourite = None,
            RoomTagKind::LowPriority => self.low_priority = None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomTagInfo {
    pub order: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomTagKind {
    Favourite,
    LowPriority,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvitePreview {
    pub room_id: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub topic: Option<String>,
    pub inviter_display_name: Option<String>,
    pub is_dm: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomAttentionKind {
    Mention,
    Dm,
    Message,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAttentionSummary {
    pub room_display_name: String,
    pub kind: RoomAttentionKind,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub unread_count: u64,
}

pub fn room_attention_kind(
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
) -> Option<RoomAttentionKind> {
    if highlight_count > 0 {
        return Some(RoomAttentionKind::Mention);
    }

    if notification_count == 0 && unread_count == 0 {
        return None;
    }

    if is_dm {
        Some(RoomAttentionKind::Dm)
    } else {
        Some(RoomAttentionKind::Message)
    }
}

pub fn room_attention_summary(
    room_display_name: String,
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
) -> Option<RoomAttentionSummary> {
    let kind = room_attention_kind(is_dm, notification_count, highlight_count, unread_count)?;

    Some(RoomAttentionSummary {
        room_display_name,
        kind,
        notification_count,
        highlight_count,
        unread_count,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelinePaneState {
    pub room_id: Option<String>,
    pub is_subscribed: bool,
    pub is_paginating_backwards: bool,
    pub composer: ComposerState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerState {
    pub pending_transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_send_kind: Option<PendingComposerSendKind>,
    pub draft: String,
    pub mode: ComposerMode,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ThreadPaneState {
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
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FocusedContextState {
    Closed,
    Opening {
        room_id: String,
        event_id: String,
    },
    Open {
        room_id: String,
        event_id: String,
        is_subscribed: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchState {
    Closed,
    Editing {
        query: String,
        scope: SearchScope,
    },
    Searching {
        request_id: u64,
        query: String,
        scope: SearchScope,
    },
    Results {
        request_id: u64,
        query: String,
        scope: SearchScope,
        results: Vec<SearchResult>,
    },
    Failed {
        request_id: u64,
        query: String,
        scope: SearchScope,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchScope {
    CurrentRoom { room_id: String },
    CurrentSpace { space_id: String },
    Dms,
    AllRooms,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub timestamp_ms: u64,
    pub score_millis: u32,
    pub snippet: String,
    pub match_field: SearchMatchField,
    pub highlights: Vec<TextRange>,
    pub match_kind: SearchMatchKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextRange {
    /// Half-open range in UTF-16 code units relative to `SearchResult::snippet`.
    pub start_utf16: u32,
    pub end_utf16: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchMatchKind {
    Exact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchMatchField {
    MessageBody,
    AttachmentFileName,
}

/// In-flight status of a basic room/space operation, modeled as a guarded state
/// machine (see `docs/architecture/state-machine.md`): only `Idle` accepts a new
/// request, and a pending operation can only be settled by a completion whose
/// `request_id` matches the one carried by the in-flight state. This mirrors the
/// composer's pending-transaction rule and search's `request_id` correlation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BasicOperationState {
    #[default]
    Idle,
    CreatingRoom {
        request_id: u64,
        name: String,
    },
    CreatingSpace {
        request_id: u64,
        name: String,
    },
    LinkingSpaceChild {
        request_id: u64,
        space_id: String,
        child_room_id: String,
    },
}

impl BasicOperationState {
    /// Correlation id of the in-flight operation, or `None` when `Idle`.
    pub fn request_id(&self) -> Option<u64> {
        match self {
            BasicOperationState::Idle => None,
            BasicOperationState::CreatingRoom { request_id, .. }
            | BasicOperationState::CreatingSpace { request_id, .. }
            | BasicOperationState::LinkingSpaceChild { request_id, .. } => Some(*request_id),
        }
    }

    /// Whether no basic operation is currently in flight.
    pub fn is_idle(&self) -> bool {
        matches!(self, BasicOperationState::Idle)
    }
}

/// A requested basic operation: user intent, kept distinct from the resulting
/// state. The reducer pairs this with a correlation `request_id` to derive the
/// in-flight `BasicOperationState`; a request never names the target state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BasicOperationRequest {
    CreateRoom {
        name: String,
    },
    CreateSpace {
        name: String,
    },
    LinkSpaceChild {
        space_id: String,
        child_room_id: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveSignalsState {
    pub rooms: BTreeMap<String, RoomLiveSignals>,
    pub presence: BTreeMap<String, PresenceKind>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomLiveSignals {
    pub receipts_by_event: BTreeMap<String, Vec<LiveReadReceipt>>,
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveReadReceipt {
    pub user_id: String,
    pub timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveEventReceipts {
    pub event_id: String,
    pub receipts: Vec<LiveReadReceipt>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveRoomSignalUpdate {
    pub receipts_by_event: Vec<LiveEventReceipts>,
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
}

impl LiveRoomSignalUpdate {
    pub fn into_room_signals(self) -> RoomLiveSignals {
        let receipts_by_event = self
            .receipts_by_event
            .into_iter()
            .map(|entry| {
                let receipts = normalize_receipts(entry.receipts);
                (entry.event_id, receipts)
            })
            .collect();

        RoomLiveSignals {
            receipts_by_event,
            fully_read_event_id: self.fully_read_event_id,
            typing_user_ids: sorted_unique(self.typing_user_ids),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresenceKind {
    Online,
    Away,
    Offline,
}

fn normalize_receipts(receipts: Vec<LiveReadReceipt>) -> Vec<LiveReadReceipt> {
    let mut by_user = BTreeMap::new();
    for receipt in receipts {
        by_user.insert(receipt.user_id.clone(), receipt);
    }
    by_user.into_values().collect()
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}
