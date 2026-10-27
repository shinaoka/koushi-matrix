import type { MessageId } from "../i18n/messages";
import type { LinkPreview } from "./linkPreview";

export type SearchScopeKind = "currentRoom" | "currentSpace" | "allRooms";

export interface RoomAddressPreview {
  localpart: string;
  full_alias: string | null;
  error: "empty" | "invalid" | "notReady" | null;
  /** The server whose alias namespace every Space on it shares (#1006). */
  server_name: string | null;
}

export interface CreateRoomRequest {
  name: string;
  topic?: string | null;
  aliasLocalpart?: string | null;
  encrypted: boolean;
  invitedOnly: boolean;
  visibility: CreateRoomVisibility;
  parentSpace?: CreateRoomParentSpace | null;
  /** An explicit access policy (#1177); omitted keeps the legacy presets. An
   * explicitly selected allow Space need not be the attachment and never
   * writes `m.space.child`. */
  accessPolicy?: RoomAccessPolicy | null;
  /** An explicit history visibility (#1177); omitted keeps the legacy
   * private-in-Space `invited` default. */
  history?: RoomHistoryVisibility | null;
}

/** A settable access policy: a join rule plus its canonical allow Space ids (#1177). */
export interface RoomAccessPolicy {
  rule: RoomJoinRule;
  allowTargets: string[];
}

export type CreateRoomVisibility = "private" | "public";

/** Rust-projected initial choices of the create-room dialog (#1023). */
export interface CreateRoomDefaults {
  visibility: CreateRoomVisibility;
  /** The private option's encryption choice; public rooms are unencrypted. */
  encrypted: boolean;
  invited_only: boolean;
}

/** Core derives both relationship events' routing from the SDK (#1007). */
export interface CreateRoomParentSpace {
  spaceId: string;
}

export interface DesktopSnapshot {
  account_tab_id?: string;
  state_generation?: number;
  state: AppState;
  sidebar: SidebarModel;
  timeline: TimelineMessage[];
  thread: ThreadSnapshot | null;
}

export type AccountTabStatus =
  | "addAccount"
  | "restoring"
  | "authenticating"
  | "needsVerification"
  | "ready"
  | "signedOut"
  | "loggingOut"
  | "error";

export interface AccountTabSummary {
  id: string;
  accountKey: string | null;
  homeserver: string | null;
  displayName: string | null;
  avatarSourceRef: string | null;
  status: AccountTabStatus;
  unreadCount: number;
  inviteCount?: number;
}

export interface AccountTabsSnapshot {
  selectedTabId: string;
  tabs: AccountTabSummary[];
  badgeCount: number;
}

export interface SavedSessionInfo {
  homeserver: string;
  user_id: string;
  device_id: string;
}

export type ComposerDraftAccountOwner = SavedSessionInfo;

/**
 * IPC snapshot contract version. Must match `dto.rs`'s `SNAPSHOT_SCHEMA_VERSION`.
 * Version 9 adds the persisted sidebar section bound
 * (`SidebarSectionSettings.expanded`) and its projection
 * (`SidebarModel.section_bounds`).
 * Version 10 adds the device-local notification delivery projection (#1427).
 * Version 11 adds the keyword-rule projection
 * (`AccountNotificationsSnapshot.keywords`), the account's own `content` rules
 * in server order with their server-owned enabled flag (#1427).
 */
export const SNAPSHOT_SCHEMA_VERSION = 12;

/**
 * Snapshot state. #87 Phase 4 sectioned this into domain (Matrix/product, Rust-owned,
 * mobile-reusable) and ui (desktop presentation/view/navigation). `schema_version` is the
 * IPC contract version; the App boundary asserts it so a stale flat (v1)
 * snapshot or a mismatched build fails loudly.
 */
export interface AppState {
  schema_version: number;
  domain: AppDomainState;
  ui: AppUiState;
}

export interface AppDomainState {
  session: SessionState;
  session_epoch?: number;
  session_lock_reason: SessionLockReason | null;
  secure_backup_gate: SecureBackupGateState;
  current_session_status: CurrentSessionStatusState;
  device_cleanup: DeviceCleanupState;
  auth: AuthDiscoveryState;
  account_management_url: string | null;
  account_management: AccountManagementState;
  account_management_capabilities: AccountManagementCapabilities;
  account_notifications: AccountNotificationsState;
  /**
   * #1427: the account-scoped runtime half of this device's own notification delivery. The
   * settings half is `settings.values.delivery`; a sink's credential is never here, only
   * whether one is configured. App state, never part of the settings file.
   */
  notification_delivery: NotificationDeliveryState;
  contact_security: ContactSecurityState;
  sender_device_advisories: SenderDeviceAdvisoriesState;
  soft_logout_reauth: SoftLogoutReauthState;
  qr_login: QrLoginState;
  settings: SettingsState;
  link_preview_settings: LinkPreviewSettingsState;
  room_preferences: RoomPreferencesState;
  locale_profile: LocaleDisplayProfile;
  typography_profile: TypographyDisplayProfile;
  profile: ProfileState;
  space_members: SpaceMembersState;
  space_children: SpaceChildrenState;
  sync: SyncState;
  spaces: SpaceSummary[];
  rooms: RoomSummary[];
  invites: InvitePreview[];
  invite_workflow?: InviteWorkflowState;
  room_notification_settings: Record<string, RoomNotificationSettings>;
  room_interactions: Record<string, RoomInteractionState>;
  directory: DirectoryState;
  room_management: RoomManagementState;
  mention_candidates: MentionCandidatesState;
  activity: ActivityState;
  thread_attention: ThreadAttentionState;
  search: SearchState;
  search_crawler: SearchCrawlerState;
  live_signals: LiveSignalsState;
  e2ee_trust: E2eeTrustState;
  local_encryption: LocalEncryptionState;
  native_attention: NativeAttentionState;
  cjk_text_policy: CjkTextPolicyState;
}

export interface AppUiState {
  navigation: NavigationState;
  room_list: RoomListProjection;
  timeline: TimelinePaneState;
  thread: ThreadPaneState;
  focused_context: FocusedContextState;
  files_view: FilesViewState;
  history_export: HistoryExportState;
  threads_list: ThreadsListState;
  /** #1160: the explicitly opened Home/Space scheduled-sends projection. */
  scheduled_sends_list: ScheduledSendsListState;
  basic_operation: BasicOperationState;
  /** Rust-owned advisory address check of the create-room dialog (#1006). */
  room_address_availability: RoomAddressAvailabilityState;
  errors: AppError[];
}

export interface SettingsState {
  values: SettingsValues;
  persistence: SettingsPersistenceState;
}

export interface LinkPreviewSettingsState {
  room_overrides: Record<string, boolean>;
}

export interface RoomPreferencesState {
  rooms: Record<string, RoomPreference>;
}

export interface RoomPreference {
  url_previews_enabled_override?: boolean;
  notification_mode?: RoomNotificationMode;
}

export interface SettingsValues {
  locale: LocaleSettings;
  appearance: AppearanceSettings;
  typography: TypographySettings;
  keyboard: KeyboardSettings;
  composer: ComposerSettings;
  notifications: NotificationSettings;
  delivery: NotificationDeliverySettings;
  display: DisplaySettings;
  window: WindowSettings;
  updates: UpdatesSettings;
  mcp: McpSettings;
  media: MediaSettings;
  timeline: TimelineSettings;
  sidebar: SidebarSettings;
  search_crawler: SearchCrawlerSettings;
  thread_list_order: ThreadListOrder;
  room_list_sort: RoomListSort;
  legacy_frontend_preferences_imported: boolean;
}

export interface SettingsPatch {
  scope?: "account" | "app";
  locale?: LocaleSettings;
  appearance?: AppearanceSettings;
  typography?: TypographySettings;
  keyboard?: KeyboardSettings;
  composer?: ComposerSettings;
  notifications?: NotificationSettings;
  delivery?: NotificationDeliverySettings;
  display?: DisplaySettings;
  window?: WindowSettings;
  updates?: UpdatesSettings;
  mcp?: McpSettings;
  media?: MediaSettings;
  timeline?: TimelineSettings;
  sidebar?: SidebarSettings;
  sidebar_section?: SidebarSectionPatch;
  search_crawler?: SearchCrawlerSettings;
  thread_list_order?: ThreadListOrder;
  room_list_sort?: RoomListSort;
}

// Rust ThreadListOrder is #[serde(tag = "kind", rename_all = "camelCase")].
export type ThreadListOrder = { kind: "latestReply" } | { kind: "rootChronology" };

// Rust TimelineThreadRootOrder is #[serde(tag = "kind", rename_all = "camelCase")].
export type TimelineThreadRootOrder = { kind: "rootEvent" } | { kind: "latestReply" };

// Rust RoomListSort is #[serde(tag = "kind", rename_all = "camelCase")].
export type RoomListSort =
  | { kind: "activity" }
  | { kind: "recentFirst" }
  | { kind: "normalLocale" };

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
export type CatalogLocale = "en" | "ja" | "zh-Hans" | "zh-Hant" | "ko" | "pseudo";
/** Rust `ReceiptTimestampLocale`: a catalog locale with pseudo folded to English. */
export type ReceiptTimestampLocale = Exclude<CatalogLocale, "pseudo">;
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
  density: DisplayDensity;
}

export type ThemePreference = "system" | "light" | "dark";
export type DisplayDensity = "compact" | "default" | "comfortable";

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

export interface ComposerSettings {
  math_mode: boolean;
  recent_emojis: string[];
}

export type SidebarCategory = "rooms" | "people";

export interface SidebarCollapsedSections {
  favourites: boolean;
  low_priority: boolean;
  not_joined: boolean;
}

export interface SidebarSettings {
  category: SidebarCategory;
  collapsed: SidebarCollapsedSections;
  scope_preferences?: Record<string, SidebarScopeSettings>;
}

export interface SidebarScopeSettings {
  rooms: SidebarSectionSettings;
  dms: SidebarSectionSettings;
  /**
   * Absent until this scope's Low priority section is edited; the legacy
   * device-global `SidebarCollapsedSections.low_priority` flag is the fallback.
   */
  low_priority?: SidebarSectionSettings | null;
}

export interface SidebarSectionSettings {
  collapsed: boolean;
  sort: RoomListSort;
  /**
   * #1366: whether this scope's section shows every row. A bounded section
   * shows at most `SidebarSectionBound.visible_limit` rows.
   */
  expanded: boolean;
}

export type SidebarSectionKind = "rooms" | "dms" | "lowPriority";

export interface SidebarSectionPatch {
  scope: string;
  section: SidebarSectionKind;
  collapsed?: boolean;
  sort?: RoomListSort;
  expanded?: boolean;
}

export interface NotificationSettings {
  desktop_notifications: boolean;
  sound: boolean;
  badges: boolean;
  message_previews: boolean;
  send_read_receipts: boolean;
  send_typing_notifications: boolean;
}

export interface DisplaySettings {
  code_block_wrap: boolean;
  hide_redacted: boolean;
  url_previews_enabled: boolean;
  encrypted_url_previews_enabled: boolean;
}

/**
 * #1427/#1460: this device's own notification delivery. Account scope; the Matrix homeserver
 * is never asked for a destination, and a sink's credential never crosses this boundary:
 * the settings carry the id the credential store is keyed by and nothing else. Every
 * route decision - targets, content, timing - belongs to one destination; there is no
 * channel-wide switch.
 */
export interface NotificationDeliverySettings {
  sinks: NotificationSinkSettings[];
}

// Rust NotificationDeliveryTarget is #[serde(tag = "kind", rename_all = "camelCase")].
export type NotificationDeliveryTarget =
  | { kind: "room"; room_id: string }
  | { kind: "space"; space_id: string };

/**
 * What one destination is sent. A watched room or space is delivered whatever the
 * categories say, but never past read, mute, or focus suppression. A watched space covers
 * every joined child, resolved from the room list, so a room added later is covered.
 */
export interface NotificationDeliveryTargets {
  direct_messages: boolean;
  mentions: boolean;
  keywords: boolean;
  watched: NotificationDeliveryTarget[];
}

// Rust NotificationDeliveryContent is #[serde(tag = "kind", rename_all = "camelCase")].
export type NotificationDeliveryContent =
  | { kind: "namesOnly" }
  | { kind: "prefix"; graphemes: number }
  | { kind: "full" };

/**
 * The user's acceptance that one destination receives message text in plaintext. Rust
 * drops it, and returns the destination to names only, when the sink kind, its endpoint,
 * or its recipient changes; for ServerChan saving or clearing the SendKey is a recipient
 * change.
 */
export interface NotificationDeliveryContentConsent {
  sink_kind: NotificationSinkConfig["kind"];
  endpoint: string;
  recipient: string;
  accepted_at_ms: number;
}

export interface NotificationDeliveryTiming {
  dwell_secs: number;
  min_interval_secs: number;
  only_when_unfocused: boolean;
}

