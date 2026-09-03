use serde::{Deserialize, Serialize};

use crate::composer_shortcuts::ComposerFormattingOptions;

use super::search_crawler::SearchCrawlerSettings;

pub(crate) fn default_true() -> bool {
    true
}

fn default_code_block_wrap() -> bool {
    true
}

fn default_hide_redacted() -> bool {
    true
}

fn default_url_previews_enabled() -> bool {
    true
}

fn default_encrypted_url_previews_enabled() -> bool {
    true
}

fn default_thread_list_order() -> ThreadListOrder {
    ThreadListOrder::LatestReply
}

fn default_timeline_thread_root_order() -> TimelineThreadRootOrder {
    // Product default since #366: threaded conversations surface at their
    // latest reply. A persisted "rootEvent" value keeps the user's choice.
    TimelineThreadRootOrder::LatestReply
}

fn default_room_list_sort() -> RoomListSort {
    RoomListSort::Activity
}

fn canonicalize_recent_emojis(emojis: Vec<String>) -> Vec<String> {
    let mut canonical = Vec::with_capacity(emojis.len().min(24));
    for emoji in emojis {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 16 || emoji.chars().any(char::is_control) {
            continue;
        }
        if !canonical.iter().any(|existing| existing == emoji) {
            canonical.push(emoji.to_owned());
            if canonical.len() == 24 {
                break;
            }
        }
    }
    canonical
}

fn deserialize_recent_emojis<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(canonicalize_recent_emojis)
}

pub type RoomUrlPreviews = std::collections::BTreeMap<String, bool>;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomPreferencesState {
    #[serde(default)]
    pub rooms: std::collections::BTreeMap<String, RoomPreference>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomPreference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_previews_enabled_override: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_mode: Option<RoomNotificationMode>,
}

