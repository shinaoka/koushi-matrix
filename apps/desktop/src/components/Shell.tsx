import {
  type CSSProperties,
  type DragEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useId,
  useRef,
  useState
} from "react";
import {
  Activity,
  AlertTriangle,
  Bell,
  Bug,
  ChevronDown,
  Clock3,
  Compass,
  Globe2,
  Home,
  LockKeyhole,
  CircleHelp,
  Info,
  LoaderCircle,
  MessageSquare,
  MoreHorizontal,
  Plus,
  Search,
  Settings,
  Users,
  X
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  AccountHomeItem,
  AccountTabSummary,
  AccountTabsSnapshot,
  DesktopSnapshot,
  DisplayPlatform,
  RoomListItem,
  RoomListSort,
  RoomSummary,
  SearchScopeKind,
  SettingsPatch,
  SidebarSectionKind
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import { renderableThumbnailSourceUrl } from "../backend/linkMediaRuntime";
import {
  ROOM_ACCESS_CHECKING,
  roomAccessHeaderBadges,
  roomAccessIndicator,
  roomAccessRailSummary
} from "../domain/accessCondition";
import { Tooltip } from "./Tooltip";
import { ImeTextField } from "./ImeTextControl";
import { useRecoverableImageSource } from "./avatarImage";
import {
  ICON_SIZE,
  roomAccessTooltipLabel,
  type OpenContextMenu,
  type PrimaryView,
  avatarInitial,
  elementAvatarColorIndex,
  graphemeCount,
  roomListItemLabel,
  EMPTY_ROOM_TAGS
} from "../app/uiShared";
const HOME_SCOPE_KEY = "__home__";

export type RuntimeAlertKind = "secureBackup" | "sync" | "session";

export interface RuntimeAlert {
  kind: RuntimeAlertKind;
  severity: "warning" | "error";
  title: string;
  detail: string;
  retryable: boolean;
}

function filterSidebarRooms(rooms: RoomListItem[], query: string): RoomListItem[] {
  const normalized = query.trim().toLocaleLowerCase();
  return normalized.length === 0
    ? rooms
    : rooms.filter((room) => roomListItemLabel(room).toLocaleLowerCase().includes(normalized));
}

/**
 * Accessible name for the Home rail button.
 *
 * The badge shows one Rust-owned total, so the label is where unread messages
 * and invites stay individually readable (#330). A quiet Home keeps the plain
 * name rather than announcing two zeroes.
 */
function accountHomeLabel(home: AccountHomeItem): string {
  if (home.attention_count === 0) {
    return home.display_name;
  }
  return t("workspace.homeAttention", {
    name: home.display_name,
    unread: String(home.unread_count),
    invites: String(home.invite_count)
  });
}

function shouldStartTitlebarDrag(event: MouseEvent<HTMLElement>): boolean {
  if (event.buttons !== 1 || !(event.target instanceof Element)) {
    return false;
  }
  return !event.target.closest("button, input, select, textarea, a, label");
}

/**
 * Name the search target the scope actually covers.
 *
 * The placeholder used to be derived from the active space no matter what the
 * scope selector said, so `All` read as `Search in <space>` and told the user
 * the search was narrower than it was.
 */
function searchScopePlaceholder(
  scope: SearchScopeKind,
  activeSpaceName: string,
  activeRoomName: string | null
): string {
  switch (scope) {
    case "allRooms":
      return t("workspace.searchEverywhere");
    case "currentRoom": {
      const roomName = activeRoomName?.trim();
      // With no room selected there is no target to name; claiming one would
      // be worse than the generic label.
      return roomName ? t("workspace.searchInRoom", { roomName }) : t("workspace.search");
    }
    case "currentSpace":
      return t("workspace.searchPlaceholder", { spaceName: activeSpaceName });
  }
}

function accountTabLabel(tab: AccountTabSummary): string {
  return (
    tab.displayName?.trim() ||
    tab.accountKey?.replace(/^@/, "").split(":", 1)[0] ||
    t("accountTabs.addAccount")
  );
}

function shortHomeserverName(homeserver: string | null): string {
  if (!homeserver) return "";
  try {
    return new URL(homeserver.includes("://") ? homeserver : `https://${homeserver}`).hostname;
  } catch {
    return homeserver;
  }
}

export function AccountTabStrip({
  tabs,
  selectedTabId,
  onSelect,
  onAdd,
  onRemove
}: {
  tabs: AccountTabSummary[];
  selectedTabId: string | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
  onRemove: (id: string) => void;
}) {
  function statusLabel(status: AccountTabSummary["status"]): string {
    switch (status) {
      case "addAccount": return t("accountTabs.addAccount");
      case "restoring": return t("accountTabs.restoring");
      case "authenticating": return t("accountTabs.authenticating");
      case "needsVerification": return t("accountTabs.needsVerification");
      case "ready": return t("accountTabs.ready");
      case "signedOut": return t("accountTabs.signedOut");
      case "loggingOut": return t("accountTabs.loggingOut");
      case "error": return t("accountTabs.error");
    }
  }

  // An unfinished add-account tab closes back to the previous account (#1101);
  // only a signed-out account is removed from the list.
  function removeLabel(tab: AccountTabSummary, label: string): string {
    return tab.status === "addAccount"
      ? t("accountTabs.cancelAddAccount")
      : t("accountTabs.removeFromList", { account: label });
  }

  return (
    <nav className="account-tab-strip" aria-label={t("settings.accountSettings")}>
      {tabs.map((tab) => {
        const label = accountTabLabel(tab);
        const status = statusLabel(tab.status);
        const selected = tab.id === selectedTabId;
        const avatarSource = tab.avatarSourceRef
          ? renderableThumbnailSourceUrl(tab.avatarSourceRef)
          : null;
        return (
          <div className="account-tab-host" key={tab.id}>
            <button
              className="account-tab"
              type="button"
              data-selected={selected}
              data-status={tab.status}
              aria-current={selected ? "page" : undefined}
              aria-label={t("accountTabs.select", { account: label, status })}
              title={`${label}${tab.homeserver ? ` · ${shortHomeserverName(tab.homeserver)}` : ""}`}
              onClick={() => onSelect(tab.id)}
            >
              <span className={`account-tab-avatar ${avatarColorClass(tab.accountKey ?? tab.id)}`}>
                {avatarSource ? (
                  <img src={avatarSource} alt="" />
                ) : (
                  avatarInitial(label)
                )}
              </span>
              <span className="account-tab-label">{label}</span>
              <span className="account-tab-server">{shortHomeserverName(tab.homeserver)}</span>
              {tab.status === "ready" ? (
                tab.unreadCount > 0 ? (
                  <span className="account-tab-unread">{tab.unreadCount > 99 ? "99+" : tab.unreadCount}</span>
                ) : (
                  <span className="account-tab-ready-dot" role="img" aria-label={status} />
                )
              ) : tab.status === "needsVerification" ? (
                <AlertTriangle size={ICON_SIZE.small} aria-label={status} />
              ) : tab.status === "restoring" || tab.status === "authenticating" || tab.status === "loggingOut" ? (
                <span className="account-tab-spinner" role="status" aria-label={status} />
              ) : tab.status === "error" ? (
                <span className="account-tab-error-dot" role="img" aria-label={status} />
              ) : tab.status === "signedOut" ? (
                <span className="account-tab-signed-out-dot" role="img" aria-label={status} />
              ) : null}
            </button>
            {tab.status === "signedOut" || (tab.status === "addAccount" && tabs.length > 1) ? (
              <button
                className="account-tab-remove"
                type="button"
                aria-label={removeLabel(tab, label)}
                title={removeLabel(tab, label)}
                onClick={() => onRemove(tab.id)}
              >
                <X size={ICON_SIZE.micro} aria-hidden="true" />
              </button>
            ) : null}
          </div>
        );
      })}
      <button
        className="account-tab-add"
        type="button"
        aria-label={t("accountTabs.addAccount")}
        title={t("accountTabs.addAccount")}
        onClick={onAdd}
      >
        <Plus size={ICON_SIZE.control} aria-hidden="true" />
      </button>
    </nav>
  );
}

export function PersistentAccountShell({
  accountTabs,
  selectedAccountTabId,
  accountScopedContentReady = false,
  platform = "linux",
  onSelectAccountTab,
  onAddAccountTab,
  onRemoveSignedOutAccountTab,
  onOpenAppSettings,
  onOpenDiagnostics,
  children
}: {
  accountTabs: AccountTabsSnapshot | null;
  selectedAccountTabId: string | null;
  accountScopedContentReady?: boolean;
  platform?: DisplayPlatform;
  onSelectAccountTab: (id: string) => void;
  onAddAccountTab: () => void;
  onRemoveSignedOutAccountTab: (id: string) => void;
  onOpenAppSettings: () => void;
  onOpenDiagnostics: () => void;
  children: ReactNode;
}) {
  return (
    <div
      className="account-tab-shell"
      data-account-content-ready={accountScopedContentReady ? "true" : "false"}
    >
      {!accountScopedContentReady ? <TopBar
        accountTabs={accountTabs}
        selectedAccountTabId={selectedAccountTabId}
        accountScopedContentReady={false}
        platform={platform}
        onSelectAccountTab={onSelectAccountTab}
        onAddAccountTab={onAddAccountTab}
        onRemoveSignedOutAccountTab={onRemoveSignedOutAccountTab}
        onOpenAppSettings={onOpenAppSettings}
        onOpenDiagnostics={onOpenDiagnostics}
      /> : null}
      <div className="account-tab-shell-content">{children}</div>
    </div>
  );
}

export function TopBar({
  activeRoomName = null,
  activeSpaceName = "",
  platform = "linux",
  searchInputRef,
  searchQuery = "",
  searchScope = "currentRoom",
  onOpenDiagnostics = () => undefined,
  onSearchQueryChange = () => undefined,
  onSearchScopeChange = () => undefined,
  onStartWindowDrag = () => undefined,
  onOpenAppSettings = () => undefined,
  onSelectAccountTab = () => undefined,
  onAddAccountTab = () => undefined,
  onRemoveSignedOutAccountTab = () => undefined,
  accountTabs = null,
  selectedAccountTabId = null,
  accountScopedContentReady = true
}: {
  activeRoomName?: string | null;
  activeSpaceName?: string;
  platform?: DisplayPlatform;
  searchInputRef?: RefObject<HTMLInputElement | null>;
  searchQuery?: string;
  searchScope?: SearchScopeKind;
  onOpenDiagnostics?: () => void;
  onSearchQueryChange?: (value: string) => void;
  onSearchScopeChange?: (value: SearchScopeKind) => void;
  onStartWindowDrag?: () => void;
  onOpenAppSettings?: () => void;
  onSelectAccountTab?: (id: string) => void;
  onAddAccountTab?: () => void;
  onRemoveSignedOutAccountTab?: (id: string) => void;
  accountTabs?: AccountTabsSnapshot | null;
  selectedAccountTabId?: string | null;
  accountScopedContentReady?: boolean;
}) {
  return (
    <header
      className="titlebar"
      data-platform={platform}
      data-account-content-ready={accountScopedContentReady}
      data-tauri-drag-region=""
      onMouseDown={(event) => {
        if (!shouldStartTitlebarDrag(event)) {
          return;
        }
        event.preventDefault();
        onStartWindowDrag();
      }}
    >
      <AccountTabStrip
        tabs={accountTabs?.tabs ?? []}
        selectedTabId={selectedAccountTabId ?? accountTabs?.selectedTabId ?? null}
        onSelect={onSelectAccountTab}
        onAdd={onAddAccountTab}
        onRemove={onRemoveSignedOutAccountTab}
      />
      {accountScopedContentReady ? <label className="top-search">
        <Search size={ICON_SIZE.input} />
        <ImeTextField
          ref={searchInputRef}
          aria-label={t("workspace.search")}
          value={searchQuery}
          syncKey="workspace-search"
          dir="auto"
          placeholder={searchScopePlaceholder(searchScope, activeSpaceName, activeRoomName)}
          onChange={(event) => onSearchQueryChange(event.target.value)}
        />
      </label> : null}
      {accountScopedContentReady ? <select
        className="scope-select"
        aria-label={t("workspace.searchScope")}
        value={searchScope}
        onChange={(event) => onSearchScopeChange(event.target.value as SearchScopeKind)}
      >
        <option value="allRooms">{t("search.scopeAll")}</option>
        <option value="currentSpace">{t("search.scopeSpace")}</option>
        <option value="currentRoom">{t("search.scopeRoom")}</option>
      </select> : null}
      <div className="top-actions">
        <button
          className="icon-button app-settings-button"
          type="button"
          aria-label={t("settings.appSettings")}
          title={t("settings.appSettings")}
          onClick={onOpenAppSettings}
        >
          <Settings size={ICON_SIZE.small} aria-hidden="true" />
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
  onCreateSpace,
  onOpenContextMenu,
  onOpenUserSettings,
  onReorderSpaces,
  onSelectSpace,
  onRequestAvatarThumbnail
}: {
  snapshot: DesktopSnapshot;
  onCreateSpace: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenUserSettings: () => void;
  onReorderSpaces: (spaceIds: string[]) => void;
  onSelectSpace: (spaceId: string | null) => void;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
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
          <button
            className={`workspace-button workspace-system-button workspace-home-button ${
              snapshot.sidebar.account_home.is_active ? "is-active" : ""
            }`}
            data-count={snapshot.sidebar.account_home.attention_count || undefined}
            type="button"
            aria-label={accountHomeLabel(snapshot.sidebar.account_home)}
            aria-current={snapshot.sidebar.account_home.is_active ? "page" : undefined}
            onClick={() => onSelectSpace(null)}
          >
            <Home size={ICON_SIZE.rail} />
          </button>
        </div>
        <div className="workspace-rail-separator" role="separator" aria-orientation="horizontal" />
        <div className="workspace-list workspace-space-list">
          {snapshot.sidebar.space_rail.map((space) => {
            const localIcon = space.local_icon?.trim();
            // #1217: an explicit local name is user text, so the generated tile
            // renders the whole name. A Space with only a Matrix name keeps the
            // Element/Compound single-grapheme fallback (#414).
            const hasLocalName = Boolean(
              snapshot.state.ui.navigation.space_local_presentations[space.space_id]?.name?.trim()
            );
            const fallbackName = space.display_name.trim() || space.space_id || "?";
            // #1166: the rail item keeps the Space name and explains its access
            // condition, and a bounded overlay summarises it at the avatar.
            // #1166: the rail item carries its own projected access condition.
            const spaceRule = space.access_join_rule ?? null;
            const spaceAccess =
              roomAccessIndicator(spaceRule, space.access_restricted_conditions, {
                spaceMembersRoute: space.access_space_members_route,
                allowedRoomNames: space.access_allowed_room_names
              }) ?? ROOM_ACCESS_CHECKING;
            const railSummary = roomAccessRailSummary(spaceRule);
            return (
            <Tooltip
              label={`${fallbackName}${t("access.conditionSummarySeparator")}${roomAccessTooltipLabel(
                spaceAccess.descriptionMessageId,
                spaceAccess.descriptionAllowedRoomNames,
                spaceAccess.descriptionSpaceName
              )}`}
              key={space.space_id}
            >
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
                  aria-label={fallbackName}
                  aria-current={space.is_active ? "page" : undefined}
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
                    fallback={localIcon || (hasLocalName ? fallbackName : avatarInitial(fallbackName))}
                    fallbackMode={localIcon || hasLocalName ? "compactLabel" : "elementSpace"}
                    onRequestAvatarThumbnail={onRequestAvatarThumbnail}
                  />
                  {/* The overlay sits at the avatar's *upper* trailing corner: the
                      unread/notification count owns the lower one, and the access
                      overlay must never obscure it. */}
                  <span
                    className="workspace-access-overlay"
                    data-space-access={railSummary}
                    aria-hidden="true"
                  >
                    {railSummary === "globe" ? (
                      <Globe2 size={ICON_SIZE.micro} aria-hidden="true" />
                    ) : railSummary === "padlock" ? (
                      <LockKeyhole size={ICON_SIZE.micro} aria-hidden="true" />
                    ) : railSummary === "info" ? (
                      <Info size={ICON_SIZE.micro} aria-hidden="true" />
                    ) : railSummary === "question" ? (
                      <CircleHelp size={ICON_SIZE.micro} aria-hidden="true" />
                    ) : (
                      <LoaderCircle size={ICON_SIZE.micro} aria-hidden="true" />
                    )}
                  </span>
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
  onCreateRoom,
  onAddExistingRoom,
  onNewDm,
  onOpenContextMenu,
  onOpenActivity,
  onOpenExplore,
  onOpenInvites,
  onOpenThreads = () => undefined,
  onOpenScheduledMessages = () => undefined,
  onOpenSpaceInfo,
  onOpenSpaceMembers = () => undefined,
  spaceMemberCounts,
  onJoinRoom,
  onSelectRoom,
  onUpdateSettings = () => undefined,
  onRequestAvatarThumbnail
}: {
  activeRoomId: string | null;
  activeView: PrimaryView;
  snapshot: DesktopSnapshot;
  onCreateRoom: () => void;
  /** #1007: open Add existing room for the active Space. */
  onAddExistingRoom?: (spaceId: string) => void;
  onNewDm: () => void;
  onOpenContextMenu: OpenContextMenu;
  onOpenActivity: () => void;
  onOpenExplore: () => void;
  onOpenInvites: () => void;
  onOpenThreads?: () => void;
  onOpenScheduledMessages?: () => void;
  onOpenSpaceInfo: () => void;
  onOpenSpaceMembers?: () => void;
  spaceMemberCounts?: { joined: number; childOnly: number };
  onJoinRoom?: (roomId: string) => void;
  onSelectRoom: (roomId: string) => void;
  onUpdateSettings?: (patch: SettingsPatch) => void;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const sections = snapshot.sidebar.sections;
  const roomListReadiness = snapshot.state.ui.room_list.readiness;
  const roomListReady = roomListReadiness.kind === "ready";
  const hasProvisionalRoomList =
    snapshot.sidebar.space_rooms.length > 0 ||
    snapshot.sidebar.global_dms.length > 0 ||
    sections.not_joined.length > 0;
  const sidebarSettings = snapshot.state.domain.settings.values.sidebar;
  const collapsedSections = sidebarSettings.collapsed;
  const activeSpace = snapshot.sidebar.space_rail.find((space) => space.is_active);
  const activeSpaceName = activeSpace?.display_name ?? snapshot.sidebar.account_home.display_name;
  // #1166: the active Space's access condition comes from the rail item Rust
  // projected for it.
  const activeSpaceAccess = activeSpace
    ? roomAccessIndicator(
        activeSpace.access_join_rule ?? null,
        activeSpace.access_restricted_conditions,
        {
          spaceMembersRoute: activeSpace.access_space_members_route,
          allowedRoomNames: activeSpace.access_allowed_room_names
        }
      ) ?? ROOM_ACCESS_CHECKING
    : null;
  const accountHomeActive = snapshot.sidebar.account_home.is_active && !activeSpace;
  const roomById = new Map(snapshot.state.domain.rooms.map((room) => [room.room_id, room]));
  const presence = snapshot.state.domain.live_signals.presence;
  const [roomFilter, setRoomFilter] = useState("");
  const activeSpaceId = snapshot.state.ui.navigation.active_space_id;
  const scopeKey = activeSpaceId ?? HOME_SCOPE_KEY;
  // Rust projects mutually exclusive sections; React only applies the search
  // filter to them (state-machine.md, "Sidebar Sections And Low Priority").
  const visibleRooms = filterSidebarRooms(sections.rooms, roomFilter);
  const visibleDms = filterSidebarRooms(sections.people, roomFilter);
  const visibleLowPriority = filterSidebarRooms(sections.low_priority, roomFilter);
  const roomsSort = snapshot.sidebar.rooms_sort ?? snapshot.state.domain.settings.values.room_list_sort;
  const dmsSort = snapshot.sidebar.dms_sort ?? snapshot.state.domain.settings.values.room_list_sort;
  const roomsCollapsed = snapshot.sidebar.rooms_collapsed ?? false;
  const dmsCollapsed = snapshot.sidebar.dms_collapsed ?? false;
  const lowPriorityCollapsed = snapshot.sidebar.low_priority_collapsed ?? false;
  const resolvedSpaceMemberCounts = spaceMemberCounts ?? {
    joined: snapshot.state.domain.space_members.space_joined.length,
    childOnly: snapshot.state.domain.space_members.child_room_only.length
  };

  useEffect(() => {
    setRoomFilter("");
  }, [activeSpaceId]);

  function updateSectionPreference(
    section: SidebarSectionKind,
    patch: { collapsed?: boolean; sort?: RoomListSort }
  ) {
    onUpdateSettings({
      sidebar_section: { scope: scopeKey, section, ...patch }
    });
  }

  return (
    <aside className="sidebar" aria-label={t("workspace.rooms")}>
      <div className="workspace-header">
        <div className="workspace-header-title">
          {activeSpaceAccess?.icon ? (
            <Tooltip
              label={roomAccessTooltipLabel(
                activeSpaceAccess.descriptionMessageId,
                activeSpaceAccess.descriptionAllowedRoomNames,
                activeSpaceAccess.descriptionSpaceName
              )}
            >
              {(triggerProps) => (
                <span
                  className="workspace-access-icon"
                  data-space-access={activeSpaceAccess.icon}
                  tabIndex={0}
                  {...triggerProps}
                >
                  {activeSpaceAccess.icon === "globe" ? (
                    <Globe2 size={ICON_SIZE.small} aria-hidden="true" />
                  ) : (
                    <LockKeyhole size={ICON_SIZE.small} aria-hidden="true" />
                  )}
                </span>
              )}
            </Tooltip>
          ) : null}
          <div className="workspace-name" dir="auto">
            {activeSpaceName}
          </div>
          {activeSpaceAccess
            ? roomAccessHeaderBadges(activeSpaceAccess).map((badge) => (
                <Tooltip
                  key={badge.labelMessageId}
                  label={roomAccessTooltipLabel(
                    badge.descriptionMessageId,
                    badge.descriptionAllowedRoomNames,
                    badge.descriptionSpaceName
                  )}
                >
                  {(triggerProps) => (
                    <span className="workspace-access-badge" tabIndex={0} {...triggerProps}>
                      {t(badge.labelMessageId)}
                    </span>
                  )}
                </Tooltip>
              ))
            : null}
        </div>
        <div className="workspace-header-actions no-wrap">
          <div className="workspace-header-context-actions" data-toolbar-group="context">
            {activeSpace ? (
              <SpaceMembersNavButton
                childOnlyCount={resolvedSpaceMemberCounts.childOnly}
                joinedCount={resolvedSpaceMemberCounts.joined}
                onClick={onOpenSpaceMembers}
              />
            ) : accountHomeActive ? (
              <>
                <HeaderActionButton
                  action="activity"
                  icon={<Activity size={ICON_SIZE.control} />}
                  label={t("workspace.activity")}
                  onClick={onOpenActivity}
                  pressed={activeView === "activity"}
                />
                <HeaderActionButton
                  action="explore"
                  icon={<Compass size={ICON_SIZE.control} />}
                  label={t("workspace.explore")}
                  onClick={onOpenExplore}
                  pressed={activeView === "explore"}
                />
                <HeaderActionButton
                  action="invites"
                  count={snapshot.state.domain.invites.length}
                  icon={<Bell size={ICON_SIZE.control} />}
                  label={t("workspace.invites")}
                  onClick={onOpenInvites}
                  pressed={activeView === "invites"}
                />
              </>
            ) : null}
          </div>
          <div className="workspace-header-end-actions" data-toolbar-group="end">
            <HeaderActionButton
              action="threads"
              icon={<MessageSquare size={ICON_SIZE.control} />}
              label={t("threads.title")}
              onClick={onOpenThreads}
            />
            <HeaderActionButton
              action="scheduled"
              icon={<Clock3 size={ICON_SIZE.control} />}
              label={t("workspace.scheduledMessages")}
              onClick={onOpenScheduledMessages}
            />
            <HeaderActionButton
              action="info"
              icon={<Settings size={ICON_SIZE.control} />}
              label={t("workspace.spaceInfoSettings")}
              onClick={onOpenSpaceInfo}
            />
          </div>
        </div>
      </div>
      <div className="sidebar-scroll">
        {!roomListReady ? (
          <div className="room-list-status" role="status">
            {roomListReadiness.kind === "failed" ? t("roomList.failed") : t("roomList.loading")}
          </div>
        ) : null}
        {roomListReady || hasProvisionalRoomList ? (
          <RoomListControls
            filter={roomFilter}
            filterPlaceholder={t("roomList.filterConversationsPlaceholder")}
            onFilterChange={setRoomFilter}
          />
        ) : null}
        <RoomSection
          activeRoomId={activeRoomId}
          collapsed={roomsCollapsed}
          id="rooms"
          kind="room"
          label={t("roomList.categoryRooms")}
          presence={presence}
          roomById={roomById}
          rooms={visibleRooms}
          unreadCount={snapshot.sidebar.space_unread_count}
          emptyMessage={roomFilter ? t("roomList.noMatchingConversations") : undefined}
          showWhenEmpty={true}
          onCreate={onCreateRoom}
          onAddExisting={
            activeSpace && onAddExistingRoom
              ? () => onAddExistingRoom(activeSpace.space_id)
              : undefined
          }
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
          onSelectSort={(sort) => updateSectionPreference("rooms", { sort })}
          onToggleCollapsed={() =>
            updateSectionPreference("rooms", { collapsed: !roomsCollapsed })
          }
          selectedSort={roomsSort}
          onRequestAvatarThumbnail={onRequestAvatarThumbnail}
        />
        <RoomSection
          activeRoomId={activeRoomId}
          collapsed={dmsCollapsed}
          id="dms"
          kind="dm"
          label={t("roomList.categoryDms")}
          presence={presence}
          roomById={roomById}
          rooms={visibleDms}
          unreadCount={snapshot.sidebar.dm_unread_count}
          emptyMessage={roomFilter ? t("roomList.noMatchingConversations") : undefined}
          showWhenEmpty={true}
          onCreate={onNewDm}
          onOpenContextMenu={onOpenContextMenu}
          onSelectRoom={onSelectRoom}
          onSelectSort={(sort) => updateSectionPreference("dms", { sort })}
          onToggleCollapsed={() => updateSectionPreference("dms", { collapsed: !dmsCollapsed })}
          selectedSort={dmsSort}
          onRequestAvatarThumbnail={onRequestAvatarThumbnail}
        />
        {sections.low_priority.length > 0 ? (
          <RoomSection
            activeRoomId={activeRoomId}
            collapsed={lowPriorityCollapsed}
            id="low-priority"
            kind="room"
            label={t("workspace.lowPriority")}
            presence={presence}
            roomById={roomById}
            rooms={visibleLowPriority}
            emptyMessage={roomFilter ? t("roomList.noMatchingConversations") : undefined}
            showWhenEmpty={true}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
            onToggleCollapsed={() =>
              updateSectionPreference("lowPriority", { collapsed: !lowPriorityCollapsed })
            }
            onRequestAvatarThumbnail={onRequestAvatarThumbnail}
          />
        ) : null}
        {sections.not_joined.length > 0 ? (
          <RoomSection
            activeRoomId={activeRoomId}
            collapsed={Boolean(collapsedSections.not_joined)}
            id="not-joined"
            kind="notJoined"
            label={t("workspace.notJoined")}
            presence={presence}
            roomById={roomById}
            rooms={sections.not_joined}
            onJoinRoom={onJoinRoom}
            onOpenInvites={onOpenInvites}
            onOpenContextMenu={onOpenContextMenu}
            onSelectRoom={onSelectRoom}
            onToggleCollapsed={() =>
              onUpdateSettings({
                sidebar: {
                  ...sidebarSettings,
                  collapsed: {
                    ...collapsedSections,
                    not_joined: !collapsedSections.not_joined
                  }
                }
              })
            }
            onRequestAvatarThumbnail={onRequestAvatarThumbnail}
          />
        ) : null}
        {roomFilter.trim().length > 0 &&
        visibleRooms.length === 0 &&
        visibleDms.length === 0 &&
        visibleLowPriority.length === 0 ? (
          <div className="room-list-no-matches" role="status">
            {t("roomList.noMatchingConversations")}
          </div>
        ) : null}
      </div>
    </aside>
  );
}

function RoomListControls({
  filter,
  filterPlaceholder,
  onFilterChange
}: {
  filter: string;
  filterPlaceholder: string;
  onFilterChange: (value: string) => void;
}) {
  return (
    <div className="room-list-controls">
      <div className="room-list-filter">
        <Search size={ICON_SIZE.input} aria-hidden="true" />
        <ImeTextField
          aria-label={filterPlaceholder}
          className="room-list-filter-input"
          type="search"
          value={filter}
          onChange={(event) => onFilterChange(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && filter.length > 0) {
              event.preventDefault();
              onFilterChange("");
            }
          }}
          placeholder={filterPlaceholder}
        />
        {filter.length > 0 ? (
          <button
            className="icon-button room-list-filter-clear"
            type="button"
            aria-label={t("roomList.clearFilter")}
            onClick={() => onFilterChange("")}
          >
            <X size={ICON_SIZE.input} aria-hidden="true" />
          </button>
        ) : null}
      </div>
    </div>
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
  emptyMessage,
  showHeader = true,
  showWhenEmpty = false,
  unreadCount,
  onCreate,
  onAddExisting,
  onOpenContextMenu,
  onJoinRoom,
  onOpenInvites,
  onSelectInvite,
  onSelectRoom,
  onSelectSort,
  onToggleCollapsed,
  selectedSort,
  onRequestAvatarThumbnail
}: {
  activeRoomId: string | null;
  collapsed: boolean;
  id: string;
  kind: "room" | "dm" | "invite" | "notJoined";
  label: string;
  emptyMessage?: string;
  presence: DesktopSnapshot["state"]["domain"]["live_signals"]["presence"];
  roomById: Map<string, RoomSummary>;
  rooms: RoomListItem[];
  showHeader?: boolean;
  showWhenEmpty?: boolean;
  /**
   * Rust-owned unread total for this section's whole scope. When present it
   * replaces the conversation-count meta with the red unread badge; it is not
   * derived from `rooms`, so search and collapse never change it.
   */
  unreadCount?: number;
  onCreate?: () => void;
  onAddExisting?: () => void;
  onOpenContextMenu: OpenContextMenu;
  onJoinRoom?: (roomId: string) => void;
  onOpenInvites?: () => void;
  onSelectInvite?: () => void;
  onSelectRoom: (roomId: string) => void;
  onSelectSort?: (sort: RoomListSort) => void;
  onToggleCollapsed?: () => void;
  selectedSort?: RoomListSort;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  if (!showWhenEmpty && rooms.length === 0) {
    return null;
  }

  return (
    <section className="room-section" id={`${id}-room-list`} data-room-section={id} aria-label={label}>
      {showHeader ? (
        <SectionTitle
          collapsed={collapsed}
          count={rooms.length}
          unreadCount={unreadCount}
          label={label}
          onCreate={onCreate}
          onAddExisting={onAddExisting}
          createLabel={kind === "dm" ? t("workspace.newDm") : t("action.createRoom")}
          onSelectSort={onSelectSort}
          sectionId={id}
          onToggle={onToggleCollapsed ?? (() => undefined)}
          selectedSort={selectedSort}
        />
      ) : null}
      {!collapsed
        ? rooms.length > 0
          ? rooms.map((room) => (
              <RoomButton
                activeRoomId={activeRoomId}
                kind={kind}
                presence={presence}
                roomById={roomById}
                key={room.room_id}
                room={room}
                onJoinRoom={onJoinRoom}
                onOpenInvites={onOpenInvites}
                onOpenContextMenu={onOpenContextMenu}
                onSelectInvite={onSelectInvite}
                onSelectRoom={onSelectRoom}
                onRequestAvatarThumbnail={onRequestAvatarThumbnail}
              />
            ))
          : emptyMessage
            ? <div className="room-list-empty">{emptyMessage}</div>
            : null
        : null}
    </section>
  );
}

function HeaderActionButton({
  action,
  count = 0,
  icon,
  label,
  pressed,
  onClick
}: {
  action: string;
  count?: number;
  icon: ReactNode;
  label: string;
  pressed?: boolean;
  onClick: () => void;
}) {
  return (
    <Tooltip label={label}>
      {(triggerProps) => (
        <button
          {...triggerProps}
          className={`icon-button ${pressed ? "is-active" : ""}`}
          data-count={count || undefined}
          data-header-action={action}
          type="button"
          aria-label={label}
          aria-pressed={pressed}
          onClick={onClick}
        >
          {icon}
          {count > 0 ? (
            <span className="workspace-header-badge" aria-hidden="true">
              {count}
            </span>
          ) : null}
        </button>
      )}
    </Tooltip>
  );
}

function SpaceMembersNavButton({
  childOnlyCount,
  joinedCount,
  onClick
}: {
  childOnlyCount: number;
  joinedCount: number;
  onClick: () => void;
}) {
  const label = t("spaceMembers.navAccessible", {
    joined: joinedCount,
    childOnly: childOnlyCount
  });
  return (
    <Tooltip label={label}>
      {(triggerProps) => (
        <button
          {...triggerProps}
          className="icon-button space-members-nav"
          data-header-action="members"
          type="button"
          aria-label={label}
          onClick={onClick}
        >
          <Users size={ICON_SIZE.control} aria-hidden="true" />
          <span className="space-members-nav-count">
            {joinedCount}
            {childOnlyCount > 0 ? (
              <span className="space-members-nav-warning"> · +{childOnlyCount}</span>
            ) : null}
          </span>
        </button>
      )}
    </Tooltip>
  );
}

function SectionTitle({
  collapsed,
  count,
  createLabel,
  label,
  onCreate,
  onAddExisting,
  onSelectSort,
  onToggle,
  sectionId,
  selectedSort,
  unreadCount
}: {
  collapsed: boolean;
  count: number;
  createLabel?: string;
  label: string;
  unreadCount?: number;
  onCreate?: () => void;
  onAddExisting?: () => void;
  onSelectSort?: (sort: RoomListSort) => void;
  onToggle: () => void;
  sectionId: string;
  selectedSort?: RoomListSort;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuTriggerRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!menuOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setMenuOpen(false);
        menuTriggerRef.current?.focus();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [menuOpen]);

  const sortOptions: Array<[RoomListSort, string]> = [
    [{ kind: "recentFirst" }, t("roomList.sortRecent")],
    [{ kind: "normalLocale" }, t("roomList.sortName")],
    [{ kind: "activity" }, t("roomList.sortAttention")]
  ];

  return (
    <div
      className="section-title"
    >
      <button
        className="section-title-toggle"
        type="button"
        aria-expanded={!collapsed}
        aria-controls={`${sectionId}-room-list`}
        onClick={onToggle}
      >
        <span className="section-title-label">{label}</span>
        <ChevronDown size={ICON_SIZE.compact} aria-hidden="true" />
      </button>
      <span className="section-title-meta">
        {unreadCount === undefined ? (
          <span className="section-count">{count}</span>
        ) : unreadCount > 0 ? (
          <span
            className="section-unread-count"
            aria-label={t("roomList.sectionUnreadAccessible", {
              section: label,
              count: unreadCount
            })}
          >
            {unreadCount > 99 ? "99+" : unreadCount}
          </span>
        ) : null}
        {onCreate ? (
          <button
            className="section-title-action"
            type="button"
            aria-label={createLabel}
            onClick={onCreate}
          >
            <Plus size={ICON_SIZE.compact} aria-hidden="true" />
          </button>
        ) : null}
        {onSelectSort && selectedSort ? (
          <span className="section-menu-wrap">
            <button
              ref={menuTriggerRef}
              className="section-title-action"
              type="button"
              aria-label={t("roomList.sectionOptions", { section: label })}
              aria-expanded={menuOpen}
              onClick={() => setMenuOpen((open) => !open)}
            >
              <MoreHorizontal size={ICON_SIZE.compact} aria-hidden="true" />
            </button>
            {menuOpen ? (
              <div className="section-menu" role="menu" aria-label={t("roomList.sectionOptions", { section: label })}>
                {onAddExisting ? (
                  <>
                    <button
                      className="section-menu-item"
                      type="button"
                      role="menuitem"
                      onClick={() => {
                        setMenuOpen(false);
                        onAddExisting();
                      }}
                    >
                      <span aria-hidden="true" />
                      <span>{t("spaceAddRooms.action")}</span>
                    </button>
                    <div className="section-menu-separator" role="separator" />
                  </>
                ) : null}
                <div className="section-menu-title">{t("roomList.sort")}</div>
                {sortOptions.map(([sort, sortLabel]) => (
                  <button
                    className="section-menu-item"
                    key={sort.kind}
                    type="button"
                    role="menuitemradio"
                    aria-checked={selectedSort.kind === sort.kind}
                    onClick={() => {
                      onSelectSort(sort);
                      setMenuOpen(false);
                      menuTriggerRef.current?.focus();
                    }}
                  >
                    <span aria-hidden="true">{selectedSort.kind === sort.kind ? "✓" : ""}</span>
                    <span>{sortLabel}</span>
                  </button>
                ))}
              </div>
            ) : null}
          </span>
        ) : null}
      </span>
    </div>
  );
}

function RoomButton({
  activeRoomId,
  kind,
  presence,
  roomById,
  room,
  onJoinRoom,
  onOpenInvites,
  onOpenContextMenu,
  onSelectInvite,
  onSelectRoom,
  onRequestAvatarThumbnail
}: {
  activeRoomId: string | null;
  kind: "room" | "dm" | "invite" | "notJoined";
  presence: DesktopSnapshot["state"]["domain"]["live_signals"]["presence"];
  roomById: Map<string, RoomSummary>;
  room: RoomListItem;
  onJoinRoom?: (roomId: string) => void;
  onOpenInvites?: () => void;
  onOpenContextMenu: OpenContextMenu;
  onSelectInvite?: () => void;
  onSelectRoom: (roomId: string) => void;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const sourceRoom = roomById.get(room.room_id);
  const dmUserIds = sourceRoom?.dm_user_ids ?? [];
  const dmUserId =
    kind === "dm" && sourceRoom?.is_dm && dmUserIds.length === 1
      ? dmUserIds[0]
      : null;
  const isOnlineDm = dmUserId ? presence[dmUserId] === "online" : false;
  const hasUnreadContent = room.has_unread_content ?? room.unread_count > 0;
  const displayCount = room.display_count ?? room.unread_count;
  const mentionCount = room.highlight_count ?? (room.has_unread_mention ? 1 : 0);
  const attentionHighlighted = room.is_attention_highlighted ?? mentionCount;
  // #1166: a joined row shows its own access condition; while it has not been
  // projected the row says so instead of guessing. Lanes that have no joined
  // condition (invitations, not-joined) render none.
  const access =
    roomAccessIndicator(room.access_join_rule, room.access_restricted_conditions, {
      spaceMembersRoute: room.access_space_members_route,
      allowedRoomNames: room.access_allowed_room_names
    }) ?? (kind === "room" || kind === "dm" ? ROOM_ACCESS_CHECKING : null);
  const roomLabel = roomListItemLabel(room);
  // #1166: the condition is announced as the row's *description*, so the row's
  // accessible name stays exactly the room label other surfaces and tests match
  // on. The visible icon and badges stay the sighted affordance.
  const accessDescriptionId = useId();
  return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      aria-label={roomLabel}
      aria-describedby={access ? accessDescriptionId : undefined}
      data-access={access?.icon ?? undefined}
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
          // Issue #961: an invitation is answered through the invite workflow,
          // which owns `state.invites` and the Home invite count; a single
          // click never silently accepts it. Anything the server's join rule
          // does not admit offers no action at all rather than firing a join it
          // would reject.
          if (room.membership === "invited") {
            onOpenInvites?.();
            return;
          }
          if (room.can_join) {
            onJoinRoom?.(room.room_id);
          }
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
          fallback={avatarInitial(roomListItemLabel(room))}
          onRequestAvatarThumbnail={onRequestAvatarThumbnail}
        />
        {isOnlineDm ? <span className="room-presence-dot" aria-hidden="true" /> : null}
      </span>
      {access ? (
        <span className="sr-only" id={accessDescriptionId}>
          {roomAccessTooltipLabel(
            access.descriptionMessageId,
            access.descriptionAllowedRoomNames,
            access.descriptionSpaceName
          )}
        </span>
      ) : null}
      {/* #1166: the icon, name and compact badges share the grid's name cell so
          the avatar and the trailing unread area keep their columns. */}
      <span className="room-name-shell">
        {access?.icon ? (
          <Tooltip
            label={roomAccessTooltipLabel(
              access.descriptionMessageId,
              access.descriptionAllowedRoomNames,
              access.descriptionSpaceName
            )}
          >
            {(triggerProps) => (
              <span
                className="room-access-icon"
                data-room-access={access.icon}
                {...triggerProps}
              >
                {access.icon === "globe" ? (
                  <Globe2 size={ICON_SIZE.micro} aria-hidden="true" />
                ) : (
                  <LockKeyhole size={ICON_SIZE.micro} aria-hidden="true" />
                )}
              </span>
            )}
          </Tooltip>
        ) : null}
        <span className="room-name" dir="auto">{roomLabel}</span>
        {access
          ? access.badges.map((badge) => (
              <Tooltip
                key={badge.labelMessageId}
                label={roomAccessTooltipLabel(
                  badge.descriptionMessageId,
                  badge.descriptionAllowedRoomNames,
                  badge.descriptionSpaceName
                )}
              >
                {(triggerProps) => (
                  <span className="room-access-badge" {...triggerProps}>
                    {t(badge.labelMessageId)}
                  </span>
                )}
              </Tooltip>
            ))
          : null}
      </span>
      <span className="room-trailing">
        {/*
          Issue #961: a room outside the account's joined rooms says which
          relationship it is in, so an invitation is not mistaken for a room
          that is simply open to join.
        */}
        {kind === "notJoined" ? (
          <span className="room-membership-badge">
            {roomMembershipLabel(room.membership)}
          </span>
        ) : null}
        {mentionCount ? <span className="room-mention-dot" aria-hidden="true" /> : null}
        {hasUnreadContent && displayCount === 0 ? (
          <span className="room-unread-dot" aria-hidden="true" />
        ) : null}
        {displayCount > 0 ? (
          <span className={`room-count ${attentionHighlighted ? "is-attention" : ""}`}>
            {displayCount}
          </span>
        ) : null}
      </span>
    </button>
  );
}

function roomMembershipLabel(membership: RoomListItem["membership"]): string {
  switch (membership) {
    case "invited":
      return t("roomList.membershipInvited");
    case "knocked":
      return t("roomList.membershipKnocked");
    case "unknown":
      return t("roomList.membershipUnknown");
    default:
      return t("roomList.membershipNotJoined");
  }
}

function useVisibleAvatarThumbnailRequest(
  avatar: RoomListItem["avatar"],
  onRequestAvatarThumbnail:
    | ((mxcUri: string) => void | Promise<void | (() => void)>)
    | undefined
): RefObject<HTMLSpanElement | null> {
  const avatarRef = useRef<HTMLSpanElement>(null);
  const requestedAvatarUriRef = useRef<string | null>(null);
  const previousThumbnailKindRef = useRef<string | null>(null);
  const mxcUri = avatar?.mxc_uri ?? null;
  const thumbnailKind = avatar?.thumbnail.kind ?? null;

  useEffect(() => {
    if (
      thumbnailKind === "notRequested" &&
      (previousThumbnailKindRef.current !== "notRequested" ||
        requestedAvatarUriRef.current !== mxcUri)
    ) {
      requestedAvatarUriRef.current = null;
    }
    previousThumbnailKindRef.current = thumbnailKind;
    if (
      !onRequestAvatarThumbnail ||
      !mxcUri ||
      thumbnailKind !== "notRequested" ||
      !avatarRef.current ||
      requestedAvatarUriRef.current === mxcUri ||
      typeof IntersectionObserver === "undefined"
    ) {
      return undefined;
    }

    const row = avatarRef.current;
    const requestUri = mxcUri;
    let disposed = false;
    let release: (() => void) | undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (
          requestedAvatarUriRef.current === requestUri ||
          !entries.some((entry) => entry.isIntersecting)
        ) {
          return;
        }
        requestedAvatarUriRef.current = requestUri;
        observer.disconnect();
        try {
          void Promise.resolve(onRequestAvatarThumbnail(requestUri))
            .then((nextRelease) => {
              if (disposed) nextRelease?.();
              else if (nextRelease) release = nextRelease;
            })
            .catch(() => undefined);
        } catch {
          // Demand admission failures are represented by the Rust state/event.
        }
      },
      { root: row.closest(".sidebar-scroll") }
    );
    observer.observe(row);
    return () => {
      disposed = true;
      observer.disconnect();
      release?.();
    };
  }, [mxcUri, onRequestAvatarThumbnail, thumbnailKind]);

  return avatarRef;
}

export function EntityAvatar({
  avatar,
  className,
  colorSeed,
  fallback,
  fallbackMode = "initials",
  onRequestAvatarThumbnail,
  sourceUrl
}: {
  avatar: RoomListItem["avatar"];
  className: string;
  colorSeed?: string | null;
  fallback: string;
  fallbackMode?: "initials" | "compactLabel" | "elementSpace";
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
  /** Explicit resource URL; null suppresses the unscoped thumbnail fallback. */
  sourceUrl?: string | null;
}) {
  const avatarRef = useVisibleAvatarThumbnailRequest(avatar, onRequestAvatarThumbnail);
  const resolvedSourceUrl =
    sourceUrl !== undefined
      ? sourceUrl
      : avatar?.thumbnail.kind === "ready"
        ? renderableThumbnailSourceUrl(avatar.thumbnail.source_ref)
        : null;
  const { displaySourceUrl, onImageError, onImageLoad } = useRecoverableImageSource(resolvedSourceUrl);
  const showImage = Boolean(displaySourceUrl);
  const colorClassName = avatarColorClass(colorSeed || fallback);
  const fallbackClassName =
    fallbackMode === "compactLabel"
      ? `avatar-fallback compact-label ${colorClassName}`
      : fallbackMode === "elementSpace"
        ? "avatar-fallback element-space"
        : `avatar-fallback ${colorClassName}`;
  const fallbackStyle =
    fallbackMode === "compactLabel"
      ? ({
          "--avatar-label-length": Math.max(graphemeCount(fallback), 1)
        } as CSSProperties)
      : undefined;
  const elementColor =
    fallbackMode === "elementSpace" ? elementAvatarColorIndex(colorSeed || fallback) : undefined;
  return (
    <span ref={avatarRef} className={className} aria-hidden="true">
      {showImage ? (
        <img
          src={displaySourceUrl ?? undefined}
          onError={onImageError}
          onLoad={onImageLoad}
        />
      ) : (
        <span
          className={fallbackClassName}
          data-color={elementColor}
          dir="auto"
          style={fallbackStyle}
        >
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

/**
 * The composer names its sender only when more than one account is signed in;
 * a single account needs no disambiguation. The Matrix ID is always included
 * because display names alone do not distinguish accounts.
 */
export function composerSendingAccount(
  tabs: readonly AccountTabSummary[] | undefined,
  selectedTabId: string | null
): { name: string; userId: string; colorClassName: string } | null {
  if (!tabs) return null;
  const signedInCount = tabs.filter(
    (tab) => tab.accountKey !== null && tab.status !== "signedOut"
  ).length;
  const selected = tabs.find((tab) => tab.id === selectedTabId);
  if (signedInCount <= 1 || !selected?.accountKey) return null;
  return {
    name: selected.displayName?.trim() || selected.accountKey,
    userId: selected.accountKey,
    colorClassName: avatarColorClass(selected.accountKey)
  };
}