export interface NotificationSinkSettings {
  id: string;
  /** The mailbox for a mail sink; empty for ServerChan, whose SendKey names the recipient. */
  destination: string;
  from_address: string;
  sender_name: string;
  config: NotificationSinkConfig;
  /**
   * What the stored credential was entered for; Rust drops it when the server or account
   * changes. The user never edits it: a new sink sends `null`, an existing one echoes what
   * Rust last reported. A binding with no credential beside it is a password Rust retired
   * because the sink stopped being the server or account it was given for.
   */
  secret_binding: string | null;
  targets: NotificationDeliveryTargets;
  content: NotificationDeliveryContent;
  content_consent: NotificationDeliveryContentConsent | null;
  timing: NotificationDeliveryTiming;
  /**
   * #1492: how a one-to-one direct message names its partner - the local alias when one
   * is set, or always the full Matrix user id.
   */
  direct_partner_label: NotificationDeliveryPartnerLabel;
}

export type NotificationDeliveryPartnerLabel = "localAlias" | "matrixId";

/** #1492: the catalog's sample direct message the preview renders; never account data. */
export interface NotificationDeliveryPreviewSample {
  partnerUserId: string;
  partnerAlias: string;
  messageText: string;
}

/** #1492: the sample message a draft destination would send, rendered by Rust's digest. */
export interface NotificationDeliveryPreview {
  /** Empty for ServerChan, which has no sender name. */
  sender_name: string;
  subject: string;
  body: string;
}

/**
 * #1427: app state for this device's own notification delivery. The settings file holds
 * the policy; this holds what only the running app knows - whether a sink's credential is
 * in the credential store, and what its last send did. Rust owns every field.
 */
export interface NotificationDeliveryState {
  sinks: NotificationDeliverySinkState[];
  /** Mailbox presets Rust ships; `secret_hint` is a catalog key, resolved through `t`. */
  presets: NotificationDeliveryPresetState[];
}

export interface NotificationDeliverySinkState {
  sink_id: string;
  /** A credential exists in the OS credential store; the value itself never crosses IPC. */
  secret_configured: boolean;
  /**
   * Rust deleted a stored credential for this sink because its binding no longer named the
   * sink's server and account, whether on a patch or on the settings load that reconciled a
   * binding-less record. The reason is Rust's to state: the credential and the binding go in
   * the same round trip, so no comparison over the settings can recover why the password is
   * gone. False again as soon as a secret is saved or the user clears one.
   */
  secret_retired: boolean;
  last_outcome: NotificationDeliveryOutcomeState | null;
}

export type NotificationDeliveryOutcomeState =
  /** The provider accepted the request; never a claim that it was received. */
  | { kind: "accepted"; at_ms: number }
  | { kind: "invalidSecret"; at_ms: number }
  | { kind: "notPermitted"; at_ms: number }
  | { kind: "rejected"; at_ms: number }
  | { kind: "rateLimited"; at_ms: number; retry_after_secs: number | null }
  | { kind: "transport"; at_ms: number };

export interface NotificationDeliveryPresetState {
  id: string;
  /** The vendor's own proper name, deliberately not localized. */
  label: string;
  host: string;
  port: number;
  tls: SmtpTls;
  username_is_full_address: boolean;
  /** A catalog key; resolve it through the catalog, never render it directly. */
  secret_hint: MessageId;
}

// Rust NotificationSinkConfig is #[serde(tag = "kind", rename_all = "camelCase")].
export type NotificationSinkConfig =
  | { kind: "resend"; endpoint: string }
  | {
      kind: "smtp";
      preset?: string | null;
      host: string;
      port: number;
      tls: SmtpTls;
      username: string;
    }
  /** WeChat through ServerChan Turbo; the SendKey is the credential and names the recipient. */
  | { kind: "serverChan"; endpoint: string };

export type SmtpTls = "implicit" | "startTls";

export interface WindowSettings {
  emoji_picker_size?: [number, number] | null;
  close_to_tray: boolean;
}

export interface UpdatesSettings {
  auto_check: boolean;
  include_prereleases: boolean;
}

/** Local MCP bridge switch (#1536). App scope; grants live in the bridge's store. */
export interface McpSettings {
  enabled: boolean;
}

export type McpNeed = "read" | "send" | "attachments" | "message_actions";

export interface McpPendingApproval {
  request_id: number;
  /** Self-declared by the assistant; validated to `[A-Za-z0-9._-]{1,64}`. */
  client_name: string;
  client_version: string;
  account_id: string;
  need: McpNeed;
  room_label: string | null;
  message_preview: string | null;
  waiting: boolean;
  /** Tool calls waiting for this answer; the preview shows the newest. */
  waiting_calls: number;
}

export interface McpAssistantAccount {
  account_id: string;
  read: boolean;
  send: boolean;
  attachments: boolean;
  message_actions: boolean;
  /** Permissions denied for this account during this bridge run. */
  denied: McpNeed[];
}

export interface McpAssistant {
  client_name: string;
  client_version: string | null;
  last_used_ms: number | null;
  accounts: McpAssistantAccount[];
}

export type McpRegistrationClient =
  | "claude_code"
  | "codex"
  | "pi"
  | "claude_desktop"
  | "deepseek_harness"
  | "other";

export interface McpRegistration {
  client: McpRegistrationClient;
  client_name: string;
  kind: "command" | "yaml" | "json";
  text: string;
  config_hint: string | null;
  /** Besides copying: run the client's own CLI, or build a Claude Desktop extension. */
  action: "run_command" | "create_extension" | null;
}

/** Result of running a client's registration CLI (`register_mcp_client`). */
export type McpRegisterOutcome =
  | { outcome: "registered"; output: string }
  | { outcome: "not_found"; program: string }
  | { outcome: "failed"; exit_code: number | null; output: string }
  | { outcome: "timed_out" }
  | { outcome: "in_progress" }
  | { outcome: "unsafe_arguments" }
  | { outcome: "launcher_unavailable" }
  | { outcome: "unsupported" };

/** Result of `create_claude_desktop_extension`. */
export type McpExtensionOutcome =
  | { outcome: "opened"; path: string }
  | { outcome: "saved"; path: string }
  | { outcome: "unsupported" }
  | { outcome: "launcher_unavailable" }
  | { outcome: "write_failed" };

/** Process-wide state of the MCP consent broker, pushed on `koushi-desktop://mcp`. */
export interface McpAssistantsState {
  bridge: "disabled" | "running" | "start_failed";
  store: "ok" | "missing" | "malformed" | "unreadable" | "unsupported_schema" | "write_failed";
  launcher:
    | { state: "available"; path: string }
    | { state: "unavailable"; reason: "not_bundled" | "translocated" | "ephemeral" };
  pending: McpPendingApproval[];
  assistants: McpAssistant[];
  registrations: McpRegistration[];
  /** Client names whose launcher is from an older Koushi. */
  outdated_launchers: string[];
}

export type DesktopUpdateState =
  | { kind: "unsupported"; reason: "build" | "package_managed" }
  | { kind: "idle" }
  | { kind: "up_to_date"; version: string }
  | { kind: "checking" }
  | { kind: "available"; version: string; generation: number; notification_only: boolean; release_url: string | null; ignored: boolean; check_failed: boolean }
  | { kind: "downloading"; version: string }
  | { kind: "ready"; version: string }
  | { kind: "failed"; stage: "check" | "download_or_verify" | "install" }
  | { kind: "installing"; version: string };

export interface MediaSettings {
  image_upload_compression_policy: ImageUploadCompressionPolicy;
}

export interface TimelineSettings {
  thread_root_order: TimelineThreadRootOrder;
}

export type ImageUploadCompressionMode = "always" | "ask" | "never";

export interface ImageUploadCompressionPolicy {
  threshold_bytes: number;
  threshold_long_edge: number;
  target_long_edge: number;
  quality_percent: number;
}

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

export interface ComposerSelection {
  start: number;
  end: number;
}

export interface ComposerKeyEvent {
  key: ComposerKey;
  modifiers: ComposerKeyModifiers;
  is_composing: boolean;
  selection: ComposerSelection | null;
}

export interface ComposerResolverOptions {
  autocomplete_open: boolean;
  send_enabled: boolean;
}

export type ComposerResolvedAction =
  | "send"
  | "insertNewline"
  | "acceptAutocomplete"
  | "closeAutocomplete"
  | "cancel"
  | "commitImeCandidate"
  | "noop";

export type MentionTarget =
  | { kind: "user"; user_id: string; display_label: string }
  | { kind: "room"; room_id: string; display_label: string }
  | { kind: "roomMention"; display_label: string };

export type MentionSurface = "main" | "thread" | "edit";
export type MentionCandidatesCompleteness = "loading" | "partial" | "complete" | "failed";
export type RoomMentionPermission = "allowed" | "denied" | "unknown";
export type MentionCandidateMembership = "joined" | "invited";
export type MentionCandidatesFailureKind = "network" | "forbidden" | "sdk";

export interface MentionCandidate {
  user_id: string;
  display_label: string | null;
  original_display_label: string | null;
  avatar: AvatarImage | null;
  membership: MentionCandidateMembership;
}

export interface MentionCandidatesTarget {
  room_id: string;
  generation: number;
  request_id: number;
  query: string;
  surface: MentionSurface;
  completeness: MentionCandidatesCompleteness;
  candidates: MentionCandidate[];
  room_mention_allowed: RoomMentionPermission;
  failure_kind: MentionCandidatesFailureKind | null;
}

export interface MentionCandidatesState {
  targets: MentionCandidatesTarget[];
}

export interface MentionIntent {
  targets: MentionTarget[];
}

export type ComposerInline =
  | { kind: "text"; text: string }
  | { kind: "mention"; target: MentionTarget; display_label: string };

export interface ComposerDocument {
  version: 2;
  inlines: ComposerInline[];
}

export interface FormattedMessageDraft {
  plain_body: string;
  formatted_body: string | null;
  mentions: MentionIntent;
}

export type ResolveComposerKeyAction = (
  surface: ComposerSurface,
  keyEvent: ComposerKeyEvent,
  options: ComposerResolverOptions
) => Promise<ComposerResolvedAction>;

export type AuthDiscoveryState =
  | { kind: "unknown" }
  | { kind: "discovering"; homeserver: string }
  | { kind: "ready"; homeserver: string; flows: LoginFlow[]; delegated: DelegatedAuthLinks }
  | { kind: "failed"; homeserver: string; failureKind: AuthFailureKind };

export interface DelegatedAuthLinks {
  registration_url: string | null;
}

export interface CommandAdmission {
  protocolVersion: 1;
  admittedGeneration: number;
}

export interface CommandSettlement {
  protocolVersion: 1;
  publishedGeneration: number;
}

/** `create_room` settlement; a failed parent-Space link does not fail creation (#1007). */
export interface CreateRoomSettlement extends CommandSettlement {
  spaceLinkFailure: OperationFailureKind | null;
}

/**
 * `start_advisory_direct_message` settlement (#1494). `missing` means Core
 * found no reusable direct chat and created nothing; `draft` says whether the
 * opened chat now holds the advisory or kept the user's existing draft.
 */
export interface AdvisoryDirectMessageSettlement extends CommandSettlement {
  status: "opened" | "missing";
  draft: "seeded" | "kept_existing" | null;
}

export type CommandReceipt = CommandAdmission | CommandSettlement;

export interface OidcBrowserLaunchResponse {
  outcome: "launched" | "invalid_authorization_url" | "browser_launch_failed";
  settlement: CommandSettlement;
}

export interface LoginFlow {
  kind: "password" | "sso" | "oidc" | "token" | { unknown: string };
  delegated_oidc_compatibility: boolean;
  display_name: string | null;
}

export type AuthFailureKind =
  | "network"
  | "unsupported"
  | "cancelled"
  | "forbidden"
  | "rateLimited"
  | "invalidCredentials"
  | "store"
  | "timeout"
  | "sdk";

/** The user's bounded chosen method for a browser sign-in start (#1267). */
export type DelegatedAuthMethod = "oauth" | "sso";

export type AccountManagementState =
  | { kind: "idle" }
  | { kind: "working"; request_id: number; operation: AccountManagementOperation }
  | {
      kind: "awaitingUia";
      request_id: number;
      flow_id: number;
      operation: AccountManagementOperation;
    }
  | { kind: "succeeded"; request_id: number; operation: AccountManagementOperation }
  | {
      kind: "failed";
      request_id: number;
      operation: AccountManagementOperation;
      failureKind: AuthFailureKind;
    };

export type AccountManagementOperation =
  | "changePassword"
  | "deactivateAccount";

export interface AccountManagementCapabilities {
  change_password: CapabilityState;
  deactivate_account: CapabilityState;
}

// Mirrors koushi_state::state::account_notifications (#981).
export type NotificationCategory =
  | "directMessages"
  | "groupMessages"
  | "mentionsAndReplies"
  | "invites";

export type NotificationCategoryState = "on" | "off" | "mixed" | "unavailable";

