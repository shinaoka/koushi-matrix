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
import { invoke } from "@tauri-apps/api/core";
// eslint-disable-next-line no-restricted-imports
import { listen } from "@tauri-apps/api/event";
// eslint-disable-next-line no-restricted-imports
import { getCurrentWindow } from "@tauri-apps/api/window";
// eslint-disable-next-line no-restricted-imports
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { createDesktopApi } from "./backend/client";
import { setActiveLocaleProfile, t } from "./i18n/messages";
import { ContextMenuSurface } from "./components/ContextMenuSurface";
import {
  type TimelineDiagnosticLogEntry,
  type TimelineDiagnostics,
  type TimelineTransport
} from "./components/TimelineView";
import {
  type CoreEventPayload,
  type TimelineKey,
  focusedTimelineKey,
  roomTimelineKey,
  threadTimelineKey
} from "./domain/coreEvents";
import {
  applyGlobalResync,
  applyTimelineEventWithRetention,
  createTimelineStore,
  pruneTimelineStore,
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
  applyDesktopAttentionToWindow,
  dispatchDesktopAttentionTransientEffects,
  desktopAttentionSummary,
  desktopAttentionWindowTitle,
  desktopAttentionNotificationCandidate
} from "./domain/desktopAttention";
import {
  clearDesktopAttentionNotifications,
  createTauriDesktopNotificationTransport,
  sendDesktopAttentionNotification
} from "./domain/desktopNotification";
import {
  qaDomDiagnosticTokens,
  qaTimelineDiagnosticTokens,
  qaWindowTitle,
  type QaDomDiagnostics,
  type QaTimelineDiagnostics
} from "./domain/qaTitle";
import {
  appendDiagnosticLogEntry,
  diagnosticReport,
  type DiagnosticLogEntry,
  type SecurityDiagnostics
} from "./domain/diagnostics";
import {
  createUiLatencySampler,
  EMPTY_UI_LATENCY_DIAGNOSTICS,
  type UiLatencyDiagnostics
} from "./domain/uiLatency";
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
  planSnapshotAvatarThumbnailRequests
} from "./domain/avatarThumbnails";
import type {
  ActivityMarkReadTarget,
  ActivityTab,
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  CreateRoomRequest,
  DesktopSnapshot,
  DirectoryRoomSummary,
  FilesViewScope,
  ImageUploadCompressionMode,
  ImageUploadCompressionPolicy,
  InviteScopeSelection,
  InviteWorkflowState,
  MentionIntent,
  ResolveComposerKeyAction,
  RoomModerationAction,
  RoomNotificationMode,
  RoomSettingChange,
  SavedSessionInfo,
  SearchScopeKind,
  SettingsPatch,
  StagedUploadCompressionChoice,
  StagedUploadItem,
  TimelineScrollAnchor,
  UploadStagingRequestItem
} from "./domain/types";
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
import {
  applyAppStoreDelta,
  getAppStoreDeltaStats,
  selectSnapshot,
  setAppStoreSnapshot,
  useAppStore
} from "./domain/appStore";
import { getRecentJsErrors } from "./domain/jsErrorLog";
import { getTimelineTransportStats } from "./domain/timelineTransportStats";
import { openExternalHttpUrl } from "./domain/externalLinks";

import {
  EMPTY_MENTION_INTENT,
  captionBody,
  composerModeProp,
  pruneMentionIntentForDraft,
  serverNameFromAlias,
  serverNameFromRoomId,
  type ActiveContextMenu,
  type ContextMenuTarget,
  type ImageCompressionDialogState,
  type ImageCompressionPlan,
  type ImageCompressionVariant,
  type ImageUploadDimensionsPayload,
  type ImageUploadVariantInfoPayload,
  type ImageUploadVariantKindPayload,
  type PreparedMediaUpload,
  type PrimaryView,
  type UploadMediaThumbnailPayload
} from "./app/uiShared";
import {
  ActivityPane,
  ExplorePane,
  InvitesPane,
  TimelinePane
} from "./components/panes";
import { AuthScreen } from "./components/auth";
import {
  CreateEntityDialog,
  type CreateRoomDialogOptions,
  DiagnosticDialog,
  ImageCompressionDialog,
  InviteTargetsDialog,
  ReportReasonDialog,
  UserIdDialog
} from "./components/dialogs";
import {
  TopBar,
  WorkspaceRail,
  Sidebar
} from "./components/Shell";
import { ContextualRightPanel } from "./components/rightPanel";

const api = createDesktopApi();
const DEFAULT_HOMESERVER = "https://matrix.org";
const MENU_EVENT_NAME = "koushi-desktop://menu";
const STATE_EVENT_NAME = "koushi-desktop://state";
const CORE_EVENT_NAME = "koushi-desktop://event";
const STATE_EVENT_REFRESH_DEBOUNCE_MS = 250;
const INITIAL_TIMELINE_DIAGNOSTICS: QaTimelineDiagnostics = {
  visibleItems: 0,
  downloadedItems: 0,
  backfill: "unknown",
  avatarMxcItems: 0,
  avatarReadyItems: 0,
  avatarPendingItems: 0,
  avatarFailedItems: 0,
  avatarMissingItems: 0,
  avatarRenderedImages: 0,
  avatarBrokenImages: 0
};
let tauriCoreEventListenerReady: Promise<void> = Promise.resolve();

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

/**
 * Tauri transport for the event-driven timeline (Async rule 4: timeline data
 * flows ONLY as CoreEvent diffs over `koushi-desktop://event`; AppState
 * snapshots never embed item lists). Null in browser preview mode, where the
 * fixture snapshot rendering below is used instead.
 */
const tauriTimelineTransport: TimelineTransport | null = isTauriRuntime()
  ? {
      listenCoreEvents(listener: (payload: CoreEventPayload) => void) {
        let disposed = false;
        let unlisten: (() => void) | null = null;
        tauriCoreEventListenerReady = listen<CoreEventPayload>(CORE_EVENT_NAME, (event) => {
          listener(event.payload);
        }).then((dispose) => {
          if (disposed) {
            dispose();
          } else {
            unlisten = dispose;
          }
        });
        void tauriCoreEventListenerReady;
        return () => {
          disposed = true;
          unlisten?.();
        };
      },
      async ensureSubscribed(timelineKey: TimelineKey) {
        await tauriCoreEventListenerReady;
        await invoke("ensure_timeline_subscribed", { timelineKey });
      },
      async paginateBackwards(timelineKey: TimelineKey) {
        if ("Room" in timelineKey.kind) {
          await invoke("paginate_timeline_backwards", {
            roomId: timelineKey.kind.Room.room_id
          });
          return;
        }
        if ("Thread" in timelineKey.kind) {
          await invoke("paginate_thread_timeline_backwards", {
            roomId: timelineKey.kind.Thread.room_id,
            rootEventId: timelineKey.kind.Thread.root_event_id
          });
        }
      },
      async sendReaction(roomId: string, eventId: string, reactionKey: string) {
        await invoke("send_reaction", { roomId, eventId, reactionKey });
      },
      async retrySend(roomId: string, transactionId: string) {
        await invoke("retry_send", { roomId, transactionId });
      },
      async cancelSend(roomId: string, transactionId: string) {
        await invoke("cancel_send", { roomId, transactionId });
      },
      async redactReaction(
        roomId: string,
        eventId: string,
        reactionKey: string,
        reactionEventId: string
      ) {
        await invoke("redact_reaction", {
          roomId,
          eventId,
          reactionKey,
          reactionEventId
        });
      },
      async sendReadReceipt(roomId: string, eventId: string) {
        await invoke("send_read_receipt", { roomId, eventId });
      },
      async setFullyRead(roomId: string, eventId: string) {
        await invoke("set_fully_read", { roomId, eventId });
      },
      async setTyping(roomId: string, isTyping: boolean) {
        await invoke("set_typing", { roomId, isTyping });
      },
      async editMessage(roomId: string, eventId: string, body: string) {
        await invoke("edit_message", { roomId, eventId, body });
      },
      async redactMessage(roomId: string, eventId: string) {
        await invoke("redact_message", { roomId, eventId });
      },
      async pinEvent(roomId: string, eventId: string) {
        await invoke("pin_event", { roomId, eventId });
      },
      async unpinEvent(roomId: string, eventId: string) {
        await invoke("unpin_event", { roomId, eventId });
      },
      async downloadMedia(roomId: string, eventId: string) {
        await invoke("download_media", { roomId, eventId });
      },
      async saveMediaFile(sourceUrl: string, filename: string) {
        await saveReadyMediaFile(sourceUrl, filename);
      },
      async downloadAvatarThumbnail(mxcUri: string) {
        await invoke("download_avatar_thumbnail", { mxcUri });
      },
      async loadMessageSource(roomId: string, eventId: string) {
        await invoke("load_message_source", { roomId, eventId });
      },
      async requestRoomKey(roomId: string, eventId: string) {
        await invoke("request_room_key", { roomId, eventId });
      },
      async forwardMessage(
        roomId: string,
        sourceEventId: string,
        destinationRoomId: string
      ) {
        await invoke("forward_message", { roomId, sourceEventId, destinationRoomId });
      },
      async loadLinkPreviews(roomId: string, eventId: string) {
        await invoke("load_link_previews", { roomId, eventId });
      },
      async hideLinkPreview(roomId: string, eventId: string) {
        await invoke("hide_link_preview", { roomId, eventId });
      },
      async observeViewport(
        roomId: string,
        firstVisibleEventId: string | null,
        lastVisibleEventId: string | null,
        atBottom: boolean
      ) {
        await invoke("observe_timeline_viewport", {
          roomId,
          firstVisibleEventId,
          lastVisibleEventId,
          atBottom
        });
      },
      async updateScrollAnchor(roomId: string, anchor: TimelineScrollAnchor) {
        await invoke("update_navigation_scroll_anchor", { roomId, anchor });
      },
      async openAtTimestamp(roomId: string, timestampMs: number) {
        await invoke("open_timeline_at_timestamp", { roomId, timestampMs });
      }
    }
  : null;
