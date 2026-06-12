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
import { listen } from "@tauri-apps/api/event";

import { createDesktopApi } from "./backend/client";
import { ContextMenuSurface } from "./components/ContextMenuSurface";
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
import { qaWindowTitle } from "./domain/qaTitle";
import type {
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
  const [savedSessions, setSavedSessions] = useState<SavedSessionInfo[]>([]);
  const [contextMenu, setContextMenu] = useState<ActiveContextMenu | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const searchTimer = useRef<number | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const loginPasswordRef = useRef<HTMLInputElement>(null);
  const recoverySecretRef = useRef<HTMLInputElement>(null);

  function handleShortcutAction(shortcutId: string): boolean {
    switch (shortcutId) {
      case "showKeyboardSettings":
        setRightPanelMode("keyboardSettings");
        return true;
      case "openUserSettings":
        setRightPanelMode("userSettings");
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
        setRightPanelMode((mode) => (mode === "closed" ? "roomInfo" : "closed"));
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
    if (!qaTitleEnabled()) {
      return;
    }
    document.title = snapshot ? qaWindowTitle(snapshot) : "matrix-desktop qa session=booting";
  }, [snapshot]);

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

  async function selectSpace(spaceId: string | null) {
    setSnapshot(await api.selectSpace(spaceId));
  }

  async function selectRoom(roomId: string) {
    setSnapshot(await api.selectRoom(roomId));
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

    setSnapshot(await api.sendText(roomId, body));
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
    setSnapshot(await api.openThread(roomId, rootEventId));
    setRightPanelMode("thread");
  }

  async function closeThread() {
    setSnapshot(await api.closeThread());
    setRightPanelMode("closed");
  }

  function selectSearchResult(roomId: string, eventId: string) {
    void selectRoom(roomId).then(() => {
      setSearchQuery("");
      setRightPanelMode("closed");
      requestAnimationFrame(() => {
        document.querySelector(`[data-event-id="${cssEscape(eventId)}"]`)?.scrollIntoView({
          block: "center"
        });
      });
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

    const applyIntentMode = () => {
      if (intent.mode) {
        setRightPanelMode(intent.mode);
      }
      if (intent.focusSearch) {
        setSearchScope("currentRoom");
        searchInputRef.current?.focus();
      }
    };

    if (intent.selectRoomId) {
      void selectRoom(intent.selectRoomId).then(applyIntentMode);
      return;
    }
    if (intent.selectSpaceId) {
      void selectSpace(intent.selectSpaceId).then(applyIntentMode);
      return;
    }
    applyIntentMode();
    if (actionId === "switchAccount") {
      void refreshSavedSessions();
    }
  }

  async function runSearch(query: string, scope: SearchScopeKind) {
    const trimmed = query.trim();
    const searchMode = rightPanelModeForSearchQuery(trimmed);
    if (!trimmed) {
      setSnapshot(await api.getSnapshot());
      setRightPanelMode((mode) => (mode === "search" ? "closed" : mode));
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
        onOpenKeyboardSettings={() => setRightPanelMode("keyboardSettings")}
        onSearchQueryChange={setSearchQuery}
        onSearchScopeChange={setSearchScope}
      />
      <div className={`app-grid ${rightPanelOpen ? "right-panel-open" : "thread-closed"}`}>
        <WorkspaceRail
          snapshot={snapshot}
          onOpenContextMenu={openContextMenu}
          onOpenUserSettings={() => setRightPanelMode("userSettings")}
          onSelectSpace={selectSpace}
        />
        <Sidebar
          activeRoomId={snapshot.state.navigation.active_room_id}
          snapshot={snapshot}
          onOpenContextMenu={openContextMenu}
          onOpenSpaceInfo={() => setRightPanelMode("spaceInfo")}
          onSelectRoom={selectRoom}
        />
        <TimelinePane
          activeRoomName={activeRoom?.display_name ?? "No room"}
          composerDraft={composerDraft}
          searchQuery={searchQuery}
          searchResults={searchResults}
          showSearchResults={effectiveRightPanelMode !== "search"}
          snapshot={snapshot}
          onComposerDraftChange={setComposerDraft}
          onOpenThread={openThread}
          onPaginateBackwards={paginateTimelineBackwards}
          onSendText={sendText}
          onEditMessage={editMessage}
          onOpenContextMenu={openContextMenu}
          onRedactMessage={redactMessage}
          onResultSelect={selectSearchResult}
          onToggleThread={() => {
            if (rightPanelOpen) {
              void closeThread();
            } else {
              const messageWithReplies = snapshot.timeline.find((message) => message.reply_count > 0);
              if (messageWithReplies) {
                void openThread(messageWithReplies.room_id, messageWithReplies.event_id);
              } else {
                setRightPanelMode("roomInfo");
              }
            }
          }}
          onOpenRoomInfo={() => setRightPanelMode("roomInfo")}
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
          searchQuery={searchQuery}
          searchResults={searchResults}
          savedSessions={savedSessions}
          onCloseThread={() => {
            void closeThread();
          }}
          onClosePanel={() => setRightPanelMode("closed")}
          onOpenKeyboardSettings={() => setRightPanelMode("keyboardSettings")}
          onRecoverySecretPresenceChange={setRecoverySecretFilled}
          onResultSelect={selectSearchResult}
          onSubmitRecovery={submitRecovery}
          onSwitchAccount={(session) => {
            void switchAccount(session);
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
          <span>Homeserver</span>
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

function recoveryMethodLabel(method: NonNullable<DesktopSnapshot["state"]["session"]["recovery_methods"]>[number]) {
  switch (method) {
    case "recoveryKey":
      return "Recovery key";
    case "securityPhrase":
      return "Security phrase";
  }
}

function TopBar({
  activeSpaceName,
  isBusy,
  searchInputRef,
  searchQuery,
  searchScope,
  onOpenKeyboardSettings,
  onSearchQueryChange,
  onSearchScopeChange
}: {
  activeSpaceName: string;
  isBusy: boolean;
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchQuery: string;
  searchScope: SearchScopeKind;
  onOpenKeyboardSettings: () => void;
  onSearchQueryChange: (value: string) => void;
  onSearchScopeChange: (value: SearchScopeKind) => void;
}) {
  return (
    <header className="titlebar">
      <div className="traffic">
        <span className="dot red" />
        <span className="dot yellow" />
        <span className="dot green" />
      </div>
      <div className="history">
        <button className="icon-button" type="button" aria-label="Back">
          ‹
        </button>
        <button className="icon-button" type="button" aria-label="Forward">
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
          value={searchQuery}
          placeholder={`${activeSpaceName} 内を検索する`}
          onChange={(event) => onSearchQueryChange(event.target.value)}
        />
      </label>
      <select
        className="scope-select"
        aria-label="Search scope"
        value={searchScope}
        onChange={(event) => onSearchScopeChange(event.target.value as SearchScopeKind)}
      >
        <option value="allRooms">All</option>
        <option value="currentSpace">Space</option>
        <option value="currentRoom">Room</option>
        <option value="dms">DM</option>
      </select>
      <div className="top-actions">
        <span className={`sync-dot ${isBusy ? "busy" : ""}`} />
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

function WorkspaceRail({
  snapshot,
  onOpenContextMenu,
  onOpenUserSettings,
  onSelectSpace
}: {
  snapshot: DesktopSnapshot;
  onOpenContextMenu: OpenContextMenu;
  onOpenUserSettings: () => void;
  onSelectSpace: (spaceId: string | null) => void;
}) {
  return (
    <nav className="workspace-rail" aria-label="Workspaces">
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
        <button className="rail-action" type="button" aria-label="Add workspace">
          <Plus size={22} />
        </button>
        <button
          className="user-presence"
          type="button"
          aria-label="User settings"
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
  onOpenContextMenu,
  onOpenSpaceInfo,
  onSelectRoom
}: {
  activeRoomId: string | null;
  snapshot: DesktopSnapshot;
  onOpenContextMenu: OpenContextMenu;
  onOpenSpaceInfo: () => void;
  onSelectRoom: (roomId: string) => void;
}) {
  return (
    <aside className="sidebar">
      <div className="workspace-header">
        <div className="workspace-name">
          {snapshot.sidebar.space_rail.find((space) => space.is_active)?.display_name ??
            snapshot.sidebar.account_home.display_name}
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Space info and settings"
          onClick={onOpenSpaceInfo}
        >
          <Settings size={18} />
        </button>
        <button className="icon-button" type="button" aria-label="New message">
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
        <SectionTitle label="Rooms" />
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
  searchQuery,
  searchResults,
  showSearchResults,
  snapshot,
  onComposerDraftChange,
  onEditMessage,
  onOpenContextMenu,
  onOpenThread,
  onPaginateBackwards,
  onRedactMessage,
  onResultSelect,
  onSendText,
  onToggleThread,
  onOpenRoomInfo
}: {
  activeRoomName: string;
  composerDraft: string;
  searchQuery: string;
  searchResults: SearchResult[];
  showSearchResults: boolean;
  snapshot: DesktopSnapshot;
  onComposerDraftChange: (value: string) => void;
  onEditMessage: (message: TimelineMessage) => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenThread: (roomId: string, rootEventId: string) => void;
  onPaginateBackwards: (roomId: string) => void;
  onRedactMessage: (roomId: string, eventId: string) => void;
  onResultSelect: (roomId: string, eventId: string) => void;
  onSendText: () => void;
  onToggleThread: () => void;
  onOpenRoomInfo: () => void;
}) {
  const timelineRoomId = snapshot.state.timeline.room_id;
  const currentUserId = snapshot.state.session.user_id ?? null;

  return (
    <main className="main-pane">
      <header className="channel-header">
        <div className="channel-title">
          <Hash size={22} />
          <span>{activeRoomName}</span>
        </div>
        <div className="channel-actions">
          <button className="member-pill" type="button" aria-label="Members">
            <Users size={16} />
            <span>8</span>
          </button>
          <button className="icon-button" type="button" aria-label="Toggle thread" onClick={onToggleThread}>
            {snapshot.thread ? <PanelRightClose size={19} /> : <PanelRightOpen size={19} />}
          </button>
          <button className="icon-button" type="button" aria-label="Room info" onClick={onOpenRoomInfo}>
            <MoreVertical size={19} />
          </button>
        </div>
      </header>
      <nav className="tabs" aria-label="Room tabs">
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
                  ? "読み込み中"
                  : "以前のメッセージ"}
              </span>
            </button>
          </div>
          {snapshot.timeline.map((message) => (
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
          ))}
        </div>
      </section>
      <Composer
        roomName={activeRoomName}
        value={composerDraft}
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
                aria-label="Edit message"
                onClick={() => onEditMessage(message)}
              >
                <Edit3 size={14} />
              </button>
              <button
                className="message-action"
                type="button"
                aria-label="Redact message"
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
            新しい返信を確認する · {message.reply_count}
          </button>
        ) : null}
      </div>
    </article>
  );
}

function Composer({
  roomName,
  value,
  onSend,
  onValueChange
}: {
  roomName: string;
  value: string;
  onSend: () => void;
  onValueChange: (value: string) => void;
}) {
  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      onSend();
    }
  }

  return (
    <section className="composer" aria-label="Message composer">
      <div className="composer-tools">
        <button className="icon-button" type="button" aria-label="Bold">
          <Bold size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="Italic">
          <Italic size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="Link">
          <Link2 size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="List">
          <List size={17} />
        </button>
        <button className="icon-button" type="button" aria-label="Code">
          <Code2 size={17} />
        </button>
      </div>
      <textarea
        value={value}
        placeholder={`${roomName} へのメッセージ`}
        onKeyDown={onComposerKeyDown}
        onChange={(event) => onValueChange(event.target.value)}
      />
      <div className="composer-footer">
        <div>
          <button className="icon-button" type="button" aria-label="Add">
            <Plus size={19} />
          </button>
          <button className="icon-button" type="button" aria-label="Mention">
            <AtSign size={18} />
          </button>
          <button className="icon-button" type="button" aria-label="Emoji">
            <Smile size={18} />
          </button>
        </div>
        <button
          className={`send-button ${value.trim() ? "ready" : ""}`}
          type="button"
          aria-label="Send"
          disabled={!value.trim()}
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
  searchQuery,
  searchResults,
  savedSessions,
  onCloseThread,
  onClosePanel,
  onOpenKeyboardSettings,
  onRecoverySecretPresenceChange,
  onResultSelect,
  onSubmitRecovery,
  onSwitchAccount
}: {
  activeRoom: DesktopSnapshot["state"]["rooms"][number] | null;
  activeSpace: DesktopSnapshot["state"]["spaces"][number] | null;
  activeSpaceName: string;
  isRecoveryBusy: boolean;
  mode: RightPanelMode;
  recoverySecretFilled: boolean;
  recoverySecretInputRef: RefObject<HTMLInputElement | null>;
  snapshot: DesktopSnapshot;
  searchQuery: string;
  searchResults: SearchResult[];
  savedSessions: SavedSessionInfo[];
  onCloseThread: () => void;
  onClosePanel: () => void;
  onOpenKeyboardSettings: () => void;
  onRecoverySecretPresenceChange: (value: boolean) => void;
  onResultSelect: (roomId: string, eventId: string) => void;
  onSubmitRecovery: (event: FormEvent<HTMLFormElement>) => void;
  onSwitchAccount: (session: SavedSessionInfo) => void;
}) {
  if (mode === "closed") {
    return <aside className="thread-pane" />;
  }

  if (mode === "recovery") {
    return (
      <aside className="thread-pane">
        <PanelHeader title="Recovery" onClose={onClosePanel} showClose={false} />
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
      <aside className="thread-pane">
        <PanelHeader title="Keyboard" onClose={onClosePanel} />
        <KeyboardSettingsPanel />
      </aside>
    );
  }

  if (mode === "userSettings") {
    return (
      <aside className="thread-pane">
        <PanelHeader title="User settings" onClose={onClosePanel} />
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
      <aside className="thread-pane">
        <PanelHeader title="Room info" onClose={onClosePanel} />
        <RoomInfoPanel room={activeRoom} spaces={snapshot.state.spaces} />
      </aside>
    );
  }

  if (mode === "spaceInfo") {
    return (
      <aside className="thread-pane">
        <PanelHeader title="Space info" onClose={onClosePanel} />
        <SpaceInfoPanel fallbackName={activeSpaceName} rooms={snapshot.state.rooms} space={activeSpace} />
      </aside>
    );
  }

  if (mode === "search") {
    return (
      <aside className="thread-pane">
        <PanelHeader title="Search" onClose={onClosePanel} />
        <SearchResults
          query={searchQuery}
          results={searchResults}
          rooms={snapshot.state.rooms}
          onResultSelect={onResultSelect}
        />
      </aside>
    );
  }

  if (!snapshot.thread) {
    return <aside className="thread-pane" />;
  }

  const root = snapshot.timeline.find((message) => message.event_id === snapshot.thread?.root_event_id);

  return (
    <aside className="thread-pane">
      <PanelHeader title="Thread" onClose={onCloseThread} />
      <section className="thread-scroll">
        {root ? (
          <div className="thread-root">
            <MessageArticle
              currentUserId={null}
              message={root}
              query={searchQuery}
              onEditMessage={() => undefined}
              onOpenThread={() => undefined}
              onRedactMessage={() => undefined}
            />
          </div>
        ) : null}
        {snapshot.thread.replies.map((reply) => (
          <article className="thread-reply" key={reply.event_id}>
            <div className="avatar" aria-hidden="true">
              {initials(reply.sender)}
            </div>
            <div className="message-main">
              <div className="message-heading">
                <span className="sender">{reply.sender}</span>
                <span className="time">{formatTime(reply.timestamp_ms)}</span>
              </div>
              <div className="message-body">{reply.body}</div>
            </div>
          </article>
        ))}
      </section>
      <section className="thread-composer" aria-label="Thread composer">
        <textarea placeholder="返信する..." />
      </section>
    </aside>
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
        <button className="icon-button" type="button" aria-label={`Close ${title}`} onClick={onClose}>
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

function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
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