export interface NotificationCategoryStates {
  direct_messages: NotificationCategoryState;
  group_messages: NotificationCategoryState;
  mentions_and_replies: NotificationCategoryState;
  invites: NotificationCategoryState;
}

export type NotificationEmailManagement =
  | "available"
  | "unsupported"
  | "delegatedToAccountManagement";

export interface NotificationEmailAddress {
  address: string;
  notifications_active: boolean;
}

/** One account `content` push rule shown by the keyword editor (#1427). */
export interface NotificationKeywordRule {
  rule_id: string;
  pattern: string;
  /**
   * Whether the account has this rule switched on. A rule another client turned
   * off is still on the account: it still refuses a same-pattern add and is
   * still removable here, so the list shows it and says it is off.
   */
  enabled: boolean;
}

export interface AccountNotificationsSnapshot {
  account_push_enabled: boolean;
  encrypted_event_push: boolean;
  categories: NotificationCategoryStates;
  email_management: NotificationEmailManagement;
  emails: NotificationEmailAddress[];
  unverified_email_pusher_count: number;
  /** The account's own `kind: content` rules, in the server's rule order. */
  keywords: NotificationKeywordRule[];
  /**
   * #1460: whether the homeserver is known not to send mail for an email pusher. Only
   * `knownUnavailable` is a claim; `unknown` is every other server.
   */
  server_email_delivery: ServerEmailDelivery;
}

export type ServerEmailDelivery = "unknown" | "knownUnavailable";

export type AccountNotificationsFailureKind =
  | "unsupported"
  | "emailInUse"
  | "emailDenied"
  | "invalidEmail"
  | "emailNotVerified"
  | "emailNotRegistered"
  | "authRejected"
  | "forbidden"
  | "rateLimited"
  | "network"
  | "server"
  | "sessionRequired"
  // #1427 keyword-rule validation, decided by Rust before the homeserver is
  // asked. `emptyPattern` is empty after trimming, `patternTooLong` exceeds the
  // character bound, `duplicate` matches an existing rule without regard to
  // case, `limitReached` means the account already holds the maximum number of
  // keyword rules, and `unknownRule` is a write that names a rule the account
  // does not have (a race with another client, not a condition the user made).
  | "emptyPattern"
  | "patternTooLong"
  | "duplicate"
  | "limitReached"
  | "unknownRule";

export type AccountNotificationsLoadState =
  | { kind: "notLoaded" }
  | { kind: "loading"; request_id: number }
  | { kind: "loaded" }
  | { kind: "failed"; request_id: number; failureKind: AccountNotificationsFailureKind };

export type AccountNotificationsOperation =
  | { kind: "setCategory"; category: NotificationCategory; enabled: boolean }
  | { kind: "setAccountPush"; enabled: boolean }
  | { kind: "requestEmailToken" }
  | { kind: "resendEmailToken" }
  | { kind: "confirmEmail" }
  | { kind: "enableEmailNotifications" }
  | { kind: "disableEmailNotifications" }
  | { kind: "addNotificationKeyword"; pattern: string }
  | { kind: "removeNotificationKeyword"; ruleId: string };

export type AccountNotificationsOperationState =
  | { kind: "idle" }
  | { kind: "working"; request_id: number; operation: AccountNotificationsOperation }
  | {
      kind: "awaitingUia";
      request_id: number;
      flow_id: number;
      operation: AccountNotificationsOperation;
    }
  | { kind: "succeeded"; request_id: number; operation: AccountNotificationsOperation }
  | {
      kind: "failed";
      request_id: number;
      operation: AccountNotificationsOperation;
      failureKind: AccountNotificationsFailureKind;
    };

export interface PendingNotificationEmail {
  address: string;
  resend_count: number;
}

export interface AccountNotificationsState {
  load: AccountNotificationsLoadState;
  snapshot: AccountNotificationsSnapshot | null;
  pending_email: PendingNotificationEmail | null;
  operation: AccountNotificationsOperationState;
}

// Mirrors koushi_state::state::sender_device_advisories (#1494): per open room,
// the senders who have devices they never confirmed. Rust classifies; React only
// renders the mark for `rooms[roomId]` members. No entry is ever empty.
export interface SenderDeviceAdvisoriesState {
  rooms: Record<string, string[]>;
}

// Mirrors koushi_state::state::contact_security (#1024). Two independent
// facts: the contact's devices confirmed (signed) by their owner, and your
// verification of the contact's identity. No device ids or key material.
export type ContactSecurityFailureKind = "sessionRequired" | "network" | "sdk";

export type ContactSecurityLoadState =
  | { kind: "idle" }
  | { kind: "loading"; request_id: number }
  | { kind: "loaded"; request_id: number }
  | { kind: "failed"; request_id: number; failureKind: ContactSecurityFailureKind };

export type ContactDevicesStatus =
  | "allOwnerSigned"
  | "someNotOwnerSigned"
  | "noDevices"
  | "ownerIdentityMissing";

export type ContactDeviceSignature =
  | "ownerSigned"
  | "notOwnerSigned"
  | "ownerSignatureInvalid"
  | "ownerIdentityMissing";

export type ContactIdentityVerification =
  | "verifiedByYou"
  | "notVerifiedByYou"
  | "changedAfterVerification"
  | "unknown";

export interface ContactDeviceCounts {
  total: number;
  owner_signed: number;
  not_owner_signed: number;
  owner_signature_invalid: number;
  excluded_dehydrated: number;
}

export type ContactVerificationDirectChat = "existingEncrypted" | "existingUnencrypted" | "new";

export type ContactVerificationOffer =
  | { kind: "notOffered" }
  | { kind: "requiresYourCrossSigning" }
  | { kind: "offered"; direct_chat: ContactVerificationDirectChat };

export interface ContactSecuritySummary {
  devices: ContactDevicesStatus;
  device_counts: ContactDeviceCounts;
  device_signatures: ContactDeviceSignature[];
  identity: ContactIdentityVerification;
  verification: ContactVerificationOffer;
}

export interface ContactSecurityState {
  user_id: string | null;
  load: ContactSecurityLoadState;
  summary: ContactSecuritySummary | null;
  /** Rust-derived: another verification flow is in progress, so Verify user
   * cannot start one until it settles. */
  verification_busy: boolean;
}

export type CapabilityState =
  | { kind: "unknown" }
  | { kind: "enabled" }
  | { kind: "disabled" };

export type QrLoginState =
  | { kind: "idle" }
  | { kind: "checkingCapability"; request_id: number }
  | { kind: "unavailable" }
  | { kind: "displaying"; request_id: number }
  | { kind: "scanning"; request_id: number }
  | { kind: "verified"; request_id: number }
  | { kind: "failed"; request_id: number; failureKind: AuthFailureKind };

export type SoftLogoutReauthState =
  | { kind: "idle" }
  | { kind: "authenticating"; request_id: number }
  | { kind: "succeeded"; request_id: number }
  | { kind: "failed"; request_id: number; failureKind: AuthFailureKind };

export type RecoveryMethod = "recoveryKey" | "securityPhrase";

export type SecureBackupSetupIntent =
  | { kind: "initialSetup" }
  | { kind: "reenable"; confirmed: boolean }
  | { kind: "resetRecoveryKey"; confirmed: boolean };

export type SecureBackupGateFailureKind =
  | "network"
  | "rateLimited"
  | "invalidRecoveryKey"
  | "backupKeyMismatch"
  | "secretStorageIncomplete"
  | "artifactDelivery"
  | "forbidden"
  | "timeout"
  | "sdk"
  | "serverResponse"
  | "unauthorized";

/** Mirror of the Rust `SecureBackupFailureStage` vocabulary. */
export type SecureBackupFailureStage =
  | "inspectServerTrust"
  | "roomKeyCounts"
  | "recoveryKeyDelivery"
  | "crossSigningStatus"
  | "inspectionDeadline"
  | "unknown";

/** Mirror of the Rust `SecureBackupFailureTransport` vocabulary. */
export type SecureBackupFailureTransport =
  | "noResponse"
  | "httpResponse"
  | "timeout"
  | "local";

/** Mirror of the Rust `SecureBackupMatrixErrorKind` allowlist. */
export type SecureBackupMatrixErrorKind =
  | "forbidden"
  | "unknownToken"
  | "missingToken"
  | "limitExceeded"
  | "unrecognized"
  | "badJson"
  | "notFound"
  | "unknown";

/** Bounded, privacy-safe Secure Backup failure facts (Rust-owned). */
export interface SecureBackupFailureDetail {
  stage: SecureBackupFailureStage;
  transport: SecureBackupFailureTransport;
  httpStatus?: number | null;
  matrixErrorKind?: SecureBackupMatrixErrorKind | null;
  retryable: boolean;
}

export type PendingKeyCountBucket =
  | "zero"
  | "one"
  | "two_to_ten"
  | "eleven_to_one_hundred"
  | "over_one_hundred"
  | "unknown";

export type SecureBackupGateState =
  | { kind: "inactive" }
  | { kind: "checking" }
  | {
      kind: "existingBackupNeedsRecovery";
      failure?: SecureBackupGateFailureKind | null;
    }
  | { kind: "secureStorageIncomplete" }
  | { kind: "setupRequired" }
  | { kind: "explicitlyDisabledRequiresSetup" }
  | { kind: "creatingBackup" }
  | { kind: "recoveryKeyDeliveryRequired" }
  | { kind: "uploadingExistingKeys"; pending: PendingKeyCountBucket }
  | {
      kind: "degradedRetrying";
      failure: SecureBackupGateFailureKind;
      detail?: SecureBackupFailureDetail | null;
    }
  | {
      kind: "blockedFailed";
      failure: SecureBackupGateFailureKind;
      detail?: SecureBackupFailureDetail | null;
    }
  | { kind: "ready" };

export type VerificationMethodCapability = "existingDeviceSas" | "recoveryKey" | "securityPhrase" | "bootstrap";
export type VerificationMethod = VerificationMethodCapability;
export type VerificationAccountKind = "existingIdentity" | "newIdentity" | "unknown";
export type VerificationGateFailureKind = "network" | "cancelled" | "mismatch" | "forbidden" | "timeout" | "sdk" | "noProofMethod";
export interface VerificationGateState { methods: VerificationMethodCapability[]; account_kind: VerificationAccountKind; failureKind?: VerificationGateFailureKind | null }
export type DeviceCleanupAuthMode = "legacy" | "oAuth" | "unknown";
export type DeviceCleanupOfferReason = "recoveryFailed" | "noProofMethod";
export type DeviceCleanupRemoteOutcome = "success" | "alreadyAbsent";
export type DeviceCleanupFailureKind = "network" | "forbidden" | "timeout" | "sdk" | "localData";
export type DeviceCleanupLocalMode =
  | { kind: "remoteRemoved"; outcome: DeviceCleanupRemoteOutcome }
  | { kind: "remoteMayRemain" };
export type DeviceCleanupState =
  | { kind: "idle" }
  | { kind: "offered"; reason: DeviceCleanupOfferReason }
  | { kind: "resolvingRemote"; request_id: number }
  | { kind: "removingRemote"; request_id: number; auth_mode: DeviceCleanupAuthMode }
  | { kind: "awaitingUia"; request_id: number; flow_id: number }
  | {
      kind: "remoteFailed";
      request_id: number;
      auth_mode: DeviceCleanupAuthMode;
      failureKind: DeviceCleanupFailureKind;
    }
  | { kind: "resettingLocal"; request_id: number; mode: DeviceCleanupLocalMode }
  | {
      kind: "localResetFailed";
      request_id: number;
      mode: DeviceCleanupLocalMode;
      failureKind: DeviceCleanupFailureKind;
    }
  | { kind: "erasingLocalAnyway"; request_id: number };
export type ProvisionalPhase =
  | "checkingTrust"
  | "discoveringMethods"
  | { recheckingTrust: { failureKind?: VerificationGateFailureKind | null } }
  | { kind: "checkingTrust" }
  | { kind: "discoveringMethods" }
  | { kind: "recheckingTrust"; failureKind?: VerificationGateFailureKind | null };

interface SessionStateFields {
  homeserver?: string;
  user_id?: string;
  device_id?: string;
  attempt_id?: { connection_id: number; sequence: number };
  phase?: ProvisionalPhase;
  gate?: VerificationGateState;
  method?: VerificationMethod;
  flow_id?: number;
  sas_emojis?: SasEmoji[];
  reason?: "existingIdentityWithoutProof" | "userRejected";
  recovery_methods?: RecoveryMethod[];
  failure?: "unsupported" | "unreachable" | "invalidResponse";
}

type ExactSessionState<
  K extends string,
  RequiredKeys extends keyof SessionStateFields = never,
  HasFailure extends boolean = false
> = {
  kind: K;
} & Required<Pick<SessionStateFields, RequiredKeys>> &
  Omit<SessionStateFields, RequiredKeys | "failure"> &
  (HasFailure extends true
    ? Required<Pick<SessionStateFields, "failure">>
    : { failure?: never });