const tauriNotificationTransport = isTauriRuntime()
  ? createTauriDesktopNotificationTransport()
  : null;
type ReportDialogState =
  | { kind: "user"; userId: string }
  | { kind: "content"; roomId: string; eventId: string }
  | { kind: "room"; roomId: string };
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
  operation: { kind: "idle" }
};

function inviteScopeKey(scope: InviteScopeSelection): string {
  return scope.kind === "roomOnly" ? "roomOnly" : `parent:${scope.space_id}`;
}

function inviteScopeFromWorkflow(workflow: InviteWorkflowState): InviteScopeSelection {
  return workflow.scope_plan?.default_scope ?? DEFAULT_INVITE_SCOPE;
}

async function prepareMediaUpload(
  file: File,
  mode: ImageUploadCompressionMode,
  policy: ImageUploadCompressionPolicy,
  chooseImageVariant: (
    plan: ImageCompressionPlan
  ) => Promise<ImageUploadVariantKindPayload | "cancel">
): Promise<PreparedMediaUpload | null> {
  const originalBytes = await bytesFromFile(file);
  if (originalBytes.length === 0) {
    return null;
  }

  if (!isImageCompressionCandidate(file)) {
    return {
      filename: file.name || "attachment",
      mimeType: file.type || "application/octet-stream",
      bytes: originalBytes
    };
  }

  const loaded = await loadImageElement(file).catch(() => null);
  if (!loaded) {
    return {
      filename: file.name || "attachment",
      mimeType: file.type || "application/octet-stream",
      bytes: originalBytes
    };
  }

  const originalDimensions = loaded.dimensions;
  const originalThumbnail = await thumbnailForImageElement(loaded.image, originalDimensions);
  const originalVariant: ImageCompressionVariant = {
    filename: file.name || "attachment",
    mimeType: file.type || "image/png",
    bytes: originalBytes,
    byteCount: originalBytes.length,
    dimensions: originalDimensions,
    previewUrl: loaded.objectUrl,
    thumbnail: originalThumbnail
  };
  const skippedSmallImage = imageCompressionShouldSkip(originalVariant, policy);
  let plan: ImageCompressionPlan | null = null;

  try {
    if (mode === "never" || skippedSmallImage) {
      return preparedImageUploadFromChoice(
        {
          mode,
          policy,
          original: originalVariant,
          compressed: originalVariant,
          skippedSmallImage
        },
        "Original"
      );
    }

    const compressed = await compressedImageVariantForElement(
      loaded.image,
      originalDimensions,
      file.name || "attachment",
      policy
    );
    plan = {
      mode,
      policy,
      original: originalVariant,
      compressed,
      skippedSmallImage: false
    };

    const choice = mode === "always" ? "Compressed" : await chooseImageVariant(plan);
    if (choice === "cancel") {
      return null;
    }
    return preparedImageUploadFromChoice(plan, choice);
  } finally {
    if (!plan || mode !== "ask") {
      releaseImageCompressionPlan(plan ?? {
        mode,
        policy,
        original: originalVariant,
        compressed: originalVariant,
        skippedSmallImage
      });
    }
  }
}

function preparedImageUploadFromChoice(
  plan: ImageCompressionPlan,
  choice: ImageUploadVariantKindPayload
): PreparedMediaUpload {
  const selected = choice === "Compressed" ? plan.compressed : plan.original;
  return {
    filename: selected.filename,
    mimeType: selected.mimeType,
    bytes: selected.bytes,
    imageDimensions: selected.dimensions,
    imageCompression: {
      mode: plan.mode,
      policy: plan.policy,
      original: variantInfoForUpload(plan.original),
      selected: variantInfoForUpload(selected),
      selected_variant: choice,
      skipped_small_image: plan.skippedSmallImage,
      metadata_stripped: choice === "Compressed",
      thumbnail_refreshed: true
    },
    thumbnail: selected.thumbnail
  };
}

function variantInfoForUpload(variant: ImageCompressionVariant): ImageUploadVariantInfoPayload {
  return {
    mime_type: variant.mimeType,
    byte_count: variant.byteCount,
    dimensions: variant.dimensions
  };
}

async function bytesFromFile(file: File): Promise<number[]> {
  return Array.from(new Uint8Array(await file.arrayBuffer()));
}

function safeDownloadFilename(filename: string): string {
  const trimmed = filename.trim();
  return (trimmed || "download").replace(/[\\/:*?"<>|]+/g, "_");
}

async function saveReadyMediaFile(sourceUrl: string, filename: string): Promise<void> {
  if (!isTauriRuntime()) {
    return;
  }
  const safeFilename = safeDownloadFilename(filename);
  const defaultPath = await invoke<string>("default_media_save_path", {
    filename: safeFilename
  }).catch(() => safeFilename);
  const selected = await saveDialog({
    title: t("timeline.downloadMedia", { filename: safeFilename }),
    defaultPath
  });
  if (!selected) {
    return;
  }
  await invoke("save_downloaded_media", {
    sourceUrl,
    destinationPath: selected
  });
}

function isImageCompressionCandidate(file: File): boolean {
  return ["image/jpeg", "image/png", "image/webp"].includes(file.type.toLowerCase());
}

async function loadImageElement(
  file: File
): Promise<{ image: HTMLImageElement; objectUrl: string; dimensions: ImageUploadDimensionsPayload }> {
  const objectUrl = URL.createObjectURL(file);
  const image = new globalThis.Image();
  image.decoding = "async";
  image.src = objectUrl;
  try {
    await image.decode();
  } catch (error) {
    URL.revokeObjectURL(objectUrl);
    throw error;
  }
  return {
    image,
    objectUrl,
    dimensions: {
      width: image.naturalWidth,
      height: image.naturalHeight
    }
  };
}

function imageCompressionShouldSkip(
  variant: ImageCompressionVariant,
  policy: ImageUploadCompressionPolicy
): boolean {
  return (
    variant.byteCount <= policy.threshold_bytes &&
    Math.max(variant.dimensions.width, variant.dimensions.height) <= policy.threshold_long_edge
  );
}

async function compressedImageVariantForElement(
  image: HTMLImageElement,
  originalDimensions: ImageUploadDimensionsPayload,
  originalFilename: string,
  policy: ImageUploadCompressionPolicy
): Promise<ImageCompressionVariant> {
  const dimensions = targetImageDimensions(originalDimensions, policy.target_long_edge);
  const mimeType = "image/jpeg";
  const blob = await imageBlobForElement(
    image,
    dimensions,
    mimeType,
    policy.quality_percent / 100
  );
  const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
  return {
    filename: imageFilenameWithExtension(originalFilename, "jpg"),
    mimeType,
    bytes,
    byteCount: bytes.length,
    dimensions,
    previewUrl: URL.createObjectURL(blob),
    thumbnail: await thumbnailForImageElement(image, dimensions)
  };
}

async function thumbnailForImageElement(
  image: HTMLImageElement,
  sourceDimensions: ImageUploadDimensionsPayload
): Promise<UploadMediaThumbnailPayload> {
  const dimensions = targetImageDimensions(sourceDimensions, 320);
  const blob = await imageBlobForElement(image, dimensions, "image/jpeg", 0.78);
  return {
    mime_type: "image/jpeg",
    bytes: Array.from(new Uint8Array(await blob.arrayBuffer())),
    width: dimensions.width,
    height: dimensions.height
  };
}

async function imageBlobForElement(
  image: HTMLImageElement,
  dimensions: ImageUploadDimensionsPayload,
  mimeType: string,
  quality: number
): Promise<Blob> {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, dimensions.width);
  canvas.height = Math.max(1, dimensions.height);
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("2d canvas unavailable");
  }
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob) {
          resolve(blob);
        } else {
          reject(new Error("image encode failed"));
        }
      },
      mimeType,
      quality
    );
  });
}

