use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::composer_shortcuts::FormattedMessageDraft;
use crate::locale_profile::DisplayPlatform;

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
    pub room_interactions: BTreeMap<String, RoomInteractionState>,
    #[serde(skip)]
    pub composer_drafts: ComposerDraftStore,
    #[serde(skip)]
    pub scheduled_sends: ScheduledSendStore,
    #[serde(skip)]
    pub upload_staging: UploadStagingStore,
    #[serde(skip)]
    pub media_gallery: MediaGalleryStore,
    pub directory: DirectoryState,
    pub room_management: RoomManagementState,
    pub activity: ActivityState,
    pub timeline: TimelinePaneState,
    pub thread: ThreadPaneState,
    pub thread_attention: ThreadAttentionState,
    pub focused_context: FocusedContextState,
    pub search: SearchState,
    pub basic_operation: BasicOperationState,
    pub live_signals: LiveSignalsState,
    pub e2ee_trust: E2eeTrustState,
    pub local_encryption: LocalEncryptionState,
    pub native_attention: NativeAttentionState,
    pub cjk_text_policy: CjkTextPolicyState,
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
            room_interactions: BTreeMap::new(),
            composer_drafts: ComposerDraftStore::default(),
            scheduled_sends: ScheduledSendStore::default(),
            upload_staging: UploadStagingStore::default(),
            media_gallery: MediaGalleryStore::default(),
            directory: DirectoryState::default(),
            room_management: RoomManagementState::default(),
            activity: ActivityState::Closed,
            timeline: TimelinePaneState::default(),
            thread: ThreadPaneState::Closed,
            thread_attention: ThreadAttentionState::Closed,
            focused_context: FocusedContextState::Closed,
            search: SearchState::Closed,
            basic_operation: BasicOperationState::Idle,
            live_signals: LiveSignalsState::default(),
            e2ee_trust: E2eeTrustState::default(),
            local_encryption: LocalEncryptionState::Unknown,
            native_attention: NativeAttentionState::default(),
            cjk_text_policy: CjkTextPolicyState::default(),
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
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub display: DisplaySettings,
    #[serde(default)]
    pub media: MediaSettings,
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
        if let Some(notifications) = patch.notifications {
            self.notifications = notifications;
        }
        if let Some(display) = patch.display {
            self.display = display;
        }
        if let Some(media) = patch.media {
            self.media = media;
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
            notifications: NotificationSettings::default(),
            display: DisplaySettings::default(),
            media: MediaSettings::default(),
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
pub struct NotificationSettings {
    pub desktop_notifications: bool,
    pub sound: bool,
    pub badges: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            desktop_notifications: true,
            sound: true,
            badges: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    #[serde(default = "default_code_block_wrap")]
    pub code_block_wrap: bool,
    #[serde(default)]
    pub hide_redacted: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            code_block_wrap: true,
            hide_redacted: false,
        }
    }
}

