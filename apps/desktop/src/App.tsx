import {
  AtSign,
  Bell,
  Bold,
  ChevronLeft,
  ChevronRight,
  Check,
  Clock3,
  Code2,
  Compass,
  Edit3,
  FileText,
  Hash,
  HelpCircle,
  Home,
  Image as ImageIcon,
  Italic,
  KeyRound,
  Link2,
  List,
  MessageCircle,
  MoreHorizontal,
  MoreVertical,
  PanelRightClose,
  PanelRightOpen,
  Paperclip,
  Pin,
  PinOff,
  Plus,
  Search,
  RefreshCw,
  Send,
  Settings,
  ShieldCheck,
  Smile,
  Users,
  X,
  ZoomIn,
  ZoomOut
} from "lucide-react";
import {
  type FormEvent,
  type ChangeEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useMemo,
  useRef,
  useState
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { createDesktopApi } from "./backend/client";
import { setActiveLocaleProfile, t, type MessageId } from "./i18n/messages";
import { ContextMenuSurface } from "./components/ContextMenuSurface";
import {
  TimelineView,
  renderTimelineMessageText,
  type TimelineForwardDestination,
  type TimelineRowActionHandlers,
  type TimelineTransport
} from "./components/TimelineView";
import {
  type CoreEventPayload,
  type TimelineKey,
  focusedTimelineKey,
  roomTimelineKey,
  threadTimelineKey
} from "./domain/coreEvents";
import { KeyboardSettingsPanel } from "./components/KeyboardSettingsPanel";
import { RoomInfoPanel } from "./components/RoomInfoPanel";
import { SpaceInfoPanel } from "./components/SpaceInfoPanel";
import { Tooltip } from "./components/Tooltip";
import { UserSettingsPanel } from "./components/UserSettingsPanel";
import {
  type ContextMenuActionId,
  type ContextMenuItem,
  contextMenuItems
} from "./domain/contextMenus";
import {
  shortcutActionFromMenuPayload,
  type ShortcutLabelProfile,
  shortcutIdForKeyboardEvent
} from "./domain/shortcuts";
import {
  composerKeyEventFromDom,
  insertNewlineAtSelection,
  shouldLetNativeImeHandleComposerKeyEvent,
  shouldResolveComposerKeyEvent
} from "./domain/composerKeyEvents";
import { roomListSections } from "./domain/desktopModel";
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
  ActivityRow,
  ActivityState,
  ActivityStream,
  ActivityTab,
  AuthFailureKind,
  ComposerMode,
  DesktopSnapshot,
  DirectoryRoomSummary,
  ImageUploadCompressionMode,
  ImageUploadCompressionPolicy,
  LocaleDisplayProfile,
  LoginFlow,
  MentionIntent,
  MentionTarget,
  OperationFailureKind,
  ResolveComposerKeyAction,
  RoomListFilter,
  RoomListItem,
  RoomModerationAction,
  RoomSettingChange,
  RoomTags,
  SavedSessionInfo,
  SearchResult,
  SearchScopeKind,
  ScheduledSendCapability,
  ScheduledSendItem,
  SettingsPatch,
  StagedUploadCompressionChoice,
  StagedUploadItem,
  TimelineMessage,
  TimelineMediaGalleryItem,
  UploadStagingRequestItem,
  UserProfile
} from "./domain/types";

const api = createDesktopApi();
const DEFAULT_HOMESERVER = "https://matrix.org";
const MENU_EVENT_NAME = "matrix-desktop://menu";
const STATE_EVENT_NAME = "matrix-desktop://state";
const CORE_EVENT_NAME = "matrix-desktop://event";
const EMPTY_ROOM_TAGS: RoomTags = { favourite: null, low_priority: null };
const EMPTY_MENTION_INTENT: MentionIntent = { targets: [] };
const ICON_SIZE = {
  micro: 14,
  compact: 15,
  small: 16,
  input: 17,
  control: 18,
  panel: 19,
  rail: 20,
  large: 22,
  auth: 23,
  emptyState: 24
} as const;

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

type ImageUploadDimensionsPayload = {
  width: number;
  height: number;
};

type ImageUploadVariantInfoPayload = {
  mime_type: string;
  byte_count: number;
  dimensions: ImageUploadDimensionsPayload | null;
};

type ImageUploadVariantKindPayload = "Original" | "Compressed";

type ImageUploadCompressionStatePayload = {
  mode: ImageUploadCompressionMode;
  policy: ImageUploadCompressionPolicy;
  original: ImageUploadVariantInfoPayload;
  selected: ImageUploadVariantInfoPayload;
  selected_variant: ImageUploadVariantKindPayload;
  skipped_small_image: boolean;
  metadata_stripped: boolean;
  thumbnail_refreshed: boolean;
};

type UploadMediaThumbnailPayload = {
  mime_type: string;
  bytes: number[];
  width: number;
  height: number;
};

type PreparedMediaUpload = {
  filename: string;
  mimeType: string;
  bytes: number[];
  imageDimensions?: ImageUploadDimensionsPayload;
  imageCompression?: ImageUploadCompressionStatePayload;
  thumbnail?: UploadMediaThumbnailPayload;
};

type ImageCompressionVariant = {
  filename: string;
  mimeType: string;
  bytes: number[];
  byteCount: number;
  dimensions: ImageUploadDimensionsPayload;
  previewUrl: string;
  thumbnail: UploadMediaThumbnailPayload;
};

type ImageCompressionPlan = {
  mode: ImageUploadCompressionMode;
  policy: ImageUploadCompressionPolicy;
  original: ImageCompressionVariant;
  compressed: ImageCompressionVariant;
  skippedSmallImage: boolean;
};

type ImageCompressionDialogState = {
  plan: ImageCompressionPlan;
  resolve: (choice: ImageUploadVariantKindPayload | "cancel") => void;
};

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
type ContextMenuTarget =
  | { kind: "message"; message: TimelineMessage }
  | { kind: "room"; roomId: string }
  | { kind: "space"; spaceId: string }
  | { kind: "account" };
type OpenContextMenu = (
  event: MouseEvent<HTMLElement>,
  target: ContextMenuTarget,
  items: ContextMenuItem[]
) => void;
type ActiveContextMenu = {
  x: number;
  y: number;
  target: ContextMenuTarget;
  items: ContextMenuItem[];
};
type PrimaryView = "timeline" | "invites" | "explore" | "activity";
type InviteUserDialogState = {
  roomId: string;
  title: string;
} | null;

const ignoreComposerKeyAction: ResolveComposerKeyAction = async () => "noop";

/**
 * React-local view of the composer mode. Matrix semantics (the reply target)
 * stay Rust-owned; this is a presentational mapping of the WIRE value
 * `snapshot.state.timeline.composer.mode` (externally tagged ComposerMode).
 */
type ComposerModeProp =
  | { kind: "plain" }
  | { kind: "reply"; in_reply_to_event_id: string };

type MentionCandidate = {
  key: string;
  label: string;
  searchText: string;
  target: MentionTarget;
};