function targetImageDimensions(
  dimensions: ImageUploadDimensionsPayload,
  targetLongEdge: number
): ImageUploadDimensionsPayload {
  const longEdge = Math.max(dimensions.width, dimensions.height);
  if (longEdge <= 0 || longEdge <= targetLongEdge) {
    return dimensions;
  }
  const scale = targetLongEdge / longEdge;
  return {
    width: Math.max(1, Math.round(dimensions.width * scale)),
    height: Math.max(1, Math.round(dimensions.height * scale))
  };
}

function imageFilenameWithExtension(filename: string, extension: string): string {
  const fallback = `attachment.${extension}`;
  if (!filename.trim()) {
    return fallback;
  }
  const dot = filename.lastIndexOf(".");
  if (dot <= 0) {
    return `${filename}.${extension}`;
  }
  return `${filename.slice(0, dot)}.${extension}`;
}

function releaseImageCompressionPlan(plan: ImageCompressionPlan) {
  URL.revokeObjectURL(plan.original.previewUrl);
  if (plan.compressed.previewUrl !== plan.original.previewUrl) {
    URL.revokeObjectURL(plan.compressed.previewUrl);
  }
}

function createStagedUploadId(index: number): string {
  const random =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  return `staged-upload-${index}-${random}`;
}

function stagedUploadKindForFile(file: File): StagedUploadItem["kind"] {
  return file.type.toLowerCase().startsWith("image/")
    ? { kind: "image", width: null, height: null }
    : { kind: "file" };
}

function initialStagedCompressionChoice(
  file: File,
  mode: ImageUploadCompressionMode
): StagedUploadCompressionChoice {
  if (!isImageCompressionCandidate(file)) {
    return { kind: "notApplicable" };
  }
  if (mode === "always") {
    return { kind: "compressed", mode };
  }
  if (mode === "ask") {
    return { kind: "ask" };
  }
  return { kind: "original" };
}