impl RoomPreference {
    pub fn is_empty(&self) -> bool {
        self.url_previews_enabled_override.is_none() && self.notification_mode.is_none()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkPreviewSettingsState {
    #[serde(default)]
    pub room_overrides: RoomUrlPreviews,
}

impl Default for LinkPreviewSettingsState {
    fn default() -> Self {
        Self {
            room_overrides: RoomUrlPreviews::new(),
        }
    }
}

impl std::fmt::Debug for LinkPreviewSettingsState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LinkPreviewSettingsState")
            .field("room_override_count", &self.room_overrides.len())
            .finish()
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
    pub composer: ComposerSettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub display: DisplaySettings,
    #[serde(default)]
    pub media: MediaSettings,
    #[serde(default)]
    pub timeline: TimelineSettings,
    #[serde(default = "default_thread_list_order")]
    pub thread_list_order: ThreadListOrder,
    #[serde(default = "default_room_list_sort")]
    pub room_list_sort: RoomListSort,
    #[serde(default)]
    pub search_crawler: SearchCrawlerSettings,
    #[serde(default)]
    pub sidebar: SidebarSettings,
    #[serde(default)]
    pub window: WindowSettings,
    #[serde(default)]
    pub legacy_frontend_preferences_imported: bool,
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
        if let Some(mut composer) = patch.composer {
            composer.recent_emojis = canonicalize_recent_emojis(composer.recent_emojis);
            self.composer = composer;
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
        if let Some(timeline) = patch.timeline {
            self.timeline = timeline;
        }
        if let Some(thread_list_order) = patch.thread_list_order {
            self.thread_list_order = thread_list_order;
        }
        if let Some(room_list_sort) = patch.room_list_sort {
            self.room_list_sort = room_list_sort;
        }
        if let Some(search_crawler) = patch.search_crawler {
            self.search_crawler = search_crawler;
        }
        if let Some(sidebar) = patch.sidebar {
            self.sidebar = sidebar;
        }
        if let Some(window) = patch.window {
            self.window = window;
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
            composer: ComposerSettings::default(),
            notifications: NotificationSettings::default(),
            display: DisplaySettings::default(),
            media: MediaSettings::default(),
            timeline: TimelineSettings::default(),
            thread_list_order: ThreadListOrder::default(),
            room_list_sort: RoomListSort::default(),
            search_crawler: SearchCrawlerSettings::default(),
            sidebar: SidebarSettings::default(),
            window: WindowSettings::default(),
            legacy_frontend_preferences_imported: false,
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
    #[serde(default)]
    pub density: DisplayDensity,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            density: DisplayDensity::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayDensity {
    Compact,
    #[default]
    Comfortable,
    Default,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarSettings {
    #[serde(default)]
    pub category: SidebarCategory,
    #[serde(default)]
    pub collapsed: SidebarCollapsedSections,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            category: SidebarCategory::default(),
            collapsed: SidebarCollapsedSections::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SidebarCategory {
    #[default]
    Rooms,
    People,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarCollapsedSections {
    #[serde(default)]
    pub favourites: bool,
    #[serde(default)]
    pub low_priority: bool,
    #[serde(default)]
    pub not_joined: bool,
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerSettings {
    #[serde(default = "default_true")]
    pub math_mode: bool,
    #[serde(default, deserialize_with = "deserialize_recent_emojis")]
    pub recent_emojis: Vec<String>,
}

impl ComposerSettings {
    pub fn formatting_options(&self) -> ComposerFormattingOptions {
        ComposerFormattingOptions {
            math_mode: self.math_mode,
        }
    }
}

impl std::fmt::Debug for ComposerSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComposerSettings")
            .field("math_mode", &self.math_mode)
            .field("recent_emoji_count", &self.recent_emojis.len())
            .finish()
    }
}

impl Default for ComposerSettings {
    fn default() -> Self {
        Self {
            math_mode: true,
            recent_emojis: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotificationSettings {
    pub desktop_notifications: bool,
    pub sound: bool,
    pub badges: bool,
    #[serde(default = "default_true")]
    pub send_read_receipts: bool,
    #[serde(default = "default_true")]
    pub send_typing_notifications: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            desktop_notifications: true,
            sound: true,
            badges: true,
            send_read_receipts: true,
            send_typing_notifications: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomNotificationMode {
    All,
    Mentions,
    Mute,
}

impl Default for RoomNotificationMode {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct RoomNotificationSettings {
    pub mode: RoomNotificationMode,
    pub operation: RoomNotificationModeOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomNotificationModeOperation {
    Idle,
    Pending {
        request_id: u64,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        failure_kind: super::errors::OperationFailureKind,
    },
}

impl Default for RoomNotificationModeOperation {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    #[serde(default = "default_code_block_wrap")]
    pub code_block_wrap: bool,
    #[serde(default = "default_hide_redacted")]
    pub hide_redacted: bool,
    #[serde(default = "default_url_previews_enabled")]
    pub url_previews_enabled: bool,
    #[serde(default = "default_encrypted_url_previews_enabled")]
    pub encrypted_url_previews_enabled: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            code_block_wrap: true,
            hide_redacted: true,
            url_previews_enabled: true,
            encrypted_url_previews_enabled: true,
        }
    }
}

/// Rust-owned desktop window lifecycle preferences.
///
/// `close_to_tray` gates close-to-hide on Linux and Windows (overview.md,
/// "Desktop Window Lifecycle And Tray"). macOS hides on close unconditionally
/// per platform convention and ignores this value. The Tauri adapter also
/// requires an actually-created tray icon before honouring it, so turning this
/// on can never make the only window unreachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WindowSettings {
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
}

fn default_close_to_tray() -> bool {
    true
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            close_to_tray: default_close_to_tray(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Media settings.
///
/// #305 retired the automatic-compression mode: the staging dialog always asks
/// and starts at the untouched output, so there is no preference left to store.
/// The policy remains because the encoder still reads its quality value and the
/// direct upload path still reads its thresholds.
pub struct MediaSettings {
    #[serde(default)]
    pub image_upload_compression_policy: ImageUploadCompressionPolicy,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            image_upload_compression_policy: ImageUploadCompressionPolicy::default(),
        }
    }
}

/// Per-item compression choice payload.
///
/// This is no longer a stored preference: #305 retired the settings field. It
/// survives only as the `StagedUploadCompressionChoice::Compressed` payload, and
/// that path is unreachable from the product UI now that the staging dialog
/// offers explicit resize/format pairs, so it is a candidate for removal after a
/// dedicated audit of the upload-staging command surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageUploadCompressionMode {
    Always,
    #[default]
    Ask,
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
pub struct TimelineSettings {
    #[serde(default = "default_true")]
    pub auto_load_older_messages: bool,
    #[serde(default = "default_timeline_thread_root_order")]
    pub thread_root_order: TimelineThreadRootOrder,
}

impl Default for TimelineSettings {
    fn default() -> Self {
        Self {
            auto_load_older_messages: true,
            thread_root_order: default_timeline_thread_root_order(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimelineThreadRootOrder {
    RootEvent,
    LatestReply,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadListOrder {
    LatestReply,
    RootChronology,
}

impl Default for ThreadListOrder {
    fn default() -> Self {
        Self::LatestReply
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomListSort {
    Activity,
    RecentFirst,
    NormalLocale,
}

impl Default for RoomListSort {
    fn default() -> Self {
        Self::Activity
    }
}

// SearchCrawlerSettings and SearchCrawlerSpeed live in state/search_crawler.rs
// and are re-exported from mod.rs.

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
    pub composer: Option<ComposerSettings>,
    pub notifications: Option<NotificationSettings>,
    pub display: Option<DisplaySettings>,
    pub media: Option<MediaSettings>,
    pub timeline: Option<TimelineSettings>,
    pub thread_list_order: Option<ThreadListOrder>,
    pub room_list_sort: Option<RoomListSort>,
    pub search_crawler: Option<SearchCrawlerSettings>,
    #[serde(default)]
    pub sidebar: Option<SidebarSettings>,
    #[serde(default)]
    pub window: Option<WindowSettings>,
}