function composerModeProp(mode: ComposerMode): ComposerModeProp {
  return mode === "Plain"
    ? { kind: "plain" }
    : { kind: "reply", in_reply_to_event_id: mode.Reply.in_reply_to_event_id };
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

function formatUploadBytes(byteCount: number): string {
  if (byteCount >= 1024 * 1024) {
    return `${(byteCount / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (byteCount >= 1024) {
    return `${Math.max(1, Math.round(byteCount / 1024))} KB`;
  }
  return `${byteCount} B`;
}

function formatUploadDimensions(dimensions: ImageUploadDimensionsPayload): string {
  return `${dimensions.width}x${dimensions.height}`;
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
  return mode === "always" ? { kind: "compressed", mode } : { kind: "original" };
}

function forcedUploadMode(
  choice: StagedUploadCompressionChoice | undefined,
  fallback: ImageUploadCompressionMode
): ImageUploadCompressionMode {
  if (choice?.kind === "compressed") {
    return "always";
  }
  if (choice?.kind === "original") {
    return "never";
  }
  return fallback;
}

function captionBody(item: StagedUploadItem): string {
  return item.caption?.plain_body ?? "";
}

function mediaGalleryItemLabel(item: TimelineMediaGalleryItem): string {
  return item.media.filename || item.event_id;
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

export function App() {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot | null>(null);
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
  const [rightPanelMode, setRightPanelMode] = useState<RightPanelMode>("thread");
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
  const appTimelineTransport = useMemo<TimelineTransport | null>(() => {
    if (!tauriTimelineTransport) {
      return null;
    }
    return {
      ...tauriTimelineTransport,
      async openAtTimestamp(roomId: string, timestampMs: number) {
        const nextSnapshot = await api.openTimelineAtTimestamp(roomId, timestampMs);
        setSnapshot(nextSnapshot);
        setPrimaryView("timeline");
        setRightPanelMode("focusedContext");
      }
    };
  }, []);
  const attentionSummary = snapshot
    ? desktopAttentionSummary(snapshot.state.native_attention)
    : null;
  const safeAttentionSummary =
    attentionSummary ?? {
      unreadTotal: 0,
      badgeCount: 0,
      notificationKind: "none" as const,
      titleHint: null,
      qaTitleToken: "unread=0 badge=0 notify=none"
    };
  const composerDraft = snapshot?.state.timeline.composer.draft ?? "";
  const stagedUploads = snapshot?.state.timeline.staged_uploads ?? [];
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
    const roomId = snapshot?.state.timeline.room_id ?? null;
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
  }, [composerDraft, snapshot?.state.timeline.room_id]);

  useEffect(() => {
    const theme = snapshot?.state.settings.values.appearance.theme ?? "system";
    if (theme === "system") {
      delete document.documentElement.dataset.theme;
      return;
    }
    document.documentElement.dataset.theme = theme;
  }, [snapshot?.state.settings.values.appearance.theme]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    const profile = snapshot.state.locale_profile;
    document.documentElement.lang = profile.lang;
    document.documentElement.dir = profile.dir;
    document.documentElement.dataset.catalogLocale = profile.catalog_locale;
    document.documentElement.dataset.pseudoLocale = profile.pseudo_locale;
  }, [
    snapshot?.state.locale_profile.lang,
    snapshot?.state.locale_profile.dir,
    snapshot?.state.locale_profile.catalog_locale,
    snapshot?.state.locale_profile.pseudo_locale
  ]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    const profile = snapshot.state.typography_profile;
    document.documentElement.dataset.uiFont = profile.font;
    document.documentElement.dataset.emojiFont = profile.emoji;
    document.documentElement.dataset.fontAsset = profile.font_asset;
    document.documentElement.dataset.emojiAsset = profile.emoji_asset;
  }, [
    snapshot?.state.typography_profile.font,
    snapshot?.state.typography_profile.emoji,
    snapshot?.state.typography_profile.font_asset,
    snapshot?.state.typography_profile.emoji_asset
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
    snapshot?.state.navigation.active_room_id,
    snapshot?.state.navigation.active_space_id
  ]);

  useEffect(() => {
    const title = snapshot
      ? qaTitleEnabled()
        ? qaWindowTitle(
            snapshot,
            effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot),
            qaSendStatus
          )
        : desktopAttentionWindowTitle("matrix-desktop", safeAttentionSummary)
      : qaTitleEnabled()
        ? "matrix-desktop qa session=booting"
        : "matrix-desktop";

    document.title = title;
    if (!isTauriRuntime()) {
      return;
    }

    void applyDesktopAttentionToWindow(
      getCurrentWindow(),
      title,
      safeAttentionSummary.badgeCount,
      snapshot?.state.native_attention.summary.capabilities
    );
  }, [snapshot, rightPanelMode, qaSendStatus, safeAttentionSummary.badgeCount, safeAttentionSummary.qaTitleToken]);

  useEffect(() => {
    if (!snapshot) {
      return;
    }

    const candidate = desktopAttentionNotificationCandidate(
      snapshot.state.native_attention
    );

    if (!candidate || !tauriNotificationTransport) {
      return;
    }

    void dispatchDesktopAttentionTransientEffects(
      getCurrentWindow(),
      candidate,
      snapshot.state.native_attention.summary.capabilities,
      snapshot.state.settings.values.notifications
    );
    void sendDesktopAttentionNotification(candidate, tauriNotificationTransport);
  }, [
    snapshot?.state.native_attention.dispatch.kind,
    snapshot?.state.native_attention.summary.candidate?.room_display_name,
    snapshot?.state.native_attention.summary.candidate?.kind,
    snapshot?.state.native_attention.summary.candidate?.unread_count,
    snapshot?.state.native_attention.summary.candidate?.highlight_count
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
    const roomId = snapshot.state.timeline.room_id;
    if (!roomId) {
      return;
    }

    qaSendStarted.current = true;
    qaSendBaselineErrorCount.current = snapshot.state.errors.length;
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
    const activeRoomId = snapshot.state.navigation.active_room_id;
    if (!activeRoomId) {
      return;
    }
    const roomManagement = snapshot.state.room_management;
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
    snapshot?.state.navigation.active_room_id,
    snapshot?.state.room_management.operation,
    snapshot?.state.room_management.selected_room_id,
    snapshot?.state.room_management.settings
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

  async function setDisplayName(displayName: string | null) {
    setSnapshot(await api.setDisplayName(displayName));
  }

  async function setLocalUserAlias(userId: string, alias: string | null) {
    setSnapshot(await api.setLocalUserAlias(userId, alias));
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
    if (!alias || isBusy || snapshot?.state.directory.join.kind === "joining") {
      return;
    }
    setSnapshot(await api.joinDirectoryRoom(alias, serverNameFromAlias(alias)));
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
      setSnapshot(await api.acceptInvite(roomId));
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
    // Guard against double-submit: a create already in flight (isBusy) or a
    // pending basic_operation (Rust-owned) must block re-entry.
    if (
      !kind ||
      !name ||
      isBusy ||
      (snapshot && snapshot.state.basic_operation.kind !== "idle")
    ) {
      return;
    }
    setIsBusy(true);
    try {
      const nextSnapshot =
        kind === "space" ? await api.createSpace(name) : await api.createRoom(name);
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
    const roomId = snapshot?.state.timeline.room_id;
    const body = composerDraft;
    const uploads = snapshot?.state.timeline.staged_uploads ?? [];
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
    const composerMode = snapshot?.state.timeline.composer.mode ?? "Plain";

    qaSendStarted.current = true;
    qaSendBaselineErrorCount.current = snapshot?.state.errors.length ?? 0;
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
    const roomId = snapshot?.state.timeline.room_id;
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
    const roomId = snapshot?.state.timeline.room_id;
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
    const roomId = snapshot?.state.timeline.room_id;
    if (!roomId || files.length === 0) {
      return;
    }
    const startPosition = stagedUploads.length;
    const mediaSettings = snapshot.state.settings.values.media;
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
    const roomId = snapshot?.state.timeline.room_id;
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
    const roomId = snapshot?.state.timeline.room_id;
    if (!roomId || !isTauriRuntime()) {
      return false;
    }

    const mediaSettings = snapshot.state.settings.values.media;
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

  function settleImageCompressionDialog(choice: ImageUploadVariantKindPayload | "cancel") {
    if (!imageCompressionDialog) {
      return;
    }
    imageCompressionDialog.resolve(choice);
    releaseImageCompressionPlan(imageCompressionDialog.plan);
    setImageCompressionDialog(null);
  }

  async function editMessage(message: TimelineMessage) {
    const body = window.prompt(t("timeline.editMessage"), message.body);
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
    const focusedContext = snapshot?.state.focused_context;
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
          const eventId = snapshot?.state.live_signals.rooms[target.roomId]?.fully_read_event_id ?? "";
          void api.markRoomAsRead(target.roomId, eventId).then(setSnapshot);
          return;
        }
        case "markRoomAsUnread":
          void api.markRoomAsUnread(target.roomId, true).then(setSnapshot);
          return;
        default:
          break;
      }
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
    return <div className="boot-screen">matrix-desktop</div>;
  }

  const sessionKind = snapshot.state.session.kind;
  const recoveryRequired = sessionKind === "needsRecovery" || sessionKind === "recovering";

  if (sessionKind === "restoring" || sessionKind === "loggingOut") {
    return <div className="boot-screen">matrix-desktop</div>;
  }

  setActiveLocaleProfile(
    snapshot.state.locale_profile.catalog_locale,
    snapshot.state.locale_profile.pseudo_locale
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

  const activeRoom = snapshot.state.rooms.find(
    (room) => room.room_id === snapshot.state.navigation.active_room_id
  );
  const activeSpace = snapshot.state.spaces.find(
    (space) => space.space_id === snapshot.state.navigation.active_space_id
  );
  const searchResults = snapshot.state.search.kind === "results" ? snapshot.state.search.results : [];
  const effectiveRightPanelMode = effectiveRightPanelModeForSnapshot(rightPanelMode, snapshot);
  const rightPanelOpen = effectiveRightPanelMode !== "closed";

  return (
    <div className="desktop">
      <TopBar
        activeSpaceName={activeSpace?.display_name ?? t("auth.matrixAccount")}
        isBusy={isBusy}
        searchInputRef={searchInputRef}
        searchQuery={searchQuery}
        searchScope={searchScope}
        sync={snapshot.state.sync}
        onOpenKeyboardSettings={() => {
          void setRightPanelModeClosingFocusedContext("keyboardSettings");
        }}
        onRestartSync={restartSync}
        onSearchQueryChange={setSearchQuery}
        onSearchScopeChange={setSearchScope}
      />
      <div className={`app-grid ${rightPanelOpen ? "right-panel-open" : "thread-closed"}`}>
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
          onSelectSpace={selectSpace}
        />
        <Sidebar
          activeRoomId={snapshot.state.navigation.active_room_id}
          activeView={primaryView}
          snapshot={snapshot}
          onCreateRoom={() => openCreateDialog("room")}
          onNewDm={openNewDmDialog}
          onOpenContextMenu={openContextMenu}
          onOpenExplore={() => {
            void openExploreView();
          }}
          onOpenInvites={() => {
            void openInvitesView();
          }}
          onOpenSpaceInfo={() => {
            void setRightPanelModeClosingFocusedContext("spaceInfo");
          }}
          onSelectRoom={selectRoom}
          onSelectRoomListFilter={selectRoomListFilter}
        />
        {primaryView === "activity" ? (
          <ActivityPane
            activity={snapshot.state.activity}
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
            composerMode={composerModeProp(snapshot.state.timeline.composer.mode)}
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
          onQueryDevices={() => {
            void queryDevices();
          }}
          onRenameDevice={(deviceOrdinal, displayName) => {
            void renameDevice(deviceOrdinal, displayName);
          }}
          onDeleteDevices={(deviceOrdinals) => {
            void deleteDevices(deviceOrdinals);
          }}
          onSubmitAccountManagementUia={(flowId, password) => {
            void submitAccountManagementUia(flowId, password);
          }}
          onUpdateRoomSetting={(roomId, change) => {
            void updateRoomSetting(roomId, change);
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
          isBusy={isBusy || snapshot.state.basic_operation.kind !== "idle"}
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
      {imageCompressionDialog ? (
        <ImageCompressionDialog
          plan={imageCompressionDialog.plan}
          onCancel={() => settleImageCompressionDialog("cancel")}
          onChoose={(choice) => settleImageCompressionDialog(choice)}
        />
      ) : null}
    </div>
  );
}

/**
 * Modal for creating a room or a space. Open state + the unsent name are
 * React-local (passed in); the create itself goes through the Rust-owned API.
 */
function CreateEntityDialog({
  isBusy,
  kind,
  value,
  onCancel,
  onSubmit,
  onValueChange
}: {
  isBusy: boolean;
  kind: "room" | "space";
  value: string;
  onCancel: () => void;
  onSubmit: () => void;
  onValueChange: (value: string) => void;
}) {
  const isSpace = kind === "space";
  const title = isSpace ? t("dialog.createSpaceTitle") : t("dialog.createRoomTitle");
  const inputLabel = isSpace ? t("dialog.spaceName") : t("dialog.roomName");
  const submitLabel = isSpace
    ? t("dialog.submitCreateSpace")
    : t("dialog.submitCreateRoom");
  const canSubmit = value.trim().length > 0 && !isBusy;

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={onDialogKeyDown}
    >
      <form
        className="dialog-box"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) {
            onSubmit();
          }
        }}
      >
        <div className="dialog-title">{title}</div>
        <input
          className="dialog-input"
          type="text"
          autoFocus
          aria-label={inputLabel}
          placeholder={inputLabel}
          value={value}
          onChange={(event) => onValueChange(event.target.value)}
        />
        <div className="dialog-actions">
          <button
            className="dialog-button"
            type="button"
            aria-label={t("dialog.cancelCreate")}
            onClick={onCancel}
          >
            {t("action.cancel")}
          </button>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={submitLabel}
            disabled={!canSubmit}
          >
            {isSpace ? t("action.createSpace") : t("action.createRoom")}
          </button>
        </div>
      </form>
    </div>
  );
}

function ImageCompressionDialog({
  plan,
  onCancel,
  onChoose
}: {
  plan: ImageCompressionPlan;
  onCancel: () => void;
  onChoose: (choice: ImageUploadVariantKindPayload) => void;
}) {
  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={t("composer.imageCompressionTitle")}
      onKeyDown={onDialogKeyDown}
    >
      <div className="dialog-box image-compression-dialog">
        <div className="dialog-title">{t("composer.imageCompressionTitle")}</div>
        <div className="image-compression-preview">
          <img src={plan.compressed.previewUrl} alt={t("composer.imageCompressionPreviewAlt")} />
        </div>
        <div className="image-compression-options">
          <button
            className="image-compression-option"
            type="button"
            onClick={() => onChoose("Original")}
          >
            <span>{t("composer.imageCompressionOriginal")}</span>
            <strong>
              {formatUploadBytes(plan.original.byteCount)} · {formatUploadDimensions(plan.original.dimensions)}
            </strong>
          </button>
          <button
            className="image-compression-option is-preferred"
            type="button"
            autoFocus
            onClick={() => onChoose("Compressed")}
          >
            <span>{t("composer.imageCompressionCompressed")}</span>
            <strong>
              {formatUploadBytes(plan.compressed.byteCount)} · {formatUploadDimensions(plan.compressed.dimensions)}
            </strong>
          </button>
        </div>
        <div className="dialog-actions">
          <button className="dialog-button" type="button" onClick={onCancel}>
            {t("dialog.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}

function UserIdDialog({
  inputLabel,
  isBusy,
  submitLabel,
  title,
  value,
  onCancel,
  onSubmit,
  onValueChange
}: {
  inputLabel: string;
  isBusy: boolean;
  submitLabel: string;
  title: string;
  value: string;
  onCancel: () => void;
  onSubmit: () => void;
  onValueChange: (value: string) => void;
}) {
  const canSubmit = value.trim().length > 0 && !isBusy;

  function onDialogKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="dialog-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onKeyDown={onDialogKeyDown}
    >
      <form
        className="dialog-box"
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) {
            onSubmit();
          }
        }}
      >
        <div className="dialog-title">{title}</div>
        <input
          className="dialog-input"
          type="text"
          autoFocus
          aria-label={inputLabel}
          placeholder={inputLabel}
          spellCheck={false}
          value={value}
          onChange={(event) => onValueChange(event.target.value)}
        />
        <div className="dialog-actions">
          <button
            className="dialog-button"
            type="button"
            aria-label={t("action.cancel")}
            onClick={onCancel}
          >
            {t("action.cancel")}
          </button>
          <button
            className="dialog-button is-primary"
            type="submit"
            aria-label={submitLabel}
            disabled={!canSubmit}
          >
            {submitLabel}
          </button>
        </div>
      </form>
    </div>
  );
}

function RecoveryPanel({
  isBusy,
  secretFilled,
  secretInputRef,
  snapshot,
  onSecretPresenceChange,
  onSubmit
}: {
  isBusy: boolean;
  secretFilled: boolean;
  secretInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  onSecretPresenceChange: (value: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}) {
  const primaryError = snapshot.state.errors.at(-1);
  const session = snapshot.state.session;

  return (
    <section className="recovery-panel-body" data-testid="recovery-panel">
      <form className="recovery-panel-form" onSubmit={onSubmit}>
        <div className="auth-brand">
          <div className="auth-mark recovery-mark">
            <ShieldCheck size={ICON_SIZE.auth} />
          </div>
          <div>
            <h1>{t("auth.encryptionRecovery")}</h1>
            <p dir="auto">{session.user_id ?? t("auth.matrixAccount")}</p>
          </div>
        </div>
        <div className="recovery-summary">
          <KeyRound size={ICON_SIZE.control} />
          <div className="recovery-methods" aria-label={t("auth.supportedRecoveryMethods")}>
            {(session.recovery_methods ?? ["recoveryKey", "securityPhrase"]).map((method) => (
              <span className="recovery-chip" key={method}>
                {recoveryMethodLabel(method)}
              </span>
            ))}
          </div>
        </div>
        <label className="auth-field">
          <span>{t("auth.recoverySecret")}</span>
          <input
            autoComplete="off"
            name="recoverySecret"
            ref={secretInputRef}
            spellCheck={false}
            type="password"
            onInput={(event) => onSecretPresenceChange(event.currentTarget.value.length > 0)}
          />
        </label>
        {primaryError ? (
          <div className="auth-error" role="alert">
            {primaryError.message}
          </div>
        ) : null}
        <button className="auth-submit" disabled={isBusy || !secretFilled} type="submit">
          {isBusy ? t("action.recovering") : t("action.recover")}
        </button>
      </form>
    </section>
  );
}

function AuthScreen({
  deviceName,
  homeserver,
  isBusy,
  passwordFilled,
  passwordInputRef,
  snapshot,
  username,
  onDeviceNameChange,
  onDiscoverLoginMethods,
  onHomeserverChange,
  onPasswordPresenceChange,
  onSubmit,
  onUsernameChange
}: {
  deviceName: string;
  homeserver: string;
  isBusy: boolean;
  passwordFilled: boolean;
  passwordInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  username: string;
  onDeviceNameChange: (value: string) => void;
  onDiscoverLoginMethods: () => void;
  onHomeserverChange: (value: string) => void;
  onPasswordPresenceChange: (value: boolean) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onUsernameChange: (value: string) => void;
}) {
  const primaryError = snapshot.state.errors.at(-1);
  const auth = snapshot.state.auth;
  const passwordLoginAvailable =
    auth.kind !== "ready" || auth.flows.some((flow) => flow.kind === "password");

  return (
    <main className="auth-screen" data-testid="auth-screen">
      <form className="auth-panel" onSubmit={onSubmit}>
        <div className="auth-brand">
          <div className="auth-mark">
            <Hash size={ICON_SIZE.large} />
          </div>
          <div>
            <h1>{t("auth.matrixDesktop")}</h1>
            <p>{sessionLabel(snapshot.state.session.kind)}</p>
          </div>
        </div>
        <label className="auth-field">
          <span>{t("settings.homeserver")}</span>
          <input
            autoComplete="url"
            name="homeserver"
            spellCheck={false}
            value={homeserver}
            onChange={(event) => onHomeserverChange(event.target.value)}
          />
        </label>
        <div className="auth-discovery">
          <button
            className="auth-secondary"
            disabled={isBusy || !homeserver.trim()}
            type="button"
            onClick={onDiscoverLoginMethods}
          >
            {t("auth.checkLoginMethods")}
          </button>
          <div className="auth-flows">{authDiscoveryLabel(auth)}</div>
        </div>
        <label className="auth-field">
          <span>{t("auth.usernameOrMatrixId")}</span>
          <input
            autoComplete="username"
            name="username"
            spellCheck={false}
            value={username}
            onChange={(event) => onUsernameChange(event.target.value)}
          />
        </label>
        <label className="auth-field">
          <span>{t("auth.password")}</span>
          <input
            autoComplete="current-password"
            name="password"
            ref={passwordInputRef}
            type="password"
            disabled={!passwordLoginAvailable}
            onInput={(event) => onPasswordPresenceChange(event.currentTarget.value.length > 0)}
          />
        </label>
        <label className="auth-field">
          <span>{t("auth.deviceName")}</span>
          <input
            autoComplete="off"
            name="deviceName"
            spellCheck={false}
            value={deviceName}
            onChange={(event) => onDeviceNameChange(event.target.value)}
          />
        </label>
        {primaryError ? (
          <div className="auth-error" role="alert">
            {primaryError.message}
          </div>
        ) : null}
        <button
          className="auth-submit"
          disabled={
            isBusy ||
            !homeserver.trim() ||
            !username.trim() ||
            !passwordFilled ||
            !passwordLoginAvailable
          }
          type="submit"
        >
          {isBusy ? t("auth.connecting") : t("auth.continue")}
        </button>
      </form>
    </main>
  );
}

function authDiscoveryLabel(auth: DesktopSnapshot["state"]["auth"]) {
  switch (auth.kind) {
    case "discovering":
      return t("auth.checking");
    case "ready": {
      const labels = auth.flows.map(authFlowLabel);
      return labels.length ? labels.join(" / ") : t("auth.noLoginMethods");
    }
    case "failed":
      return authFailureLabel(auth.failureKind);
    case "unknown":
    default:
      return t("auth.notChecked");
  }
}

function authFlowLabel(flow: LoginFlow): string {
  if (flow.display_name) {
    return flow.display_name;
  }

  switch (flow.kind) {
    case "password":
      return t("auth.flowPassword");
    case "sso":
      return t("auth.flowSso");
    case "oidc":
      return t("auth.flowOidc");
    case "token":
      return t("auth.flowToken");
    default:
      return t("auth.flowUnknown");
  }
}

function authFailureLabel(kind: AuthFailureKind): string {
  switch (kind) {
    case "network":
      return t("auth.failureNetwork");
    case "unsupported":
      return t("auth.failureUnsupported");
    case "cancelled":
      return t("auth.notChecked");
    case "forbidden":
      return t("auth.failureForbidden");
    case "timeout":
      return t("auth.failureTimeout");
    case "sdk":
      return t("auth.failureSdk");
  }
}

function sessionLabel(kind: DesktopSnapshot["state"]["session"]["kind"]) {
  switch (kind) {
    case "authenticating":
      return t("auth.connecting");
    case "needsRecovery":
      return t("auth.encryptionRecovery");
    case "recovering":
      return t("action.recovering");
    case "locked":
      return t("auth.sessionLocked");
    case "signedOut":
    default:
      return t("auth.signIn");
  }
}

function recoveryMethodLabel(
  method: NonNullable<DesktopSnapshot["state"]["session"]["recovery_methods"]>[number]
) {
  switch (method) {
    case "recoveryKey":
      return t("auth.recoveryKey");
    case "securityPhrase":
      return t("auth.securityPhrase");
  }
}

type SyncPresentation = {
  state: "running" | "starting" | "reconnecting" | "failed" | "stopped";
  label: string;
  detail: string | null;
  ariaLabel: string;
  restartable: boolean;
};

function syncStatePresentation(sync: DesktopSnapshot["state"]["sync"]): SyncPresentation {
  if (typeof sync === "string") {
    switch (sync) {
      case "starting":
        return {
          state: "starting",
          label: t("sync.starting"),
          detail: null,
          ariaLabel: t("sync.starting"),
          restartable: false
        };
      case "running":
        return {
          state: "running",
          label: t("sync.running"),
          detail: null,
          ariaLabel: t("sync.running"),
          restartable: false
        };
      case "stopped":
      default:
        return {
          state: "stopped",
          label: t("sync.stopped"),
          detail: null,
          ariaLabel: t("sync.stopped"),
          restartable: true
        };
    }
  }

  if ("reconnecting" in sync) {
    return {
      state: "reconnecting",
      label: t("sync.reconnecting"),
      detail: sync.reconnecting,
      ariaLabel: sync.reconnecting
        ? t("sync.reconnectingWithReason", { reason: sync.reconnecting })
        : t("sync.reconnecting"),
      restartable: true
    };
  }

  return {
    state: "failed",
    label: t("sync.failed"),
    detail: sync.failed,
    ariaLabel: sync.failed ? t("sync.failedWithReason", { reason: sync.failed }) : t("sync.failed"),
    restartable: true
  };
}

export function TopBar({
  activeSpaceName,
  isBusy,
  searchInputRef,
  searchQuery,
  searchScope,
  sync,
  onOpenKeyboardSettings,
  onRestartSync,
  onSearchQueryChange,
  onSearchScopeChange
}: {
  activeSpaceName: string;
  isBusy: boolean;
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchQuery: string;
  searchScope: SearchScopeKind;
  sync: DesktopSnapshot["state"]["sync"];
  onOpenKeyboardSettings: () => void;
  onRestartSync: () => void;
  onSearchQueryChange: (value: string) => void;
  onSearchScopeChange: (value: SearchScopeKind) => void;
}) {
  const syncStatus = syncStatePresentation(sync);
  return (
    <header className="titlebar">
      <div className="traffic">
        <span className="dot red" />
        <span className="dot yellow" />
        <span className="dot green" />
      </div>
      <div className="history">
        <button className="icon-button" type="button" aria-label={t("action.back")}>
          ‹
        </button>
        <button className="icon-button" type="button" aria-label={t("action.forward")}>
          ›
        </button>
        <button className="icon-button" type="button" aria-label={t("action.history")}>
          <Clock3 size={ICON_SIZE.control} />
        </button>
      </div>
      <label className="top-search">
        <Search size={ICON_SIZE.input} />
        <input
          ref={searchInputRef}
          aria-label={t("workspace.search")}
          value={searchQuery}
          dir="auto"
          placeholder={t("workspace.searchPlaceholder", { spaceName: activeSpaceName })}
          onChange={(event) => onSearchQueryChange(event.target.value)}
        />
      </label>
      <select
        className="scope-select"
        aria-label={t("workspace.searchScope")}
        value={searchScope}
        onChange={(event) => onSearchScopeChange(event.target.value as SearchScopeKind)}
      >
        <option value="allRooms">{t("search.scopeAll")}</option>
        <option value="currentSpace">{t("search.scopeSpace")}</option>
        <option value="currentRoom">{t("search.scopeRoom")}</option>
        <option value="dms">{t("search.scopeDm")}</option>
      </select>
      <div className="top-actions">
        <div
          className="sync-status"
          data-sync-state={syncStatus.state}
          role="status"
          aria-live="polite"
          aria-label={syncStatus.ariaLabel}
        >
          <span className={`sync-dot ${isBusy ? "busy" : ""}`} />
          <span className="sync-status-label">{syncStatus.label}</span>
          {syncStatus.detail ? (
            <span className="sync-status-detail">{syncStatus.detail}</span>
          ) : null}
        </div>
        {syncStatus.restartable ? (
          <button
            className="icon-button"
            type="button"
            aria-label={t("action.restartSync")}
            disabled={isBusy}
            onClick={onRestartSync}
          >
            <RefreshCw size={ICON_SIZE.control} />
          </button>
        ) : null}
        <button
          className="icon-button"
          type="button"
          aria-label={t("shortcut.showKeyboardSettings")}
          onClick={onOpenKeyboardSettings}
        >
          <HelpCircle size={ICON_SIZE.control} />
        </button>
      </div>
    </header>
  );
}

export function WorkspaceRail({
  activeView,
  snapshot,
  onCreateSpace,
  onOpenContextMenu,
  onOpenActivity,
  onOpenUserSettings,
  onSelectSpace
}: {
  activeView: PrimaryView;
  snapshot: DesktopSnapshot;
  onCreateSpace: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenActivity: () => void;
  onOpenUserSettings: () => void;
  onSelectSpace: (spaceId: string | null) => void;
}) {
  return (
    <nav className="workspace-rail" aria-label={t("workspace.workspaces")}>
      <div className="workspace-list">
        <button
          className={`workspace-button ${activeView === "activity" ? "is-active" : ""}`}
          data-count={snapshot.sidebar.account_home.unread_count || undefined}
          data-mention-count={snapshot.sidebar.account_home.highlight_count || undefined}
          type="button"
          aria-label={t("workspace.activity")}
          onClick={onOpenActivity}
        >
          <Clock3 size={ICON_SIZE.rail} />
        </button>
        <Tooltip label={snapshot.sidebar.account_home.display_name}>
          {(tooltipProps) => (
            <button
              className={`workspace-button ${
                activeView === "timeline" && snapshot.sidebar.account_home.is_active
                  ? "is-active"
                  : ""
              }`}
              data-count={snapshot.sidebar.account_home.unread_count || undefined}
              data-mention-count={snapshot.sidebar.account_home.highlight_count || undefined}
              type="button"
              aria-label={snapshot.sidebar.account_home.display_name}
              onClick={() => onSelectSpace(null)}
              {...tooltipProps}
            >
              <Home size={ICON_SIZE.rail} />
            </button>
          )}
        </Tooltip>
        {snapshot.sidebar.space_rail.map((space) => (
          <Tooltip label={space.display_name} key={space.space_id}>
            {(tooltipProps) => (
              <button
                className={`workspace-button ${space.is_active ? "is-active" : ""}`}
                data-count={space.unread_count || undefined}
                data-mention-count={space.highlight_count || undefined}
                type="button"
                aria-label={space.display_name}
                onClick={() => onSelectSpace(space.space_id)}
                onContextMenu={(event) =>
                  onOpenContextMenu(
                    event,
                    { kind: "space", spaceId: space.space_id },
                    contextMenuItems({ kind: "space" })
                  )
                }
                {...tooltipProps}
              >
                <EntityAvatar
                  avatar={space.avatar}
                  className="workspace-button-avatar is-space"
                  fallback={initials(space.display_name)}
                />
              </button>
            )}
          </Tooltip>
        ))}
      </div>
      <div className="rail-footer">
        <button
          className="rail-action"
          type="button"
          aria-label={t("action.createSpace")}
          onClick={onCreateSpace}
        >
          <Plus size={ICON_SIZE.large} />
        </button>
        <button
          className="user-presence"
          type="button"
          aria-label={t("workspace.userSettings")}
          onClick={onOpenUserSettings}
          onContextMenu={(event) =>
            onOpenContextMenu(event, { kind: "account" }, contextMenuItems({ kind: "account" }))
          }
        />
      </div>
    </nav>
  );
}

const ROOM_LIST_FILTERS: { filter: RoomListFilter; label: MessageId }[] = [
  { filter: { kind: "rooms" }, label: "roomList.filterRooms" },
  { filter: { kind: "unread" }, label: "roomList.filterUnread" },
  { filter: { kind: "people" }, label: "roomList.filterPeople" },
  { filter: { kind: "favourites" }, label: "roomList.filterFavourites" },
  { filter: { kind: "invites" }, label: "roomList.filterInvites" }
];

function RoomListFilterTabs({
  activeFilter,
  onSelectFilter
}: {
  activeFilter: RoomListFilter;
  onSelectFilter: (filter: RoomListFilter) => void;
}) {
  return (
    <div className="room-list-filter-tabs" role="tablist" aria-label={t("workspace.filters")}>
      {ROOM_LIST_FILTERS.map(({ filter, label }) => {
        const isActive = filter.kind === activeFilter.kind;
        return (
          <button
            key={filter.kind}
            className={`room-list-filter-tab ${isActive ? "room-list-filter-tab-active" : ""}`}
            data-active={isActive || undefined}
            role="tab"
            aria-selected={isActive}
            type="button"
            onClick={() => onSelectFilter(filter)}
          >
            {t(label)}
          </button>
        );
      })}
    </div>
  );
}

function Sidebar({
  activeRoomId,
  activeView,
  snapshot,
  onCreateRoom,
  onNewDm,
  onOpenContextMenu,
  onOpenExplore,
  onOpenInvites,
  onOpenSpaceInfo,
  onSelectRoom,
  onSelectRoomListFilter
}: {
  activeRoomId: string | null;
  activeView: PrimaryView;
  snapshot: DesktopSnapshot;
  onCreateRoom: () => void;
  onNewDm: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenExplore: () => void;
  onOpenInvites: () => void;
  onOpenSpaceInfo: () => void;
  onSelectRoom: (roomId: string) => void;
  onSelectRoomListFilter: (filter: RoomListFilter) => void;
}) {
  const sections = roomListSections(
    snapshot.state.room_list,
    snapshot.state.spaces,
    snapshot.state.rooms,
    snapshot.state.invites
  );
  const threadAttention =
    snapshot.state.thread_attention.kind === "tracking"
      ? snapshot.state.thread_attention
      : null;
  return (
    <aside className="sidebar" aria-label={t("workspace.rooms")}>
      <div className="workspace-header">
        <div className="workspace-name" dir="auto">
          {snapshot.sidebar.space_rail.find((space) => space.is_active)?.display_name ??
            snapshot.sidebar.account_home.display_name}
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label={t("workspace.newDm")}
          onClick={onNewDm}
        >
          <MessageCircle size={ICON_SIZE.control} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("workspace.spaceInfoSettings")}
          onClick={onOpenSpaceInfo}
        >
          <Settings size={ICON_SIZE.control} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("action.createRoom")}
          onClick={onCreateRoom}
        >
          <Edit3 size={ICON_SIZE.control} />
        </button>
      </div>
      <RoomListFilterTabs
        activeFilter={snapshot.state.room_list.active_filter}
        onSelectFilter={onSelectRoomListFilter}
      />
      <div className="sidebar-scroll">
        <NavButton
          active={activeView === "timeline" && snapshot.sidebar.account_home.is_active}
          icon={<Home size={ICON_SIZE.control} />}
          label={t("workspace.home")}
        />
        <NavButton
          count={threadAttention?.notification_count ?? 0}
          icon={<MessageCircle size={ICON_SIZE.control} />}
          label={t("workspace.threads")}
          liveCount={threadAttention?.live_event_marker_count ?? 0}
          mentionCount={threadAttention?.highlight_count ?? 0}
        />
        <NavButton
          active={activeView === "explore"}
          icon={<Compass size={ICON_SIZE.control} />}
          label={t("workspace.explore")}
          onClick={onOpenExplore}
        />
        <NavButton
          active={activeView === "invites"}
          count={snapshot.state.invites.length}
          icon={<Bell size={ICON_SIZE.control} />}
          label={t("workspace.invites")}
          onClick={onOpenInvites}
        />
        <RoomSection
          activeRoomId={activeRoomId}
          id="favourites"
          kind="room"
          label={t("workspace.favourites")}
          rooms={sections.favourites}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
        />
        <RoomSection
          activeRoomId={activeRoomId}
          id="people"
          kind="dm"
          label={t("workspace.people")}
          rooms={sections.people}
          showWhenEmpty={true}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
        />
        <RoomSection
          activeRoomId={activeRoomId}
          id="rooms"
          kind="room"
          label={t("workspace.rooms")}
          rooms={sections.rooms}
          showWhenEmpty={true}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
        />
        <RoomSection
          activeRoomId={activeRoomId}
          id="low-priority"
          kind="room"
          label={t("workspace.lowPriority")}
          rooms={sections.lowPriority}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
        />
      </div>
    </aside>
  );
}

function RoomSection({
  activeRoomId,
  id,
  kind,
  label,
  rooms,
  showWhenEmpty = false,
  onOpenContextMenu,
  onSelectRoom
}: {
  activeRoomId: string | null;
  id: string;
  kind: "room" | "dm";
  label: string;
  rooms: RoomListItem[];
  showWhenEmpty?: boolean;
  onOpenContextMenu: OpenContextMenu;
  onSelectRoom: (roomId: string) => void;
}) {
  if (!showWhenEmpty && rooms.length === 0) {
    return null;
  }

  return (
    <section className="room-section" data-room-section={id} aria-label={label}>
      <SectionTitle count={rooms.length} label={label} />
      {rooms.map((room) => (
        <RoomButton
          activeRoomId={activeRoomId}
          kind={kind}
          key={room.room_id}
          room={room}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
        />
      ))}
    </section>
  );
}

function NavButton({
  active = false,
  count = 0,
  icon,
  label,
  liveCount = 0,
  mentionCount = 0,
  onClick
}: {
  active?: boolean;
  count?: number;
  icon: ReactNode;
  label: string;
  liveCount?: number;
  mentionCount?: number;
  onClick?: () => void;
}) {
  return (
    <button
      className={`nav-item ${active ? "is-active" : ""}`}
      data-count={count || undefined}
      data-live-count={liveCount || undefined}
      data-mention-count={mentionCount || undefined}
      type="button"
      aria-label={label}
      onClick={onClick}
    >
      {icon}
      <span className="nav-label">{label}</span>
    </button>
  );
}

function SectionTitle({ count, label }: { count: number; label: string }) {
  return (
    <div className="section-title">
      <span className="section-title-label">{label}</span>
      <span className="section-title-meta">
        <span className="section-count">{count}</span>
        <Plus size={ICON_SIZE.compact} />
      </span>
    </div>
  );
}

function RoomButton({
  activeRoomId,
  kind,
  room,
  onOpenContextMenu,
  onSelectRoom
}: {
  activeRoomId: string | null;
  kind: "room" | "dm";
  room: RoomListItem;
  onOpenContextMenu: OpenContextMenu;
  onSelectRoom: (roomId: string) => void;
}) {
  const mentionCount = room.highlight_count || 0;
  return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      data-mention-count={mentionCount || undefined}
      data-room-kind={kind}
      data-testid="room-item"
      type="button"
      onClick={() => onSelectRoom(room.room_id)}
      onContextMenu={(event) =>
        onOpenContextMenu(
          event,
          { kind: "room", roomId: room.room_id },
          contextMenuItems({ kind: "room", tags: room.tags ?? EMPTY_ROOM_TAGS })
        )
      }
    >
      <EntityAvatar
        avatar={room.avatar}
        className={`room-avatar ${kind === "dm" ? "is-user" : "is-room"}`}
        fallback={initials(room.display_name)}
      />
      <span className="room-name" dir="auto">{room.display_name}</span>
      <span className="room-trailing">
        {mentionCount > 0 ? <span className="room-mention-dot" aria-hidden="true" /> : null}
        <span className="room-count">{room.unread_count || ""}</span>
      </span>
    </button>
  );
}

function EntityAvatar({
  avatar,
  className,
  fallback
}: {
  avatar: RoomListItem["avatar"];
  className: string;
  fallback: string;
}) {
  const sourceUrl = avatar?.thumbnail.kind === "ready" ? avatar.thumbnail.source_url : null;
  return (
    <span className={className} aria-hidden="true">
      {sourceUrl ? <img src={sourceUrl} /> : <span dir="auto">{fallback}</span>}
    </span>
  );
}

function ActivityPane({
  activity,
  onClose,
  onLoadMore,
  onMarkRead,
  onOpenRow,
  onSetTab
}: {
  activity: ActivityState;
  onClose: () => void;
  onLoadMore: (tab: ActivityTab, cursor: string | null) => void;
  onMarkRead: (target: ActivityMarkReadTarget) => void;
  onOpenRow: (row: ActivityRow) => void;
  onSetTab: (tab: ActivityTab) => void;
}) {
  const activeTab =
    activity.kind === "open" ? activity.active_tab : activity.kind === "opening" ? activity.tab : "recent";
  const stream = activity.kind === "open" ? activityStream(activity, activeTab) : null;
  const rows = stream?.rows ?? [];
  const markReadState = activity.kind === "open" ? activity.mark_read : { kind: "idle" as const };
  const markAllPending =
    markReadState.kind === "pending" && markReadState.target.kind === "all";
  const markRoomPending = (row: ActivityRow) =>
    markReadState.kind === "pending" &&
    markReadState.target.kind === "room" &&
    markReadState.target.room_id === row.room_id;

  return (
    <main className="main-pane activity-pane" aria-labelledby="activity-title">
      <header className="channel-header">
        <div className="channel-title">
          <Clock3 size={ICON_SIZE.large} />
          <h1 id="activity-title">{t("workspace.activity")}</h1>
        </div>
        <div className="activity-actions">
          {activity.kind === "open" && activeTab === "unread" && rows.length > 0 ? (
            <button
              className="dialog-button secondary"
              type="button"
              disabled={markAllPending}
              onClick={() => onMarkRead({ kind: "all" })}
            >
              <Check size={ICON_SIZE.small} />
              <span>{t("activity.markAllRead")}</span>
            </button>
          ) : null}
          <button
            className="icon-button"
            type="button"
            aria-label={t("action.close", { title: t("workspace.activity") })}
            onClick={onClose}
          >
            <X size={ICON_SIZE.control} />
          </button>
        </div>
      </header>
      <div className="tabs" role="tablist" aria-label={t("activity.tabs")}>
        {(["recent", "unread"] as ActivityTab[]).map((tab) => (
          <button
            className={`tab ${activeTab === tab ? "is-active" : ""}`}
            role="tab"
            aria-selected={activeTab === tab}
            type="button"
            key={tab}
            disabled={activity.kind !== "open"}
            onClick={() => onSetTab(tab)}
          >
            {activityTabLabel(tab)}
          </button>
        ))}
      </div>
      {markReadState.kind === "failed" ? (
        <p className="activity-status" role="alert">
          {t("activity.markReadFailed")}
        </p>
      ) : null}
      <section className="activity-scroll" aria-label={activityTabLabel(activeTab)}>
        {activity.kind === "opening" ? (
          <div className="activity-empty">
            <Clock3 size={ICON_SIZE.emptyState} />
            <span>{t("activity.loading")}</span>
          </div>
        ) : rows.length === 0 ? (
          <div className="activity-empty">
            <Clock3 size={ICON_SIZE.emptyState} />
            <span>
              {activeTab === "recent" ? t("activity.noRecent") : t("activity.noUnread")}
            </span>
          </div>
        ) : (
          <ol className="activity-list">
            {rows.map((row) => (
              <li
                className={`activity-row ${row.unread ? "is-unread" : ""} ${
                  row.highlight ? "is-highlight" : ""
                }`}
                data-event-id={row.event_id}
                data-room-id={row.room_id}
                key={`${row.room_id}:${row.event_id}`}
              >
                <button
                  className="activity-row-open"
                  type="button"
                  aria-label={t("activity.openItem", { room: row.room_label })}
                  onClick={() => onOpenRow(row)}
                >
                  <span className="activity-row-topline">
                    <strong dir="auto">{row.room_label}</strong>
                    <time dateTime={new Date(row.timestamp_ms).toISOString()}>
                      {activityTimestamp(row.timestamp_ms)}
                    </time>
                  </span>
                  <span className="activity-row-meta">
                    <span dir="auto">
                      {row.sender_label ?? t("timeline.replyQuoteUnknownSender")}
                    </span>
                    {row.unread ? <span>{t("activity.unreadBadge")}</span> : null}
                    {row.highlight ? <span>{t("activity.highlightBadge")}</span> : null}
                  </span>
                  <span className="activity-row-preview" dir="auto">
                    {row.preview ?? t("activity.noPreview")}
                  </span>
                </button>
                {activeTab === "unread" ? (
                  <button
                    className="activity-row-action"
                    type="button"
                    aria-label={t("activity.markRoomRead")}
                    disabled={markRoomPending(row)}
                    onClick={() =>
                      onMarkRead({
                        kind: "room",
                        room_id: row.room_id,
                        up_to_event_id: row.event_id
                      })
                    }
                  >
                    <Check size={ICON_SIZE.small} />
                  </button>
                ) : null}
              </li>
            ))}
          </ol>
        )}
      </section>
      {stream?.next_batch ? (
        <div className="activity-load-more">
          <button
            className="load-more-button"
            type="button"
            onClick={() => onLoadMore(activeTab, stream.next_batch)}
          >
            {t("activity.loadMore")}
          </button>
        </div>
      ) : null}
    </main>
  );
}

function activityStream(activity: Extract<ActivityState, { kind: "open" }>, tab: ActivityTab): ActivityStream {
  return tab === "recent" ? activity.recent : activity.unread;
}

function activityTabLabel(tab: ActivityTab): string {
  return tab === "recent" ? t("activity.recent") : t("activity.unread");
}

function activityTimestamp(timestampMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(timestampMs));
}

function ExplorePane({
  isBusy,
  queryDraft,
  snapshot,
  onJoinRoom,
  onQueryChange,
  onSearch
}: {
  isBusy: boolean;
  queryDraft: string;
  snapshot: DesktopSnapshot;
  onJoinRoom: (room: DirectoryRoomSummary) => void;
  onQueryChange: (value: string) => void;
  onSearch: () => void;
}) {
  const queryState = snapshot.state.directory.query;
  const joinState = snapshot.state.directory.join;
  const rooms = queryState.kind === "results" ? queryState.rooms : [];
  const searchDisabled = isBusy || queryState.kind === "querying";

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSearch();
  }

  return (
    <main className="main-pane explore-pane" aria-labelledby="explore-title">
      <header className="channel-header">
        <div className="channel-title">
          <Compass size={ICON_SIZE.large} />
          <h1 id="explore-title">{t("workspace.explore")}</h1>
        </div>
      </header>
      <form className="directory-search" onSubmit={submitSearch}>
        <label className="directory-search-field">
          <span>{t("directory.searchPublicRooms")}</span>
          <input
            type="search"
            value={queryDraft}
            aria-label={t("directory.searchPublicRooms")}
            placeholder={t("directory.searchPlaceholder")}
            onChange={(event) => onQueryChange(event.currentTarget.value)}
          />
        </label>
        <button
          className="dialog-button is-primary"
          type="submit"
          aria-label={t("directory.searchPublicRooms")}
          disabled={searchDisabled}
        >
          <Search size={ICON_SIZE.small} />
          <span>
            {queryState.kind === "querying"
              ? t("directory.searching")
              : t("directory.search")}
          </span>
        </button>
      </form>
      {queryState.kind === "failed" ? (
        <div className="directory-status" role="status">
          {t("directory.searchFailed", {
            reason: operationFailureLabel(queryState.failureKind)
          })}
        </div>
      ) : null}
      <section className="directory-results" aria-label={t("directory.results")}>
        {queryState.kind === "querying" ? (
          <div className="empty-results" role="status">
            {t("directory.searching")}
          </div>
        ) : rooms.length ? (
          rooms.map((room) => {
            const alias = room.canonical_alias?.trim() || null;
            const joiningThisRoom =
              joinState.kind === "joining" && joinState.alias === alias;
            const joinFailed =
              joinState.kind === "failed" && joinState.alias === alias ? joinState : null;
            const canJoin = Boolean(alias) && !joiningThisRoom && !isBusy;
            return (
              <article className="directory-result" key={room.room_id}>
                <div className="directory-result-avatar" aria-hidden="true">
                  <span dir="auto">{initials(room.name)}</span>
                </div>
                <div className="directory-result-main">
                  <h2 dir="auto">{room.name}</h2>
                  <p dir="auto">
                    {room.topic?.trim() || alias || t("directory.noAlias")}
                  </p>
                  <div className="directory-result-meta">
                    <span>
                      {t("directory.memberCount", {
                        count: new Intl.NumberFormat().format(room.joined_members)
                      })}
                    </span>
                    {room.world_readable ? <span>{t("directory.worldReadable")}</span> : null}
                    {room.guest_can_join ? <span>{t("directory.guestCanJoin")}</span> : null}
                  </div>
                  {joinFailed ? (
                    <div className="directory-status" role="status">
                      {t("directory.joinFailed", {
                        reason: operationFailureLabel(joinFailed.failureKind)
                      })}
                    </div>
                  ) : null}
                </div>
                <button
                  className="dialog-button is-primary directory-join-button"
                  type="button"
                  aria-label={t("directory.joinRoom", { name: room.name })}
                  disabled={!canJoin}
                  onClick={() => onJoinRoom(room)}
                >
                  {joiningThisRoom ? t("directory.joining") : t("directory.join")}
                </button>
              </article>
            );
          })
        ) : (
          <div className="empty-results" role="status">
            {t("directory.noResults")}
          </div>
        )}
      </section>
    </main>
  );
}

function InvitesPane({
  isBusy,
  snapshot,
  onAcceptInvite,
  onDeclineInvite,
  onNewDm
}: {
  isBusy: boolean;
  snapshot: DesktopSnapshot;
  onAcceptInvite: (roomId: string) => void;
  onDeclineInvite: (roomId: string) => void;
  onNewDm: () => void;
}) {
  const invites = snapshot.state.invites;
  const [selectedInviteId, setSelectedInviteId] = useState<string | null>(null);
  const selectedInvite =
    invites.find((invite) => invite.room_id === selectedInviteId) ?? invites[0] ?? null;

  return (
    <main className="main-pane invites-pane" aria-labelledby="invites-title">
      <header className="channel-header">
        <div className="channel-title">
          <Bell size={ICON_SIZE.large} />
          <h1 id="invites-title">{t("workspace.invites")}</h1>
        </div>
        <div className="channel-actions">
          <button
            className="member-pill"
            type="button"
            aria-label={t("workspace.newDm")}
            onClick={onNewDm}
          >
            <MessageCircle size={ICON_SIZE.small} />
            <span>{t("workspace.newDm")}</span>
          </button>
        </div>
      </header>
      <nav className="tabs" aria-label={t("invite.tabs")}>
        <button className="tab is-active" type="button">
          {t("invite.pendingInvites")}
        </button>
      </nav>
      <section className="invites-layout" aria-label={t("invite.pendingInvites")}>
        <div className="invite-list">
          {invites.length ? (
            invites.map((invite) => (
              <button
                className={`invite-row ${invite.room_id === selectedInvite?.room_id ? "is-active" : ""}`}
                key={invite.room_id}
                type="button"
                aria-label={invite.display_name}
                onClick={() => setSelectedInviteId(invite.room_id)}
              >
                <EntityAvatar
                  avatar={invite.avatar}
                  className={`invite-row-icon ${invite.is_dm ? "is-user" : "is-room"}`}
                  fallback={initials(invite.display_name)}
                />
                <span className="invite-row-main">
                  <strong dir="auto">{invite.display_name}</strong>
                  <small dir="auto">
                    {invite.inviter_display_name ?? t("invite.unknownInviter")}
                  </small>
                </span>
              </button>
            ))
          ) : (
            <div className="empty-results" role="status">
              {t("invite.noPending")}
            </div>
          )}
        </div>
        <section className="invite-preview" aria-label={t("invite.preview")}>
          {selectedInvite ? (
            <>
              <div className="invite-preview-heading">
                <EntityAvatar
                  avatar={selectedInvite.avatar}
                  className={`invite-preview-icon ${selectedInvite.is_dm ? "is-user" : "is-room"}`}
                  fallback={initials(selectedInvite.display_name)}
                />
                <div>
                  <h2 dir="auto">{selectedInvite.display_name}</h2>
                  <p dir="auto">
                    {selectedInvite.inviter_display_name
                      ? t("invite.fromInviter", {
                          inviter: selectedInvite.inviter_display_name
                        })
                      : t("invite.unknownInviter")}
                  </p>
                </div>
              </div>
              <div className="settings-summary-grid" aria-label={t("invite.summary")}>
                <SummaryTile
                  label={t("room.type")}
                  value={
                    selectedInvite.is_dm
                      ? t("room.directMessage")
                      : t("search.scopeRoom")
                  }
                />
                <SummaryTile
                  label={t("invite.topic")}
                  value={selectedInvite.topic ?? t("invite.noTopic")}
                />
              </div>
              <div className="invite-actions">
                <button
                  className="dialog-button"
                  type="button"
                  aria-label={t("invite.decline")}
                  disabled={isBusy}
                  onClick={() => onDeclineInvite(selectedInvite.room_id)}
                >
                  <X size={ICON_SIZE.small} />
                  <span>{t("invite.decline")}</span>
                </button>
                <button
                  className="dialog-button is-primary"
                  type="button"
                  aria-label={t("invite.accept")}
                  disabled={isBusy}
                  onClick={() => onAcceptInvite(selectedInvite.room_id)}
                >
                  <Check size={ICON_SIZE.small} />
                  <span>{t("invite.accept")}</span>
                </button>
              </div>
            </>
          ) : (
            <div className="invite-empty-preview">
              <Bell size={ICON_SIZE.emptyState} />
              <span>{t("invite.noPending")}</span>
            </div>
          )}
        </section>
      </section>
    </main>
  );
}

function SummaryTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-summary-tile">
      <span>{label}</span>
      <strong dir="auto">{value}</strong>
    </div>
  );
}

function TimelinePane({
  activeRoomName,
  composerDraft,
  composerMode,
  mentionIntent,
  resolveComposerKeyAction,
  searchQuery,
  searchResults,
  showSearchResults,
  snapshot,
  timelineTransport,
  onCancelReply,
  onCancelScheduledSend,
  onAttachFiles,
  onClearUploadStaging,
  onUpdateStagedUploadCaption,
  onUpdateStagedUploadCompression,
  onComposerDraftChange,
  onMentionIntentChange,
  onEditMessage,
  onOpenContextMenu,
  onOpenThread,
  onPaginateBackwards,
  onRedactMessage,
  onReply,
  onRescheduleScheduledSend,
  onResultSelect,
  onScheduleSend,
  onSendText,
  onSetLocalUserAlias,
  onUnpinPinnedEvent,
  onToggleThread,
  onOpenRoomInfo
}: {
  activeRoomName: string;
  composerDraft: string;
  composerMode: ComposerModeProp;
  mentionIntent: MentionIntent;
  resolveComposerKeyAction: ResolveComposerKeyAction;
  searchQuery: string;
  searchResults: SearchResult[];
  showSearchResults: boolean;
  snapshot: DesktopSnapshot;
  timelineTransport: TimelineTransport | null;
  onCancelReply: () => void;
  onCancelScheduledSend: (scheduledId: string) => void;
  onAttachFiles: (files: File[]) => void | Promise<void>;
  onClearUploadStaging: () => void | Promise<void>;
  onUpdateStagedUploadCaption: (stagedId: string, caption: string) => void | Promise<void>;
  onUpdateStagedUploadCompression: (
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ) => void | Promise<void>;
  onComposerDraftChange: (value: string) => void;
  onMentionIntentChange: (intent: MentionIntent) => void;
  onEditMessage: (message: TimelineMessage) => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onPaginateBackwards: (roomId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onRescheduleScheduledSend: (scheduledId: string, sendAtMs: number) => void;
  onResultSelect: (roomId: string, eventId: string) => void;
  onScheduleSend: (sendAtMs: number) => void;
  onSendText: () => void;
  onSetLocalUserAlias: (userId: string, alias: string | null) => void;
  onUnpinPinnedEvent: (roomId: string, eventId: string) => void;
  onToggleThread: () => void;
  onOpenRoomInfo: () => void;
}) {
  const timelineRoomId = snapshot.state.timeline.room_id;
  const currentUserId = snapshot.state.session.user_id ?? null;
  const activeRoom = timelineRoomId
    ? snapshot.state.rooms.find((room) => room.room_id === timelineRoomId) ?? null
    : null;
  const pinnedEvents = pinnedEventsForRoom(snapshot, timelineRoomId);
  const pinnedEventIds = pinnedEvents.map((event) => event.event_id);
  const stagedUploads = snapshot.state.timeline.staged_uploads ?? [];
  const mediaGallery = snapshot.state.timeline.media_gallery ?? [];
  const [galleryOpen, setGalleryOpen] = useState(false);
  const [viewerIndex, setViewerIndex] = useState<number | null>(null);

  return (
    <main className="main-pane" aria-label={t("timeline.conversation")}>
      <header className="channel-header">
        <div className="channel-title">
          <EntityAvatar
            avatar={activeRoom?.avatar ?? null}
            className="channel-avatar is-room"
            fallback={initials(activeRoomName)}
          />
          <span>{activeRoomName}</span>
        </div>
        <div className="channel-actions">
          <button className="member-pill" type="button" aria-label={t("room.members")}>
            <Users size={ICON_SIZE.small} />
            <span>8</span>
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.open")}
            disabled={mediaGallery.length === 0}
            onClick={() => setGalleryOpen((open) => !open)}
          >
            <ImageIcon size={ICON_SIZE.panel} />
          </button>
          <button className="icon-button" type="button" aria-label={t("room.threadToggle")} onClick={onToggleThread}>
            {snapshot.state.thread.kind !== "closed" ? <PanelRightClose size={ICON_SIZE.panel} /> : <PanelRightOpen size={ICON_SIZE.panel} />}
          </button>
          <button className="icon-button" type="button" aria-label={t("room.roomInfo")} onClick={onOpenRoomInfo}>
            <MoreVertical size={ICON_SIZE.panel} />
          </button>
        </div>
      </header>
      <nav className="tabs" aria-label={t("room.tabs")}>
        <button className="tab is-active" type="button">
          {t("timeline.messagesTab")}
        </button>
      </nav>
      {galleryOpen ? (
        <RoomMediaGallery
          items={mediaGallery}
          onOpenItem={(index) => setViewerIndex(index)}
        />
      ) : null}
      <section className="timeline-scroll">
        {timelineRoomId && pinnedEvents.length > 0 ? (
          <PinnedEventsList
            roomId={timelineRoomId}
            pinnedEvents={pinnedEvents}
            onUnpin={onUnpinPinnedEvent}
          />
        ) : null}
        {showSearchResults ? (
          <SearchResults
            query={searchQuery}
            results={searchResults}
            rooms={snapshot.state.rooms}
            onResultSelect={onResultSelect}
          />
        ) : null}
        <div className="message-list">
          <div className="timeline-load-more">
            <button
              className="load-more-button"
              type="button"
              disabled={!timelineRoomId || snapshot.state.timeline.is_paginating_backwards}
              onClick={() => {
                if (timelineRoomId) {
                  onPaginateBackwards(timelineRoomId);
                }
              }}
            >
              <Clock3 size={ICON_SIZE.compact} />
              <span>
                {snapshot.state.timeline.is_paginating_backwards
                  ? t("timeline.loading")
                  : t("timeline.olderMessages")}
              </span>
            </button>
          </div>
          {timelineTransport && timelineRoomId && currentUserId ? (
            // Production path: render from the event-driven timeline store
            // (CoreEvent diffs), never from AppState timeline fields.
            <TimelineView
              key={timelineRoomId}
              roomId={timelineRoomId}
              timelineKey={roomTimelineKey(currentUserId, timelineRoomId)}
              transport={timelineTransport}
              onReply={onReply}
              onOpenThread={onOpenThread}
              resolveComposerKeyAction={resolveComposerKeyAction}
              liveSignals={snapshot.state.live_signals}
              profileUsers={snapshot.state.profile.users}
              pinnedEventIds={pinnedEventIds}
              forwardDestinations={forwardDestinationsFromSnapshot(snapshot)}
              onSetLocalUserAlias={onSetLocalUserAlias}
              codeBlockWrap={snapshot.state.settings.values.display.code_block_wrap}
              searchQuery={searchQuery}
            />
          ) : (
            // Browser fixture preview only (no Tauri runtime).
            snapshot.timeline.map((message) => (
              <MessageArticle
                key={message.event_id}
                message={message}
                query={searchQuery}
                currentUserId={currentUserId}
                onOpenContextMenu={onOpenContextMenu}
                onEditMessage={onEditMessage}
                onOpenThread={onOpenThread}
                onRedactMessage={onRedactMessage}
                profileUsers={snapshot.state.profile.users}
              />
            ))
          )}
        </div>
      </section>
      <ScheduledMessagesList
        capability={snapshot.state.timeline.scheduled_send_capability}
        items={snapshot.state.timeline.scheduled_sends}
        onCancel={onCancelScheduledSend}
        onReschedule={onRescheduleScheduledSend}
      />
      {stagedUploads.length > 0 ? (
        <UploadStagingDialog
          items={stagedUploads}
          onClear={onClearUploadStaging}
          onUpdateCaption={onUpdateStagedUploadCaption}
          onUpdateCompression={onUpdateStagedUploadCompression}
        />
      ) : null}
      <Composer
        composerMode={composerMode}
        hasStagedUploads={stagedUploads.length > 0}
        isSending={Boolean(snapshot.state.timeline.composer.pending_transaction_id)}
        mentionCandidates={mentionCandidatesFromSnapshot(snapshot)}
        mentionIntent={mentionIntent}
        resolveComposerKeyAction={resolveComposerKeyAction}
        roomName={activeRoomName}
        value={composerDraft}
        onCancelReply={onCancelReply}
        onAttachFiles={onAttachFiles}
        onMentionIntentChange={onMentionIntentChange}
        onScheduleSend={onScheduleSend}
        onSend={onSendText}
        onValueChange={onComposerDraftChange}
      />
      {viewerIndex !== null && mediaGallery[viewerIndex] ? (
        <MediaViewer
          index={viewerIndex}
          items={mediaGallery}
          onClose={() => setViewerIndex(null)}
          onSelectIndex={setViewerIndex}
        />
      ) : null}
    </main>
  );
}

function UploadStagingDialog({
  items,
  onClear,
  onUpdateCaption,
  onUpdateCompression
}: {
  items: StagedUploadItem[];
  onClear: () => void | Promise<void>;
  onUpdateCaption: (stagedId: string, caption: string) => void | Promise<void>;
  onUpdateCompression: (
    stagedId: string,
    compressionChoice: StagedUploadCompressionChoice
  ) => void | Promise<void>;
}) {
  return (
    <section
      className="upload-staging-dialog"
      role="dialog"
      aria-label={t("upload.dialogTitle")}
    >
      <div className="upload-staging-header">
        <h2>{t("upload.dialogTitle")}</h2>
        <button className="icon-button" type="button" aria-label={t("upload.clear")} onClick={onClear}>
          <X size={ICON_SIZE.small} />
        </button>
      </div>
      <div className="upload-staging-list">
        {items.map((item) => (
          <article className="upload-staging-item" key={item.staged_id}>
            <div className="upload-staging-file">
              {item.kind.kind === "image" ? (
                <ImageIcon size={ICON_SIZE.control} aria-hidden="true" />
              ) : (
                <FileText size={ICON_SIZE.control} aria-hidden="true" />
              )}
              <span className="upload-staging-name" dir="auto">
                {item.filename || t("composer.attachmentFallback")}
              </span>
              <span className="upload-staging-meta">
                {formatUploadBytes(item.byte_count)}
              </span>
            </div>
            <label className="upload-staging-caption">
              <span>{t("upload.captionForFile", { filename: item.filename })}</span>
              <input
                value={captionBody(item)}
                aria-label={t("upload.captionForFile", { filename: item.filename })}
                onChange={(event) => {
                  void onUpdateCaption(item.staged_id, event.currentTarget.value);
                }}
              />
            </label>
            {item.kind.kind === "image" ? (
              <div className="upload-staging-choice" role="group" aria-label={t("upload.sizeChoice")}>
                <button
                  className="dialog-button"
                  type="button"
                  aria-pressed={item.compression_choice.kind === "original"}
                  onClick={() => {
                    void onUpdateCompression(item.staged_id, { kind: "original" });
                  }}
                >
                  {t("upload.original")}
                </button>
                <button
                  className="dialog-button"
                  type="button"
                  aria-pressed={item.compression_choice.kind === "compressed"}
                  onClick={() => {
                    void onUpdateCompression(item.staged_id, {
                      kind: "compressed",
                      mode: "always"
                    });
                  }}
                >
                  {t("upload.compressed")}
                </button>
              </div>
            ) : null}
          </article>
        ))}
      </div>
    </section>
  );
}

function RoomMediaGallery({
  items,
  onOpenItem
}: {
  items: TimelineMediaGalleryItem[];
  onOpenItem: (index: number) => void;
}) {
  return (
    <section className="room-media-gallery" role="region" aria-label={t("mediaGallery.region")}>
      {items.map((item, index) => {
        const label = mediaGalleryItemLabel(item);
        return (
          <button
            className="room-media-gallery-item"
            key={item.event_id}
            type="button"
            aria-label={t("mediaGallery.openItem", { filename: label })}
            onClick={() => onOpenItem(index)}
          >
            {item.media.kind === "Image" ? (
              <ImageIcon size={ICON_SIZE.control} aria-hidden="true" />
            ) : (
              <FileText size={ICON_SIZE.control} aria-hidden="true" />
            )}
            <span className="room-media-gallery-name" dir="auto">
              {label}
            </span>
            <span className="room-media-gallery-meta">
              {item.media.size !== null ? formatUploadBytes(item.media.size) : item.media.kind}
              {item.media.source.encrypted ? ` - ${t("mediaGallery.encrypted")}` : ""}
            </span>
          </button>
        );
      })}
    </section>
  );
}

function MediaViewer({
  index,
  items,
  onClose,
  onSelectIndex
}: {
  index: number;
  items: TimelineMediaGalleryItem[];
  onClose: () => void;
  onSelectIndex: (index: number) => void;
}) {
  const [zoom, setZoom] = useState(1);
  const item = items[index];
  const previousIndex = (index + items.length - 1) % items.length;
  const nextIndex = (index + 1) % items.length;
  const label = mediaGalleryItemLabel(item);

  return (
    <div className="media-viewer-backdrop" role="dialog" aria-label={t("mediaGallery.viewerTitle")}>
      <div className="media-viewer">
        <header className="media-viewer-header">
          <div>
            <h2 dir="auto">{label}</h2>
            <p>
              {item.media.mimetype ?? item.media.kind}
              {item.media.size !== null ? ` - ${formatUploadBytes(item.media.size)}` : ""}
            </p>
          </div>
          <button className="icon-button" type="button" aria-label={t("mediaGallery.close")} onClick={onClose}>
            <X size={ICON_SIZE.small} />
          </button>
        </header>
        <div className="media-viewer-stage">
          {item.media.kind === "Image" ? (
            <div className="media-viewer-image-placeholder" style={{ transform: `scale(${zoom})` }}>
              <ImageIcon size={ICON_SIZE.emptyState} aria-hidden="true" />
            </div>
          ) : (
            <div className="media-viewer-file-placeholder">
              <FileText size={ICON_SIZE.emptyState} aria-hidden="true" />
            </div>
          )}
        </div>
        <footer className="media-viewer-actions">
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.previous")}
            onClick={() => {
              setZoom(1);
              onSelectIndex(previousIndex);
            }}
          >
            <ChevronLeft size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.zoomOut")}
            onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}
          >
            <ZoomOut size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.zoomIn")}
            onClick={() => setZoom((value) => Math.min(3, value + 0.25))}
          >
            <ZoomIn size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("mediaGallery.next")}
            onClick={() => {
              setZoom(1);
              onSelectIndex(nextIndex);
            }}
          >
            <ChevronRight size={ICON_SIZE.control} />
          </button>
        </footer>
      </div>
    </div>
  );
}

function ScheduledMessagesList({
  capability,
  items,
  onCancel,
  onReschedule
}: {
  capability: ScheduledSendCapability;
  items: ScheduledSendItem[];
  onCancel: (scheduledId: string) => void;
  onReschedule: (scheduledId: string, sendAtMs: number) => void;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  if (items.length === 0) {
    return null;
  }

  function openEdit(item: ScheduledSendItem) {
    setEditingId(item.scheduled_id);
    setEditValue(datetimeLocalValueFromTimestamp(item.send_at_ms));
  }

  function submitEdit(event: FormEvent<HTMLFormElement>, item: ScheduledSendItem) {
    event.preventDefault();
    const sendAtMs = scheduledSendTimestampFromInput(editValue);
    if (sendAtMs === null) {
      return;
    }
    onReschedule(item.scheduled_id, sendAtMs);
    setEditingId(null);
  }

  return (
    <section className="scheduled-messages" aria-label={t("scheduled.title")}>
      <div className="scheduled-messages-heading">
        <span>
          <Clock3 size={ICON_SIZE.compact} aria-hidden="true" />
          <strong>{t("scheduled.title")}</strong>
        </span>
        <span className="scheduled-messages-capability">
          {scheduledSendCapabilityLabel(capability)}
        </span>
      </div>
      {capability === "localFallback" ? (
        <p className="scheduled-messages-note">{t("scheduled.localFallbackNotice")}</p>
      ) : null}
      <ul className="scheduled-message-list">
        {items.map((item) => {
          const isEditing = editingId === item.scheduled_id;
          return (
            <li className="scheduled-message-item" key={item.scheduled_id}>
              <div className="scheduled-message-main">
                <span className="scheduled-message-time">
                  {formatScheduledSendTime(item.send_at_ms)}
                </span>
                <span className="scheduled-message-body" dir="auto">
                  {item.body}
                </span>
              </div>
              {isEditing ? (
                <form
                  className="scheduled-message-edit"
                  onSubmit={(event) => submitEdit(event, item)}
                >
                  <label className="scheduled-send-field">
                    <span>{t("scheduled.timeInput")}</span>
                    <input
                      aria-label={t("scheduled.timeInput")}
                      type="datetime-local"
                      value={editValue}
                      onChange={(event) => setEditValue(event.currentTarget.value)}
                    />
                  </label>
                  <div className="scheduled-message-actions">
                    <button
                      className="timeline-send-bar-action"
                      type="button"
                      onClick={() => setEditingId(null)}
                    >
                      {t("action.cancel")}
                    </button>
                    <button
                      className="timeline-send-bar-action"
                      type="submit"
                      disabled={scheduledSendTimestampFromInput(editValue) === null}
                    >
                      {t("scheduled.save")}
                    </button>
                  </div>
                </form>
              ) : (
                <div className="scheduled-message-actions">
                  <button
                    className="timeline-send-bar-action"
                    type="button"
                    aria-label={t("scheduled.edit")}
                    onClick={() => openEdit(item)}
                  >
                    <Edit3 size={ICON_SIZE.micro} aria-hidden="true" />
                    <span>{t("context.editMessage")}</span>
                  </button>
                  <button
                    className="timeline-send-bar-action danger"
                    type="button"
                    aria-label={t("scheduled.cancel")}
                    onClick={() => onCancel(item.scheduled_id)}
                  >
                    <X size={ICON_SIZE.micro} aria-hidden="true" />
                    <span>{t("action.cancel")}</span>
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function PinnedEventsList({
  roomId,
  pinnedEvents,
  onUnpin
}: {
  roomId: string;
  pinnedEvents: DesktopSnapshot["state"]["room_interactions"][string]["pinned_events"];
  onUnpin: (roomId: string, eventId: string) => void;
}) {
  return (
    <section className="pinned-events" aria-label={t("timeline.pinnedMessages")}>
      <div className="pinned-events-heading">
        <Pin size={ICON_SIZE.compact} aria-hidden="true" />
        <span>{t("timeline.pinnedMessages")}</span>
      </div>
      <div className="pinned-events-list">
        {pinnedEvents.map((event) => (
          <div className="pinned-event" key={event.event_id}>
            <div className="pinned-event-main">
              {event.sender ? (
                <span className="pinned-event-sender" dir="auto">
                  {event.sender}
                </span>
              ) : null}
              <span className="pinned-event-body" dir="auto">
                {event.redacted
                  ? t("timeline.redactedMessage")
                  : event.body_preview ?? t("timeline.pinnedMessage")}
              </span>
            </div>
            <button
              className="pinned-event-action"
              type="button"
              aria-label={t("timeline.unpinMessage")}
              onClick={() => onUnpin(roomId, event.event_id)}
            >
              <PinOff size={ICON_SIZE.micro} aria-hidden="true" />
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

function pinnedEventsForRoom(
  snapshot: DesktopSnapshot,
  roomId: string | null | undefined
): DesktopSnapshot["state"]["room_interactions"][string]["pinned_events"] {
  return roomId ? snapshot.state.room_interactions[roomId]?.pinned_events ?? [] : [];
}

function forwardDestinationsFromSnapshot(snapshot: DesktopSnapshot): TimelineForwardDestination[] {
  return snapshot.state.rooms.map((room) => ({
    room_id: room.room_id,
    display_name: room.display_label
  }));
}

function mentionCandidatesFromSnapshot(snapshot: DesktopSnapshot): MentionCandidate[] {
  return Object.values(snapshot.state.profile.users)
    .map((profile) => {
      const label = mentionLabel(profile);
      const target: MentionTarget = {
        kind: "user",
        user_id: profile.user_id,
        display_label: label
      };
      return {
        key: profile.user_id,
        label,
        searchText: (
          profile.mention_search_terms.length
            ? profile.mention_search_terms.join(" ")
            : `${label} ${profile.user_id}`
        ).toLowerCase(),
        target
      };
    })
    .sort(
      (a, b) =>
        a.label.localeCompare(b.label, undefined, { sensitivity: "base" }) ||
        a.key.localeCompare(b.key)
    );
}

function mentionLabel(profile: UserProfile): string {
  return profile.display_label.trim() || profile.user_id;
}

function activeMentionQuery(value: string): { start: number; end: number; query: string } | null {
  const match = /(^|\s)@([^\s@]*)$/u.exec(value);
  if (!match) {
    return null;
  }
  const query = match[2] ?? "";
  return {
    start: value.length - query.length - 1,
    end: value.length,
    query
  };
}

function appendMentionTarget(intent: MentionIntent, target: MentionTarget): MentionIntent {
  const targetKey = mentionTargetKey(target);
  if (intent.targets.some((candidate) => mentionTargetKey(candidate) === targetKey)) {
    return intent;
  }
  return { targets: [...intent.targets, target] };
}

function mentionTargetKey(target: MentionTarget): string {
  switch (target.kind) {
    case "user":
      return `user:${target.user_id}`;
    case "room":
      return `room:${target.room_id}`;
    case "roomMention":
      return "roomMention";
  }
}

function mentionDraftToken(target: MentionTarget): string {
  return `@${target.display_label}`;
}

function mentionPillLabel(target: MentionTarget): string {
  return mentionDraftToken(target);
}

function pruneMentionIntentForDraft(intent: MentionIntent, draft: string): MentionIntent {
  const targets = intent.targets.filter((target) => draft.includes(mentionDraftToken(target)));
  return targets.length === intent.targets.length ? intent : { targets };
}

function SearchResults({
  query,
  results,
  rooms,
  onResultSelect
}: {
  query: string;
  results: SearchResult[];
  rooms: DesktopSnapshot["state"]["rooms"];
  onResultSelect: (roomId: string, eventId: string) => void;
}) {
  if (!query.trim()) {
    return null;
  }

  return (
    <section className="search-results">
      <div className="search-results-header">
        <span dir="auto">
          {t(results.length === 1 ? "search.resultCountOne" : "search.resultCountMany", {
            count: results.length,
            query
          })}
        </span>
      </div>
      <div className="result-list">
        {results.length ? (
          results.map((result) => {
            const room = rooms.find((candidate) => candidate.room_id === result.room_id);
            return (
              <button
                className="result-button"
                key={`${result.room_id}:${result.event_id}`}
                type="button"
                onClick={() => onResultSelect(result.room_id, result.event_id)}
              >
                <span dir="auto">{highlight(result.snippet, result.highlights)}</span>
                <span className="result-meta">
                  <span dir="auto">{room?.display_label ?? result.room_id}</span> ·{" "}
                  {matchFieldLabel(result.match_field)}
                </span>
              </button>
            );
          })
        ) : (
          <div className="empty-results">{t("search.noExactMatches")}</div>
        )}
      </div>
    </section>
  );
}

function MessageArticle({
  currentUserId,
  message,
  query,
  onOpenContextMenu,
  onEditMessage,
  onOpenThread,
  onRedactMessage,
  profileUsers
}: {
  currentUserId: string | null;
  message: TimelineMessage;
  query: string;
  onOpenContextMenu?: OpenContextMenu;
  onEditMessage: (message: TimelineMessage) => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
  profileUsers: Record<string, UserProfile>;
}) {
  const canManage = currentUserId === message.sender;

  return (
    <article
      className="message"
      data-event-id={message.event_id}
      onContextMenu={
        onOpenContextMenu
          ? (event) =>
              onOpenContextMenu(
                event,
                { kind: "message", message },
                contextMenuItems({
                  kind: "message",
                  canManage,
                  hasThread: true
                })
              )
          : undefined
      }
    >
      <div className="avatar" aria-hidden="true">
        {initials(message.sender)}
      </div>
      <div className="message-main">
        <div className="message-heading">
          <span className="sender" dir="auto">{message.sender}</span>
          <span className="time">{formatTime(message.timestamp_ms)}</span>
          {canManage ? (
            <span className="message-actions">
              <button
                className="message-action"
                type="button"
                aria-label={t("timeline.editMessage")}
                onClick={() => onEditMessage(message)}
              >
                <Edit3 size={ICON_SIZE.micro} />
              </button>
              <button
                className="message-action"
                type="button"
                aria-label={t("timeline.redactMessage")}
                onClick={() => onRedactMessage(message.room_id, message.event_id)}
              >
                <X size={ICON_SIZE.micro} />
              </button>
            </span>
          ) : null}
        </div>
        <div className="message-body" dir="auto">
          {renderTimelineMessageText(message.body, query, profileUsers)}
        </div>
        {message.attachment_filename ? (
          <div className="attachment">
            <Paperclip size={ICON_SIZE.small} />
            <span dir="auto">{highlightQueryLines(message.attachment_filename, query)}</span>
          </div>
        ) : null}
        {message.reply_count ? (
          <button
            className="reply-link"
            type="button"
            onClick={() => onOpenThread(message.room_id, message.event_id)}
          >
            {t("timeline.viewReplies", { count: message.reply_count })}
          </button>
        ) : null}
      </div>
    </article>
  );
}

export function Composer({
  composerMode,
  hasStagedUploads = false,
  isSending,
  mentionCandidates = [],
  mentionIntent = EMPTY_MENTION_INTENT,
  resolveComposerKeyAction = ignoreComposerKeyAction,
  roomName,
  value,
  onCancelReply,
  onAttachFiles = async () => undefined,
  onMentionIntentChange = () => undefined,
  onScheduleSend = async () => undefined,
  onSend,
  onValueChange
}: {
  composerMode: ComposerModeProp;
  hasStagedUploads?: boolean;
  isSending: boolean;
  mentionCandidates?: MentionCandidate[];
  mentionIntent?: MentionIntent;
  resolveComposerKeyAction?: ResolveComposerKeyAction;
  roomName: string;
  value: string;
  onCancelReply: () => void;
  onAttachFiles?: (files: File[]) => void | Promise<void>;
  onMentionIntentChange?: (intent: MentionIntent) => void;
  onScheduleSend?: (sendAtMs: number) => void | Promise<void>;
  onSend: () => void | Promise<void>;
  onValueChange: (value: string) => void;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [scheduleOpen, setScheduleOpen] = useState(false);
  const [scheduleValue, setScheduleValue] = useState(() => defaultScheduleDateTimeValue());
  const activeMention = activeMentionQuery(value);
  const activeMentionSuggestions =
    activeMention === null
      ? []
      : mentionCandidates
          .filter((candidate) => candidate.searchText.includes(activeMention.query.toLowerCase()))
          .slice(0, 5);
  const autocompleteOpen = activeMentionSuggestions.length > 0;

  function replaceTextRange(
    start: number,
    end: number,
    replacement: string,
    cursorOffset = replacement.length
  ) {
    const nextValue = `${value.slice(0, start)}${replacement}${value.slice(end)}`;
    const cursor = start + cursorOffset;
    onValueChange(nextValue);
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(cursor, cursor);
    });
  }

  function selectionRange(): { start: number; end: number } {
    const textarea = textareaRef.current;
    return {
      start: textarea?.selectionStart ?? value.length,
      end: textarea?.selectionEnd ?? value.length
    };
  }

  function keepComposerFocus(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
  }

  function applyInlineMarkdown(prefix: string, suffix = prefix, placeholder = "") {
    const { start, end } = selectionRange();
    const selected = value.slice(start, end) || placeholder;
    replaceTextRange(
      start,
      end,
      `${prefix}${selected}${suffix}`,
      prefix.length + selected.length + suffix.length
    );
  }

  function applyLinkMarkdown() {
    const { start, end } = selectionRange();
    const selected = value.slice(start, end) || "link";
    const replacement = `[${selected}](https://)`;
    replaceTextRange(start, end, replacement, replacement.length - 1);
  }

  function applyListMarkdown() {
    const { start, end } = selectionRange();
    const selected = value.slice(start, end);
    if (!selected) {
      replaceTextRange(start, end, "- ", 2);
      return;
    }
    const replacement = selected
      .split("\n")
      .map((line) => (line.startsWith("- ") ? line : `- ${line}`))
      .join("\n");
    replaceTextRange(start, end, replacement);
  }

  function insertMentionTrigger() {
    const { start, end } = selectionRange();
    replaceTextRange(start, end, "@");
  }

  function acceptMention(candidate: MentionCandidate) {
    if (!activeMention) {
      return;
    }
    const token = `${mentionDraftToken(candidate.target)} `;
    onValueChange(`${value.slice(0, activeMention.start)}${token}${value.slice(activeMention.end)}`);
    onMentionIntentChange(appendMentionTarget(mentionIntent, candidate.target));
    const cursor = activeMention.start + token.length;
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(cursor, cursor);
    });
  }

  async function onAttachFileChange(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.currentTarget.files ?? []);
    event.currentTarget.value = "";
    if (files.length === 0) {
      return;
    }
    try {
      await onAttachFiles(files);
    } catch {
      // Upload failure is reported through the Rust-owned operation/event path.
    }
  }

  async function attachDroppedOrPastedFiles(files: File[]) {
    if (files.length === 0) {
      return;
    }
    try {
      await onAttachFiles(files);
    } catch {
      // Upload failure is reported through the Rust-owned operation/event path.
    }
  }

  function openScheduleForm() {
    setScheduleValue(defaultScheduleDateTimeValue());
    setScheduleOpen(true);
  }

  async function submitSchedule(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const sendAtMs = scheduledSendTimestampFromInput(scheduleValue);
    if (sendAtMs === null || !value.trim() || hasStagedUploads || isSending) {
      return;
    }
    await onScheduleSend(sendAtMs);
    setScheduleOpen(false);
  }

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (!shouldResolveComposerKeyEvent(event)) {
      return;
    }

    const textarea = event.currentTarget;
    const selectionStart = textarea.selectionStart;
    const selectionEnd = textarea.selectionEnd;
    const keyEvent = composerKeyEventFromDom(event, {
      start: selectionStart,
      end: selectionEnd
    });
    const resolverOptions = {
      autocomplete_open: autocompleteOpen,
      send_enabled: !isSending && (value.trim().length > 0 || hasStagedUploads)
    };
    if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
      void resolveComposerKeyAction("main", keyEvent, resolverOptions).catch(() => undefined);
      return;
    }
    event.preventDefault();

    void resolveComposerKeyAction("main", keyEvent, resolverOptions)
      .then((action) => {
        if (action === "send") {
          void onSend();
          return;
        }
        if (action === "insertNewline") {
          const nextValue = insertNewlineAtSelection(value, selectionStart, selectionEnd);
          onValueChange(nextValue.value);
          requestAnimationFrame(() => {
            textarea.selectionStart = nextValue.cursor;
            textarea.selectionEnd = nextValue.cursor;
          });
          return;
        }
        if (action === "acceptAutocomplete") {
          const firstSuggestion = activeMentionSuggestions[0];
          if (firstSuggestion) {
            acceptMention(firstSuggestion);
          }
          return;
        }
        if (action === "cancel" && composerMode.kind === "reply") {
          onCancelReply();
        }
      })
      .catch(() => undefined);
  }

  return (
    <section className="composer" aria-label={t("composer.messageComposer")}>
      {composerMode.kind === "reply" ? (
        <div className="composer-reply-banner">
          <span className="composer-reply-label">{t("composer.replying")}</span>
          <button
            className="icon-button"
            type="button"
            aria-label={t("composer.cancelReply")}
            onClick={onCancelReply}
          >
            <X size={ICON_SIZE.small} />
          </button>
        </div>
      ) : null}
      <div className="composer-tools">
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.bold")}
          onMouseDown={keepComposerFocus}
          onClick={() => applyInlineMarkdown("**", "**", "bold")}
        >
          <Bold size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.italic")}
          onMouseDown={keepComposerFocus}
          onClick={() => applyInlineMarkdown("_", "_", "italic")}
        >
          <Italic size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.link")}
          onMouseDown={keepComposerFocus}
          onClick={applyLinkMarkdown}
        >
          <Link2 size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.list")}
          onMouseDown={keepComposerFocus}
          onClick={applyListMarkdown}
        >
          <List size={ICON_SIZE.input} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("composer.code")}
          onMouseDown={keepComposerFocus}
          onClick={() => applyInlineMarkdown("`", "`", "code")}
        >
          <Code2 size={ICON_SIZE.input} />
        </button>
      </div>
      {mentionIntent.targets.length ? (
        <div className="composer-mention-pills" aria-label={t("composer.selectedMentions")}>
          {mentionIntent.targets.map((target) => (
            <span className="mention-pill" key={mentionTargetKey(target)} dir="auto">
              {mentionPillLabel(target)}
            </span>
          ))}
        </div>
      ) : null}
      {autocompleteOpen ? (
        <div
          className="composer-autocomplete"
          role="listbox"
          aria-label={t("composer.mentionSuggestions")}
        >
          {activeMentionSuggestions.map((candidate) => (
            <button
              className="composer-autocomplete-option"
              key={candidate.key}
              type="button"
              role="option"
              aria-label={candidate.label}
              aria-selected="false"
              onMouseDown={keepComposerFocus}
              onClick={() => acceptMention(candidate)}
            >
              <span className="mention-option-label" dir="auto">
                {candidate.label}
              </span>
              {candidate.target.kind === "user" ? (
                <span className="mention-option-meta" dir="auto" aria-hidden="true">
                  {candidate.target.user_id}
                </span>
              ) : null}
            </button>
          ))}
        </div>
      ) : null}
      <textarea
        ref={textareaRef}
        aria-label={t("composer.messageComposer")}
        value={value}
        placeholder={t("composer.placeholder", { roomName })}
        onKeyDown={onComposerKeyDown}
        onPaste={(event) => {
          const files = Array.from(event.clipboardData.files);
          if (files.length > 0) {
            event.preventDefault();
            void attachDroppedOrPastedFiles(files);
          }
        }}
        onChange={(event) => onValueChange(event.target.value)}
      />
      <div
        className="composer-footer"
        onDragOver={(event) => {
          event.preventDefault();
        }}
        onDrop={(event) => {
          const files = Array.from(event.dataTransfer.files);
          if (files.length > 0) {
            event.preventDefault();
            void attachDroppedOrPastedFiles(files);
          }
        }}
      >
        <div>
          <input
            ref={fileInputRef}
            className="composer-file-input"
            type="file"
            multiple
            aria-label={t("composer.attachFileInput")}
            onChange={(event) => {
              void onAttachFileChange(event);
            }}
          />
          <button
            className="icon-button"
            type="button"
            aria-label={t("composer.attachFile")}
            onClick={() => fileInputRef.current?.click()}
          >
            <Paperclip size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("composer.mention")}
            onMouseDown={keepComposerFocus}
            onClick={insertMentionTrigger}
          >
            <AtSign size={ICON_SIZE.control} />
          </button>
          <button className="icon-button" type="button" aria-label={t("composer.emoji")}>
            <Smile size={ICON_SIZE.control} />
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={t("scheduled.sendLater")}
            disabled={isSending || !value.trim() || hasStagedUploads}
            onClick={openScheduleForm}
          >
            <Clock3 size={ICON_SIZE.control} />
          </button>
        </div>
        <button
          className={`send-button ${(value.trim() || hasStagedUploads) && !isSending ? "ready" : ""} ${isSending ? "is-sending" : ""}`}
          type="button"
          aria-label={isSending ? t("action.sending") : t("action.send")}
          disabled={isSending || (!value.trim() && !hasStagedUploads)}
          onClick={onSend}
        >
          <Send size={ICON_SIZE.input} />
        </button>
      </div>
      {scheduleOpen ? (
        <form className="scheduled-send-form" onSubmit={submitSchedule}>
          <label className="scheduled-send-field">
            <span>{t("scheduled.timeInput")}</span>
            <input
              aria-label={t("scheduled.timeInput")}
              type="datetime-local"
              value={scheduleValue}
              onChange={(event) => setScheduleValue(event.currentTarget.value)}
            />
          </label>
          <div className="scheduled-send-form-actions">
            <button className="dialog-button" type="button" onClick={() => setScheduleOpen(false)}>
              {t("action.cancel")}
            </button>
            <button
              className="dialog-button is-primary"
              type="submit"
              disabled={scheduledSendTimestampFromInput(scheduleValue) === null}
            >
              {t("scheduled.schedule")}
            </button>
          </div>
        </form>
      ) : null}
    </section>
  );
}

