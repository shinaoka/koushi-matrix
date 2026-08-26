import {
  type FormEvent,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState
} from "react";
// App.tsx is the Tauri integration host. The @tauri-apps imports below
// are acknowledged in-progress transport wiring tracked for Phase 2 migration
// to backend/client.ts (#87). Each line has its own disable directive so the
// rule still catches any NEW @tauri-apps import added without a comment.
// eslint-disable-next-line no-restricted-imports
import { listen } from "@tauri-apps/api/event";
// eslint-disable-next-line no-restricted-imports
import { getCurrentWindow } from "@tauri-apps/api/window";
// eslint-disable-next-line no-restricted-imports
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { api } from "./backend/appRuntime";
import {
  CORE_EVENT_NAME,
  isTauriRuntime,
  tauriTimelineTransport
} from "./backend/tauriTimelineTransport";
import {
  classifySubmissionFailure,
  createComposerSubmissionControllerRegistry,
  mainSubmissionTarget,
  threadSubmissionTarget,
  type ComposerSubmissionControllerRegistry
} from "./domain/composerSubmission";
import {
  COMPOSER_DRAFT_REVISION_ZERO,
  compareComposerDraftRevisions
} from "./domain/composerDraftRevision";
import {
  createComposerDraftLifecycleRegistry,
  type ComposerDraftLifecycleRegistry,
  type ComposerDraftOperationCapture,
  type ComposerDraftScope
} from "./domain/composerDraftLifecycle";
import type { ComposerDocument, ComposerDraftRevision, TimelinePaneState } from "./domain/types";
import {
  documentFromText,
  plainBodyFromDocument
} from "./domain/composerDocument";
import type { MatrixPermalinkTarget } from "./domain/matrixPermalink";
import { resolveDirectorySubmission } from "./domain/directorySubmission";
import { serverNameFromMatrixId } from "./domain/matrixPermalink";
import { setActiveLocaleProfile, t } from "./i18n/messages";

export function reconcileComposerSubmissionSnapshot(
  registry: ComposerSubmissionControllerRegistry,
  timeline: TimelinePaneState
): void {
  registry.reconcile(
    timeline.submission_registry.accepted_submission_ids,
    timeline.submission_registry.settled_submission_ids
  );
}

import { ContextMenuSurface } from "./components/ContextMenuSurface";
import {
  SessionVerificationGate,
  secureBackupFailureLabel,
  secureBackupGateFailure
} from "./components/SessionVerificationGate";
import {
  type TimelineDiagnosticLogEntry,
  type TimelineDiagnostics,
  type TimelineTransport,
  roomLatestDisplayEventId
} from "./components/TimelineView";
import {
  type CoreEventPayload,
  type TimelineKey,
  focusedTimelineKey,
  isUnsupportedSlashCommandRejection,
  noticeMatchesMainComposer,
  noticeMatchesThreadComposer,
  roomTimelineKey,
  threadTimelineKey
} from "./domain/coreEvents";
import {
  applyGlobalResync,
  applyRoomKeyRequestStateChanged,
  applyTimelineEventWithProjectionResultAndRetention,
  createTimelineStore,
  pruneTimelineStore,
  timelineStoreInitialItemsDiagnosticMessage,
  threadTimelineStoreDiagnosticMessage,
  timelineStoreKeyId,
  type TimelineStoreState
} from "./domain/timelineStore";
import { TimelineStoreContext } from "./components/timelineStoreContext";
import {
  type ContextMenuActionId,
  type ContextMenuItem
} from "./domain/contextMenus";
import {
  shortcutActionFromMenuPayload,
  shortcutIdForKeyboardEvent
} from "./domain/shortcuts";
import {
  effectiveRightPanelModeForSnapshot,
  type PeoplePanelScope,
  type RightPanelContextMenuTarget,
  type RightPanelMode,
  rightPanelIntentForContextMenuAction,
  rightPanelModeForSearchQuery
} from "./domain/rightPanel";
import {
  desktopAttentionSummary,
  desktopAttentionWindowTitle
} from "./domain/desktopAttention";
import {
  qaDomDiagnosticTokens,
  qaTimelineDiagnosticTokens,
  qaWindowTitle,
  type QaTimelineDiagnostics
} from "./domain/qaTitle";
import {
  INITIAL_TIMELINE_DIAGNOSTICS,
  qaRenderedDomDiagnostics,
  qaSecurityDiagnostics,
  timelineDiagnosticsEqual,
  timelineDiagnosticsLogMessage
} from "./app/qaDiagnostics";
import { useDesktopAttentionEffects } from "./app/useDesktopAttentionEffects";
import { useUiLatencyDiagnostics } from "./app/useUiLatencyDiagnostics";
import {
  createDiagnosticLogBuffer,
  diagnosticReport,
  schemaMismatchDiagnosticEntry,
  type DiagnosticLogSnapshot
} from "./domain/diagnostics";
import { e2eeSendDiagnosticMessage } from "./domain/e2eeSendDiagnostics";
import {
  type QaSendSmokeStatus,
  qaSendCompletionStatusFromCoreEvent,
  qaSendSmokeCanStart,
  qaSendSmokeCompletionStatus,
  qaSendSmokeMessageFromEnv,
  qaSendSmokeTargetDiagnosticTokens,
  qaSendSmokeTargetRoom,
  qaSendSmokeTargetUserIdFromEnv
} from "./domain/qaSendSmoke";
import {
  AVATAR_THUMBNAIL_DOWNLOADS_ENABLED,
  planSnapshotAvatarThumbnailRequests,
  requestAvatarThumbnailWithDedupe
} from "./domain/avatarThumbnails";
import type {
  ActivityMarkReadTarget,
  ActivityTab,
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  ComposerTarget,
  CreateRoomRequest,
  DesktopSnapshot,
  DirectoryRoomSummary,
  FilesViewScope,
  InviteScopeSelection,
  InviteWorkflowState,
  StagedUploadOutputSelection,
  ResolveComposerKeyAction,
  RoomModerationAction,
  RoomNotificationMode,
  RoomSettingChange,
  SavedSessionInfo,
  SearchScopeKind,
  SettingsPatch,
  SpaceMemberRoleOption,
  ThreadOpenIntent,
  ThreadsListScope,
  PinnedEventNavigation
} from "./domain/types";
import { stageAttachmentFiles } from "./domain/attachmentIngestion";
import { createLatestMutationOperationQueue } from "./domain/latestAsyncResult";
import { createOrderedEventBatcher } from "./domain/orderedEventBatcher";
import { SNAPSHOT_SCHEMA_VERSION } from "./domain/types";
import {
  type DisplayDensity,
  type SpaceLocalOverrides,
  readDisplayDensity,
  readSpaceLocalOverrides,
  setSpaceLocalOverride,
  spaceDisplayName,
  SPACE_OVERRIDES_CHANGED_EVENT,
  writeDisplayDensity
} from "./app/localPresentation";
import { createViewportSyncReporter } from "./app/viewportSyncReporter";
import {
  applyAppStoreDeltas,
  getAppStoreDeltaStats,
  selectSnapshot,
  setAppStoreSnapshot,
  useAppStore
} from "./domain/appStore";
import { getRecentJsErrors } from "./domain/jsErrorLog";
import { getTimelineTransportStats } from "./domain/timelineTransportStats";
import { openExternalHttpUrl } from "./domain/externalLinks";

import {
  composerModeProp,
  serverNameFromRoomId,
  syncStatePresentation,
  type ActiveContextMenu,
  type ContextMenuTarget,
  type PrimaryView,
} from "./app/uiShared";
import {
  ActivityPane,
  ExplorePane,
  InvitesPane,
  TimelinePane
} from "./components/panes";
import { AuthScreen, SlidingSyncCapabilityBlockedScreen } from "./components/auth";
import {
  CreateEntityDialog,
  type CreateRoomDialogOptions,
  DiagnosticDialog,
  DirectoryPreviewDialog,
  InviteTargetsDialog,
  ReportReasonDialog,
  ResetLocalDataConfirmationDialog,
  uploadStagingItemsAreSendable,
  UserIdDialog
} from "./components/dialogs";
import {
  TopBar,
  WorkspaceRail,
  Sidebar,
  type RuntimeAlert
} from "./components/Shell";
import { ContextualRightPanel } from "./components/rightPanel";
import type {
  SpaceInviteAvailabilityReason,
  SpaceInviteCancellationAvailabilityReason
} from "./components/SpaceMembersPanel";

type ActivityOpenTrigger = "home_rail" | "activity_sidebar" | "initial_home" | "other";
type SpaceMembersOpenTrigger = "sidebar" | "space_info";
type SpaceMemberInviteTrigger = "inline" | "context" | "search";
type SpaceMemberCancelTrigger = "inline";
type SpaceMemberFence = { spaceId: string; generation: number };

function exactRoomSettingsForRoom(
  snapshot: Pick<DesktopSnapshot, "state"> | null,
  roomId: string
) {
  if (!snapshot) {
    return null;
  }
  const roomManagement = snapshot.state.domain.room_management;
  return roomManagement.selected_room_id === roomId &&
    roomManagement.settings?.room_id === roomId
    ? roomManagement.settings
    : null;
}

function spaceMembersFenceForSnapshot(
  snapshot: Pick<DesktopSnapshot, "state"> | null
): SpaceMemberFence | null {
  const spaceId = snapshot?.state.ui.navigation.active_space_id;
  const members = snapshot?.state.domain.space_members;
  if (!spaceId || !members || members.selected_space_id !== spaceId) {
    return null;
  }
  return { spaceId, generation: members.generation };
}

function spaceMembersSnapshotMatches(
  snapshot: Pick<DesktopSnapshot, "state"> | null,
  fence: SpaceMemberFence
): boolean {
  return (
    snapshot?.state.ui.navigation.active_space_id === fence.spaceId &&
    snapshot.state.domain.space_members.selected_space_id === fence.spaceId &&
    snapshot.state.domain.space_members.generation === fence.generation
  );
}

function spaceInviteAvailabilityReasonForSnapshot(
  snapshot: Pick<DesktopSnapshot, "state"> | null,
  spaceId: string
): SpaceInviteAvailabilityReason {
  if (
    snapshot?.state.ui.navigation.active_space_id !== spaceId ||
    snapshot.state.domain.space_members.selected_space_id !== spaceId
  ) {
    return "settings_unavailable";
  }
  const settings = exactRoomSettingsForRoom(snapshot, spaceId);
  if (!settings) {
    return "settings_unavailable";
  }
  if (!settings.permissions.can_invite) {
    return "permission_denied";
  }
  const operation = snapshot.state.domain.space_members.operation.kind;
  return operation === "loading" || operation === "inviting" || operation === "cancellingInvite"
    ? "operation_pending"
    : "available";
}

function spaceInviteCancellationAvailabilityReasonForSnapshot(
  snapshot: Pick<DesktopSnapshot, "state"> | null,
  spaceId: string
): SpaceInviteCancellationAvailabilityReason {
  if (
    snapshot?.state.ui.navigation.active_space_id !== spaceId ||
    snapshot.state.domain.space_members.selected_space_id !== spaceId
  ) {
    return "settings_unavailable";
  }
  const settings = exactRoomSettingsForRoom(snapshot, spaceId);
  if (!settings) {
    return "settings_unavailable";
  }
  if (!settings.permissions.can_kick) {
    return "permission_denied";
  }
  const operation = snapshot.state.domain.space_members.operation.kind;
  return operation === "loading" || operation === "inviting" || operation === "cancellingInvite"
    ? "operation_pending"
    : "available";
}

const DEFAULT_HOMESERVER = "https://matrix.org";
const MENU_EVENT_NAME = "koushi-desktop://menu";
const STATE_EVENT_NAME = "koushi-desktop://state";
const STATE_EVENT_REFRESH_DEBOUNCE_MS = 250;
declare global {
  interface Window {
    __matrixDesktopQaErrorCaptureInstalled?: boolean;
    __matrixDesktopQaLastError?: string;
  }
}

if (
  typeof window !== "undefined" &&
  import.meta.env.VITE_KOUSHI_QA_TITLE === "1" &&
  !window.__matrixDesktopQaErrorCaptureInstalled
) {
  window.__matrixDesktopQaErrorCaptureInstalled = true;
  window.addEventListener("error", (event) => {
    window.__matrixDesktopQaLastError = event.message;
  });
  window.addEventListener("unhandledrejection", (event) => {
    window.__matrixDesktopQaLastError =
      event.reason instanceof Error ? event.reason.message : String(event.reason);
  });
}

type ReportDialogState =
  | { kind: "user"; userId: string }
  | { kind: "content"; roomId: string; eventId: string }
  | { kind: "room"; roomId: string };
/**
 * Right-panel modes whose content belongs to the selected room (#373). Leaving
 * that room makes them stale, so they close once the Rust snapshot confirms the
 * room is gone.
 */
const ROOM_BOUND_RIGHT_PANEL_MODES = new Set<RightPanelMode>([
  "thread",
  "threads",
  "focusedContext",
  "search",
  "files",
  "pinned",
  "people",
  "profile",
  "roomInfo"
]);

const DEFAULT_CREATE_ROOM_OPTIONS: CreateRoomDialogOptions = {
  aliasLocalpart: "",
  encrypted: true,
  topic: "",
  visibility: "private"
};
const DEFAULT_SIDEBAR_WIDTH = 318;
const MIN_SIDEBAR_WIDTH = 260;
const MAX_SIDEBAR_WIDTH = 440;
const DEFAULT_RIGHT_PANEL_WIDTH = 390;
const MIN_RIGHT_PANEL_WIDTH = 320;
const MAX_RIGHT_PANEL_WIDTH = 680;
const COMPACT_RAIL_WIDTH = 56;
const MIN_TIMELINE_WIDTH_WHILE_RESIZING = 180;
const HOME_SELECTION_KEY = "koushi.homeSelection.v1";
type HomeSelection =
  | { kind: "activity" }
  | { kind: "explore" }
  | { kind: "invites" }
  | { kind: "dm"; roomId: string };
const DEFAULT_HOME_SELECTION: HomeSelection = { kind: "activity" };

function readHomeSelection(): HomeSelection {
  if (typeof window === "undefined" || !("localStorage" in window)) {
    return DEFAULT_HOME_SELECTION;
  }
  try {
    const parsed = JSON.parse(window.localStorage.getItem(HOME_SELECTION_KEY) ?? "");
    if (!parsed || typeof parsed !== "object" || !("kind" in parsed)) {
      return DEFAULT_HOME_SELECTION;
    }
    if (
      parsed.kind === "activity" ||
      parsed.kind === "explore" ||
      parsed.kind === "invites"
    ) {
      return { kind: parsed.kind };
    }
    if (parsed.kind === "dm" && typeof parsed.roomId === "string") {
      return { kind: "dm", roomId: parsed.roomId };
    }
  } catch {
    return DEFAULT_HOME_SELECTION;
  }
  return DEFAULT_HOME_SELECTION;
}

function writeHomeSelection(selection: HomeSelection): void {
  if (typeof window === "undefined" || !("localStorage" in window)) {
    return;
  }
  window.localStorage.setItem(HOME_SELECTION_KEY, JSON.stringify(selection));
}

function defaultCreateRoomDialogOptions(): CreateRoomDialogOptions {
  return { ...DEFAULT_CREATE_ROOM_OPTIONS };
}

function createRoomRequestFromDraft(
  name: string,
  options: CreateRoomDialogOptions,
  activeSpaceId: string | null
): CreateRoomRequest {
  const visibility = options.visibility;
  const parentViaServer = activeSpaceId ? serverNameFromRoomId(activeSpaceId) : null;
  return {
    name,
    topic: options.topic.trim() || null,
    aliasLocalpart: visibility === "public" ? options.aliasLocalpart.trim() || null : null,
    encrypted: visibility === "private" ? options.encrypted : false,
    visibility,
    parentSpace:
      activeSpaceId && parentViaServer
        ? {
            spaceId: activeSpaceId,
            viaServer: parentViaServer
          }
        : null
  };
}

function clampSidebarWidth(width: number, viewportWidth = window.innerWidth): number {
  const responsiveMax =
    viewportWidth <= 760
      ? Math.max(
          MIN_SIDEBAR_WIDTH,
          Math.min(
            MAX_SIDEBAR_WIDTH,
            viewportWidth - COMPACT_RAIL_WIDTH - MIN_TIMELINE_WIDTH_WHILE_RESIZING
          )
        )
      : MAX_SIDEBAR_WIDTH;
  return Math.min(responsiveMax, Math.max(MIN_SIDEBAR_WIDTH, Math.round(width)));
}
function clampRightPanelWidth(
  width: number,
  sidebarWidth: number,
  viewportWidth = window.innerWidth
): number {
  const responsiveMax =
    viewportWidth <= 760
      ? MIN_RIGHT_PANEL_WIDTH
      : Math.max(
          MIN_RIGHT_PANEL_WIDTH,
          Math.min(
            MAX_RIGHT_PANEL_WIDTH,
            viewportWidth -
              COMPACT_RAIL_WIDTH -
              sidebarWidth -
              MIN_TIMELINE_WIDTH_WHILE_RESIZING
          )
        );
  return Math.min(responsiveMax, Math.max(MIN_RIGHT_PANEL_WIDTH, Math.round(width)));
}
type InviteUserDialogState = {
  roomId: string;
  title: string;
} | null;

const DEFAULT_INVITE_SCOPE: InviteScopeSelection = { kind: "roomOnly" };
const DEFAULT_INVITE_WORKFLOW: InviteWorkflowState = {
  query: {
    room_id: null,
    query: "",
    candidates: [],
    explicit_user_id: null
  },
  selected_targets: [],
  scope_plan: null,
  selected_scope: null,
  history_policy: null,
  operation: { kind: "idle" }
};

function inviteScopeFromWorkflow(workflow: InviteWorkflowState): InviteScopeSelection {
  return workflow.selected_scope ?? workflow.scope_plan?.default_scope ?? DEFAULT_INVITE_SCOPE;
}

function threadsListScopeFromKey(key: string): ThreadsListScope {
  if (key === "home") {
    return { kind: "home" };
  }
  if (key.startsWith("space:")) {
    return { kind: "space", space_id: key.slice("space:".length) };
  }
  return { kind: "room", room_id: key };
}

function createStagedUploadId(index: number): string {
  const random =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  return `staged-upload-${index}-${random}`;
}

function rightPanelTargetFromContextMenuTarget(
  target: ContextMenuTarget
): RightPanelContextMenuTarget {
  if (target.kind === "message") {
    return {
      kind: "message",
      roomId: target.message.room_id,
      eventId: target.message.event_id
    };
  }
  return target;
}

function initialSearchQuery(): string {
  return new URLSearchParams(window.location.search).get("q") ?? "";
}

function correlatedSearchState(
  search: DesktopSnapshot["state"]["domain"]["search"],
  query: string,
  scope: SearchScopeKind
): DesktopSnapshot["state"]["domain"]["search"] | null {
  if (search.kind === "closed" || !query.trim()) {
    return null;
  }
  return search.query === query.trim() && search.scope === scope ? search : null;
}

function searchCrawlerHasPendingIndexing(
  crawler: DesktopSnapshot["state"]["domain"]["search_crawler"]
): boolean {
  return Object.values(crawler.rooms).some(
    (room) => room.kind === "queued" || room.kind === "running"
  );
}

function qaTitleEnabled(): boolean {
  return import.meta.env.VITE_KOUSHI_QA_TITLE === "1";
}

function qaSendSmokeMessage(): string | null {
  return qaSendSmokeMessageFromEnv(import.meta.env.VITE_KOUSHI_QA_SEND_SMOKE_MESSAGE);
}

function qaSendSmokeTargetUserId(): string | null {
  return qaSendSmokeTargetUserIdFromEnv(
    import.meta.env.VITE_KOUSHI_QA_SEND_SMOKE_USER_ID
  );
}

function timelineStoreSessionKey(snapshot: DesktopSnapshot | null): string {
  const session = snapshot?.state.domain.session;
  if (!session || session.kind !== "ready" || !session.user_id) {
    return "signed-out";
  }
  return [
    session.homeserver ?? "",
    session.user_id,
    session.device_id ?? ""
  ].join("\u0000");
}

export function retainedTimelineStoreKeyIds(snapshot: DesktopSnapshot | null): Set<string> {
  const userId =
    snapshot?.state.domain.session.kind === "ready"
      ? snapshot.state.domain.session.user_id ?? null
      : null;
  if (!snapshot || !userId) {
    return new Set();
  }

  const retained = new Set<string>();
  const roomId = snapshot.state.ui.timeline.room_id;
  if (roomId) {
    retained.add(timelineStoreKeyId(roomTimelineKey(userId, roomId)));
    const mainTimelineAnchorEventId =
      snapshot.state.ui.navigation.main_timeline_anchor?.event_id ?? null;
    if (mainTimelineAnchorEventId) {
      retained.add(
        timelineStoreKeyId(
          focusedTimelineKey(userId, roomId, mainTimelineAnchorEventId)
        )
      );
    }
  }

  const focusedContext = snapshot.state.ui.focused_context;
  if (focusedContext.kind === "opening" || focusedContext.kind === "open") {
    retained.add(
      timelineStoreKeyId(
        focusedTimelineKey(userId, focusedContext.room_id, focusedContext.event_id)
      )
    );
  }

  const thread = snapshot.state.ui.thread;
  if (
    (thread.kind === "opening" || thread.kind === "open") &&
    thread.room_id &&
    thread.root_event_id
  ) {
    retained.add(
      timelineStoreKeyId(threadTimelineKey(userId, thread.room_id, thread.root_event_id))
    );
  }

  return retained;
}

function currentSessionStatusFailureLabel(kind: "sdk" | "timed_out" | "unavailable"): string {
  switch (kind) {
    case "sdk":
      return t("sessionStatus.failureSdk");
    case "timed_out":
      return t("sessionStatus.failureTimedOut");
    case "unavailable":
      return t("sessionStatus.failureUnavailable");
  }
}

export async function settleLoginTransport(
  login: Promise<DesktopSnapshot>,
  refresh: () => Promise<DesktopSnapshot>,
  apply: (snapshot: DesktopSnapshot) => void
): Promise<string | null> {
  try {
    apply(await login);
    return null;
  } catch {
    try {
      const snapshot = await refresh();
      apply(snapshot);
      const hasProjectedLoginFailure = snapshot.state.ui.errors.some((error) =>
        error.code === "login_failed" || error.code === "sync_auth_required"
      );
      return hasProjectedLoginFailure || snapshot.state.domain.session.gate?.failureKind || snapshot.state.domain.session.kind === "rejecting"
        ? null
        : "Sign-in failed. Please try again.";
    } catch {
      return "Sign-in failed. Please try again.";
    }
  }
}

function composerDraftAccountOwnerKey(account: {
  homeserver: string;
  userId: string;
  deviceId: string;
}): string {
  return JSON.stringify([account.homeserver, account.userId, account.deviceId]);
}

function readyComposerDraftAccountOwner(snapshot: DesktopSnapshot | null): {
  homeserver: string;
  userId: string;
  deviceId: string;
} | null {
  const session = snapshot?.state.domain.session;
  return session?.kind === "ready" &&
    session.homeserver &&
    session.user_id &&
    session.device_id
    ? {
        homeserver: session.homeserver,
        userId: session.user_id,
        deviceId: session.device_id
      }
    : null;
}

function composerDraftScope(
  account: { homeserver: string; userId: string; deviceId: string },
  target: ComposerTarget
): ComposerDraftScope {
  return {
    account: {
      homeserver: account.homeserver,
      user_id: account.userId,
      device_id: account.deviceId
    },
    target
  };
}

function composerDraftScopesEqual(
  left: ComposerDraftScope,
  right: ComposerDraftScope
): boolean {
  const targetsEqual =
    left.target.kind === right.target.kind &&
    left.target.room_id === right.target.room_id &&
    (left.target.kind === "main" ||
      (right.target.kind === "thread" &&
        left.target.root_event_id === right.target.root_event_id));
  return (
    left.account.homeserver === right.account.homeserver &&
    left.account.user_id === right.account.user_id &&
    left.account.device_id === right.account.device_id &&
    targetsEqual
  );
}

function composerDraftApiAccount(scope: ComposerDraftScope): {
  homeserver: string;
  userId: string;
  deviceId: string;
} {
  return {
    homeserver: scope.account.homeserver,
    userId: scope.account.user_id,
    deviceId: scope.account.device_id
  };
}

