export type SearchScopeKind = "currentRoom" | "currentSpace" | "dms" | "allRooms";

export interface DesktopSnapshot {
  state: AppState;
  sidebar: SidebarModel;
  timeline: TimelineMessage[];
  thread: ThreadSnapshot | null;
}

export interface SavedSessionInfo {
  homeserver: string;
  user_id: string;
  device_id: string;
}

export interface AppState {
  session: SessionState;
  auth: AuthDiscoveryState;
  settings: SettingsState;
  locale_profile: LocaleDisplayProfile;
  typography_profile: TypographyDisplayProfile;
  profile: ProfileState;
  sync: SyncState;
  navigation: NavigationState;
  spaces: SpaceSummary[];
  rooms: RoomSummary[];
  invites: InvitePreview[];
  room_interactions: Record<string, RoomInteractionState>;
  directory: DirectoryState;
  room_management: RoomManagementState;
  activity: ActivityState;
  timeline: TimelinePaneState;
  thread: ThreadPaneState;
  focused_context: FocusedContextState;
  search: SearchState;
  errors: AppError[];
  basic_operation: BasicOperationState;
  live_signals: LiveSignalsState;
  e2ee_trust: E2eeTrustState;
  local_encryption: LocalEncryptionState;
  native_attention: NativeAttentionState;
  cjk_text_policy: CjkTextPolicyState;
}

export interface SettingsState {
  values: SettingsValues;
  persistence: SettingsPersistenceState;
}

export interface SettingsValues {
  locale: LocaleSettings;
  appearance: AppearanceSettings;
  typography: TypographySettings;
  keyboard: KeyboardSettings;
}

export interface SettingsPatch {
  locale?: LocaleSettings;
  appearance?: AppearanceSettings;
  typography?: TypographySettings;
  keyboard?: KeyboardSettings;
}

export interface LocaleSettings {
  language_tag: string | null;
  text_direction: TextDirectionPreference;
}

export type TextDirectionPreference = "auto" | "ltr" | "rtl";

export interface LocaleDisplayProfile {
  lang: string;
  dir: LocaleDirection;
  catalog_locale: CatalogLocale;
  pseudo_locale: LocalePseudoMode;
  platform: DisplayPlatform;
  modifier_labels: ModifierLabelProfile;
}

export type LocaleDirection = "ltr" | "rtl";
export type CatalogLocale = "en" | "ja" | "pseudo";
export type LocalePseudoMode = "none" | "accented" | "bidi";
export type DisplayPlatform = "macos" | "windows" | "linux";
export type PrimaryModifierLabel = "Cmd" | "Ctrl";

export interface ModifierLabelProfile {
  primary: PrimaryModifierLabel;
}

export interface TypographyDisplayProfile {
  font: FontPreference;
  emoji: EmojiPreference;
  platform: DisplayPlatform;
  font_asset: TypographyAssetStatus;
  emoji_asset: TypographyAssetStatus;
}

export type TypographyAssetStatus = "systemFallback" | "bundledPreferred";

export interface AppearanceSettings {
  theme: ThemePreference;
}

export type ThemePreference = "system" | "light" | "dark";

export interface TypographySettings {
  font: FontPreference;
  emoji: EmojiPreference;
}

export type FontPreference = "system" | "inter";
export type EmojiPreference = "system" | "twemojiColr";

export interface KeyboardSettings {
  composer_send_shortcut: ComposerSendShortcut;
}

export type ComposerSendShortcut = "enter" | "modEnter";

export type SettingsPersistenceState =
  | { kind: "idle" }
  | { kind: "saving"; request_id: number };

export type ComposerSurface = "main" | "thread" | "edit";
export type ComposerKey = "enter" | "escape" | "other";

export interface ComposerKeyModifiers {
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
  alt: boolean;
}

export interface ComposerKeyEvent {
  key: ComposerKey;
  modifiers: ComposerKeyModifiers;
  is_composing: boolean;
}

export interface ComposerResolverOptions {
  autocomplete_open: boolean;
  send_enabled: boolean;
}

export type ComposerResolvedAction =
  | "send"
  | "insertNewline"
  | "acceptAutocomplete"
  | "cancel"
  | "ignore";

export type ResolveComposerKeyAction = (
  surface: ComposerSurface,
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
) => Promise<ComposerResolvedAction>;

export type AuthDiscoveryState =
  | { kind: "unknown" }
  | { kind: "discovering"; homeserver: string }
  | { kind: "ready"; homeserver: string; flows: LoginFlow[] }
  | { kind: "failed"; homeserver: string; message: string };