export function ContextualRightPanel({
  activeRoom,
  activeSpace,
  activeSpaceName,
  isRecoveryBusy,
  mode,
  recoverySecretFilled,
  recoverySecretInputRef,
  snapshot,
  timelineTransport = null,
  searchQuery,
  searchResults,
  savedSessions,
  onCloseThread,
  onClosePanel,
  onOpenKeyboardSettings,
  onOpenRecovery,
  onProbeLocalEncryption,
  onResetLocalData,
  onInviteUser = () => undefined,
  onModerateMember = () => undefined,
  onSetLocalUserAlias = () => undefined,
  onUpdateMemberRole = () => undefined,
  onRecoverySecretPresenceChange,
  onReply,
  onResultSelect,
  onSubmitRecovery,
  onSwitchAccount,
  onAcceptVerification,
  onBootstrapCrossSigning,
  onCancelVerification,
  onConfirmSasVerification,
  onExportRoomKeys,
  onImportRoomKeys,
  onBootstrapSecureBackup,
  onChangeSecureBackupPassphrase,
  onEnableKeyBackup,
  onResetIdentity,
  onResolveComposerKeyAction = ignoreComposerKeyAction,
  onSetAvatar = () => undefined,
  onSetDisplayName = () => undefined,
  onSubmitIdentityResetOAuth,
  onSubmitIdentityResetPassword,
  onUpdateSettings = () => undefined,
  onUpdateRoomSetting = () => undefined,
  onQueryDevices = () => undefined,
  onRenameDevice = () => undefined,
  onDeleteDevices = () => undefined,
  onSubmitAccountManagementUia = () => undefined,
  onThreadComposerDraftChange,
  onThreadReplySend
}: {
  activeRoom: DesktopSnapshot["state"]["rooms"][number] | null;
  activeSpace: DesktopSnapshot["state"]["spaces"][number] | null;
  activeSpaceName: string;
  isRecoveryBusy: boolean;
  mode: RightPanelMode;
  recoverySecretFilled: boolean;
  recoverySecretInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  timelineTransport?: TimelineTransport | null;
  searchQuery: string;
  searchResults: SearchResult[];
  savedSessions: SavedSessionInfo[];
  onCloseThread: () => void;
  onClosePanel: () => void;
  onOpenKeyboardSettings: () => void;
  onOpenRecovery: () => void;
  onProbeLocalEncryption: () => void;
  onResetLocalData: () => void;
  onInviteUser?: (roomId: string, title: string) => void;
  onModerateMember?: (
    roomId: string,
    targetUserId: string,
    action: RoomModerationAction,
    reason: string | null
  ) => void;
  onSetLocalUserAlias?: (userId: string, alias: string | null) => void;
  onUpdateMemberRole?: (
    roomId: string,
    targetUserId: string,
    powerLevel: number
  ) => void;
  onRecoverySecretPresenceChange: (value: boolean) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onResultSelect: (roomId: string, eventId: string) => void;
  onSubmitRecovery: (event: FormEvent<HTMLFormElement>) => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
  onAcceptVerification: (flowId: number) => void;
  onBootstrapCrossSigning: () => void;
  onCancelVerification: (flowId: number) => void;
  onConfirmSasVerification: (flowId: number) => void;
  onExportRoomKeys: (destinationPath: string, passphrase: string) => void;
  onImportRoomKeys: (sourcePath: string, passphrase: string) => void;
  onBootstrapSecureBackup: (
    passphrase: string | null,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onChangeSecureBackupPassphrase: (
    oldSecret: string,
    newPassphrase: string,
    recoveryKeyDestinationPath: string | null
  ) => void;
  onEnableKeyBackup: () => void;
  onResetIdentity: () => void;
  onResolveComposerKeyAction?: ResolveComposerKeyAction;
  onSetAvatar?: (file: File) => void;
  onSetDisplayName?: (displayName: string | null) => void;
  onSubmitIdentityResetOAuth: (flowId: number) => void;
  onSubmitIdentityResetPassword: (flowId: number, password: string) => void;
  onUpdateSettings?: (patch: SettingsPatch) => void;
  onQueryDevices?: () => void;
  onRenameDevice?: (deviceOrdinal: number, displayName: string) => void;
  onDeleteDevices?: (deviceOrdinals: number[]) => void;
  onSubmitAccountManagementUia?: (flowId: number, password: string) => void;
  onUpdateRoomSetting?: (roomId: string, change: RoomSettingChange) => void;
  onThreadComposerDraftChange: (roomId: string, rootEventId: string, draft: string) => void;
  onThreadReplySend: (roomId: string, rootEventId: string, body: string) => void;
}) {
  if (mode === "closed") {
    return <aside className="thread-pane" aria-label={t("panel.context")} />;
  }

  if (mode === "recovery") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.recovery")} onClose={onClosePanel} showClose={false} />
        <RecoveryPanel
          isBusy={isRecoveryBusy}
          secretFilled={recoverySecretFilled}
          secretInputRef={recoverySecretInputRef}
          snapshot={snapshot}
          onSecretPresenceChange={onRecoverySecretPresenceChange}
          onSubmit={onSubmitRecovery}
        />
      </aside>
    );
  }

  if (mode === "keyboardSettings") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.keyboard")} onClose={onClosePanel} />
        <KeyboardSettingsPanel
          labelProfile={shortcutLabelProfileFromLocaleProfile(snapshot.state.locale_profile)}
          settings={snapshot.state.settings}
          onUpdateSettings={onUpdateSettings}
        />
      </aside>
    );
  }

  if (mode === "userSettings") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.userSettings")} onClose={onClosePanel} />
        <UserSettingsPanel
          currentSession={currentSavedSession(snapshot)}
          e2eeTrust={snapshot.state.e2ee_trust}
          localEncryption={snapshot.state.local_encryption}
          platform={snapshot.state.locale_profile.platform}
          profile={snapshot.state.profile}
          savedSessions={savedSessions}
          settings={snapshot.state.settings}
          onAcceptVerification={onAcceptVerification}
          onBootstrapCrossSigning={onBootstrapCrossSigning}
          onCancelVerification={onCancelVerification}
          onConfirmSasVerification={onConfirmSasVerification}
          onExportRoomKeys={onExportRoomKeys}
          onImportRoomKeys={onImportRoomKeys}
          onBootstrapSecureBackup={onBootstrapSecureBackup}
          onChangeSecureBackupPassphrase={onChangeSecureBackupPassphrase}
          onEnableKeyBackup={onEnableKeyBackup}
          onOpenRecovery={onOpenRecovery}
          onOpenKeyboardSettings={onOpenKeyboardSettings}
          onProbeLocalEncryption={onProbeLocalEncryption}
          onResetLocalData={onResetLocalData}
          onResetIdentity={onResetIdentity}
          onSetAvatar={onSetAvatar}
          onSetDisplayName={onSetDisplayName}
          onSubmitIdentityResetOAuth={onSubmitIdentityResetOAuth}
          onSubmitIdentityResetPassword={onSubmitIdentityResetPassword}
          onUpdateSettings={onUpdateSettings}
          onSwitchAccount={onSwitchAccount}
          deviceSessions={snapshot.state.device_sessions}
          accountManagement={snapshot.state.account_management}
          onQueryDevices={onQueryDevices ?? (() => undefined)}
          onRenameDevice={onRenameDevice ?? (() => undefined)}
          onDeleteDevices={onDeleteDevices ?? (() => undefined)}
          onSubmitAccountManagementUia={onSubmitAccountManagementUia ?? (() => undefined)}
        />
      </aside>
    );
  }

  if (mode === "roomInfo") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.roomInfo")} onClose={onClosePanel} />
        <RoomInfoPanel
          currentUserId={snapshot.state.session.user_id ?? null}
          room={activeRoom}
          roomManagement={snapshot.state.room_management}
          spaces={snapshot.state.spaces}
          onInvitePeople={
            activeRoom
              ? () =>
                  onInviteUser(
                    activeRoom.room_id,
                    t("dialog.invitePeopleTitle", { name: activeRoom.display_label })
                  )
              : undefined
          }
          onModerateMember={onModerateMember}
          onSetLocalUserAlias={onSetLocalUserAlias}
          onUpdateMemberRole={onUpdateMemberRole}
          onUpdateRoomSetting={onUpdateRoomSetting}
        />
      </aside>
    );
  }

  if (mode === "spaceInfo") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.spaceInfo")} onClose={onClosePanel} />
        <SpaceInfoPanel
          fallbackName={activeSpaceName}
          rooms={snapshot.state.rooms}
          space={activeSpace}
          onInvitePeople={
            activeSpace
              ? () =>
                  onInviteUser(
                    activeSpace.space_id,
                    t("dialog.invitePeopleTitle", { name: activeSpace.display_name })
                  )
              : undefined
          }
        />
      </aside>
    );
  }

  if (mode === "search" || mode === "focusedContext") {
    const focusedContext = snapshot.state.focused_context;
    const currentUserId = snapshot.state.session.user_id ?? null;
    const focusedTimelineKeyValue =
      currentUserId &&
      timelineTransport &&
      (focusedContext.kind === "opening" || focusedContext.kind === "open")
        ? focusedTimelineKey(currentUserId, focusedContext.room_id, focusedContext.event_id)
        : null;
    const focusedRoomId =
      focusedContext.kind === "opening" || focusedContext.kind === "open"
        ? focusedContext.room_id
        : null;
    const focusedTimelineTransport = timelineTransport;
    const focusedPinnedEventIds = pinnedEventsForRoom(snapshot, focusedRoomId).map(
      (event) => event.event_id
    );

    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader
          title={mode === "search" ? t("panel.search") : t("panel.focusedContext")}
          onClose={onClosePanel}
        />
        {focusedTimelineKeyValue && focusedRoomId && focusedTimelineTransport ? (
          <section className="focused-context-panel" aria-label={t("panel.focusedContext")}>
            {mode === "search" ? (
              <div className="search-results-header">
                <span>{t("panel.focusedContext")}</span>
              </div>
            ) : null}
            <TimelineView
              roomId={focusedRoomId}
              timelineKey={focusedTimelineKeyValue}
              transport={focusedTimelineTransport}
              suppressPaginationUi={true}
              onReply={onReply}
              resolveComposerKeyAction={onResolveComposerKeyAction}
              liveSignals={snapshot.state.live_signals}
              profileUsers={snapshot.state.profile.users}
              pinnedEventIds={focusedPinnedEventIds}
              forwardDestinations={forwardDestinationsFromSnapshot(snapshot)}
              onSetLocalUserAlias={onSetLocalUserAlias}
              codeBlockWrap={snapshot.state.settings.values.display.code_block_wrap}
              searchQuery={searchQuery}
            />
          </section>
        ) : null}
        {mode === "search" ? (
          <SearchResults
            query={searchQuery}
            results={searchResults}
            rooms={snapshot.state.rooms}
            onResultSelect={onResultSelect}
          />
        ) : null}
      </aside>
    );
  }

  const threadState = snapshot.state.thread;
  if (threadState.kind !== "opening" && threadState.kind !== "open") {
    return <aside className="thread-pane" aria-label={t("panel.context")} />;
  }

  const currentUserId = snapshot.state.session.user_id ?? null;
  const threadRoomId = threadState.room_id;
  const rootEventId = threadState.root_event_id;
  const threadComposer = threadState.kind === "open" ? threadState.composer : undefined;
  const threadDraft = threadComposer?.draft ?? "";
  const threadSendPending = Boolean(threadComposer?.pending_transaction_id);
  const threadTimelineKeyValue =
    currentUserId && timelineTransport && threadRoomId && rootEventId
      ? threadTimelineKey(currentUserId, threadRoomId, rootEventId)
      : null;
  const threadPinnedEventIds = pinnedEventsForRoom(snapshot, threadRoomId).map(
    (event) => event.event_id
  );

  return (
    <aside className="thread-pane" aria-label={t("panel.context")}>
      <PanelHeader title={t("panel.thread")} onClose={onCloseThread} />
      <section className="thread-scroll thread-timeline-panel">
        {threadTimelineKeyValue && threadRoomId && timelineTransport ? (
          <TimelineView
            key={`${threadRoomId}:${rootEventId}`}
            roomId={threadRoomId}
            timelineKey={threadTimelineKeyValue}
            transport={timelineTransport}
            onReply={onReply}
            onOpenThread={() => undefined}
            resolveComposerKeyAction={onResolveComposerKeyAction}
            liveSignals={snapshot.state.live_signals}
            profileUsers={snapshot.state.profile.users}
            pinnedEventIds={threadPinnedEventIds}
            forwardDestinations={forwardDestinationsFromSnapshot(snapshot)}
            onSetLocalUserAlias={onSetLocalUserAlias}
            codeBlockWrap={snapshot.state.settings.values.display.code_block_wrap}
            searchQuery={searchQuery}
          />
        ) : (
          <div className="thread-root-placeholder">{t("timeline.openingThread")}</div>
        )}
      </section>
      <ThreadComposer
        draft={threadDraft}
        isSending={threadSendPending}
        resolveComposerKeyAction={onResolveComposerKeyAction}
        canEdit={threadState.kind === "open" && Boolean(threadRoomId && rootEventId && threadComposer)}
        onDraftChange={(draft) => {
          if (threadRoomId && rootEventId) {
            onThreadComposerDraftChange(threadRoomId, rootEventId, draft);
          }
        }}
        onSend={() => {
          if (threadRoomId && rootEventId) {
            onThreadReplySend(threadRoomId, rootEventId, threadDraft);
          }
        }}
      />
    </aside>
  );
}