type SessionAccountState<K extends string> = ExactSessionState<
  K,
  "homeserver" | "user_id" | "device_id"
>;

export type SessionLockReason = { kind: "unknownToken"; soft_logout: boolean };

export type SessionState =
  | ExactSessionState<"signedOut">
  | ExactSessionState<"restoring">
  | SessionAccountState<"switchingAccount">
  | ExactSessionState<"authenticating", "homeserver" | "attempt_id">
  | ExactSessionState<"needsRecovery", "user_id" | "recovery_methods">
  | ExactSessionState<"recovering">
  | ExactSessionState<
      "provisional",
      "homeserver" | "user_id" | "device_id" | "phase"
    >
  | ExactSessionState<
      "awaitingVerification",
      "homeserver" | "user_id" | "device_id" | "gate"
    >
  | ExactSessionState<
      "verifying",
      "homeserver" | "user_id" | "device_id" | "gate" | "method" | "flow_id" | "sas_emojis"
    >
  | ExactSessionState<
      "awaitingBootstrapConfirmation",
      "homeserver" | "user_id" | "device_id" | "gate" | "flow_id"
    >
  | ExactSessionState<"rejecting", "homeserver" | "user_id" | "device_id" | "reason">
  | SessionAccountState<"ready">
  | SessionAccountState<"locked">
  | ExactSessionState<
      "capabilityBlocked",
      "homeserver" | "user_id" | "device_id" | "failure",
      true
    >
  | ExactSessionState<"loggingOut">;

export type SessionStatusRefreshTrigger = "open" | "manual" | "scheduled" | "recovery";
export type SessionStatusRefreshCommandTrigger = Exclude<SessionStatusRefreshTrigger, "scheduled" | "recovery">;
export type SessionAuthenticationMethod = "password" | "sso" | "oauth" | "token" | "unknown";
export type CurrentSessionSyncState = "stopped" | "starting" | "running" | "error";
export type OwnIdentityVerification = "missing" | "unverified" | "verified";
export type CurrentSessionBackupState = "ready" | "disabled" | "unknown";
export type CurrentSessionVerification = "verified" | "unverified" | "unknown";
export type CurrentSessionStatusFailureKind =
  | "sdk"
  | "timed_out"
  | "unavailable"
  | "connectivity_unavailable"
  | "authentication"
  | "network"
  | "server";

export interface CurrentSessionStatusDetails {
  device_display_name: string | null;
  device_id: string;
  authentication_method: SessionAuthenticationMethod;
  sync_state: CurrentSessionSyncState;
  is_cross_signed_by_owner: boolean;
  own_identity_verification: OwnIdentityVerification;
  key_backup: CurrentSessionBackupState;
  verification: CurrentSessionVerification;
  checked_at_ms: number;
}

export type CurrentSessionStatusState =
  | { status: "idle" }
  | {
      status: "checking";
      request_id: number;
      trigger: SessionStatusRefreshTrigger;
      last_known_details: CurrentSessionStatusDetails | null;
      consecutive_failures: number;
    }
  | { status: "ready"; request_id: number; details: CurrentSessionStatusDetails }
  | {
      status: "failed";
      request_id: number;
      kind: CurrentSessionStatusFailureKind;
      checked_at_ms: number;
      last_known_details: CurrentSessionStatusDetails | null;
      consecutive_failures: number;
    };

export type SyncState =
  | "stopped"
  | "starting"
  | "running"
  | { failed: string }
  | { reconnecting: string };

// Rust RoomListFilter is #[serde(tag = "kind", rename_all = "camelCase")].
export type RoomListFilter =
  | { kind: "rooms" }
  | { kind: "unread" }
  | { kind: "people" }
  | { kind: "favourites" }
  | { kind: "invites" };

export type RoomListSource = "cache" | "live";

export type RoomListFailureKind = "connectivity" | "service" | "stopped";

export type RoomListReadiness =
  | { kind: "uninitialized" }
  | { kind: "loading"; source: RoomListSource; generation: number }
  | { kind: "ready"; source: RoomListSource; generation: number }
  | {
      kind: "failed";
      source: RoomListSource;
      generation: number;
      failureKind: RoomListFailureKind;
    };

export interface RoomListProjection {
  readiness: RoomListReadiness;
  active_filter: RoomListFilter;
  sort: RoomListSort;
  items: RoomListProjectionItem[] | null;
}

export interface RoomListProjectionItem {
  room_id: string;
  kind: RoomListEntryKind;
}

export type RoomListEntryKind = "room" | "invite";

export type RoomNotificationMode =
  | { kind: "all" }
  | { kind: "mentions" }
  | { kind: "mute" };

export type RoomNotificationModeOperation =
  | { kind: "idle" }
  | { kind: "pending"; request_id: number }
  | { kind: "failed"; request_id: number; failureKind: OperationFailureKind };

export interface RoomNotificationSettings {
  mode: RoomNotificationMode;
  operation: RoomNotificationModeOperation;
}

// #445: which conversation surface a Space was last showing. Every Space has a
// DMs surface and a Rooms surface, so a remembered conversation only means
// something together with its surface.
export type SpaceConversationSurface = "rooms" | "dms";

export interface SpaceNavigationSelection {
  surface: SpaceConversationSurface;
  room_id?: string | null;
}

export type HomeSelection =
  | { kind: "activity" }
  | { kind: "explore" }
  | { kind: "invites" }
  | { kind: "directMessage"; room_id: string };

export interface SpaceLocalPresentation {
  name?: string | null;
  icon?: string | null;
}

export type NavigationPreferenceUpdate =
  | { kind: "setHomeSelection"; selection: HomeSelection }
  | {
      kind: "setSpacePresentation";
      space_id: string;
      presentation: SpaceLocalPresentation | null;
    }
  | {
      kind: "importLegacy";
      home_selection?: HomeSelection | null;
      space_local_presentations: Record<string, SpaceLocalPresentation>;
    };

export type EventNavigationSource = "activity" | "search" | "pinned" | "notification";

export type EventNavigationFailureKind =
  | "targetMissing"
  | "roomUnavailable"
  | "sessionUnavailable"
  | "timeline";

export type EventNavigationState =
  | { kind: "idle" }
  | { kind: "opening"; generation: number; source: EventNavigationSource }
  | { kind: "anchored"; generation: number; source: EventNavigationSource }
  | { kind: "liveFallback"; generation: number; source: EventNavigationSource }
  | {
      kind: "failed";
      generation: number;
      source: EventNavigationSource;
      failureKind: EventNavigationFailureKind;
    };

export interface NavigationState {
  active_space_id: string | null;
  active_room_id: string | null;
  home_selection: HomeSelection;
  space_local_presentations: Record<string, SpaceLocalPresentation>;
  legacy_frontend_preferences_imported: boolean;
  space_order?: string[];
  /** Legacy non-DM-only memory, superseded by `last_selection_by_space_id`. */
  last_room_by_space_id?: Record<string, string>;
  last_selection_by_space_id?: Record<string, SpaceNavigationSelection>;
  room_scroll_anchors?: Record<string, TimelineScrollAnchor>;
  // #161: when set, the main pane renders the focused timeline anchored to this
  // event (jump-to-date), not the live timeline; the right panel is not opened.
  main_timeline_anchor?: MainTimelineAnchor | null;
  event_navigation: EventNavigationState;
}

export interface MainTimelineAnchor {
  event_id: string;
}

export interface TimelineScrollAnchor {
  event_id: string;
  edge?: "top" | "bottom";
  offset_px: number;
  updated_at_ms: number;
}

export interface ProfileState {
  own: OwnProfile;
  users: Record<string, UserProfile>;
  room_users: Record<string, Record<string, UserProfile>>;
  local_aliases: Record<string, string>;
  local_alias_update: LocalUserAliasUpdateState;
  ignored_user_ids: string[];
  ignored_user_update: IgnoredUserUpdateState;
  update: ProfileUpdateState;
}

export type IgnoredUserUpdateState =
  | { kind: "idle" }
  | { kind: "saving"; request_id: number };

export interface OwnProfile {
  display_name: string | null;
  avatar: AvatarImage | null;
}

export interface UserProfile {
  user_id: string;
  display_name: string | null;
  display_label: string;
  original_display_label: string;
  mention_search_terms: string[];
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
      source_ref: string;
      width: number | null;
      height: number | null;
      mime_type: string | null;
    }
  | { kind: "failed"; request_id: number; failureKind: AvatarThumbnailFailureKind };

export type AvatarThumbnailFailureKind =
  | "network"
  | "forbidden"
  | "unsupported"
  | "sdk"
  | "capacity";

export type ProfileUpdateState =
  | { kind: "idle" }
  | { kind: "settingDisplayName"; request_id: number; display_name: string | null }
  | { kind: "settingAvatar"; request_id: number; mime_type: string; byte_count: number };

export type LocalUserAliasUpdateState =
  | { kind: "idle" }
  | { kind: "saving"; request_id: number };

export interface SpaceSummary {
  space_id: string;
  /**
   * The Space's canonical `m.room.name`, or null when it has none. An alias or
   * a computed name is never promoted into it; `display_name` owns those.
   */
  raw_name: string | null;
  display_name: string;
  avatar: AvatarImage | null;
  /** The join rule as last synced; null until a room-list update projects it (#935). */
  join_rule: RoomJoinRule | null;
  child_room_ids: string[];
  /** The Space's own parent-side `m.space.child` rooms; add-existing eligibility uses this list, while `child_room_ids` is the display union (#1098). */
  parent_side_child_room_ids: string[];
}

/**
 * Issue #961: the viewer's relationship to a Space child room. "unknown" is the
 * honest answer for a child the server did not describe — a room the account
 * may not see is reported as such, never probed around.
 */
export type SpaceChildMembership =
  | "joined"
  | "invited"
  | "knocked"
  | "left"
  | "banned"
  | "not_joined"
  | "unknown";

export interface SpaceChildSummary {
  room_id: string;
  display_name: string;
  /** Set when `display_name` is the SDK's English calculated empty-room name (#1070). */
  display_name_placeholder?: RoomNamePlaceholder | null;
  avatar: AvatarImage | null;
  membership: SpaceChildMembership;
  /** Whether the join rule the server reported permits an attempt to join. */
  can_join: boolean;
  is_space: boolean;
  joined_members: number;
  /** Issue #1410: a Space admin suggests this child to members. */
  suggested?: boolean;
}

export type SpaceChildrenLoadState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "failed"; failure: OperationFailureKind };

/** The advertised children of the currently selected Space. */
export interface SpaceChildrenState {
  selected_space_id: string | null;
  generation: number;
  children: SpaceChildSummary[];
  load: SpaceChildrenLoadState;
  /** Issue #1410: whether this account may change the suggested flags. */
  can_edit_suggested?: boolean;
  suggested_edit?: SpaceChildSuggestedEditState;
  /** Issue #1410: the one-time offer; it outlives the Space selection. */
  suggested_offer?: SuggestedRoomsOffer;
}

/** Issue #1410: the admin toggle of one child's suggested flag. */
export type SpaceChildSuggestedEditState =
  | { kind: "idle" }
  | { kind: "saving"; request_id: number; child_room_id: string; suggested: boolean }
  | {
      kind: "failed";
      child_room_id: string;
      suggested: boolean;
      failure: OperationFailureKind;
    };

export interface SuggestedRoomCandidate {
  room_id: string;
  display_name: string;
  display_name_placeholder?: RoomNamePlaceholder | null;
}

export interface SuggestedRoomJoinResult {
  room_id: string;
  display_name: string;
  display_name_placeholder?: RoomNamePlaceholder | null;
  failure?: OperationFailureKind | null;
}

/** Issue #1410: the one-time offer to join a newly joined Space's suggested rooms. */
export type SuggestedRoomsOffer =
  | { kind: "none" }
  | { kind: "offered"; space_id: string; rooms: SuggestedRoomCandidate[] }
  | { kind: "joining"; request_id: number; space_id: string; rooms: SuggestedRoomCandidate[] }
  | { kind: "completed"; space_id: string; results: SuggestedRoomJoinResult[] };

export interface SpaceMembersState {
  selected_space_id: string | null;
  generation: number;
  space_joined: SpaceMemberEntry[];
  space_invited: SpaceMemberEntry[];
  child_room_only: SpaceMemberEntry[];
  child_room_count: number;
  complete_child_room_count: number;
  incomplete_child_room_count: number;
  power_levels_revision: string | null;
  can_edit_roles: boolean;
  operation: SpaceMembersOperationState;
}

export type SpaceInviteAvailabilityReason =
  | "available"
  | "settings_unavailable"
  | "permission_denied"
  | "operation_pending"
  | "invite_pending"
  | "already_in_space";

export type SpaceInviteCancellationAvailabilityReason =
  | "available"
  | "settings_unavailable"
  | "permission_denied"
  | "operation_pending"
  | "invite_unavailable";