export interface LoginFlow {
  kind: "password" | "sso" | "token" | { unknown: string };
  delegated_oidc_compatibility: boolean;
}

export type RecoveryMethod = "recoveryKey" | "securityPhrase";

export interface SessionState {
  kind:
    | "signedOut"
    | "restoring"
    | "switchingAccount"
    | "authenticating"
    | "needsRecovery"
    | "recovering"
    | "ready"
    | "locked"
    | "loggingOut";
  homeserver?: string;
  user_id?: string;
  device_id?: string;
  recovery_methods?: RecoveryMethod[];
}

export type SyncState =
  | "stopped"
  | "starting"
  | "running"
  | { failed: string }
  | { reconnecting: string };

export interface NavigationState {
  active_space_id: string | null;
  active_room_id: string | null;
}

export interface ProfileState {
  own: OwnProfile;
  users: Record<string, UserProfile>;
  update: ProfileUpdateState;
}

export interface OwnProfile {
  display_name: string | null;
  avatar: AvatarImage | null;
}

export interface UserProfile {
  user_id: string;
  display_name: string | null;
  avatar: AvatarImage | null;
}

export interface AvatarImage {
  mxc_uri: string;
  thumbnail: AvatarThumbnailState;
}

export type AvatarThumbnailState =
  | { kind: "notRequested" }
  | { kind: "loading"; request_id: number }
  | {
      kind: "ready";
      source_url: string;
      width: number | null;
      height: number | null;
      mime_type: string | null;
    }
  | { kind: "failed"; request_id: number; failureKind: AvatarThumbnailFailureKind };

export type AvatarThumbnailFailureKind = "network" | "forbidden" | "unsupported" | "sdk";

export type ProfileUpdateState =
  | { kind: "idle" }
  | { kind: "settingDisplayName"; request_id: number; display_name: string | null }
  | { kind: "settingAvatar"; request_id: number; mime_type: string; byte_count: number };

export interface SpaceSummary {
  space_id: string;
  display_name: string;
  avatar: AvatarImage | null;
  child_room_ids: string[];
}

export type RoomTagKind = "favourite" | "lowPriority";

export interface RoomTagInfo {
  order: string | null;
}

export interface RoomTags {
  favourite: RoomTagInfo | null;
  low_priority: RoomTagInfo | null;
}

export interface RoomSummary {
  room_id: string;
  display_name: string;
  avatar: AvatarImage | null;
  is_dm: boolean;
  tags: RoomTags;
  unread_count: number;
  notification_count?: number;
  highlight_count?: number;
  parent_space_ids: string[];
}

export interface InvitePreview {
  room_id: string;
  display_name: string;
  avatar: AvatarImage | null;
  topic: string | null;
  inviter_display_name: string | null;
  is_dm: boolean;
}

export interface RoomInteractionState {
  pinned_events: PinnedEvent[];
  pin_operation: PinOperationState;
}

export interface PinnedEvent {
  event_id: string;
  sender_display_name: string | null;
  body_preview: string | null;
  timestamp_ms: number | null;
}

export type PinOperationState =
  | { kind: "idle" }
  | { kind: "pinning"; request_id: number; event_id: string }
  | { kind: "unpinning"; request_id: number; event_id: string }
  | {
      kind: "failed";
      request_id: number;
      event_id: string;
      failureKind: OperationFailureKind;
    };

export type OperationFailureKind =
  | "forbidden"
  | "notFound"
  | "network"
  | "timeout"
  | "invalid"
  | "sdk";

export type ActivityState =
  | { kind: "closed" }
  | { kind: "opening"; request_id: number; tab: ActivityTab }
  | { kind: "open"; active_tab: ActivityTab };

export type ActivityTab = "recent" | "unread";

export type DirectoryState =
  | { kind: "closed" }
  | { kind: "querying"; request_id: number; query: DirectoryQuery }
  | {
      kind: "results";
      request_id: number;
      query: DirectoryQuery;
      rooms: DirectoryRoomSummary[];
      next_batch: string | null;
    }
  | { kind: "joining"; request_id: number; room_id: string }
  | {
      kind: "failed";
      request_id: number;
      query: DirectoryQuery;
      failureKind: OperationFailureKind;
    };

export interface DirectoryQuery {
  term: string | null;
  server_name: string | null;
  limit: number | null;
}

export interface DirectoryRoomSummary {
  room_id: string;
  name: string;
  topic: string | null;
  avatar_url: string | null;
  joined_members: number;
  world_readable: boolean;
  guest_can_join: boolean;
}

export interface RoomManagementState {
  selected_room_id: string | null;
  operation: RoomManagementOperationState;
}