function ThreadComposer({
  canEdit,
  draft,
  isSending,
  resolveComposerKeyAction,
  onDraftChange,
  onSend
}: {
  canEdit: boolean;
  draft: string;
  isSending: boolean;
  resolveComposerKeyAction: ResolveComposerKeyAction;
  onDraftChange: (draft: string) => void;
  onSend: () => void | Promise<void>;
}) {
  const canSend = canEdit && !isSending && draft.trim().length > 0;

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (!shouldResolveComposerKeyEvent(event)) {
      return;
    }

    const textarea = event.currentTarget;
    const selectionStart = textarea.selectionStart;
    const selectionEnd = textarea.selectionEnd;
    const keyEvent = composerKeyEventFromDom(event, {
      start: selectionStart,
      end: selectionEnd
    });
    const resolverOptions = {
      autocomplete_open: false,
      send_enabled: canSend
    };
    if (shouldLetNativeImeHandleComposerKeyEvent(keyEvent)) {
      void resolveComposerKeyAction("thread", keyEvent, resolverOptions).catch(() => undefined);
      return;
    }
    event.preventDefault();

    void resolveComposerKeyAction("thread", keyEvent, resolverOptions)
      .then((action) => {
        if (action === "send") {
          void onSend();
          return;
        }
        if (action === "insertNewline") {
          const nextDraft = insertNewlineAtSelection(draft, selectionStart, selectionEnd);
          onDraftChange(nextDraft.value);
          requestAnimationFrame(() => {
            textarea.selectionStart = nextDraft.cursor;
            textarea.selectionEnd = nextDraft.cursor;
          });
        }
      })
      .catch(() => undefined);
  }

  return (
    <section className="thread-composer" aria-label={t("timeline.threadComposer")}>
      <textarea
        aria-label={t("timeline.threadComposer")}
        disabled={!canEdit}
        placeholder={t("timeline.threadPlaceholder")}
        value={draft}
        onChange={(event) => onDraftChange(event.target.value)}
        onKeyDown={onComposerKeyDown}
      />
      <div className="thread-composer-footer">
        <button
          className={`send-button ${canSend ? "ready" : ""} ${isSending ? "is-sending" : ""}`}
          type="button"
          aria-label={isSending ? t("action.sending") : t("action.send")}
          disabled={!canSend}
          onClick={onSend}
        >
          <Send size={ICON_SIZE.input} />
        </button>
      </div>
    </section>
  );
}