fn default_code_block_wrap() -> bool {
    true
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaSettings {
    #[serde(default)]
    pub image_upload_compression: ImageUploadCompressionMode,
    #[serde(default)]
    pub image_upload_compression_policy: ImageUploadCompressionPolicy,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            image_upload_compression: ImageUploadCompressionMode::Never,
            image_upload_compression_policy: ImageUploadCompressionPolicy::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageUploadCompressionMode {
    Always,
    Ask,
    #[default]
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadCompressionPolicy {
    pub threshold_bytes: u64,
    pub threshold_long_edge: u64,
    pub target_long_edge: u64,
    pub quality_percent: u8,
}

impl Default for ImageUploadCompressionPolicy {
    fn default() -> Self {
        Self {
            threshold_bytes: 1_048_576,
            threshold_long_edge: 2560,
            target_long_edge: 2048,
            quality_percent: 82,
        }
    }
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
    pub notifications: Option<NotificationSettings>,
    pub display: Option<DisplaySettings>,
    pub media: Option<MediaSettings>,
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

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileState {
    pub own: OwnProfile,
    pub users: BTreeMap<String, UserProfile>,
    #[serde(default)]
    pub local_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub local_alias_update: LocalUserAliasUpdateState,
    pub update: ProfileUpdateState,
}

impl fmt::Debug for ProfileState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileState")
            .field("has_own_display_name", &self.own.display_name.is_some())
            .field("has_own_avatar", &self.own.avatar.is_some())
            .field("user_count", &self.users.len())
            .field("local_alias_count", &self.local_aliases.len())
            .field("local_alias_update", &self.local_alias_update)
            .field("update", &self.update)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnProfile {
    pub display_name: Option<String>,
    pub avatar: Option<AvatarImage>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    #[serde(default)]
    pub mention_search_terms: Vec<String>,
    pub avatar: Option<AvatarImage>,
}

impl fmt::Debug for UserProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserProfile")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field("mention_search_terms", &self.mention_search_terms.len())
            .field("has_avatar", &self.avatar.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalUserAliasUpdateState {
    #[default]
    Idle,
    Saving {
        request_id: u64,
    },
}

impl LocalUserAliasUpdateState {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Saving { request_id } => Some(*request_id),
        }
    }
}

pub fn normalize_local_user_alias(alias: Option<String>) -> Option<String> {
    alias.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

pub fn resolve_user_display_name(
    profiles: &ProfileState,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    let upstream_display_name = upstream_display_name
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty());
    let display_name = upstream_display_name.or_else(|| {
        profiles
            .users
            .get(user_id)
            .and_then(|profile| profile.display_name.as_deref())
    });
    resolve_user_display_name_from_parts(
        &profiles.local_aliases,
        profiles.own.display_name.as_deref(),
        user_id,
        display_name,
        own_user_id,
    )
}

pub fn original_user_display_name(
    profiles: &ProfileState,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    let upstream_display_name = upstream_display_name
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty());
    let display_name = upstream_display_name.or_else(|| {
        profiles
            .users
            .get(user_id)
            .and_then(|profile| profile.display_name.as_deref())
    });
    original_user_display_name_from_parts(
        profiles.own.display_name.as_deref(),
        user_id,
        display_name,
        own_user_id,
    )
}

pub fn refresh_profile_user_display_projection(
    profiles: &mut ProfileState,
    own_user_id: Option<&str>,
) {
    let local_aliases = &profiles.local_aliases;
    let own_display_name = profiles.own.display_name.as_deref();
    for (user_id, profile) in &mut profiles.users {
        let original_display_label = original_user_display_name_from_parts(
            own_display_name,
            user_id,
            profile.display_name.as_deref(),
            own_user_id,
        );
        let display_label = resolve_user_display_name_from_parts(
            local_aliases,
            own_display_name,
            user_id,
            profile.display_name.as_deref(),
            own_user_id,
        );
        profile.mention_search_terms = user_mention_search_terms(
            display_label.clone(),
            original_display_label.clone(),
            user_id,
        );
        profile.original_display_label = original_display_label;
        profile.display_label = display_label;
    }
}

pub fn refresh_room_settings_member_display_projection(
    settings: &mut RoomSettingsSnapshot,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> bool {
    let mut changed = false;
    for member in &mut settings.members {
        let display_label = resolve_user_display_name(
            profiles,
            &member.user_id,
            member.display_name.as_deref(),
            own_user_id,
        );
        let original_display_label = original_user_display_name(
            profiles,
            &member.user_id,
            member.display_name.as_deref(),
            own_user_id,
        );
        if member.display_label != display_label
            || member.original_display_label != original_display_label
        {
            member.display_label = display_label;
            member.original_display_label = original_display_label;
            changed = true;
        }
    }
    changed
}

pub fn refresh_room_summary_display_projection(
    rooms: &mut [RoomSummary],
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> bool {
    let mut changed = false;
    for room in rooms {
        let (display_label, original_display_label) =
            projected_room_summary_display_labels(room, profiles, own_user_id);
        if room.display_label != display_label
            || room.original_display_label != original_display_label
        {
            room.display_label = display_label;
            room.original_display_label = original_display_label;
            changed = true;
        }
    }
    changed
}

fn projected_room_summary_display_labels(
    room: &RoomSummary,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> (String, String) {
    if room.is_dm
        && room.dm_user_ids.len() == 1
        && let Some(user_id) = room.dm_user_ids.first()
    {
        return (
            resolve_user_display_name(profiles, user_id, Some(&room.display_name), own_user_id),
            original_user_display_name(profiles, user_id, Some(&room.display_name), own_user_id),
        );
    }

    let display_label = room
        .display_name
        .trim()
        .is_empty()
        .then(|| room.room_id.clone())
        .unwrap_or_else(|| room.display_name.trim().to_owned());
    (display_label.clone(), display_label)
}

fn resolve_user_display_name_from_parts(
    local_aliases: &BTreeMap<String, String>,
    own_display_name: Option<&str>,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    local_aliases
        .get(user_id)
        .filter(|alias| !alias.trim().is_empty())
        .cloned()
        .or_else(|| {
            upstream_display_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            own_user_id
                .filter(|own| *own == user_id)
                .and_then(|_| own_display_name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| user_id.to_owned())
}

fn original_user_display_name_from_parts(
    own_display_name: Option<&str>,
    user_id: &str,
    upstream_display_name: Option<&str>,
    own_user_id: Option<&str>,
) -> String {
    upstream_display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            own_user_id
                .filter(|own| *own == user_id)
                .and_then(|_| own_display_name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| user_id.to_owned())
}

fn user_mention_search_terms(
    display_label: String,
    original_display_label: String,
    user_id: &str,
) -> Vec<String> {
    let mut terms = Vec::new();
    push_unique_search_term(&mut terms, display_label);
    push_unique_search_term(&mut terms, original_display_label);
    push_unique_search_term(&mut terms, user_id.to_owned());
    terms
}

fn push_unique_search_term(terms: &mut Vec<String>, term: String) {
    if !terms.iter().any(|existing| existing == &term) {
        terms.push(term);
    }
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub display_name: String,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub is_dm: bool,
    #[serde(default)]
    pub dm_user_ids: Vec<String>,
    #[serde(default)]
    pub tags: RoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub parent_space_ids: Vec<String>,
}

impl fmt::Debug for RoomSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomSummary")
            .field("room_id", &"RoomId(..)")
            .field("display_name", &"RoomName(..)")
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field("avatar", &self.avatar.as_ref().map(|_| "AvatarImage(..)"))
            .field("is_dm", &self.is_dm)
            .field("dm_user_ids", &self.dm_user_ids.len())
            .field("tags", &self.tags)
            .field("unread_count", &self.unread_count)
            .field("notification_count", &self.notification_count)
            .field("highlight_count", &self.highlight_count)
            .field("parent_space_ids", &self.parent_space_ids.len())
            .finish()
    }
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
        room_display_name: private_safe_room_display_name(room_display_name),
        kind,
        notification_count,
        highlight_count,
        unread_count,
    })
}

fn private_safe_room_display_name(room_display_name: String) -> String {
    if room_display_name.trim().is_empty() {
        "Room".to_owned()
    } else {
        room_display_name
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomInteractionState {
    pub pinned_events: Vec<PinnedEvent>,
    pub pin_operation: PinOperationState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PinnedEvent {
    pub event_id: String,
    pub sender: Option<String>,
    pub body_preview: Option<String>,
    pub redacted: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplyQuote {
    pub event_id: String,
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    pub body_preview: Option<String>,
    pub state: ReplyQuoteState,
}

impl fmt::Debug for ReplyQuote {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplyQuote")
            .field("event_id", &"EventId(..)")
            .field("sender", &self.sender.as_ref().map(|_| "UserId(..)"))
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "body_preview",
                &self.body_preview.as_ref().map(|_| "BodyPreview(..)"),
            )
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyQuoteState {
    Ready,
    Redacted,
    Missing,
    Unsupported,
}

impl ReplyQuoteState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Redacted => "redacted",
            Self::Missing => "missing",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinOp {
    Pin,
    Unpin,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PinOperationState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        room_id: String,
        event_id: String,
        op: PinOp,
    },
    Failed {
        room_id: String,
        event_id: String,
        op: PinOp,
        recoverable: bool,
    },
}

impl PinOperationState {
    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Pending { request_id, .. } => Some(*request_id),
            Self::Failed { .. } => None,
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn accepts_new_request(&self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::Failed {
                    recoverable: true,
                    ..
                }
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationFailureKind {
    Forbidden,
    NotFound,
    Network,
    Timeout,
    Invalid,
    Sdk,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityState {
    #[default]
    Closed,
    Opening {
        request_id: u64,
        tab: ActivityTab,
    },
    Open {
        active_tab: ActivityTab,
        recent: ActivityStream,
        unread: ActivityStream,
        mark_read: ActivityMarkReadState,
    },
}

impl ActivityState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Opening { .. } => "opening",
            Self::Open { .. } => "open",
        }
    }
}

impl fmt::Debug for ActivityState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("ActivityState::Closed"),
            Self::Opening { request_id, tab } => formatter
                .debug_struct("ActivityOpening")
                .field("request_id", request_id)
                .field("tab", tab)
                .finish(),
            Self::Open {
                active_tab,
                recent,
                unread,
                mark_read,
            } => formatter
                .debug_struct("ActivityOpen")
                .field("active_tab", active_tab)
                .field("recent", recent)
                .field("unread", unread)
                .field("mark_read", mark_read)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityTab {
    #[default]
    Recent,
    Unread,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityStream {
    pub rows: Vec<ActivityRow>,
    pub next_batch: Option<String>,
}

impl fmt::Debug for ActivityStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivityStream")
            .field("rows", &format_args!("{} row(s)", self.rows.len()))
            .field(
                "next_batch",
                &self.next_batch.as_ref().map(|_| "PageToken(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityRow {
    pub room_id: String,
    pub event_id: String,
    pub room_label: String,
    pub sender_label: Option<String>,
    pub preview: Option<String>,
    pub timestamp_ms: u64,
    pub unread: bool,
    pub highlight: bool,
}

impl fmt::Debug for ActivityRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivityRow")
            .field("room_id", &"RoomId(..)")
            .field("event_id", &"EventId(..)")
            .field("room_label", &"RoomLabel(..)")
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field("preview", &self.preview.as_ref().map(|_| "Preview(..)"))
            .field("timestamp_ms", &self.timestamp_ms)
            .field("unread", &self.unread)
            .field("highlight", &self.highlight)
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityMarkReadState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        target: ActivityMarkReadTarget,
    },
    Failed {
        target: ActivityMarkReadTarget,
        failure_kind: OperationFailureKind,
    },
}

impl fmt::Debug for ActivityMarkReadState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("ActivityMarkReadState::Idle"),
            Self::Pending { request_id, target } => formatter
                .debug_struct("ActivityMarkReadPending")
                .field("request_id", request_id)
                .field("target", target)
                .finish(),
            Self::Failed {
                target,
                failure_kind,
            } => formatter
                .debug_struct("ActivityMarkReadFailed")
                .field("target", target)
                .field("kind", failure_kind)
                .finish(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityMarkReadTarget {
    Room {
        room_id: String,
        up_to_event_id: String,
    },
    All,
}

impl fmt::Debug for ActivityMarkReadTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Room { .. } => formatter
                .debug_struct("ActivityMarkReadTarget::Room")
                .field("room_id", &"RoomId(..)")
                .field("up_to_event_id", &"EventId(..)")
                .finish(),
            Self::All => formatter.write_str("ActivityMarkReadTarget::All"),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryState {
    pub query: DirectoryQueryState,
    pub join: DirectoryJoinState,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DirectoryQueryState {
    #[default]
    Closed,
    Querying {
        request_id: u64,
        query: DirectoryQuery,
    },
    Results {
        request_id: u64,
        query: DirectoryQuery,
        rooms: Vec<DirectoryRoomSummary>,
        next_batch: Option<String>,
    },
    Failed {
        request_id: u64,
        query: DirectoryQuery,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl fmt::Debug for DirectoryQueryState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("Closed"),
            Self::Querying { request_id, query } => formatter
                .debug_struct("Querying")
                .field("request_id", request_id)
                .field("query", query)
                .finish(),
            Self::Results {
                request_id,
                query,
                rooms,
                next_batch,
            } => formatter
                .debug_struct("Results")
                .field("request_id", request_id)
                .field("query", query)
                .field("rooms", rooms)
                .field("next_batch", &next_batch.as_ref().map(|_| "PageToken(..)"))
                .finish(),
            Self::Failed {
                request_id,
                query,
                kind,
            } => formatter
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("query", query)
                .field("kind", kind)
                .finish(),
        }
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DirectoryJoinState {
    #[default]
    Idle,
    Joining {
        request_id: u64,
        alias: String,
        via_server: Option<String>,
    },
    Failed {
        request_id: u64,
        alias: String,
        via_server: Option<String>,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl fmt::Debug for DirectoryJoinState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Joining {
                request_id,
                via_server,
                ..
            } => formatter
                .debug_struct("Joining")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &via_server.as_ref().map(|_| "ServerName(..)"))
                .finish(),
            Self::Failed {
                request_id,
                via_server,
                kind,
                ..
            } => formatter
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("alias", &"RoomAlias(..)")
                .field("via_server", &via_server.as_ref().map(|_| "ServerName(..)"))
                .field("kind", kind)
                .finish(),
        }
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryQuery {
    pub term: Option<String>,
    pub server_name: Option<String>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub since: Option<String>,
}

impl fmt::Debug for DirectoryQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryQuery")
            .field("term", &self.term.as_ref().map(|_| "QueryText(..)"))
            .field(
                "server_name",
                &self.server_name.as_ref().map(|_| "ServerName(..)"),
            )
            .field("limit", &self.limit)
            .field("since", &self.since.as_ref().map(|_| "PageToken(..)"))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryRoomSummary {
    pub room_id: String,
    pub canonical_alias: Option<String>,
    pub name: String,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub joined_members: u64,
    pub world_readable: bool,
    pub guest_can_join: bool,
}

impl fmt::Debug for DirectoryRoomSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryRoomSummary")
            .field("room_id", &"RoomId(..)")
            .field(
                "canonical_alias",
                &self.canonical_alias.as_ref().map(|_| "RoomAlias(..)"),
            )
            .field("name", &"RoomName(..)")
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("joined_members", &self.joined_members)
            .field("world_readable", &self.world_readable)
            .field("guest_can_join", &self.guest_can_join)
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomManagementState {
    pub selected_room_id: Option<String>,
    pub settings: Option<RoomSettingsSnapshot>,
    pub operation: RoomManagementOperationState,
}

impl fmt::Debug for RoomManagementState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomManagementState")
            .field(
                "selected_room_id",
                &self.selected_room_id.as_ref().map(|_| "RoomId(..)"),
            )
            .field(
                "settings",
                &self.settings.as_ref().map(|_| "RoomSettingsSnapshot(..)"),
            )
            .field("operation", &self.operation)
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomManagementOperationState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        room_id: String,
        operation: RoomManagementOperationKind,
    },
    Failed {
        request_id: u64,
        room_id: String,
        operation: RoomManagementOperationKind,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl fmt::Debug for RoomManagementOperationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Pending {
                request_id,
                operation,
                ..
            } => formatter
                .debug_struct("Pending")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("operation", operation)
                .finish(),
            Self::Failed {
                request_id,
                operation,
                kind,
                ..
            } => formatter
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("operation", operation)
                .field("kind", kind)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomManagementOperationKind {
    Settings,
    Moderation,
    Roles,
    Permissions,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSettingsSnapshot {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub join_rule: RoomJoinRule,
    pub history_visibility: RoomHistoryVisibility,
    pub permissions: RoomPermissionFacts,
    pub members: Vec<RoomMemberSummary>,
}

impl fmt::Debug for RoomSettingsSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomSettingsSnapshot")
            .field("room_id", &"RoomId(..)")
            .field("name", &self.name.as_ref().map(|_| "RoomName(..)"))
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("join_rule", &self.join_rule)
            .field("history_visibility", &self.history_visibility)
            .field("permissions", &self.permissions)
            .field("members", &self.members.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomMemberSummary {
    pub user_id: String,
    pub display_name: Option<String>,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: RoomMemberRole,
}

impl fmt::Debug for RoomMemberSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomMemberSummary")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("power_level", &self.power_level)
            .field("role", &self.role)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomMemberRole {
    Creator,
    Administrator,
    Moderator,
    User,
}

impl RoomMemberRole {
    pub fn from_power_level(power_level: Option<i64>) -> Self {
        match power_level {
            None => Self::Creator,
            Some(level) if level >= 100 => Self::Administrator,
            Some(level) if level >= 50 => Self::Moderator,
            Some(_) => Self::User,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomJoinRule {
    Public,
    Invite,
    Knock,
    Restricted,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomHistoryVisibility {
    WorldReadable,
    Shared,
    Invited,
    Joined,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomPermissionFacts {
    pub can_edit_settings: bool,
    pub can_edit_roles: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_unban: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomSettingChange {
    Name(Option<String>),
    Topic(Option<String>),
    AvatarUrl(Option<String>),
    JoinRule(RoomJoinRule),
    HistoryVisibility(RoomHistoryVisibility),
}

impl fmt::Debug for RoomSettingChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(value) => formatter
                .debug_tuple("Name")
                .field(&value.as_ref().map(|_| "RoomName(..)"))
                .finish(),
            Self::Topic(value) => formatter
                .debug_tuple("Topic")
                .field(&value.as_ref().map(|_| "RoomTopic(..)"))
                .finish(),
            Self::AvatarUrl(value) => formatter
                .debug_tuple("AvatarUrl")
                .field(&value.as_ref().map(|_| "MxcUri(..)"))
                .finish(),
            Self::JoinRule(rule) => formatter.debug_tuple("JoinRule").field(rule).finish(),
            Self::HistoryVisibility(visibility) => formatter
                .debug_tuple("HistoryVisibility")
                .field(visibility)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomModerationAction {
    Kick,
    Ban,
    Unban,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalEncryptionState {
    #[default]
    Unknown,
    Probing {
        request_id: u64,
    },
    Healthy,
    Unavailable,
    LockedOrInaccessible,
    MissingCredential,
    ResetRequired,
    Resetting {
        request_id: u64,
    },
}

impl LocalEncryptionState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Probing { .. } => "probing",
            Self::Healthy => "healthy",
            Self::Unavailable => "unavailable",
            Self::LockedOrInaccessible => "locked_or_inaccessible",
            Self::MissingCredential => "missing_credential",
            Self::ResetRequired => "reset_required",
            Self::Resetting { .. } => "resetting",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalEncryptionHealth {
    Unknown,
    Healthy,
    Unavailable,
    LockedOrInaccessible,
    MissingCredential,
    ResetRequired,
}

impl From<LocalEncryptionHealth> for LocalEncryptionState {
    fn from(health: LocalEncryptionHealth) -> Self {
        match health {
            LocalEncryptionHealth::Unknown => Self::Unknown,
            LocalEncryptionHealth::Healthy => Self::Healthy,
            LocalEncryptionHealth::Unavailable => Self::Unavailable,
            LocalEncryptionHealth::LockedOrInaccessible => Self::LockedOrInaccessible,
            LocalEncryptionHealth::MissingCredential => Self::MissingCredential,
            LocalEncryptionHealth::ResetRequired => Self::ResetRequired,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionState {
    pub summary: NativeAttentionSummary,
    pub dispatch: NativeAttentionDispatchState,
}

impl NativeAttentionState {
    pub fn kind(&self) -> &'static str {
        self.dispatch.kind()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionSummary {
    pub unread_count: u64,
    pub highlight_count: u64,
    pub badge_count: u64,
    pub candidate: Option<NativeAttentionCandidate>,
    pub capabilities: NativeAttentionCapabilities,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionCandidate {
    pub room_display_name: String,
    pub kind: RoomAttentionKind,
    pub unread_count: u64,
    pub highlight_count: u64,
}

impl fmt::Debug for NativeAttentionCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeAttentionCandidate")
            .field("room_display_name", &"RoomName(..)")
            .field("kind", &self.kind)
            .field("unread_count", &self.unread_count)
            .field("highlight_count", &self.highlight_count)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAttentionObservationKind {
    Live,
    InitialSync,
    Backfill,
    SelfEvent,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeAttentionProjectionInput<'a> {
    pub rooms: &'a [RoomSummary],
    pub active_room_id: Option<&'a str>,
    pub muted_room_ids: &'a [String],
    pub window_focused: bool,
    pub observation: NativeAttentionObservationKind,
    pub previous_candidate: Option<&'a NativeAttentionCandidate>,
    pub capabilities: NativeAttentionCapabilities,
}

struct NativeAttentionCandidateEntry<'a> {
    room_id: &'a str,
    candidate: NativeAttentionCandidate,
}

pub fn native_attention_state_from_rooms(
    input: NativeAttentionProjectionInput<'_>,
) -> NativeAttentionState {
    let mut unread_count = 0;
    let mut highlight_count = 0;
    let mut candidates = Vec::new();

    for room in input.rooms {
        if room.tags.low_priority.is_some()
            || input
                .muted_room_ids
                .iter()
                .any(|room_id| room_id == &room.room_id)
        {
            continue;
        }

        unread_count += room.unread_count;
        highlight_count += room.highlight_count;

        if let Some(summary) = room_attention_summary(
            room.display_label.clone(),
            room.is_dm,
            room.notification_count,
            room.highlight_count,
            room.unread_count,
        ) {
            candidates.push(NativeAttentionCandidateEntry {
                room_id: &room.room_id,
                candidate: NativeAttentionCandidate {
                    room_display_name: summary.room_display_name,
                    kind: summary.kind,
                    unread_count: summary.unread_count,
                    highlight_count: summary.highlight_count,
                },
            });
        }
    }

    candidates.sort_by(|left, right| {
        attention_kind_priority(right.candidate.kind)
            .cmp(&attention_kind_priority(left.candidate.kind))
            .then_with(|| {
                right
                    .candidate
                    .highlight_count
                    .cmp(&left.candidate.highlight_count)
            })
            .then_with(|| {
                right
                    .candidate
                    .unread_count
                    .cmp(&left.candidate.unread_count)
            })
            .then_with(|| {
                left.candidate
                    .room_display_name
                    .cmp(&right.candidate.room_display_name)
            })
    });

    let candidate_entry = candidates.first();
    let mut candidate = candidate_entry.map(|entry| entry.candidate.clone());
    let mut dispatch = NativeAttentionDispatchState::Idle;

    if let Some(entry) = candidate_entry {
        if let Some(reason) = native_attention_suppression_reason(input, entry) {
            candidate = None;
            dispatch = NativeAttentionDispatchState::Suppressed { reason };
        }
    }

    let badge_count = match input.capabilities.badge {
        NativeAttentionCapability::Unavailable => 0,
        NativeAttentionCapability::Available | NativeAttentionCapability::Unknown => unread_count,
    };

    NativeAttentionState {
        summary: NativeAttentionSummary {
            unread_count,
            highlight_count,
            badge_count,
            candidate,
            capabilities: input.capabilities,
        },
        dispatch,
    }
}

fn attention_kind_priority(kind: RoomAttentionKind) -> u8 {
    match kind {
        RoomAttentionKind::Mention => 3,
        RoomAttentionKind::Dm => 2,
        RoomAttentionKind::Message => 1,
    }
}

fn native_attention_suppression_reason(
    input: NativeAttentionProjectionInput<'_>,
    entry: &NativeAttentionCandidateEntry<'_>,
) -> Option<NativeAttentionSuppressionReason> {
    match input.observation {
        NativeAttentionObservationKind::InitialSync => {
            return Some(NativeAttentionSuppressionReason::InitialSync);
        }
        NativeAttentionObservationKind::Backfill => {
            return Some(NativeAttentionSuppressionReason::Backfill);
        }
        NativeAttentionObservationKind::SelfEvent => {
            return Some(NativeAttentionSuppressionReason::SelfMessage);
        }
        NativeAttentionObservationKind::Live => {}
    }

    if input.window_focused && input.active_room_id == Some(entry.room_id) {
        return Some(NativeAttentionSuppressionReason::WindowFocused);
    }

    if input.capabilities.notifications == NativeAttentionCapability::Unavailable {
        return Some(NativeAttentionSuppressionReason::CapabilityUnavailable);
    }

    if input.previous_candidate == Some(&entry.candidate) {
        return Some(NativeAttentionSuppressionReason::Duplicate);
    }

    None
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionCapabilities {
    pub notifications: NativeAttentionCapability,
    pub badge: NativeAttentionCapability,
    pub overlay_icon: NativeAttentionCapability,
    pub sound: NativeAttentionCapability,
    pub tray: NativeAttentionCapability,
    pub activation: NativeAttentionCapability,
}

pub fn native_attention_capabilities_for_platform(
    platform: DisplayPlatform,
) -> NativeAttentionCapabilities {
    let badge = match platform {
        DisplayPlatform::Macos | DisplayPlatform::Windows => NativeAttentionCapability::Available,
        DisplayPlatform::Linux => NativeAttentionCapability::Unknown,
    };

    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge,
        overlay_icon: match platform {
            DisplayPlatform::Windows => NativeAttentionCapability::Available,
            DisplayPlatform::Macos | DisplayPlatform::Linux => {
                NativeAttentionCapability::Unavailable
            }
        },
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Unknown,
        activation: NativeAttentionCapability::Unknown,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeAttentionCapability {
    Available,
    Unavailable,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NativeAttentionDispatchState {
    #[default]
    Idle,
    Dispatching {
        request_id: u64,
    },
    Delivered {
        request_id: u64,
    },
    Suppressed {
        reason: NativeAttentionSuppressionReason,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl NativeAttentionDispatchState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Dispatching { .. } => "dispatching",
            Self::Delivered { .. } => "delivered",
            Self::Suppressed { .. } => "suppressed",
            Self::Failed { .. } => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeAttentionSuppressionReason {
    InitialSync,
    Backfill,
    SelfMessage,
    WindowFocused,
    RoomMuted,
    LowPriority,
    Duplicate,
    CapabilityUnavailable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CjkTextPolicyState {
    pub japanese_catalog: JapaneseCatalogProfile,
    pub normalization: CjkNormalizationProfile,
    pub collation: CjkCollationProfile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JapaneseCatalogProfile {
    pub catalog_locale: String,
    pub complete: bool,
    pub missing_message_ids: Vec<String>,
}

impl Default for JapaneseCatalogProfile {
    fn default() -> Self {
        Self {
            catalog_locale: "en".to_owned(),
            complete: true,
            missing_message_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CjkNormalizationProfile {
    pub form: String,
    pub width_fold: bool,
    pub kana_fold: bool,
}

impl Default for CjkNormalizationProfile {
    fn default() -> Self {
        Self {
            form: "nfkc".to_owned(),
            width_fold: true,
            kana_fold: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CjkCollationProfile {
    pub locale: String,
    pub numeric: bool,
    pub case_first: Option<String>,
}

impl Default for CjkCollationProfile {
    fn default() -> Self {
        Self {
            locale: "ja".to_owned(),
            numeric: true,
            case_first: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelinePaneState {
    pub room_id: Option<String>,
    pub is_subscribed: bool,
    pub is_paginating_backwards: bool,
    pub composer: ComposerState,
    pub scheduled_send_capability: ScheduledSendCapability,
    pub scheduled_sends: Vec<ScheduledSendItem>,
    pub staged_uploads: Vec<StagedUploadItem>,
    pub media_gallery: Vec<TimelineMediaGalleryItem>,
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
    Original,
    Compressed { mode: ImageUploadCompressionMode },
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UploadStagingStore {
    pub items: BTreeMap<String, StagedUploadItem>,
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
    pub rooms: BTreeMap<String, Vec<TimelineMediaGalleryItem>>,
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
    pub items: BTreeMap<String, ScheduledSendItem>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rooms: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub threads: BTreeMap<String, BTreeMap<String, String>>,
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
        let mut threads = BTreeMap::new();
        for (room_id, room_threads) in &self.threads {
            if remaining_threads == 0 {
                break;
            }
            let mut bounded_room_threads = BTreeMap::new();
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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadAttentionState {
    #[default]
    Closed,
    Tracking {
        room_id: String,
        root_event_id: String,
        notification_count: u64,
        highlight_count: u64,
        live_event_marker_count: u64,
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
    pub receipts_by_event: BTreeMap<String, LiveEventReceiptSummary>,
    pub fully_read_event_id: Option<String>,
    pub typing_user_ids: Vec<String>,
}

pub const LIVE_READ_RECEIPT_READER_CAP: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveReadReceipt {
    pub user_id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub original_display_label: String,
    pub avatar: Option<AvatarImage>,
    pub timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LiveEventReceiptSummary {
    pub readers: Vec<LiveReadReceipt>,
    pub total_count: u64,
    pub overflow_count: u64,
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
        self.into_room_signals_with_profiles(&ProfileState::default(), None)
    }

    pub fn into_room_signals_with_profiles(
        self,
        profiles: &ProfileState,
        own_user_id: Option<&str>,
    ) -> RoomLiveSignals {
        let receipts_by_event = self
            .receipts_by_event
            .into_iter()
            .map(|entry| {
                let receipts = normalize_receipts(entry.receipts, profiles, own_user_id);
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

fn normalize_receipts(
    receipts: Vec<LiveReadReceipt>,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> LiveEventReceiptSummary {
    let mut by_user = BTreeMap::new();
    for receipt in receipts {
        let receipt = enrich_receipt(receipt, profiles, own_user_id);
        by_user
            .entry(receipt.user_id.clone())
            .and_modify(|existing: &mut LiveReadReceipt| {
                if receipt_is_newer(&receipt, existing) {
                    *existing = receipt.clone();
                }
            })
            .or_insert(receipt);
    }
    let mut readers = by_user.into_values().collect::<Vec<_>>();
    readers.sort_by(|left, right| {
        right
            .timestamp_ms
            .unwrap_or_default()
            .cmp(&left.timestamp_ms.unwrap_or_default())
            .then_with(|| left.user_id.cmp(&right.user_id))
    });

    let total_count = readers.len() as u64;
    let overflow_count = total_count.saturating_sub(LIVE_READ_RECEIPT_READER_CAP as u64);
    readers.truncate(LIVE_READ_RECEIPT_READER_CAP);

    LiveEventReceiptSummary {
        readers,
        total_count,
        overflow_count,
    }
}

fn receipt_is_newer(candidate: &LiveReadReceipt, existing: &LiveReadReceipt) -> bool {
    candidate.timestamp_ms.unwrap_or_default() >= existing.timestamp_ms.unwrap_or_default()
}

fn enrich_receipt(
    mut receipt: LiveReadReceipt,
    profiles: &ProfileState,
    own_user_id: Option<&str>,
) -> LiveReadReceipt {
    let own_profile = own_user_id
        .filter(|user_id| *user_id == receipt.user_id)
        .map(|_| &profiles.own);
    let user_profile = profiles.users.get(&receipt.user_id);

    let receipt_display_name = receipt.display_name.clone();
    let receipt_original_display_label = receipt.original_display_label.clone();
    let original_source = if receipt_original_display_label.trim().is_empty() {
        receipt_display_name.as_deref()
    } else {
        Some(receipt_original_display_label.as_str())
    };
    let display_label = resolve_user_display_name(
        profiles,
        &receipt.user_id,
        receipt_display_name.as_deref(),
        own_user_id,
    );
    let original_display_label =
        original_user_display_name(profiles, &receipt.user_id, original_source, own_user_id);
    receipt.display_name = Some(display_label);
    receipt.original_display_label = original_display_label;
    if receipt.avatar.is_none() {
        receipt.avatar = own_profile
            .and_then(|profile| profile.avatar.clone())
            .or_else(|| user_profile.and_then(|profile| profile.avatar.clone()));
    }
    receipt
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