export type RoomManagementOperationState =
  | { kind: "idle" }
  | { kind: "loading"; request_id: number; room_id: string }
  | {
      kind: "mutating";
      request_id: number;
      room_id: string;
      operation: RoomManagementOperationKind;
    }
  | {
      kind: "failed";
      request_id: number;
      room_id: string;
      failureKind: OperationFailureKind;
    };

export type RoomManagementOperationKind = "settings" | "moderation" | "permissions";

export interface TimelinePaneState {
  room_id: string | null;
  is_subscribed: boolean;
  is_paginating_backwards: boolean;
  composer: ComposerState;
}

export interface ComposerState {
  pending_transaction_id: string | null;
  draft: string;
  mode: ComposerMode;
}

// Rust ComposerMode has NO serde tag → externally tagged (like SyncState in this file)
export type ComposerMode =
  | "Plain"
  | { Reply: { in_reply_to_event_id: string } };

// Rust BasicOperationState is #[serde(tag = "kind", rename_all = "camelCase")]
// → internally tagged, camelCase VARIANT names, snake_case fields. Pending
// variants carry the correlation request_id (see docs/architecture/state-machine.md).
export type BasicOperationState =
  | { kind: "idle" }
  | { kind: "creatingRoom"; request_id: number; name: string }
  | { kind: "creatingSpace"; request_id: number; name: string }
  | { kind: "linkingSpaceChild"; request_id: number; space_id: string; child_room_id: string };

export interface LiveSignalsState {
  rooms: Record<string, RoomLiveSignals>;
  presence: Record<string, PresenceKind>;
}

export interface RoomLiveSignals {
  receipts_by_event: Record<string, LiveReadReceipt[]>;
  fully_read_event_id: string | null;
  typing_user_ids: string[];
}

export interface LiveReadReceipt {
  user_id: string;
  timestamp_ms: number | null;
}

export interface LiveEventReceipts {
  event_id: string;
  receipts: LiveReadReceipt[];
}

export interface LiveRoomSignalUpdate {
  receipts_by_event: LiveEventReceipts[];
  fully_read_event_id: string | null;
  typing_user_ids: string[];
}

export type PresenceKind = "online" | "away" | "offline";

export interface E2eeTrustState {
  verification: VerificationFlowState;
  cross_signing: CrossSigningStatus;
  key_backup: KeyBackupStatus;
  identity_reset: IdentityResetState;
  devices: DeviceTrustSummary[];
}

export type VerificationFlowState =
  | { kind: "idle" }
  | { kind: "requested"; request_id: number; target: VerificationTarget }
  | { kind: "accepted"; request_id: number; target: VerificationTarget }
  | {
      kind: "sasPresented";
      request_id: number;
      target: VerificationTarget;
      emojis: SasEmoji[];
    }
  | {
      kind: "confirming";
      request_id: number;
      target: VerificationTarget;
      emojis: SasEmoji[];
    }
  | { kind: "done"; request_id: number; target: VerificationTarget }
  | {
      kind: "failed";
      request_id: number;
      target: VerificationTarget;
      failureKind: TrustOperationFailureKind;
    };

export interface VerificationTarget {
  user_id: string;
  device_id: string;
}

export interface SasEmoji {
  symbol: string;
  description: string;
}

export type CrossSigningStatus =
  | { kind: "unknown" }
  | { kind: "missing" }
  | { kind: "bootstrapping"; request_id: number }
  | { kind: "trusted" }
  | { kind: "notTrusted" }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type KeyBackupStatus =
  | { kind: "unknown" }
  | { kind: "disabled" }
  | { kind: "enabling"; request_id: number }
  | { kind: "enabled"; version: string }
  | {
      kind: "restoring";
      request_id: number;
      version: string | null;
      restored_rooms: number;
      total_rooms: number | null;
    }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type IdentityResetState =
  | { kind: "idle" }
  | { kind: "resetting"; request_id: number }
  | { kind: "awaitingAuth"; request_id: number; auth_type: IdentityResetAuthType }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type IdentityResetAuthType = "uiaa" | "oauth" | "unknown";

export interface DeviceTrustSummary {
  user_id: string;
  device_id: string;
  trust_level: DeviceTrustLevel;
}

export type DeviceTrustLevel = "unknown" | "unverified" | "verified" | "blocked";

export type TrustOperationFailureKind =
  | "cancelled"
  | "mismatch"
  | "network"
  | "forbidden"
  | "timeout"
  | "sdk";

export type LocalEncryptionState =
  | { kind: "unknown" }
  | { kind: "probing"; request_id: number }
  | { kind: "healthy" }
  | { kind: "unavailable" }
  | { kind: "lockedOrInaccessible" }
  | { kind: "missingCredential" }
  | { kind: "resetRequired" }
  | { kind: "resetting"; request_id: number };

