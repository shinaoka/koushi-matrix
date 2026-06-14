import {
  AtSign,
  Bell,
  Bold,
  Clock3,
  Code2,
  Edit3,
  Hash,
  HelpCircle,
  Home,
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
  Plus,
  Search,
  RefreshCw,
  Send,
  Settings,
  ShieldCheck,
  Smile,
  Users,
  X
} from "lucide-react";
import {
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
  type RefObject,
  useEffect,
  useRef,
  useState
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { createDesktopApi } from "./backend/client";
import { t } from "./i18n/messages";
import { ContextMenuSurface } from "./components/ContextMenuSurface";
import {
  TimelineView,
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
import { UserSettingsPanel } from "./components/UserSettingsPanel";
import {
  type ContextMenuActionId,
  type ContextMenuItem,
  contextMenuItems
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
  desktopAttentionSummary,
  desktopAttentionWindowTitle,
  desktopAttentionNotificationCandidate
} from "./domain/desktopAttention";
import {
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
  ComposerMode,
  DesktopSnapshot,
  RoomListItem,
  SavedSessionInfo,
  SearchResult,
  SearchScopeKind,
  TimelineMessage
} from "./domain/types";

const api = createDesktopApi();
const DEFAULT_HOMESERVER = "https://matrix.org";
const MENU_EVENT_NAME = "matrix-desktop://menu";
const STATE_EVENT_NAME = "matrix-desktop://state";
const CORE_EVENT_NAME = "matrix-desktop://event";

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
      async toggleReaction(roomId: string, eventId: string, reactionKey: string) {
        await invoke("toggle_reaction", { roomId, eventId, reactionKey });
      },
      async editMessage(roomId: string, eventId: string, body: string) {
        await invoke("edit_message", { roomId, eventId, body });
      },
      async redactMessage(roomId: string, eventId: string) {
        await invoke("redact_message", { roomId, eventId });
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

/**
 * React-local view of the composer mode. Matrix semantics (the reply target)
 * stay Rust-owned; this is a presentational mapping of the WIRE value
 * `snapshot.state.timeline.composer.mode` (externally tagged ComposerMode).
 */
type ComposerModeProp =
  | { kind: "plain" }
  | { kind: "reply"; in_reply_to_event_id: string };

function composerModeProp(mode: ComposerMode): ComposerModeProp {
  return mode === "Plain"
    ? { kind: "plain" }
    : { kind: "reply", in_reply_to_event_id: mode.Reply.in_reply_to_event_id };
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
  const [composerDraft, setComposerDraft] = useState("");
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
  const previousAttentionInput = useRef<{
    activeRoomId: string | null;
    rooms: DesktopSnapshot["state"]["rooms"];
  } | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const loginPasswordRef = useRef<HTMLInputElement>(null);
  const recoverySecretRef = useRef<HTMLInputElement>(null);
  const attentionSummary = snapshot
    ? desktopAttentionSummary({
        activeRoomId: snapshot.state.navigation.active_room_id,
        rooms: snapshot.state.rooms
      })
    : null;
  const safeAttentionSummary =
    attentionSummary ?? {
      unreadTotal: 0,
      badgeCount: 0,
      notificationKind: "none" as const,
      titleHint: null,
      qaTitleToken: "unread=0 badge=0 notify=none"
    };

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
      safeAttentionSummary.badgeCount
    );
  }, [snapshot, rightPanelMode, qaSendStatus, safeAttentionSummary.badgeCount, safeAttentionSummary.qaTitleToken]);

  useEffect(() => {
    if (!snapshot) {
      previousAttentionInput.current = null;
      return;
    }

    const currentAttentionInput = {
      activeRoomId: snapshot.state.navigation.active_room_id,
      rooms: snapshot.state.rooms
    };
    const candidate = desktopAttentionNotificationCandidate(
      currentAttentionInput,
      previousAttentionInput.current
    );
    previousAttentionInput.current = currentAttentionInput;

    if (!candidate || !tauriNotificationTransport) {
      return;
    }

    void sendDesktopAttentionNotification(candidate, tauriNotificationTransport);
  }, [snapshot]);

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

  async function selectSpace(spaceId: string | null) {
    setSnapshot(await api.selectSpace(spaceId));
  }

  async function selectRoom(roomId: string) {
    setSnapshot(await api.selectRoom(roomId));
  }

  function openCreateDialog(kind: "room" | "space") {
    setCreateDraftName("");
    setCreateDialog(kind);
  }

  function closeCreateDialog() {
    setCreateDialog(null);
    setCreateDraftName("");
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
    if (!roomId || !body.trim()) {
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
          ? await api.sendText(roomId, body)
          : await api.sendReply(roomId, composerMode.Reply.in_reply_to_event_id, body);
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
    setComposerDraft("");
  }

  async function editMessage(message: TimelineMessage) {
    const body = window.prompt("Edit message", message.body);
    if (body === null || !body.trim()) {
      return;
    }

    setSnapshot(await api.editMessage(message.room_id, message.event_id, body));
  }

  async function redactMessage(roomId: string, eventId: string) {
    setSnapshot(await api.redactMessage(roomId, eventId));
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
        activeSpaceName={activeSpace?.display_name ?? "Matrix"}
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
          snapshot={snapshot}
          onCreateSpace={() => openCreateDialog("space")}
          onOpenContextMenu={openContextMenu}
          onOpenUserSettings={() => {
            void setRightPanelModeClosingFocusedContext("userSettings");
          }}
          onSelectSpace={selectSpace}
        />
        <Sidebar
          activeRoomId={snapshot.state.navigation.active_room_id}
          snapshot={snapshot}
          onCreateRoom={() => openCreateDialog("room")}
          onOpenContextMenu={openContextMenu}
          onOpenSpaceInfo={() => {
            void setRightPanelModeClosingFocusedContext("spaceInfo");
          }}
          onSelectRoom={selectRoom}
        />
        <TimelinePane
          activeRoomName={activeRoom?.display_name ?? "No room"}
          composerDraft={composerDraft}
          composerMode={composerModeProp(snapshot.state.timeline.composer.mode)}
          searchQuery={searchQuery}
          searchResults={searchResults}
          showSearchResults={effectiveRightPanelMode !== "search"}
          snapshot={snapshot}
          onCancelReply={() => {
            void cancelComposerReply();
          }}
          onComposerDraftChange={setComposerDraft}
          onOpenThread={openThread}
          onPaginateBackwards={paginateTimelineBackwards}
          onReply={(roomId, eventId) => {
            void setComposerReplyTarget(roomId, eventId);
          }}
          onSendText={sendText}
          onEditMessage={editMessage}
          onOpenContextMenu={openContextMenu}
          onRedactMessage={redactMessage}
          onResultSelect={selectSearchResult}
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
        <ContextualRightPanel
          activeRoom={activeRoom ?? null}
          activeSpace={activeSpace ?? null}
          activeSpaceName={activeSpace?.display_name ?? snapshot.sidebar.account_home.display_name}
          isRecoveryBusy={isBusy || sessionKind === "recovering"}
          mode={effectiveRightPanelMode}
          recoverySecretFilled={recoverySecretFilled}
          recoverySecretInputRef={recoverySecretRef}
          snapshot={snapshot}
          timelineTransport={tauriTimelineTransport}
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
            <ShieldCheck size={23} />
          </div>
          <div>
            <h1>Encryption Recovery</h1>
            <p>{session.user_id ?? "Matrix account"}</p>
          </div>
        </div>
        <div className="recovery-summary">
          <KeyRound size={18} />
          <div className="recovery-methods" aria-label="Supported recovery methods">
            {(session.recovery_methods ?? ["recoveryKey", "securityPhrase"]).map((method) => (
              <span className="recovery-chip" key={method}>
                {recoveryMethodLabel(method)}
              </span>
            ))}
          </div>
        </div>
        <label className="auth-field">
          <span>Recovery key or security phrase</span>
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
          {isBusy ? "Recovering" : "Recover"}
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
            <Hash size={22} />
          </div>
          <div>
            <h1>Matrix Desktop</h1>
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
            Check login methods
          </button>
          <div className="auth-flows">{authDiscoveryLabel(auth)}</div>
        </div>
        <label className="auth-field">
          <span>Username or Matrix ID</span>
          <input
            autoComplete="username"
            name="username"
            spellCheck={false}
            value={username}
            onChange={(event) => onUsernameChange(event.target.value)}
          />
        </label>
        <label className="auth-field">
          <span>Password</span>
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
          <span>Device name</span>
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
          {isBusy ? "Connecting" : "Continue"}
        </button>
      </form>
    </main>
  );
}

function authDiscoveryLabel(auth: DesktopSnapshot["state"]["auth"]) {
  switch (auth.kind) {
    case "discovering":
      return "Checking";
    case "ready": {
      const labels = auth.flows.map((flow) =>
        typeof flow.kind === "string" ? flow.kind : "unknown"
      );
      return labels.length ? labels.join(" / ") : "No login methods";
    }
    case "failed":
      return auth.message;
    case "unknown":
    default:
      return "Not checked";
  }
}

function sessionLabel(kind: DesktopSnapshot["state"]["session"]["kind"]) {
  switch (kind) {
    case "authenticating":
      return "Connecting";
    case "needsRecovery":
      return "Encryption recovery";
    case "recovering":
      return "Recovering";
    case "locked":
      return "Session locked";
    case "signedOut":
    default:
      return "Sign in";
  }
}

function recoveryMethodLabel(
  method: NonNullable<DesktopSnapshot["state"]["session"]["recovery_methods"]>[number]
) {
  switch (method) {
    case "recoveryKey":
      return "Recovery key";
    case "securityPhrase":
      return "Security phrase";
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
          label: "Starting",
          detail: null,
          ariaLabel: "Sync starting",
          restartable: false
        };
      case "running":
        return {
          state: "running",
          label: "Running",
          detail: null,
          ariaLabel: "Sync running",
          restartable: false
        };
      case "stopped":
      default:
        return {
          state: "stopped",
          label: "Stopped",
          detail: null,
          ariaLabel: "Sync stopped",
          restartable: true
        };
    }
  }

  if ("reconnecting" in sync) {
    return {
      state: "reconnecting",
      label: "Reconnecting",
      detail: sync.reconnecting,
      ariaLabel: sync.reconnecting ? `Sync reconnecting: ${sync.reconnecting}` : "Sync reconnecting",
      restartable: true
    };
  }

  return {
    state: "failed",
    label: "Failed",
    detail: sync.failed,
    ariaLabel: sync.failed ? `Sync failed: ${sync.failed}` : "Sync failed",
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
        <button className="icon-button" type="button" aria-label="History">
          <Clock3 size={18} />
        </button>
      </div>
      <label className="top-search">
        <Search size={17} />
        <input
          ref={searchInputRef}
          aria-label={t("workspace.search")}
          value={searchQuery}
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
        <option value="allRooms">All</option>
        <option value="currentSpace">Space</option>
        <option value="currentRoom">Room</option>
        <option value="dms">DM</option>
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
            <RefreshCw size={18} />
          </button>
        ) : null}
        <button
          className="icon-button"
          type="button"
          aria-label="Keyboard settings"
          onClick={onOpenKeyboardSettings}
        >
          <HelpCircle size={18} />
        </button>
      </div>
    </header>
  );
}

