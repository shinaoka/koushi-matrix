import {
  type CSSProperties,
  type DragEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
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
  DisplayPlatform,
  RoomListItem,
  RoomSummary,
  SearchScopeKind
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import { mediaSourceUrl } from "../domain/mediaUrl";
import { Tooltip } from "./Tooltip";
import { ImeTextField } from "./ImeTextControl";
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
import {
  readSidebarRoomCategory,
  readSidebarRoomSort,
  type SidebarRoomCategory,
  type SidebarRoomSort,
  type SpaceLocalOverrides,
  spaceDisplayName,
  writeSidebarRoomCategory,
  writeSidebarRoomSort
} from "../app/localPresentation";

const ROOM_SECTION_COLLAPSE_KEY = "koushi.roomSectionCollapsed.v1";

function sortedSidebarRooms(
  rooms: RoomListItem[],
  sort: SidebarRoomSort
): RoomListItem[] {
  if (sort === "active") {
    return rooms;
  }
  return [...rooms].sort((left, right) => {
    const nameOrder = left.display_name.localeCompare(right.display_name, undefined, {
      numeric: true,
      sensitivity: "base"
    });
    return nameOrder || left.room_id.localeCompare(right.room_id);
  });
}

function shouldStartTitlebarDrag(event: MouseEvent<HTMLElement>): boolean {
  if (event.buttons !== 1 || !(event.target instanceof Element)) {
    return false;
  }
  return !event.target.closest("button, input, select, textarea, a, label");
}

export function TopBar({
  activeSpaceName,
  homeserver,
  isBusy,
  platform = "linux",
  searchInputRef,
  searchQuery,
  searchScope,
  sync,
  onOpenKeyboardSettings,
  onOpenDiagnostics = () => undefined,
  onRestartSync,
  onSearchQueryChange,
  onSearchScopeChange,
  onStartWindowDrag = () => undefined
}: {
  activeSpaceName: string;
  homeserver?: string | null;
  isBusy: boolean;
  platform?: DisplayPlatform;
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchQuery: string;
  searchScope: SearchScopeKind;
  sync: DesktopSnapshot["state"]["domain"]["sync"];
  onOpenKeyboardSettings: () => void;
  onOpenDiagnostics?: () => void;
  onRestartSync: () => void;
  onSearchQueryChange: (value: string) => void;
  onSearchScopeChange: (value: SearchScopeKind) => void;
  onStartWindowDrag?: () => void;
}) {
  const syncStatus = syncStatePresentation(sync);
  const serverLabel = matrixServerLabel(homeserver);
  const syncAriaLabel = serverLabel
    ? `${serverLabel} · ${syncStatus.ariaLabel}`
    : syncStatus.ariaLabel;
  return (
    <header
      className="titlebar"
      data-platform={platform}
      data-tauri-drag-region=""
      onMouseDown={(event) => {
        if (!shouldStartTitlebarDrag(event)) {
          return;
        }
        event.preventDefault();
        onStartWindowDrag();
      }}
    >
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
        <ImeTextField
          ref={searchInputRef}
          aria-label={t("workspace.search")}
          value={searchQuery}
          syncKey="workspace-search"
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
          aria-label={syncAriaLabel}
        >
          <span className={`sync-dot ${isBusy ? "busy" : ""}`} />
          {serverLabel ? <span className="sync-status-server">{serverLabel}</span> : null}
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

function matrixServerLabel(homeserver: string | null | undefined): string | null {
  const trimmed = homeserver?.trim();
  if (!trimmed) {
    return null;
  }
  try {
    return new URL(trimmed).host || trimmed;
  } catch {
    return trimmed.replace(/^https?:\/\//i, "").replace(/\/.*$/, "") || trimmed;
  }
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
                    colorSeed={space.space_id}
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
  onJoinRoom,
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
  onJoinRoom?: (roomId: string) => void;
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
  const activeSpace = snapshot.sidebar.space_rail.find((space) => space.is_active);
  const activeSpaceName = activeSpace
    ? spaceDisplayName(activeSpace.space_id, activeSpace.display_name, spaceOverrides)
    : snapshot.sidebar.account_home.display_name;
  const accountHomeActive = snapshot.sidebar.account_home.is_active && !activeSpace;
  const roomById = new Map(snapshot.state.domain.rooms.map((room) => [room.room_id, room]));
  const presence = snapshot.state.domain.live_signals.presence;
  const [roomCategory, setRoomCategory] = useState<SidebarRoomCategory>(readSidebarRoomCategory);
  const [roomSort, setRoomSort] = useState<SidebarRoomSort>(readSidebarRoomSort);
  const roomCategoryRooms = roomCategory === "dms" ? snapshot.sidebar.global_dms : sections.rooms;
  const visibleCategoryRooms = sortedSidebarRooms(roomCategoryRooms, roomSort);
  const visibleNotJoinedRooms =
    accountHomeActive || roomCategory !== "rooms"
      ? []
      : sortedSidebarRooms(sections.notJoined, roomSort);
  const visibleCategoryLabel =
    roomCategory === "dms" ? t("workspace.people") : t("workspace.rooms");
  const visibleCategoryKind = roomCategory === "dms" ? "dm" : "room";
  const visibleCategoryId = roomCategory === "dms" ? "people" : "rooms";

  function selectRoomCategory(category: SidebarRoomCategory) {
    setRoomCategory(category);
    writeSidebarRoomCategory(category);
  }

  function selectRoomSort(sort: SidebarRoomSort) {
    setRoomSort(sort);
    writeSidebarRoomSort(sort);
  }

  function toggleSection(sectionId: string) {
    setCollapsedSections((current) => {
      const next = { ...current, [sectionId]: !current[sectionId] };
      writeCollapsedSections(next);
      return next;
    });
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
        <RoomListControls
          dmTotal={snapshot.sidebar.global_dms.length}
          dmUnread={snapshot.sidebar.dm_unread_count}
          dmHighlights={snapshot.sidebar.dm_highlight_count}
          roomTotal={snapshot.sidebar.space_rooms.length}
          roomUnread={snapshot.sidebar.space_unread_count}
          roomHighlights={snapshot.sidebar.space_highlight_count}
          selectedCategory={roomCategory}
          selectedSort={roomSort}
          onSelectCategory={selectRoomCategory}
          onSelectSort={selectRoomSort}
        />
        {visibleNotJoinedRooms.length > 0 ? (
          <RoomSection
            activeRoomId={activeRoomId}
            collapsed={Boolean(collapsedSections["not-joined"])}
            id="not-joined"
            kind="notJoined"
            label={t("workspace.notJoined")}
            presence={presence}
            roomById={roomById}
            rooms={visibleNotJoinedRooms}
            onJoinRoom={onJoinRoom}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
            onToggleCollapsed={() => toggleSection("not-joined")}
          />
        ) : null}
        <RoomSection
          activeRoomId={activeRoomId}
          collapsed={false}
          id={visibleCategoryId}
          kind={visibleCategoryKind}
          label={visibleCategoryLabel}
          presence={presence}
          roomById={roomById}
          rooms={visibleCategoryRooms}
          showHeader={false}
          showWhenEmpty={true}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
        />
        {!accountHomeActive ? (
          <RoomSection
            activeRoomId={activeRoomId}
            collapsed={Boolean(collapsedSections.favourites)}
            id="favourites"
            kind="room"
            label={t("roomList.filterFavourites")}
            presence={presence}
            roomById={roomById}
            rooms={sections.favourites}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
            onToggleCollapsed={() => toggleSection("favourites")}
          />
        ) : null}
        {!accountHomeActive ? (
          <RoomSection
            activeRoomId={activeRoomId}
            collapsed={Boolean(collapsedSections["low-priority"])}
            id="low-priority"
            kind="room"
            label={t("workspace.lowPriority")}
            presence={presence}
            roomById={roomById}
            rooms={sections.lowPriority}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
            onToggleCollapsed={() => toggleSection("low-priority")}
          />
        ) : null}
      </div>
    </aside>
  );
}

function RoomListControls({
  dmTotal,
  dmUnread,
  dmHighlights,
  roomTotal,
  roomUnread,
  roomHighlights,
  selectedCategory,
  selectedSort,
  onSelectCategory,
  onSelectSort
}: {
  dmTotal: number;
  dmUnread: number;
  dmHighlights: number;
  roomTotal: number;
  roomUnread: number;
  roomHighlights: number;
  selectedCategory: SidebarRoomCategory;
  selectedSort: SidebarRoomSort;
  onSelectCategory: (category: SidebarRoomCategory) => void;
  onSelectSort: (sort: SidebarRoomSort) => void;
}) {
  return (
    <div className="room-list-controls">
      <div className="room-list-category" role="group" aria-label={t("roomList.category")}>
        <button
          className={`room-list-chip ${selectedCategory === "dms" ? "is-selected" : ""}`}
          type="button"
          aria-label={roomListCategoryAccessibleLabel(
            t("roomList.categoryDms"),
            dmTotal,
            dmUnread,
            dmHighlights
          )}
          aria-pressed={selectedCategory === "dms"}
          onClick={() => onSelectCategory("dms")}
        >
          <span>{t("roomList.categoryDms")}</span>
          <span className="room-list-chip-total" aria-hidden="true">{dmTotal}</span>
          {dmUnread > 0 ? (
            <span
              className={`room-list-chip-unread ${dmHighlights > 0 ? "is-highlight" : ""}`}
              aria-hidden="true"
            >
              {compactAttentionCount(dmUnread)}
            </span>
          ) : null}
        </button>
        <button
          className={`room-list-chip ${selectedCategory === "rooms" ? "is-selected" : ""}`}
          type="button"
          aria-label={roomListCategoryAccessibleLabel(
            t("roomList.categoryRooms"),
            roomTotal,
            roomUnread,
            roomHighlights
          )}
          aria-pressed={selectedCategory === "rooms"}
          onClick={() => onSelectCategory("rooms")}
        >
          <span>{t("roomList.categoryRooms")}</span>
          <span className="room-list-chip-total" aria-hidden="true">{roomTotal}</span>
          {roomUnread > 0 ? (
            <span
              className={`room-list-chip-unread ${roomHighlights > 0 ? "is-highlight" : ""}`}
              aria-hidden="true"
            >
              {compactAttentionCount(roomUnread)}
            </span>
          ) : null}
        </button>
      </div>
      <div className="room-list-sort" role="group" aria-label={t("roomList.sort")}>
        <span className="room-list-sort-label">{t("roomList.sortLabel")}</span>
        <button
          className={`room-list-sort-button ${selectedSort === "active" ? "is-selected" : ""}`}
          type="button"
          aria-pressed={selectedSort === "active"}
          onClick={() => onSelectSort("active")}
        >
          {t("roomList.sortActive")}
        </button>
        <button
          className={`room-list-sort-button ${selectedSort === "name" ? "is-selected" : ""}`}
          type="button"
          aria-pressed={selectedSort === "name"}
          onClick={() => onSelectSort("name")}
        >
          {t("roomList.sortName")}
        </button>
      </div>
    </div>
  );
}

function compactAttentionCount(count: number): string {
  return count > 99 ? "99+" : String(count);
}

function roomListCategoryAccessibleLabel(
  category: string,
  total: number,
  unread: number,
  highlights: number
): string {
  return highlights > 0
    ? t("roomList.categorySummaryWithHighlights", {
        category,
        unread,
        total,
        highlights
      })
    : t("roomList.categorySummary", { category, unread, total });
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
  showHeader = true,
  showWhenEmpty = false,
  onOpenContextMenu,
  onJoinRoom,
  onSelectInvite,
  onSelectRoom,
  onToggleCollapsed
}: {
  activeRoomId: string | null;
  collapsed: boolean;
  id: string;
  kind: "room" | "dm" | "invite" | "notJoined";
  label: string;
  presence: DesktopSnapshot["state"]["domain"]["live_signals"]["presence"];
  roomById: Map<string, RoomSummary>;
  rooms: RoomListItem[];
  showHeader?: boolean;
  showWhenEmpty?: boolean;
  onOpenContextMenu: OpenContextMenu;
  onJoinRoom?: (roomId: string) => void;
  onSelectInvite?: () => void;
  onSelectRoom: (roomId: string) => void;
  onToggleCollapsed?: () => void;
}) {
  if (!showWhenEmpty && rooms.length === 0) {
    return null;
  }

  return (
    <section className="room-section" data-room-section={id} aria-label={label}>
      {showHeader ? (
        <SectionTitle
          collapsed={collapsed}
          count={rooms.length}
          label={label}
          onToggle={onToggleCollapsed ?? (() => undefined)}
        />
      ) : null}
      {!collapsed
        ? rooms.map((room) => (
            <RoomButton
              activeRoomId={activeRoomId}
              kind={kind}
              presence={presence}
              roomById={roomById}
              key={room.room_id}
              room={room}
              onJoinRoom={onJoinRoom}
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
  onJoinRoom,
  onOpenContextMenu,
  onSelectInvite,
  onSelectRoom
}: {
  activeRoomId: string | null;
  kind: "room" | "dm" | "invite" | "notJoined";
  presence: DesktopSnapshot["state"]["domain"]["live_signals"]["presence"];
  roomById: Map<string, RoomSummary>;
  room: RoomListItem;
  onJoinRoom?: (roomId: string) => void;
  onOpenContextMenu: OpenContextMenu;
  onSelectInvite?: () => void;
  onSelectRoom: (roomId: string) => void;
}) {
  const sourceRoom = roomById.get(room.room_id);
  const dmUserIds = sourceRoom?.dm_user_ids ?? [];
  const dmUserId =
    kind === "dm" && sourceRoom?.is_dm && dmUserIds.length === 1
      ? dmUserIds[0]
      : null;
  const isOnlineDm = dmUserId ? presence[dmUserId] === "online" : false;
  const mentionCount = room.highlight_count ?? 0;
  return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      aria-label={room.display_name}
      data-mention-count={mentionCount || undefined}
      data-room-kind={kind}
      data-testid="room-item"
      type="button"
      onClick={() => {
        if (kind === "invite") {
          onSelectInvite?.();
          return;
        }
        if (kind === "notJoined") {
          onJoinRoom?.(room.room_id);
          return;
        }
        onSelectRoom(room.room_id);
      }}
      onContextMenu={(event) => {
        if (kind === "invite" || kind === "notJoined") {
          event.preventDefault();
          return;
        }
        onOpenContextMenu(
          event,
          { kind: "room", roomId: room.room_id, dmUserId },
          contextMenuItems({
            kind: "room",
            roomId: room.room_id,
            tags: room.tags ?? EMPTY_ROOM_TAGS,
            dmUserIds: dmUserId ? [dmUserId] : []
          })
        );
      }}
    >
      <span className="room-avatar-shell">
        <EntityAvatar
          avatar={room.avatar}
          className={`room-avatar ${kind === "dm" ? "is-user" : "is-room"}`}
          colorSeed={room.room_id}
          fallback={initials(room.display_name)}
        />
        {isOnlineDm ? <span className="room-presence-dot" aria-hidden="true" /> : null}
      </span>
      <span className="room-name" dir="auto">{room.display_name}</span>
      <span className="room-trailing">
        {mentionCount ? <span className="room-mention-dot" aria-hidden="true" /> : null}
        <span className="room-count">{room.unread_count || ""}</span>
      </span>
    </button>
  );
}

export function EntityAvatar({
  avatar,
  className,
  colorSeed,
  fallback,
  fallbackMode = "initials"
}: {
  avatar: RoomListItem["avatar"];
  className: string;
  colorSeed?: string | null;
  fallback: string;
  fallbackMode?: "initials" | "compactLabel";
}) {
  const sourceUrl =
    avatar?.thumbnail.kind === "ready" ? mediaSourceUrl(avatar.thumbnail.source_url) : null;
  const { displaySourceUrl, onImageError, onImageLoad } = useRecoverableImageSource(sourceUrl);
  const showImage = Boolean(displaySourceUrl);
  const colorClassName = avatarColorClass(colorSeed || fallback);
  const fallbackClassName =
    fallbackMode === "compactLabel"
      ? `avatar-fallback compact-label ${colorClassName}`
      : `avatar-fallback ${colorClassName}`;
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

export function avatarColorClass(seed: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < seed.length; index += 1) {
    hash ^= seed.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `avatar-c${(hash % 8) + 1}`;
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