export type LocalEncryptionHealth =
  | "unknown"
  | "healthy"
  | "unavailable"
  | "lockedOrInaccessible"
  | "missingCredential"
  | "resetRequired";

export interface NativeAttentionState {
  summary: NativeAttentionSummary;
  dispatch: NativeAttentionDispatchState;
}

export interface NativeAttentionSummary {
  unread_count: number;
  highlight_count: number;
  badge_count: number;
  candidate: NativeAttentionCandidate | null;
  capabilities: NativeAttentionCapabilities;
}

export interface NativeAttentionCandidate {
  room_display_name: string;
  kind: RoomAttentionKind;
  unread_count: number;
  highlight_count: number;
}

export type RoomAttentionKind = "mention" | "dm" | "message";

export interface NativeAttentionCapabilities {
  notifications: NativeAttentionCapability;
  badge: NativeAttentionCapability;
  sound: NativeAttentionCapability;
  tray: NativeAttentionCapability;
  activation: NativeAttentionCapability;
}

export type NativeAttentionCapability = "available" | "unavailable" | "unknown";

export type NativeAttentionDispatchState =
  | { kind: "idle" }
  | { kind: "dispatching"; request_id: number }
  | { kind: "delivered"; request_id: number }
  | { kind: "suppressed"; reason: NativeAttentionSuppressionReason }
  | { kind: "failed"; request_id: number; failureKind: OperationFailureKind };

export type NativeAttentionSuppressionReason =
  | "windowFocused"
  | "roomMuted"
  | "lowPriority"
  | "duplicate"
  | "capabilityUnavailable";

export interface CjkTextPolicyState {
  japanese_catalog: JapaneseCatalogProfile;
  normalization: CjkNormalizationProfile;
  collation: CjkCollationProfile;
}

export interface JapaneseCatalogProfile {
  catalog_locale: string;
  complete: boolean;
  missing_message_ids: string[];
}

export interface CjkNormalizationProfile {
  form: string;
  width_fold: boolean;
  kana_fold: boolean;
}

export interface CjkCollationProfile {
  locale: string;
  numeric: boolean;
  case_first: string | null;
}

export interface ThreadPaneState {
  kind: "closed" | "opening" | "open";
  room_id?: string;
  root_event_id?: string;
  is_subscribed?: boolean;
  composer?: ComposerState;
}

export type FocusedContextState =
  | { kind: "closed" }
  | { kind: "opening"; room_id: string; event_id: string }
  | {
      kind: "open";
      room_id: string;
      event_id: string;
      is_subscribed: boolean;
    };

export type SearchState =
  | { kind: "closed" }
  | { kind: "editing"; query: string; scope: SearchScopeKind }
  | { kind: "searching"; request_id: number; query: string; scope: SearchScopeKind }
  | {
      kind: "results";
      request_id: number;
      query: string;
      scope: SearchScopeKind;
      results: SearchResult[];
    }
  | {
      kind: "failed";
      request_id: number;
      query: string;
      scope: SearchScopeKind;
      message: string;
    };

export interface SearchResult {
  room_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  score_millis: number;
  snippet: string;
  match_field: "messageBody" | "attachmentFileName";
  highlights: TextRange[];
  match_kind: "exact";
}

export interface TextRange {
  start_utf16: number;
  end_utf16: number;
}

export interface AppError {
  code: string;
  message: string;
  recoverable: boolean;
}

export interface SidebarModel {
  active_space_id: string | null;
  account_home: AccountHomeItem;
  space_rail: SpaceRailItem[];
  space_rooms: RoomListItem[];
  global_dms: RoomListItem[];
  space_unread_count: number;
  dm_unread_count: number;
}

export interface AccountHomeItem {
  display_name: string;
  unread_count: number;
  is_active: boolean;
}

export interface SpaceRailItem {
  space_id: string;
  display_name: string;
  avatar: AvatarImage | null;
  unread_count: number;
  is_active: boolean;
}

export interface RoomListItem {
  room_id: string;
  display_name: string;
  avatar: AvatarImage | null;
  tags: RoomTags;
  unread_count: number;
}

export interface TimelineMessage {
  room_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  body: string;
  attachment_filename: string | null;
  reply_count: number;
}

export interface ThreadSnapshot {
  room_id: string;
  root_event_id: string;
  replies: ThreadMessage[];
}

export interface ThreadMessage {
  room_id: string;
  root_event_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  body: string;
}

export interface VisibleRooms {
  spaceRooms: RoomListItem[];
  globalDms: RoomListItem[];
}