export interface SpaceMemberEntry {
  user_id: string;
  display_name: string | null;
  display_label: string;
  original_display_label: string;
  avatar_url: string | null;
  power_level: number | null;
  role: RoomMemberRole;
  membership: SpaceMemberMembership;
  child_room_ids: string[];
  invite_pending: boolean;
  role_options: SpaceMemberRoleOption[];
}

export interface SpaceMemberRoleOption {
  power_level: number;
  role: RoomMemberRole;
  requires_confirmation: boolean;
}

export type SpaceMemberMembership =
  | "space_joined"
  | "space_invited"
  | "child_room_only";

export type SpaceMembersOperationState =
  | { kind: "idle" }
  | {
      kind: "loading";
      request_id: number | null;
      space_id: string;
      generation: number;
    }
  | {
      kind: "inviting";
      request_id: number;
      space_id: string;
      user_id: string;
      generation: number;
    }
  | {
      kind: "cancellingInvite";
      request_id: number;
      space_id: string;
      user_id: string;
      generation: number;
    }
  | {
      kind: "failed";
      request_id: number;
      space_id: string;
      user_id: string | null;
      generation: number;
      failureKind: OperationFailureKind;
    }
  | {
      kind: "updatingRole";
      request_id: number;
      space_id: string;
      user_id: string;
      generation: number;
      expected_power_levels_revision: string | null;
      expected_power_level: number;
      power_level: number;
      confirmed: boolean;
    }
  | {
      kind: "roleUpdateFailed";
      request_id: number;
      space_id: string;
      user_id: string;
      generation: number;
      expected_power_levels_revision: string | null;
      expected_power_level: number;
      power_level: number;
      sent_revision: string | null;
      failureKind: SpaceMemberRoleFailureKind;
    };

export type SpaceMemberRoleFailureKind =
  | "forbidden"
  | "stale"
  | "not_found"
  | "network"
  | "timeout"
  | "invalid"
  | "sdk";

export type SpaceMemberRoleUpdateOutcome =
  | "succeeded"
  | { failed: SpaceMemberRoleFailureKind };

export type RoomTagKind = "favourite" | "lowPriority";

export interface RoomTagInfo {
  order: string | null;
}

export interface RoomTags {
  favourite: RoomTagInfo | null;
  low_priority: RoomTagInfo | null;
}

/**
 * #1050: the SDK's calculated empty-room name, marked structurally by Rust so
 * product text comes from the message catalog. `previous_names` is caller data.
 */
export type RoomNamePlaceholder =
  | { kind: "empty" }
  | { kind: "emptyWas"; previous_names: string };

export interface RoomSummary {
  room_id: string;
  display_name: string;
  /** Set when `display_name` is the SDK's English empty-room name. */
  display_name_placeholder?: RoomNamePlaceholder | null;
  display_label: string;
  original_display_label: string;
  /** Set when `display_label` is that placeholder; render via `roomDisplayLabel`. */
  display_label_placeholder?: RoomNamePlaceholder | null;
  avatar: AvatarImage | null;
  is_dm: boolean;
  dm_user_ids: string[];
  tags: RoomTags;
  unread_count: number;
  notification_count?: number;
  highlight_count?: number;
  /** #1238: summed per-root SDK thread-cache unread replies; `unread_count` stays main-only navigation state. */
  thread_unread_count?: number;
  /** #1238: summed per-root SDK thread-cache unread mentions, so a thread mention renders mention styling. */
  thread_highlight_count?: number;
  marked_unread?: boolean;
  recency_stamp?: number | null;
  conversation_activity?: ConversationActivity | null;
  latest_event?: RoomLatestEventSummary | null;
  parent_space_ids: string[];
  dm_space_ids: string[];
  is_encrypted: boolean;
  joined_members?: number;
}

export type ConversationActivitySource = "message" | "encryptedMessage" | "threadReply";

export interface ConversationActivity {
  timestamp_ms: number;
  source: ConversationActivitySource;
}

export interface RoomLatestEventSummary {
  event_id: string;
  is_redacted: boolean;
  relation_type?: string | null;
  relation_event_id?: string | null;
  thread_root_event_id?: string | null;
  sender_id: string | null;
  sender_label: string | null;
  sender_avatar: AvatarImage | null;
  preview: string | null;
  timestamp_ms: number;
}

export interface InvitePreview {
  room_id: string;
  display_name: string;
  display_name_placeholder?: RoomNamePlaceholder | null;
  avatar: AvatarImage | null;
  topic: string | null;
  inviter_display_name: string | null;
  inviter_user_id: string | null;
  is_dm: boolean;
  is_space: boolean;
}

export interface InviteWorkflowState {
  query: InviteTargetQueryState;
  selected_targets: InviteSelectedTarget[];
  scope_plan: InviteScopePlan | null;
  selected_scope: InviteScopeSelection | null;
  history_policy: InviteHistoryPolicy | null;
  operation: InviteOperationState;
}

export type InviteHistoryReadiness = "ready" | "recoveryRequired";

export interface InviteHistoryPolicy {
  current_visibility: RoomHistoryVisibility;
  encrypted: boolean;
  can_edit: boolean;
  readiness: InviteHistoryReadiness;
}

export interface InviteTargetQueryState {
  room_id: string | null;
  query: string;
  candidates: InviteTargetCandidate[];
  explicit_user_id: InviteTargetCandidate | null;
}

export interface InviteTargetCandidate {
  user_id: string;
  display_label: string;
  original_display_label: string;
  avatar: AvatarImage | null;
  source: InviteTargetCandidateSource;
  status: InviteTargetCandidateStatus;
  status_message: string | null;
}

export type InviteTargetCandidateSource =
  | "profile"
  | "localAlias"
  | "roomMember"
  | "directMessage"
  | "matrixId";

export type InviteTargetCandidateStatus =
  | "selectable"
  | "alreadySelected"
  | "alreadyInDestination"
  | "invalidMatrixId";

export interface InviteSelectedTarget {
  user_id: string;
  display_label: string;
  avatar: AvatarImage | null;
}

export interface InviteScopePlan {
  room_id: string;
  destination_kind: InviteDestinationKind;
  default_scope: InviteScopeSelection;
  options: InviteScopeOption[];
}

export type InviteDestinationKind = "room" | "space";

export type InviteScopeSelection =
  | { kind: "roomOnly" }
  | { kind: "parentSpaceAndRoom"; space_id: string };

export interface InviteScopeOption {
  scope: InviteScopeSelection;
  label: string;
  detail: string | null;
}

export type InviteOperationState =
  | { kind: "idle" }
  | {
      kind: "pending";
      request_id: number;
      room_id: string;
      user_ids: string[];
      scope: InviteScopeSelection;
    }
  | {
      kind: "completed";
      request_id: number;
      room_id: string;
      results: InviteDestinationResult[];
      notice: string | null;
    }
  | {
      kind: "failed";
      request_id: number;
      room_id: string;
      failureKind: OperationFailureKind;
    };

export interface InviteDestinationResult {
  user_id: string;
  destination: InviteDestination;
  kind: InviteDestinationResultKind;
  message: string | null;
}

export type InviteDestination =
  | { kind: "room"; room_id: string }
  | { kind: "space"; space_id: string };

export type InviteDestinationResultKind = "invited" | "alreadyInSpace" | "failed";

export interface RoomInteractionState {
  pinned_events: PinnedEvent[];
  pin_operation: PinOperationState;
}

export interface PinnedEvent {
  event_id: string;
  sender: string | null;
  sender_label: string | null;
  body_preview: string | null;
  redacted: boolean;
  timestamp_ms?: number | null;
  state?: "ready" | "unableToDecrypt" | "unavailable";
  thread_root_event_id?: string | null;
}

export type PinOp = "pin" | "unpin";

export type PinOperationState =
  | { kind: "idle" }
  | { kind: "pending"; request_id: number; room_id: string; event_id: string; op: PinOp }
  | {
      kind: "failed";
      room_id: string;
      event_id: string;
      op: PinOp;
      recoverable: boolean;
    };

export interface ReplyQuote {
  event_id: string;
  sender: string | null;
  sender_label?: string | null;
  body_preview: string | null;
  formatted?: ReplyQuoteFormattedBody | null;
  permalink?: string | null;
  can_open?: boolean;
  state: ReplyQuoteState;
}

export interface ReplyQuoteCodeBlock {
  language: string | null;
  body: string;
}

export interface ReplyQuoteFormattedBody {
  html: string;
  plain_text: string;
  code_blocks: ReplyQuoteCodeBlock[];
}

export type ReplyQuoteState =
  | "loading"
  | "ready"
  | "redacted"
  | "missing"
  | "unsupported"
  | "failed";

export type OperationFailureKind =
  | "forbidden"
  | "notFound"
  | "network"
  | "timeout"
  | "invalid"
  | "sdk"
  /** The current join-rule content has allow conditions this client does not model (#1177). */
  | "unsupportedPolicyCondition"
  /** The current join-rule policy could not be read before the write (#1177). */
  | "policyNotVerified";

export interface MediaTransferProgress {
  current: number;
  total: number;
}

export type TimelineMediaDownloadState =
  | { kind: "notRequested" }
  | { kind: "pending"; progress: MediaTransferProgress | null }
  | {
      kind: "ready";
      source_ref: string;
      width: number | null;
      height: number | null;
      mime_type: string | null;
    }
  | { kind: "failed"; failure_kind: OperationFailureKind };

export type ActivityState =
  | { kind: "closed"; last_selected_tab?: ActivityTab }
  | { kind: "opening"; request_id: number; tab: ActivityTab }
  | {
      kind: "open";
      active_tab: ActivityTab;
      recent: ActivityStream;
      unread: ActivityStream;
      mark_read: ActivityMarkReadState;
    };

export type ActivityTab = "recent" | "unread";

export interface ActivityStream {
  rows: ActivityRow[];
  next_batch: string | null;
  resolution: ActivityResolutionState;
}

export type ActivityResolutionState =
  | { kind: "idle" }
  | { kind: "resolving"; generation: number; unresolved_room_count: number }
  | {
      kind: "failed";
      generation: number;
      unresolved_room_count: number;
      failure_kind: OperationFailureKind;
    };

export type ActivityRowKind = "event" | "roomUnread";

interface ActivityRowBase {
  room_id: string;
  room_label: string;
  timestamp_ms: number;
  unread: boolean;
  highlight: boolean;
  context_label: string;
}

export interface ActivityEventRow extends ActivityRowBase {
  kind: "event";
  event_id: string;
  thread_root_event_id: string | null;
  sender_id: string | null;
  sender_label: string | null;
  sender_avatar: AvatarImage | null;
  preview: string | null;
}

export interface ActivityRoomUnreadRow extends ActivityRowBase {
  kind: "roomUnread";
  event_id: null;
  thread_root_event_id: null;
  sender_id: null;
  sender_label: null;
  sender_avatar: null;
  preview: null;
}

export type ActivityRow = ActivityEventRow | ActivityRoomUnreadRow;

export type ActivityMarkReadState =
  | { kind: "idle" }
  | { kind: "pending"; request_id: number; target: ActivityMarkReadTarget }
  | {
      kind: "failed";
      target: ActivityMarkReadTarget;
      failure_kind: OperationFailureKind;
    };

export type ActivityMarkReadTarget =
  | { kind: "room"; room_id: string; up_to_event_id: string }
  | { kind: "all" };

export interface DirectoryState {
  query: DirectoryQueryState;
  preview: DirectoryPreviewState;
  join: DirectoryJoinState;
}

export type DirectoryQueryState =
  | { kind: "closed" }
  | { kind: "querying"; request_id: number; query: DirectoryQuery }
  | {
      kind: "results";
      request_id: number;
      query: DirectoryQuery;
      rooms: DirectoryRoomSummary[];
      next_batch: string | null;
    }
  | {
      kind: "failed";
      request_id: number;
      query: DirectoryQuery;
      failureKind: OperationFailureKind;
    };

/** What the user is shown before committing to a join. The target and via
 * servers are carried through every stage because the join that follows must
 * reuse exactly what resolved the preview. */
export type DirectoryPreviewState =
  | { kind: "closed" }
  | {
      kind: "loading";
      request_id: number;
      /** `#alias:server` or `!id:server`. */
      room_id_or_alias: string;
      via_servers: string[];
    }
  | {
      kind: "ready";
      request_id: number;
      room_id_or_alias: string;
      via_servers: string[];
      room: DirectoryRoomPreview;
    }
  | {
      kind: "failed";
      request_id: number;
      room_id_or_alias: string;
      via_servers: string[];
      failureKind: OperationFailureKind;
    };

/** Whether a plain join is expected to work. Restricted and knock variants
 * collapse into coarse buckets; the exact join rule is server policy. */
export type DirectoryPreviewJoinability = "open" | "inviteOnly" | "restricted" | "unknown";

/** Membership the current account already has in the previewed room. */
export type DirectoryPreviewMembership = "joined" | "invited" | "none";