function PanelHeader({
  title,
  onClose,
  showClose = true
}: {
  title: string;
  onClose: () => void;
  showClose?: boolean;
}) {
  return (
    <header className="thread-header">
      <div className="thread-title">{title}</div>
      <button className="icon-button" type="button" aria-label={t("action.more")}>
        <MoreHorizontal size={ICON_SIZE.panel} />
      </button>
      {showClose ? (
        <button className="icon-button" type="button" aria-label={t("action.close", { title })} onClick={onClose}>
          <X size={ICON_SIZE.panel} />
        </button>
      ) : null}
    </header>
  );
}

function currentSavedSession(snapshot: DesktopSnapshot): SavedSessionInfo | null {
  const session = snapshot.state.session;
  if (!session.homeserver || !session.user_id || !session.device_id) {
    return null;
  }
  return {
    homeserver: session.homeserver,
    user_id: session.user_id,
    device_id: session.device_id
  };
}

function shortcutLabelProfileFromLocaleProfile(
  profile: LocaleDisplayProfile
): ShortcutLabelProfile {
  return {
    platform: profile.platform,
    modLabel: profile.modifier_labels.primary
  };
}

function highlightQueryLines(text: string, query: string) {
  if (!query.trim()) {
    return text.split("\n").map((line, index) => (
      <span key={`${line}:${index}`}>
        {index > 0 ? <br /> : null}
        {line}
      </span>
    ));
  }

  return text.split("\n").map((line, index) => (
    <span key={`${line}:${index}`}>
      {index > 0 ? <br /> : null}
      {highlightString(line, query)}
    </span>
  ));
}

