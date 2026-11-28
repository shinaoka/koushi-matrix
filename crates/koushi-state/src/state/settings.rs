use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::composer_shortcuts::ComposerFormattingOptions;

use super::notification_delivery::{
    NotificationDeliverySettings, deserialize_notification_delivery,
};
use super::search_crawler::SearchCrawlerSettings;

pub(crate) fn default_true() -> bool {
    true
}

fn default_code_block_wrap() -> bool {
    true
}

/// #2166: a deleted message leaves a placeholder until the reader opts in to
/// hiding it (#35 asked for an opt-in preference).
fn default_hide_redacted() -> bool {
    false
}

/// #2041: a room's state changes are shown until the reader turns them off.
fn default_show_room_and_member_changes() -> bool {
    true
}

fn default_url_previews_enabled() -> bool {
    true
}

/// Encrypted-room link previews reveal URLs to the homeserver and destination
/// site, so they stay off until the user explicitly opts in.
fn default_encrypted_url_previews_enabled() -> bool {
    false
}

fn default_thread_list_order() -> ThreadListOrder {
    ThreadListOrder::LatestReply
}

fn default_timeline_thread_root_order() -> TimelineThreadRootOrder {
    // Product default since #366: threaded conversations surface at their
    // latest reply. A persisted "rootEvent" value keeps the user's choice.
    TimelineThreadRootOrder::LatestReply
}