export interface DirectoryRoomPreview {
  room_id: string;
  canonical_alias: string | null;
  /** Matrix `room_type`, e.g. `m.space`. Absent for an ordinary room. */
  room_type: string | null;
  /** Empty when the room has no name; the GUI supplies a type-appropriate
   * fallback rather than Rust inventing prose. */
  name: string;
  topic: string | null;
  joined_members: number;
  joinability: DirectoryPreviewJoinability;
  membership: DirectoryPreviewMembership;
}

export type DirectoryJoinState =
  | { kind: "idle" }
  | {
      kind: "joining";
      request_id: number;
      /** `#alias:server` or `!id:server`. */
      room_id_or_alias: string;
      /** Servers to try when the homeserver does not already know the room. */
      via_servers: string[];
    }
  | {
      kind: "failed";
      request_id: number;
      room_id_or_alias: string;
      via_servers: string[];
      failureKind: OperationFailureKind;
    };

export interface DirectoryQuery {
  term: string | null;
  server_name: string | null;
  limit: number | null;
  since: string | null;
}

export interface DirectoryRoomSummary {
  room_id: string;
  canonical_alias: string | null;
  /** Matrix `room_type`, e.g. `m.space`. Absent for an ordinary room. */
  room_type: string | null;
  /** Empty when the directory entry has no name; the GUI supplies a
   * type-appropriate fallback rather than Rust inventing prose. */
  name: string;
  topic: string | null;
  avatar_url: string | null;
  joined_members: number;
  world_readable: boolean;
  guest_can_join: boolean;
}

export type HistoryKeyRecoveryFailure = "network" | "verificationRequired" | "membershipChanged" | "historyNotShared" | "tooLarge" | "noRecipients" | "rateLimited" | "storage" | "invalidResponse";
export interface HistoryKeyRecoveryState {
  generation: number;
  status:
    | { kind: "idle" | "sending" }
    | { kind: "requested"; recipients: number; responses: number; received_keys: number }
    | { kind: "failed"; reason: HistoryKeyRecoveryFailure }
    | { kind: "partial"; received_keys: number; reason: HistoryKeyRecoveryFailure };
}

export interface RoomManagementState {
  history_keys?: Record<string, HistoryKeyRecoveryState>;
  selected_room_id: string | null;
  settings: RoomSettingsSnapshot | null;
  /**
   * The Rust-owned access/history draft (#1177). React keeps only DOM/focus
   * state; every rule, target and history selection lives here.
   */
  draft?: RoomAccessDraft | null;
  operation: RoomManagementOperationState;
}

/** The scope a Rust-owned access/history draft belongs to (#1177). */
export type RoomAccessDraftScope =
  | { kind: "room"; roomId: string }
  | { kind: "create"; sessionId: number };

export interface RoomAccessDraft {
  scope: RoomAccessDraftScope;
  revision: number;
  rule?: RoomJoinRule | null;
  /** The renderable subset of the selected allow targets (#1177): only joined
   * Spaces the picker offers. Unrenderable confirmed identities stay in Rust. */
  allowTargets?: string[];
  history?: RoomHistoryVisibility | null;
  /** Whether the user made an explicit choice; an untouched create draft keeps
   * the legacy preset path to Create. */
  touched?: boolean;
}

/** The create dialog facts that seed an opened create editor (#1177). */
export interface CreateRoomAccessSeed {
  visibility: CreateRoomVisibility;
  invitedOnly?: boolean;
  parentSpaceId?: string | null;
}

/** A typed mutation of the Rust-owned access/history draft (#1177). */
export type RoomAccessDraftCommand =
  | { kind: "rule"; scope: RoomAccessDraftScope; rule?: RoomJoinRule | null }
  | { kind: "allowTargets"; scope: RoomAccessDraftScope; allowTargets: string[] }
  | {
      kind: "toggleAllowTarget";
      scope: RoomAccessDraftScope;
      target: string;
      selected: boolean;
    }
  | { kind: "history"; scope: RoomAccessDraftScope; history?: RoomHistoryVisibility | null }
  | { kind: "reset"; scope: RoomAccessDraftScope }
  | { kind: "open"; scope: RoomAccessDraftScope; create?: CreateRoomAccessSeed };

/** Which property a Rust access preview describes (#1177). */
export type RoomAccessPreviewContext = "access" | "history";

/** One resolved outcome line: a catalog id plus ordered substitutions. */
export interface RoomAccessOutcomeLine {
  messageId: MessageId;
  substitutions?: string[];
}

export interface RoomAccessOutcome {
  join: RoomAccessOutcomeLine;
  joinRequest?: RoomAccessOutcomeLine;
  history: RoomAccessOutcomeLine;
  encryption: RoomAccessOutcomeLine;
  directory: RoomAccessOutcomeLine;
  historyKeyCaveat?: RoomAccessOutcomeLine;
  nonRetroactive: RoomAccessOutcomeLine;
}

/** A stateless Rust preview of one Room Info panel (#1177). */
export interface RoomAccessPreview {
  scope: RoomAccessDraftScope;
  context: RoomAccessPreviewContext;
  confirmed: boolean;
  outcome: RoomAccessOutcome;
  /** The canonical key of the effective access policy, Rust-owned (#1177). */
  canonicalPolicyKey?: string;
}

/** Why a create proposal cannot be submitted (#1177). */
export type CreateRoomAccessRejection =
  | "publicWithRestrictedAccess"
  | "explicitPolicyWithInvitedOnly"
  | "emptyAccessTargets"
  | "publicWithInvitedOnly";

/** Effective create inputs the Rust preview normalizes with Create's rules. */
export interface CreateRoomAccessPreviewInput {
  visibility: CreateRoomVisibility;
  invitedOnly: boolean;
  encrypted: boolean;
  parentSpaceId?: string | null;
}

export interface CreateRoomAccessPreview {
  scope: RoomAccessDraftScope;
  confirmed: boolean;
  outcome: RoomAccessOutcome;
  /** The effective join rule Create would submit (legacy preset included). */
  effectiveRule: RoomJoinRule | null;
  /** The effective history visibility Create would submit. */
  effectiveHistory: RoomHistoryVisibility;
  rejection?: CreateRoomAccessRejection;
  roomVersionPinned: boolean;
  /** Rust-owned availability (#1421): the choice is rendered only when true. */
  knockAvailable: boolean;
  knockRestrictedAvailable: boolean;
}

export type RoomManagementOperationState =
  | { kind: "idle" }
  | {
      kind: "pending";
      request_id: number;
      room_id: string;
      operation: RoomManagementOperationKind;
    }
  | {
      kind: "failed";
      request_id: number;
      room_id: string;
      operation: RoomManagementOperationKind;
      failureKind: OperationFailureKind;
    };

export type RoomManagementOperationKind = "settings" | "moderation" | "permissions" | "roles";

export interface RoomSettingsSnapshot {
  room_id: string;
  name: string | null;
  topic: string | null;
  avatar_url: string | null;
  canonical_alias?: string | null;
  alternate_aliases?: string[];
  share_link?: string | null;
  join_rule: RoomJoinRule;
  history_visibility: RoomHistoryVisibility;
  permissions: RoomPermissionFacts;
  members: RoomMemberSummary[];
}

export interface RoomMemberSummary {
  membership: "joined" | "invited" | "knocked" | "unknown";
  user_id: string;
  display_name: string | null;
  display_label: string;
  original_display_label: string;
  avatar_url: string | null;
  power_level: number | null;
  role: RoomMemberRole;
  role_options: RoomMemberRoleOption[];
  user_trust?: UserTrustState;
}

export interface RoomMemberRoleOption {
  power_level: number;
  role: RoomMemberRole;
  requires_confirmation: boolean;
}

export type RoomMemberRole = "creator" | "administrator" | "moderator" | "user";

export type UserTrustState =
  | { kind: "unverified" }
  | { kind: "verified" }
  | { kind: "identityReset" };

export type RoomJoinRule =
  | "public"
  | "invite"
  | "knock"
  | "restricted"
  | "knockRestricted"
  | "private"
  /** A rule the Rust core does not model; shown as-is, never sent back. */
  | "unknown";

export type RoomHistoryVisibility = "worldReadable" | "shared" | "invited" | "joined";

export interface RoomPermissionFacts {
  can_edit_settings: boolean;
  /** May send `m.room.join_rules`, which is all changing who can join needs (#935). */
  can_change_join_rule: boolean;
  can_change_topic: boolean;
  can_edit_roles: boolean;
  can_invite: boolean;
  can_kick: boolean;
  can_ban: boolean;
  can_unban: boolean;
}

export type RoomSettingChange =
  | { name: string | null }
  | { topic: string | null }
  | { avatarUrl: string | null }
  | { joinRule: RoomJoinRule }
  | { accessPolicy: RoomAccessPolicy }
  | { historyVisibility: RoomHistoryVisibility };

export type RoomModerationAction = "kick" | "ban" | "unban";

export interface TimelinePaneState {
  room_id: string | null;
  is_subscribed: boolean;
  is_paginating_backwards: boolean;
  composer: ComposerState;
  submission_registry: ComposerSubmissionRegistry;
  scheduled_send_capability: ScheduledSendCapability;
  scheduled_sends: ScheduledSendItem[];
  staged_uploads: StagedUploadItem[];
  media_gallery: TimelineMediaGalleryItem[];
  media_downloads: Record<string, TimelineMediaDownloadState>;
  continuity: TimelineContinuityState;
}

export type TimelineGapRepairFailureKind =
  | "network"
  | "timeout"
  | "sdk"
  | "cancelled"
  | "unsupportedAnchor";

export type TimelineContinuityState =
  | { kind: "unknown" }
  | { kind: "inspecting"; generation: number; known_gap_count: number }
  | { kind: "healthy"; generation: number; authoritative_start: boolean }
  | { kind: "incomplete"; generation: number; gap_count: number }
  | {
      kind: "repairing";
      generation: number;
      gap_count: number;
      batches_processed: number;
      minimum_batch_id: number | null;
    }
  | {
      kind: "failedIncomplete";
      generation: number;
      gap_count: number;
      batches_processed: number;
      failure_kind: TimelineGapRepairFailureKind;
    };

export interface ComposerSubmissionRegistry {
  accepted_submission_ids: string[];
  queued_submission_ids?: string[];
  settled_submission_ids: string[];
}

export type ScheduledSendCapability = "unknown" | "serverDelayedEvents" | "localFallback";

export interface ScheduledSendItem {
  scheduled_id: string;
  room_id: string;
  thread_root_event_id?: string | null;
  body: string;
  send_at_ms: number;
  handle: ScheduledSendHandle;
}

export type ScheduledSendHandle =
  | { kind: "local" }
  | { kind: "server"; delay_id: string };

export type StagedUploadKind =
  | { kind: "image"; width: number | null; height: number | null }
  | { kind: "file" };

export type StagedUploadCompressionChoice =
  | { kind: "notApplicable" }
  | { kind: "ask" }
  | { kind: "original" }
  | { kind: "compressed"; mode: ImageUploadCompressionMode };

export type ComposerTarget =
  | { kind: "main"; room_id: string }
  | { kind: "thread"; room_id: string; root_event_id: string };

export type PreparedUploadFormat = "original" | "png" | "jpeg" | "webp";

/** Linear scale chosen for the upload; both dimensions share the divisor. */
export type StagedUploadResizeChoice = "original" | "half" | "quarter" | "eighth";

/** Encoding chosen for the upload; `keep` preserves the source encoding. */
export type StagedUploadFormatChoice = "keep" | "png" | "jpeg" | "webp";

/** The two independent axes that identify one prepared output. */
export interface StagedUploadOutputSelection {
  resize: StagedUploadResizeChoice;
  format: StagedUploadFormatChoice;
}

export interface PreparedUploadVariant {
  variant_id: string;
  resize: StagedUploadResizeChoice;
  format_choice: StagedUploadFormatChoice;
  filename: string;
  mime_type: string;
  byte_count: number;
  width: number | null;
  height: number | null;
  format: PreparedUploadFormat;
  savings_percent: number;
  metadata_stripped: boolean;
  thumbnail_refreshed: boolean;
}

export type StagedUploadPreparation =
  | { kind: "preparing" }
  | {
      kind: "ready";
      /** Completed combinations, reused immediately when re-selected. */
      variants: PreparedUploadVariant[];
      /** The single owner of which output will be uploaded. */
      selected: StagedUploadOutputSelection;
      /** Set while `selected` has no prepared output yet. */
      pending?: StagedUploadOutputSelection | null;
      /** Fences stale encodes so the latest selection wins. */
      generation: number;
    }
  | {
      kind: "failed";
      failure_kind: "empty" | "unsupported" | "decode" | "encode" | "missingPreparedBytes";
      can_use_original: boolean;
    };

export interface StagedUploadItem {
  staged_id: string;
  room_id: string;
  position: number;
  filename: string;
  mime_type: string;
  byte_count: number;
  kind: StagedUploadKind;
  caption: ComposerDocument | null;
  compression_choice: StagedUploadCompressionChoice;
  preparation: StagedUploadPreparation;
}

