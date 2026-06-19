import {
  type FormEvent,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
  useEffect,
  useMemo,
  useRef,
  useState
} from "react";
// App.tsx is the Tauri integration host. The three @tauri-apps imports below
// are acknowledged in-progress transport wiring tracked for Phase 2 migration
// to backend/client.ts (#87). Each line has its own disable directive so the
// rule still catches any NEW @tauri-apps import added without a comment.
// eslint-disable-next-line no-restricted-imports
import { invoke } from "@tauri-apps/api/core";
// eslint-disable-next-line no-restricted-imports
import { listen } from "@tauri-apps/api/event";
// eslint-disable-next-line no-restricted-imports
import { getCurrentWindow } from "@tauri-apps/api/window";

import { createDesktopApi } from "./backend/client";
import { setActiveLocaleProfile, t } from "./i18n/messages";
import { ContextMenuSurface } from "./components/ContextMenuSurface";
import {
  type TimelineTransport
} from "./components/TimelineView";
import {
  type CoreEventPayload,
  type TimelineKey
} from "./domain/coreEvents";
import {
  type ContextMenuActionId,
  type ContextMenuItem
} from "./domain/contextMenus";
import {
  shortcutActionFromMenuPayload,
  shortcutIdForKeyboardEvent
} from "./domain/shortcuts";
import {
  restoreTimelineAnchor,
  timelinePaginationAnchorEventId
} from "./domain/timelineAnchor";
import {
  effectiveRightPanelModeForSnapshot,
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
import { qaWindowTitle } from "./domain/qaTitle";
import {
  type QaSendSmokeStatus,
  qaSendCompletionStatusFromCoreEvent,
  qaSendSmokeCanStart,
  qaSendSmokeCompletionStatus,
  qaSendSmokeMessageFromEnv
} from "./domain/qaSendSmoke";
import type {
  ActivityMarkReadTarget,
  ActivityTab,
  AttachmentFilter,
  AttachmentScope,
  AttachmentSort,
  DesktopSnapshot,
  DirectoryRoomSummary,
  FilesViewScope,
  ImageUploadCompressionMode,
  ImageUploadCompressionPolicy,
  MentionIntent,
  ResolveComposerKeyAction,
  RoomListFilter,
  RoomModerationAction,
  RoomNotificationMode,
  RoomSettingChange,
  SavedSessionInfo,
  SearchScopeKind,
  SettingsPatch,
  StagedUploadCompressionChoice,
  StagedUploadItem,
  UploadStagingRequestItem
} from "./domain/types";
import { SNAPSHOT_SCHEMA_VERSION } from "./domain/types";

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
  ImageCompressionDialog,
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
const MENU_EVENT_NAME = "matrix-desktop://menu";
const STATE_EVENT_NAME = "matrix-desktop://state";
const CORE_EVENT_NAME = "matrix-desktop://event";

declare global {
  interface Window {
    __matrixDesktopQaErrorCaptureInstalled?: boolean;
    __matrixDesktopQaLastError?: string;
  }
}

if (
  typeof window !== "undefined" &&
  import.meta.env.VITE_MATRIX_DESKTOP_QA_TITLE === "1" &&
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
 * flows ONLY as CoreEvent diffs over `matrix-desktop://event`; AppState
 * snapshots never embed item lists). Null in browser preview mode, where the
 * fixture snapshot rendering below is used instead.
 */
const tauriTimelineTransport: TimelineTransport | null = isTauriRuntime()
  ? {
      listenCoreEvents(listener: (payload: CoreEventPayload) => void) {
        let disposed = false;
        let unlisten: (() => void) | null = null;
        void listen<CoreEventPayload>(CORE_EVENT_NAME, (event) => {
          listener(event.payload);
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
      async loadMessageSource(roomId: string, eventId: string) {
        await invoke("load_message_source", { roomId, eventId });
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
const DEFAULT_SIDEBAR_WIDTH = 318;
const MIN_SIDEBAR_WIDTH = 260;
const MAX_SIDEBAR_WIDTH = 440;
const COMPACT_RAIL_WIDTH = 56;
const MIN_TIMELINE_WIDTH_WHILE_RESIZING = 180;
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
type InviteUserDialogState = {
  roomId: string;
  title: string;
} | null;

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

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

function qaTitleEnabled(): boolean {
  return import.meta.env.VITE_MATRIX_DESKTOP_QA_TITLE === "1";
}

function qaSendSmokeMessage(): string | null {
  return qaSendSmokeMessageFromEnv(import.meta.env.VITE_MATRIX_DESKTOP_QA_SEND_SMOKE_MESSAGE);
}

export function App() {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot | null>(null);
  // #87 Phase 4 IPC contract guard: fail loudly on a stale flat (v1) snapshot or a
  // mismatched Rust/TS build instead of silently reading `undefined` domain/ui sections.
  useEffect(() => {
    if (snapshot && snapshot.state.schema_version !== SNAPSHOT_SCHEMA_VERSION) {
      console.error(
        `Koushi snapshot schema_version ${snapshot.state.schema_version} != expected ` +
          `${SNAPSHOT_SCHEMA_VERSION}: stale or mismatched IPC contract.`
      );
    }
  }, [snapshot]);
  const [searchQuery, setSearchQuery] = useState(() => initialSearchQuery());
  const [searchScope, setSearchScope] = useState<SearchScopeKind>("allRooms");
  const [composerMentions, setComposerMentions] = useState<MentionIntent>(EMPTY_MENTION_INTENT);
  const [stagedUploadFiles, setStagedUploadFiles] = useState<Map<string, File>>(() => new Map());
  const [imageCompressionDialog, setImageCompressionDialog] =
    useState<ImageCompressionDialogState | null>(null);
  const [loginHomeserver, setLoginHomeserver] = useState(DEFAULT_HOMESERVER);
  const [loginUsername, setLoginUsername] = useState("");
  const [loginDeviceName, setLoginDeviceName] = useState("Matrix Desktop");
  const [loginPasswordFilled, setLoginPasswordFilled] = useState(false);
  const [recoverySecretFilled, setRecoverySecretFilled] = useState(false);
  const [rightPanelMode, setRightPanelMode] = useState<RightPanelMode>("closed");
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);
  const [qaSendStatus, setQaSendStatus] = useState<QaSendSmokeStatus>("idle");
  const [savedSessions, setSavedSessions] = useState<SavedSessionInfo[]>([]);
  const [contextMenu, setContextMenu] = useState<ActiveContextMenu | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [primaryView, setPrimaryView] = useState<PrimaryView>("timeline");
  const [directorySearchDraft, setDirectorySearchDraft] = useState("");
  const [newDmDialogOpen, setNewDmDialogOpen] = useState(false);
  const [newDmDraftUserId, setNewDmDraftUserId] = useState("");
  const [inviteUserDialog, setInviteUserDialog] = useState<InviteUserDialogState>(null);
  const [inviteUserDraftUserId, setInviteUserDraftUserId] = useState("");
  // React-local ephemeral state only: which create dialog is open and the
  // unsent name draft. The pending op status comes from the snapshot
  // (basic_operation); the created room/space identity comes from the API.
  const [createDialog, setCreateDialog] = useState<"room" | "space" | null>(null);
  const [createDraftName, setCreateDraftName] = useState("");
  const [reportDialog, setReportDialog] = useState<ReportDialogState | null>(null);
  const [reportReasonDraft, setReportReasonDraft] = useState("");
  const searchTimer = useRef<number | null>(null);
  const qaSendStarted = useRef(false);
  const qaSendPending = useRef(false);
  const qaSendBaselineErrorCount = useRef(0);
  const qaSendBaselineTimelineItems = useRef(0);
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
        setPrimaryView("timeline");
        setRightPanelMode("focusedContext");
      }
    };
  }, []);
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
  const composerDraft = snapshot?.state.ui.timeline.composer.draft ?? "";
  const stagedUploads = snapshot?.state.ui.timeline.staged_uploads ?? [];
  const stagedUploadIdKey = stagedUploads.map((item) => item.staged_id).join("\n");

  useEffect(() => {
    const activeIds = new Set(stagedUploads.map((item) => item.staged_id));
    setStagedUploadFiles((files) => {
      const next = new Map(
        [...files.entries()].filter(([stagedId]) => activeIds.has(stagedId))
      );
      return next.size === files.size ? files : next;
    });
  }, [stagedUploadIdKey]);

  function handleShortcutAction(shortcutId: string): boolean {
    switch (shortcutId) {
      case "showKeyboardSettings":
        void setRightPanelModeClosingFocusedContext("keyboardSettings");
        return true;
      case "openUserSettings":
        void setRightPanelModeClosingFocusedContext("userSettings");
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
    if (rightPanelMode === "userSettings") {
      void refreshSavedSessions();
    }
  }, [rightPanelMode]);

  useEffect(() => {
    const roomId = snapshot?.state.ui.timeline.room_id ?? null;
    const isTyping = Boolean(roomId && composerDraft.trim());
    const previous = typingSignalRef.current;

    if (previous.roomId && previous.roomId !== roomId && previous.isTyping) {
      void api.setTyping(previous.roomId, false).catch(() => undefined);
    }

    typingSignalRef.current = { roomId, isTyping };
    if (!roomId) {
      return;
    }
    if (previous.roomId === roomId && previous.isTyping === isTyping) {
      return;
    }
    void api.setTyping(roomId, isTyping).catch(() => undefined);
  }, [composerDraft, snapshot?.state.ui.timeline.room_id]);

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
            qaSendStatus
          )
        : desktopAttentionWindowTitle("Koushi", safeAttentionSummary)
      : qaTitleEnabled()
        ? "matrix-desktop qa session=booting"
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
  }, [snapshot, rightPanelMode, qaSendStatus, safeAttentionSummary.badgeCount, safeAttentionSummary.qaTitleToken]);

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
    if (!message || !snapshot || qaSendStarted.current || !qaSendSmokeCanStart(snapshot)) {
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
      qaSendStatus !== "pending" ||
      isTauriRuntime()
    ) {
      return;
    }
    const completionStatus = qaSendSmokeCompletionStatus(
      snapshot,
      qaSendBaselineErrorCount.current,
      qaSendBaselineTimelineItems.current
    );
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
    void listen<string>(STATE_EVENT_NAME, () => {
      void refresh();
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

  async function submitRecovery(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const secret = recoverySecretRef.current?.value ?? "";
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

  async function startRoomCrawl(roomId: string) {
    setSnapshot(await api.startRoomCrawl(roomId));
  }

  async function stopRoomCrawl(roomId: string) {
    setSnapshot(await api.stopRoomCrawl(roomId));
  }

  async function setRoomUrlPreviewOverride(roomId: string, enabled: boolean) {
    setSnapshot(await api.setRoomUrlPreviewOverride(roomId, enabled));
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

  async function selectSpace(spaceId: string | null) {
    setPrimaryView("timeline");
    setSnapshot(await api.selectSpace(spaceId));
  }

  async function reorderSpaces(spaceIds: string[]) {
    setSnapshot(await api.reorderSpaces(spaceIds));
  }

  async function selectRoom(roomId: string) {
    setPrimaryView("timeline");
    setSnapshot(await api.selectRoom(roomId));
  }

  async function selectRoomListFilter(filter: RoomListFilter) {
    setSnapshot(await api.selectRoomListFilter(filter));
  }

  async function openInvitesView() {
    setSnapshot(await api.getSnapshot());
    setPrimaryView("invites");
  }

  async function openExploreView() {
    setSnapshot(await api.getSnapshot());
    setPrimaryView("explore");
  }

  async function openActivityView() {
    setSnapshot(await api.openActivity());
    setPrimaryView("activity");
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
    setCreateDialog(kind);
  }

  function closeCreateDialog() {
    setCreateDialog(null);
    setCreateDraftName("");
  }

  function openNewDmDialog() {
    setNewDmDraftUserId("");
    setNewDmDialogOpen(true);
  }

  function closeNewDmDialog() {
    setNewDmDialogOpen(false);
    setNewDmDraftUserId("");
  }

  function openInviteUserDialog(roomId: string, title: string) {
    setInviteUserDraftUserId("");
    setInviteUserDialog({ roomId, title });
  }

  function closeInviteUserDialog() {
    setInviteUserDialog(null);
    setInviteUserDraftUserId("");
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

  async function submitInviteUserDialog() {
    const dialog = inviteUserDialog;
    const userId = inviteUserDraftUserId.trim();
    if (!dialog || !userId || isBusy) {
      return;
    }
    setIsBusy(true);
    try {
      setSnapshot(await api.inviteUser(dialog.roomId, userId));
      closeInviteUserDialog();
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
      isBusy ||
      (snapshot && snapshot.state.ui.basic_operation.kind !== "idle")
    ) {
      return;
    }
    setIsBusy(true);
    try {
      let nextSnapshot =
        kind === "space" ? await api.createSpace(name) : await api.createRoom(name);
      const createdRoomId = nextSnapshot.state.ui.navigation.active_room_id;
      const viaServer = createdRoomId ? serverNameFromRoomId(createdRoomId) : null;
      if (kind === "room" && activeSpaceIdForCreatedRoom && createdRoomId && viaServer) {
        nextSnapshot = await api.setSpaceChild(
          activeSpaceIdForCreatedRoom,
          createdRoomId,
          viaServer
        );
      }
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

  async function paginateTimelineBackwards(roomId: string) {
    const anchorEventId = timelinePaginationAnchorEventId(snapshot?.timeline ?? []);
    setSnapshot(await api.paginateTimelineBackwards(roomId));
    requestAnimationFrame(() => {
      restoreTimelineAnchor(document, anchorEventId);
    });
  }

  async function sendText() {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const body = composerDraft;
    const uploads = snapshot?.state.ui.timeline.staged_uploads ?? [];
    if (!roomId || (!body.trim() && uploads.length === 0)) {
      return;
    }
    if (uploads.length > 0) {
      for (const item of uploads) {
        const file = stagedUploadFiles.get(item.staged_id);
        if (!file) {
          return;
        }
        const uploaded = await uploadMediaFile(file, captionBody(item), item.compression_choice);
        if (!uploaded) {
          return;
        }
      }
      setStagedUploadFiles(new Map());
      setSnapshot(await api.clearUploadStaging(roomId));
      setSnapshot(await api.setComposerDraft(roomId, ""));
      setComposerMentions(EMPTY_MENTION_INTENT);
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
    try {
      const nextSnapshot =
        composerMode === "Plain"
          ? await api.sendText(roomId, body, composerMentions)
          : await api.sendReply(
              roomId,
              composerMode.Reply.in_reply_to_event_id,
              body,
              composerMentions
            );
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
    } catch {
      qaSendPending.current = false;
      setQaSendStatus("failed");
      return;
    }
    setComposerMentions(EMPTY_MENTION_INTENT);
  }

  async function scheduleSend(sendAtMs: number) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    const body = composerDraft;
    if (!roomId || !body.trim() || stagedUploads.length > 0) {
      return;
    }

    try {
      setSnapshot(await api.scheduleSend(roomId, body, sendAtMs));
      setComposerMentions(EMPTY_MENTION_INTENT);
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

  async function updateComposerDraft(value: string) {
    const roomId = snapshot?.state.ui.timeline.room_id;
    setComposerMentions((mentions) => pruneMentionIntentForDraft(mentions, value));
    if (!roomId) {
      return;
    }
    try {
      setSnapshot(await api.setComposerDraft(roomId, value));
    } catch {
      // Command failures are surfaced through the Rust-owned error/event path.
    }
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
    setStagedUploadFiles((current) => {
      const next = new Map(current);
      newItems.forEach((item, index) => {
        next.set(item.stagedId, files[index]);
      });
      return next;
    });
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
    setStagedUploadFiles(new Map());
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

  async function setThreadComposerDraft(
    roomId: string,
    rootEventId: string,
    draft: string
  ) {
    setSnapshot(await api.setThreadComposerDraft(roomId, rootEventId, draft));
  }

  async function sendThreadReply(roomId: string, rootEventId: string, body: string) {
    setSnapshot(await api.sendThreadReply(roomId, rootEventId, body));
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
    await setRightPanelModeClosingFocusedContext("closed");
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
          const eventId = snapshot?.state.domain.live_signals.rooms[target.roomId]?.fully_read_event_id ?? "";
          void api.markRoomAsRead(target.roomId, eventId).then(setSnapshot);
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
      if (focusedContextVisibleForMode(rightPanelMode)) {
        await setRightPanelModeClosingFocusedContext("closed");
      } else {
        setSnapshot(await api.getSnapshot());
      }
      return;
    }
    setSnapshot(await api.submitSearch(trimmed, scope));
    if (searchMode) {
      setRightPanelMode(searchMode);
    }
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
  const searchResults = snapshot.state.domain.search.kind === "results" ? snapshot.state.domain.search.results : [];
  const effectiveRightPanelMode = effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot);
  const rightPanelOpen = effectiveRightPanelMode !== "closed";
  const appGridStyle = {
    "--sidebar-width": `${sidebarWidth}px`
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

  return (
    <div className="desktop">
      <TopBar
        activeSpaceName={activeSpace?.display_name ?? t("auth.matrixAccount")}
        isBusy={isBusy}
        searchInputRef={searchInputRef}
        searchQuery={searchQuery}
        searchScope={searchScope}
        sync={snapshot.state.domain.sync}
        onOpenKeyboardSettings={() => {
          void setRightPanelModeClosingFocusedContext("keyboardSettings");
        }}
        onRestartSync={restartSync}
        onSearchQueryChange={setSearchQuery}
        onSearchScopeChange={setSearchScope}
      />
      <div
        className={`app-grid ${rightPanelOpen ? "right-panel-open" : "thread-closed"}`}
        style={appGridStyle}
      >
        <WorkspaceRail
          activeView={primaryView}
          snapshot={snapshot}
          onCreateSpace={() => openCreateDialog("space")}
          onOpenContextMenu={openContextMenu}
          onOpenActivity={() => {
            void openActivityView();
          }}
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
          onCreateRoom={() => openCreateDialog("room")}
          onNewDm={openNewDmDialog}
          onOpenContextMenu={openContextMenu}
          onOpenExplore={() => {
            void openExploreView();
          }}
          onOpenHome={() => {
            void selectSpace(null);
          }}
          onOpenInvites={() => {
            void openInvitesView();
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
          onSelectRoom={selectRoom}
          onSelectRoomListFilter={selectRoomListFilter}
        />
        <button
          className="app-grid-resizer"
          type="button"
          aria-label={t("workspace.resizeRoomList")}
          onPointerDown={beginSidebarResize}
        />
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
              selectSearchResult(row.room_id, row.event_id);
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
            searchQuery={searchQuery}
            searchResults={searchResults}
            showSearchResults={effectiveRightPanelMode !== "search"}
            snapshot={snapshot}
            timelineTransport={appTimelineTransport}
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
            onPaginateBackwards={paginateTimelineBackwards}
            onReply={(roomId, eventId) => {
              void setComposerReplyTarget(roomId, eventId);
            }}
            onRescheduleScheduledSend={(scheduledId, sendAtMs) => {
              void rescheduleScheduledSend(scheduledId, sendAtMs);
            }}
            onScheduleSend={(sendAtMs) => {
              void scheduleSend(sendAtMs);
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
            onToggleThread={() => {
              if (rightPanelOpen) {
                if (effectiveRightPanelMode === "thread") {
                  void closeThread();
                } else {
                  void setRightPanelModeClosingFocusedContext("closed");
                }
              } else {
                // Opening a specific thread is driven by a message's "view replies"
                // action (openThread -> Rust ThreadPaneState), not by scanning the
                // legacy snapshot.timeline placeholder. The panel toggle opens room
                // info as the default right-panel surface.
                void setRightPanelModeClosingFocusedContext("roomInfo");
              }
            }}
            onOpenRoomInfo={() => {
              void setRightPanelModeClosingFocusedContext("roomInfo");
            }}
            onOpenThreadsList={() => {
              const roomId = snapshot.state.ui.navigation.active_room_id;
              if (roomId) {
                void openThreadsListPanel(roomId);
              }
            }}
          />
        )}
        <ContextualRightPanel
          activeRoom={activeRoom ?? null}
          activeSpace={activeSpace ?? null}
          activeSpaceName={activeSpace?.display_name ?? snapshot.sidebar.account_home.display_name}
          isRecoveryBusy={isBusy || sessionKind === "recovering"}
          mode={effectiveRightPanelMode}
          recoverySecretFilled={recoverySecretFilled}
          recoverySecretInputRef={recoverySecretRef}
          snapshot={snapshot}
          timelineTransport={appTimelineTransport}
          searchQuery={searchQuery}
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
              ? () => {
                  spaceSettingsLoadRef.current = null;
                  void api.loadRoomSettings(activeSpace.space_id).then(setSnapshot);
                }
              : undefined
          }
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
          onInviteUser={openInviteUserDialog}
          onModerateMember={(roomId, targetUserId, action, reason) => {
            void moderateRoomMember(roomId, targetUserId, action, reason);
          }}
          onSetLocalUserAlias={(userId, alias) => {
            void setLocalUserAlias(userId, alias);
          }}
          onSetRoomNotificationMode={(roomId, mode) => {
            void setRoomNotificationMode(roomId, mode);
          }}
          onUpdateMemberRole={(roomId, targetUserId, powerLevel) => {
            void updateRoomMemberRole(roomId, targetUserId, powerLevel);
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
            void setThreadComposerDraft(roomId, rootEventId, draft);
          }}
          onThreadReplySend={(roomId, rootEventId, body) => {
            void sendThreadReply(roomId, rootEventId, body);
          }}
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
          isBusy={isBusy || snapshot.state.ui.basic_operation.kind !== "idle"}
          kind={createDialog}
          value={createDraftName}
          onCancel={closeCreateDialog}
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
        <UserIdDialog
          isBusy={isBusy}
          inputLabel={t("dialog.matrixUserId")}
          submitLabel={t("dialog.sendInvite")}
          title={inviteUserDialog.title}
          value={inviteUserDraftUserId}
          onCancel={closeInviteUserDialog}
          onSubmit={() => {
            void submitInviteUserDialog();
          }}
          onValueChange={setInviteUserDraftUserId}
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
    </div>
  );
}

// Preserve App.tsx's original public export surface; these components now live in
// dedicated modules under ./components.
export { Composer } from "./components/composer";
export { ContextualRightPanel } from "./components/rightPanel";
export { TopBar, WorkspaceRail } from "./components/Shell";