function forcedUploadMode(
  choice: StagedUploadCompressionChoice | undefined,
  fallback: ImageUploadCompressionMode
): ImageUploadCompressionMode {
  if (choice?.kind === "compressed") {
    return "always";
  }
  if (choice?.kind === "ask") {
    return "ask";
  }
  if (choice?.kind === "original") {
    return "never";
  }
  return fallback;
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

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
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

function verboseDiagnosticsEnabled(): boolean {
  return import.meta.env.VITE_KOUSHI_VERBOSE_DIAGNOSTICS === "1";
}

function qaRenderedDomDiagnostics(): QaDomDiagnostics {
  const root = document.getElementById("root");
  const screen = document.querySelector('[data-testid="boot-error"]')
    ? "boot_error"
    : document.querySelector('[data-testid="auth-screen"]')
      ? "auth"
      : document.querySelector('[data-testid="recovery-panel"]')
        ? "recovery"
        : document.querySelector('[data-testid="timeline-view"]')
          ? "timeline"
          : root?.childElementCount
            ? "unknown"
            : "empty";

  return {
    screen,
    rootChildren: root?.childElementCount ?? 0,
    bodyTextLength: document.body.innerText.length
  };
}

function qaSecurityDiagnostics(): SecurityDiagnostics {
  const avatarImages = Array.from(
    document.querySelectorAll<HTMLImageElement>(
      ".avatar img, .room-avatar img, .space-avatar img, .receipt-reader-avatar img"
    )
  );
  return {
    secureContext: window.isSecureContext,
    locationProtocol: window.location.protocol,
    locationOrigin: window.location.origin,
    avatarImageSchemes: avatarImages.reduce<Record<string, number>>((counts, image) => {
      const scheme = imageSrcScheme(image.currentSrc || image.src);
      counts[scheme] = (counts[scheme] ?? 0) + 1;
      return counts;
    }, {}),
    avatarBrokenImages: avatarImages.filter((image) => !image.complete || image.naturalWidth === 0)
      .length
  };
}

function imageSrcScheme(src: string): string {
  try {
    const protocol = new URL(src, window.location.href).protocol;
    return protocol.endsWith(":") ? protocol.slice(0, -1) : protocol;
  } catch {
    return "invalid";
  }
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

function retainedTimelineStoreKeyIds(snapshot: DesktopSnapshot | null): Set<string> {
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

function useUiLatencyDiagnostics(): UiLatencyDiagnostics {
  const [diagnostics, setDiagnostics] = useState<UiLatencyDiagnostics>(
    EMPTY_UI_LATENCY_DIAGNOSTICS
  );

  useEffect(() => {
    if (typeof window.requestAnimationFrame !== "function") {
      return;
    }
    const sampler = createUiLatencySampler();
    let frameId = 0;
    let lastFrameAt = 0;
    let lastPublishedAt = 0;
    let cancelled = false;

    const publishIfChanged = (next: UiLatencyDiagnostics) => {
      setDiagnostics((current) =>
        current.samples === next.samples &&
        current.lastFrameGapMs === next.lastFrameGapMs &&
        current.averageFrameGapMs === next.averageFrameGapMs &&
        current.maxFrameGapMs === next.maxFrameGapMs &&
        current.longFrameCount === next.longFrameCount
          ? current
          : next
      );
    };

    const tick = (now: number) => {
      if (cancelled) {
        return;
      }
      if (lastFrameAt === 0) {
        lastFrameAt = now;
        lastPublishedAt = now;
      } else {
        const next = sampler.recordFrame(now - lastFrameAt);
        lastFrameAt = now;
        if (now - lastPublishedAt >= 1000) {
          lastPublishedAt = now;
          publishIfChanged(next);
        }
      }
      frameId = window.requestAnimationFrame(tick);
    };

    frameId = window.requestAnimationFrame(tick);
    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frameId);
    };
  }, []);

  return diagnostics;
}

export function App() {
  const snapshot = useAppStore(selectSnapshot);
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
      console.error(
        `Koushi snapshot schema_version ${next.state.schema_version} != expected ` +
          `${SNAPSHOT_SCHEMA_VERSION}: stale or mismatched IPC contract.`
      );
      setSchemaMismatchVersion(next.state.schema_version ?? -1);
      return;
    }
    setSchemaMismatchVersion(null);
    setAppStoreSnapshot(next);
  }, []);
  const [searchQuery, setSearchQuery] = useState(() => initialSearchQuery());
  const [searchScope, setSearchScope] = useState<SearchScopeKind>("allRooms");
  const [composerMentions, setComposerMentions] = useState<MentionIntent>(EMPTY_MENTION_INTENT);
  const [localThreadComposerDrafts, setLocalThreadComposerDrafts] = useState<Record<string, string>>({});
  const stagedUploadFilesRef = useRef<Map<string, File>>(new Map());
  const [imageCompressionDialog, setImageCompressionDialog] =
    useState<ImageCompressionDialogState | null>(null);
  const [loginHomeserver, setLoginHomeserver] = useState(DEFAULT_HOMESERVER);
  const [loginUsername, setLoginUsername] = useState("");
  const [loginDeviceName, setLoginDeviceName] = useState("Koushi");
  const [loginPasswordFilled, setLoginPasswordFilled] = useState(false);
  const [recoverySecretFilled, setRecoverySecretFilled] = useState(false);
  const [rightPanelMode, setRightPanelMode] = useState<RightPanelMode>("closed");
  const [selectedProfileUserId, setSelectedProfileUserId] = useState<string | null>(null);
  const [peoplePanelScope, setPeoplePanelScope] = useState<PeoplePanelScope | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);
  const [rightPanelWidth, setRightPanelWidth] = useState(DEFAULT_RIGHT_PANEL_WIDTH);
  const [qaSendStatus, setQaSendStatus] = useState<QaSendSmokeStatus>("idle");
  const [timelineDiagnostics, setTimelineDiagnostics] =
    useState<QaTimelineDiagnostics>(INITIAL_TIMELINE_DIAGNOSTICS);
  const timelineDiagnosticsRef = useRef<QaTimelineDiagnostics>(INITIAL_TIMELINE_DIAGNOSTICS);
  const [diagnosticLogEntries, setDiagnosticLogEntries] = useState<DiagnosticLogEntry[]>([]);
  const [savedSessions, setSavedSessions] = useState<SavedSessionInfo[]>([]);
  const [contextMenu, setContextMenu] = useState<ActiveContextMenu | null>(null);
  const [isBusy, setIsBusy] = useState(false);
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
  const [newDmDialogOpen, setNewDmDialogOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [displayDensity, setDisplayDensityState] =
    useState<DisplayDensity>(readDisplayDensity);
  const [spaceLocalOverrides, setSpaceLocalOverrides] =
    useState<SpaceLocalOverrides>(readSpaceLocalOverrides);
  const [newDmDraftUserId, setNewDmDraftUserId] = useState("");
  const [inviteUserDialog, setInviteUserDialog] = useState<InviteUserDialogState>(null);
  const [inviteUserDraftQuery, setInviteUserDraftQuery] = useState("");
  const [inviteScopeSelection, setInviteScopeSelection] =
    useState<InviteScopeSelection>(DEFAULT_INVITE_SCOPE);
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
  const uiLatencyDiagnostics = useUiLatencyDiagnostics();
  const verboseDiagnosticBuild = verboseDiagnosticsEnabled();
  const searchTimer = useRef<number | null>(null);
  const qaSendStarted = useRef(false);
  const qaSendPending = useRef(false);
  const qaSendTargetRequested = useRef(false);
  const qaSendTargetSelectionRequested = useRef<string | null>(null);
  const qaSendBaselineErrorCount = useRef(0);
  const initialHomeSelectionApplied = useRef(false);
  const requestedAvatarMxcsRef = useRef<Set<string>>(new Set());
  const avatarRetryCountsRef = useRef<Map<string, number>>(new Map());

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
  const composerDraftPersistTimer = useRef<number | null>(null);
  const localComposerDraftsRef = useRef<Record<string, string>>({});
  const threadComposerDraftPersistTimers = useRef<Record<string, number>>({});
  const panelDiagnosticRef = useRef<string | null>(null);
  const typingSignalRef = useRef<{ roomId: string | null; isTyping: boolean }>({
    roomId: null,
    isTyping: false
  });
  const searchInputRef = useRef<HTMLInputElement>(null);
  const loginPasswordRef = useRef<HTMLInputElement>(null);
  const recoverySecretRef = useRef<HTMLInputElement>(null);
  const roomSettingsLoadRef = useRef<string | null>(null);
  const spaceSettingsLoadRef = useRef<string | null>(null);
  const appTimelineTransport = useMemo<TimelineTransport | null>(() => {
    if (!tauriTimelineTransport) {
      return null;
    }
    return {
      ...tauriTimelineTransport,
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
    setDiagnosticLogEntries((current) => appendDiagnosticLogEntry(current, entry));
  }, []);
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
  const snapshotComposerDraft = snapshot?.state.ui.timeline.composer.draft ?? "";
  const composerDraft =
    timelineRoomId && Object.prototype.hasOwnProperty.call(localComposerDraftsRef.current, timelineRoomId)
      ? localComposerDraftsRef.current[timelineRoomId] ?? ""
      : snapshotComposerDraft;
  const stagedUploads = snapshot?.state.ui.timeline.staged_uploads ?? [];
  const stagedUploadIdKey = stagedUploads.map((item) => item.staged_id).join("\n");
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
    const activeIds = new Set(stagedUploads.map((item) => item.staged_id));
    const next = new Map(
      [...stagedUploadFilesRef.current.entries()].filter(([stagedId]) =>
        activeIds.has(stagedId)
      )
    );
    if (next.size !== stagedUploadFilesRef.current.size) {
      stagedUploadFilesRef.current = next;
    }
  }, [stagedUploadIdKey]);

  useEffect(() => {
    if (!snapshot || !tauriTimelineTransport?.downloadAvatarThumbnail) {
      requestedAvatarMxcsRef.current.clear();
      avatarRetryCountsRef.current.clear();
      return;
    }
    // #116 perf gate: avatar downloads are disabled by default to prevent the
    // AccountActor command flood that froze room selection.
    if (!AVATAR_THUMBNAIL_DOWNLOADS_ENABLED) {
      return;
    }

    const plan = planSnapshotAvatarThumbnailRequests(
      snapshot,
      requestedAvatarMxcsRef.current,
      avatarRetryCountsRef.current
    );
    requestedAvatarMxcsRef.current = plan.requestedMxcUris;
    avatarRetryCountsRef.current = plan.retryCounts;

    for (const mxcUri of plan.requestMxcUris) {
      void tauriTimelineTransport.downloadAvatarThumbnail(mxcUri).catch(() => {
        requestedAvatarMxcsRef.current.delete(mxcUri);
      });
    }
  }, [snapshot]);

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
      cancelComposerDraftPersist();
      cancelThreadComposerDraftPersists();
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

  useEffect(() => {
    const title = snapshot
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

    document.title = title;
    if (!isTauriRuntime()) {
      return;
    }

    void applyDesktopAttentionToWindow(
      getCurrentWindow(),
      title,
      safeAttentionSummary.badgeCount,
      snapshot?.state.domain.native_attention.summary.capabilities
    );
  }, [
    snapshot,
    rightPanelMode,
    qaSendStatus,
    safeAttentionSummary.badgeCount,
    safeAttentionSummary.qaTitleToken,
    timelineDiagnostics
  ]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    const candidate = desktopAttentionNotificationCandidate(
      snapshot.state.domain.native_attention
    );

    if (!candidate || !tauriNotificationTransport) {
      return;
    }

    void dispatchDesktopAttentionTransientEffects(
      getCurrentWindow(),
      candidate,
      snapshot.state.domain.native_attention.summary.capabilities,
      snapshot.state.domain.settings.values.notifications
    );
    void sendDesktopAttentionNotification(candidate, tauriNotificationTransport);
  }, [
    snapshot?.state.domain.native_attention.dispatch.kind,
    snapshot?.state.domain.native_attention.summary.candidate?.room_display_name,
    snapshot?.state.domain.native_attention.summary.candidate?.kind,
    snapshot?.state.domain.native_attention.summary.candidate?.unread_count,
    snapshot?.state.domain.native_attention.summary.candidate?.highlight_count
  ]);

  useEffect(() => {
    if (!tauriNotificationTransport || safeAttentionSummary.badgeCount !== 0) {
      return;
    }

    void clearDesktopAttentionNotifications(tauriNotificationTransport);
  }, [safeAttentionSummary.badgeCount]);

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
          void api.selectRoom(targetRoom.room_id).then(setSnapshot).catch(() => {
            qaSendPending.current = false;
            setQaSendStatus("failed");
          });
        }
      }
      return;
    }
    const roomId = snapshot.state.ui.timeline.room_id;
    if (!roomId) {
      return;
    }

    qaSendStarted.current = true;
    qaSendBaselineErrorCount.current = snapshot.state.ui.errors.length;
    qaSendBaselineTimelineItems.current = snapshot.timeline.length;
    qaSendPending.current = true;
    setQaSendStatus("pending");
    void api
      .sendText(roomId, message)
      .then((nextSnapshot) => {
        setSnapshot(nextSnapshot);
        if (!isTauriRuntime()) {
          const completionStatus = qaSendSmokeCompletionStatus(
            nextSnapshot,
            qaSendBaselineErrorCount.current,
            qaSendBaselineTimelineItems.current
          );
          qaSendPending.current = completionStatus === "pending";
          setQaSendStatus(completionStatus);
        }
      })
      .catch(() => {
        qaSendPending.current = false;
        setQaSendStatus("failed");
      });
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
    void listen<CoreEventPayload>(CORE_EVENT_NAME, (event) => {
      if (event.payload.kind !== "StateDelta") {
        return;
      }
      const applied = applyAppStoreDelta({
        generation: event.payload.generation,
        changed: event.payload.changed
      });
      if (!applied) {
        void refresh();
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

  // App-level timeline store: apply CoreEvent::Timeline diffs once, then feed
  // the resulting store into every TimelineView. This keeps Matrix timeline
  // semantics Rust-owned and avoids per-view reducer ownership.
  useEffect(() => {
    if (!appTimelineTransport) {
      return;
    }

    let disposed = false;
    const unsubscribe = appTimelineTransport.listenCoreEvents((payload) => {
      if (disposed) {
        return;
      }
      if (payload.kind === "ResyncMarker") {
        setTimelineStore((current) =>
          pruneTimelineStore(
            applyGlobalResync(current),
            retainedTimelineKeyIdsRef.current
          )
        );
        return;
      }
      if (payload.kind !== "Timeline") {
        return;
      }
      setTimelineStore((current) =>
        applyTimelineEventWithRetention(
          current,
          payload.event,
          retainedTimelineKeyIdsRef.current
        )
      );
    });

    return () => {
      disposed = true;
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
    void api.loadRoomSettings(activeRoomId).then(setSnapshot);
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
    void api.loadRoomSettings(activeSpaceId).then(setSnapshot);
  }, [
    rightPanelMode,
    snapshot?.state.ui.navigation.active_space_id,
    snapshot?.state.domain.room_management.operation,
    snapshot?.state.domain.room_management.selected_room_id,
    snapshot?.state.domain.room_management.settings
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

  async function submitLogin(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const password = loginPasswordRef.current?.value ?? "";
    setIsBusy(true);
    try {
      setSnapshot(
        await api.submitLogin(
          loginHomeserver,
          loginUsername,
          password,
          loginDeviceName
        )
      );
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

  async function resetRoomTimelineCache(roomId: string) {
    setSnapshot(await api.resetRoomTimelineCache(roomId));
  }

  async function queryDevices() {
    setSnapshot(await api.queryDevices());
  }

  async function renameDevice(deviceOrdinal: number, displayName: string) {
    setSnapshot(await api.renameDevice(deviceOrdinal, displayName));
  }

  async function deleteDevices(deviceOrdinals: number[]) {
    setSnapshot(await api.deleteDevices(deviceOrdinals));
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
    setSnapshot(await api.setLocalUserAlias(userId, alias));
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
      message: `manual reshare requested room=${roomId}`
    });
    try {
      setSnapshot(await api.reshareRoomKey(roomId));
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "e2ee.room_key",
        message: `manual reshare completed room=${roomId}`
      });
    } catch (error) {
      appendDiagnosticLog({
        timestampMs: Date.now(),
        source: "e2ee.room_key",
        message: `manual reshare failed room=${roomId} error=${String(error)}`
      });
      throw error;
    }
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

  async function resetLocalData() {
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

  const openHomeSelection = useCallback(async (selection = homeSelection) => {
    const homeSnapshot = await api.selectSpace(null);
    if (selection.kind === "dm") {
      const room = homeSnapshot.state.domain.rooms.find(
        (candidate) => candidate.room_id === selection.roomId && candidate.is_dm
      );
      if (room) {
        setPrimaryView("timeline");
        setSnapshot(await api.selectRoom(selection.roomId));
        return;
      }
    }
    if (selection.kind === "explore") {
      setSnapshot(homeSnapshot);
      setPrimaryView("explore");
      return;
    }
    if (selection.kind === "invites") {
      setSnapshot(homeSnapshot);
      setPrimaryView("invites");
      return;
    }
    setSnapshot(await api.openActivity());
    setPrimaryView("activity");
  }, [homeSelection, setSnapshot]);

  async function selectSpace(spaceId: string | null) {
    if (spaceId === null) {
      // The workspace-rail Home button always resets to Activity/Recent.
      await openHomeActivityView();
      return;
    }
    setPrimaryView("timeline");
    setSnapshot(await api.selectSpace(spaceId));
  }

  async function reorderSpaces(spaceIds: string[]) {
    setSnapshot(await api.reorderSpaces(spaceIds));
  }

  async function selectRoom(roomId: string) {
    const selectedRoom = snapshot?.state.domain.rooms.find((room) => room.room_id === roomId);
    const previousActiveRoomId = snapshot?.state.ui.navigation.active_room_id ?? null;
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=select_start current_active=${Boolean(previousActiveRoomId)} target_known=${Boolean(selectedRoom)} same_active=${previousActiveRoomId === roomId}`
    });
    if (snapshot?.sidebar.account_home.is_active && selectedRoom?.is_dm) {
      setHomeSelection({ kind: "dm", roomId });
    }
    setPrimaryView("timeline");
    const nextSnapshot = await api.selectRoom(roomId);
    appendDiagnosticLog({
      timestampMs: Date.now(),
      source: "room.transition",
      message: `stage=select_done active_changed=${nextSnapshot.state.ui.navigation.active_room_id !== previousActiveRoomId} timeline_matches=${nextSnapshot.state.ui.timeline.room_id === nextSnapshot.state.ui.navigation.active_room_id}`
    });
    setSnapshot(nextSnapshot);
  }

  async function openDmUserInfo(roomId: string, userId: string) {
    await selectRoom(roomId);
    roomSettingsLoadRef.current = null;
    const next = await api.loadRoomSettings(roomId);
    setSnapshot(next);
    setPeoplePanelScope({ kind: "room", roomId });
    setSelectedProfileUserId(userId);
    await setRightPanelModeClosingFocusedContext("profile");
  }

  async function openHomeActivityView() {
    setHomeSelection({ kind: "activity" });
    await openHomeSelection({ kind: "activity" });
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
    void openHomeSelection(homeSelection);
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

  async function markActivityRead(target: ActivityMarkReadTarget) {
    setSnapshot(await api.markActivityRead(target));
  }

  async function queryDirectory() {
    if (isBusy) {
      return;
    }
    const term = directorySearchDraft.trim();
    setSnapshot(
      await api.queryDirectory({
        term: term || null,
        server_name: null,
        limit: 20,
        since: null
      })
    );
  }

  async function joinDirectoryRoom(room: DirectoryRoomSummary) {
    const alias = room.canonical_alias?.trim();
    if (!alias || isBusy || snapshot?.state.domain.directory.join.kind === "joining") {
      return;
    }
    const nextSnapshot = await api.joinDirectoryRoom(alias, serverNameFromAlias(alias));
    setPrimaryView("timeline");
    setSnapshot(nextSnapshot);
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
    setInviteScopeSelection(DEFAULT_INVITE_SCOPE);
    setInviteUserDialog({ roomId, title });
    const nextSnapshot = await api.openInviteWorkflow(roomId);
    const workflow = nextSnapshot.state.domain.invite_workflow ?? DEFAULT_INVITE_WORKFLOW;
    setInviteScopeSelection(inviteScopeFromWorkflow(workflow));
    setSnapshot(nextSnapshot);
  }

  async function closeInviteUserDialog() {
    setInviteUserDialog(null);
    setInviteUserDraftQuery("");
    setInviteScopeSelection(DEFAULT_INVITE_SCOPE);
    setSnapshot(await api.closeInviteWorkflow());
  }

  async function updateInviteUserQuery(value: string) {
    const dialog = inviteUserDialog;
    setInviteUserDraftQuery(value);
    if (!dialog) {
      return;
    }
    const nextSnapshot = await api.searchInviteTargets(dialog.roomId, value);
    const workflow = nextSnapshot.state.domain.invite_workflow ?? DEFAULT_INVITE_WORKFLOW;
    if (
      workflow.scope_plan &&
      !workflow.scope_plan.options.some(
        (option) => inviteScopeKey(option.scope) === inviteScopeKey(inviteScopeSelection)
      )
    ) {
      setInviteScopeSelection(inviteScopeFromWorkflow(workflow));
    }
    setSnapshot(nextSnapshot);
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
      let nextSnapshot = await api.acceptInvite(roomId);
      if (nextSnapshot.state.domain.rooms.some((room) => room.room_id === roomId)) {
        nextSnapshot = await api.selectRoom(roomId);
      }
      setSnapshot(nextSnapshot);
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
      let nextSnapshot = await api.joinRoom(trimmedRoomId);
      if (nextSnapshot.state.domain.rooms.some((room) => room.room_id === trimmedRoomId)) {
        nextSnapshot = await api.selectRoom(trimmedRoomId);
      }
      setPrimaryView("timeline");
      setSnapshot(nextSnapshot);
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
      const nextSnapshot = await api.inviteTargets(dialog.roomId, userIds, inviteScopeSelection);
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

  async function sendText(bodyOverride?: string) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const body = bodyOverride ?? composerDraft;
    const uploads = snapshot?.state.ui.timeline.staged_uploads ?? [];
    if (!roomId || (!body.trim() && uploads.length === 0)) {
      return;
    }
    if (uploads.length > 0) {
      for (const item of uploads) {
        const file = stagedUploadFilesRef.current.get(item.staged_id);
        if (!file) {
          return;
        }
        const uploaded = await uploadMediaFile(file, captionBody(item), item.compression_choice);
        if (!uploaded) {
          return;
        }
      }
      stagedUploadFilesRef.current = new Map();
      cancelComposerDraftPersist();
      clearLocalComposerDraft(roomId);
      setSnapshot(await api.clearUploadStaging(roomId));
      setSnapshot(await api.setComposerDraft(roomId, ""));
      setComposerMentions(EMPTY_MENTION_INTENT);
      updateComposerTypingSignal(roomId, "");
      return;
    }
    // Reply semantics are Rust-owned: dispatch sendReply when the composer is
    // in reply mode, otherwise plain sendText.
    const composerMode = snapshot?.state.ui.timeline.composer.mode ?? "Plain";

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
    try {
      const mentions = pruneMentionIntentForDraft(composerMentions, body);
      const nextSnapshot =
        composerMode === "Plain"
          ? await api.sendText(roomId, body, mentions)
          : await api.sendReply(
              roomId,
              composerMode.Reply.in_reply_to_event_id,
              body,
              mentions
            );
      cancelComposerDraftPersist();
      clearLocalComposerDraft(roomId);
      setSnapshot(nextSnapshot);
      updateComposerTypingSignal(roomId, "");
      if (!isTauriRuntime()) {
        const completionStatus = qaSendSmokeCompletionStatus(
          nextSnapshot,
          qaSendBaselineErrorCount.current,
          qaSendBaselineTimelineItems.current
        );
        qaSendPending.current = completionStatus === "pending";
        setQaSendStatus(completionStatus);
      }
    } catch {
      qaSendPending.current = false;
      setQaSendStatus("failed");
      return;
    }
    setComposerMentions(EMPTY_MENTION_INTENT);
  }

  async function scheduleSend(sendAtMs: number, bodyOverride?: string) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const body = bodyOverride ?? composerDraft;
    if (!roomId || !body.trim() || stagedUploads.length > 0) {
      return;
    }

    try {
      cancelComposerDraftPersist();
      clearLocalComposerDraft(roomId);
      setSnapshot(await api.scheduleSend(roomId, body, sendAtMs));
      setComposerMentions(EMPTY_MENTION_INTENT);
      updateComposerTypingSignal(roomId, "");
    } catch {
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

  async function rescheduleScheduledSend(scheduledId: string, sendAtMs: number) {
    try {
      setSnapshot(await api.rescheduleScheduledSend(scheduledId, sendAtMs));
    } catch {
      // Command failures are surfaced through the Rust-owned error/event path.
    }
  }

  function updateComposerDraft(value: string) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) {
      return;
    }
    localComposerDraftsRef.current[roomId] = value;
    updateComposerTypingSignal(roomId, value);
    queueComposerDraftPersist(roomId, value);
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

  function cancelComposerDraftPersist() {
    if (composerDraftPersistTimer.current === null) {
      return;
    }
    window.clearTimeout(composerDraftPersistTimer.current);
    composerDraftPersistTimer.current = null;
  }

  function queueComposerDraftPersist(roomId: string, value: string) {
    cancelComposerDraftPersist();
    composerDraftPersistTimer.current = window.setTimeout(() => {
      composerDraftPersistTimer.current = null;
      void api
        .setComposerDraft(roomId, value)
        .then((nextSnapshot) => {
          if ((localComposerDraftsRef.current[roomId] ?? "") !== value) {
            return;
          }
          setSnapshot(nextSnapshot);
        })
        .catch(() => undefined);
    }, 350);
  }

  function clearLocalComposerDraft(roomId: string) {
    delete localComposerDraftsRef.current[roomId];
  }

  async function stageUploadFiles(files: File[]): Promise<void> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId || files.length === 0) {
      return;
    }
    const startPosition = stagedUploads.length;
    const mediaSettings = snapshot.state.domain.settings.values.media;
    const existingItems: UploadStagingRequestItem[] = stagedUploads.map((item) => ({
      stagedId: item.staged_id,
      position: item.position,
      filename: item.filename,
      mimeType: item.mime_type,
      byteCount: item.byte_count,
      kind: item.kind,
      compressionChoice: item.compression_choice
    }));
    const newItems: UploadStagingRequestItem[] = files.map((file, index) => {
      const stagedId = createStagedUploadId(startPosition + index);
      return {
        stagedId,
        position: startPosition + index + 1,
        filename: file.name || "attachment",
        mimeType: file.type || "application/octet-stream",
        byteCount: file.size,
        kind: stagedUploadKindForFile(file),
        compressionChoice: initialStagedCompressionChoice(
          file,
          mediaSettings.image_upload_compression
        )
      };
    });
    const items = [...existingItems, ...newItems];
    const nextFiles = new Map(stagedUploadFilesRef.current);
    newItems.forEach((item, index) => {
      nextFiles.set(item.stagedId, files[index]);
    });
    stagedUploadFilesRef.current = nextFiles;
    setSnapshot(await api.stageUploads(roomId, items));
  }

  async function updateStagedUploadCaption(stagedId: string, caption: string): Promise<void> {
    setSnapshot(await api.updateStagedUploadCaption(stagedId, caption));
  }

  async function updateStagedUploadCompression(
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ): Promise<void> {
    setSnapshot(await api.updateStagedUploadCompression(stagedId, compressionChoice));
  }

  async function clearUploadStaging(): Promise<void> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId) {
      return;
    }
    stagedUploadFilesRef.current = new Map();
    setSnapshot(await api.clearUploadStaging(roomId));
  }

  async function uploadMediaFile(
    file: File,
    caption = "",
    compressionChoice?: StagedUploadCompressionChoice
  ): Promise<boolean> {
    const roomId = snapshot?.state.ui.timeline.room_id;
    if (!roomId || !isTauriRuntime()) {
      return false;
    }

    const mediaSettings = snapshot.state.domain.settings.values.media;
    const prepared = await prepareMediaUpload(
      file,
      forcedUploadMode(compressionChoice, mediaSettings.image_upload_compression),
      mediaSettings.image_upload_compression_policy,
      requestImageCompressionChoice
    );
    if (!prepared) {
      return false;
    }
    await invoke("upload_media", {
      roomId,
      filename: prepared.filename,
      mimeType: prepared.mimeType,
      bytes: prepared.bytes,
      caption,
      imageDimensions: prepared.imageDimensions,
      imageCompression: prepared.imageCompression,
      thumbnail: prepared.thumbnail
    });
    return true;
  }

  function requestImageCompressionChoice(
    plan: ImageCompressionPlan
  ): Promise<ImageUploadVariantKindPayload | "cancel"> {
    return new Promise((resolve) => {
      setImageCompressionDialog({ plan, resolve });
    });
  }

  async function settleImageCompressionDialog(
    choice: ImageUploadVariantKindPayload | "cancel",
    saveDefault = false
  ) {
    const dialog = imageCompressionDialog;
    if (!dialog) {
      return;
    }
    setImageCompressionDialog(null);
    try {
      if (choice !== "cancel" && saveDefault && snapshot) {
        await updateSettings({
          media: {
            ...snapshot.state.domain.settings.values.media,
            image_upload_compression: choice === "Compressed" ? "always" : "never"
          }
        });
      }
    } finally {
      dialog.resolve(choice);
      releaseImageCompressionPlan(dialog.plan);
    }
  }

  async function editMessage(message: { body: string | null; room_id: string; event_id: string }) {
    const body = window.prompt(t("timeline.editMessage"), message.body ?? undefined);
    if (body === null || !body.trim()) {
      return;
    }

    setSnapshot(await api.editMessage(message.room_id, message.event_id, body));
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

  async function openThread(roomId: string, rootEventId: string) {
    await closeFocusedContextIfHiddenBy("thread");
    setSnapshot(await api.openThread(roomId, rootEventId));
    setRightPanelMode("thread");
  }

  async function closeThread() {
    setSnapshot(await api.closeThread());
    setRightPanelMode("closed");
  }

  async function openThreadsListPanel(roomId: string) {
    await closeFocusedContextIfHiddenBy("threads");
    setSnapshot(await api.openThreadsList(roomId));
    setRightPanelMode("threads");
  }

  async function closeThreadsListPanel() {
    setSnapshot(await api.closeThreadsList());
    setRightPanelMode("closed");
  }

  async function paginateThreadsList(roomId: string) {
    setSnapshot(await api.paginateThreadsList(roomId));
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
    draft: string
  ) {
    const key = threadComposerDraftKey(roomId, rootEventId);
    setLocalThreadComposerDrafts((drafts) =>
      drafts[key] === draft ? drafts : { ...drafts, [key]: draft }
    );
    queueThreadComposerDraftPersist(roomId, rootEventId, draft);
  }

  async function sendThreadReply(roomId: string, rootEventId: string, body: string) {
    cancelThreadComposerDraftPersist(roomId, rootEventId);
    clearLocalThreadComposerDraft(roomId, rootEventId);
    setSnapshot(await api.sendThreadReply(roomId, rootEventId, body));
  }

  function queueThreadComposerDraftPersist(roomId: string, rootEventId: string, draft: string) {
    cancelThreadComposerDraftPersist(roomId, rootEventId);
    const key = threadComposerDraftKey(roomId, rootEventId);
    threadComposerDraftPersistTimers.current[key] = window.setTimeout(() => {
      delete threadComposerDraftPersistTimers.current[key];
      void api
        .setThreadComposerDraft(roomId, rootEventId, draft)
        .then((nextSnapshot) => {
          setSnapshot(nextSnapshot);
        })
        .catch(() => undefined);
    }, 350);
  }

  function cancelThreadComposerDraftPersist(roomId: string, rootEventId: string) {
    const key = threadComposerDraftKey(roomId, rootEventId);
    const timer = threadComposerDraftPersistTimers.current[key];
    if (timer === undefined) {
      return;
    }
    window.clearTimeout(timer);
    delete threadComposerDraftPersistTimers.current[key];
  }

  function cancelThreadComposerDraftPersists() {
    Object.values(threadComposerDraftPersistTimers.current).forEach((timer) => {
      window.clearTimeout(timer);
    });
    threadComposerDraftPersistTimers.current = {};
  }

  function clearLocalThreadComposerDraft(roomId: string, rootEventId: string) {
    const key = threadComposerDraftKey(roomId, rootEventId);
    setLocalThreadComposerDrafts((drafts) => {
      if (!Object.prototype.hasOwnProperty.call(drafts, key)) {
        return drafts;
      }
      const next = { ...drafts };
      delete next[key];
      return next;
    });
  }

  function threadComposerDraftKey(roomId: string, rootEventId: string): string {
    return `${roomId}\u0000${rootEventId}`;
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

  async function setRightPanelModeClosingFocusedContext(nextMode: RightPanelMode) {
    await closeFocusedContextIfHiddenBy(nextMode);
    if (nextMode !== "profile") {
      setSelectedProfileUserId(null);
    }
    if (nextMode !== "people" && nextMode !== "profile") {
      setPeoplePanelScope(null);
    }
    setRightPanelMode(nextMode);
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

  function openActivityRow(roomId: string, eventId: string) {
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

    setSnapshot(await api.selectRoom(roomId));
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
    if (target.kind === "message") {
      switch (actionId) {
        case "replyToMessage":
          void setComposerReplyTarget(target.message.room_id, target.message.event_id);
          return;
        case "openThread":
          void openThread(target.message.room_id, target.message.event_id);
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
            room?.latest_event?.event_id ??
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
    setSnapshot(await api.submitSearch(trimmed, scope));
    if (searchMode) {
      setRightPanelMode(searchMode);
    }
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
  const recoveryRequired = sessionKind === "needsRecovery" || sessionKind === "recovering";

  if (sessionKind === "restoring" || sessionKind === "loggingOut") {
    return <div className="boot-screen">{t("app.title")}</div>;
  }

  setActiveLocaleProfile(
    snapshot.state.domain.locale_profile.catalog_locale,
    snapshot.state.domain.locale_profile.pseudo_locale
  );

  if (sessionKind !== "ready" && !recoveryRequired) {
    return (
      <AuthScreen
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
      />
    );
  }

  const activeRoom = snapshot.state.domain.rooms.find(
    (room) => room.room_id === snapshot.state.ui.navigation.active_room_id
  );
  const activeSpace = snapshot.state.domain.spaces.find(
    (space) => space.space_id === snapshot.state.ui.navigation.active_space_id
  );
  const homeContextActive = snapshot.sidebar.account_home.is_active && !activeSpace;
  const activeSpaceName = activeSpace
    ? spaceDisplayName(activeSpace.space_id, activeSpace.display_name, spaceLocalOverrides)
    : snapshot.sidebar.account_home.display_name;
  const activeSearchState = correlatedSearchState(
    snapshot.state.domain.search,
    searchQuery,
    searchScope
  );
  const searchResults = activeSearchState?.kind === "results" ? activeSearchState.results : [];
  const searchResultsQuery = activeSearchState?.kind === "results" ? activeSearchState.query : "";
  const searchHighlightQuery = searchResultsQuery;
  const effectiveRightPanelMode = effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot);
  const rightPanelOpen = effectiveRightPanelMode !== "closed";
  const appGridStyle = {
    "--sidebar-width": `${sidebarWidth}px`,
    "--right-panel-width": `${rightPanelWidth}px`
  } as CSSProperties;

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
          activeSpaceName={activeSpaceName}
          homeserver={snapshot.state.domain.session.homeserver ?? null}
          isBusy={isBusy}
          platform={snapshot.state.domain.locale_profile.platform}
          searchInputRef={searchInputRef}
          searchQuery={searchQuery}
          searchScope={searchScope}
          sync={snapshot.state.domain.sync}
          onOpenKeyboardSettings={() => {
            void setRightPanelModeClosingFocusedContext("keyboardSettings");
          }}
          onOpenDiagnostics={() => setDiagnosticsOpen(true)}
          onRestartSync={restartSync}
          onSearchQueryChange={setSearchQuery}
          onSearchScopeChange={setSearchScope}
          onStartWindowDrag={startWindowDrag}
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
          onOpenHome={() => {
            void openHomeActivityView();
          }}
          onOpenInvites={() => {
            void (homeContextActive ? openHomeInvitesView() : openInvitesView());
          }}
          onOpenSpaceInfo={() => {
            void setRightPanelModeClosingFocusedContext("spaceInfo");
          }}
          onOpenThreads={() => {
            const roomId = snapshot.state.ui.navigation.active_room_id;
            if (roomId) {
              void openThreadsListPanel(roomId);
            }
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
                openActivityRow(row.room_id, row.event_id);
              } else if (row.kind === "roomUnread") {
                void openActivityRoom(row.room_id);
              }
            }}
            onSetTab={(tab) => {
              void setActivityTab(tab);
            }}
          />
        ) : primaryView === "explore" ? (
          <ExplorePane
            isBusy={isBusy}
            queryDraft={directorySearchDraft}
            snapshot={snapshot}
            onJoinRoom={(room) => {
              void joinDirectoryRoom(room);
            }}
            onQueryChange={setDirectorySearchDraft}
            onSearch={() => {
              void queryDirectory();
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
            composerDraft={composerDraft}
            composerMode={composerModeProp(snapshot.state.ui.timeline.composer.mode)}
            mentionIntent={composerMentions}
            resolveComposerKeyAction={resolveComposerKeyAction}
            searchQuery={searchHighlightQuery}
            searchResults={searchResults}
            showSearchResults={false}
            snapshot={snapshot}
            timelineTransport={appTimelineTransport}
            onReturnToLive={() => {
              // #161: leave the anchored (jump-to-date) main-pane view. Closing
              // the focused context clears navigation.main_timeline_anchor in
              // Rust, so the main pane re-renders the live room timeline.
              void api.closeFocusedContext().then(setSnapshot);
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
            onUpdateStagedUploadCompression={(stagedId, compressionChoice) => {
              void updateStagedUploadCompression(stagedId, compressionChoice);
            }}
            onComposerDraftChange={(value) => {
              void updateComposerDraft(value);
            }}
            onMentionIntentChange={setComposerMentions}
            onOpenThread={openThread}
            onReply={(roomId, eventId) => {
              void setComposerReplyTarget(roomId, eventId);
            }}
            onRescheduleScheduledSend={(scheduledId, sendAtMs) => {
              void rescheduleScheduledSend(scheduledId, sendAtMs);
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
            onUnpinPinnedEvent={unpinPinnedEvent}
            onOpenPeople={async () => {
              const roomId = snapshot.state.ui.navigation.active_room_id;
              if (roomId) {
                roomSettingsLoadRef.current = null;
                const next = await api.loadRoomSettings(roomId);
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
                void openThreadsListPanel(roomId);
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
          displayDensity={displayDensity}
          isRecoveryBusy={isBusy || sessionKind === "recovering"}
          mode={effectiveRightPanelMode}
          peoplePanelScope={peoplePanelScope}
          selectedProfileUserId={selectedProfileUserId}
          recoverySecretFilled={recoverySecretFilled}
          recoverySecretInputRef={recoverySecretRef}
          snapshot={snapshot}
          timelineTransport={appTimelineTransport}
          searchQuery={searchResultsQuery}
          searchResults={searchResults}
          savedSessions={savedSessions}
          onCloseThread={() => {
            void closeThread();
          }}
          onClosePanel={() => {
            void closeFocusedContextPanel();
          }}
          onOpenThread={(roomId, rootEventId) => {
            void openThread(roomId, rootEventId);
          }}
          onOpenFiles={(scope) => {
            void openFilesView(scope);
          }}
          onOpenSpaceMembers={
            activeSpace
              ? async () => {
                  spaceSettingsLoadRef.current = null;
                  const next = await api.loadRoomSettings(activeSpace.space_id);
                  setSnapshot(next);
                  setPeoplePanelScope({ kind: "space", spaceId: activeSpace.space_id });
                  setSelectedProfileUserId(null);
                  await setRightPanelModeClosingFocusedContext("people");
                }
              : undefined
          }
          onOpenPeople={async () => {
            if (activeRoom) {
              roomSettingsLoadRef.current = null;
              const next = await api.loadRoomSettings(activeRoom.room_id);
              setSnapshot(next);
              setPeoplePanelScope({ kind: "room", roomId: activeRoom.room_id });
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
          onPaginateThreadsList={(roomId) => {
            void paginateThreadsList(roomId);
          }}
          onOpenKeyboardSettings={() => {
            void setRightPanelModeClosingFocusedContext("keyboardSettings");
          }}
          onOpenRecovery={() => {
            void setRightPanelModeClosingFocusedContext("recovery");
          }}
          onProbeLocalEncryption={() => {
            void probeLocalEncryptionHealth();
          }}
          onResetLocalData={() => {
            void resetLocalData();
          }}
          onLogout={() => {
            void logout();
          }}
          onInviteUser={openInviteUserDialog}
          onModerateMember={(roomId, targetUserId, action, reason) => {
            void moderateRoomMember(roomId, targetUserId, action, reason);
          }}
          onSetLocalUserAlias={(userId, alias) => {
            void setLocalUserAlias(userId, alias);
          }}
          onRequestMemberAvatarThumbnail={
            AVATAR_THUMBNAIL_DOWNLOADS_ENABLED
              ? tauriTimelineTransport?.downloadAvatarThumbnail
              : undefined
          }
          onSetRoomNotificationMode={(roomId, mode) => {
            void setRoomNotificationMode(roomId, mode);
          }}
          onResetRoomTimelineCache={(roomId) => {
            void resetRoomTimelineCache(roomId);
          }}
          onStartDirectMessage={(userId) => {
            void startDirectMessage(userId);
          }}
          onUpdateMemberRole={(roomId, targetUserId, powerLevel) => {
            void updateRoomMemberRole(roomId, targetUserId, powerLevel);
          }}
          onReshareRoomKey={(roomId) => {
            void reshareRoomKey(roomId);
          }}
          onRecoverySecretPresenceChange={setRecoverySecretFilled}
          onReply={(roomId, eventId) => {
            void setComposerReplyTarget(roomId, eventId);
          }}
          onResultSelect={selectSearchResult}
          onSubmitRecovery={submitRecovery}
          onSwitchAccount={(session) => {
            void switchAccount(session);
          }}
          onThreadComposerDraftChange={(roomId, rootEventId, draft) => {
            updateThreadComposerDraft(roomId, rootEventId, draft);
          }}
          threadComposerDraftOverrides={localThreadComposerDrafts}
          onThreadReplySend={(roomId, rootEventId, body) => {
            void sendThreadReply(roomId, rootEventId, body);
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
          onQueryDevices={() => {
            void queryDevices();
          }}
          onRenameDevice={(deviceOrdinal, displayName) => {
            void renameDevice(deviceOrdinal, displayName);
          }}
          onDeleteDevices={(deviceOrdinals) => {
            void deleteDevices(deviceOrdinals);
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
      {inviteUserDialog ? (
        <InviteTargetsDialog
          isBusy={isBusy}
          query={inviteUserDraftQuery}
          scope={inviteScopeSelection}
          title={inviteUserDialog.title}
          workflow={snapshot?.state.domain.invite_workflow ?? DEFAULT_INVITE_WORKFLOW}
          onCancel={() => {
            void closeInviteUserDialog();
          }}
          onQueryChange={(value) => {
            void updateInviteUserQuery(value);
          }}
          onRemoveTarget={(userId) => {
            void removeInviteTarget(userId);
          }}
          onScopeChange={setInviteScopeSelection}
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
      {imageCompressionDialog ? (
        <ImageCompressionDialog
          plan={imageCompressionDialog.plan}
          onCancel={() => settleImageCompressionDialog("cancel")}
          onChoose={(choice, saveDefault) => settleImageCompressionDialog(choice, saveDefault)}
        />
      ) : null}
      {diagnosticsOpen ? (
        <DiagnosticDialog
          report={diagnosticReport({
            snapshot,
            panelMode: effectiveRightPanelMode,
            sendStatus: qaSendStatus,
            timelineDiagnostics,
            domDiagnostics: qaRenderedDomDiagnostics(),
            uiLatencyDiagnostics,
            stateDeltaStats: getAppStoreDeltaStats(),
            timelineTransportStats: getTimelineTransportStats(),
            jsErrors: getRecentJsErrors(),
            logEntries: diagnosticLogEntries,
            verboseDiagnostics: {
              enabled: verboseDiagnosticBuild,
              security: verboseDiagnosticBuild ? qaSecurityDiagnostics() : undefined
            }
          })}
          onClose={() => setDiagnosticsOpen(false)}
        />
      ) : null}
      </div>
    </TimelineStoreContext.Provider>
  );
}

function timelineDiagnosticsEqual(
  left: QaTimelineDiagnostics,
  right: QaTimelineDiagnostics
): boolean {
  return (
    left.visibleItems === right.visibleItems &&
    left.downloadedItems === right.downloadedItems &&
    left.backfill === right.backfill &&
    left.avatarMxcItems === right.avatarMxcItems &&
    left.avatarReadyItems === right.avatarReadyItems &&
    left.avatarPendingItems === right.avatarPendingItems &&
    left.avatarFailedItems === right.avatarFailedItems &&
    left.avatarMissingItems === right.avatarMissingItems &&
    left.avatarRenderedImages === right.avatarRenderedImages &&
    left.avatarBrokenImages === right.avatarBrokenImages
  );
}

function timelineDiagnosticsLogMessage(diagnostics: QaTimelineDiagnostics): string {
  return [
    `items visible=${diagnostics.visibleItems}`,
    `downloaded=${diagnostics.downloadedItems}`,
    `backfill=${diagnostics.backfill}`,
    `avatars mxc=${diagnostics.avatarMxcItems}`,
    `ready=${diagnostics.avatarReadyItems}`,
    `pending=${diagnostics.avatarPendingItems}`,
    `failed=${diagnostics.avatarFailedItems}`,
    `missing=${diagnostics.avatarMissingItems}`,
    `rendered=${diagnostics.avatarRenderedImages}`,
    `broken=${diagnostics.avatarBrokenImages}`
  ].join(" ");
}

// Preserve App.tsx's original public export surface; these components now live in
// dedicated modules under ./components.
export { Composer } from "./components/composer";
export { ContextualRightPanel } from "./components/rightPanel";
export { TopBar, WorkspaceRail } from "./components/Shell";