export function WorkspaceRail({
  snapshot,
  onCreateSpace,
  onOpenContextMenu,
  onOpenUserSettings,
  onSelectSpace
}: {
  snapshot: DesktopSnapshot;
  onCreateSpace: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenUserSettings: () => void;
  onSelectSpace: (spaceId: string | null) => void;
}) {
  return (
    <nav className="workspace-rail" aria-label={t("workspace.workspaces")}>
      <div className="workspace-list">
        <button
          className={`workspace-button ${snapshot.sidebar.account_home.is_active ? "is-active" : ""}`}
          data-count={snapshot.sidebar.account_home.unread_count || undefined}
          type="button"
          aria-label={snapshot.sidebar.account_home.display_name}
          onClick={() => onSelectSpace(null)}
        >
          <Home size={20} />
        </button>
        {snapshot.sidebar.space_rail.map((space) => (
          <button
            className={`workspace-button ${space.is_active ? "is-active" : ""}`}
            data-count={space.unread_count || undefined}
            key={space.space_id}
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
          >
            {initials(space.display_name)}
          </button>
        ))}
      </div>
      <div className="rail-footer">
        <button
          className="rail-action"
          type="button"
          aria-label={t("action.createSpace")}
          onClick={onCreateSpace}
        >
          <Plus size={22} />
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

function Sidebar({
  activeRoomId,
  snapshot,
  onCreateRoom,
  onOpenContextMenu,
  onOpenSpaceInfo,
  onSelectRoom
}: {
  activeRoomId: string | null;
  snapshot: DesktopSnapshot;
  onCreateRoom: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenSpaceInfo: () => void;
  onSelectRoom: (roomId: string) => void;
}) {
  return (
    <aside className="sidebar" aria-label={t("workspace.rooms")}>
      <div className="workspace-header">
        <div className="workspace-name">
          {snapshot.sidebar.space_rail.find((space) => space.is_active)?.display_name ??
            snapshot.sidebar.account_home.display_name}
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label={t("workspace.spaceInfoSettings")}
          onClick={onOpenSpaceInfo}
        >
          <Settings size={18} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={t("action.createRoom")}
          onClick={onCreateRoom}
        >
          <Edit3 size={18} />
        </button>
      </div>
      <div className="sidebar-scroll">
        <NavButton
          active={snapshot.sidebar.account_home.is_active}
          icon={<Home size={18} />}
          label="Home"
        />
        <NavButton icon={<MessageCircle size={18} />} label="Threads" />
        <NavButton icon={<Bell size={18} />} label="Invites" />
        <SectionTitle label={t("workspace.rooms")} />
        {snapshot.sidebar.space_rooms.map((room) => (
          <RoomButton
            activeRoomId={activeRoomId}
            icon={<Hash size={16} />}
            key={room.room_id}
            room={room}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
          />
        ))}
        <SectionTitle label="People" />
        {snapshot.sidebar.global_dms.map((room) => (
          <RoomButton
            activeRoomId={activeRoomId}
            icon={<span className="presence-dot" />}
            key={room.room_id}
            room={room}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
          />
        ))}
      </div>
    </aside>
  );
}

function NavButton({
  active = false,
  icon,
  label
}: {
  active?: boolean;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button className={`nav-item ${active ? "is-active" : ""}`} type="button">
      {icon}
      <span className="nav-label">{label}</span>
    </button>
  );
}

function SectionTitle({ label }: { label: string }) {
  return (
    <div className="section-title">
      <span>{label}</span>
      <Plus size={15} />
    </div>
  );
}

function RoomButton({
  activeRoomId,
  icon,
  room,
  onOpenContextMenu,
  onSelectRoom
}: {
  activeRoomId: string | null;
  icon: React.ReactNode;
  room: RoomListItem;
  onOpenContextMenu: OpenContextMenu;
  onSelectRoom: (roomId: string) => void;
}) {
  return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      type="button"
      onClick={() => onSelectRoom(room.room_id)}
      onContextMenu={(event) =>
        onOpenContextMenu(
          event,
          { kind: "room", roomId: room.room_id },
          contextMenuItems({ kind: "room" })
        )
      }
    >
      {icon}
      <span className="room-name">{room.display_name}</span>
      <span className="room-count">{room.unread_count || ""}</span>
    </button>
  );
}

function TimelinePane({
  activeRoomName,
  composerDraft,
  composerMode,
  searchQuery,
  searchResults,
  showSearchResults,
  snapshot,
  onCancelReply,
  onComposerDraftChange,
  onEditMessage,
  onOpenContextMenu,
  onOpenThread,
  onPaginateBackwards,
  onRedactMessage,
  onReply,
  onResultSelect,
  onSendText,
  onToggleThread,
  onOpenRoomInfo
}: {
  activeRoomName: string;
  composerDraft: string;
  composerMode: ComposerModeProp;
  searchQuery: string;
  searchResults: SearchResult[];
  showSearchResults: boolean;
  snapshot: DesktopSnapshot;
  onCancelReply: () => void;
  onComposerDraftChange: (value: string) => void;
  onEditMessage: (message: TimelineMessage) => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onPaginateBackwards: (roomId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onResultSelect: (roomId: string, eventId: string) => void;
  onSendText: () => void;
  onToggleThread: () => void;
  onOpenRoomInfo: () => void;
}) {
  const timelineRoomId = snapshot.state.timeline.room_id;
  const currentUserId = snapshot.state.session.user_id ?? null;

  return (
    <main className="main-pane" aria-label={t("timeline.conversation")}>
      <header className="channel-header">
        <div className="channel-title">
          <Hash size={22} />
          <span>{activeRoomName}</span>
        </div>
        <div className="channel-actions">
          <button className="member-pill" type="button" aria-label={t("room.members")}>
            <Users size={16} />
            <span>8</span>
          </button>
          <button className="icon-button" type="button" aria-label={t("room.threadToggle")} onClick={onToggleThread}>
            {snapshot.state.thread.kind !== "closed" ? <PanelRightClose size={19} /> : <PanelRightOpen size={19} />}
          </button>
          <button className="icon-button" type="button" aria-label={t("room.roomInfo")} onClick={onOpenRoomInfo}>
            <MoreVertical size={19} />
          </button>
        </div>
      </header>
      <nav className="tabs" aria-label={t("room.tabs")}>
        <button className="tab is-active" type="button">
          Messages
        </button>
      </nav>
      <section className="timeline-scroll">
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
              <Clock3 size={15} />
              <span>
                {snapshot.state.timeline.is_paginating_backwards
                  ? t("timeline.loading")
                  : t("timeline.olderMessages")}
              </span>
            </button>
          </div>
          {tauriTimelineTransport && timelineRoomId && currentUserId ? (
            // Production path: render from the event-driven timeline store
            // (CoreEvent diffs), never from AppState timeline fields.
            <TimelineView
              key={timelineRoomId}
              roomId={timelineRoomId}
              timelineKey={roomTimelineKey(currentUserId, timelineRoomId)}
              transport={tauriTimelineTransport}
              onReply={onReply}
              onOpenThread={onOpenThread}
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
              />
            ))
          )}
        </div>
      </section>
      <Composer
        composerMode={composerMode}
        isSending={Boolean(snapshot.state.timeline.composer.pending_transaction_id)}
        roomName={activeRoomName}
        value={composerDraft}
        onCancelReply={onCancelReply}
        onSend={onSendText}
        onValueChange={onComposerDraftChange}
      />
    </main>
  );
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
        <span>
          {results.length} result{results.length === 1 ? "" : "s"} for "{query}"
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
                <span>{highlight(result.snippet, result.highlights)}</span>
                <span className="result-meta">
                  {room?.display_name ?? result.room_id} · {matchFieldLabel(result.match_field)}
                </span>
              </button>
            );
          })
        ) : (
          <div className="empty-results">No exact matches</div>
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
  onRedactMessage
}: {
  currentUserId: string | null;
  message: TimelineMessage;
  query: string;
  onOpenContextMenu?: OpenContextMenu;
  onEditMessage: (message: TimelineMessage) => void;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
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
          <span className="sender">{message.sender}</span>
          <span className="time">{formatTime(message.timestamp_ms)}</span>
          {canManage ? (
            <span className="message-actions">
              <button
                className="message-action"
                type="button"
                aria-label={t("timeline.editMessage")}
                onClick={() => onEditMessage(message)}
              >
                <Edit3 size={14} />
              </button>
              <button
                className="message-action"
                type="button"
                aria-label={t("timeline.redactMessage")}
                onClick={() => onRedactMessage(message.room_id, message.event_id)}
              >
                <X size={14} />
              </button>
            </span>
          ) : null}
        </div>
        <div className="message-body">{highlightQueryLines(message.body, query)}</div>
        {message.attachment_filename ? (
          <div className="attachment">
            <Paperclip size={16} />
            <span>{highlightQueryLines(message.attachment_filename, query)}</span>
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
  isSending,
  roomName,
  value,
  onCancelReply,
  onSend,
  onValueChange
}: {
  composerMode: ComposerModeProp;
  isSending: boolean;
  roomName: string;
  value: string;
  onCancelReply: () => void;
  onSend: () => void;
  onValueChange: (value: string) => void;
}) {
  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (!isSending) {
        onSend();
      }
    }
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
            <X size={16} />
          </button>
        </div>
      ) : null}
      <div className="composer-tools">
        <button className="icon-button" type="button" aria-label={t("composer.bold")}>
          <Bold size={17} />
        </button>
        <button className="icon-button" type="button" aria-label={t("composer.italic")}>
          <Italic size={17} />
        </button>
        <button className="icon-button" type="button" aria-label={t("composer.link")}>
          <Link2 size={17} />
        </button>
        <button className="icon-button" type="button" aria-label={t("composer.list")}>
          <List size={17} />
        </button>
        <button className="icon-button" type="button" aria-label={t("composer.code")}>
          <Code2 size={17} />
        </button>
      </div>
      <textarea
        aria-label={t("composer.messageComposer")}
        value={value}
        placeholder={t("composer.placeholder", { roomName })}
        onKeyDown={onComposerKeyDown}
        onChange={(event) => onValueChange(event.target.value)}
      />
      <div className="composer-footer">
        <div>
          <button className="icon-button" type="button" aria-label={t("action.add")}>
            <Plus size={19} />
          </button>
          <button className="icon-button" type="button" aria-label={t("composer.mention")}>
            <AtSign size={18} />
          </button>
          <button className="icon-button" type="button" aria-label={t("composer.emoji")}>
            <Smile size={18} />
          </button>
        </div>
        <button
          className={`send-button ${value.trim() && !isSending ? "ready" : ""} ${isSending ? "is-sending" : ""}`}
          type="button"
          aria-label={isSending ? t("action.sending") : t("action.send")}
          disabled={isSending || !value.trim()}
          onClick={onSend}
        >
          <Send size={17} />
        </button>
      </div>
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
  onRecoverySecretPresenceChange,
  onReply,
  onResultSelect,
  onSubmitRecovery,
  onSwitchAccount,
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
  onRecoverySecretPresenceChange: (value: boolean) => void;
  onReply: TimelineRowActionHandlers["onReply"];
  onResultSelect: (roomId: string, eventId: string) => void;
  onSubmitRecovery: (event: FormEvent<HTMLFormElement>) => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
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
        <KeyboardSettingsPanel />
      </aside>
    );
  }

  if (mode === "userSettings") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.userSettings")} onClose={onClosePanel} />
        <UserSettingsPanel
          currentSession={currentSavedSession(snapshot)}
          savedSessions={savedSessions}
          onOpenKeyboardSettings={onOpenKeyboardSettings}
          onSwitchAccount={onSwitchAccount}
        />
      </aside>
    );
  }

  if (mode === "roomInfo") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.roomInfo")} onClose={onClosePanel} />
        <RoomInfoPanel room={activeRoom} spaces={snapshot.state.spaces} />
      </aside>
    );
  }

  if (mode === "spaceInfo") {
    return (
      <aside className="thread-pane" aria-label={t("panel.context")}>
        <PanelHeader title={t("panel.spaceInfo")} onClose={onClosePanel} />
        <SpaceInfoPanel fallbackName={activeSpaceName} rooms={snapshot.state.rooms} space={activeSpace} />
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
          />
        ) : (
          <div className="thread-root-placeholder">{t("timeline.openingThread")}</div>
        )}
      </section>
      <ThreadComposer
        draft={threadDraft}
        isSending={threadSendPending}
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
  onDraftChange,
  onSend
}: {
  canEdit: boolean;
  draft: string;
  isSending: boolean;
  onDraftChange: (draft: string) => void;
  onSend: () => void;
}) {
  const canSend = canEdit && !isSending && draft.trim().length > 0;

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (canSend) {
        onSend();
      }
    }
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
          <Send size={17} />
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
      <button className="icon-button" type="button" aria-label="More">
        <MoreHorizontal size={19} />
      </button>
      {showClose ? (
        <button className="icon-button" type="button" aria-label={t("action.close", { title })} onClick={onClose}>
          <X size={19} />
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
      return "message";
    case "attachmentFileName":
      return "attachment filename";
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