function highlightString(text: string, query: string) {
  const index = text.indexOf(query);
  if (index < 0 || query.length === 0) {
    return text;
  }
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}

function highlight(text: string, ranges: SearchResult["highlights"]) {
  if (!ranges.length) {
    return text;
  }

  const range = ranges[0];
  const chars = Array.from(text);
  const start = utf16OffsetToCodePointIndex(text, range.start_utf16);
  const end = utf16OffsetToCodePointIndex(text, range.end_utf16);
  return (
    <>
      {chars.slice(0, start).join("")}
      <mark>{chars.slice(start, end).join("")}</mark>
      {chars.slice(end).join("")}
    </>
  );
}

function utf16OffsetToCodePointIndex(value: string, offset: number): number {
  let utf16Count = 0;
  for (const [index, char] of Array.from(value).entries()) {
    if (utf16Count >= offset) {
      return index;
    }
    utf16Count += char.length;
  }
  return Array.from(value).length;
}

function matchFieldLabel(field: SearchResult["match_field"]): string {
  switch (field) {
    case "messageBody":
      return t("search.matchMessage");
    case "attachmentFileName":
      return t("search.matchAttachmentFileName");
  }
}

function serverNameFromAlias(alias: string): string | null {
  const separatorIndex = alias.indexOf(":");
  if (separatorIndex < 0 || separatorIndex + 1 >= alias.length) {
    return null;
  }
  return alias.slice(separatorIndex + 1).trim() || null;
}

