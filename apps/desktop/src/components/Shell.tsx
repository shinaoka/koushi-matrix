import {
  type CSSProperties,
  type DragEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useState
} from "react";
import {
  Bell,
  Bug,
  ChevronDown,
  Clock3,
  Compass,
  Edit3,
  HelpCircle,
  Home,
  MessageCircle,
  Plus,
  RefreshCw,
  Search,
  Settings
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  DesktopSnapshot,
  RoomListItem,
  RoomSummary,
  SearchScopeKind
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import { mediaSourceUrl } from "../domain/mediaUrl";
import { Tooltip } from "./Tooltip";
import { useRecoverableImageSource } from "./avatarImage";
import {
  ICON_SIZE,
  syncStatePresentation,
  type OpenContextMenu,
  type PrimaryView,
  initials,
  compactAvatarLabel,
  EMPTY_ROOM_TAGS
} from "../app/uiShared";
import { roomListSections } from "../domain/desktopModel";
import { type SpaceLocalOverrides, spaceDisplayName } from "../app/localPresentation";

const ROOM_SECTION_COLLAPSE_KEY = "koushi.roomSectionCollapsed.v1";
const ROOM_RECENCY_KEY = "koushi.roomRecency.v1";

export function TopBar({
  activeSpaceName,
  isBusy,
  searchInputRef,
  searchQuery,
  searchScope,
  sync,
  onOpenKeyboardSettings,
  onOpenDiagnostics = () => undefined,
  onRestartSync,
  onSearchQueryChange,
  onSearchScopeChange
}: {
  activeSpaceName: string;
  isBusy: boolean;
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchQuery: string;
  searchScope: SearchScopeKind;
  sync: DesktopSnapshot["state"]["domain"]["sync"];
  onOpenKeyboardSettings: () => void;
  onOpenDiagnostics?: () => void;
  onRestartSync: () => void;
  onSearchQueryChange: (value: string) => void;
  onSearchScopeChange: (value: SearchScopeKind) => void;
}) {
  const syncStatus = syncStatePresentation(sync);
  return (
    <header className="titlebar">
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
        <button
          className="icon-button"
          type="button"
          aria-label={t("diagnostics.open")}
          onClick={onOpenDiagnostics}
        >
          <Bug size={ICON_SIZE.control} />
        </button>
      </div>
    </header>
  );
}

export function WorkspaceRail({
  snapshot,
  spaceOverrides = {},
  onCreateSpace,
  onOpenContextMenu,
  onOpenUserSettings,
  onReorderSpaces,
  onSelectSpace
}: {
  snapshot: DesktopSnapshot;
  spaceOverrides?: SpaceLocalOverrides;
  onCreateSpace: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenUserSettings: () => void;
  onReorderSpaces: (spaceIds: string[]) => void;
  onSelectSpace: (spaceId: string | null) => void;
}) {
  const [draggedSpaceId, setDraggedSpaceId] = useState<string | null>(null);
  const [dragOverSpaceId, setDragOverSpaceId] = useState<string | null>(null);
  const spaceIds = snapshot.sidebar.space_rail.map((space) => space.space_id);

  function dropSpaceOn(targetSpaceId: string, event: DragEvent<HTMLButtonElement>) {
    event.preventDefault();
    const sourceSpaceId = draggedSpaceId ?? event.dataTransfer.getData("text/plain");
    setDraggedSpaceId(null);
    setDragOverSpaceId(null);

    if (!sourceSpaceId || sourceSpaceId === targetSpaceId) {
      return;
    }

    const sourceIndex = spaceIds.indexOf(sourceSpaceId);
    const targetIndex = spaceIds.indexOf(targetSpaceId);
    if (sourceIndex < 0 || targetIndex < 0) {
      return;
    }

    const nextSpaceIds = [...spaceIds];
    const [movedSpaceId] = nextSpaceIds.splice(sourceIndex, 1);
    if (!movedSpaceId) {
      return;
    }
    nextSpaceIds.splice(targetIndex, 0, movedSpaceId);
    onReorderSpaces(nextSpaceIds);
  }

  return (
    <nav className="workspace-rail" aria-label={t("workspace.workspaces")}>
      <div className="workspace-rail-main">
        <div className="workspace-list workspace-system-list">
          <Tooltip label={snapshot.sidebar.account_home.display_name}>
            {(tooltipProps) => (
              <button
                className={`workspace-button workspace-system-button workspace-home-button ${
                  snapshot.sidebar.account_home.is_active ? "is-active" : ""
                }`}
                data-count={snapshot.sidebar.account_home.unread_count || undefined}
                type="button"
                aria-label={snapshot.sidebar.account_home.display_name}
                onClick={() => onSelectSpace(null)}
                {...tooltipProps}
              >
                <Home size={ICON_SIZE.rail} />
              </button>
            )}
          </Tooltip>
        </div>
        <div className="workspace-rail-separator" role="separator" aria-orientation="horizontal" />
        <div className="workspace-list workspace-space-list">
          {snapshot.sidebar.space_rail.map((space) => {
            const displayName = spaceDisplayName(
              space.space_id,
              space.display_name,
              spaceOverrides
            );
            const localIcon = spaceOverrides[space.space_id]?.icon?.trim();
            return (
            <Tooltip label={displayName} key={space.space_id}>
              {(tooltipProps) => (
                <button
                  className={`workspace-button workspace-space-button ${
                    space.is_active ? "is-active" : ""
                  }`}
                  data-dragging={draggedSpaceId === space.space_id || undefined}
                  data-drag-over={dragOverSpaceId === space.space_id || undefined}
                  data-count={space.unread_count || undefined}
                  draggable
                  type="button"
                  aria-label={displayName}
                  onClick={() => onSelectSpace(space.space_id)}
                  onDragStart={(event) => {
                    setDraggedSpaceId(space.space_id);
                    event.dataTransfer.effectAllowed = "move";
                    event.dataTransfer.setData("text/plain", space.space_id);
                  }}
                  onDragOver={(event) => {
                    event.preventDefault();
                    event.dataTransfer.dropEffect = "move";
                    setDragOverSpaceId(space.space_id);
                  }}
                  onDragLeave={() => {
                    setDragOverSpaceId((current) =>
                      current === space.space_id ? null : current
                    );
                  }}
                  onDrop={(event) => dropSpaceOn(space.space_id, event)}
                  onDragEnd={() => {
                    setDraggedSpaceId(null);
                    setDragOverSpaceId(null);
                  }}
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
                    fallback={localIcon || compactAvatarLabel(displayName)}
                    fallbackMode="compactLabel"
                  />
                </button>
              )}
            </Tooltip>
          );
          })}
        </div>
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

export function Sidebar({
  activeRoomId,
  activeView,
  snapshot,
  spaceOverrides = {},
  onCreateRoom,
  onNewDm,
  onOpenContextMenu,
  onOpenActivity,
  onOpenExplore,
  onOpenHome,
  onOpenInvites,
  onOpenSpaceInfo,
  onOpenThreads,
  onSelectRoom
}: {
  activeRoomId: string | null;
  activeView: PrimaryView;
  snapshot: DesktopSnapshot;
  spaceOverrides?: SpaceLocalOverrides;
  onCreateRoom: () => void;
  onNewDm: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenActivity: () => void;
  onOpenExplore: () => void;
  onOpenHome: () => void;
  onOpenInvites: () => void;
  onOpenSpaceInfo: () => void;
  onOpenThreads: () => void;
  onSelectRoom: (roomId: string) => void;
}) {
  const sections = roomListSections(
    snapshot.state.ui.room_list,
    snapshot.state.ui.navigation.active_space_id,
    snapshot.state.domain.spaces,
    snapshot.state.domain.rooms,
    snapshot.state.domain.invites
  );
  const threadAttention =
    snapshot.state.domain.thread_attention.kind === "tracking"
      ? snapshot.state.domain.thread_attention
      : null;
  const [collapsedSections, setCollapsedSections] = useState<Record<string, boolean>>(
    readCollapsedSections
  );
  const [recentRoomIds, setRecentRoomIds] = useState<string[]>(readRecentRoomIds);
  const activeSpace = snapshot.sidebar.space_rail.find((space) => space.is_active);
  const activeSpaceName = activeSpace
    ? spaceDisplayName(activeSpace.space_id, activeSpace.display_name, spaceOverrides)
    : snapshot.sidebar.account_home.display_name;
  const accountHomeActive = snapshot.sidebar.account_home.is_active && !activeSpace;
  const roomById = new Map(snapshot.state.domain.rooms.map((room) => [room.room_id, room]));
  const presence = snapshot.state.domain.live_signals.presence;
  const rooms = sortRoomsByRecency(
    uniqueRooms([...sections.favourites, ...sections.rooms, ...sections.lowPriority]),
    recentRoomIds
  );
  const dms = sortRoomsByRecency(sections.people, recentRoomIds);

  useEffect(() => {
    if (!activeRoomId) {
      return;
    }
    setRecentRoomIds((current) => rememberRecentRoomId(current, activeRoomId));
  }, [activeRoomId]);

  function toggleSection(sectionId: string) {
    setCollapsedSections((current) => {
      const next = { ...current, [sectionId]: !current[sectionId] };
      writeCollapsedSections(next);
      return next;
    });
  }

  function selectRoom(roomId: string) {
    setRecentRoomIds((current) => rememberRecentRoomId(current, roomId));
    onSelectRoom(roomId);
  }

  return (
    <aside className="sidebar" aria-label={t("workspace.rooms")}>
      <div className="workspace-header">
        <div className="workspace-name" dir="auto">
          {activeSpaceName}
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
      <div className="sidebar-scroll">
        {accountHomeActive ? (
          <NavButton
            active={activeView === "activity"}
            icon={<Clock3 size={ICON_SIZE.control} />}
            label={t("workspace.activity")}
            onClick={onOpenActivity}
          />
        ) : (
          <>
            <NavButton
              active={activeView === "timeline" && snapshot.sidebar.account_home.is_active}
              icon={<Home size={ICON_SIZE.control} />}
              label={t("workspace.home")}
              onClick={onOpenHome}
            />
            <NavButton
              count={threadAttention?.notification_count ?? 0}
              icon={<MessageCircle size={ICON_SIZE.control} />}
              label={t("workspace.threads")}
              liveCount={threadAttention?.live_event_marker_count ?? 0}
              mentionCount={threadAttention?.highlight_count ?? 0}
              onClick={onOpenThreads}
            />
          </>
        )}
        <NavButton
          active={activeView === "explore"}
          icon={<Compass size={ICON_SIZE.control} />}
          label={t("workspace.explore")}
          onClick={onOpenExplore}
        />
        <NavButton
          active={activeView === "invites"}
          count={snapshot.state.domain.invites.length}
          icon={<Bell size={ICON_SIZE.control} />}
          label={t("workspace.invites")}
          onClick={onOpenInvites}
        />
        {!accountHomeActive ? (
          <RoomSection
            activeRoomId={activeRoomId}
            collapsed={Boolean(collapsedSections.rooms)}
            id="rooms"
            kind="room"
            label={t("workspace.rooms")}
            presence={presence}
            roomById={roomById}
            rooms={rooms}
            showWhenEmpty={true}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={selectRoom}
            onToggleCollapsed={() => toggleSection("rooms")}
          />
        ) : null}
        <RoomSection
          activeRoomId={activeRoomId}
          collapsed={Boolean(collapsedSections.people)}
          id="people"
          kind="dm"
          label={t("workspace.people")}
          presence={presence}
          roomById={roomById}
          rooms={dms}
          showWhenEmpty={true}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={selectRoom}
          onToggleCollapsed={() => toggleSection("people")}
        />
      </div>
    </aside>
  );
}

function RoomSection({
  activeRoomId,
  collapsed,
  id,
  kind,
  label,
  presence,
  roomById,
  rooms,
  showWhenEmpty = false,
  onOpenContextMenu,
  onSelectInvite,
  onSelectRoom,
  onToggleCollapsed
}: {
  activeRoomId: string | null;
  collapsed: boolean;
  id: string;
  kind: "room" | "dm" | "invite";
  label: string;
  presence: DesktopSnapshot["state"]["domain"]["live_signals"]["presence"];
  roomById: Map<string, RoomSummary>;
  rooms: RoomListItem[];
  showWhenEmpty?: boolean;
  onOpenContextMenu: OpenContextMenu;
  onSelectInvite?: () => void;
  onSelectRoom: (roomId: string) => void;
  onToggleCollapsed: () => void;
}) {
  if (!showWhenEmpty && rooms.length === 0) {
    return null;
  }

  return (
    <section className="room-section" data-room-section={id} aria-label={label}>
      <SectionTitle
        collapsed={collapsed}
        count={rooms.length}
        label={label}
        onToggle={onToggleCollapsed}
      />
      {!collapsed
        ? rooms.map((room) => (
            <RoomButton
              activeRoomId={activeRoomId}
              kind={kind}
              presence={presence}
              roomById={roomById}
              key={room.room_id}
              room={room}
              onOpenContextMenu={onOpenContextMenu}
              onSelectInvite={onSelectInvite}
              onSelectRoom={onSelectRoom}
            />
          ))
        : null}
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

function SectionTitle({
  collapsed,
  count,
  label,
  onToggle
}: {
  collapsed: boolean;
  count: number;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      className="section-title"
      type="button"
      aria-expanded={!collapsed}
      onClick={onToggle}
    >
      <span className="section-title-label">{label}</span>
      <span className="section-title-meta">
        <span className="section-count">{count}</span>
        <ChevronDown size={ICON_SIZE.compact} aria-hidden="true" />
      </span>
    </button>
  );
}

function RoomButton({
  activeRoomId,
  kind,
  presence,
  roomById,
  room,
  onOpenContextMenu,
  onSelectInvite,
  onSelectRoom
}: {
  activeRoomId: string | null;
  kind: "room" | "dm" | "invite";
  presence: DesktopSnapshot["state"]["domain"]["live_signals"]["presence"];
  roomById: Map<string, RoomSummary>;
  room: RoomListItem;
  onOpenContextMenu: OpenContextMenu;
  onSelectInvite?: () => void;
  onSelectRoom: (roomId: string) => void;
}) {
  const sourceRoom = roomById.get(room.room_id);
  const dmUserId =
    kind === "dm" && sourceRoom?.is_dm && sourceRoom.dm_user_ids.length === 1
      ? sourceRoom.dm_user_ids[0]
      : null;
  const isOnlineDm = dmUserId ? presence[dmUserId] === "online" : false;
  return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      aria-label={room.display_name}
      data-room-kind={kind}
      data-testid="room-item"
      type="button"
      onClick={() => {
        if (kind === "invite") {
          onSelectInvite?.();
          return;
        }
        onSelectRoom(room.room_id);
      }}
      onContextMenu={(event) => {
        if (kind === "invite") {
          event.preventDefault();
          return;
        }
        onOpenContextMenu(
          event,
          { kind: "room", roomId: room.room_id },
          contextMenuItems({
            kind: "room",
            roomId: room.room_id,
            tags: room.tags ?? EMPTY_ROOM_TAGS
          })
        );
      }}
    >
      <span className="room-avatar-shell">
        <EntityAvatar
          avatar={room.avatar}
          className={`room-avatar ${kind === "dm" ? "is-user" : "is-room"}`}
          fallback={initials(room.display_name)}
        />
        {isOnlineDm ? <span className="room-presence-dot" aria-hidden="true" /> : null}
      </span>
      <span className="room-name" dir="auto">{room.display_name}</span>
      <span className="room-trailing">
        <span className="room-count">{room.unread_count || ""}</span>
      </span>
    </button>
  );
}

export function EntityAvatar({
  avatar,
  className,
  fallback,
  fallbackMode = "initials"
}: {
  avatar: RoomListItem["avatar"];
  className: string;
  fallback: string;
  fallbackMode?: "initials" | "compactLabel";
}) {
  const sourceUrl =
    avatar?.thumbnail.kind === "ready" ? mediaSourceUrl(avatar.thumbnail.source_url) : null;
  const { displaySourceUrl, onImageError, onImageLoad } = useRecoverableImageSource(sourceUrl);
  const showImage = Boolean(displaySourceUrl);
  const fallbackClassName =
    fallbackMode === "compactLabel" ? "avatar-fallback compact-label" : "avatar-fallback";
  const fallbackStyle =
    fallbackMode === "compactLabel"
      ? ({
          "--avatar-label-length": Math.max(fallback.length, 1)
        } as CSSProperties)
      : undefined;
  return (
    <span className={className} aria-hidden="true">
      {showImage ? (
        <img
          src={displaySourceUrl ?? undefined}
          onError={onImageError}
          onLoad={onImageLoad}
        />
      ) : (
        <span className={fallbackClassName} dir="auto" style={fallbackStyle}>
          {fallback}
        </span>
      )}
    </span>
  );
}

function readJsonRecord<T>(key: string, fallback: T): T {
  if (typeof window === "undefined") {
    return fallback;
  }
  try {
    return JSON.parse(window.localStorage.getItem(key) ?? "") as T;
  } catch {
    return fallback;
  }
}

function readCollapsedSections(): Record<string, boolean> {
  return readJsonRecord<Record<string, boolean>>(ROOM_SECTION_COLLAPSE_KEY, {});
}

function writeCollapsedSections(collapsedSections: Record<string, boolean>): void {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(ROOM_SECTION_COLLAPSE_KEY, JSON.stringify(collapsedSections));
}

function readRecentRoomIds(): string[] {
  const value = readJsonRecord<unknown>(ROOM_RECENCY_KEY, []);
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function writeRecentRoomIds(roomIds: string[]): void {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(ROOM_RECENCY_KEY, JSON.stringify(roomIds));
}

function rememberRecentRoomId(current: string[], roomId: string): string[] {
  const next = [roomId, ...current.filter((id) => id !== roomId)].slice(0, 200);
  writeRecentRoomIds(next);
  return next;
}

function uniqueRooms(rooms: RoomListItem[]): RoomListItem[] {
  const seen = new Set<string>();
  return rooms.filter((room) => {
    if (seen.has(room.room_id)) {
      return false;
    }
    seen.add(room.room_id);
    return true;
  });
}

function sortRoomsByRecency(rooms: RoomListItem[], recentRoomIds: string[]): RoomListItem[] {
  const recentIndex = new Map(recentRoomIds.map((roomId, index) => [roomId, index]));
  return [...rooms].sort((left, right) => {
    const leftRecent = recentIndex.get(left.room_id);
    const rightRecent = recentIndex.get(right.room_id);
    if (leftRecent !== undefined || rightRecent !== undefined) {
      return (leftRecent ?? Number.MAX_SAFE_INTEGER) - (rightRecent ?? Number.MAX_SAFE_INTEGER);
    }
    return (
      right.highlight_count - left.highlight_count ||
      right.unread_count - left.unread_count ||
      left.display_name.localeCompare(right.display_name)
    );
  });
}