export interface StageUploadBytesRequestItem {
  stagedId: string;
  position: number;
  filename: string;
  mimeType: string;
  bytes: number[];
}

export type TimelineMediaKind = "Image" | "File" | "Audio" | "Video";

export interface TimelineMediaGallerySource {
  mxc_uri: string;
  encrypted: boolean;
  encryption_version: string | null;
}

export interface TimelineMediaGalleryThumbnail {
  source: TimelineMediaGallerySource;
  mimetype: string | null;
  size: number | null;
  width: number | null;
  height: number | null;
}

export interface TimelineMediaGalleryMedia {
  kind: TimelineMediaKind;
  filename: string;
  source: TimelineMediaGallerySource;
  mimetype: string | null;
  size: number | null;
  width: number | null;
  height: number | null;
  thumbnail: TimelineMediaGalleryThumbnail | null;
}

export interface TimelineMediaGalleryItem {
  event_id: string;
  room_id: string;
  sender: string | null;
  sender_label?: string | null;
  timestamp_ms: number;
  media: TimelineMediaGalleryMedia;
}

export interface ComposerState {
  accepted_submission_ids: string[];
  pending_submission_id?: string | null;
  pending_transaction_id: string | null;
  draft: string;
  document: ComposerDocument;
  draft_revision: ComposerDraftRevision;
  last_accepted_clear_revision: ComposerDraftRevision;
  mode: ComposerMode;
}

export type ComposerDraftRevision = string & {
  readonly __composerDraftRevision: "ComposerDraftRevision";
};

export type SubmissionOutcome = "accepted" | { rejected: { kind: string } };

export interface SubmissionResponse {
  outcome: SubmissionOutcome;
  submissionId: string;
  transactionId: string | null;
  settlement: CommandSettlement;
}

/** Prepared-upload send settlement; the flag is the one case where Core consumed the
 * composer draft into the message caption (#1204). */
export interface PreparedUploadSendResponse extends CommandSettlement {
  consumedDraft: boolean;
}

export interface ComposerDraftAcceptanceResponse {
  acceptedRevision: ComposerDraftRevision;
  settlement: CommandSettlement;
}

// Rust ComposerMode has NO serde tag → externally tagged (like SyncState in this file)
export type ComposerMode =
  | "Plain"
  | { Reply: { in_reply_to_event_id: string } };

// Rust BasicOperationState is #[serde(tag = "kind", rename_all = "camelCase")]
// → internally tagged, camelCase VARIANT names, snake_case fields. Pending
// variants carry the correlation request_id (see docs/architecture/state-machine.md).
export type RoomAddressAvailability = "available" | "inUse" | "unknown";

export interface RoomAddressSuggestion {
  localpart: string;
  full_alias: string;
}

/** Advisory only: never a reservation; room creation is authoritative. */
export type RoomAddressAvailabilityState =
  | { kind: "idle" }
  | { kind: "checking"; request_id: number; full_alias: string }
  | {
      kind: "checked";
      request_id: number;
      full_alias: string;
      availability: RoomAddressAvailability;
      /** Offered only for an address in use; not checked yet. */
      suggestion: RoomAddressSuggestion | null;
    };

export type BasicOperationState =
  | { kind: "idle" }
  | { kind: "creatingRoom"; request_id: number; name: string }
  | { kind: "creatingSpace"; request_id: number; name: string }
  | { kind: "linkingSpaceChild"; request_id: number; space_id: string; child_room_id: string }
  /** #1518: a direct-message start with `user_id` is in flight. */
  | { kind: "startingDirectMessage"; request_id: number; user_id: string };

export interface LiveSignalsState {
  rooms: Record<string, RoomLiveSignals>;
  presence: Record<string, PresenceKind>;
}

export interface RoomLiveSignalMetadata {
  fully_read_event_id: string | null;
  typing_user_ids: string[];
  typing_users: LiveTypingUser[];
}

export interface RoomLiveSignals {
  receipts_by_event: Record<string, LiveEventReceiptSummary>;
  focused_receipts_by_event: Record<string, Record<string, LiveEventReceiptSummary>>;
  thread_receipts_by_event: Record<string, Record<string, LiveEventReceiptSummary>>;
  fully_read_event_id: string | null;
  typing_user_ids: string[];
  typing_users: LiveTypingUser[];
}

export interface LiveTypingUser {
  user_id: string;
  display_label: string | null;
}

export interface LiveReadReceipt {
  user_id: string;
  display_name: string | null;
  original_display_label: string;
  avatar: AvatarImage | null;
  timestamp_ms: number | null;
}

export interface LiveEventReceiptSummary {
  readers: LiveReadReceipt[];
  total_count: number;
  overflow_count: number;
}

export interface LiveEventReceipts {
  event_id: string;
  receipts: LiveReadReceipt[];
}

export type PresenceKind = "online" | "away" | "offline";

export interface E2eeTrustState {
  verification: VerificationFlowState;
  cross_signing: CrossSigningStatus;
  key_backup: KeyBackupStatus;
  identity_reset: IdentityResetState;
  key_management: E2eeKeyManagementState;
  /** The account's own sessions from the last own-session check (#1505). */
  devices: DeviceTrustSummary[];
  /** Pending popup naming unverified own sessions; null once dismissed. */
  unverified_own_sessions_alert: UnverifiedOwnSessionsAlert | null;
}

export interface UnverifiedOwnSessionsAlert {
  total_sessions: number;
  unverified_sessions: number;
}

export interface E2eeKeyManagementState {
  room_key_export: RoomKeyExportState;
  room_key_import: RoomKeyImportState;
  secure_backup_setup: SecureBackupSetupState;
  passphrase_change: SecureBackupPassphraseChangeState;
}

export type RoomKeyExportState =
  | { kind: "idle" }
  | { kind: "exporting"; request_id: number }
  | { kind: "exported"; request_id: number; exported_sessions: number | null }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type RoomKeyImportState =
  | { kind: "idle" }
  | { kind: "importing"; request_id: number }
  | { kind: "imported"; request_id: number; imported_count: number; total_count: number }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type SecureBackupSetupState =
  | { kind: "idle" }
  | { kind: "settingUp"; request_id: number }
  // #927: `recovery_key` is the live on-screen reveal. Render it only; never
  // copy it into React state, storage, logs, or diagnostics.
  | {
      kind: "recoveryKeyReady";
      request_id: number;
      recovery_key: string;
      delivery: RecoveryKeyDeliveryState;
      // Rust restored the reveal because the saved confirmation could not
      // be persisted; ask the user to confirm again.
      confirmation_failed: boolean;
    }
  | { kind: "enabled"; request_id: number }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

export type RecoveryKeyDeliveryState =
  | { kind: "notWritten" }
  | { kind: "written" }
  | { kind: "writeFailed" };

export type SecureBackupPassphraseChangeState =
  | { kind: "idle" }
  | { kind: "changing"; request_id: number }
  | {
      kind: "changed";
      request_id: number;
      recovery_key: string;
      delivery: RecoveryKeyDeliveryState;
    }
  | { kind: "failed"; request_id: number; failureKind: TrustOperationFailureKind };

/** Who sent a verification request (#1024). Only `them` can be accepted. */
export type VerificationInitiator = "us" | "them";

export type VerificationFlowState =
  | { kind: "idle" }
  | {
      kind: "requested";
      request_id: number;
      target: VerificationTarget;
      initiator: VerificationInitiator;
    }
  | {
      kind: "accepted";
      request_id: number;
      target: VerificationTarget;
      initiator: VerificationInitiator;
    }
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
  display_name: string | null;
  /** Milliseconds since the Unix epoch, as reported by the homeserver. */
  last_seen_ts: number | null;
  is_current: boolean;
}

export type DeviceTrustLevel = "unknown" | "unverified" | "verified" | "blocked";

export type TrustOperationFailureKind =
  | "cancelled"
  | "mismatch"
  | "invalidPassphrase"
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
  overlay_icon: NativeAttentionCapability;
  sound: NativeAttentionCapability;
  tray: NativeAttentionCapability;
  activation: NativeAttentionCapability;
}

export type NativeAttentionCapability = "available" | "unavailable" | "unknown";

export interface NativeAttentionDispatchId {
  connection_id: number;
  sequence: number;
}

export type NativeAttentionDispatchState =
  | { kind: "idle" }
  | { kind: "dispatching"; dispatch_id: NativeAttentionDispatchId }
  | { kind: "delivered"; dispatch_id: NativeAttentionDispatchId }
  | { kind: "unsupported"; dispatch_id: NativeAttentionDispatchId }
  | { kind: "suppressed"; reason: NativeAttentionSuppressionReason }
  | { kind: "failed"; dispatch_id: NativeAttentionDispatchId; failureKind: OperationFailureKind };

export type NativeAttentionSuppressionReason =
  | "initialSync"
  | "backfill"
  | "selfMessage"
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

export type ThreadOpenIntent =
  | "existingThread"
  | "newThreadDraft"
  | { pinnedReply: { event_id: string } };

export type ThreadPaneState =
  | { kind: "closed" }
  | {
      kind: "opening";
      room_id: string;
      root_event_id: string;
      intent: ThreadOpenIntent;
    }
  | {
      kind: "open";
      room_id: string;
      root_event_id: string;
      intent: ThreadOpenIntent;
      is_subscribed: boolean;
      composer: ComposerState;
      staged_uploads: StagedUploadItem[];
    }
  | {
      // #1473: the thread timeline could not be loaded; Retry or Close.
      kind: "failed";
      room_id: string;
      root_event_id: string;
      intent: ThreadOpenIntent;
      failure_kind: OperationFailureKind;
    };

export type ThreadAttentionState =
  | { kind: "closed" }
  | {
      kind: "tracking";
      room_id: string;
      root_event_id: string;
      notification_count: number;
      highlight_count: number;
      live_event_marker_count: number;
    };

export interface ThreadsListItem {
  room_id: string;
  root_event_id: string;
  root_sender: string;
  root_sender_label: string | null;
  root_body_preview: string | null;
  root_timestamp_ms: number | null;
  latest_event_id: string | null;
  latest_sender: string | null;
  latest_sender_label: string | null;
  latest_body_preview: string | null;
  latest_timestamp_ms: number | null;
  reply_count: number;
  /** #1259: unread replies from the SDK thread cache, the same value the chip shows. */
  unread_count: number;
}

export type ThreadsListScope =
  | { kind: "room"; room_id: string }
  | { kind: "home" }
  | { kind: "space"; space_id: string };

export type ThreadsListState =
  | { kind: "closed" }
  | { kind: "loading"; room_id: string; request_id: number }
  | {
      kind: "open";
      room_id: string;
      request_id: number;
      items: ThreadsListItem[];
      is_paginating: boolean;
      end_reached: boolean;
    }
  | {
      kind: "failed";
      room_id: string;
      request_id: number;
      failure_kind: OperationFailureKind;
      items: ThreadsListItem[];
    };

export type ScheduledSendsScope =
  | { kind: "home" }
  | { kind: "space"; space_id: string };

export type ScheduledSendsListState =
  | { kind: "closed" }
  | {
      kind: "open";
      scope: ScheduledSendsScope;
      capability: ScheduledSendCapability;
      items: ScheduledSendItem[];
    };

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
  | {
      kind: "tooShort";
      request_id: number;
      query: string;
      scope: SearchScopeKind;
      min_chars: number;
    }
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
  context_label?: string | null;
  sender: string;
  timestamp_ms: number;
  score_millis: number;
  snippet: string;
  match_field: "messageBody" | "attachmentFileName";
  highlights: TextRange[];
  match_kind: "exact";
}

export type AttachmentKind = "image" | "video" | "audio" | "file" | "sticker";

export type AttachmentScope =
  | { kind: "room"; room_id: string }
  | { kind: "space"; space_id: string; child_room_ids: string[] }
  | { kind: "account" };

export interface AttachmentFilter {
  kinds: AttachmentKind[];
  filename_query: string | null;
}

export type AttachmentSort =
  | "newestFirst"
  | "oldestFirst"
  | "sender"
  | "filename";

export interface AttachmentResult {
  room_id: string;
  event_id: string;
  sender: string;
  sender_label: string | null;
  timestamp_ms: number;
  kind: AttachmentKind;
  filename: string;
  mimetype: string | null;
  size: number | null;
  source_mxc: string;
  thumbnail_mxc: string | null;
  thread_root: string | null;
  encrypted: boolean;
  encryption_version: string | null;
  width: number | null;
  height: number | null;
  is_edited: boolean;
}

export type FilesViewScope =
  | { kind: "room"; room_id: string }
  | { kind: "space"; space_id: string }
  | { kind: "account" };

export type HistoryExportRange =
  | { kind: "allAvailable" }
  | { kind: "period"; start_ms: number; end_exclusive_ms: number; time_zone: string };

export type HistoryExportScope = { kind: "room"; room_id: string } | { kind: "space"; space_id: string };