/// #542: the sidebar's product default is true recent-conversation order.
/// Attention stays visible through badges, highlighting, and the Activity
/// surface, but no longer moves a room; `Activity` remains deserializable for
/// settings an older build persisted.
fn default_room_list_sort() -> RoomListSort {
    RoomListSort::RecentFirst
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct LinkPreviewSettingsState {
    #[serde(default)]
    pub room_overrides: RoomUrlPreviews,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct SettingsValues {
    pub locale: LocaleSettings,
    pub appearance: AppearanceSettings,
    pub typography: TypographySettings,
    pub keyboard: KeyboardSettings,
    #[serde(default)]
    pub composer: ComposerSettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
    /// Account-owned device-local notification delivery (#1427). Boxed like the
    /// settings patch, so the block does not dominate the effect enum's largest variant.
    #[serde(default, deserialize_with = "deserialize_notification_delivery")]
    pub delivery: Box<NotificationDeliverySettings>,
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
    pub updates: UpdatesSettings,
    #[serde(default)]
    pub mcp: McpSettings,
    #[serde(default)]
    pub agent_accounts: AgentAccounts,
    #[serde(default)]
    pub legacy_frontend_preferences_imported: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettingsValues {
    pub locale: LocaleSettings,
    pub appearance: AppearanceSettings,
    pub typography: TypographySettings,
    pub keyboard: KeyboardSettings,
    pub composer_math_mode: bool,
    pub display: AppDisplaySettings,
    pub media: MediaSettings,
    pub timeline: TimelineSettings,
    pub thread_list_order: ThreadListOrder,
    pub search_crawler_speed: super::search_crawler::SearchCrawlerSpeed,
    pub window: WindowSettings,
    pub updates: UpdatesSettings,
    pub mcp: McpSettings,
    pub agent_accounts: AgentAccounts,
    pub legacy_frontend_preferences_imported: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppDisplaySettings {
    pub code_block_wrap: bool,
    pub hide_redacted: bool,
    /// #2041: hides profile and membership notices in Core. Only joins, leaves,
    /// invites and knocks are hideable; every other row — encryption, a room
    /// upgrade, moderation, a member event that changes no membership — stays
    /// visible whatever this says.
    pub show_room_and_member_changes: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountSettingsValues {
    pub notifications: AccountNotificationSettings,
    #[serde(default, deserialize_with = "deserialize_notification_delivery")]
    pub delivery: Box<NotificationDeliverySettings>,
    pub display: AccountDisplaySettings,
    pub sidebar: SidebarSettings,
    pub room_list_sort: RoomListSort,
    pub recent_emojis: Vec<String>,
    pub search_crawler: AccountSearchCrawlerSettings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountNotificationSettings {
    pub sound: bool,
    pub badges: bool,
    pub desktop_notifications: bool,
    pub message_previews: bool,
    pub send_read_receipts: bool,
    pub send_typing_notifications: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountDisplaySettings {
    pub url_previews_enabled: bool,
    pub encrypted_url_previews_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountSearchCrawlerSettings {
    pub include_media_captions: bool,
    pub include_filenames: bool,
}

impl SettingsValues {
    pub fn app_settings(&self) -> AppSettingsValues {
        AppSettingsValues {
            locale: self.locale.clone(),
            appearance: self.appearance.clone(),
            typography: self.typography.clone(),
            keyboard: self.keyboard.clone(),
            composer_math_mode: self.composer.math_mode,
            display: AppDisplaySettings {
                code_block_wrap: self.display.code_block_wrap,
                hide_redacted: self.display.hide_redacted,
                show_room_and_member_changes: self.display.show_room_and_member_changes,
            },
            media: self.media.clone(),
            timeline: self.timeline.clone(),
            thread_list_order: self.thread_list_order,
            search_crawler_speed: self.search_crawler.speed,
            window: self.window,
            updates: self.updates,
            mcp: self.mcp.clone(),
            agent_accounts: self.agent_accounts.clone(),
            legacy_frontend_preferences_imported: self.legacy_frontend_preferences_imported,
        }
    }

    pub fn account_settings(&self) -> AccountSettingsValues {
        AccountSettingsValues {
            delivery: self.delivery.clone(),
            notifications: AccountNotificationSettings {
                sound: self.notifications.sound,
                badges: self.notifications.badges,
                desktop_notifications: self.notifications.desktop_notifications,
                message_previews: self.notifications.message_previews,
                send_read_receipts: self.notifications.send_read_receipts,
                send_typing_notifications: self.notifications.send_typing_notifications,
            },
            display: AccountDisplaySettings {
                url_previews_enabled: self.display.url_previews_enabled,
                encrypted_url_previews_enabled: self.display.encrypted_url_previews_enabled,
            },
            sidebar: self.sidebar.clone(),
            room_list_sort: self.room_list_sort,
            recent_emojis: self.composer.recent_emojis.clone(),
            search_crawler: AccountSearchCrawlerSettings {
                include_media_captions: self.search_crawler.include_media_captions,
                include_filenames: self.search_crawler.include_filenames,
            },
        }
    }

    pub fn apply_app_settings(&mut self, app: &AppSettingsValues) {
        self.locale = app.locale.clone();
        self.appearance = app.appearance.clone();
        self.typography = app.typography.clone();
        self.keyboard = app.keyboard.clone();
        self.composer.math_mode = app.composer_math_mode;
        self.display.code_block_wrap = app.display.code_block_wrap;
        self.display.hide_redacted = app.display.hide_redacted;
        self.media = app.media.clone();
        self.timeline = app.timeline.clone();
        self.thread_list_order = app.thread_list_order;
        self.search_crawler.speed = app.search_crawler_speed;
        self.window = app.window;
        self.updates = app.updates;
        self.mcp = app.mcp.clone();
        self.agent_accounts = app.agent_accounts.clone();
        self.legacy_frontend_preferences_imported = app.legacy_frontend_preferences_imported;
    }

    pub fn apply_account_settings(&mut self, account: &AccountSettingsValues) {
        self.notifications.sound = account.notifications.sound;
        self.notifications.badges = account.notifications.badges;
        self.delivery = account.delivery.clone();
        self.delivery.canonicalize();
        self.notifications.desktop_notifications = account.notifications.desktop_notifications;
        self.notifications.message_previews = account.notifications.message_previews;
        self.notifications.send_read_receipts = account.notifications.send_read_receipts;
        self.notifications.send_typing_notifications =
            account.notifications.send_typing_notifications;
        self.display.url_previews_enabled = account.display.url_previews_enabled;
        self.display.encrypted_url_previews_enabled =
            account.display.encrypted_url_previews_enabled;
        self.sidebar = account.sidebar.clone();
        self.room_list_sort = account.room_list_sort;
        self.composer.recent_emojis = canonicalize_recent_emojis(account.recent_emojis.clone());
        self.search_crawler.include_media_captions = account.search_crawler.include_media_captions;
        self.search_crawler.include_filenames = account.search_crawler.include_filenames;
    }

    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        let apply_app = patch.scope != Some(SettingsPatchScope::Account);
        let apply_account = patch.scope != Some(SettingsPatchScope::App);
        if apply_app {
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
            if let Some(media) = patch.media {
                self.media = media;
            }
            if let Some(timeline) = patch.timeline {
                self.timeline = timeline;
            }
            if let Some(thread_list_order) = patch.thread_list_order {
                self.thread_list_order = thread_list_order;
            }
            if let Some(window) = patch.window {
                self.window = window;
            }
            if let Some(updates) = patch.updates {
                self.updates = updates;
            }
            if let Some(mcp) = patch.mcp {
                // The bridge switch only: the allow-list has its own field, so
                // flipping the switch never wipes the owner's edits.
                self.mcp.enabled = mcp.enabled;
            }
            if let Some(allowed) = &patch.mcp_allowed_senders {
                self.mcp.allowed_senders = allowed.clone();
                self.mcp.allowed_senders.canonicalize();
            }
            if let Some(change) = &patch.agent_account {
                self.agent_accounts.apply(change);
            }
            if let Some(imported) = patch.legacy_frontend_preferences_imported {
                self.legacy_frontend_preferences_imported = imported;
            }
        }
        if let Some(composer) = patch.composer {
            if apply_app {
                self.composer.math_mode = composer.math_mode;
            }
            if apply_account {
                self.composer.recent_emojis = canonicalize_recent_emojis(composer.recent_emojis);
            }
        }
        if apply_account && let Some(notifications) = patch.notifications {
            self.notifications = notifications;
        }
        if let Some(display) = patch.display {
            if apply_app {
                self.display.code_block_wrap = display.code_block_wrap;
                self.display.hide_redacted = display.hide_redacted;
                self.display.show_room_and_member_changes = display.show_room_and_member_changes;
            }
            if apply_account {
                self.display.url_previews_enabled = display.url_previews_enabled;
                self.display.encrypted_url_previews_enabled =
                    display.encrypted_url_previews_enabled;
            }
        }
        if apply_account && let Some(delivery) = &patch.delivery {
            self.delivery
                .apply_patch(delivery, patch.delivery_bindings_from_core);
        }
        if apply_account {
            if let Some(room_list_sort) = patch.room_list_sort {
                self.room_list_sort = room_list_sort;
            }
            if let Some(search_crawler) = &patch.search_crawler {
                self.search_crawler.include_media_captions = search_crawler.include_media_captions;
                self.search_crawler.include_filenames = search_crawler.include_filenames;
            }
            if let Some(sidebar) = patch.sidebar {
                self.sidebar = sidebar;
            }
            if let Some(sidebar_section) = patch.sidebar_section {
                self.sidebar
                    .apply_section_patch(sidebar_section, self.room_list_sort);
            }
        }
        if apply_app && let Some(search_crawler) = &patch.search_crawler {
            self.search_crawler.speed = search_crawler.speed;
        }
    }
}

impl AppSettingsValues {
    pub fn apply_patch(&mut self, patch: &SettingsPatch) {
        if patch.scope == Some(SettingsPatchScope::Account) {
            return;
        }
        if let Some(value) = &patch.locale {
            self.locale = value.clone();
        }
        if let Some(value) = &patch.appearance {
            self.appearance = value.clone();
        }
        if let Some(value) = &patch.typography {
            self.typography = value.clone();
        }
        if let Some(value) = &patch.keyboard {
            self.keyboard = value.clone();
        }
        if let Some(value) = &patch.composer {
            self.composer_math_mode = value.math_mode;
        }
        if let Some(value) = &patch.display {
            self.display = AppDisplaySettings {
                code_block_wrap: value.code_block_wrap,
                hide_redacted: value.hide_redacted,
                show_room_and_member_changes: value.show_room_and_member_changes,
            };
        }
        if let Some(value) = &patch.media {
            self.media = value.clone();
        }
        if let Some(value) = &patch.timeline {
            self.timeline = value.clone();
        }
        if let Some(value) = patch.thread_list_order {
            self.thread_list_order = value;
        }
        if let Some(value) = &patch.search_crawler {
            self.search_crawler_speed = value.speed;
        }
        if let Some(value) = patch.window {
            self.window = value;
        }
        if let Some(value) = &patch.updates {
            self.updates = *value;
        }
        if let Some(value) = &patch.mcp {
            self.mcp.enabled = value.enabled;
        }
        if let Some(value) = &patch.mcp_allowed_senders {
            self.mcp.allowed_senders = value.clone();
            self.mcp.allowed_senders.canonicalize();
        }
        if let Some(change) = &patch.agent_account {
            self.agent_accounts.apply(change);
        }
        if let Some(value) = patch.legacy_frontend_preferences_imported {
            self.legacy_frontend_preferences_imported = value;
        }
    }
}

impl Default for AppSettingsValues {
    fn default() -> Self {
        SettingsValues::default().app_settings()
    }
}

impl Default for AccountSettingsValues {
    fn default() -> Self {
        SettingsValues::default().account_settings()
    }
}

impl AccountSettingsValues {
    /// In-memory fallback for a settings read failure; never persist these values.
    pub fn privacy_safe_fallback() -> Self {
        let mut settings = Self::default();
        settings.notifications.sound = false;
        settings.notifications.badges = false;
        *settings.delivery = Default::default();
        settings.notifications.desktop_notifications = false;
        settings.notifications.message_previews = false;
        settings.notifications.send_read_receipts = false;
        settings.notifications.send_typing_notifications = false;
        settings.display.url_previews_enabled = false;
        settings.display.encrypted_url_previews_enabled = false;
        settings.search_crawler.include_media_captions = false;
        settings.search_crawler.include_filenames = false;
        settings
    }
}

impl Default for AppDisplaySettings {
    fn default() -> Self {
        let settings = DisplaySettings::default();
        Self {
            code_block_wrap: settings.code_block_wrap,
            hide_redacted: settings.hide_redacted,
            show_room_and_member_changes: settings.show_room_and_member_changes,
        }
    }
}

impl Default for AccountNotificationSettings {
    fn default() -> Self {
        let settings = NotificationSettings::default();
        Self {
            sound: settings.sound,
            badges: settings.badges,
            desktop_notifications: settings.desktop_notifications,
            message_previews: settings.message_previews,
            send_read_receipts: settings.send_read_receipts,
            send_typing_notifications: settings.send_typing_notifications,
        }
    }
}

impl Default for AccountDisplaySettings {
    fn default() -> Self {
        let settings = DisplaySettings::default();
        Self {
            url_previews_enabled: settings.url_previews_enabled,
            encrypted_url_previews_enabled: settings.encrypted_url_previews_enabled,
        }
    }
}

impl Default for AccountSearchCrawlerSettings {
    fn default() -> Self {
        let settings = SearchCrawlerSettings::default();
        Self {
            include_media_captions: settings.include_media_captions,
            include_filenames: settings.include_filenames,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct SidebarSettings {
    #[serde(default)]
    pub category: SidebarCategory,
    #[serde(default)]
    pub collapsed: SidebarCollapsedSections,
    #[serde(default)]
    pub scope_preferences: BTreeMap<String, SidebarScopeSettings>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct SidebarSectionSettings {
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub sort: RoomListSort,
    /// #1363/#1366: the persisted row bound of this section. `false`, the
    /// default, keeps the section at
    /// [`crate::SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT`] rows; `true` shows every
    /// row. The bound is a second, independent dimension from `collapsed`:
    /// collapsing hides every row, expanding shows the bounded list.
    #[serde(default)]
    pub expanded: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarScopeSettings {
    #[serde(default)]
    pub rooms: SidebarSectionSettings,
    #[serde(default)]
    pub dms: SidebarSectionSettings,
    /// Absent until this scope's Low priority section is edited. `None` keeps
    /// the legacy device-global [`SidebarCollapsedSections::low_priority`] flag
    /// authoritative for the collapse state, so an older persisted settings
    /// file does not silently expand a section the user had collapsed.
    #[serde(default)]
    pub low_priority: Option<SidebarSectionSettings>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SidebarSectionKind {
    Rooms,
    Dms,
    LowPriority,
}

/// #1366: the section a conversation selection revealed in the current session.
///
/// A Space or the user can leave a section collapsed or bounded, and selecting a
/// conversation through a notification or permalink must still show its row.
/// The reducer records the reveal here when the selection settles; the sidebar
/// projection renders that section open until the user's own collapse choice for
/// the same scope and section clears it. Session-scoped on purpose: the persisted section
/// preferences stay the user's, and a restart honors them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarSectionReveal {
    /// The `SidebarSettings::scope` key: a Space id, or
    /// [`SIDEBAR_HOME_SCOPE_KEY`] at Home.
    pub scope: String,
    pub section: SidebarSectionKind,
}

/// The Home scope's key in `SidebarSettings.scope_preferences`.
pub const SIDEBAR_HOME_SCOPE_KEY: &str = "__home__";

/// The `SidebarSettings::scope` key for a scope id (`None` is Home).
pub fn sidebar_scope_key(scope: Option<&str>) -> &str {
    scope.unwrap_or(SIDEBAR_HOME_SCOPE_KEY)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarSectionPatch {
    pub scope: String,
    pub section: SidebarSectionKind,
    #[serde(default)]
    pub collapsed: Option<bool>,
    #[serde(default)]
    pub sort: Option<RoomListSort>,
    #[serde(default)]
    pub expanded: Option<bool>,
}

impl SidebarSectionKind {
    /// The Settings scope field that owns this section, or `None` for a section
    /// without a scope preference.
    pub fn scope_field(self, settings: &SidebarScopeSettings) -> Option<&SidebarSectionSettings> {
        match self {
            SidebarSectionKind::Rooms => Some(&settings.rooms),
            SidebarSectionKind::Dms => Some(&settings.dms),
            SidebarSectionKind::LowPriority => settings.low_priority.as_ref(),
        }
    }
}

impl SidebarSettings {
    pub fn scope(&self, scope: Option<&str>, fallback_sort: RoomListSort) -> SidebarScopeSettings {
        let scope = sidebar_scope_key(scope);
        let mut resolved = self
            .scope_preferences
            .get(scope)
            .copied()
            .unwrap_or_else(|| SidebarScopeSettings {
                rooms: SidebarSectionSettings {
                    sort: fallback_sort,
                    ..SidebarSectionSettings::default()
                },
                dms: SidebarSectionSettings {
                    sort: fallback_sort,
                    ..SidebarSectionSettings::default()
                },
                low_priority: None,
            });
        // The Low priority section has no independent sort: it follows the
        // scope's Rooms order. Its unset collapse state falls back to the
        // legacy device-global flag (state-machine.md, Settings). Its bound is
        // scoped like the other sections and defaults to the bounded state.
        resolved.low_priority = Some(SidebarSectionSettings {
            collapsed: resolved
                .low_priority
                .map_or(self.collapsed.low_priority, |section| section.collapsed),
            sort: resolved.rooms.sort,
            expanded: resolved
                .low_priority
                .is_some_and(|section| section.expanded),
        });
        resolved
    }

    pub fn apply_section_patch(&mut self, patch: SidebarSectionPatch, fallback_sort: RoomListSort) {
        let inherited = self.scope(Some(&patch.scope), fallback_sort);
        let settings = self
            .scope_preferences
            .entry(patch.scope)
            .or_insert(inherited);
        let section = match patch.section {
            SidebarSectionKind::Rooms => &mut settings.rooms,
            SidebarSectionKind::Dms => &mut settings.dms,
            SidebarSectionKind::LowPriority => settings
                .low_priority
                .get_or_insert(SidebarSectionSettings::default()),
        };
        if let Some(collapsed) = patch.collapsed {
            section.collapsed = collapsed;
        }
        if let Some(sort) = patch.sort {
            section.sort = sort;
        }
        if let Some(expanded) = patch.expanded {
            section.expanded = expanded;
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
    /// Show the triggering message's plain-text content in OS notifications.
    ///
    /// Device-local and OFF by default (#994, #1054, engineering rule 9): OS
    /// notifications carry counts only until the user opts in. Settings files
    /// written before this field existed load as OFF instead of failing the
    /// whole settings load; schema-version 2 resets the retired ON default that
    /// older files persisted (see `koushi_core::settings`).
    #[serde(default)]
    pub message_previews: bool,
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
            message_previews: false,
            send_read_receipts: true,
            send_typing_notifications: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[derive(Default)]
pub enum RoomNotificationMode {
    #[default]
    All,
    Mentions,
    Mute,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct RoomNotificationSettings {
    pub mode: RoomNotificationMode,
    pub operation: RoomNotificationModeOperation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[derive(Default)]
pub enum RoomNotificationModeOperation {
    #[default]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    #[serde(default = "default_code_block_wrap")]
    pub code_block_wrap: bool,
    #[serde(default = "default_hide_redacted")]
    pub hide_redacted: bool,
    #[serde(default = "default_show_room_and_member_changes")]
    pub show_room_and_member_changes: bool,
    #[serde(default = "default_url_previews_enabled")]
    pub url_previews_enabled: bool,
    #[serde(default = "default_encrypted_url_previews_enabled")]
    pub encrypted_url_previews_enabled: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            code_block_wrap: true,
            hide_redacted: default_hide_redacted(),
            show_room_and_member_changes: true,
            url_previews_enabled: true,
            encrypted_url_previews_enabled: default_encrypted_url_previews_enabled(),
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
    /// Preferred emoji panel width/height in CSS pixels, shared by all pickers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji_picker_size: Option<[u16; 2]>,
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
            emoji_picker_size: None,
        }
    }
}

/// Cross-platform desktop update preference.
///
/// The desktop adapter decides whether the current release target supports
/// updates. Keeping only the user policy here avoids platform-specific state in
/// the reusable application model.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdatesSettings {
    #[serde(default = "default_true")]
    pub auto_check: bool,
    #[serde(default)]
    pub include_prereleases: bool,
}

impl Default for UpdatesSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            include_prereleases: false,
        }
    }
}

/// Local MCP bridge policy (#1536), app scope. When enabled, assistants on this
/// computer may connect and ask for access, which Koushi approves per account
/// and permission; the grants themselves live in the MCP grant store, never
/// here, because they name accounts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Who may appear in an account's mention feed (#2110).
    #[serde(default)]
    pub allowed_senders: MentionAllowedSenders,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_senders: MentionAllowedSenders::default(),
        }
    }
}

/// An account's agent role (#2110). Its presence in [`AgentAccounts`] is the
/// role. It carries no options: an agent tab is always visible (#2130), and a
/// `hide_tab` stored by an earlier version is ignored on load and not written
/// back on the next save.
///
/// Ignoring the old field relies on this struct **not** having
/// `#[serde(deny_unknown_fields)]`; do not add it, or a stored `hide_tab`
/// would fail the whole settings load.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentAccountRole {}

/// The accounts the owner marked as agent accounts, keyed by user id.
///
/// Device-level and keyed by user id rather than kept in the account's own
/// settings: signing an account out deletes its per-account store, and the role
/// has to still be there when the same user signs in again.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentAccounts {
    #[serde(default)]
    pub accounts: BTreeMap<String, AgentAccountRole>,
}

impl AgentAccounts {
    pub fn role_for(&self, user_id: &str) -> Option<AgentAccountRole> {
        self.accounts.get(user_id).copied()
    }

    pub fn apply(&mut self, change: &AgentAccountPatch) {
        let user_id = change.user_id.trim();
        if user_id.is_empty() {
            return;
        }
        match change.role {
            Some(role) => {
                self.accounts.insert(user_id.to_owned(), role);
            }
            None => {
                self.accounts.remove(user_id);
            }
        }
    }
}

/// Sets (`Some`) or clears (`None`) one account's agent role. One account per
/// patch, so two edits cannot overwrite each other's accounts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentAccountPatch {
    pub user_id: String,
    pub role: Option<AgentAccountRole>,
}

/// The owner's edits to the mention feed's allow-list (#2110). The list is
/// *derived*: by default it is every account signed into this Koushi that is
/// not an agent account (#2130; two agents would otherwise wake each other in a
/// reply loop), so an account added later is covered without editing anything.
/// `added` widens it, which is how an agent account is allowed explicitly
/// with other users; `removed` narrows it, which is how the owner stops
/// answering one of their own accounts. Both are user ids.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionAllowedSenders {
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

impl MentionAllowedSenders {
    /// The users whose mentions may enter the feed: the signed-in accounts
    /// that are not agent accounts, minus `removed`, plus `added`.
    pub fn effective<'a>(
        &'a self,
        signed_in_user_ids: impl IntoIterator<Item = &'a str>,
        agent_accounts: &AgentAccounts,
    ) -> std::collections::BTreeSet<String> {
        signed_in_user_ids
            .into_iter()
            .filter(|user_id| agent_accounts.role_for(user_id).is_none())
            .filter(|user_id| !self.removed.iter().any(|removed| removed == user_id))
            .map(str::to_owned)
            .chain(self.added.iter().cloned())
            .collect()
    }

    /// Normalizes a patch: trimmed, de-duplicated, non-empty ids, and a user is
    /// never both added and removed (the later intent, `added`, wins).
    pub fn canonicalize(&mut self) {
        for list in [&mut self.added, &mut self.removed] {
            let mut seen = std::collections::BTreeSet::new();
            list.iter_mut().for_each(|id| *id = id.trim().to_owned());
            list.retain(|id| !id.is_empty() && seen.insert(id.clone()));
        }
        let added = self.added.clone();
        self.removed.retain(|id| !added.contains(id));
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Media settings.
///
/// #305 retired the automatic-compression mode: the staging dialog always asks
/// and starts at the untouched output, so there is no preference left to store.
/// The policy remains because the encoder still reads its quality value and the
/// direct upload path still reads its thresholds.
#[derive(Default)]
pub struct MediaSettings {
    #[serde(default)]
    pub image_upload_compression_policy: ImageUploadCompressionPolicy,
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
    #[serde(default = "default_timeline_thread_root_order")]
    pub thread_root_order: TimelineThreadRootOrder,
}

impl Default for TimelineSettings {
    fn default() -> Self {
        Self {
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
#[derive(Default)]
pub enum ThreadListOrder {
    #[default]
    LatestReply,
    RootChronology,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[derive(Default)]
pub enum RoomListSort {
    /// #438's attention-first order. No longer the default and no longer
    /// offered by the sidebar selector (#542), but still deserialized so an
    /// older persisted preference keeps its explicit choice.
    Activity,
    #[default]
    RecentFirst,
    NormalLocale,
}

// SearchCrawlerSettings and SearchCrawlerSpeed live in state/search_crawler.rs
// and are re-exported from mod.rs.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsPatchScope {
    Account,
    App,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SettingsPersistenceState {
    Idle,
    Saving { request_id: u64 },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<SettingsPatchScope>,
    pub locale: Option<LocaleSettings>,
    pub appearance: Option<AppearanceSettings>,
    pub typography: Option<TypographySettings>,
    pub keyboard: Option<KeyboardSettings>,
    pub composer: Option<ComposerSettings>,
    pub notifications: Option<NotificationSettings>,
    /// Account-owned device-local delivery (#1427). The patch carries the whole
    /// block, as the notification block does, and Core re-canonicalizes it.
    #[serde(default)]
    pub delivery: Option<NotificationDeliverySettings>,
    /// Whether `delivery` carries credential bindings the core itself just wrote. It never
    /// crosses IPC, so every patch from the renderer keeps the bindings Rust holds.
    #[serde(skip)]
    pub delivery_bindings_from_core: bool,
    pub display: Option<DisplaySettings>,
    pub media: Option<MediaSettings>,
    pub timeline: Option<TimelineSettings>,
    pub thread_list_order: Option<ThreadListOrder>,
    pub room_list_sort: Option<RoomListSort>,
    pub search_crawler: Option<SearchCrawlerSettings>,
    #[serde(default)]
    pub sidebar: Option<SidebarSettings>,
    #[serde(default)]
    pub sidebar_section: Option<SidebarSectionPatch>,
    #[serde(default)]
    pub window: Option<WindowSettings>,
    #[serde(default)]
    pub updates: Option<UpdatesSettings>,
    #[serde(default)]
    pub mcp: Option<McpSettings>,
    /// The owner's edits to the mention feed's allow-list (#2110). Separate from
    /// `mcp` so that flipping the bridge switch, which sends only `enabled`,
    /// never replaces the list.
    #[serde(default)]
    pub mcp_allowed_senders: Option<MentionAllowedSenders>,
    /// Marks or unmarks one account as an agent account (#2110); app scope.
    #[serde(default)]
    pub agent_account: Option<AgentAccountPatch>,
    #[serde(default)]
    pub legacy_frontend_preferences_imported: Option<bool>,
}

impl SettingsPatch {
    pub fn affects_account_settings(&self) -> bool {
        self.scope != Some(SettingsPatchScope::App)
            && (self.composer.is_some()
                || self.notifications.is_some()
                || self.delivery.is_some()
                || self.display.is_some()
                || self.room_list_sort.is_some()
                || self.search_crawler.is_some()
                || self.sidebar.is_some()
                || self.sidebar_section.is_some())
    }
}