function operationFailureLabel(kind: OperationFailureKind): string {
  switch (kind) {
    case "forbidden":
      return t("directory.failureForbidden");
    case "notFound":
      return t("directory.failureNotFound");
    case "network":
      return t("directory.failureNetwork");
    case "timeout":
      return t("directory.failureTimeout");
    case "invalid":
      return t("directory.failureInvalid");
    case "sdk":
      return t("directory.failureSdk");
  }
}

function initials(value: string): string {
  const ascii = value.match(/[A-Za-z]/g);
  if (ascii?.length) {
    return ascii.slice(0, 2).join("").toUpperCase();
  }
  return value.slice(0, 2);
}

function formatTime(timestampMs: number): string {
  return new Intl.DateTimeFormat("ja-JP", {
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(timestampMs));
}

function formatScheduledSendTime(timestampMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(timestampMs));
}

function defaultScheduleDateTimeValue(): string {
  return datetimeLocalValueFromTimestamp(Date.now() + 10 * 60 * 1000);
}

function datetimeLocalValueFromTimestamp(timestampMs: number): string {
  const date = new Date(timestampMs);
  const pad = (value: number) => String(value).padStart(2, "0");
  return [
    date.getFullYear(),
    "-",
    pad(date.getMonth() + 1),
    "-",
    pad(date.getDate()),
    "T",
    pad(date.getHours()),
    ":",
    pad(date.getMinutes())
  ].join("");
}

function scheduledSendTimestampFromInput(value: string): number | null {
  if (!value.trim()) {
    return null;
  }
  const timestampMs = new Date(`${value}:00`).getTime();
  return Number.isFinite(timestampMs) ? timestampMs : null;
}

function scheduledSendCapabilityLabel(capability: ScheduledSendCapability): string {
  switch (capability) {
    case "serverDelayedEvents":
      return t("scheduled.serverDelayedEvents");
    case "localFallback":
      return t("scheduled.localFallback");
    case "unknown":
      return t("scheduled.unknownCapability");
  }
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