export interface HistoryExportRoomCounts {
  fetched_events: number;
  exported_events: number;
  undecryptable_events: number;
  attachments_total: number;
  attachments_done: number;
  attachments_failed: number;
}

export type HistoryExportRoomPhase =
  | "pending"
  | "fetching"
  | "attachments"
  | "rendering"
  | "completed"
  | "skipped"
  | "failed";

export type HistoryExportRoomFailureKind = "network" | "sdk" | "write";

export interface HistoryExportRoom {
  room_id: string;
  display_name: string;
  phase: HistoryExportRoomPhase;
  counts: HistoryExportRoomCounts;
  skip_reason: "notJoined" | null;
  failure_kind: HistoryExportRoomFailureKind | null;
}

export type HistoryExportFailureKind =
  | "invalidRange"
  | "roomNotFound"
  | "spaceNotFound"
  | "destinationUnavailable"
  | "manifestMismatch"
  | "write"
  | "noSpace"
  | "network"
  | "sdk";

/** Rust-owned history export state; React renders it only. */
export type HistoryExportState =
  | { kind: "idle" }
  | {
      kind: "preparing";
      request_id: number;
      scope: HistoryExportScope;
      range: HistoryExportRange;
      stop_requested: boolean;
    }
  | {
      kind: "running";
      request_id: number;
      scope: HistoryExportScope;
      range: HistoryExportRange;
      rooms: HistoryExportRoom[];
      stop_requested: boolean;
    }
  | {
      kind: "completed" | "stopped";
      request_id: number;
      scope: HistoryExportScope;
      range: HistoryExportRange;
      rooms: HistoryExportRoom[];
    }
  | {
      kind: "failed";
      request_id: number;
      scope: HistoryExportScope;
      range: HistoryExportRange;
      rooms: HistoryExportRoom[];
      failure_kind: HistoryExportFailureKind;
    };

/** What the dialog asks the adapter to export. */
export type HistoryExportScopeInput = { kind: "room"; roomId: string } | { kind: "space"; spaceId: string };

/** The export range as the dialog shows it. The Tauri adapter resolves the
 * inclusive civil dates into instants in the named IANA time zone. */
export type HistoryExportRangeInput =
  | { kind: "allAvailable" }
  | { kind: "period"; startDate: string; endDate: string; timeZone: string };

/** Catalog-resolved page text Rust writes into exported pages
 * (`koushi_protocol::HistoryExportLabels`). */
export interface HistoryExportLabels {
  lang: string;
  edited: string;
  inReplyTo: string;
  replyUnavailable: string;
  threadReply: string;
  threadRootLink: string;
  redacted: string;
  undecryptable: string;
  notRetrieved: string;
  reactions: string;
  timesInZone: string;
  exportedAt: string;
  rangeAll: string;
  rangePeriod: string;
  roomsHeading: string;
  statusCompleted: string;
  statusSkipped: string;
  statusFailed: string;
  statusPending: string;
  eventsCount: string;
  attachmentsCount: string;
  failedAttachmentsCount: string;
  stateJoined: string;
  stateLeft: string;
  stateInvited: string;
  stateRemoved: string;
  stateBanned: string;
  stateRenamed: string;
  stateTopic: string;
  stateAvatar: string;
  stateOther: string;
}

/** `dismissed` means the native folder dialog was closed without a choice. */
export type HistoryExportStart =
  | { kind: "dismissed" }
  | { kind: "submitted"; requestId: number; admission: CommandAdmission };

export type FilesViewState =
  | { kind: "closed" }
  | {
      kind: "loading";
      request_id: number;
      scope: AttachmentScope;
      filter: AttachmentFilter;
      sort: AttachmentSort;
    }
  | {
      kind: "open";
      request_id: number;
      scope: AttachmentScope;
      filter: AttachmentFilter;
      sort: AttachmentSort;
      items: AttachmentResult[];
      selected_event_id: string | null;
    }
  | {
      kind: "failed";
      request_id: number;
      scope: AttachmentScope;
      filter: AttachmentFilter;
      sort: AttachmentSort;
      message: string;
    };

export interface TextRange {
  start_utf16: number;
  end_utf16: number;
}

export interface AppError {
  code: string;
  message: string;
  recoverable: boolean;
  /** Bounded, Rust-classified cause for visible guidance (#1268). */
  reason?: AuthFailureKind;
}

export interface SidebarModel {
  active_space_id: string | null;
  account_home: AccountHomeItem;
  space_rail: SpaceRailItem[];
  space_rooms: RoomListItem[];
  not_joined_space_rooms: RoomListItem[];
  global_dms: RoomListItem[];
  space_unread_count: number;
  dm_unread_count: number;
  space_highlight_count: number;
  dm_highlight_count: number;
  /** Effective values for the active Home/Space scope. */
  rooms_sort?: RoomListSort;
  dms_sort?: RoomListSort;
  rooms_collapsed?: boolean;
  dms_collapsed?: boolean;
  low_priority_collapsed?: boolean;
  sections: SidebarSections;
  /**
   * #1363/#1366: Rust-projected row bound for each visible section. The
   * renderer slices the matching `sections` list and shows the control; it
   * never computes the bound itself.
   */
  section_bounds?: SidebarSectionBounds | null;
  /** Rust-projected Add existing room rows for the active Space (#1007). */
  space_add_rooms?: SpaceAddRoomsModel | null;
  /** Rust-projected create-room dialog defaults for the active scope (#1023). */
  create_room_defaults?: CreateRoomDefaults;
}

export interface SpaceAddRoomsModel {
  space_id: string;
  candidates: SpaceAddRoomCandidate[];
}

export interface SpaceAddRoomCandidate {
  room_id: string;
  display_name: string;
  avatar: AvatarImage | null;
  status: SpaceAddRoomStatus;
}

export type SpaceAddRoomStatus =
  | { kind: "available" }
  | { kind: "adding" }
  | { kind: "added" }
  | { kind: "failed"; reason: OperationFailureKind };

/**
 * `rooms`, `people`, and `low_priority` are the mutually exclusive visible
 * sections projected by Rust. `favourites` is a derived subset of `rooms`.
 */
export interface SidebarSections {
  favourites: RoomListItem[];
  rooms: RoomListItem[];
  people: RoomListItem[];
  low_priority: RoomListItem[];
  not_joined: RoomListItem[];
}

/**
 * #1363/#1366: Rust-projected bound for one visible sidebar section.
 *
 * `expanded` is the effective bound state: `expanded_by_active_room` marks the
 * case where the open conversation sits past the bound, so the bound cannot
 * hide it. `shows_expand_control` is false while that override applies, because
 * collapsing the section could not change what is visible.
 */
export interface SidebarSectionBound {
  visible_limit: number;
  total_count: number;
  visible_count: number;
  hidden_count: number;
  expanded: boolean;
  expanded_by_active_room: boolean;
  shows_expand_control: boolean;
}

export interface SidebarSectionBounds {
  rooms?: SidebarSectionBound | null;
  dms?: SidebarSectionBound | null;
  low_priority?: SidebarSectionBound | null;
}

export interface AccountHomeItem {
  display_name: string;
  /** Unread messages only; invites are counted separately (#330). */
  unread_count: number;
  highlight_count: number;
  /** Invites pending for the account, not silenced by room notification settings. */
  invite_count: number;
  /** What the Home rail badge renders: unread_count + invite_count, summed in Rust. */
  attention_count: number;
  is_active: boolean;
}

export interface SpaceRailItem {
  space_id: string;
  display_name: string;
  local_icon?: string | null;
  avatar: AvatarImage | null;
  unread_count: number;
  highlight_count: number;
  is_active: boolean;
  /** The Space's own access condition (#1166); absent means not yet projected. */
  access_join_rule?: RoomJoinRule | null;
  /**
   * Restricted-rule allow-condition facts; absent when the rule is not
   * restricted or has not been projected.
   */
  access_restricted_conditions?: RestrictedConditions | null;
  /** Resolved labels of the rooms/Spaces a restricted rule names (#1166). */
  access_allowed_room_names?: string[];
  /**
   * The verified single-Space membership route (#1220): present only when the
   * rule names exactly one distinct target whose create event proves it is a
   * Space and whose name resolves. This is the tooltip substitution for the
   * specific sentence.
   */
  access_space_members_route?: string | null;
  /**
   * Joined, non-DM rooms this Space shows, which a Space leave may take with
   * it. Rust projection; the `leave_space` command re-admits against it.
   */
  leave_candidates: SpaceLeaveCandidate[];
}

export interface SpaceLeaveCandidate {
  room_id: string;
  display_name: string;
  /** Mirrors `RoomSummary.display_label_placeholder`; render via `roomListItemLabel`. */
  display_name_placeholder?: RoomNamePlaceholder | null;
  avatar: AvatarImage | null;
  /** Another joined Space also shows this room. */
  in_other_space: boolean;
}

/**
 * What the client could determine about a `restricted`/`knock_restricted`
 * rule's allow conditions (#1220). Mirrors the Rust `RestrictedConditions`:
 * an allow-rule type the client does not model is never reported as a confirmed
 * empty set, and an unavailable rule is never reported as inspected.
 */
export type RestrictedConditions =
  | "notInspected"
  | "confirmedEmpty"
  | "membershipOnly"
  | "membershipPlusUnsupported"
  | "unsupportedOnly";

export interface RoomListItem {
  room_id: string;
  /**
   * Issue #961: the viewer's relationship to this room. Rows the joined room
   * list produces are always "joined"; the not-joined lane is the only source
   * of anything else.
   */
  membership?: SpaceChildMembership;
  /** Whether this row offers a join. Always false for a joined room. */
  can_join?: boolean;
  /** Issue #1410: a suggested not-joined Space child. Never true for a joined room. */
  suggested?: boolean;
  /**
   * The room's own authoritative access condition (#1166), from its
   * `m.room.join_rules`. Absent means not yet projected, never a guessed rule;
   * independent of encryption, DM status, membership and `can_join`.
   */
  access_join_rule?: RoomJoinRule | null;
  /**
   * Restricted-rule allow-condition facts; absent when the rule is not
   * restricted or has not been projected. Never inferred here.
   */
  access_restricted_conditions?: RestrictedConditions | null;
  /** Resolved labels of the rooms/Spaces a restricted rule names (#1166). */
  access_allowed_room_names?: string[];
  /**
   * The verified single-Space membership route (#1220), or absent for every
   * other restricted shape.
   */
  access_space_members_route?: string | null;
  display_name: string;
  /** Mirrors `RoomSummary.display_label_placeholder`; render via `roomListItemLabel`. */
  display_name_placeholder?: RoomNamePlaceholder | null;
  avatar: AvatarImage | null;
  tags: RoomTags;
  unread_count: number;
  highlight_count: number;
  /** Rust-owned notification count selected for the row's display mode. */
  notification_count?: number;
  /** Verified numeric unread badge; manual markers remain nonnumeric. */
  display_count?: number;
  /** Rust-owned parts of the numeric badge; their sum is `display_count`. */
  main_unread_count?: number;
  thread_unread_count?: number;
  has_unread_content?: boolean;
  is_attention_highlighted?: boolean;
  has_unread_mention?: boolean;
  is_muted?: boolean;
}

export interface TimelineMessage {
  room_id: string;
  event_id: string;
  sender: string;
  timestamp_ms: number;
  body: string;
  attachment_filename: string | null;
  reply_count: number;
  thread_root?: string | null;
  link_previews?: LinkPreview[];
  link_ranges?: { url: string; start_utf16: number; end_utf16: number }[];
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

export interface RoomListSections {
  favourites: RoomListItem[];
  invites: RoomListItem[];
  notJoined: RoomListItem[];
  rooms: RoomListItem[];
  people: RoomListItem[];
  lowPriority: RoomListItem[];
}

// Search history crawler — #77

export type SearchCrawlerSpeed = "standard" | "fast" | "slow" | "paused";

export interface SearchCrawlerSettings {
  speed: SearchCrawlerSpeed;
  include_media_captions: boolean;
  include_filenames: boolean;
}

/** Coarse failure kind — never carries raw SDK error text. */
export type SearchCrawlerFailureKind =
  | "roomNotFound"
  | "sdk"
  | "decryption"
  | "indexUnavailable";

export type SearchCrawlerRoomState =
  | { kind: "idle" }
  | { kind: "queued" }
  | { kind: "running"; processed: number; indexed: number }
  | { kind: "completed"; indexed: number }
  | { kind: "failed"; failureKind: SearchCrawlerFailureKind };

export type SearchCrawlerLastActiveStatus = "queued" | "running" | "completed" | "failed";

export interface SearchCrawlerLastActive {
  room_id: string;
  updated_at_ms: number;
  status: SearchCrawlerLastActiveStatus;
  processed: number;
  indexed: number;
}

export interface SearchCrawlerState {
  rooms: Record<string, SearchCrawlerRoomState>;
  last_active: SearchCrawlerLastActive | null;
}