export function App() {
  const snapshot = useAppStore(selectSnapshot);
  const snapshotRef = useRef(snapshot);
  const secureBackupShellAccountRef = useRef<string | null>(null);
  const secureBackupShellExposedRef = useRef(false);
  const diagnosticLogBufferRef = useRef<ReturnType<typeof createDiagnosticLogBuffer> | null>(null);
  const diagnosticLogBuffer =
    diagnosticLogBufferRef.current ?? (diagnosticLogBufferRef.current = createDiagnosticLogBuffer());
  snapshotRef.current = snapshot;
  const initialAccount = readyComposerDraftAccountOwner(snapshot);
  const secureBackupShellAccount = initialAccount
    ? composerDraftAccountOwnerKey(initialAccount)
    : null;
  if (secureBackupShellAccountRef.current !== secureBackupShellAccount) {
    secureBackupShellAccountRef.current = secureBackupShellAccount;
    secureBackupShellExposedRef.current = false;
  }
  const secureBackupGateIsOperational =
    snapshot?.state.domain.secure_backup_gate.kind === "ready" ||
    snapshot?.state.domain.secure_backup_gate.kind === "uploadingExistingKeys" ||
    snapshot?.state.domain.secure_backup_gate.kind === "degradedRetrying";
  if (secureBackupShellAccount !== null && secureBackupGateIsOperational) {
    secureBackupShellExposedRef.current = true;
  }
  const submissionAccountOwnerRef = useRef<string | null>(
    initialAccount ? composerDraftAccountOwnerKey(initialAccount) : null
  );
  const composerDraftLifecycleOwnerRef = useRef<string | null>(
    submissionAccountOwnerRef.current
  );
  const [schemaMismatchVersion, setSchemaMismatchVersion] = useState<number | null>(null);
  // #87 Phase 4 IPC contract guard (fail-closed at the data boundary): every snapshot enters
  // render state through this setter, so we reject one whose schema_version does not match the
  // renderer's SNAPSHOT_SCHEMA_VERSION — a stale flat (v1) snapshot or a mismatched Rust/TS
  // build. Such a snapshot may be missing the `domain`/`ui` sections entirely, so it must never
  // reach the render body's `snapshot.state.domain|ui.*` reads (which would throw before any
  // render gate could run); instead it records the offending version, which drives an explicit
  // recovery screen below. A later compatible snapshot clears the mismatch, so the guard is
  // self-healing rather than latching the app into the recovery screen.
  const setSnapshot = useCallback((next: DesktopSnapshot | null) => {
    if (next && next.state.schema_version !== SNAPSHOT_SCHEMA_VERSION) {
      diagnosticLogBuffer.append(schemaMismatchDiagnosticEntry(Date.now()));
      setSchemaMismatchVersion(next.state.schema_version ?? -1);
      return;
    }
    const account = readyComposerDraftAccountOwner(next);
    submissionAccountOwnerRef.current = account
      ? composerDraftAccountOwnerKey(account)
      : null;
    setSchemaMismatchVersion(null);
    setAppStoreSnapshot(next);
  }, [diagnosticLogBuffer]);
  const latestTextMutationQueueRef = useRef(createLatestMutationOperationQueue<string>());

  async function applyLatestTextMutationSnapshot(
    key: string,
    operation: () => Promise<DesktopSnapshot>
  ): Promise<void> {
    const result = await latestTextMutationQueueRef.current.run(key, operation);
    if (result.kind === "applied") {
      setSnapshot(result.value);
    }
  }
  const [searchQuery, setSearchQuery] = useState(() => initialSearchQuery());
  const [searchScope, setSearchScope] = useState<SearchScopeKind>("currentRoom");
  // #373: the room/DM leave confirmation. React owns only dialog visibility and
  // the in-flight guard; membership and the resulting room list stay Rust-owned.
  const [pendingRoomLeave, setPendingRoomLeave] = useState<{
    roomId: string;
    isDm: boolean;
  } | null>(null);
  const [roomLeaveInFlight, setRoomLeaveInFlight] = useState(false);
  const mainComposerOverlayRef = useRef<{
    scope: ComposerDraftScope;
    document: ComposerDocument;
    revision: ComposerDraftRevision | null;
    debounceHandle: number | null;
  } | null>(null);
  const threadComposerOverlayRef = useRef<{
    scope: ComposerDraftScope;
    document: ComposerDocument;
    revision: ComposerDraftRevision | null;
    debounceHandle: number | null;
  } | null>(null);
  const composerDraftLifecycleRegistryRef = useRef<ComposerDraftLifecycleRegistry | null>(null);
  if (composerDraftLifecycleRegistryRef.current === null) {
    composerDraftLifecycleRegistryRef.current = createComposerDraftLifecycleRegistry({
      begin: () => api.beginComposerDraftRendererGeneration(),
      acquire: (scope, rendererGeneration) =>
        api.acquireComposerDraftLease(scope, rendererGeneration),
      release: (lease) =>
        api.releaseComposerDraftLease(lease.leaseId, lease.rendererGeneration)
    });
  }
  const submissionRegistryRef = useRef<ComposerSubmissionControllerRegistry | null>(null);
  if (submissionRegistryRef.current === null) {
    submissionRegistryRef.current = createComposerSubmissionControllerRegistry();
  }

  function retireComposerRendererGeneration(): void {
    const mainOverlay = mainComposerOverlayRef.current;
    if (mainOverlay?.debounceHandle !== null && mainOverlay) {
      window.clearTimeout(mainOverlay.debounceHandle);
      composerDraftLifecycleRegistryRef.current!.clearDebounce(mainOverlay.scope);
    }
    const threadOverlay = threadComposerOverlayRef.current;
    if (threadOverlay?.debounceHandle !== null && threadOverlay) {
      window.clearTimeout(threadOverlay.debounceHandle);
      composerDraftLifecycleRegistryRef.current!.clearDebounce(threadOverlay.scope);
    }
    composerDraftLifecycleRegistryRef.current!.revokeRendererGeneration();
    submissionRegistryRef.current?.reset();
    mainComposerOverlayRef.current = null;
    threadComposerOverlayRef.current = null;
  }

  useEffect(() => {
    const account = readyComposerDraftAccountOwner(snapshot);
    const owner = account ? composerDraftAccountOwnerKey(account) : null;
    const ownerChanged = composerDraftLifecycleOwnerRef.current !== owner;
    if (ownerChanged) {
      retireComposerRendererGeneration();
    }
    submissionAccountOwnerRef.current = owner;
    composerDraftLifecycleOwnerRef.current = owner;
  }, [
    snapshot?.state.domain.session.homeserver,
    snapshot?.state.domain.session.user_id,
    snapshot?.state.domain.session.device_id,
    snapshot?.state.domain.session.kind
  ]);
  useEffect(() => {
    const account = readyComposerDraftAccountOwner(snapshot);
    const timeline = snapshot?.state.ui.timeline;
    if (account && timeline?.room_id) {
      const scope = composerDraftScope(account, {
        kind: "main",
        room_id: timeline.room_id
      });
      composerDraftLifecycleRegistryRef.current!.observe(
        scope,
        timeline.composer.draft_revision,
        timeline.composer.last_accepted_clear_revision,
        timeline.composer.draft.length > 0
      );
      void composerDraftLifecycleRegistryRef.current!
        .activate(scope)
        .then(() => {
          const overlay = mainComposerOverlayRef.current;
          if (overlay && overlay.revision === null && composerDraftScopesEqual(overlay.scope, scope)) {
            overlay.revision = composerDraftLifecycleRegistryRef.current!.nextDraft(scope);
            composerDraftLifecycleRegistryRef.current!.setActiveOverlay(
              scope,
              overlay.document,
              overlay.revision
            );
            queueComposerDraftPersist(scope, overlay.document, overlay.revision);
          }
        })
        .catch(() => undefined);
    }
    const thread = snapshot?.state.ui.thread;
    if (
      account &&
      thread?.kind === "open" &&
      thread.room_id &&
      thread.root_event_id &&
      thread.composer
    ) {
      const scope = composerDraftScope(account, {
          kind: "thread",
          room_id: thread.room_id,
          root_event_id: thread.root_event_id
      });
      composerDraftLifecycleRegistryRef.current!.observe(
        scope,
        thread.composer.draft_revision,
        thread.composer.last_accepted_clear_revision,
        thread.composer.draft.length > 0
      );
      void composerDraftLifecycleRegistryRef.current!
        .activate(scope)
        .then(() => {
          const overlay = threadComposerOverlayRef.current;
          if (overlay && overlay.revision === null && composerDraftScopesEqual(overlay.scope, scope)) {
            overlay.revision = composerDraftLifecycleRegistryRef.current!.nextDraft(scope);
            composerDraftLifecycleRegistryRef.current!.setActiveOverlay(
              scope,
              overlay.document,
              overlay.revision
            );
            queueThreadComposerDraftPersist(scope, overlay.document, overlay.revision);
          }
        })
        .catch(() => undefined);
    }
  }, [snapshot]);
  const [loginHomeserver, setLoginHomeserver] = useState(DEFAULT_HOMESERVER);
  const [loginUsername, setLoginUsername] = useState("");
  const [loginDeviceName, setLoginDeviceName] = useState("");
  const [loginPasswordFilled, setLoginPasswordFilled] = useState(false);
  const [recoverySecretFilled, setRecoverySecretFilled] = useState(false);
  const [rightPanelMode, setRightPanelMode] = useState<RightPanelMode>("closed");
  const [pinnedNavigation, setPinnedNavigation] = useState<PinnedEventNavigation | null>(null);
  const [selectedProfileUserId, setSelectedProfileUserId] = useState<string | null>(null);
  const [peoplePanelScope, setPeoplePanelScope] = useState<PeoplePanelScope | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);
  const [rightPanelWidth, setRightPanelWidth] = useState(DEFAULT_RIGHT_PANEL_WIDTH);
  const [qaSendStatus, setQaSendStatus] = useState<QaSendSmokeStatus>("idle");
  // Issue #450: transient localized notice for recognized-but-unavailable
  // slash commands (e.g. /join, /invite), rendered above the composer that
  // owns the rejected target (main vs thread, keyed by the event's key).
  const [composerNotice, setComposerNotice] = useState<{
    key: TimelineKey;
    message: string;
  } | null>(null);
  const composerNoticeTimerRef = useRef<number | null>(null);
  const showComposerNotice = useCallback((key: TimelineKey, message: string) => {
    setComposerNotice({ key, message });
    if (composerNoticeTimerRef.current !== null) {
      window.clearTimeout(composerNoticeTimerRef.current);
    }
    composerNoticeTimerRef.current = window.setTimeout(() => {
      setComposerNotice(null);
      composerNoticeTimerRef.current = null;
    }, 4000);
  }, []);
  useEffect(() => {
    return () => {
      if (composerNoticeTimerRef.current !== null) {
        window.clearTimeout(composerNoticeTimerRef.current);
      }
    };
  }, []);
  const [timelineDiagnostics, setTimelineDiagnostics] =
    useState<QaTimelineDiagnostics>(INITIAL_TIMELINE_DIAGNOSTICS);
  const timelineDiagnosticsRef = useRef<QaTimelineDiagnostics>(INITIAL_TIMELINE_DIAGNOSTICS);
  const [spaceMembersCancelFailure, setSpaceMembersCancelFailure] =
    useState<SpaceMemberFence | null>(null);
  const [spaceMembersRoleTransportFailure, setSpaceMembersRoleTransportFailure] =
    useState<SpaceMemberFence | null>(null);
  const [savedSessions, setSavedSessions] = useState<SavedSessionInfo[]>([]);
  const [contextMenu, setContextMenu] = useState<ActiveContextMenu | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [loginTransportError, setLoginTransportError] = useState<string | null>(null);
  const [primaryView, setPrimaryView] = useState<PrimaryView>("timeline");
  // #161: while the main pane is anchored to a jump-to-date event, the focused
  // timeline renders in the MAIN pane, so a focused-context right panel must be
  // closed. Search keeps its results panel open while the main pane anchors the
  // selected hit.
  const mainTimelineAnchorEventId =
    snapshot?.state.ui.navigation.main_timeline_anchor?.event_id ?? null;
  useEffect(() => {
    if (mainTimelineAnchorEventId && rightPanelMode === "focusedContext") {
      setRightPanelMode("closed");
    }
  }, [mainTimelineAnchorEventId, rightPanelMode]);
  const [homeSelection, setHomeSelectionState] =
    useState<HomeSelection>(readHomeSelection);
  const [directorySearchDraft, setDirectorySearchDraft] = useState("");
  // Blank means the user's own homeserver directory.
  const [directoryServerDraft, setDirectoryServerDraft] = useState("");
  // #330: joining by address is its own Explore section, so it has its own
  // draft. Preview and join state stay Rust-owned; this notice only explains
  // input that never became a target.
  const [directoryAddressDraft, setDirectoryAddressDraft] = useState("");
  const [directoryAddressNotice, setDirectoryAddressNotice] = useState<
    "user" | "notRecognized" | null
  >(null);
  const [newDmDialogOpen, setNewDmDialogOpen] = useState(false);
  const [resetLocalDataConfirmOpen, setResetLocalDataConfirmOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [secureBackupInspectionRetrying, setSecureBackupInspectionRetrying] = useState(false);
  const secureBackupInspectionRetryInFlightRef = useRef(false);
  const [runtimeDiagnosticSnapshot, setRuntimeDiagnosticSnapshot] =
    useState<DiagnosticLogSnapshot>({ entries: [], droppedEntries: 0 });
  const [displayDensity, setDisplayDensityState] =
    useState<DisplayDensity>(readDisplayDensity);
  const viewportSyncReporter = useMemo(() => createViewportSyncReporter(api), []);
  useEffect(() => {
    void viewportSyncReporter("density_commit", displayDensity);
  }, [displayDensity, viewportSyncReporter]);
  useEffect(() => {
    const reportBrowserResize = () => {
      void viewportSyncReporter("browser_resize", displayDensity);
    };
    window.addEventListener("resize", reportBrowserResize);
    return () => window.removeEventListener("resize", reportBrowserResize);
  }, [displayDensity, viewportSyncReporter]);
  const [spaceLocalOverrides, setSpaceLocalOverrides] =
    useState<SpaceLocalOverrides>(readSpaceLocalOverrides);
  const [newDmDraftUserId, setNewDmDraftUserId] = useState("");
  const [inviteUserDialog, setInviteUserDialog] = useState<InviteUserDialogState>(null);
  const [inviteUserDialogVisible, setInviteUserDialogVisible] = useState(false);
  const [inviteUserDraftQuery, setInviteUserDraftQuery] = useState("");
  // React-local ephemeral state only: which create dialog is open and the
  // unsent name draft. The pending op status comes from the snapshot
  // (basic_operation); the created room/space identity comes from the API.
  const [createDialog, setCreateDialog] = useState<"room" | "space" | null>(null);
  const [createDraftName, setCreateDraftName] = useState("");
  const [createRoomDraftOptions, setCreateRoomDraftOptions] =
    useState<CreateRoomDialogOptions>(defaultCreateRoomDialogOptions);
  const [reportDialog, setReportDialog] = useState<ReportDialogState | null>(null);
  const [reportReasonDraft, setReportReasonDraft] = useState("");
  const [timelineStore, setTimelineStore] = useState<TimelineStoreState>(createTimelineStore);
  const threadStoreDiagnosticSignaturesRef = useRef<Map<string, string>>(new Map());
  const focusedStoreDiagnosticSignaturesRef = useRef<Map<string, string>>(new Map());
  const uiLatencyDiagnostics = useUiLatencyDiagnostics();
  const searchTimer = useRef<number | null>(null);
  const qaSendStarted = useRef(false);
  const qaSendPending = useRef(false);
  // #500: the send shortcut and the staging button share this path; one
  // staged-upload send in flight must not be re-entered (double Enter).
  const stagedUploadSendInFlightRef = useRef(false);
  const qaSendTargetRequested = useRef(false);
  const qaSendTargetSelectionRequested = useRef<string | null>(null);
  const qaSendBaselineErrorCount = useRef(0);
  const initialHomeSelectionApplied = useRef(false);
  const requestedAvatarMxcsRef = useRef<Set<string>>(new Set());
  const avatarRetryCountsRef = useRef<Map<string, number>>(new Map());
  const requestedMemberAvatarMxcsRef = useRef<Set<string>>(new Set());
  const memberAvatarRetryCountsRef = useRef<Map<string, number>>(new Map());

  useEffect(() => {
    const refreshOverrides = () => setSpaceLocalOverrides(readSpaceLocalOverrides());
    window.addEventListener(SPACE_OVERRIDES_CHANGED_EVENT, refreshOverrides);
    window.addEventListener("storage", refreshOverrides);
    return () => {
      window.removeEventListener(SPACE_OVERRIDES_CHANGED_EVENT, refreshOverrides);
      window.removeEventListener("storage", refreshOverrides);
    };
  }, []);



  function setDisplayDensity(density: DisplayDensity) {
    setDisplayDensityState(density);
    writeDisplayDensity(density);
  }

  function updateSpaceLocalOverride(
    spaceId: string,
    override: { name?: string; icon?: string } | null
  ) {
    setSpaceLocalOverrides(setSpaceLocalOverride(spaceId, override));
  }
  const qaSendBaselineTimelineItems = useRef(0);
  const stateRefreshTimerRef = useRef<number | null>(null);
  const panelDiagnosticRef = useRef<string | null>(null);
  const diagnosticSnapshotRequestGenerationRef = useRef(0);
  const typingSignalRef = useRef<{ roomId: string | null; isTyping: boolean }>({
    roomId: null,
    isTyping: false
  });
  const searchInputRef = useRef<HTMLInputElement>(null);
  const loginPasswordRef = useRef<HTMLInputElement>(null);
  const recoverySecretRef = useRef<HTMLInputElement>(null);
  const roomSettingsLoadRef = useRef<string | null>(null);
  const roomSettingsRequestRef = useRef(0);
  const spaceSettingsLoadRef = useRef<string | null>(null);
  const spaceSettingsRequestRef = useRef(0);
  const roomNavigationRequestRef = useRef(0);
  const spaceNavigationRequestRef = useRef(0);
  const spaceMembersOpenRequestRef = useRef(0);
  const spaceMembersInviteRequestRef = useRef(0);
  const spaceMembersCancelRequestRef = useRef(0);
  const spaceMembersRoleRequestRef = useRef(0);
  const spaceMembersLoadInFlightRef = useRef<Map<string, Promise<DesktopSnapshot | null>>>(
    new Map()
  );
  const spaceMembersLoadedRef = useRef<Set<string>>(new Set());
  const appTimelineTransport = useMemo<TimelineTransport | null>(() => {
    if (!tauriTimelineTransport) {
      return null;
    }
    return {
      ...tauriTimelineTransport,
      async acknowledgeProjection(
        projectionRequestId,
        key,
        generation,
        itemCount,
        targetPresent
      ) {
        await api.acknowledgeTimelineProjection(
          projectionRequestId,
          key,
          generation,
          itemCount,
          targetPresent
        );
      },
      async acknowledgeRenderedBatch(
        key,
        actorGeneration,
        timelineGeneration,
        repairGeneration,
        batchId
      ) {
        await api.acknowledgeTimelineBatchRendered(
          key,
          actorGeneration,
          timelineGeneration,
          repairGeneration,
          batchId
        );
      },
      async pinEvent(roomId: string, eventId: string) {
        setSnapshot(await api.pinEvent(roomId, eventId));
      },
      async unpinEvent(roomId: string, eventId: string) {
        setSnapshot(await api.unpinEvent(roomId, eventId));
      },
      async openAtTimestamp(roomId: string, timestampMs: number) {
        const nextSnapshot = await api.openTimelineAtTimestamp(roomId, timestampMs);
        setSnapshot(nextSnapshot);
        // #161: jump-to-date renders the focused timeline in the MAIN pane
        // (via navigation.main_timeline_anchor), not the right panel. Explicitly
        // close the right panel so an already-open focused-context/search panel
        // does not linger over the anchored main timeline.
        setPrimaryView("timeline");
        setRightPanelMode("closed");
      }
    };
  }, []);
  const appendDiagnosticLog = useCallback((entry: TimelineDiagnosticLogEntry) => {
    diagnosticLogBuffer.append(entry);
  }, [diagnosticLogBuffer]);
  const appendComposerSubmitDiagnostic = useCallback(
    (surface: "main" | "thread", stage: string, details: string) => {
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "composer.submit",
        message: `stage=${stage} surface=${surface} layer=app ${details}`
      });
    },
    [appendDiagnosticLog]
  );
  const updateTimelineDiagnostics = useCallback((diagnostics: TimelineDiagnostics) => {
    if (timelineDiagnosticsEqual(timelineDiagnosticsRef.current, diagnostics)) {
      return;
    }
    timelineDiagnosticsRef.current = diagnostics;
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "timeline",
      message: timelineDiagnosticsLogMessage(diagnostics)
    });
    setTimelineDiagnostics(diagnostics);
  }, [appendDiagnosticLog]);
  const appendPanelDiagnosticLog = useCallback((message: string) => {
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "panel",
      message
    });
  }, [appendDiagnosticLog]);
  const appendSpaceMembersDiagnosticLog = useCallback((message: string) => {
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "ui.space_members_panel",
      message
    });
  }, [appendDiagnosticLog]);
  const ensureSpaceMembersLoaded = useCallback(
    (spaceId: string, generation: number): Promise<DesktopSnapshot | null> => {
      const fence = { spaceId, generation };
      if (!spaceMembersSnapshotMatches(snapshotRef.current, fence)) {
        return Promise.resolve(null);
      }

      const key = `${spaceId}\u0000${generation}`;
      const completed = spaceMembersLoadedRef.current;
      if (completed.has(key)) {
        return Promise.resolve(snapshotRef.current);
      }

      const inFlight = spaceMembersLoadInFlightRef.current;
      const existing = inFlight.get(key);
      if (existing) {
        return existing;
      }

      const request = api.loadSpaceMembers(spaceId, generation).then((nextSnapshot) => {
        if (
          !spaceMembersSnapshotMatches(snapshotRef.current, fence) ||
          !spaceMembersSnapshotMatches(nextSnapshot, fence)
        ) {
          return null;
        }
        completed.add(key);
        setSnapshot(nextSnapshot);
        return nextSnapshot;
      });
      inFlight.set(key, request);
      const clearInFlight = () => {
        if (inFlight.get(key) === request) {
          inFlight.delete(key);
        }
      };
      request.then(clearInFlight, clearInFlight);
      return request;
    },
    [setSnapshot]
  );
  const attentionSummary = snapshot
    ? desktopAttentionSummary(snapshot.state.domain.native_attention)
    : null;
  const safeAttentionSummary =
    attentionSummary ?? {
      unreadTotal: 0,
      badgeCount: 0,
      notificationKind: "none" as const,
      titleHint: null,
      qaTitleToken: "unread=0 badge=0 notify=none"
    };
  const timelineRoomId = snapshot?.state.ui.timeline.room_id ?? null;
  const snapshotComposerDocument =
    snapshot?.state.ui.timeline.composer.document ??
    documentFromText(snapshot?.state.ui.timeline.composer.draft ?? "");
  const currentComposerAccount = readyComposerDraftAccountOwner(snapshot);
  const accountOwnerKey = currentComposerAccount
    ? composerDraftAccountOwnerKey(currentComposerAccount)
    : "no-account";
  const mainComposerOverlay = mainComposerOverlayRef.current;
  const composerDocument =
    timelineRoomId &&
    mainComposerOverlay?.scope.target.kind === "main" &&
    mainComposerOverlay.scope.target.room_id === timelineRoomId &&
    mainComposerOverlay.scope.account.homeserver === currentComposerAccount?.homeserver &&
    mainComposerOverlay.scope.account.user_id === currentComposerAccount?.userId &&
    mainComposerOverlay.scope.account.device_id === currentComposerAccount?.deviceId
      ? mainComposerOverlay.document
      : snapshotComposerDocument;
  const mainComposerDraftImeKey = [accountOwnerKey, "main", timelineRoomId ?? "no-room",
    snapshot?.state.ui.timeline.composer.last_accepted_clear_revision ??
      COMPOSER_DRAFT_REVISION_ZERO
  ].join("\u0000");
  const activeThreadState = snapshot?.state.ui.thread;
  const activeThreadTarget: ComposerTarget | null =
    activeThreadState?.kind === "open" &&
    activeThreadState.room_id &&
    activeThreadState.root_event_id
      ? {
          kind: "thread",
          room_id: activeThreadState.room_id,
          root_event_id: activeThreadState.root_event_id
        }
      : null;
  const activeThreadScope =
    currentComposerAccount && activeThreadTarget
      ? composerDraftScope(currentComposerAccount, activeThreadTarget)
      : null;
  const threadComposerOverlay = threadComposerOverlayRef.current;
  const threadComposerDocumentOverride =
    activeThreadScope &&
    threadComposerOverlay &&
    composerDraftScopesEqual(activeThreadScope, threadComposerOverlay.scope)
      ? threadComposerOverlay.document
      : undefined;
  const threadComposerDraftImeKey =
    activeThreadState?.kind === "open" && activeThreadState.composer
      ? [
          accountOwnerKey,
          "thread",
          activeThreadState.room_id,
          activeThreadState.root_event_id,
          activeThreadState.composer.last_accepted_clear_revision
        ].join("\u0000")
      : undefined;
  const stagedUploads = snapshot?.state.ui.timeline.staged_uploads ?? [];
  const retainedTimelineKeyIds = useMemo(
    () => retainedTimelineStoreKeyIds(snapshot),
    [snapshot]
  );
  const retainedTimelineKeyIdsRef = useRef(retainedTimelineKeyIds);
  retainedTimelineKeyIdsRef.current = retainedTimelineKeyIds;
  const currentTimelineStoreSessionKey = timelineStoreSessionKey(snapshot);
  const timelineStoreSessionKeyRef = useRef(currentTimelineStoreSessionKey);
  const timelineStoreContextValue = useMemo(
    () =>
      appTimelineTransport
        ? { store: timelineStore, setStore: setTimelineStore }
        : null,
    [appTimelineTransport, timelineStore]
  );

  useEffect(() => {
    if (timelineStoreSessionKeyRef.current === currentTimelineStoreSessionKey) {
      return;
    }
    timelineStoreSessionKeyRef.current = currentTimelineStoreSessionKey;
    threadStoreDiagnosticSignaturesRef.current.clear();
    focusedStoreDiagnosticSignaturesRef.current.clear();
    setTimelineStore(createTimelineStore());
  }, [currentTimelineStoreSessionKey]);

  useEffect(() => {
    setTimelineStore((current) => pruneTimelineStore(current, retainedTimelineKeyIds));
  }, [retainedTimelineKeyIds]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }
    const effectiveMode = effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot);
    const token = [
      `mode=${effectiveMode}`,
      `requested=${rightPanelMode}`,
      `thread=${snapshot.state.ui.thread.kind}`,
      `threads=${snapshot.state.ui.threads_list.kind}`
    ].join(" ");
    if (panelDiagnosticRef.current === token) {
      return;
    }
    panelDiagnosticRef.current = token;
    appendPanelDiagnosticLog(token);
  }, [
    appendPanelDiagnosticLog,
    rightPanelMode,
    snapshot?.state.ui.thread.kind,
    snapshot?.state.ui.threads_list.kind
  ]);

  useEffect(() => {
    if (snapshot?.state.ui.timeline.room_id) {
      return;
    }
    timelineDiagnosticsRef.current = INITIAL_TIMELINE_DIAGNOSTICS;
    setTimelineDiagnostics((current) =>
      current.visibleItems === 0 &&
      current.downloadedItems === 0 &&
      current.backfill === "unknown" &&
      current.avatarMxcItems === 0 &&
      current.avatarReadyItems === 0 &&
      current.avatarPendingItems === 0 &&
      current.avatarFailedItems === 0 &&
      current.avatarMissingItems === 0 &&
      current.avatarRenderedImages === 0 &&
      current.avatarBrokenImages === 0
        ? current
        : INITIAL_TIMELINE_DIAGNOSTICS
    );
  }, [snapshot?.state.ui.timeline.room_id]);

  useEffect(() => {
    if (!snapshot || !tauriTimelineTransport?.downloadAvatarThumbnail) {
      requestedAvatarMxcsRef.current.clear();
      avatarRetryCountsRef.current.clear();
      requestedMemberAvatarMxcsRef.current.clear();
      memberAvatarRetryCountsRef.current.clear();
      return;
    }
    // #116 perf gate: avatar downloads are disabled by default to prevent the
    // AccountActor command flood that froze room selection.
    if (!AVATAR_THUMBNAIL_DOWNLOADS_ENABLED) {
      return;
    }

    for (const profile of Object.values(snapshot.state.domain.profile.users)) {
      const avatar = profile.avatar;
      if (!avatar || !requestedMemberAvatarMxcsRef.current.has(avatar.mxc_uri)) {
        continue;
      }
      if (avatar.thumbnail.kind === "ready") {
        requestedMemberAvatarMxcsRef.current.delete(avatar.mxc_uri);
        memberAvatarRetryCountsRef.current.delete(avatar.mxc_uri);
      } else if (avatar.thumbnail.kind === "failed") {
        requestedMemberAvatarMxcsRef.current.delete(avatar.mxc_uri);
      }
    }

    const plan = planSnapshotAvatarThumbnailRequests(
      snapshot,
      requestedAvatarMxcsRef.current,
      avatarRetryCountsRef.current
    );
    requestedAvatarMxcsRef.current = plan.requestedMxcUris;
    avatarRetryCountsRef.current = plan.retryCounts;

    for (const mxcUri of plan.requestMxcUris) {
      if (requestedMemberAvatarMxcsRef.current.has(mxcUri)) {
        continue;
      }
      void tauriTimelineTransport.downloadAvatarThumbnail(mxcUri).catch(() => {
        requestedAvatarMxcsRef.current.delete(mxcUri);
      });
    }
  }, [snapshot]);

  const requestMemberAvatarThumbnail = useCallback((mxcUri: string): Promise<void> => {
    if (!AVATAR_THUMBNAIL_DOWNLOADS_ENABLED || !tauriTimelineTransport?.downloadAvatarThumbnail) {
      return Promise.resolve();
    }
    return requestAvatarThumbnailWithDedupe(
      mxcUri,
      requestedAvatarMxcsRef.current,
      requestedMemberAvatarMxcsRef.current,
      memberAvatarRetryCountsRef.current,
      tauriTimelineTransport.downloadAvatarThumbnail
    );
  }, []);

  function handleShortcutAction(shortcutId: string): boolean {
    switch (shortcutId) {
      case "showKeyboardSettings":
        void setRightPanelModeClosingFocusedContext("keyboardSettings");
        return true;
      case "openUserSettings":
        void setRightPanelModeClosingFocusedContext("userSettings");
        return true;
      case "logout":
        void logout();
        return true;
      case "searchInRoom":
        setSearchScope("currentRoom");
        searchInputRef.current?.focus();
        return true;
      case "filterRooms":
        setSearchScope("allRooms");
        searchInputRef.current?.focus();
        return true;
      case "toggleRightPanel":
        void setRightPanelModeClosingFocusedContext(
          rightPanelMode === "closed" ? "roomInfo" : "closed"
        );
        return true;
      case "toggleFullscreen":
        void (async () => {
          const win = getCurrentWindow();
          const fullscreen = await win.isFullscreen();
          await win.setFullscreen(!fullscreen);
        })();
        return true;
      default:
        return false;
    }
  }

  function openContextMenu(
    event: MouseEvent<HTMLElement>,
    target: ContextMenuTarget,
    items: ContextMenuItem[]
  ) {
    if (!items.length) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    setContextMenu({
      x: event.clientX,
      y: event.clientY,
      target,
      items
    });
  }

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    return () => {
      const mainOverlay = mainComposerOverlayRef.current;
      if (mainOverlay && mainOverlay.debounceHandle !== null) {
        window.clearTimeout(mainOverlay.debounceHandle);
      }
      const threadOverlay = threadComposerOverlayRef.current;
      if (threadOverlay && threadOverlay.debounceHandle !== null) {
        window.clearTimeout(threadOverlay.debounceHandle);
      }
      composerDraftLifecycleRegistryRef.current?.revokeRendererGeneration();
    };
  }, []);

  useEffect(() => {
    if (rightPanelMode === "userSettings") {
      void refreshSavedSessions();
    }
  }, [rightPanelMode]);

  useEffect(() => {
    const roomId = snapshot?.state.ui.timeline.room_id ?? null;
    const previous = typingSignalRef.current;

    if (previous.roomId && previous.roomId !== roomId && previous.isTyping) {
      void api.setTyping(previous.roomId, false).catch(() => undefined);
    }

    if (previous.roomId !== roomId) {
      typingSignalRef.current = { roomId, isTyping: false };
    }
  }, [snapshot?.state.ui.timeline.room_id]);

  useEffect(() => {
    const theme = snapshot?.state.domain.settings.values.appearance.theme ?? "system";
    if (theme === "system") {
      delete document.documentElement.dataset.theme;
      return;
    }
    document.documentElement.dataset.theme = theme;
  }, [snapshot?.state.domain.settings.values.appearance.theme]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    const profile = snapshot.state.domain.locale_profile;
    document.documentElement.lang = profile.lang;
    document.documentElement.dir = profile.dir;
    document.documentElement.dataset.catalogLocale = profile.catalog_locale;
    document.documentElement.dataset.pseudoLocale = profile.pseudo_locale;
  }, [
    snapshot?.state.domain.locale_profile.lang,
    snapshot?.state.domain.locale_profile.dir,
    snapshot?.state.domain.locale_profile.catalog_locale,
    snapshot?.state.domain.locale_profile.pseudo_locale
  ]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    const profile = snapshot.state.domain.typography_profile;
    document.documentElement.dataset.uiFont = profile.font;
    document.documentElement.dataset.emojiFont = profile.emoji;
    document.documentElement.dataset.fontAsset = profile.font_asset;
    document.documentElement.dataset.emojiAsset = profile.emoji_asset;
  }, [
    snapshot?.state.domain.typography_profile.font,
    snapshot?.state.domain.typography_profile.emoji,
    snapshot?.state.domain.typography_profile.font_asset,
    snapshot?.state.domain.typography_profile.emoji_asset
  ]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    if (searchTimer.current) {
      window.clearTimeout(searchTimer.current);
    }

    searchTimer.current = window.setTimeout(() => {
      void runSearch(searchQuery, searchScope);
    }, 120);

    return () => {
      if (searchTimer.current) {
        window.clearTimeout(searchTimer.current);
      }
    };
  }, [
    searchQuery,
    searchScope,
    snapshot?.state.ui.navigation.active_room_id,
    snapshot?.state.ui.navigation.active_space_id
  ]);

  const attentionWindowTitle = snapshot
    ? qaTitleEnabled()
      ? qaWindowTitle(
          snapshot,
          effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot),
          qaSendStatus,
          [
            ...qaSendSmokeTargetDiagnosticTokens(snapshot, qaSendSmokeTargetUserId()),
            ...qaTimelineDiagnosticTokens(timelineDiagnostics),
            ...qaDomDiagnosticTokens(qaRenderedDomDiagnostics())
          ]
        )
      : desktopAttentionWindowTitle("Koushi", safeAttentionSummary)
    : qaTitleEnabled()
      ? "koushi-desktop qa session=booting"
      : "Koushi";
  useDesktopAttentionEffects({
    snapshot,
    attentionWindowTitle,
    safeAttentionSummary,
    appendDiagnosticLog
  });

  useEffect(() => {
    const message = qaSendSmokeMessage();
    const targetUserId = qaSendSmokeTargetUserId();
    const targetRoom =
      targetUserId && snapshot ? qaSendSmokeTargetRoom(snapshot, targetUserId) : null;
    const targetRoomIsSelected =
      !targetUserId ||
      (targetRoom !== null && snapshot?.state.ui.timeline.room_id === targetRoom.room_id);
    if (
      !message ||
      !snapshot ||
      qaSendStarted.current ||
      !targetRoomIsSelected ||
      !qaSendSmokeCanStart(snapshot)
    ) {
      if (
        message &&
        targetUserId &&
        snapshot &&
        !qaSendStarted.current &&
        snapshot.state.domain.session.kind === "ready" &&
        snapshot.state.ui.errors.length === 0
      ) {
        if (!targetRoom && !qaSendTargetRequested.current) {
          qaSendTargetRequested.current = true;
          void api.startDirectMessage(targetUserId).then(setSnapshot).catch(() => {
            qaSendPending.current = false;
            setQaSendStatus("failed");
          });
          return;
        }
        if (
          targetRoom &&
          snapshot.state.ui.timeline.room_id !== targetRoom.room_id &&
          qaSendTargetSelectionRequested.current !== targetRoom.room_id
        ) {
          qaSendTargetSelectionRequested.current = targetRoom.room_id;
          void selectRoom(targetRoom.room_id).catch(() => {
            qaSendPending.current = false;
            setQaSendStatus("failed");
          });
        }
      }
      return;
    }
    const roomId = snapshot.state.ui.timeline.room_id;
    const account = readyComposerDraftAccountOwner(snapshot);
    if (!roomId || !account) {
      return;
    }

    qaSendStarted.current = true;
    qaSendBaselineErrorCount.current = snapshot.state.ui.errors.length;
    qaSendBaselineTimelineItems.current = snapshot.timeline.length;
    qaSendPending.current = true;
    setQaSendStatus("pending");
    void sendText(documentFromText(message));
  }, [snapshot]);

  useEffect(() => {
    if (
      !snapshot ||
      !qaSendStarted.current ||
      qaSendStatus !== "pending"
    ) {
      return;
    }
    const completionStatus = qaSendSmokeCompletionStatus(
      snapshot,
      qaSendBaselineErrorCount.current,
      qaSendBaselineTimelineItems.current
    );
    if (isTauriRuntime() && completionStatus !== "failed") {
      return;
    }
    qaSendPending.current = completionStatus === "pending";
    setQaSendStatus(completionStatus);
  }, [snapshot, qaSendStatus]);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }

    // Tauri production sends complete on the CoreEvent stream. Snapshots do
    // not carry timeline rows, so SendCompleted/OperationFailed owns the QA
    // send status while a WebDriver-driven send is pending.
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<CoreEventPayload>(CORE_EVENT_NAME, (event) => {
      if (!qaSendPending.current) {
        return;
      }
      const eventStatus = qaSendCompletionStatusFromCoreEvent(event.payload);
      if (eventStatus) {
        qaSendPending.current = false;
        setQaSendStatus(eventStatus);
      }
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent) {
      const shortcutId = shortcutIdForKeyboardEvent(event);
      if (!shortcutId) {
        return;
      }

      if (handleShortcutAction(shortcutId)) {
        event.preventDefault();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<string>(MENU_EVENT_NAME, (event) => {
      const shortcutId = shortcutActionFromMenuPayload(event.payload);
      if (shortcutId) {
        handleShortcutAction(shortcutId);
      }
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;
    const deltaBatcher = createOrderedEventBatcher<
      Extract<CoreEventPayload, { kind: "StateDelta" }>
    >((deltas) => {
      const applied = applyAppStoreDeltas(
        deltas.map((delta) => ({
          generation: delta.generation,
          changed: delta.changed
        }))
      );
      if (!applied) {
        void refresh();
      }
    });
    void listen<CoreEventPayload>(CORE_EVENT_NAME, (event) => {
      if (event.payload.kind !== "StateDelta") {
        return;
      }
      deltaBatcher.enqueue(event.payload);
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    });

    return () => {
      disposed = true;
      deltaBatcher.dispose();
      unlisten?.();
    };
  }, []);

  // App-level timeline store: apply CoreEvent::Timeline diffs once, then feed
  // the resulting store into every TimelineView. This keeps Matrix timeline
  // semantics Rust-owned and avoids per-view reducer ownership.
  useEffect(() => {
    if (!appTimelineTransport) {
      return;
    }

    let disposed = false;
    const eventBatcher = createOrderedEventBatcher<CoreEventPayload>((payloads) => {
      if (disposed) {
        return;
      }
      setTimelineStore((current) => {
        let next = current;
        for (const payload of payloads) {
          if (payload.kind === "ResyncMarker") {
            next = pruneTimelineStore(
              applyGlobalResync(next),
              retainedTimelineKeyIdsRef.current
            );
            continue;
          }
          if (payload.kind === "OperationFailed") {
            // Issue #450: a recognized-but-unavailable slash command (e.g.
            // /join, /invite) is rejected before any Matrix send; surface the
            // localized explanation near the composer instead of appearing
            // inert. Transient: auto-dismissed. The OperationFailed envelope
            // carries no key, so the notice keys to the active main room
            // (schedule-time rejections use the keyed
            // ComposerSlashCommandRejected event instead).
            if (isUnsupportedSlashCommandRejection(payload)) {
              // The keyless OperationFailed surface is the legacy send path;
              // only show the notice when a ready account exists (the key
              // must be canonical to route at all).
              const account = readyComposerDraftAccountOwner(snapshotRef.current);
              const activeRoomId = snapshotRef.current?.state.ui.timeline.room_id;
              if (account && activeRoomId) {
                showComposerNotice(
                  roomTimelineKey(account.userId, activeRoomId),
                  t("composer.slashCommandUnavailable")
                );
              }
            }
            continue;
          }
          if (
            payload.kind === "Room" &&
            typeof payload.event === "object" &&
            payload.event !== null &&
            "ComposerSlashCommandRejected" in payload.event
          ) {
            // Issue #450: the schedule-time rejection is keyed to the exact
            // composer target by Rust, so the notice routes without any
            // frontend correlation.
            showComposerNotice(
              payload.event.ComposerSlashCommandRejected.key,
              t("composer.slashCommandUnavailable")
            );
            continue;
          }
          if (
            payload.kind === "Room" &&
            typeof payload.event === "object" &&
            payload.event !== null &&
            "RoomKeyRequestStateChanged" in payload.event
          ) {
            const change = payload.event.RoomKeyRequestStateChanged;
            next = applyRoomKeyRequestStateChanged(
              next,
              change.key,
              change.event_id,
              change.stage,
              change.withheld_code
            );
            continue;
          }
          if (payload.kind !== "Timeline") {
            continue;
          }
          if (isUnsupportedSlashCommandRejection(payload)) {
            // Issue #450: the production submission path rejects recognized
            // but unavailable slash commands via SubmissionRejected (not
            // OperationFailed); surface the localized notice near the
            // composer that owns the rejected target (keyed by the event).
            if ("SubmissionRejected" in payload.event) {
              showComposerNotice(
                payload.event.SubmissionRejected.key,
                t("composer.slashCommandUnavailable")
              );
            }
          }
          const applied = applyTimelineEventWithProjectionResultAndRetention(
            next,
            payload.event,
            retainedTimelineKeyIdsRef.current
          );
          if (
            "InitialItems" in payload.event &&
            "Focused" in payload.event.InitialItems.key.kind
          ) {
            const message = timelineStoreInitialItemsDiagnosticMessage(
              next,
              applied.store,
              payload.event.InitialItems,
              retainedTimelineKeyIdsRef.current
            );
            const keyId = timelineStoreKeyId(payload.event.InitialItems.key);
            if (focusedStoreDiagnosticSignaturesRef.current.get(keyId) !== message) {
              focusedStoreDiagnosticSignaturesRef.current.set(keyId, message);
              appendDiagnosticLog({
                timestampMs: Date.now(),
                source: "timeline.store",
                message
              });
            }
          }
          if (
            "ItemsUpdated" in payload.event &&
            "Thread" in payload.event.ItemsUpdated.key.kind
          ) {
            const message = threadTimelineStoreDiagnosticMessage(
              next,
              applied.store,
              payload.event.ItemsUpdated
            );
            const keyId = timelineStoreKeyId(payload.event.ItemsUpdated.key);
            if (threadStoreDiagnosticSignaturesRef.current.get(keyId) !== message) {
              threadStoreDiagnosticSignaturesRef.current.set(keyId, message);
              appendDiagnosticLog({
                timestampMs: Date.now(),
                source: "thread.timeline",
                message
              });
            }
          }
          if (
            applied.projection.kind === "applied" &&
            ("Focused" in applied.projection.key.kind ||
              "Thread" in applied.projection.key.kind)
          ) {
            // The Core command is idempotent because React may replay updater
            // functions in development. Store application always precedes ACK.
            void api.acknowledgeTimelineProjection(
              applied.projection.requestId,
              applied.projection.key,
              applied.projection.generation,
              applied.projection.itemCount,
              applied.projection.targetPresent
            );
          }
          next = applied.store;
        }
        return next;
      });
    });
    const unsubscribe = appTimelineTransport.listenCoreEvents((payload) => {
      if (disposed) {
        return;
      }
      eventBatcher.enqueue(payload);
    });

    return () => {
      disposed = true;
      eventBatcher.dispose();
      unsubscribe();
    };
  }, [appTimelineTransport]);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<string>(STATE_EVENT_NAME, () => {
      if (stateRefreshTimerRef.current !== null) {
        return;
      }
      stateRefreshTimerRef.current = window.setTimeout(() => {
        stateRefreshTimerRef.current = null;
        void refresh();
      }, STATE_EVENT_REFRESH_DEBOUNCE_MS);
    }).then((dispose) => {
      if (disposed) {
        dispose();
      } else {
        unlisten = dispose;
      }
    });

    return () => {
      disposed = true;
      if (stateRefreshTimerRef.current !== null) {
        window.clearTimeout(stateRefreshTimerRef.current);
        stateRefreshTimerRef.current = null;
      }
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!snapshot || rightPanelMode !== "roomInfo") {
      return;
    }
    const activeRoomId = snapshot.state.ui.navigation.active_room_id;
    if (!activeRoomId) {
      return;
    }
    const roomManagement = snapshot.state.domain.room_management;
    if (
      roomManagement.selected_room_id === activeRoomId &&
      roomManagement.settings
    ) {
      roomSettingsLoadRef.current = activeRoomId;
      return;
    }
    if (
      roomManagement.operation.kind === "pending" &&
      roomManagement.operation.room_id === activeRoomId
    ) {
      return;
    }
    if (roomSettingsLoadRef.current === activeRoomId) {
      return;
    }
    roomSettingsLoadRef.current = activeRoomId;
    const requestId = ++roomSettingsRequestRef.current;
    const navigationRequestId = roomNavigationRequestRef.current;
    void api.loadRoomSettings(activeRoomId).then((nextSnapshot) => {
      if (
        roomSettingsRequestRef.current !== requestId ||
        roomNavigationRequestRef.current !== navigationRequestId ||
        snapshotRef.current?.state.ui.navigation.active_room_id !== activeRoomId ||
        nextSnapshot.state.ui.navigation.active_room_id !== activeRoomId ||
        !exactRoomSettingsForRoom(nextSnapshot, activeRoomId)
      ) {
        return;
      }
      setSnapshot(nextSnapshot);
    });
  }, [
    rightPanelMode,
    snapshot?.state.ui.navigation.active_room_id,
    snapshot?.state.domain.room_management.operation,
    snapshot?.state.domain.room_management.selected_room_id,
    snapshot?.state.domain.room_management.settings
  ]);

  useEffect(() => {
    if (!snapshot || rightPanelMode !== "spaceInfo") {
      return;
    }
    const activeSpaceId = snapshot.state.ui.navigation.active_space_id;
    if (!activeSpaceId) {
      return;
    }
    const roomManagement = snapshot.state.domain.room_management;
    if (
      roomManagement.selected_room_id === activeSpaceId &&
      roomManagement.settings
    ) {
      spaceSettingsLoadRef.current = activeSpaceId;
      return;
    }
    if (
      roomManagement.operation.kind === "pending" &&
      roomManagement.operation.room_id === activeSpaceId
    ) {
      return;
    }
    if (spaceSettingsLoadRef.current === activeSpaceId) {
      return;
    }
    spaceSettingsLoadRef.current = activeSpaceId;
    const requestId = ++spaceSettingsRequestRef.current;
    const navigationRequestId = roomNavigationRequestRef.current;
    void api.loadRoomSettings(activeSpaceId).then((nextSnapshot) => {
      if (
        spaceSettingsRequestRef.current !== requestId ||
        roomNavigationRequestRef.current !== navigationRequestId ||
        snapshotRef.current?.state.ui.navigation.active_space_id !== activeSpaceId ||
        nextSnapshot.state.ui.navigation.active_space_id !== activeSpaceId ||
        !exactRoomSettingsForRoom(nextSnapshot, activeSpaceId)
      ) {
        return;
      }
      setSnapshot(nextSnapshot);
    });
  }, [
    rightPanelMode,
    snapshot?.state.ui.navigation.active_space_id,
    snapshot?.state.domain.room_management.operation,
    snapshot?.state.domain.room_management.selected_room_id,
    snapshot?.state.domain.room_management.settings
  ]);

  useEffect(() => {
    const fence = spaceMembersFenceForSnapshot(snapshot);
    if (!fence) {
      return;
    }

    void ensureSpaceMembersLoaded(fence.spaceId, fence.generation).catch(() => {
      if (spaceMembersSnapshotMatches(snapshotRef.current, fence)) {
        appendSpaceMembersDiagnosticLog("load outcome=failed");
      }
    });
  }, [
    appendSpaceMembersDiagnosticLog,
    ensureSpaceMembersLoaded,
    snapshot?.state.domain.space_members?.generation,
    snapshot?.state.domain.space_members?.selected_space_id,
    snapshot?.state.ui.navigation.active_space_id
  ]);

  async function refresh() {
    setIsBusy(true);
    try {
      setSnapshot(await api.getSnapshot());
    } finally {
      setIsBusy(false);
    }
  }

  async function refreshSavedSessions() {
    setSavedSessions(await api.listSavedSessions());
  }

  async function switchAccount(session: SavedSessionInfo) {
    setIsBusy(true);
    try {
      setSnapshot(await api.switchAccount(session));
      setRightPanelMode("thread");
      await refreshSavedSessions();
    } finally {
      setIsBusy(false);
    }
  }

  async function logout() {
    setIsBusy(true);
    try {
      setSnapshot(await api.logout());
      setRightPanelMode("thread");
      await refreshSavedSessions();
    } finally {
      setIsBusy(false);
    }
  }

  async function retrySlidingSyncCapability() {
    setIsBusy(true);
    try {
      setSnapshot(await api.retrySlidingSyncCapability());
    } finally {
      setIsBusy(false);
    }
  }

  async function changeCapabilityHomeserver() {
    setIsBusy(true);
    try {
      setSnapshot(await api.changeHomeserver());
    } finally {
      setIsBusy(false);
    }
  }

  async function submitLogin(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const password = loginPasswordRef.current?.value ?? "";
    const sessionKind = snapshot?.state.domain.session.kind;
    setIsBusy(true);
    setLoginTransportError(null);
    try {
      const login =
        sessionKind === "locked"
          ? api.submitSoftLogoutReauth(password)
          : api.submitLogin(
              loginHomeserver,
              loginUsername,
              password,
              loginDeviceName,
              snapshot?.state.domain.locale_profile.platform ?? "linux"
            );
      setLoginTransportError(await settleLoginTransport(login, () => api.getSnapshot(), setSnapshot));
    } finally {
      if (loginPasswordRef.current) {
        loginPasswordRef.current.value = "";
      }
      setLoginPasswordFilled(false);
      setIsBusy(false);
    }
  }

  async function discoverLoginMethods() {
    setIsBusy(true);
    try {
      setSnapshot(await api.discoverLoginMethods(loginHomeserver));
    } finally {
      setIsBusy(false);
    }
  }

  async function startOidcLogin() {
    setIsBusy(true);
    try {
      const authorization = await api.startOidcLogin(loginHomeserver);
      await openExternalHttpUrl(authorization.authorization_url);
    } finally {
      setIsBusy(false);
    }
  }

  async function submitRecovery(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const secret = recoverySecretRef.current?.value.trim() ?? "";
    setIsBusy(true);
    try {
      setSnapshot(await api.submitRecovery(secret));
    } finally {
      if (recoverySecretRef.current) {
        recoverySecretRef.current.value = "";
      }
      setRecoverySecretFilled(false);
      setIsBusy(false);
    }
  }

  async function restartSync() {
    setIsBusy(true);
    try {
      setSnapshot(await api.restartSync());
    } finally {
      setIsBusy(false);
    }
  }

  async function updateSettings(patch: SettingsPatch) {
    setSnapshot(await api.updateSettings(patch));
  }

  async function rebuildSearchIndex() {
    setSnapshot(await api.rebuildSearchIndex());
  }

  async function startRoomCrawl(roomId: string) {
    setSnapshot(await api.startRoomCrawl(roomId));
  }

  async function stopRoomCrawl(roomId: string) {
    setSnapshot(await api.stopRoomCrawl(roomId));
  }

  async function setRoomUrlPreviewOverride(roomId: string, enabled: boolean) {
    setSnapshot(await api.setRoomUrlPreviewOverride(roomId, enabled));
  }

  async function repairRoomTimeline(roomId: string) {
    setSnapshot(await api.repairRoomTimeline(roomId));
  }

  async function submitAccountManagementUia(flowId: number, password: string) {
    setSnapshot(await api.submitAccountManagementUia(flowId, password));
  }

  async function loadAccountManagementCapabilities() {
    setSnapshot(await api.loadAccountManagementCapabilities());
  }

  async function changePassword(newPassword: string) {
    setSnapshot(await api.changePassword(newPassword));
  }

  async function deactivateAccount(eraseData: boolean) {
    setSnapshot(await api.deactivateAccount(eraseData));
  }

  async function setDisplayName(displayName: string | null) {
    setSnapshot(await api.setDisplayName(displayName));
  }

  async function setLocalUserAlias(userId: string, alias: string | null) {
    await applyLatestTextMutationSnapshot(`alias:${userId}`, () => api.setLocalUserAlias(userId, alias));
  }

  async function ignoreUser(userId: string) {
    setSnapshot(await api.ignoreUser(userId));
  }

  async function unignoreUser(userId: string) {
    setSnapshot(await api.unignoreUser(userId));
  }

  function openReportDialog(state: ReportDialogState) {
    setReportDialog(state);
    setReportReasonDraft("");
  }

  async function openDiagnostics() {
    const requestGeneration = ++diagnosticSnapshotRequestGenerationRef.current;
    try {
      const nextSnapshot = await api.getDiagnosticSnapshot();
      if (requestGeneration !== diagnosticSnapshotRequestGenerationRef.current) {
        return;
      }
      setRuntimeDiagnosticSnapshot(nextSnapshot);
    } catch {
      if (requestGeneration !== diagnosticSnapshotRequestGenerationRef.current) {
        return;
      }
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "diagnostics.fetch",
        message: "kind=unavailable"
      });
    }
    setDiagnosticsOpen(true);
  }

  async function retrySecureBackupInspection() {
    const operation = api.retrySecureBackupInspection;
    if (!operation || secureBackupInspectionRetryInFlightRef.current) {
      return;
    }
    secureBackupInspectionRetryInFlightRef.current = true;
    setSecureBackupInspectionRetrying(true);
    try {
      setSnapshot(await operation());
    } catch {
      // The gate remains closed; typed Rust state or the next inspection owns the copy.
    } finally {
      secureBackupInspectionRetryInFlightRef.current = false;
      setSecureBackupInspectionRetrying(false);
    }
  }

  function closeReportDialog() {
    setReportDialog(null);
    setReportReasonDraft("");
  }

  function submitReportDialog() {
    const reason = reportReasonDraft.trim();
    if (!reason || !reportDialog) {
      return;
    }
    switch (reportDialog.kind) {
      case "user":
        void api.reportUser(reportDialog.userId, reason).then(setSnapshot);
        break;
      case "content":
        void api.reportContent(reportDialog.roomId, reportDialog.eventId, reason).then(setSnapshot);
        break;
      case "room":
        void api.reportRoom(reportDialog.roomId, reason).then(setSnapshot);
        break;
    }
    closeReportDialog();
  }

  async function setRoomNotificationMode(roomId: string, mode: RoomNotificationMode) {
    setSnapshot(await api.setRoomNotificationMode(roomId, mode));
  }

  async function setAvatar(file: File) {
    const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
    if (bytes.length === 0) {
      return;
    }
    setSnapshot(await api.setAvatar(file.type || "application/octet-stream", bytes));
  }

  async function bootstrapCrossSigning() {
    setSnapshot(await api.bootstrapCrossSigning());
  }

  async function enableKeyBackup() {
    setSnapshot(await api.enableKeyBackup());
  }

  async function exportRoomKeys(destinationPath: string, passphrase: string) {
    setSnapshot(await api.exportRoomKeys(destinationPath, passphrase));
  }

  async function importRoomKeys(sourcePath: string, passphrase: string) {
    setSnapshot(await api.importRoomKeys(sourcePath, passphrase));
  }

  async function reshareRoomKey(roomId: string) {
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "e2ee.room_key",
      message: "operation=manual_reshare stage=request"
    });
    try {
      const outcome = await api.reshareRoomKey(roomId);
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "e2ee.room_key",
        message: `operation=manual_reshare stage=completed outcome=${outcome.kind}`
      });
      return outcome;
    } catch (error) {
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "e2ee.room_key",
        message: "operation=manual_reshare stage=failed kind=transport"
      });
      throw error;
    }
  }
  async function forceNewOutboundSession(roomId: string) {
    return api.forceNewOutboundSession(roomId);
  }

  async function shareIndex0RoomKey(roomId: string) {
    return api.shareIndex0RoomKey(roomId);
  }

  async function resendIndex0RoomKey(roomId: string) {
    return api.resendIndex0RoomKey(roomId);
  }

  async function chooseRoomKeyExportDestination(): Promise<string | null> {
    if (!isTauriRuntime()) {
      return null;
    }
    const selected = await saveDialog({
      title: t("settings.roomKeyExport"),
      defaultPath: "koushi-room-keys.txt",
      filters: [{ name: t("settings.roomKeyExport"), extensions: ["txt", "json"] }]
    });
    return selected || null;
  }

  async function chooseSecureBackupDestination(): Promise<string | null> {
    if (!isTauriRuntime()) {
      return null;
    }
    const selected = await saveDialog({
      title: t("gate.secureBackupRecoveryKeyDestination"),
      defaultPath: "koushi-secure-backup-recovery-key.txt",
      filters: [
        {
          name: t("gate.secureBackupRecoveryKeyDestination"),
          extensions: ["txt"]
        }
      ]
    });
    return selected || null;
  }

  async function chooseRoomKeyImportSource(): Promise<string | null> {
    if (!isTauriRuntime()) {
      return null;
    }
    const selected = await openDialog({
      title: t("settings.roomKeyImport"),
      multiple: false,
      filters: [{ name: t("settings.roomKeyImport"), extensions: ["txt", "json"] }],
      fileAccessMode: "scoped"
    });
    return typeof selected === "string" ? selected : null;
  }

  async function bootstrapSecureBackup(
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) {
    setSnapshot(await api.bootstrapSecureBackup(passphrase, recoveryKeyDestinationPath));
  }

  async function changeSecureBackupPassphrase(
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ) {
    setSnapshot(
      await api.changeSecureBackupPassphrase(
        oldSecret,
        newPassphrase,
        recoveryKeyDestinationPath
      )
    );
  }

  async function probeLocalEncryptionHealth() {
    setSnapshot(await api.probeLocalEncryptionHealth());
  }

  /** Rust-projected display label for the room in the leave confirmation. */
  function roomLeaveDisplayName(roomId: string): string {
    return (
      snapshot?.state.domain.rooms.find((room) => room.room_id === roomId)?.display_label ??
      roomId
    );
  }

  async function leavePendingRoom() {
    const target = pendingRoomLeave;
    if (!target || roomLeaveInFlight) {
      return;
    }
    setRoomLeaveInFlight(true);
    try {
      // The room stays visible and selected until the Rust snapshot drops it.
      const nextSnapshot = await api.leaveRoom(target.roomId);
      setSnapshot(nextSnapshot);
      setPendingRoomLeave(null);
      const stillJoined = nextSnapshot.state.domain.rooms.some(
        (room) => room.room_id === target.roomId
      );
      if (!stillJoined && ROOM_BOUND_RIGHT_PANEL_MODES.has(effectiveRightPanelMode)) {
        await setRightPanelModeClosingFocusedContext("closed");
      }
    } finally {
      // On failure the dialog stays open with the room and selection unchanged,
      // so the user can retry without losing context.
      setRoomLeaveInFlight(false);
    }
  }

  async function resetLocalData() {
    setResetLocalDataConfirmOpen(false);
    setSnapshot(await api.resetLocalData());
  }

  async function acceptVerification(flowId: number) {
    setSnapshot(await api.acceptVerification(flowId));
  }

  async function confirmSasVerification(flowId: number) {
    setSnapshot(await api.confirmSasVerification(flowId));
  }

  async function cancelVerification(flowId: number) {
    setSnapshot(await api.cancelVerification(flowId));
  }

  async function resetIdentity() {
    setSnapshot(await api.resetIdentity());
  }

  async function cancelIdentityReset(flowId: number) {
    setSnapshot(await api.cancelIdentityReset(flowId));
  }

  async function submitIdentityResetPassword(flowId: number, password: string) {
    setSnapshot(await api.submitIdentityResetPassword(flowId, password));
  }

  async function submitIdentityResetOAuth(flowId: number) {
    setSnapshot(await api.submitIdentityResetOAuth(flowId));
  }

  const resolveComposerKeyAction: ResolveComposerKeyAction = (
    surface,
    keyEvent,
    options
  ) => api.resolveComposerKeyAction(surface, keyEvent, options);

  function setHomeSelection(selection: HomeSelection) {
    setHomeSelectionState(selection);
    writeHomeSelection(selection);
  }

  async function deactivateComposerScopeForNavigation(
    scope: ComposerDraftScope,
    kind: ComposerTarget["kind"]
  ): Promise<boolean> {
    const registry = composerDraftLifecycleRegistryRef.current!;
    const overlayForScope = () => {
      const overlay =
        kind === "main"
          ? mainComposerOverlayRef.current
          : threadComposerOverlayRef.current;
      return overlay && composerDraftScopesEqual(overlay.scope, scope) ? overlay : null;
    };
    const initialOverlay = overlayForScope();
    if (initialOverlay) {
      if (kind === "main") cancelComposerDraftPersist(scope);
      else cancelThreadComposerDraftPersist(scope);
      try {
        await registry.activate(scope);
      } catch {
        return false;
      }
      while (true) {
        const activeOverlay = overlayForScope();
        if (!activeOverlay || activeOverlay.document.inlines.length === 0) break;
        const revision = activeOverlay.revision ?? registry.nextDraft(scope);
        activeOverlay.revision = revision;
        registry.setActiveOverlay(scope, activeOverlay.document, revision);
        const admitted = beginComposerOperation(scope);
        if (!admitted) return false;
        const account = composerDraftApiAccount(scope);
        try {
          if (scope.target.kind === "main") {
            await api.setComposerDraft(
              account,
              admitted.lease.leaseId,
              admitted.lease.rendererGeneration,
              scope.target.room_id,
              activeOverlay.document,
              revision
            );
          } else {
            await api.setThreadComposerDraft(
              account,
              admitted.lease.leaseId,
              admitted.lease.rendererGeneration,
              scope.target.room_id,
              scope.target.root_event_id,
              activeOverlay.document,
              revision
            );
          }
        } catch {
          settleComposerOperation(admitted);
          return false;
        }
        settleComposerOperation(admitted);
        if (overlayForScope() !== activeOverlay) continue;
        registry.setActiveOverlay(scope, null, null);
        if (kind === "main") mainComposerOverlayRef.current = null;
        else threadComposerOverlayRef.current = null;
        break;
      }
    }
    await registry.deactivate(scope);
    return true;
  }

  async function drainActiveComposerScopesForNavigation(
    includeMain: boolean,
    includeThread: boolean
  ): Promise<boolean> {
    const account = readyComposerDraftAccountOwner(snapshot);
    if (!account) return true;
    const drains: Promise<boolean>[] = [];
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (includeMain && roomId) {
      drains.push(
        deactivateComposerScopeForNavigation(
          composerDraftScope(account, { kind: "main", room_id: roomId }),
          "main"
        )
      );
    }
    const thread = snapshot?.state.ui.thread;
    if (
      includeThread &&
      thread?.kind === "open" &&
      thread.room_id &&
      thread.root_event_id
    ) {
      drains.push(
        deactivateComposerScopeForNavigation(
          composerDraftScope(account, {
            kind: "thread",
            room_id: thread.room_id,
            root_event_id: thread.root_event_id
          }),
          "thread"
        )
      );
    }
    return (await Promise.all(drains)).every(Boolean);
  }

  const invalidatePeoplePanelForNavigation = useCallback((): void => {
    roomSettingsRequestRef.current += 1;
    roomSettingsLoadRef.current = null;
    roomNavigationRequestRef.current += 1;
    spaceNavigationRequestRef.current += 1;
    spaceSettingsRequestRef.current += 1;
    spaceSettingsLoadRef.current = null;
    spaceMembersOpenRequestRef.current += 1;
    spaceMembersInviteRequestRef.current += 1;
    spaceMembersCancelRequestRef.current += 1;
    setPeoplePanelScope(null);
    setSelectedProfileUserId(null);
    setRightPanelMode((mode) =>
      mode === "people" || mode === "profile" ? "closed" : mode
    );
  }, []);

  const openHomeSelection = useCallback(
    async (selection = homeSelection, trigger: ActivityOpenTrigger = "other") => {
    const transitionStartedAt = Date.now();
    const homeSelectionKind = selection.kind;
    const currentSnapshot = snapshotRef.current;
    invalidatePeoplePanelForNavigation();
    const navigationRequestId = ++spaceNavigationRequestRef.current;
    setContextMenu(null);
    if (selection.kind === "activity") {
      const activity = currentSnapshot?.state.domain.activity;
      const previousTab =
        activity?.kind === "open"
          ? activity.active_tab
          : activity?.kind === "opening"
            ? activity.tab
            : activity?.last_selected_tab ?? "none";
      appendDiagnosticLog({
        timestampMs: transitionStartedAt,
        source: "activity.transition",
        message: `stage=open_requested trigger=${trigger} previous_view=${primaryView} previous_lifecycle=${activity?.kind ?? "unknown"} previous_tab=${previousTab}`
      });
    }
    appendDiagnosticLog({
      timestampMs: transitionStartedAt,
      source: "home.transition",
      message: `stage=submit selection=${homeSelectionKind} current_active_room_present=${Boolean(currentSnapshot?.state.ui.navigation.active_room_id)} current_timeline_present=${Boolean(currentSnapshot?.state.ui.timeline.room_id)}`
    });
    const composerDrainStartedAt = Date.now();
    if (!(await drainActiveComposerScopesForNavigation(true, true))) {
      const composerDrainFinishedAt = Date.now();
      appendDiagnosticLog({
        timestampMs: composerDrainFinishedAt,
        source: "home.transition",
        message: `stage=after_composer_drain elapsed_ms=${composerDrainFinishedAt - composerDrainStartedAt} outcome=blocked`
      });
      return;
    }
    const composerDrainFinishedAt = Date.now();
    appendDiagnosticLog({
      timestampMs: composerDrainFinishedAt,
      source: "home.transition",
      message: `stage=after_composer_drain elapsed_ms=${composerDrainFinishedAt - composerDrainStartedAt} outcome=continue`
    });
    const homeSnapshot = await api.selectSpace(null);
    if (spaceNavigationRequestRef.current !== navigationRequestId) {
      return;
    }
    const selectSpaceFinishedAt = Date.now();
    appendDiagnosticLog({
      timestampMs: selectSpaceFinishedAt,
      source: "home.transition",
      message: `stage=after_select_space elapsed_ms_since_start=${selectSpaceFinishedAt - transitionStartedAt} active_room_present=${Boolean(homeSnapshot.state.ui.navigation.active_room_id)} timeline_present=${Boolean(homeSnapshot.state.ui.timeline.room_id)}`
    });
    if (selection.kind === "dm") {
      const room = homeSnapshot.state.domain.rooms.find(
        (candidate) => candidate.room_id === selection.roomId && candidate.is_dm
      );
      if (room) {
        await selectRoom(selection.roomId);
        return;
      }
    }
    if (selection.kind === "explore") {
      setSnapshot(homeSnapshot);
      setPrimaryView("explore");
      const viewAppliedAt = Date.now();
      appendDiagnosticLog({
        timestampMs: viewAppliedAt,
        source: "home.transition",
        message: `stage=after_view_apply elapsed_ms_since_start=${viewAppliedAt - transitionStartedAt} view=explore`
      });
      return;
    }
    if (selection.kind === "invites") {
      setSnapshot(homeSnapshot);
      setPrimaryView("invites");
      const viewAppliedAt = Date.now();
      appendDiagnosticLog({
        timestampMs: viewAppliedAt,
        source: "home.transition",
        message: `stage=after_view_apply elapsed_ms_since_start=${viewAppliedAt - transitionStartedAt} view=invites`
      });
      return;
    }
    setSnapshot(await api.openActivity());
    setPrimaryView("activity");
    const viewAppliedAt = Date.now();
    appendDiagnosticLog({
      timestampMs: viewAppliedAt,
      source: "home.transition",
      message: `stage=after_view_apply elapsed_ms_since_start=${viewAppliedAt - transitionStartedAt} view=activity`
    });
    },
    [appendDiagnosticLog, homeSelection, invalidatePeoplePanelForNavigation, primaryView, setSnapshot]
  );

  async function selectSpace(spaceId: string | null) {
    setContextMenu(null);
    invalidatePeoplePanelForNavigation();
    if (spaceId === null) {
      await openHomeActivityView("home_rail");
      return;
    }
    const navigationRequestId = ++spaceNavigationRequestRef.current;
    if (!(await drainActiveComposerScopesForNavigation(true, true))) return;
    if (spaceNavigationRequestRef.current !== navigationRequestId) return;
    setPrimaryView("timeline");
    const nextSnapshot = await api.selectSpace(spaceId);
    if (spaceNavigationRequestRef.current !== navigationRequestId) return;
    setSnapshot(nextSnapshot);
  }

  async function reorderSpaces(spaceIds: string[]) {
    setSnapshot(await api.reorderSpaces(spaceIds));
  }

  async function selectRoom(roomId: string): Promise<boolean> {
    setContextMenu(null);
    const transitionStartedAt = Date.now();
    const selectedRoom = snapshot?.state.domain.rooms.find((room) => room.room_id === roomId);
    const previousActiveRoomId = snapshot?.state.ui.navigation.active_room_id ?? null;
    if (previousActiveRoomId !== roomId) {
      invalidatePeoplePanelForNavigation();
    }
    const navigationRequestId = ++roomNavigationRequestRef.current;
    appendDiagnosticLog({
      timestampMs: transitionStartedAt,
      source: "room.transition",
      message: `stage=select_start current_active=${Boolean(previousActiveRoomId)} target_known=${Boolean(selectedRoom)} same_active=${previousActiveRoomId === roomId}`
    });
    if (snapshot?.sidebar.account_home.is_active && selectedRoom?.is_dm) {
      setHomeSelection({ kind: "dm", roomId });
    }
    if (previousActiveRoomId !== roomId) {
      const composerDrainStartedAt = Date.now();
      appendDiagnosticLog({
        timestampMs: composerDrainStartedAt,
        source: "room.transition",
        message: `stage=before_composer_drain include_main=true include_thread=true current_timeline_present=${Boolean(snapshot?.state.ui.timeline.room_id)} thread_open=${snapshot?.state.ui.thread.kind === "open"} elapsed_ms_since_start=${composerDrainStartedAt - transitionStartedAt}`
      });
      if (!(await drainActiveComposerScopesForNavigation(true, true))) {
        appendDiagnosticLog({
          timestampMs: Date.now(),
          source: "room.transition",
          message: `stage=after_composer_drain elapsed_ms=${Date.now() - composerDrainStartedAt} outcome=blocked elapsed_ms_since_start=${Date.now() - transitionStartedAt}`
        });
        return false;
      }
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "room.transition",
        message: `stage=after_composer_drain elapsed_ms=${Date.now() - composerDrainStartedAt} outcome=continue elapsed_ms_since_start=${Date.now() - transitionStartedAt}`
      });
    }
    if (roomNavigationRequestRef.current !== navigationRequestId) {
      return false;
    }
    const primaryViewUpdateStartedAt = Date.now();
    appendDiagnosticLog({
      timestampMs: primaryViewUpdateStartedAt,
      source: "room.transition",
      message: `stage=before_primary_view_update previous_view=${primaryView} next_view=timeline elapsed_ms_since_start=${primaryViewUpdateStartedAt - transitionStartedAt}`
    });
    setPrimaryView("timeline");
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=after_primary_view_update elapsed_ms=${Date.now() - primaryViewUpdateStartedAt} elapsed_ms_since_start=${Date.now() - transitionStartedAt}`
    });
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=before_api_select elapsed_ms_since_start=${Date.now() - transitionStartedAt}`
    });
    const nextSnapshot = await api.selectRoom(roomId);
    if (roomNavigationRequestRef.current !== navigationRequestId) {
      return false;
    }
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=after_api_select elapsed_ms=${Date.now() - transitionStartedAt} committed_active=${nextSnapshot.state.ui.navigation.active_room_id === roomId} timeline_matches=${nextSnapshot.state.ui.timeline.room_id === nextSnapshot.state.ui.navigation.active_room_id}`
    });
    setSnapshot(nextSnapshot);
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=after_snapshot_apply elapsed_ms_since_start=${Date.now() - transitionStartedAt}`
    });
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=select_done active_changed=${nextSnapshot.state.ui.navigation.active_room_id !== previousActiveRoomId} timeline_matches=${nextSnapshot.state.ui.timeline.room_id === nextSnapshot.state.ui.navigation.active_room_id}`
    });
    return true;
  }

  async function openDmUserInfo(roomId: string, userId: string) {
    if (!(await selectRoom(roomId))) {
      return;
    }
    const navigationRequestId = roomNavigationRequestRef.current;
    roomSettingsLoadRef.current = null;
    const settingsRequestId = ++roomSettingsRequestRef.current;
    const next = await api.loadRoomSettings(roomId);
    const isCurrent = () =>
      roomNavigationRequestRef.current === navigationRequestId &&
      roomSettingsRequestRef.current === settingsRequestId;
    if (
      !isCurrent() ||
      next.state.ui.navigation.active_room_id !== roomId ||
      !exactRoomSettingsForRoom(next, roomId)
    ) {
      return;
    }
    setSnapshot(next);
    setPeoplePanelScope({ kind: "room", roomId });
    setSelectedProfileUserId(userId);
    await setRightPanelModeClosingFocusedContext("profile", isCurrent);
  }

  async function openHomeActivityView(trigger: ActivityOpenTrigger = "activity_sidebar") {
    setHomeSelection({ kind: "activity" });
    await openHomeSelection({ kind: "activity" }, trigger);
  }

  async function openHomeExploreView() {
    setHomeSelection({ kind: "explore" });
    await openHomeSelection({ kind: "explore" });
  }

  async function openHomeInvitesView() {
    setHomeSelection({ kind: "invites" });
    await openHomeSelection({ kind: "invites" });
  }

  useEffect(() => {
    if (
      initialHomeSelectionApplied.current ||
      !snapshot ||
      snapshot.state.domain.session.kind !== "ready" ||
      !snapshot.sidebar.account_home.is_active ||
      snapshot.state.ui.navigation.active_space_id !== null ||
      snapshot.state.ui.navigation.active_room_id !== null
    ) {
      return;
    }
    initialHomeSelectionApplied.current = true;
    void openHomeSelection(homeSelection, "initial_home");
  }, [
    homeSelection,
    openHomeSelection,
    snapshot?.sidebar.account_home.is_active,
    snapshot?.state.ui.navigation.active_room_id,
    snapshot?.state.domain.session.kind,
    snapshot?.state.ui.navigation.active_space_id
  ]);

  async function openInvitesView() {
    setSnapshot(await api.getSnapshot());
    setPrimaryView("invites");
  }

  async function openExploreView() {
    setSnapshot(await api.getSnapshot());
    setPrimaryView("explore");
  }

  async function closeActivityView() {
    setSnapshot(await api.closeActivity());
    setPrimaryView("timeline");
  }

  async function setActivityTab(tab: ActivityTab) {
    setSnapshot(await api.setActivityTab(tab));
  }

  async function paginateActivity(tab: ActivityTab, cursor: string | null) {
    setSnapshot(await api.paginateActivity(tab, cursor));
  }

  async function retryActivityResolution() {
    setSnapshot(await api.retryActivityResolution());
  }

  async function markActivityRead(target: ActivityMarkReadTarget) {
    setSnapshot(await api.markActivityRead(target));
  }

  async function queryDirectory(termOverride?: string) {
    if (isBusy) {
      return;
    }
    const term = (termOverride ?? directorySearchDraft).trim();
    setSnapshot(
      await api.queryDirectory({
        term: term || null,
        server_name: directoryServerDraft.trim() || null,
        limit: 20,
        since: null
      })
    );
  }

  /**
   * Submit the Explore search field.
   *
   * A full address pasted here still routes to preview rather than returning
   * "no public rooms found": a directory text search cannot find a room
   * addressed by id at all, so refusing it would only punish the paste (#330).
   */
  async function submitDirectorySearch() {
    if (isBusy) {
      return;
    }
    const submission = resolveDirectorySubmission(directorySearchDraft);
    switch (submission.kind) {
      case "empty":
        return;
      case "join":
        setDirectoryAddressNotice(null);
        await previewJoinTarget(submission.roomIdOrAlias, submission.viaServers);
        return;
      case "user":
        setDirectoryAddressNotice("user");
        return;
      case "search":
        setDirectoryAddressNotice(null);
        await queryDirectory(submission.term);
    }
  }

  /**
   * Submit the Explore address field.
   *
   * Shares `resolveDirectorySubmission` with the search field so the two entry
   * points cannot classify the same string differently. Ordinary words are not
   * an address, and a user id is not joinable, so both are explained rather
   * than silently ignored.
   */
  async function submitDirectoryAddress() {
    if (isBusy) {
      return;
    }
    const submission = resolveDirectorySubmission(directoryAddressDraft);
    switch (submission.kind) {
      case "empty":
        return;
      case "join":
        setDirectoryAddressNotice(null);
        await previewJoinTarget(submission.roomIdOrAlias, submission.viaServers);
        return;
      case "user":
        setDirectoryAddressNotice("user");
        return;
      case "search":
        setDirectoryAddressNotice("notRecognized");
    }
  }

  /**
   * Show what a target actually is before joining it.
   *
   * Every way of naming a room — a link, a typed id, a directory result — comes
   * through here, so the user never joins something sight unseen and the two
   * entry points cannot drift apart.
   */
  async function previewJoinTarget(roomIdOrAlias: string, viaServers: string[]) {
    const directory = snapshot?.state.domain.directory;
    if (directory?.join.kind === "joining" || directory?.preview.kind === "loading") {
      return;
    }
    const joined = snapshot?.state.domain.rooms.find(
      (room) => room.room_id === roomIdOrAlias
    );
    if (joined) {
      await selectRoom(joined.room_id);
      return;
    }
    setSnapshot(await api.previewJoinTarget(roomIdOrAlias, viaServers));
  }

  /** Join the previewed room, reusing exactly the target that resolved it. */
  async function confirmDirectoryJoin() {
    const preview = snapshot?.state.domain.directory.preview;
    if (preview?.kind !== "ready") {
      return;
    }
    // A room already in the list is navigation. A preview that reports
    // membership the room list has not caught up to still goes through join,
    // which is idempotent and is what makes the local state catch up.
    const joined = snapshot?.state.domain.rooms.find(
      (room) => room.room_id === preview.room.room_id
    );
    if (joined) {
      await dismissDirectoryPreview();
      await selectRoom(joined.room_id);
      return;
    }
    const nextSnapshot = await api.joinDirectoryRoom(
      preview.room_id_or_alias,
      preview.via_servers
    );
    setPrimaryView("timeline");
    setSnapshot(nextSnapshot);
  }

  async function dismissDirectoryPreview() {
    setSnapshot(await api.dismissDirectoryPreview());
  }

  /**
   * Open a Matrix entity a message linked to.
   *
   * An already-joined room is plain navigation. Anything else is handed to the
   * Rust-owned directory preview/join state machine by seeding Explore with the
   * target, so a linked room uses exactly the same path as a searched one
   * instead of growing a second join flow.
   */
  async function openMatrixTarget(target: MatrixPermalinkTarget) {
    if (target.kind !== "room") {
      return;
    }
    const joined = snapshot?.state.domain.rooms.find(
      (room) => room.room_id === target.roomIdOrAlias
    );
    if (joined) {
      await selectRoom(joined.room_id);
      return;
    }
    setDirectorySearchDraft(target.roomIdOrAlias);
    await previewJoinTarget(target.roomIdOrAlias, target.viaServers);
  }

  async function joinDirectoryRoom(room: DirectoryRoomSummary) {
    if (isBusy) {
      return;
    }
    // A public space frequently has no canonical alias, so fall back to the
    // room id rather than leaving the result findable but unjoinable.
    const alias = room.canonical_alias?.trim() || null;
    const target = alias ?? room.room_id;
    const viaServer = serverNameFromMatrixId(target);
    await previewJoinTarget(target, viaServer ? [viaServer] : []);
  }

  function openCreateDialog(kind: "room" | "space") {
    setCreateDraftName("");
    setCreateRoomDraftOptions(defaultCreateRoomDialogOptions());
    setCreateDialog(kind);
  }

  function closeCreateDialog() {
    setCreateDialog(null);
    setCreateDraftName("");
    setCreateRoomDraftOptions(defaultCreateRoomDialogOptions());
  }

  function openNewDmDialog() {
    setNewDmDraftUserId("");
    setNewDmDialogOpen(true);
  }

  function closeNewDmDialog() {
    setNewDmDialogOpen(false);
    setNewDmDraftUserId("");
  }

  async function openInviteUserDialog(roomId: string, title: string) {
    setInviteUserDraftQuery("");
    setInviteUserDialog({ roomId, title });
    setInviteUserDialogVisible(true);
    const settingsSnapshot = await api.loadRoomSettings(roomId);
    setSnapshot(settingsSnapshot);
    const nextSnapshot = await api.openInviteWorkflow(roomId);
    setInviteUserDraftQuery(nextSnapshot.state.domain.invite_workflow?.query.query ?? "");
    setSnapshot(nextSnapshot);
  }

  async function closeInviteUserDialog() {
    setInviteUserDialog(null);
    setInviteUserDialogVisible(false);
    setInviteUserDraftQuery("");
    setSnapshot(await api.closeInviteWorkflow());
  }

  async function openRoomInfoFromInvite() {
    if (!inviteUserDialog) {
      return;
    }
    setInviteUserDialogVisible(false);
    await setRightPanelModeClosingFocusedContext("roomInfo");
  }

  async function openRecoveryFromInvite() {
    if (!inviteUserDialog) {
      return;
    }
    setInviteUserDialogVisible(false);
    await setRightPanelModeClosingFocusedContext("recovery");
  }

  async function returnToInviteUserDialog() {
    const dialog = inviteUserDialog;
    if (!dialog) {
      return;
    }
    const settingsSnapshot = await api.loadRoomSettings(dialog.roomId);
    setSnapshot(settingsSnapshot);
    const nextSnapshot = await api.openInviteWorkflow(dialog.roomId);
    const workflow = nextSnapshot.state.domain.invite_workflow ?? DEFAULT_INVITE_WORKFLOW;
    setInviteUserDraftQuery(workflow.query.query);
    setInviteUserDialogVisible(true);
    setSnapshot(nextSnapshot);
    await setRightPanelModeClosingFocusedContext("closed");
  }

  async function updateInviteUserQuery(value: string) {
    const dialog = inviteUserDialog;
    setInviteUserDraftQuery(value);
    if (!dialog) {
      return;
    }
    const nextSnapshot = await api.searchInviteTargets(dialog.roomId, value);
    setSnapshot(nextSnapshot);
  }

  async function selectInviteScope(scope: InviteScopeSelection) {
    const dialog = inviteUserDialog;
    if (!dialog) {
      return;
    }
    setSnapshot(await api.setInviteScope(dialog.roomId, scope));
  }

  async function selectInviteTarget(userId: string) {
    const dialog = inviteUserDialog;
    if (!dialog) {
      return;
    }
    setSnapshot(await api.selectInviteTarget(dialog.roomId, userId));
  }

  async function removeInviteTarget(userId: string) {
    setSnapshot(await api.removeInviteTarget(userId));
  }

  async function acceptInvite(roomId: string) {
    if (isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      const nextSnapshot = await api.acceptInvite(roomId);
      setSnapshot(nextSnapshot);
      if (nextSnapshot.state.domain.rooms.some((room) => room.room_id === roomId)) {
        await selectRoom(roomId);
      }
      setPrimaryView("timeline");
    } finally {
      setIsBusy(false);
    }
  }

  async function declineInvite(roomId: string) {
    if (isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      setSnapshot(await api.declineInvite(roomId));
    } finally {
      setIsBusy(false);
    }
  }

  async function joinRoom(roomId: string) {
    const trimmedRoomId = roomId.trim();
    if (!trimmedRoomId || isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      const nextSnapshot = await api.joinRoom(trimmedRoomId);
      setSnapshot(nextSnapshot);
      if (nextSnapshot.state.domain.rooms.some((room) => room.room_id === trimmedRoomId)) {
        await selectRoom(trimmedRoomId);
      }
      setPrimaryView("timeline");
    } finally {
      setIsBusy(false);
    }
  }

  async function submitNewDmDialog() {
    const userId = newDmDraftUserId.trim();
    if (!userId || isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      setSnapshot(await api.startDirectMessage(userId));
      closeNewDmDialog();
      setPrimaryView("timeline");
    } finally {
      setIsBusy(false);
    }
  }

  async function startDirectMessage(userId: string) {
    const trimmedUserId = userId.trim();
    if (!trimmedUserId || isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      setSnapshot(await api.startDirectMessage(trimmedUserId));
      setPrimaryView("timeline");
      await setRightPanelModeClosingFocusedContext("closed");
    } finally {
      setIsBusy(false);
    }
  }

  async function submitInviteUserDialog() {
    const dialog = inviteUserDialog;
    const workflow = snapshot?.state.domain.invite_workflow ?? DEFAULT_INVITE_WORKFLOW;
    const userIds = workflow.selected_targets.map((target) => target.user_id);
    if (!dialog || userIds.length === 0 || isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      const scope = workflow.selected_scope ?? inviteScopeFromWorkflow(workflow);
      const nextSnapshot = await api.inviteTargets(dialog.roomId, userIds, scope);
      setSnapshot(nextSnapshot);
      const operation = nextSnapshot.state.domain.invite_workflow?.operation;
      const hasNotice = operation?.kind === "completed" && operation.notice;
      const hasFailedResult =
        operation?.kind === "completed" &&
        operation.results.some((result) => result.kind === "failed");
      if (!hasNotice && !hasFailedResult) {
        await closeInviteUserDialog();
      }
    } finally {
      setIsBusy(false);
    }
  }

  async function submitCreateDialog() {
    const kind = createDialog;
    const name = createDraftName.trim();
    const activeSpaceIdForCreatedRoom =
      kind === "room" ? snapshot?.state.ui.navigation.active_space_id ?? null : null;
    // Guard against double-submit: a create already in flight (isBusy) or a
    // pending basic_operation (Rust-owned) must block re-entry.
    if (
      !kind ||
      !name ||
      (kind === "room" &&
        createRoomDraftOptions.visibility === "public" &&
        !createRoomDraftOptions.aliasLocalpart.trim()) ||
      isBusy ||
      (snapshot && snapshot.state.ui.basic_operation.kind !== "idle")
    ) {
      return;
    }
    setIsBusy(true);
    try {
      const createRoomRequest =
        kind === "room"
          ? createRoomRequestFromDraft(name, createRoomDraftOptions, activeSpaceIdForCreatedRoom)
          : null;
      const nextSnapshot =
        kind === "space" ? await api.createSpace(name) : await api.createRoom(createRoomRequest!);
      setSnapshot(nextSnapshot);
      closeCreateDialog();
    } finally {
      setIsBusy(false);
    }
  }

  async function setComposerReplyTarget(roomId: string, eventId: string) {
    setSnapshot(await api.setComposerReplyTarget(roomId, eventId));
  }

  async function cancelComposerReply() {
    setSnapshot(await api.cancelComposerReply());
  }

  function beginComposerOperation(scope: ComposerDraftScope): {
    capture: ComposerDraftOperationCapture;
    lease: NonNullable<ReturnType<ComposerDraftLifecycleRegistry["snapshot"]>>;
  } | null {
    const lease = composerDraftLifecycleRegistryRef.current!.snapshot(scope);
    if (!lease?.leaseId) return null;
    return {
      capture: composerDraftLifecycleRegistryRef.current!.beginOperation(scope),
      lease
    };
  }

  function composerOperationCanApply(
    admitted: {
      capture: ComposerDraftOperationCapture;
      lease: NonNullable<ReturnType<ComposerDraftLifecycleRegistry["snapshot"]>>;
    },
    capturedRevision: ComposerDraftRevision
  ): boolean {
    return composerDraftLifecycleRegistryRef.current!.settleOperationCompletion(
      admitted.capture,
      admitted.lease.leaseId,
      capturedRevision
    );
  }

  function settleComposerOperation(admitted: {
    capture: ComposerDraftOperationCapture;
  }): void {
    const { capture } = admitted;
    composerDraftLifecycleRegistryRef.current!.settleOperation(capture);
  }

  function reserveComposerAcceptedRevision(
    admitted: { capture: ComposerDraftOperationCapture },
    draftRevision: ComposerDraftRevision
  ): boolean {
    try {
      composerDraftLifecycleRegistryRef.current!.reserveAcceptedRevision(
        admitted.capture,
        draftRevision
      );
      return true;
    } catch {
      settleComposerOperation(admitted);
      return false;
    }
  }

  function currentComposerDraftRevision(
    scope: ComposerDraftScope,
    lease: NonNullable<ReturnType<ComposerDraftLifecycleRegistry["snapshot"]>>
  ): ComposerDraftRevision {
    return composerDraftLifecycleRegistryRef.current!.snapshot(scope)?.revision ?? lease.revision;
  }

  /**
   * Sends the staged attachments only.
   *
   * Deliberately separate from `sendText`: the composer draft is never read,
   * sent, or cleared here, just as `sendText` never dispatches attachments.
   */
  async function sendStagedAttachments() {
    if (stagedUploadSendInFlightRef.current) {
      return;
    }
    stagedUploadSendInFlightRef.current = true;
    try {
      await sendStagedAttachmentsInner();
    } finally {
      stagedUploadSendInFlightRef.current = false;
    }
  }

  async function sendStagedAttachmentsInner() {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const account = readyComposerDraftAccountOwner(snapshot);
    const accountOwner = account ? composerDraftAccountOwnerKey(account) : null;
    const target: ComposerTarget | null = roomId ? { kind: "main", room_id: roomId } : null;
    const uploads = snapshot?.state.ui.timeline.staged_uploads ?? [];
    if (!roomId || !target || !account || !accountOwner || uploads.length === 0) {
      return;
    }
    if (!uploadStagingItemsAreSendable(uploads)) {
      return;
    }
    const scope = composerDraftScope(account, target);
    const admitted = beginComposerOperation(scope);
    if (!admitted) return;
    const draftRevision = currentComposerDraftRevision(scope, admitted.lease);
    if (!reserveComposerAcceptedRevision(admitted, draftRevision)) return;
    const localRevisionAtSubmission = mainComposerOverlayRef.current?.revision;
    for (const item of uploads) {
      latestTextMutationQueueRef.current.invalidate(
        `caption:main:${roomId}:${item.staged_id}`
      );
    }
    let response;
    try {
      response = await api.sendPreparedUploads(
        account,
        admitted.lease.leaseId,
        admitted.lease.rendererGeneration,
        target,
        draftRevision
      );
    } catch {
      settleComposerOperation(admitted);
      return;
    }
    const canApply = composerOperationCanApply(admitted, draftRevision);
    if (!canApply || submissionAccountOwnerRef.current !== accountOwner) return;
    const accepted =
      response.acceptedRevision !== null &&
      compareComposerDraftRevisions(response.acceptedRevision, draftRevision) > 0;
    const hasNewerDraft =
      mainComposerOverlayRef.current?.revision !== localRevisionAtSubmission;
    if (accepted && !hasNewerDraft) {
      cancelComposerDraftPersist(scope);
      clearLocalComposerDraft(scope);
      updateComposerTypingSignal(roomId, "");
    }
    setSnapshot(response.snapshot);
  }

  async function sendText(documentOverride?: ComposerDocument) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const sendDocument = documentOverride ?? composerDocument;
    const body = plainBodyFromDocument(sendDocument);
    const account = readyComposerDraftAccountOwner(snapshot);
    const accountOwner = account ? composerDraftAccountOwnerKey(account) : null;
    const target: ComposerTarget | null = roomId ? { kind: "main", room_id: roomId } : null;
    appendComposerSubmitDiagnostic(
      "main",
      "received",
      `room_present=${Boolean(roomId)} account_ready=${Boolean(account && accountOwner)} body_present=${Boolean(body.trim())}`
    );
    // Text only. Staged attachments have their own send in the staging panel,
    // so this never dispatches them and never leaves the draft silently unsent.
    if (!roomId || !target || !account || !accountOwner || !body.trim()) {
      const reason = !roomId
        ? "room_missing"
        : !account || !accountOwner
          ? "account_not_ready"
          : "empty_body";
      appendComposerSubmitDiagnostic("main", "blocked", `reason=${reason}`);
      return;
    }
    // Reply semantics are Rust-owned: dispatch sendReply when the composer is
    // in reply mode, otherwise plain sendText.
    const composerMode = snapshot?.state.ui.timeline.composer.mode ?? "Plain";
    const submissionController = submissionRegistryRef.current!.forTarget(mainSubmissionTarget(roomId));
    const submissionId = submissionController.begin();
    if (submissionId === null) {
      appendComposerSubmitDiagnostic("main", "blocked", "reason=submission_in_progress");
      return;
    }

    if (submissionController.payload(submissionId) === undefined) {
      const scope = composerDraftScope(account, target);
      const draftRevision =
        composerDraftLifecycleRegistryRef.current!.snapshot(scope)?.revision ??
        COMPOSER_DRAFT_REVISION_ZERO;
      const localRevisionAtSubmission = mainComposerOverlayRef.current?.revision ?? null;
      submissionController.capture(submissionId, {
        roomId,
        body,
        document: sendDocument,
        composerMode,
        draftRevision,
        localRevisionAtSubmission,
        account,
        accountOwner,
        scope
      });
    }
    const captured = submissionController.payload<{
      roomId: string;
      body: string;
      document: ComposerDocument;
      composerMode: typeof composerMode;
      draftRevision: ComposerDraftRevision;
      localRevisionAtSubmission: ComposerDraftRevision | null;
      account: { homeserver: string; userId: string; deviceId: string };
      accountOwner: string;
      scope: ComposerDraftScope;
    }>(submissionId)!;

    qaSendStarted.current = true;
    qaSendBaselineErrorCount.current = snapshot?.state.ui.errors.length ?? 0;
    qaSendBaselineTimelineItems.current = snapshot?.timeline.length ?? 0;
    qaSendPending.current = true;
    setQaSendStatus("pending");
    if (snapshot) {
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "e2ee.send",
        message: e2eeSendDiagnosticMessage(snapshot, roomId)
      });
    }
    const admitted = beginComposerOperation(captured.scope);
    if (!admitted) {
      appendComposerSubmitDiagnostic("main", "blocked", "reason=draft_operation_not_admitted");
      submissionController.reject(submissionId);
      return;
    }
    if (!reserveComposerAcceptedRevision(admitted, captured.draftRevision)) {
      appendComposerSubmitDiagnostic("main", "blocked", "reason=draft_revision_not_reserved");
      submissionController.reject(submissionId);
      return;
    }
    try {
      appendComposerSubmitDiagnostic(
        "main",
        "dispatch",
        `mode=${captured.composerMode === "Plain" ? "plain" : "reply"}`
      );
      const nextSnapshot =
        captured.composerMode === "Plain"
          ? await api.sendText(
              captured.account,
              admitted.lease.leaseId,
              admitted.lease.rendererGeneration,
              submissionId,
              captured.roomId,
              captured.document,
              captured.draftRevision
            )
          : await api.sendReply(
              captured.account,
              admitted.lease.leaseId,
              admitted.lease.rendererGeneration,
              submissionId,
              captured.roomId,
              captured.composerMode.Reply.in_reply_to_event_id,
              captured.document,
              captured.draftRevision
            );
      const canApply = composerOperationCanApply(admitted, captured.draftRevision);
      if (!canApply || submissionAccountOwnerRef.current !== captured.accountOwner) {
        appendComposerSubmitDiagnostic("main", "settled", "outcome=stale_context_ignored");
        return;
      }
      if (nextSnapshot.submissionId !== submissionId || nextSnapshot.outcome !== "accepted") {
        appendComposerSubmitDiagnostic("main", "settled", "outcome=rejected_by_backend");
        submissionController.reject(submissionId);
        setSnapshot(nextSnapshot.snapshot);
        return;
      }
      submissionController.accept(submissionId);
      appendComposerSubmitDiagnostic("main", "settled", "outcome=accepted");
      const hasNewerDraft =
        mainComposerOverlayRef.current?.revision !== captured.localRevisionAtSubmission;
      if (!hasNewerDraft) {
        cancelComposerDraftPersist(captured.scope);
        clearLocalComposerDraft(captured.scope);
        updateComposerTypingSignal(roomId, "");
      }
      setSnapshot(nextSnapshot.snapshot);
      if (!isTauriRuntime()) {
        const completionStatus = qaSendSmokeCompletionStatus(
          nextSnapshot.snapshot,
          qaSendBaselineErrorCount.current,
          qaSendBaselineTimelineItems.current
        );
        qaSendPending.current = completionStatus === "pending";
        setQaSendStatus(completionStatus);
      }
    } catch (error) {
      settleComposerOperation(admitted);
      if (submissionAccountOwnerRef.current !== captured.accountOwner) {
        return;
      }
      const disposition = classifySubmissionFailure(error);
      appendComposerSubmitDiagnostic(
        "main",
        "settled",
        `outcome=failed failure_class=${disposition.kind}`
      );
      if (disposition.kind === "unknown") {
        submissionController.markUnknown(submissionId, disposition.reason);
      } else {
        submissionController.reject(submissionId);
      }
      qaSendPending.current = false;
      setQaSendStatus("failed");
      return;
    }
  }

  useEffect(() => {
    if (submissionRegistryRef.current && snapshot) {
      reconcileComposerSubmissionSnapshot(
        submissionRegistryRef.current,
        snapshot.state.ui.timeline
      );
    }
  }, [snapshot]);

  async function scheduleSend(sendAtMs: number, documentOverride?: ComposerDocument) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const body = plainBodyFromDocument(documentOverride ?? composerDocument);
    const account = readyComposerDraftAccountOwner(snapshot);
    const accountOwner = account ? composerDraftAccountOwnerKey(account) : null;
    if (!roomId || !account || !accountOwner || !body.trim() || stagedUploads.length > 0) {
      return;
    }

    const target: ComposerTarget = { kind: "main", room_id: roomId };
    const scope = composerDraftScope(account, target);
    const admitted = beginComposerOperation(scope);
    if (!admitted) return;
    const draftRevision = currentComposerDraftRevision(scope, admitted.lease);
    if (!reserveComposerAcceptedRevision(admitted, draftRevision)) return;
    const localRevisionAtSubmission = mainComposerOverlayRef.current?.revision ?? null;
    try {
      const response = await api.scheduleSend(
        account,
        admitted.lease.leaseId,
        admitted.lease.rendererGeneration,
        target,
        body,
        sendAtMs,
        draftRevision
      );
      const canApply = composerOperationCanApply(admitted, draftRevision);
      if (!canApply || submissionAccountOwnerRef.current !== accountOwner) return;
      const accepted =
        response.acceptedRevision !== null &&
        compareComposerDraftRevisions(response.acceptedRevision, draftRevision) > 0;
      const hasNewerDraft = mainComposerOverlayRef.current?.revision !== localRevisionAtSubmission;
      if (accepted && !hasNewerDraft) {
        cancelComposerDraftPersist(scope);
        clearLocalComposerDraft(scope);
        updateComposerTypingSignal(roomId, "");
      }
      setSnapshot(response.snapshot);
    } catch {
      settleComposerOperation(admitted);
      // Command failures are surfaced through the Rust-owned error/event path.
    }
  }

  async function cancelScheduledSend(scheduledId: string) {
    try {
      setSnapshot(await api.cancelScheduledSend(scheduledId));
    } catch {
      // Command failures are surfaced through the Rust-owned error/event path.
    }
  }

  async function rescheduleScheduledSend(scheduledId: string, body: string, sendAtMs: number) {
    try {
      setSnapshot(await api.rescheduleScheduledSend(scheduledId, body, sendAtMs));
    } catch {
      // Command failures are surfaced through the Rust-owned error/event path.
    }
  }

  function updateComposerDraft(document: ComposerDocument) {
    const value = plainBodyFromDocument(document);
    const roomId = snapshot?.state.ui.timeline.room_id;
    const account = readyComposerDraftAccountOwner(snapshot);
    if (!roomId || !account) return;
    const target: ComposerTarget = { kind: "main", room_id: roomId };
    const scope = composerDraftScope(account, target);
    const lease = composerDraftLifecycleRegistryRef.current!.snapshot(scope);
    const revision = lease?.leaseId
      ? composerDraftLifecycleRegistryRef.current!.nextDraft(scope)
      : null;
    mainComposerOverlayRef.current = {
      scope,
      document,
      revision,
      debounceHandle: null
    };
    composerDraftLifecycleRegistryRef.current!.setActiveOverlay(scope, document, revision);
    updateComposerTypingSignal(roomId, value);
    if (revision) queueComposerDraftPersist(scope, document, revision);
  }

  function updateComposerTypingSignal(roomId: string, value: string) {
    const isTyping = Boolean(value.trim());
    const previous = typingSignalRef.current;
    if (previous.roomId === roomId && previous.isTyping === isTyping) {
      return;
    }
    typingSignalRef.current = { roomId, isTyping };
    void api.setTyping(roomId, isTyping).catch(() => undefined);
  }

  function cancelComposerDraftPersist(scope: ComposerDraftScope) {
    const overlay = mainComposerOverlayRef.current;
    if (!overlay || !composerDraftScopesEqual(overlay.scope, scope)) return;
    if (overlay.debounceHandle !== null) {
      window.clearTimeout(overlay.debounceHandle);
      overlay.debounceHandle = null;
    }
    composerDraftLifecycleRegistryRef.current!.clearDebounce(scope);
  }

  function queueComposerDraftPersist(
    scope: ComposerDraftScope,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ) {
    if (scope.target.kind !== "main") return;
    cancelComposerDraftPersist(scope);
    const handle = window.setTimeout(() => {
      const overlay = mainComposerOverlayRef.current;
      if (overlay && composerDraftScopesEqual(overlay.scope, scope)) {
        overlay.debounceHandle = null;
      }
      composerDraftLifecycleRegistryRef.current!.clearDebounce(scope);
      const admitted = beginComposerOperation(scope);
      if (!admitted) return;
      const account = composerDraftApiAccount(scope);
      void api
        .setComposerDraft(
          account,
          admitted.lease.leaseId,
          admitted.lease.rendererGeneration,
          scope.target.room_id,
          document,
          revision
        )
        .then((nextSnapshot) => {
          const canApply = composerOperationCanApply(admitted, revision);
          const currentOverlay = mainComposerOverlayRef.current;
          if (
            !canApply ||
            submissionAccountOwnerRef.current !== composerDraftAccountOwnerKey(account) ||
            !currentOverlay ||
            !composerDraftScopesEqual(currentOverlay.scope, scope) ||
            currentOverlay.document !== document ||
            currentOverlay.revision !== revision
          ) return;
          setSnapshot(nextSnapshot);
        })
        .catch(() => settleComposerOperation(admitted));
    }, 350);
    const overlay = mainComposerOverlayRef.current;
    if (overlay && composerDraftScopesEqual(overlay.scope, scope)) {
      overlay.debounceHandle = handle;
    }
    composerDraftLifecycleRegistryRef.current!.setDebounce(scope, handle);
  }

  function clearLocalComposerDraft(scope: ComposerDraftScope) {
    const overlay = mainComposerOverlayRef.current;
    if (!overlay || !composerDraftScopesEqual(overlay.scope, scope)) return;
    mainComposerOverlayRef.current = null;
    composerDraftLifecycleRegistryRef.current!.setActiveOverlay(scope, null, null);
  }

  async function stageUploadFiles(files: File[]): Promise<void> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId || files.length === 0) {
      return;
    }
    const target: ComposerTarget = { kind: "main", room_id: roomId };
    let nextSnapshot: DesktopSnapshot | null = null;
    await stageAttachmentFiles(
      target,
      files,
      stagedUploads.length,
      createStagedUploadId,
      async (capturedTarget, items) => {
        nextSnapshot = await api.stageUploadBytes(capturedTarget, items);
      }
    );
    if (nextSnapshot) {
      setSnapshot(nextSnapshot);
    }
  }

  async function updateStagedUploadCaption(stagedId: string, caption: string): Promise<void> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) return;
    await applyLatestTextMutationSnapshot(`caption:main:${roomId}:${stagedId}`, () =>
      api.updateStagedUploadCaption(
        { kind: "main", room_id: roomId },
        stagedId,
        caption
      )
    );
  }

  async function selectStagedUploadOutput(
    stagedId: string,
    selection: StagedUploadOutputSelection
  ): Promise<void> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) return;
    setSnapshot(
      await api.selectStagedUploadOutput(
        { kind: "main", room_id: roomId },
        stagedId,
        selection
      )
    );
  }

  async function loadStagedUploadPreview(
    stagedId: string,
    variantId: string
  ): Promise<number[]> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) return [];
    return api.preparedUploadPreview(
      { kind: "main", room_id: roomId },
      stagedId,
      variantId
    );
  }

  async function retryStagedUploadPreparation(stagedId: string) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) return;
    setSnapshot(
      await api.retryStagedUploadPreparation({ kind: "main", room_id: roomId }, stagedId)
    );
  }

  async function useOriginalStagedUpload(stagedId: string) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) return;
    setSnapshot(await api.useOriginalStagedUpload({ kind: "main", room_id: roomId }, stagedId));
  }

  async function clearUploadStaging(): Promise<void> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) {
      return;
    }
    for (const item of snapshot?.state.ui.timeline.staged_uploads ?? []) {
      latestTextMutationQueueRef.current.invalidate(`caption:main:${roomId}:${item.staged_id}`);
    }
    setSnapshot(await api.clearUploadStaging({ kind: "main", room_id: roomId }));
  }

  async function editMessage(message: { body: string | null; room_id: string; event_id: string }) {
    const body = window.prompt(t("timeline.editMessage"), message.body ?? undefined);
    if (body === null || !body.trim()) {
      return;
    }

    setSnapshot(
      await api.editMessage(message.room_id, message.event_id, documentFromText(body))
    );
  }

  async function redactMessage(roomId: string, eventId: string) {
    setSnapshot(await api.redactMessage(roomId, eventId));
  }

  async function unpinPinnedEvent(roomId: string, eventId: string) {
    setSnapshot(await api.unpinEvent(roomId, eventId));
  }

  async function updateRoomSetting(roomId: string, change: RoomSettingChange) {
    setSnapshot(await api.updateRoomSetting(roomId, change));
  }

  async function moderateRoomMember(
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null = null
  ) {
    setSnapshot(await api.moderateRoomMember(roomId, targetUserId, action, reason));
  }

  async function updateRoomMemberRole(
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ) {
    setSnapshot(await api.updateRoomMemberRole(roomId, targetUserId, powerLevel));
  }

  async function openThread(
    roomId: string,
    rootEventId: string,
    intent: ThreadOpenIntent
  ) {
    const thread = snapshot?.state.ui.thread;
    if (
      thread?.kind === "open" &&
      (thread.room_id !== roomId || thread.root_event_id !== rootEventId)
    ) {
      if (!(await drainActiveComposerScopesForNavigation(false, true))) return;
    }
    await closeFocusedContextIfHiddenBy("thread");
    setSnapshot(await api.openThread(roomId, rootEventId, intent));
    setRightPanelMode("thread");
  }

  async function closeThread() {
    if (!(await drainActiveComposerScopesForNavigation(false, true))) return;
    setSnapshot(await api.closeThread());
    setRightPanelMode("closed");
  }

  async function openThreadsListPanel(scope: ThreadsListScope) {
    await closeFocusedContextIfHiddenBy("threads");
    setSnapshot(await api.openThreadsList(scope));
    setRightPanelMode("threads");
  }

  async function openPinnedMessagesPanel(roomId: string) {
    if (!roomId) return;
    setPinnedNavigation(null);
    await closeFocusedContextIfHiddenBy("pinned");
    setRightPanelMode("pinned");
  }

  async function openPinnedEvent(
    roomId: string,
    eventId: string,
    threadRootEventId: string | null
  ) {
    setPinnedNavigation({
      room_id: roomId,
      event_id: eventId,
      thread_root_event_id: threadRootEventId,
      status: "loading"
    });
    if (threadRootEventId) {
      try {
        if (snapshot?.state.ui.navigation.active_room_id !== roomId) {
          await selectRoom(roomId);
        }
        await openThread(roomId, threadRootEventId, {
          pinnedReply: { event_id: eventId }
        });
        setPinnedNavigation(null);
      } catch {
        setPinnedNavigation({
          room_id: roomId,
          event_id: eventId,
          thread_root_event_id: threadRootEventId,
          status: "failed"
        });
      }
      return;
    }

    try {
      const nextSnapshot = await api.openPinnedEvent(roomId, eventId);
      setSnapshot(nextSnapshot);
      setPrimaryView("timeline");
      setRightPanelMode("pinned");
      setPinnedNavigation(null);
    } catch {
      setPinnedNavigation({
        room_id: roomId,
        event_id: eventId,
        thread_root_event_id: null,
        status: "failed"
      });
    }
  }

  function retryPinnedEvent(
    roomId: string,
    eventId: string,
    threadRootEventId: string | null
  ) {
    void openPinnedEvent(roomId, eventId, threadRootEventId);
  }

  async function closeThreadsListPanel() {
    setSnapshot(await api.closeThreadsList());
    setRightPanelMode("closed");
  }

  async function paginateThreadsList(scope: ThreadsListScope) {
    setSnapshot(await api.paginateThreadsList(scope));
  }

  async function openFilesView(scope: FilesViewScope) {
    await closeFocusedContextIfHiddenBy("files");
    const filter: AttachmentFilter = { kinds: ["image", "video", "audio", "file"], filename_query: null };
    const sort: AttachmentSort = "newestFirst";
    setSnapshot(await api.openFilesView(scope, filter, sort));
    setRightPanelMode("files");
  }

  async function closeFilesViewPanel() {
    setSnapshot(await api.closeFilesView());
    setRightPanelMode("closed");
  }

  async function refreshFilesView(scope: AttachmentScope, filter: AttachmentFilter, sort: AttachmentSort) {
    const scopeParam: FilesViewScope =
      scope.kind === "space"
        ? { kind: "space", space_id: scope.space_id }
        : scope;
    setSnapshot(await api.openFilesView(scopeParam, filter, sort));
  }

  function updateThreadComposerDraft(
    roomId: string,
    rootEventId: string,
    document: ComposerDocument
  ) {
    const account = readyComposerDraftAccountOwner(snapshot);
    if (!account) return;
    const target: ComposerTarget = { kind: "thread", room_id: roomId, root_event_id: rootEventId };
    const scope = composerDraftScope(account, target);
    const lease = composerDraftLifecycleRegistryRef.current!.snapshot(scope);
    const revision = lease?.leaseId
      ? composerDraftLifecycleRegistryRef.current!.nextDraft(scope)
      : null;
    threadComposerOverlayRef.current = {
      scope,
      document,
      revision,
      debounceHandle: null
    };
    composerDraftLifecycleRegistryRef.current!.setActiveOverlay(scope, document, revision);
    if (revision) queueThreadComposerDraftPersist(scope, document, revision);
  }

  async function stageThreadUploadFiles(
    roomId: string,
    rootEventId: string,
    files: File[]
  ): Promise<void> {
    const thread = snapshot?.state.ui.thread;
    if (
      files.length === 0 ||
      thread?.kind !== "open" ||
      thread.room_id !== roomId ||
      thread.root_event_id !== rootEventId
    ) {
      return;
    }
    const target: ComposerTarget = { kind: "thread", room_id: roomId, root_event_id: rootEventId };
    let nextSnapshot: DesktopSnapshot | null = null;
    await stageAttachmentFiles(
      target,
      files,
      thread.staged_uploads?.length ?? 0,
      createStagedUploadId,
      async (capturedTarget, items) => {
        nextSnapshot = await api.stageUploadBytes(capturedTarget, items);
      }
    );
    if (nextSnapshot) setSnapshot(nextSnapshot);
  }

  /** Sends the open thread's staged attachments only; the draft is untouched. */
  async function sendThreadStagedAttachments(roomId: string, rootEventId: string) {
    if (stagedUploadSendInFlightRef.current) {
      return;
    }
    stagedUploadSendInFlightRef.current = true;
    try {
      await sendThreadStagedAttachmentsInner(roomId, rootEventId);
    } finally {
      stagedUploadSendInFlightRef.current = false;
    }
  }

  async function sendThreadStagedAttachmentsInner(roomId: string, rootEventId: string) {
    const thread = snapshot?.state.ui.thread;
    const account = readyComposerDraftAccountOwner(snapshot);
    const accountOwner = account ? composerDraftAccountOwnerKey(account) : null;
    const target: ComposerTarget = { kind: "thread", room_id: roomId, root_event_id: rootEventId };
    const uploads =
      thread?.kind === "open" &&
      thread.room_id === roomId &&
      thread.root_event_id === rootEventId
        ? thread.staged_uploads ?? []
        : [];
    if (!account || !accountOwner || uploads.length === 0) {
      return;
    }
    if (!uploadStagingItemsAreSendable(uploads)) return;
    const scope = composerDraftScope(account, target);
    const admitted = beginComposerOperation(scope);
    if (!admitted) return;
    const draftRevision = currentComposerDraftRevision(scope, admitted.lease);
    if (!reserveComposerAcceptedRevision(admitted, draftRevision)) return;
    const localRevisionAtSubmission = threadComposerOverlayRef.current?.revision ?? null;
    for (const item of uploads) {
      latestTextMutationQueueRef.current.invalidate(
        `caption:thread:${roomId}:${rootEventId}:${item.staged_id}`
      );
    }
    let response;
    try {
      response = await api.sendPreparedUploads(
        account,
        admitted.lease.leaseId,
        admitted.lease.rendererGeneration,
        target,
        draftRevision
      );
    } catch {
      settleComposerOperation(admitted);
      return;
    }
    const canApply = composerOperationCanApply(admitted, draftRevision);
    if (!canApply || submissionAccountOwnerRef.current !== accountOwner) return;
    const accepted =
      response.acceptedRevision !== null &&
      compareComposerDraftRevisions(response.acceptedRevision, draftRevision) > 0;
    const hasNewerDraft =
      threadComposerOverlayRef.current?.revision !== localRevisionAtSubmission;
    if (accepted && !hasNewerDraft) {
      cancelThreadComposerDraftPersist(scope);
      clearLocalThreadComposerDraft(scope);
    }
    setSnapshot(response.snapshot);
  }

  async function sendThreadReply(
    roomId: string,
    rootEventId: string,
    document: ComposerDocument
  ) {
    const body = plainBodyFromDocument(document);
    const account = readyComposerDraftAccountOwner(snapshot);
    const accountOwner = account ? composerDraftAccountOwnerKey(account) : null;
    const target: ComposerTarget = { kind: "thread", room_id: roomId, root_event_id: rootEventId };
    appendComposerSubmitDiagnostic(
      "thread",
      "received",
      `room_present=${Boolean(roomId)} root_present=${Boolean(rootEventId)} account_ready=${Boolean(account && accountOwner)} body_present=${Boolean(body.trim())}`
    );
    // Text only: thread attachments are sent from the staging panel.
    if (!account || !accountOwner || !body.trim()) {
      const reason = !account || !accountOwner ? "account_not_ready" : "empty_body";
      appendComposerSubmitDiagnostic("thread", "blocked", `reason=${reason}`);
      return;
    }
    const submissionController = submissionRegistryRef.current!.forTarget(
      threadSubmissionTarget(roomId, rootEventId)
    );
    const submissionId = submissionController.begin();
    if (submissionId === null) {
      appendComposerSubmitDiagnostic("thread", "blocked", "reason=submission_in_progress");
      return;
    }
    if (submissionController.payload(submissionId) === undefined) {
      const scope = composerDraftScope(account, target);
      const draftRevision =
        composerDraftLifecycleRegistryRef.current!.snapshot(scope)?.revision ??
        COMPOSER_DRAFT_REVISION_ZERO;
      const localRevisionAtSubmission = threadComposerOverlayRef.current?.revision ?? null;
      submissionController.capture(submissionId, {
        roomId,
        rootEventId,
        body,
        document,
        draftRevision,
        localRevisionAtSubmission,
        account,
        accountOwner,
        scope
      });
    }
    const captured = submissionController.payload<{
      roomId: string;
      rootEventId: string;
      body: string;
      document: ComposerDocument;
      draftRevision: ComposerDraftRevision;
      localRevisionAtSubmission: ComposerDraftRevision | null;
      account: { homeserver: string; userId: string; deviceId: string };
      accountOwner: string;
      scope: ComposerDraftScope;
    }>(submissionId)!;
    const admitted = beginComposerOperation(captured.scope);
    if (!admitted) {
      appendComposerSubmitDiagnostic("thread", "blocked", "reason=draft_operation_not_admitted");
      submissionController.reject(submissionId);
      return;
    }
    if (!reserveComposerAcceptedRevision(admitted, captured.draftRevision)) {
      appendComposerSubmitDiagnostic("thread", "blocked", "reason=draft_revision_not_reserved");
      submissionController.reject(submissionId);
      return;
    }
    let response;
    try {
      appendComposerSubmitDiagnostic("thread", "dispatch", "mode=thread_reply");
      response = await api.sendThreadReply(
        captured.account,
        admitted.lease.leaseId,
        admitted.lease.rendererGeneration,
        submissionId,
        captured.roomId,
        captured.rootEventId,
        captured.document,
        captured.draftRevision
      );
    } catch (error) {
      settleComposerOperation(admitted);
      if (submissionAccountOwnerRef.current !== captured.accountOwner) {
        return;
      }
      const disposition = classifySubmissionFailure(error);
      appendComposerSubmitDiagnostic(
        "thread",
        "settled",
        `outcome=failed failure_class=${disposition.kind}`
      );
      if (disposition.kind === "unknown") {
        submissionController.markUnknown(submissionId, disposition.reason);
      } else {
        submissionController.reject(submissionId);
      }
      return;
    }
    const canApply = composerOperationCanApply(admitted, captured.draftRevision);
    if (!canApply || submissionAccountOwnerRef.current !== captured.accountOwner) {
      appendComposerSubmitDiagnostic("thread", "settled", "outcome=stale_context_ignored");
      return;
    }
    if (response.submissionId !== submissionId || response.outcome !== "accepted") {
      appendComposerSubmitDiagnostic("thread", "settled", "outcome=rejected_by_backend");
      submissionController.reject(submissionId);
      setSnapshot(response.snapshot);
      return;
    }
    submissionController.accept(submissionId);
    appendComposerSubmitDiagnostic("thread", "settled", "outcome=accepted");
    const hasNewerDraft =
      threadComposerOverlayRef.current?.revision !== captured.localRevisionAtSubmission;
    if (!hasNewerDraft) {
      cancelThreadComposerDraftPersist(captured.scope);
      clearLocalThreadComposerDraft(captured.scope);
    }
    setSnapshot(response.snapshot);
  }


  async function clearThreadUploadStaging(roomId: string, rootEventId: string) {
    const thread = snapshot?.state.ui.thread;
    if (
      thread?.kind === "open" &&
      thread.room_id === roomId &&
      thread.root_event_id === rootEventId
    ) {
      for (const item of thread.staged_uploads ?? []) {
        latestTextMutationQueueRef.current.invalidate(
          `caption:thread:${roomId}:${rootEventId}:${item.staged_id}`
        );
      }
    }
    setSnapshot(
      await api.clearUploadStaging({
        kind: "thread",
        room_id: roomId,
        root_event_id: rootEventId
      })
    );
  }

  async function updateThreadStagedUploadCaption(
    roomId: string,
    rootEventId: string,
    stagedId: string,
    caption: string
  ) {
    await applyLatestTextMutationSnapshot(`caption:thread:${roomId}:${rootEventId}:${stagedId}`, () =>
      api.updateStagedUploadCaption(
        { kind: "thread", room_id: roomId, root_event_id: rootEventId },
        stagedId,
        caption
      )
    );
  }

  async function selectThreadStagedUploadOutput(
    roomId: string,
    rootEventId: string,
    stagedId: string,
    selection: StagedUploadOutputSelection
  ) {
    setSnapshot(
      await api.selectStagedUploadOutput(
        { kind: "thread", room_id: roomId, root_event_id: rootEventId },
        stagedId,
        selection
      )
    );
  }

  async function loadThreadStagedUploadPreview(
    roomId: string,
    rootEventId: string,
    stagedId: string,
    variantId: string
  ): Promise<number[]> {
    return api.preparedUploadPreview(
      { kind: "thread", room_id: roomId, root_event_id: rootEventId },
      stagedId,
      variantId
    );
  }

  async function retryThreadStagedUploadPreparation(
    roomId: string,
    rootEventId: string,
    stagedId: string
  ) {
    setSnapshot(
      await api.retryStagedUploadPreparation(
        { kind: "thread", room_id: roomId, root_event_id: rootEventId },
        stagedId
      )
    );
  }

  async function useOriginalThreadStagedUpload(
    roomId: string,
    rootEventId: string,
    stagedId: string
  ) {
    setSnapshot(
      await api.useOriginalStagedUpload(
        { kind: "thread", room_id: roomId, root_event_id: rootEventId },
        stagedId
      )
    );
  }

  async function scheduleThreadSend(
    roomId: string,
    rootEventId: string,
    sendAtMs: number,
    document: ComposerDocument
  ) {
    const body = plainBodyFromDocument(document);
    const thread = snapshot?.state.ui.thread;
    const account = readyComposerDraftAccountOwner(snapshot);
    const accountOwner = account ? composerDraftAccountOwnerKey(account) : null;
    if (
      !account ||
      !accountOwner ||
      !body.trim() ||
      thread?.kind !== "open" ||
      thread.room_id !== roomId ||
      thread.root_event_id !== rootEventId ||
      (thread.staged_uploads?.length ?? 0) > 0
    ) {
      return;
    }
    const target: ComposerTarget = {
      kind: "thread",
      room_id: roomId,
      root_event_id: rootEventId
    };
    const scope = composerDraftScope(account, target);
    const admitted = beginComposerOperation(scope);
    if (!admitted) return;
    const draftRevision = currentComposerDraftRevision(scope, admitted.lease);
    if (!reserveComposerAcceptedRevision(admitted, draftRevision)) return;
    const localRevisionAtSubmission = threadComposerOverlayRef.current?.revision ?? null;
    let response;
    try {
      response = await api.scheduleSend(
        account,
        admitted.lease.leaseId,
        admitted.lease.rendererGeneration,
        target,
        body,
        sendAtMs,
        draftRevision
      );
    } catch {
      settleComposerOperation(admitted);
      return;
    }
    const canApply = composerOperationCanApply(admitted, draftRevision);
    if (!canApply || submissionAccountOwnerRef.current !== accountOwner) return;
    const accepted =
      response.acceptedRevision !== null &&
      compareComposerDraftRevisions(response.acceptedRevision, draftRevision) > 0;
    const hasNewerDraft =
      threadComposerOverlayRef.current?.revision !== localRevisionAtSubmission;
    if (accepted && !hasNewerDraft) {
      cancelThreadComposerDraftPersist(scope);
      clearLocalThreadComposerDraft(scope);
    }
    setSnapshot(response.snapshot);
  }

  function queueThreadComposerDraftPersist(
    scope: ComposerDraftScope,
    document: ComposerDocument,
    revision: ComposerDraftRevision
  ) {
    if (scope.target.kind !== "thread") return;
    const target = scope.target;
    cancelThreadComposerDraftPersist(scope);
    const handle = window.setTimeout(() => {
      const overlay = threadComposerOverlayRef.current;
      if (overlay && composerDraftScopesEqual(overlay.scope, scope)) {
        overlay.debounceHandle = null;
      }
      composerDraftLifecycleRegistryRef.current!.clearDebounce(scope);
      const admitted = beginComposerOperation(scope);
      if (!admitted) return;
      const account = composerDraftApiAccount(scope);
      void api
        .setThreadComposerDraft(
          account,
          admitted.lease.leaseId,
          admitted.lease.rendererGeneration,
          target.room_id,
          target.root_event_id,
          document,
          revision
        )
        .then((nextSnapshot) => {
          const canApply = composerOperationCanApply(admitted, revision);
          const currentOverlay = threadComposerOverlayRef.current;
          if (
            !canApply ||
            submissionAccountOwnerRef.current !== composerDraftAccountOwnerKey(account) ||
            !currentOverlay ||
            !composerDraftScopesEqual(currentOverlay.scope, scope) ||
            currentOverlay.revision !== revision ||
            currentOverlay.document !== document
          ) return;
          setSnapshot(nextSnapshot);
        })
        .catch(() => settleComposerOperation(admitted));
    }, 350);
    const overlay = threadComposerOverlayRef.current;
    if (overlay && composerDraftScopesEqual(overlay.scope, scope)) {
      overlay.debounceHandle = handle;
    }
    composerDraftLifecycleRegistryRef.current!.setDebounce(scope, handle);
  }

  function cancelThreadComposerDraftPersist(scope: ComposerDraftScope) {
    const overlay = threadComposerOverlayRef.current;
    if (!overlay || !composerDraftScopesEqual(overlay.scope, scope)) return;
    if (overlay.debounceHandle !== null) {
      window.clearTimeout(overlay.debounceHandle);
      overlay.debounceHandle = null;
    }
    composerDraftLifecycleRegistryRef.current!.clearDebounce(scope);
  }

  function clearLocalThreadComposerDraft(scope: ComposerDraftScope) {
    const overlay = threadComposerOverlayRef.current;
    if (!overlay || !composerDraftScopesEqual(overlay.scope, scope)) return;
    threadComposerOverlayRef.current = null;
    composerDraftLifecycleRegistryRef.current!.setActiveOverlay(scope, null, null);
  }

  function focusedContextVisibleForMode(mode: RightPanelMode): boolean {
    const effectiveMode = snapshot
      ? effectiveRightPanelModeForSnapshot(mode, snapshot)
      : mode;
    return effectiveMode === "search" || effectiveMode === "focusedContext";
  }

  function hasActiveFocusedContext(): boolean {
    const focusedContext = snapshot?.state.ui.focused_context;
    return focusedContext?.kind === "opening" || focusedContext?.kind === "open";
  }

  async function closeFocusedContextIfHiddenBy(nextMode: RightPanelMode): Promise<void> {
    if (
      hasActiveFocusedContext() &&
      focusedContextVisibleForMode(rightPanelMode) &&
      !focusedContextVisibleForMode(nextMode)
    ) {
      setSnapshot(await api.closeFocusedContext());
    }
  }

  async function setRightPanelModeClosingFocusedContext(
    nextMode: RightPanelMode,
    isCurrent: () => boolean = () => true
  ) {
    if (!isCurrent()) {
      return;
    }
    if (nextMode !== "spaceInfo") {
      spaceSettingsRequestRef.current += 1;
    }
    await closeFocusedContextIfHiddenBy(nextMode);
    if (!isCurrent()) {
      return;
    }
    if (nextMode !== "profile") {
      setSelectedProfileUserId(null);
    }
    if (nextMode !== "people" && nextMode !== "profile") {
      setPeoplePanelScope(null);
    }
    setRightPanelMode(nextMode);
  }

  function spaceMemberRequestStillCurrent(
    requestId: number,
    fence: SpaceMemberFence
  ): boolean {
    return (
      spaceMembersOpenRequestRef.current === requestId &&
      spaceMembersSnapshotMatches(snapshotRef.current, fence)
    );
  }

  async function openSpaceMembers(trigger: SpaceMembersOpenTrigger): Promise<void> {
    const fence = spaceMembersFenceForSnapshot(snapshotRef.current);
    if (!fence) {
      return;
    }
    const requestId = ++spaceMembersOpenRequestRef.current;
    spaceSettingsRequestRef.current += 1;
    spaceSettingsLoadRef.current = null;
    appendSpaceMembersDiagnosticLog(`open trigger=${trigger}`);
    setPeoplePanelScope({ kind: "space", spaceId: fence.spaceId });
    setSelectedProfileUserId(null);
    await setRightPanelModeClosingFocusedContext(
      "people",
      () => spaceMemberRequestStillCurrent(requestId, fence)
    );
    if (!spaceMemberRequestStillCurrent(requestId, fence)) {
      return;
    }

    try {
      const settingsSnapshot = await api.loadRoomSettings(fence.spaceId);
      if (
        !spaceMemberRequestStillCurrent(requestId, fence) ||
        !spaceMembersSnapshotMatches(settingsSnapshot, fence)
      ) {
        return;
      }
      setSnapshot(settingsSnapshot);

      const membersSnapshot = await ensureSpaceMembersLoaded(fence.spaceId, fence.generation);
      if (
        !spaceMemberRequestStillCurrent(requestId, fence) ||
        !membersSnapshot ||
        !spaceMembersSnapshotMatches(membersSnapshot, fence)
      ) {
        return;
      }
    } catch {
      if (spaceMemberRequestStillCurrent(requestId, fence)) {
        appendSpaceMembersDiagnosticLog("load outcome=failed");
      }
    }
  }

  // #508: stable callback for the Space members invite search. Reads the
  // latest snapshot via refs so the identity never changes across renders
  // (which would re-trigger the panel's debounced effect indefinitely) and
  // merges the Rust-owned exact-MXID candidate into the result list.
  // #508: invalidates in-flight space invite searches when the panel leaves
  // invite mode, so a stale response can never re-dirty the shared workflow.
  const spaceInviteSearchRequestRef = useRef(0);

  const resetSpaceInviteSearch = useCallback(async () => {
    spaceInviteSearchRequestRef.current += 1;
    const nextSnapshot = await api.closeInviteWorkflow();
    setSnapshot(nextSnapshot);
  }, []);

  const searchSpaceInviteTargets = useCallback(async (query: string) => {
    const fence = spaceMembersFenceForSnapshot(snapshotRef.current);
    if (!fence) {
      return [];
    }
    const requestId = spaceInviteSearchRequestRef.current;
    const nextSnapshot = await api.searchInviteTargets(fence.spaceId, query);
    if (spaceInviteSearchRequestRef.current !== requestId) {
      return [];
    }
    setSnapshot(nextSnapshot);
    const inviteQuery = nextSnapshot.state.domain.invite_workflow?.query;
    const candidates = inviteQuery?.candidates ?? [];
    if (!inviteQuery?.explicit_user_id) {
      return candidates;
    }
    return candidates.some(
      (candidate) => candidate.user_id === inviteQuery.explicit_user_id!.user_id
    )
      ? candidates
      : [...candidates, inviteQuery.explicit_user_id];
  }, []);

  async function inviteUserToSpace(
    userId: string,
    trigger: SpaceMemberInviteTrigger,
    expectedFence: SpaceMemberFence | null = null
  ): Promise<void> {
    const currentSnapshot = snapshotRef.current;
    const fence = expectedFence ?? spaceMembersFenceForSnapshot(currentSnapshot);
    const settings = fence ? exactRoomSettingsForRoom(currentSnapshot, fence.spaceId) : null;
    const members = currentSnapshot?.state.domain.space_members;
    const childOnlyEntry = members?.child_room_only.find((entry) => entry.user_id === userId);
    const alreadyInSpace =
      members?.space_joined.some((entry) => entry.user_id === userId) ||
      members?.space_invited.some((entry) => entry.user_id === userId);
    const operationPending =
      members?.operation.kind === "loading" ||
      members?.operation.kind === "inviting" ||
      members?.operation.kind === "cancellingInvite";
    // #508: the search flow invites brand-new users to the Space (space
    // membership only); the inline/context flows keep the child-room-only
    // invite-up semantics.
    const inviteUpBlocked =
      trigger === "search"
        ? alreadyInSpace
        : !childOnlyEntry || childOnlyEntry.invite_pending;
    const availabilityReason: SpaceInviteAvailabilityReason =
      !fence || !spaceMembersSnapshotMatches(currentSnapshot, fence)
        ? "settings_unavailable"
        : !settings
          ? "settings_unavailable"
          : !settings.permissions.can_invite
            ? "permission_denied"
            : operationPending
              ? "operation_pending"
              : inviteUpBlocked
                ? trigger === "search"
                  ? "already_in_space"
                  : "invite_pending"
                : "available";
    appendSpaceMembersDiagnosticLog(
      `invite trigger=${trigger} availability_reason=${availabilityReason}`
    );
    if (
      !fence ||
      !spaceMembersSnapshotMatches(currentSnapshot, fence) ||
      !settings?.permissions.can_invite ||
      operationPending ||
      inviteUpBlocked
    ) {
      return;
    }

    const requestId = ++spaceMembersInviteRequestRef.current;
    try {
      const nextSnapshot = await api.inviteUserToSpace(fence.spaceId, userId, fence.generation);
      if (
        spaceMembersInviteRequestRef.current !== requestId ||
        !spaceMembersSnapshotMatches(snapshotRef.current, fence) ||
        !spaceMembersSnapshotMatches(nextSnapshot, fence)
      ) {
        return;
      }
      setSnapshot(nextSnapshot);
    } catch {
      if (
        spaceMembersInviteRequestRef.current === requestId &&
        spaceMembersSnapshotMatches(snapshotRef.current, fence)
      ) {
        appendSpaceMembersDiagnosticLog("invite outcome=transport_rejected");
      }
    }
  }

  async function cancelSpaceInvite(
    userId: string,
    trigger: SpaceMemberCancelTrigger,
    expectedFence: SpaceMemberFence | null = null
  ): Promise<void> {
    const currentSnapshot = snapshotRef.current;
    const fence = expectedFence ?? spaceMembersFenceForSnapshot(currentSnapshot);
    const settings = fence ? exactRoomSettingsForRoom(currentSnapshot, fence.spaceId) : null;
    const members = currentSnapshot?.state.domain.space_members;
    const invitedEntry = members?.space_invited.find((entry) => entry.user_id === userId);
    const operationPending =
      members?.operation.kind === "loading" ||
      members?.operation.kind === "inviting" ||
      members?.operation.kind === "cancellingInvite";
    const availabilityReason: SpaceInviteCancellationAvailabilityReason =
      !fence || !spaceMembersSnapshotMatches(currentSnapshot, fence)
        ? "settings_unavailable"
        : !settings
          ? "settings_unavailable"
          : !settings.permissions.can_kick
            ? "permission_denied"
            : operationPending
              ? "operation_pending"
              : !invitedEntry
                ? "invite_unavailable"
                : "available";
    appendSpaceMembersDiagnosticLog(
      `cancel trigger=${trigger} availability_reason=${availabilityReason}`
    );
    if (
      !fence ||
      !spaceMembersSnapshotMatches(currentSnapshot, fence) ||
      !settings?.permissions.can_kick ||
      operationPending ||
      !invitedEntry
    ) {
      return;
    }

    const requestId = ++spaceMembersCancelRequestRef.current;
    setSpaceMembersCancelFailure(null);
    try {
      const nextSnapshot = await api.cancelSpaceInvite(fence.spaceId, userId, fence.generation);
      if (
        spaceMembersCancelRequestRef.current !== requestId ||
        !spaceMembersSnapshotMatches(snapshotRef.current, fence) ||
        !spaceMembersSnapshotMatches(nextSnapshot, fence)
      ) {
        return;
      }
      const nextOperation = nextSnapshot.state.domain.space_members.operation;
      const cancellationFailed =
        nextOperation.kind === "failed" &&
        nextOperation.space_id === fence.spaceId &&
        nextOperation.user_id === userId &&
        nextOperation.generation === fence.generation;
      setSpaceMembersCancelFailure(cancellationFailed ? fence : null);
      setSnapshot(nextSnapshot);
      appendSpaceMembersDiagnosticLog(
        `cancel outcome=${cancellationFailed
          ? "failed"
          : nextOperation.kind === "cancellingInvite"
            ? "pending"
            : "settled"}`
      );
    } catch {
      if (
        spaceMembersCancelRequestRef.current === requestId &&
        spaceMembersSnapshotMatches(snapshotRef.current, fence)
      ) {
        setSpaceMembersCancelFailure(fence);
        appendSpaceMembersDiagnosticLog("cancel outcome=transport_rejected");
      }
    }
  }

  async function reloadSpaceMemberRoles(): Promise<void> {
    const fence = spaceMembersFenceForSnapshot(snapshotRef.current);
    const operation = snapshotRef.current?.state.domain.space_members.operation;
    if (!fence || operation?.kind !== "roleUpdateFailed") {
      return;
    }
    spaceMembersLoadedRef.current.delete(`${fence.spaceId}\u0000${fence.generation}`);
    await ensureSpaceMembersLoaded(fence.spaceId, fence.generation);
  }

  async function updateSpaceMemberRole(
    userId: string,
    option: SpaceMemberRoleOption
  ): Promise<void> {
    const currentSnapshot = snapshotRef.current;
    const fence = spaceMembersFenceForSnapshot(currentSnapshot);
    const members = currentSnapshot?.state.domain.space_members;
    const entry = members?.space_joined.find((candidate) => candidate.user_id === userId);
    if (
      !fence ||
      !members ||
      !entry ||
      members.operation.kind === "updatingRole" ||
      !members.can_edit_roles ||
      entry.power_level === null ||
      !entry.role_options.some(
        (candidateOption) => candidateOption.power_level === option.power_level
      )
    ) {
      return;
    }

    const requestId = ++spaceMembersRoleRequestRef.current;
    setSpaceMembersRoleTransportFailure(null);
    appendSpaceMembersDiagnosticLog("role trigger=select");
    try {
      const nextSnapshot = await api.updateSpaceMemberRole(
        fence.spaceId,
        userId,
        fence.generation,
        members.power_levels_revision,
        entry.power_level,
        option.power_level,
        option.requires_confirmation
      );
      if (
        spaceMembersRoleRequestRef.current !== requestId ||
        !spaceMembersSnapshotMatches(snapshotRef.current, fence) ||
        !spaceMembersSnapshotMatches(nextSnapshot, fence)
      ) {
        return;
      }
      setSpaceMembersRoleTransportFailure(null);
      setSnapshot(nextSnapshot);
      const operation = nextSnapshot.state.domain.space_members.operation;
      appendSpaceMembersDiagnosticLog(
        `role outcome=${operation.kind === "roleUpdateFailed" ? "failed" : operation.kind === "updatingRole" ? "pending" : "settled"}`
      );
    } catch {
      if (
        spaceMembersRoleRequestRef.current === requestId &&
        spaceMembersSnapshotMatches(snapshotRef.current, fence)
      ) {
        setSpaceMembersRoleTransportFailure(fence);
        appendSpaceMembersDiagnosticLog("role outcome=transport_rejected");
      }
    }
  }

  async function closeFocusedContextPanel() {
    if (rightPanelMode === "files") {
      await closeFilesViewPanel();
      return;
    }
    if (rightPanelMode === "threads") {
      await closeThreadsListPanel();
      return;
    }
    if (rightPanelMode === "search") {
      await closeSearchPanel();
      return;
    }
    setSelectedProfileUserId(null);
    await setRightPanelModeClosingFocusedContext("closed");
  }

  async function closeSearchPanel() {
    if (searchTimer.current) {
      window.clearTimeout(searchTimer.current);
      searchTimer.current = null;
    }
    setSnapshot(await api.closeSearch());
    setSearchQuery("");
    setRightPanelMode("closed");
  }

  function openActivityRow(roomId: string, eventId: string, threadRootEventId: string | null) {
    if (threadRootEventId) {
      void (async () => {
        setPrimaryView("timeline");
        if (!(await drainActiveComposerScopesForNavigation(true, true))) return;
        await selectRoom(roomId);
        await openThread(roomId, threadRootEventId, "existingThread");
      })();
      return;
    }
    void api.openActivityEvent(roomId, eventId).then((nextSnapshot) => {
      setSnapshot(nextSnapshot);
      setPrimaryView("timeline");
      setRightPanelMode("closed");
    });
  }

  async function openActivityRoom(roomId: string) {
    setPrimaryView("timeline");
    setRightPanelMode("closed");

    const closedSnapshot = await api.closeFocusedContext();
    if (
      closedSnapshot.state.ui.navigation.active_room_id === roomId &&
      closedSnapshot.state.ui.timeline.room_id === roomId
    ) {
      setSnapshot(closedSnapshot);
      return;
    }

    await selectRoom(roomId);
  }

  function selectSearchResult(roomId: string, eventId: string) {
    void api.selectSearchResult(roomId, eventId).then((nextSnapshot) => {
      setSnapshot(nextSnapshot);
      setPrimaryView("timeline");
      setRightPanelMode("search");
    });
  }

  function runContextMenuAction(actionId: ContextMenuActionId) {
    const activeMenu = contextMenu;
    setContextMenu(null);
    if (!activeMenu) {
      return;
    }

    const { target } = activeMenu;
    if (target.kind === "spaceMember") {
      if (actionId === "inviteUserToSpace") {
        void inviteUserToSpace(
          target.userId,
          "context",
          { spaceId: target.spaceId, generation: target.generation }
        );
      }
      return;
    }
    if (target.kind === "message") {
      switch (actionId) {
        case "replyToMessage":
          void setComposerReplyTarget(target.message.room_id, target.message.event_id);
          return;
        case "openThread":
          void openThread(
            target.message.room_id,
            target.message.event_id,
            target.message.reply_count > 0 ? "existingThread" : "newThreadDraft"
          );
          return;
        case "editMessage":
          void editMessage(target.message);
          return;
        case "redactMessage":
          void redactMessage(target.message.room_id, target.message.event_id);
          return;
        case "ignoreUser":
          void ignoreUser(target.message.sender);
          return;
        case "unignoreUser":
          void unignoreUser(target.message.sender);
          return;
        case "reportUser":
          openReportDialog({ kind: "user", userId: target.message.sender });
          return;
        case "reportContent":
          openReportDialog({
            kind: "content",
            roomId: target.message.room_id,
            eventId: target.message.event_id
          });
          return;
        default:
          return;
      }
    }

    if (target.kind === "room") {
      switch (actionId) {
        case "openUserInfo":
          if (target.dmUserId) {
            void openDmUserInfo(target.roomId, target.dmUserId);
          }
          return;
        case "setRoomFavourite":
          void api.setRoomTag(target.roomId, "favourite").then(setSnapshot);
          return;
        case "removeRoomFavourite":
          void api.removeRoomTag(target.roomId, "favourite").then(setSnapshot);
          return;
        case "setRoomLowPriority":
          void api.setRoomTag(target.roomId, "lowPriority").then(setSnapshot);
          return;
        case "removeRoomLowPriority":
          void api.removeRoomTag(target.roomId, "lowPriority").then(setSnapshot);
          return;
        case "markRoomAsRead": {
          const room = snapshot?.state.domain.rooms.find((candidate) => candidate.room_id === target.roomId);
          const eventId =
            roomLatestDisplayEventId(room?.latest_event) ??
            snapshot?.state.domain.live_signals.rooms[target.roomId]?.fully_read_event_id ??
            "";
          if (eventId.trim().length > 0) {
            void api.markRoomAsRead(target.roomId, eventId).then(setSnapshot);
          }
          return;
        }
        case "markRoomAsUnread":
          void api.markRoomAsUnread(target.roomId, true).then(setSnapshot);
          return;
        case "reportRoom":
          openReportDialog({ kind: "room", roomId: target.roomId });
          return;
        case "leaveRoom":
          // #373: never leave straight from the menu click. The confirmation
          // owns the destructive step; the Rust `leave_room` command owns the
          // membership change and the resulting room list.
          setPendingRoomLeave({
            roomId: target.roomId,
            isDm: Boolean(target.dmUserId)
          });
          return;
        default:
          break;
      }
    }

    if (target.kind === "space" && actionId === "leaveSpace") {
      void api.leaveRoom(target.spaceId).then((nextSnapshot) => {
        setSnapshot(nextSnapshot);
        if (rightPanelMode === "spaceInfo") {
          void setRightPanelModeClosingFocusedContext("closed");
        }
      });
      return;
    }

    const intent = rightPanelIntentForContextMenuAction(
      rightPanelTargetFromContextMenuTarget(target),
      actionId
    );
    if (!intent) {
      return;
    }

    const applyIntentMode = async () => {
      if (intent.mode) {
        await setRightPanelModeClosingFocusedContext(intent.mode);
      }
      if (intent.focusSearch) {
        setSearchScope("currentRoom");
        searchInputRef.current?.focus();
      }
    };

    if (intent.selectRoomId) {
      void selectRoom(intent.selectRoomId).then(() => {
        void applyIntentMode();
      });
      return;
    }
    if (intent.selectSpaceId) {
      void selectSpace(intent.selectSpaceId).then(() => {
        void applyIntentMode();
      });
      return;
    }
    void applyIntentMode();
    if (actionId === "switchAccount") {
      void refreshSavedSessions();
    }
  }

  async function runSearch(query: string, scope: SearchScopeKind) {
    const trimmed = query.trim();
    const searchMode = rightPanelModeForSearchQuery(trimmed);
    if (!trimmed) {
      setSnapshot(await api.closeSearch());
      if (rightPanelMode === "search") {
        setRightPanelMode("closed");
      }
      return;
    }
    if (searchMode) {
      setRightPanelMode(searchMode);
    }
    setSnapshot(await api.submitSearch(trimmed, scope));
  }

  // #87 Phase 4 IPC contract guard (fail-closed): an incompatible snapshot (a stale flat v1
  // snapshot or a mismatched Rust/TS build) was rejected at the setSnapshot boundary above, so
  // it never reached the render body's domain/ui reads. Show an explicit recovery screen
  // instead of the normal shell. This gate runs before the `!snapshot` check so a mismatch on
  // the very first snapshot still surfaces the recovery screen rather than the bare boot screen.
  if (schemaMismatchVersion !== null) {
    return (
      <div className="boot-screen" role="alert">
        <div className="boot-screen__notice">
          <span>{t("app.versionMismatch.title")}</span>
          <span className="boot-screen__notice-detail">{t("app.versionMismatch.detail")}</span>
        </div>
      </div>
    );
  }

  if (!snapshot) {
    return <div className="boot-screen">{t("app.title")}</div>;
  }

  const sessionKind = snapshot.state.domain.session.kind;
  const secureBackupGate = snapshot.state.domain.secure_backup_gate;
  const secureBackupStartupGateRequired =
    sessionKind === "ready" &&
    !secureBackupGateIsOperational &&
    !secureBackupShellExposedRef.current;
  const secureBackupRuntimeDegraded =
    sessionKind === "ready" &&
    secureBackupGate.kind !== "ready" &&
    secureBackupShellExposedRef.current;
  const secureBackupRuntimeFailure = secureBackupGateFailure(secureBackupGate);

  if (sessionKind === "restoring" || sessionKind === "loggingOut") {
    return <div className="boot-screen">{t("app.title")}</div>;
  }

  setActiveLocaleProfile(
    snapshot.state.domain.locale_profile.catalog_locale,
    snapshot.state.domain.locale_profile.pseudo_locale
  );

  const verificationGate =
    [
      "provisional",
      "awaitingVerification",
      "verifying",
      "awaitingBootstrapConfirmation",
      "rejecting",
      "locked"
    ].includes(sessionKind) ||
    secureBackupStartupGateRequired;
  if (verificationGate) {
    return (
      <SessionVerificationGate
        snapshot={snapshot}
        onSnapshot={setSnapshot}
        onSignOut={() => void logout()}
        operations={{
          startOwnUserSas: () => api.startOwnUserSas(),
          submitRecovery: (secret) => api.submitRecovery(secret),
          recoverSecureBackup: api.recoverSecureBackup,
          setupSecureBackup: api.setupSecureBackup,
          reenableSecureBackup: api.reenableSecureBackup,
          chooseSecureBackupDestination,
          retrySecureBackupInspection: api.retrySecureBackupInspection,
          openSecureBackupDiagnostics: openDiagnostics
        }}
      />
    );
  }

  if (sessionKind === "capabilityBlocked") {
    const blockedSession = snapshot.state.domain.session;
    return (
      <SlidingSyncCapabilityBlockedScreen
        isBusy={isBusy}
        session={blockedSession}
        onRetry={() => void retrySlidingSyncCapability()}
        onSignOut={() => void logout()}
        onChangeHomeserver={() => void changeCapabilityHomeserver()}
      />
    );
  }

  if (sessionKind !== "ready") {
    return (
      <><AuthScreen
        deviceName={loginDeviceName}
        homeserver={loginHomeserver}
        isBusy={isBusy || sessionKind === "authenticating"}
        passwordFilled={loginPasswordFilled}
        passwordInputRef={loginPasswordRef}
        snapshot={snapshot}
        username={loginUsername}
        onDiscoverLoginMethods={discoverLoginMethods}
        onDeviceNameChange={setLoginDeviceName}
        onHomeserverChange={setLoginHomeserver}
        onPasswordPresenceChange={setLoginPasswordFilled}
        onStartOidcLogin={startOidcLogin}
        onSubmit={submitLogin}
        onUsernameChange={setLoginUsername}
      />{loginTransportError && <p role="alert">{loginTransportError}</p>}</>
    );
  }

  const activeRoom = snapshot.state.domain.rooms.find(
    (room) => room.room_id === snapshot.state.ui.navigation.active_room_id
  );
  const encryptedComposerBlocked =
    !secureBackupGateIsOperational && Boolean(activeRoom?.is_encrypted);
  const runtimeAlerts: RuntimeAlert[] = [];
  if (secureBackupRuntimeDegraded) {
    runtimeAlerts.push({
      kind: "secureBackup",
      severity: "warning",
      title: t("sessionStatus.runtimeAlertSecureBackup"),
      detail: [
        t("gate.secureBackupRuntimeDegraded"),
        secureBackupRuntimeFailure ? secureBackupFailureLabel(secureBackupRuntimeFailure) : null
      ]
        .filter((value): value is string => Boolean(value))
        .join(" "),
      retryable: Boolean(api.retrySecureBackupInspection)
    });
  }
  if (typeof snapshot.state.domain.sync !== "string") {
    const syncStatus = syncStatePresentation(snapshot.state.domain.sync);
    runtimeAlerts.push({
      kind: "sync",
      severity: "reconnecting" in snapshot.state.domain.sync ? "warning" : "error",
      title: t("sessionStatus.sync"),
      detail: syncStatus.ariaLabel,
      retryable: false
    });
  }
  if (snapshot.state.domain.current_session_status.status === "failed") {
    runtimeAlerts.push({
      kind: "session",
      severity: "error",
      title: t("sessionStatus.failed"),
      detail: currentSessionStatusFailureLabel(snapshot.state.domain.current_session_status.kind),
      retryable: false
    });
  }
  const activeSpace = snapshot.state.domain.spaces.find(
    (space) => space.space_id === snapshot.state.ui.navigation.active_space_id
  );
  const activeSpacePeopleScope = peoplePanelScope?.kind === "space" ? peoplePanelScope : null;
  const canInviteToSpace = Boolean(
    activeSpace &&
      activeSpacePeopleScope?.spaceId === activeSpace.space_id &&
      exactRoomSettingsForRoom(snapshot, activeSpace.space_id)?.permissions.can_invite
  );
  const spaceInviteAvailabilityReason: SpaceInviteAvailabilityReason = activeSpacePeopleScope
    ? spaceInviteAvailabilityReasonForSnapshot(snapshot, activeSpacePeopleScope.spaceId)
    : "settings_unavailable";
  const canCancelInvite = Boolean(
    activeSpace &&
      activeSpacePeopleScope?.spaceId === activeSpace.space_id &&
      exactRoomSettingsForRoom(snapshot, activeSpace.space_id)?.permissions.can_kick
  );
  const cancelAvailabilityReason: SpaceInviteCancellationAvailabilityReason = activeSpacePeopleScope
    ? spaceInviteCancellationAvailabilityReasonForSnapshot(snapshot, activeSpacePeopleScope.spaceId)
    : "settings_unavailable";
  const cancelInviteFailure = Boolean(
    spaceMembersCancelFailure &&
      spaceMembersSnapshotMatches(snapshot, spaceMembersCancelFailure)
  );
  const homeContextActive = snapshot.sidebar.account_home.is_active && !activeSpace;
  const activeSpaceName = activeSpace
    ? spaceDisplayName(activeSpace.space_id, activeSpace.display_name, spaceLocalOverrides)
    : snapshot.sidebar.account_home.display_name;
  const threadsListScope: ThreadsListScope = activeSpace
    ? { kind: "space", space_id: activeSpace.space_id }
    : { kind: "home" };
  const openThreadsListScope: ThreadsListScope =
    snapshot.state.ui.threads_list.kind === "closed"
      ? threadsListScope
      : threadsListScopeFromKey(snapshot.state.ui.threads_list.room_id);
  const activeSearchState = correlatedSearchState(
    snapshot.state.domain.search,
    searchQuery,
    searchScope
  );
  const trimmedSearchQuery = searchQuery.trim();
  const searchPending =
    Boolean(trimmedSearchQuery) &&
    (activeSearchState === null ||
      activeSearchState.kind === "editing" ||
      activeSearchState.kind === "searching");
  const searchResults = activeSearchState?.kind === "results" ? activeSearchState.results : [];
  const searchTooShortMinChars =
    activeSearchState?.kind === "tooShort" ? activeSearchState.min_chars : null;
  const searchResultsQuery =
    activeSearchState?.kind === "results"
      ? activeSearchState.query
      : activeSearchState?.kind === "tooShort"
        ? activeSearchState.query
      : searchPending
        ? trimmedSearchQuery
        : "";
  const searchHighlightQuery = activeSearchState?.kind === "results" ? searchResultsQuery : "";
  const searchIndexingPending =
    activeSearchState?.kind === "results" &&
    searchResults.length === 0 &&
    searchCrawlerHasPendingIndexing(snapshot.state.domain.search_crawler);
  const effectiveRightPanelMode = effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot);
  const rightPanelOpen = effectiveRightPanelMode !== "closed";
  const appGridStyle = {
    "--sidebar-width": `${sidebarWidth}px`,
    "--right-panel-width": `${rightPanelWidth}px`
  } as CSSProperties;

  function diagnosticReportFor(
    appSnapshot: DesktopSnapshot,
    runtimeSnapshot: DiagnosticLogSnapshot
  ): string {
    const localDiagnosticSnapshot = diagnosticLogBuffer.snapshot();
    return diagnosticReport({
      snapshot: appSnapshot,
      panelMode: effectiveRightPanelMode,
      sendStatus: qaSendStatus,
      timelineDiagnostics,
      domDiagnostics: qaRenderedDomDiagnostics(),
      uiLatencyDiagnostics,
      stateDeltaStats: getAppStoreDeltaStats(),
      timelineTransportStats: getTimelineTransportStats(),
      jsErrors: getRecentJsErrors(),
      logEntries: [...localDiagnosticSnapshot.entries, ...runtimeSnapshot.entries],
      droppedLogEntries: localDiagnosticSnapshot.droppedEntries + runtimeSnapshot.droppedEntries,
      slidingSyncDiagnostics: runtimeSnapshot.slidingSync,
      securityDiagnostics: qaSecurityDiagnostics()
    });
  }

  async function copyDiagnostics(appSnapshot: DesktopSnapshot) {
    const nextSnapshot = await api.getDiagnosticSnapshot();
    if (!navigator.clipboard) {
      throw new Error("clipboard unavailable");
    }
    await navigator.clipboard.writeText(diagnosticReportFor(appSnapshot, nextSnapshot));
  }

  function beginSidebarResize(event: PointerEvent<HTMLButtonElement>) {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = sidebarWidth;

    function onPointerMove(moveEvent: globalThis.PointerEvent) {
      setSidebarWidth(clampSidebarWidth(startWidth + moveEvent.clientX - startX));
    }

    function onPointerUp() {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    }

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  }

  function beginRightPanelResize(event: PointerEvent<HTMLButtonElement>) {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = rightPanelWidth;

    function onPointerMove(moveEvent: globalThis.PointerEvent) {
      setRightPanelWidth(
        clampRightPanelWidth(startWidth - (moveEvent.clientX - startX), sidebarWidth)
      );
    }

    function onPointerUp() {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    }

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  }

  function startWindowDrag() {
    if (!isTauriRuntime()) {
      return;
    }
    void getCurrentWindow().startDragging().catch(() => undefined);
  }

  return (
    <TimelineStoreContext.Provider value={timelineStoreContextValue}>
      <div
        className="desktop"
        data-density={displayDensity}
      >
        <TopBar
          accountManagementUrl={snapshot.state.domain.account_management_url ?? undefined}
          activeRoomName={activeRoom?.display_label ?? null}
          activeSpaceName={activeSpaceName}
          currentSessionStatus={snapshot.state.domain.current_session_status}
          deviceId={snapshot.state.domain.session.device_id ?? null}
          homeserver={snapshot.state.domain.session.homeserver ?? null}
          isBusy={isBusy}
          platform={snapshot.state.domain.locale_profile.platform}
          searchInputRef={searchInputRef}
          searchQuery={searchQuery}
          searchScope={searchScope}
          sync={snapshot.state.domain.sync}
          userId={snapshot.state.domain.session.user_id ?? null}
          onManageAccount={(safeExternalUrl) => {
            void openExternalHttpUrl(safeExternalUrl);
          }}
          onCopyDiagnostics={() => copyDiagnostics(snapshot)}
          onOpenKeyboardSettings={() => {
            void setRightPanelModeClosingFocusedContext("keyboardSettings");
          }}
          onOpenDiagnostics={() => {
            void openDiagnostics();
          }}
          onRefreshCurrentSessionStatus={(trigger) => {
            void api.refreshCurrentSessionStatus(trigger).then(setSnapshot);
          }}
          onRetryRuntimeAlert={(kind) => {
            if (kind === "secureBackup") {
              void retrySecureBackupInspection();
            }
          }}
          onRestartSync={restartSync}
          onSearchQueryChange={setSearchQuery}
          onSearchScopeChange={setSearchScope}
          onStartWindowDrag={startWindowDrag}
          runtimeAlertRetrying={secureBackupInspectionRetrying}
          runtimeAlerts={runtimeAlerts}
        />
      <div
        className={`app-grid ${rightPanelOpen ? "right-panel-open" : "thread-closed"}`}
        style={appGridStyle}
      >
        <WorkspaceRail
          snapshot={snapshot}
          spaceOverrides={spaceLocalOverrides}
          onCreateSpace={() => openCreateDialog("space")}
          onOpenContextMenu={openContextMenu}
          onOpenUserSettings={() => {
            void setRightPanelModeClosingFocusedContext("userSettings");
          }}
          onReorderSpaces={(spaceIds) => {
            void reorderSpaces(spaceIds);
          }}
          onSelectSpace={selectSpace}
        />
        <Sidebar
          activeRoomId={snapshot.state.ui.navigation.active_room_id}
          activeView={primaryView}
          snapshot={snapshot}
          spaceOverrides={spaceLocalOverrides}
          onCreateRoom={() => openCreateDialog("room")}
          onNewDm={openNewDmDialog}
          onOpenContextMenu={openContextMenu}
          onOpenActivity={() => {
            void openHomeActivityView();
          }}
          onOpenExplore={() => {
            void (homeContextActive ? openHomeExploreView() : openExploreView());
          }}
          onOpenInvites={() => {
            void (homeContextActive ? openHomeInvitesView() : openInvitesView());
          }}
          onOpenThreads={() => {
            void openThreadsListPanel(threadsListScope);
          }}
          onOpenSpaceInfo={() => {
            void setRightPanelModeClosingFocusedContext("spaceInfo");
          }}
          onOpenSpaceMembers={() => {
            void openSpaceMembers("sidebar");
          }}
          spaceMemberCounts={{
            joined: snapshot.state.domain.space_members.space_joined.length,
            childOnly: snapshot.state.domain.space_members.child_room_only.length
          }}
          onJoinRoom={(roomId) => {
            void joinRoom(roomId);
          }}
          onSelectRoom={selectRoom}
        />
        <button
          className="app-grid-resizer"
          type="button"
          aria-label={t("workspace.resizeRoomList")}
          onPointerDown={beginSidebarResize}
        />
        {rightPanelOpen ? (
          <button
            className="app-grid-right-resizer"
            type="button"
            aria-label={t("workspace.resizeRightPanel")}
            onPointerDown={beginRightPanelResize}
          />
        ) : null}
        {primaryView === "activity" ? (
          <ActivityPane
            activity={snapshot.state.domain.activity}
            onClose={() => {
              void closeActivityView();
            }}
            onLoadMore={(tab, cursor) => {
              void paginateActivity(tab, cursor);
            }}
            onMarkRead={(target) => {
              void markActivityRead(target);
            }}
            onOpenRow={(row) => {
              if (row.kind === "event" && row.event_id !== null) {
                openActivityRow(row.room_id, row.event_id, row.thread_root_event_id);
              } else if (row.kind === "roomUnread") {
                void openActivityRoom(row.room_id);
              }
            }}
            onRetryResolution={() => {
              void retryActivityResolution();
            }}
            onSetTab={(tab) => {
              void setActivityTab(tab);
            }}
          />
        ) : primaryView === "explore" ? (
          <ExplorePane
            addressDraft={directoryAddressDraft}
            addressNotice={directoryAddressNotice}
            isBusy={isBusy}
            queryDraft={directorySearchDraft}
            serverDraft={directoryServerDraft}
            snapshot={snapshot}
            onAddressChange={(value) => {
              setDirectoryAddressDraft(value);
              setDirectoryAddressNotice(null);
            }}
            onJoinRoom={(room) => {
              void joinDirectoryRoom(room);
            }}
            onQueryChange={setDirectorySearchDraft}
            onServerChange={setDirectoryServerDraft}
            onSearch={() => {
              void submitDirectorySearch();
            }}
            onSubmitAddress={() => {
              void submitDirectoryAddress();
            }}
          />
        ) : primaryView === "invites" ? (
          <InvitesPane
            isBusy={isBusy}
            snapshot={snapshot}
            onAcceptInvite={(roomId) => {
              void acceptInvite(roomId);
            }}
            onDeclineInvite={(roomId) => {
              void declineInvite(roomId);
            }}
            onNewDm={openNewDmDialog}
          />
        ) : (
          <TimelinePane
            activeRoomName={activeRoom?.display_label ?? t("room.noRoomSelected")}
            composerDocument={composerDocument}
            composerNotice={
              composerNotice &&
              "Room" in composerNotice.key.kind &&
              noticeMatchesMainComposer(
                composerNotice.key,
                snapshot.state.ui.timeline.room_id ?? "",
                readyComposerDraftAccountOwner(snapshot)?.userId ?? ""
              )
                ? composerNotice.message
                : null
            }
            composerDraftKey={mainComposerDraftImeKey}
            composerMode={composerModeProp(snapshot.state.ui.timeline.composer.mode)}
            canEdit={!encryptedComposerBlocked}
            resolveComposerKeyAction={resolveComposerKeyAction}
            searchQuery={searchHighlightQuery}
            searchResults={searchResults}
            showSearchResults={false}
            snapshot={snapshot}
            timelineTransport={appTimelineTransport}
            onReturnToLive={async () => {
              // #161: leave the anchored (jump-to-date) main-pane view. Closing
              // the focused context clears navigation.main_timeline_anchor in
              // Rust, so the main pane re-renders the live room timeline.
              setSnapshot(await api.closeFocusedContext());
            }}
            onCancelReply={() => {
              void cancelComposerReply();
            }}
            onCancelScheduledSend={(scheduledId) => {
              void cancelScheduledSend(scheduledId);
            }}
            onAttachFiles={stageUploadFiles}
            onClearUploadStaging={() => {
              void clearUploadStaging();
            }}
            onUpdateStagedUploadCaption={(stagedId, caption) => {
              void updateStagedUploadCaption(stagedId, caption);
            }}
            onSelectStagedUploadOutput={(stagedId, selection) => {
              void selectStagedUploadOutput(stagedId, selection);
            }}
            onSendStagedAttachments={() => {
              void sendStagedAttachments();
            }}
            onLoadStagedUploadPreview={loadStagedUploadPreview}
            onRetryStagedUploadPreparation={(stagedId) => {
              void retryStagedUploadPreparation(stagedId);
            }}
            onUseOriginalStagedUpload={(stagedId) => {
              void useOriginalStagedUpload(stagedId);
            }}
            onComposerDocumentChange={(document) => {
              void updateComposerDraft(document);
            }}
            onComposerMathModeChange={(enabled) => {
              void updateSettings({ composer: { math_mode: enabled } });
            }}
            onMentionQueryChange={(roomId, query) => {
              if (query !== null) {
                void (async () => {
                  await api.queryMentionCandidates(roomId, "main", query);
                  setSnapshot(await api.getSnapshot());
                })();
              }
            }}
            onOpenThread={openThread}
            onOpenMatrixTarget={(target) => {
              void openMatrixTarget(target);
            }}
            onReply={(roomId, eventId) => {
              void setComposerReplyTarget(roomId, eventId);
            }}
            onRescheduleScheduledSend={(scheduledId, body, sendAtMs) => {
              void rescheduleScheduledSend(scheduledId, body, sendAtMs);
            }}
            onScheduleSend={(sendAtMs, body) => {
              void scheduleSend(sendAtMs, body);
            }}
            onSendText={sendText}
            onEditMessage={editMessage}
            onOpenContextMenu={openContextMenu}
            onRedactMessage={redactMessage}
            onResultSelect={selectSearchResult}
            onSetLocalUserAlias={(userId, alias) => {
              void setLocalUserAlias(userId, alias);
            }}
            onOpenPinnedMessages={() => {
              const roomId = snapshot.state.ui.navigation.active_room_id;
              if (roomId) {
                void openPinnedMessagesPanel(roomId);
              }
            }}
            onOpenPeople={async () => {
              const roomId = snapshotRef.current?.state.ui.navigation.active_room_id;
              const navigationRequestId = roomNavigationRequestRef.current;
              const requestId = ++roomSettingsRequestRef.current;
              if (roomId) {
                roomSettingsLoadRef.current = null;
                const next = await api.loadRoomSettings(roomId);
                if (
                  roomSettingsRequestRef.current !== requestId ||
                  roomNavigationRequestRef.current !== navigationRequestId ||
                  snapshotRef.current?.state.ui.navigation.active_room_id !== roomId ||
                  next.state.ui.navigation.active_room_id !== roomId ||
                  !exactRoomSettingsForRoom(next, roomId)
                ) {
                  return;
                }
                setSnapshot(next);
                setPeoplePanelScope({ kind: "room", roomId });
              } else {
                setPeoplePanelScope(null);
              }
              setSelectedProfileUserId(null);
              await setRightPanelModeClosingFocusedContext("people");
            }}
            onOpenThreads={() => {
              const roomId = snapshot.state.ui.navigation.active_room_id;
              if (roomId) {
                void openThreadsListPanel({ kind: "room", room_id: roomId });
              }
            }}
            onToggleRoomInfo={() => {
              if (rightPanelOpen) {
                if (effectiveRightPanelMode === "thread") {
                  void closeThread();
                } else if (effectiveRightPanelMode === "roomInfo") {
                  void setRightPanelModeClosingFocusedContext("closed");
                } else {
                  void setRightPanelModeClosingFocusedContext("roomInfo");
                }
              } else {
                void setRightPanelModeClosingFocusedContext("roomInfo");
              }
            }}
            onTimelineDiagnosticsChange={updateTimelineDiagnostics}
            onTimelineDiagnosticLogEntry={appendDiagnosticLog}
          />
        )}
        <ContextualRightPanel
          activeRoom={activeRoom ?? null}
          activeSpace={activeSpace ?? null}
          activeSpaceName={activeSpaceName}
          accountManagementUrl={snapshot.state.domain.account_management_url}
          displayDensity={displayDensity}
          encryptedComposerBlocked={encryptedComposerBlocked}
          isRecoveryBusy={isBusy}
          mode={effectiveRightPanelMode}
          threadsListScope={openThreadsListScope}
          peoplePanelScope={peoplePanelScope}
          selectedProfileUserId={selectedProfileUserId}
          recoverySecretFilled={recoverySecretFilled}
          recoverySecretInputRef={recoverySecretRef}
          snapshot={snapshot}
          timelineTransport={appTimelineTransport}
          searchIndexingPending={searchIndexingPending}
          searchPending={searchPending}
          searchTooShortMinChars={searchTooShortMinChars}
          searchQuery={searchResultsQuery}
          searchResults={searchResults}
          savedSessions={savedSessions}
          onCloseThread={() => {
            void closeThread();
          }}
          onClosePanel={() => {
            void closeFocusedContextPanel();
          }}
          onOpenThread={(roomId, rootEventId, intent) => {
            void openThread(roomId, rootEventId, intent);
          }}
          onOpenFiles={(scope) => {
            void openFilesView(scope);
          }}
          onOpenPinnedEvent={(roomId, eventId, threadRootEventId) => {
            void openPinnedEvent(roomId, eventId, threadRootEventId);
          }}
          onUnpinPinnedEvent={(roomId, eventId) => {
            void unpinPinnedEvent(roomId, eventId);
          }}
          pinnedNavigation={pinnedNavigation}
          onRetryPinnedEvent={retryPinnedEvent}
          onOpenContextMenu={openContextMenu}
          onOpenSpaceMembers={
            activeSpace
              ? () => {
                  void openSpaceMembers("space_info");
                }
              : undefined
          }
          onDiagnostic={appendSpaceMembersDiagnosticLog}
          onInviteUserToSpace={(userId) => {
            void inviteUserToSpace(userId, "inline");
          }}
          onInviteSearchCandidateToSpace={(userId) => {
            void inviteUserToSpace(userId, "search");
          }}
          onSearchSpaceInviteTargets={searchSpaceInviteTargets}
          onResetSpaceInviteSearch={resetSpaceInviteSearch}
          canInviteToSpace={canInviteToSpace}
          spaceInviteAvailabilityReason={spaceInviteAvailabilityReason}
          onCancelInvite={(userId) => {
            void cancelSpaceInvite(userId, "inline");
          }}
          canCancelInvite={canCancelInvite}
          cancelAvailabilityReason={cancelAvailabilityReason}
          cancelInviteFailure={cancelInviteFailure}
          roleUpdateFailure={
            spaceMembersRoleTransportFailure !== null &&
            spaceMembersSnapshotMatches(snapshot, spaceMembersRoleTransportFailure)
          }
          onUpdateSpaceMemberRole={(userId, option) => {
            void updateSpaceMemberRole(userId, option);
          }}
          onReloadSpaceMemberRoles={() => {
            void reloadSpaceMemberRoles();
          }}
          onOpenPeople={async () => {
            const roomId = snapshotRef.current?.state.ui.navigation.active_room_id;
            const navigationRequestId = roomNavigationRequestRef.current;
            const requestId = ++roomSettingsRequestRef.current;
            if (roomId) {
              roomSettingsLoadRef.current = null;
              const next = await api.loadRoomSettings(roomId);
              if (
                roomSettingsRequestRef.current !== requestId ||
                roomNavigationRequestRef.current !== navigationRequestId ||
                snapshotRef.current?.state.ui.navigation.active_room_id !== roomId ||
                next.state.ui.navigation.active_room_id !== roomId ||
                !exactRoomSettingsForRoom(next, roomId)
              ) {
                return;
              }
              setSnapshot(next);
              setPeoplePanelScope({ kind: "room", roomId });
            } else {
              setPeoplePanelScope(null);
            }
            setSelectedProfileUserId(null);
            await setRightPanelModeClosingFocusedContext("people");
          }}
          onOpenProfile={(userId) => {
            setSelectedProfileUserId(userId);
            void setRightPanelModeClosingFocusedContext("profile");
          }}
          onBackToPeople={() => {
            setSelectedProfileUserId(null);
            void setRightPanelModeClosingFocusedContext("people");
          }}
          onRefreshFilesView={(scope, filter, sort) => {
            void refreshFilesView(scope, filter, sort);
          }}
          onPaginateThreadsList={(scope) => {
            void paginateThreadsList(scope);
          }}
          onOpenKeyboardSettings={() => {
            void setRightPanelModeClosingFocusedContext("keyboardSettings");
          }}
          onOpenRecovery={() => {
            void setRightPanelModeClosingFocusedContext("recovery");
          }}
          onManageAccount={() => {
            const url = snapshot.state.domain.account_management_url;
            if (url) {
              void openExternalHttpUrl(url);
            }
          }}
          onRefreshCurrentSessionStatus={() => {
            void api.refreshCurrentSessionStatus("open").then(setSnapshot);
          }}
          onProbeLocalEncryption={() => {
            void probeLocalEncryptionHealth();
          }}
          onResetLocalData={() => {
            setResetLocalDataConfirmOpen(true);
          }}
          onLogout={() => {
            void logout();
          }}
          onInviteUser={openInviteUserDialog}
          onReturnToInvite={() => {
            void returnToInviteUserDialog();
          }}
          onModerateMember={(roomId, targetUserId, action, reason) => {
            void moderateRoomMember(roomId, targetUserId, action, reason);
          }}
          onSetLocalUserAlias={(userId, alias) => {
            void setLocalUserAlias(userId, alias);
          }}
          onRequestMemberAvatarThumbnail={
            AVATAR_THUMBNAIL_DOWNLOADS_ENABLED
              ? requestMemberAvatarThumbnail
              : undefined
          }
          onSetRoomNotificationMode={(roomId, mode) => {
            void setRoomNotificationMode(roomId, mode);
          }}
          onRepairRoomTimeline={(roomId) => {
            void repairRoomTimeline(roomId);
          }}
          onStartDirectMessage={(userId) => {
            void startDirectMessage(userId);
          }}
          onUpdateMemberRole={(roomId, targetUserId, powerLevel) => {
            void updateRoomMemberRole(roomId, targetUserId, powerLevel);
          }}
          onReshareRoomKey={reshareRoomKey}
          onForceNewOutboundSession={forceNewOutboundSession}
          onShareIndex0RoomKey={shareIndex0RoomKey}
          onResendIndex0RoomKey={resendIndex0RoomKey}
          onRecoverySecretPresenceChange={setRecoverySecretFilled}
          onReply={(roomId, eventId) => {
            void setComposerReplyTarget(roomId, eventId);
          }}
          onResultSelect={selectSearchResult}
          onSubmitRecovery={submitRecovery}
          onSwitchAccount={(session) => {
            void switchAccount(session);
          }}
          onThreadComposerDocumentChange={(roomId, rootEventId, document) => {
            updateThreadComposerDraft(roomId, rootEventId, document);
          }}
          threadComposerDraftImeKey={threadComposerDraftImeKey}
          threadComposerDocumentOverride={threadComposerDocumentOverride}
          threadComposerNotice={
            composerNotice &&
            activeThreadTarget &&
            noticeMatchesThreadComposer(
              composerNotice.key,
              activeThreadTarget.room_id,
              activeThreadTarget.root_event_id,
              currentComposerAccount?.userId ?? ""
            )
              ? composerNotice.message
              : null
          }
          onThreadMentionQueryChange={(roomId, query) => {
            if (query !== null) {
              void (async () => {
                await api.queryMentionCandidates(roomId, "thread", query);
                setSnapshot(await api.getSnapshot());
              })();
            }
          }}
          onThreadAttachFiles={(roomId, rootEventId, files) => {
            void stageThreadUploadFiles(roomId, rootEventId, files);
          }}
          onThreadClearUploadStaging={(roomId, rootEventId) => {
            void clearThreadUploadStaging(roomId, rootEventId);
          }}
          onThreadUpdateStagedUploadCaption={(roomId, rootEventId, stagedId, caption) => {
            void updateThreadStagedUploadCaption(roomId, rootEventId, stagedId, caption);
          }}
          onThreadSelectStagedUploadOutput={(roomId, rootEventId, stagedId, selection) => {
            void selectThreadStagedUploadOutput(roomId, rootEventId, stagedId, selection);
          }}
          onThreadSendStagedAttachments={(roomId, rootEventId) => {
            void sendThreadStagedAttachments(roomId, rootEventId);
          }}
          onThreadLoadStagedUploadPreview={loadThreadStagedUploadPreview}
          onThreadRetryStagedUploadPreparation={(roomId, rootEventId, stagedId) => {
            void retryThreadStagedUploadPreparation(roomId, rootEventId, stagedId);
          }}
          onThreadUseOriginalStagedUpload={(roomId, rootEventId, stagedId) => {
            void useOriginalThreadStagedUpload(roomId, rootEventId, stagedId);
          }}
          onThreadScheduleSend={(roomId, rootEventId, sendAtMs, document) => {
            void scheduleThreadSend(roomId, rootEventId, sendAtMs, document);
          }}
          onThreadReplySend={(roomId, rootEventId, document) => {
            void sendThreadReply(roomId, rootEventId, document);
          }}
          onTimelineDiagnosticLogEntry={appendDiagnosticLog}
          onResolveComposerKeyAction={resolveComposerKeyAction}
          onAcceptVerification={(flowId) => {
            void acceptVerification(flowId);
          }}
          onBootstrapCrossSigning={() => {
            void bootstrapCrossSigning();
          }}
          onCancelVerification={(flowId) => {
            void cancelVerification(flowId);
          }}
          onConfirmSasVerification={(flowId) => {
            void confirmSasVerification(flowId);
          }}
          onChooseRoomKeyExportDestination={chooseRoomKeyExportDestination}
          onChooseRoomKeyImportSource={chooseRoomKeyImportSource}
          onChooseSecureBackupDestination={chooseSecureBackupDestination}
          onExportRoomKeys={(destinationPath, passphrase) => {
            void exportRoomKeys(destinationPath, passphrase);
          }}
          onImportRoomKeys={(sourcePath, passphrase) => {
            void importRoomKeys(sourcePath, passphrase);
          }}
          onBootstrapSecureBackup={(passphrase, recoveryKeyDestinationPath) => {
            void bootstrapSecureBackup(passphrase, recoveryKeyDestinationPath);
          }}
          onChangeSecureBackupPassphrase={(
            oldSecret,
            newPassphrase,
            recoveryKeyDestinationPath
          ) => {
            void changeSecureBackupPassphrase(
              oldSecret,
              newPassphrase,
              recoveryKeyDestinationPath
            );
          }}
          onEnableKeyBackup={() => {
            void enableKeyBackup();
          }}
          onResetIdentity={() => {
            void resetIdentity();
          }}
          onCancelIdentityReset={(flowId) => {
            void cancelIdentityReset(flowId);
          }}
          onSubmitIdentityResetOAuth={(flowId) => {
            void submitIdentityResetOAuth(flowId);
          }}
          onSubmitIdentityResetPassword={(flowId, password) => {
            void submitIdentityResetPassword(flowId, password);
          }}
          onSetAvatar={(file) => {
            void setAvatar(file);
          }}
          onSetDisplayName={(displayName) => {
            void setDisplayName(displayName);
          }}
          onUpdateSettings={(patch) => {
            void updateSettings(patch);
          }}
          onSetRoomUrlPreviewOverride={(roomId, enabled) => {
            void setRoomUrlPreviewOverride(roomId, enabled);
          }}
          onLoadAccountManagementCapabilities={() => {
            void loadAccountManagementCapabilities();
          }}
          onChangePassword={(newPassword) => {
            void changePassword(newPassword);
          }}
          onDeactivateAccount={(eraseData) => {
            void deactivateAccount(eraseData);
          }}
          onSubmitAccountManagementUia={(flowId, password) => {
            void submitAccountManagementUia(flowId, password);
          }}
          onUpdateRoomSetting={(roomId, change) => {
            void updateRoomSetting(roomId, change);
          }}
          onIgnoreUser={(userId) => {
            void ignoreUser(userId);
          }}
          onUnignoreUser={(userId) => {
            void unignoreUser(userId);
          }}
          onReportUser={(userId) => {
            openReportDialog({ kind: "user", userId });
          }}
          onStartCrawlRoom={(roomId) => {
            void startRoomCrawl(roomId);
          }}
          onStopCrawlRoom={(roomId) => {
            void stopRoomCrawl(roomId);
          }}
          onRebuildSearchIndex={() => {
            void rebuildSearchIndex();
          }}
          onDisplayDensityChange={setDisplayDensity}
          onSetSpaceLocalOverride={updateSpaceLocalOverride}
          spaceLocalOverrides={spaceLocalOverrides}
        />
      </div>
      {contextMenu ? (
        <ContextMenuSurface
          items={contextMenu.items}
          x={contextMenu.x}
          y={contextMenu.y}
          onAction={runContextMenuAction}
          onClose={() => setContextMenu(null)}
        />
      ) : null}
      {snapshot.state.domain.directory.preview.kind !== "closed" ? (
        <DirectoryPreviewDialog
          isBusy={isBusy || snapshot.state.domain.directory.join.kind === "joining"}
          preview={snapshot.state.domain.directory.preview}
          onCancel={() => {
            void dismissDirectoryPreview();
          }}
          onConfirm={() => {
            void confirmDirectoryJoin();
          }}
        />
      ) : null}
      {createDialog ? (
        <CreateEntityDialog
          activeSpaceName={activeSpaceName}
          isBusy={isBusy || snapshot.state.ui.basic_operation.kind !== "idle"}
          kind={createDialog}
          roomOptions={createRoomDraftOptions}
          value={createDraftName}
          onCancel={closeCreateDialog}
          onRoomOptionsChange={setCreateRoomDraftOptions}
          onSubmit={() => {
            void submitCreateDialog();
          }}
          onValueChange={setCreateDraftName}
        />
      ) : null}
      {newDmDialogOpen ? (
        <UserIdDialog
          isBusy={isBusy}
          inputLabel={t("dialog.matrixUserId")}
          submitLabel={t("dialog.startDm")}
          title={t("dialog.newDmTitle")}
          value={newDmDraftUserId}
          onCancel={closeNewDmDialog}
          onSubmit={() => {
            void submitNewDmDialog();
          }}
          onValueChange={setNewDmDraftUserId}
        />
      ) : null}
      {inviteUserDialog && inviteUserDialogVisible ? (
        <InviteTargetsDialog
          isBusy={isBusy}
          query={inviteUserDraftQuery}
          title={inviteUserDialog.title}
          workflow={snapshot?.state.domain.invite_workflow ?? DEFAULT_INVITE_WORKFLOW}
          onCancel={() => {
            void closeInviteUserDialog();
          }}
          onOpenRecovery={() => {
            void openRecoveryFromInvite();
          }}
          onQueryChange={(value) => {
            void updateInviteUserQuery(value);
          }}
          onRemoveTarget={(userId) => {
            void removeInviteTarget(userId);
          }}
          onOpenRoomInfo={() => {
            void openRoomInfoFromInvite();
          }}
          onScopeChange={(scope) => {
            void selectInviteScope(scope);
          }}
          onSelectCandidate={(userId) => {
            void selectInviteTarget(userId);
          }}
          onSubmit={() => {
            void submitInviteUserDialog();
          }}
        />
      ) : null}
      {reportDialog ? (
        <ReportReasonDialog
          reason={reportReasonDraft}
          title={t("dialog.reportReasonTitle")}
          onCancel={closeReportDialog}
          onReasonChange={setReportReasonDraft}
          onSubmit={submitReportDialog}
        />
      ) : null}
      {resetLocalDataConfirmOpen ? (
        <ResetLocalDataConfirmationDialog
          isBusy={snapshot.state.domain.local_encryption.kind === "resetting"}
          onCancel={() => setResetLocalDataConfirmOpen(false)}
          onConfirm={() => {
            void resetLocalData();
          }}
        />
      ) : null}
      {pendingRoomLeave ? (
        <ResetLocalDataConfirmationDialog
          isBusy={roomLeaveInFlight}
          title={t(
            pendingRoomLeave.isDm ? "room.leaveConfirmTitleDm" : "room.leaveConfirmTitle",
            { name: roomLeaveDisplayName(pendingRoomLeave.roomId) }
          )}
          copy={t(
            pendingRoomLeave.isDm ? "room.leaveConfirmCopyDm" : "room.leaveConfirmCopy",
            { name: roomLeaveDisplayName(pendingRoomLeave.roomId) }
          )}
          confirmLabel={t(
            pendingRoomLeave.isDm ? "room.leaveConfirmActionDm" : "room.leaveConfirmAction"
          )}
          onCancel={() => setPendingRoomLeave(null)}
          onConfirm={() => {
            void leavePendingRoom();
          }}
        />
      ) : null}
      {diagnosticsOpen ? (() => {
        return <DiagnosticDialog
          report={diagnosticReportFor(snapshot, runtimeDiagnosticSnapshot)}
          onClose={() => setDiagnosticsOpen(false)}
        />;
      })() : null}
      </div>
    </TimelineStoreContext.Provider>
  );
}

// Preserve App.tsx's original public export surface; these components now live in
// dedicated modules under ./components.
export { Composer } from "./components/composer";
export { ContextualRightPanel } from "./components/rightPanel";
export { TopBar, WorkspaceRail } from "./components/Shell";
export { ResetLocalDataConfirmationDialog } from "./components/dialogs";
export {
  SessionVerificationGate,
  type SessionVerificationGateOperations
} from "./components/SessionVerificationGate";
