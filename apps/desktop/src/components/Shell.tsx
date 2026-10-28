import {
  type CSSProperties,
  type DragEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState
} from "react";
import {
  Activity,
  AlertTriangle,
  Bell,
  Bug,
  ChevronDown,
  ChevronsDownUp,
  Clock3,
  Compass,
  Home,
  MessageSquare,
  MoreHorizontal,
  Plus,
  Search,
  Settings,
  Sparkles,
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
  SidebarSectionBound,
  SidebarSectionKind
} from "../domain/types";
import { contextMenuItems, type ContextMenuActionId } from "../domain/contextMenus";
import { renderableThumbnailSourceUrl } from "../backend/linkMediaRuntime";
import {
  ROOM_ACCESS_CHECKING,
  roomAccessIndicator
} from "../domain/accessCondition";
import { RoomAccessGlyphIcon } from "./RoomAccessGlyph";
import { ContextMenuSurface, onMenuKeyDown } from "./ContextMenuSurface";
import { accountTabLabel } from "./accountSignOut";
import { FloatingLayer, floatingPlacementStyle, useFloatingPlacement } from "./floatingLayer";
import { useOutsidePressDismissal } from "./popupDismissal";
import { Tooltip, type TooltipTriggerProps } from "./Tooltip";
import { ImeTextField } from "./ImeTextControl";
import { useRecoverableImageSource } from "./avatarImage";
import { SpaceDescriptionPreview } from "./SpaceDescriptionPreview";
import { WorkspaceRailEdgeIndicator, useWorkspaceRailOverflow } from "./WorkspaceRailEdge";
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

// #1246: the Space reorder payload stays in the app. A `text/plain` item reaches
// the system pasteboard, where macOS turns the bare room ID into a broken web
// location when a drop leaves the window; an app-private type is opaque outside
// the webview. The in-app reorder reads `draggedSpaceId` first and uses this only
// as the cross-instance fallback.
const SPACE_DRAG_TYPE = "application/x-koushi-space";

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
 * #1363/#1366: the rows and the bound controls one section renders.
 *
 * #1387: each bound action has exactly one control, so the two are mutually
 * exclusive. While bounded, the end-of-list control expands the section; once
 * expanded, the fold control moves into the section header, because the end of
 * a section holding dozens of rows is no place to reach for.
 *
 * The bound is Rust-projected (`SidebarSectionBound`); React only slices the
 * matching section list and never derives the limit itself. Three things turn
 * the bound off, and each one only ever shows more rows:
 *
 * - a non-empty search filter is presentation state Rust cannot see, so it
 *   shows every match instead of hiding a match behind a control;
 * - a collapsed section renders no rows, so it renders no control either;
 * - a bound whose `total_count` does not describe the list in hand belongs to a
 *   different list (a stale fake snapshot), so the renderer shows every row
 *   rather than hiding a conversation.
 */
function boundedSection(
  rooms: RoomListItem[],
  bound: SidebarSectionBound | null | undefined,
  collapsed: boolean,
  filterActive: boolean
): {
  rooms: RoomListItem[];
  expandControl: SidebarSectionBound | null;
  foldControl: SidebarSectionBound | null;
} {
  if (collapsed || filterActive || !bound || bound.total_count !== rooms.length) {
    return { rooms, expandControl: null, foldControl: null };
  }
  const shown = bound.shows_expand_control ? bound : null;
  return {
    rooms: rooms.slice(0, bound.visible_count),
    expandControl: shown && !bound.expanded ? shown : null,
    foldControl: shown && bound.expanded ? shown : null
  };
}

/**
 * Accessible name for the Home rail button.
 *
 * The Home rail keeps unread messages and pending invitations in separate
 * visual badges. A quiet Home keeps the plain name.
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

/** #1504: an account tab context-menu action, always naming the tab it acts on. */
export type AccountTabAction = Extract<
  ContextMenuActionId,
  | "accountTabSettings"
  | "accountTabSignOut"
  | "accountTabSignOutAndRemove"
  | "accountTabSignInAgain"
  | "accountTabRemove"
>;

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
  onRemove,
  onAccountTabAction,
  showActivity = true
}: {
  tabs: AccountTabSummary[];
  selectedTabId: string | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
  onRemove: (id: string) => void;
  /**
   * #1504: actions from a tab's context menu. Each carries the clicked tab's id,
   * which need not be the selected tab. Without it, tabs open no context menu.
   */
  onAccountTabAction?: (tabId: string, action: AccountTabAction) => void;
  /**
   * #1458: the workspace strip shows each account's connection status, unread and
   * invitation counts. The Account Settings header keeps only account selection, so it
   * renders neither the indicators nor the status in the tab's accessible name.
   */
  showActivity?: boolean;
}) {
  const [tabMenu, setTabMenu] = useState<{ tabId: string; x: number; y: number } | null>(null);
  const menuTab = tabMenu ? tabs.find((tab) => tab.id === tabMenu.tabId) ?? null : null;
  const menuItems = menuTab ? contextMenuItems({ kind: "accountTab", status: menuTab.status }) : [];

  function openTabMenu(tab: AccountTabSummary, x: number, y: number) {
    if (!onAccountTabAction) return;
    if (contextMenuItems({ kind: "accountTab", status: tab.status }).length === 0) return;
    setTabMenu({ tabId: tab.id, x, y });
  }

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
              aria-label={
                showActivity ? t("accountTabs.select", { account: label, status }) : label
              }
              title={`${label}${tab.homeserver ? ` · ${shortHomeserverName(tab.homeserver)}` : ""}`}
              onClick={() => onSelect(tab.id)}
              onContextMenu={(event) => {
                if (!onAccountTabAction) return;
                event.preventDefault();
                openTabMenu(tab, event.clientX, event.clientY);
              }}
              onKeyDown={(event) => {
                if (event.key !== "ContextMenu" && !(event.key === "F10" && event.shiftKey)) return;
                if (!onAccountTabAction) return;
                event.preventDefault();
                const rect = event.currentTarget.getBoundingClientRect();
                openTabMenu(tab, rect.left, rect.bottom);
              }}
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
              {!showActivity ? null : tab.status === "ready" ? (
                <>
                  {tab.unreadCount > 0 ? (
                    <span className="account-tab-unread">{tab.unreadCount > 99 ? "99+" : tab.unreadCount}</span>
                  ) : (tab.inviteCount ?? 0) === 0 ? (
                    <span className="account-tab-ready-dot" role="img" aria-label={status} />
                  ) : null}
                  {(tab.inviteCount ?? 0) > 0 ? (
                    <span className="account-tab-invite" title={t("accountTabs.inviteCount", { count: tab.inviteCount ?? 0 })}>
                      {tab.inviteCount! > 99 ? "99+" : tab.inviteCount}
                    </span>
                  ) : null}
                </>
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
              <Tooltip label={removeLabel(tab, label)} placement="below" align="end">
                {(tooltipProps) => (
                  <button
                    className="account-tab-remove"
                    type="button"
                    aria-label={removeLabel(tab, label)}
                    onClick={() => onRemove(tab.id)}
                    {...tooltipProps}
                  >
                    <X size={ICON_SIZE.micro} aria-hidden="true" />
                  </button>
                )}
              </Tooltip>
            ) : null}
          </div>
        );
      })}
      <Tooltip label={t("accountTabs.addAccount")} placement="below">
        {(tooltipProps) => (
          <button
            className="account-tab-add"
            type="button"
            aria-label={t("accountTabs.addAccount")}
            onClick={onAdd}
            {...tooltipProps}
          >
            <Plus size={ICON_SIZE.control} aria-hidden="true" />
          </button>
        )}
      </Tooltip>
      {tabMenu && menuTab && onAccountTabAction ? (
        <FloatingLayer>
          {/* The strip sits in the draggable titlebar; keep menu presses from
              bubbling (through the portal) into a window drag. */}
          <div onMouseDown={(event) => event.stopPropagation()}>
            <ContextMenuSurface
              items={menuItems}
              x={tabMenu.x}
              y={tabMenu.y}
              onClose={() => setTabMenu(null)}
              onAction={(action) => {
                setTabMenu(null);
                onAccountTabAction(menuTab.id, action as AccountTabAction);
              }}
            />
          </div>
        </FloatingLayer>
      ) : null}
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
  onAccountTabAction,
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
  onAccountTabAction?: (tabId: string, action: AccountTabAction) => void;
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
        onAccountTabAction={onAccountTabAction}
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
  onAccountTabAction,
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
  onAccountTabAction?: (tabId: string, action: AccountTabAction) => void;
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
        onAccountTabAction={onAccountTabAction}
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
        <Tooltip label={t("settings.appSettings")} placement="below" align="end">
          {(tooltipProps) => (
            <button
              className="icon-button app-settings-button"
              type="button"
              aria-label={t("settings.appSettings")}
              onClick={onOpenAppSettings}
              {...tooltipProps}
            >
              <Settings size={ICON_SIZE.small} aria-hidden="true" />
            </button>
          )}
        </Tooltip>
        <Tooltip label={t("diagnostics.open")} placement="below" align="end">
          {(tooltipProps) => (
            <button
              className="icon-button"
              type="button"
              aria-label={t("diagnostics.open")}
              onClick={onOpenDiagnostics}
              {...tooltipProps}
            >
              <Bug size={ICON_SIZE.control} />
            </button>
          )}
        </Tooltip>
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
  onSelectSpace: (spaceId: string | null, input?: import("../backend/desktopApi").NavigationInput) => void;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const [draggedSpaceId, setDraggedSpaceId] = useState<string | null>(null);
  const [dragOverSpaceId, setDragOverSpaceId] = useState<string | null>(null);
  const spaceIds = snapshot.sidebar.space_rail.map((space) => space.space_id);
  const { listRef, overflow, onListScroll, revealSpace } = useWorkspaceRailOverflow(
    snapshot.sidebar.space_rail
  );

  function dropSpaceOn(targetSpaceId: string, event: DragEvent<HTMLButtonElement>) {
    event.preventDefault();
    const sourceSpaceId = draggedSpaceId ?? event.dataTransfer.getData(SPACE_DRAG_TYPE);
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
          {/* The Home rail button is deliberately tooltip-free: the accessibility
              name already carries the account home label, and e2e
              (`room-space-invites.spec.ts`, "Home rail tooltip is absent")
              asserts no tooltip and no `aria-describedby` here. */}
          <button
            className={`workspace-button workspace-system-button workspace-home-button ${
              snapshot.sidebar.account_home.is_active ? "is-active" : ""
            }`}
            data-count={snapshot.sidebar.account_home.unread_count || undefined}
            data-invite-count={snapshot.sidebar.account_home.invite_count || undefined}
            type="button"
            aria-label={accountHomeLabel(snapshot.sidebar.account_home)}
            aria-current={snapshot.sidebar.account_home.is_active ? "page" : undefined}
            onClick={(event) => onSelectSpace(null, !event.isTrusted ? "programmatic" : event.detail === 0 ? "keyboard" : "pointer")}
          >
            <Home size={ICON_SIZE.rail} />
          </button>
        </div>
        <div className="workspace-rail-separator" role="separator" aria-orientation="horizontal" />
        <div
          className="workspace-space-scroller"
          data-overflow-above={overflow.overflowAbove || undefined}
          data-overflow-below={overflow.overflowBelow || undefined}
        >
          <div
            className="workspace-list workspace-space-list"
            ref={listRef}
            onScroll={onListScroll}
          >
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
              // #1327: the overlay uses the same glyph vocabulary as every other
              // surface, so a padlock here also means encryption and nothing else.
              const spaceAccess =
                roomAccessIndicator(
                  space.access_join_rule ?? null,
                  space.access_restricted_conditions,
                  {
                    spaceMembersRoute: space.access_space_members_route,
                    allowedRoomNames: space.access_allowed_room_names
                  }
                ) ?? ROOM_ACCESS_CHECKING;
              const railSummary = spaceAccess.glyph;
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
                    data-space-id={space.space_id}
                    draggable
                    type="button"
                    aria-label={fallbackName}
                    aria-current={space.is_active ? "page" : undefined}
                    onClick={(event) => onSelectSpace(space.space_id, !event.isTrusted ? "programmatic" : event.detail === 0 ? "keyboard" : "pointer")}
                    onDragStart={(event) => {
                      setDraggedSpaceId(space.space_id);
                      event.dataTransfer.effectAllowed = "move";
                      event.dataTransfer.setData(SPACE_DRAG_TYPE, space.space_id);
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
                    {/* The unread count sits above the avatar; access stays below. */}
                    <span
                      className="workspace-access-overlay"
                      data-space-access={railSummary}
                      aria-hidden="true"
                    >
                      <RoomAccessGlyphIcon glyph={railSummary} size={ICON_SIZE.micro} />
                    </span>
                  </button>
                )}
              </Tooltip>
            );
            })}
          </div>
          {/* #1509: outside the scrollport, so the indicators neither scroll
              away, grow `scrollHeight`, nor become drag-reorder targets. */}
          {overflow.above ? (
            <WorkspaceRailEdgeIndicator edge="above" attention={overflow.above} onReveal={revealSpace} />
          ) : null}
          {overflow.below ? (
            <WorkspaceRailEdgeIndicator edge="below" attention={overflow.below} onReveal={revealSpace} />
          ) : null}
        </div>
      </div>
      <div className="rail-footer">
        <Tooltip label={t("action.createSpace")}>
          {(tooltipProps) => (
            <button
              className="rail-action"
              type="button"
              aria-label={t("action.createSpace")}
              onClick={onCreateSpace}
              {...tooltipProps}
            >
              <Plus size={ICON_SIZE.large} />
            </button>
          )}
        </Tooltip>
        <Tooltip label={t("workspace.userSettings")}>
          {(tooltipProps) => (
            <button
              className="user-presence"
              type="button"
              aria-label={t("workspace.userSettings")}
              onClick={onOpenUserSettings}
              onContextMenu={(event) =>
                onOpenContextMenu(event, { kind: "account" }, contextMenuItems({ kind: "account" }))
              }
              {...tooltipProps}
            />
          )}
        </Tooltip>
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
  onSelectRoom: (roomId: string, input?: import("../backend/desktopApi").NavigationInput) => void;
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
  // #1661: the active Space's confirmed description comes from the synced
  // Space projection, never from the transient Room Info settings slot.
  const activeSpaceTopic = activeSpace
    ? snapshot.state.domain.spaces.find((space) => space.space_id === activeSpace.space_id)?.topic ?? null
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
  // #1363/#1366: the row bound per section is Rust-projected; the renderer
  // slices with it and shows the control Rust asked for.
  const sectionBounds = snapshot.sidebar.section_bounds ?? null;
  const roomFilterActive = roomFilter.trim().length > 0;
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
    patch: { collapsed?: boolean; sort?: RoomListSort; expanded?: boolean }
  ) {
    onUpdateSettings({
      sidebar_section: { scope: scopeKey, section, ...patch }
    });
  }

  return (
    <aside className="sidebar" aria-label={t("workspace.rooms")}>
      <div className="workspace-header">
        <div className="workspace-header-title">
          {activeSpaceAccess ? (
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
                  data-space-access={activeSpaceAccess.glyph}
                  tabIndex={0}
                  {...triggerProps}
                >
                  <RoomAccessGlyphIcon glyph={activeSpaceAccess.glyph} size={ICON_SIZE.small} />
                </span>
              )}
            </Tooltip>
          ) : null}
          <div className="workspace-name" dir="auto">
            {activeSpaceName}
          </div>
          {activeSpaceAccess ? (
            <Tooltip
              label={roomAccessTooltipLabel(
                activeSpaceAccess.descriptionMessageId,
                activeSpaceAccess.descriptionAllowedRoomNames,
                activeSpaceAccess.descriptionSpaceName
              )}
            >
              {(triggerProps) => (
                <span className="workspace-access-badge" tabIndex={0} {...triggerProps}>
                  {t(activeSpaceAccess.labelMessageId)}
                </span>
              )}
            </Tooltip>
          ) : null}
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
        {activeSpace ? (
          <SpaceDescriptionPreview
            key={activeSpace.space_id}
            className="sidebar-space-description"
            text={activeSpaceTopic}
          />
        ) : null}
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
          bound={sectionBounds?.rooms}
          filterActive={roomFilterActive}
          sectionLabel={t("roomList.categoryRooms")}
          onToggleExpanded={(expanded) => updateSectionPreference("rooms", { expanded })}
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
          bound={sectionBounds?.dms}
          filterActive={roomFilterActive}
          sectionLabel={t("roomList.categoryDms")}
          onToggleExpanded={(expanded) => updateSectionPreference("dms", { expanded })}
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
            bound={sectionBounds?.low_priority}
            filterActive={roomFilterActive}
            sectionLabel={t("workspace.lowPriority")}
            onToggleExpanded={(expanded) => updateSectionPreference("lowPriority", { expanded })}
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
          <Tooltip label={t("roomList.clearFilter")} placement="below" align="end">
            {(tooltipProps) => (
              <button
                className="icon-button room-list-filter-clear"
                type="button"
                aria-label={t("roomList.clearFilter")}
                onClick={() => onFilterChange("")}
                {...tooltipProps}
              >
                <X size={ICON_SIZE.input} aria-hidden="true" />
              </button>
            )}
          </Tooltip>
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
  bound,
  filterActive = false,
  sectionLabel,
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
  onToggleExpanded,
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
  /** #1363/#1366: Rust-projected row bound for this section. */
  bound?: SidebarSectionBound | null;
  /** A non-empty search filter shows every match, whatever the bound says. */
  filterActive?: boolean;
  /** Accessible name for the bound control, naming this section. */
  sectionLabel?: string;
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
  onSelectRoom: (roomId: string, input?: import("../backend/desktopApi").NavigationInput) => void;
  onSelectSort?: (sort: RoomListSort) => void;
  onToggleCollapsed?: () => void;
  onToggleExpanded?: (expanded: boolean) => void;
  selectedSort?: RoomListSort;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const { rooms: renderedRooms, expandControl, foldControl } = boundedSection(
    rooms,
    bound,
    collapsed,
    filterActive
  );
  // #1387: expanding and folding swap the bound control between the row list and
  // the section header, so the button the user just pressed unmounts and the
  // focus falls to the document, leaving the section with nothing focused. Hand
  // it to the replacement. The handoff is armed only when the pressed control
  // itself held the focus, and it lands only while nothing else has taken the
  // focus in the meantime.
  //
  // ponytail: arbitration is per section. Two sections pressed before the
  // snapshot settles can hand the focus to the earlier one, and a pending
  // handoff survives a Home/Space switch. Both only redirect the focus onto a
  // bound control of a section the user just expanded; ordering them would need
  // one handoff owned by Sidebar instead of one per section.
  const sectionRef = useRef<HTMLElement>(null);
  const boundFocusHandoff = useRef<{
    from: "fold" | "expand" | null;
    to: "fold" | "expand";
  } | null>(null);
  const boundControlState = foldControl ? "fold" : expandControl ? "expand" : null;
  useLayoutEffect(() => {
    const handoff = boundFocusHandoff.current;
    if (handoff === null) return;
    // The snapshot the press asked for has not settled yet: keep waiting, and do
    // not read the focus, because the pressed control is still on screen.
    if (boundControlState === handoff.from) return;
    // The bound state moved. Any focus the user took somewhere else ends the
    // handoff, as does a state that brings no replacement control.
    boundFocusHandoff.current = null;
    if (document.activeElement !== document.body || boundControlState !== handoff.to) return;
    sectionRef.current
      ?.querySelector<HTMLButtonElement>(
        handoff.to === "fold"
          ? `.section-title-action[aria-controls="${id}-room-list"]`
          : ".room-section-bound"
      )
      ?.focus();
  }, [boundControlState, id]);
  const toggleBoundControl = (expanded: boolean, trigger: HTMLElement) => {
    boundFocusHandoff.current =
      document.activeElement === trigger
        ? { from: boundControlState, to: expanded ? "fold" : "expand" }
        : null;
    onToggleExpanded?.(expanded);
  };
  if (!showWhenEmpty && rooms.length === 0) {
    return null;
  }

  return (
    <section
      className="room-section"
      id={`${id}-room-list`}
      ref={sectionRef}
      data-room-section={id}
      aria-label={label}
    >
      {showHeader ? (
        <SectionTitle
          collapsed={collapsed}
          count={rooms.length}
          unreadCount={unreadCount}
          label={label}
          onCreate={onCreate}
          onAddExisting={onAddExisting}
          createLabel={kind === "dm" ? t("workspace.newDm") : t("action.createRoom")}
          foldControl={foldControl}
          onSelectSort={onSelectSort}
          sectionId={id}
          onToggle={onToggleCollapsed ?? (() => undefined)}
          onToggleExpanded={toggleBoundControl}
          selectedSort={selectedSort}
        />
      ) : null}
      {!collapsed
        ? rooms.length > 0
          ? (
            <>
              {renderedRooms.map((room) => (
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
              ))}
              {expandControl && onToggleExpanded ? (
                <SectionBoundControl
                  bound={expandControl}
                  listId={`${id}-room-list`}
                  sectionLabel={sectionLabel ?? label}
                  onToggleExpanded={toggleBoundControl}
                />
              ) : null}
            </>
          )
          : emptyMessage
            ? <div className="room-list-empty">{emptyMessage}</div>
            : null
        : null}
    </section>
  );
}

/**
 * #1363/#1366: the expand control at the end of a bounded section.
 *
 * A plain button that names the action, the hidden count, and the section it
 * belongs to, and reports its state through `aria-expanded` against the row
 * list it controls. #1387: this control only ever expands; folding back is the
 * section header's fold control.
 */
function SectionBoundControl({
  bound,
  listId,
  sectionLabel,
  onToggleExpanded
}: {
  bound: SidebarSectionBound;
  listId: string;
  sectionLabel: string;
  onToggleExpanded: (expanded: boolean, trigger: HTMLElement) => void;
}) {
  const label = t("roomList.showMoreInSection", {
    count: bound.hidden_count,
    section: sectionLabel
  });
  return (
    <button
      className="room-section-bound"
      type="button"
      aria-controls={listId}
      aria-expanded={false}
      aria-label={label}
      title={label}
      onClick={(event) => onToggleExpanded(true, event.currentTarget)}
    >
      {t("roomList.showMore", { count: bound.hidden_count })}
    </button>
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
    <Tooltip label={label} placement="below">
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

/**
 * #1369: the section menu floats above the sidebar but belongs to it, so the
 * sidebar's scroll viewport is the boundary. The menu then stays inside the
 * sidebar instead of drifting over the panes beside it, and — because the layer
 * positions it `fixed` — it never extends the sidebar's scrollable content.
 */
function sectionMenuBoundaryElement(anchor: Element): Element | null {
  return anchor.closest(".sidebar-scroll");
}

function SectionTitle({
  collapsed,
  count,
  createLabel,
  foldControl,
  label,
  onCreate,
  onAddExisting,
  onSelectSort,
  onToggle,
  onToggleExpanded,
  sectionId,
  selectedSort,
  unreadCount
}: {
  collapsed: boolean;
  count: number;
  createLabel?: string;
  /** #1387: present only while Rust reports the section bounded and expanded. */
  foldControl?: SidebarSectionBound | null;
  label: string;
  unreadCount?: number;
  onCreate?: () => void;
  onAddExisting?: () => void;
  onSelectSort?: (sort: RoomListSort) => void;
  onToggle: () => void;
  onToggleExpanded?: (expanded: boolean, trigger: HTMLElement) => void;
  sectionId: string;
  selectedSort?: RoomListSort;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  // One label for the tooltip and the accessible name, so the button can never
  // explain itself with one word and announce another.
  const createActionLabel = createLabel ?? t("action.createRoom");
  const menuTriggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  // #1369: the menu renders in the shared body-level floating layer, because
  // `.sidebar-scroll` clips an absolutely positioned menu and adds it to the
  // sidebar's own scroll extent. The sidebar scroll viewport is still the
  // boundary, so the menu stays inside the sidebar instead of drifting over the
  // panes beside it. Its rows decide its height, so the layer measures the menu
  // itself instead of taking a caller size.
  const menuPlacement = useFloatingPlacement({
    anchorRef: menuTriggerRef,
    align: "end",
    contentRef: menuRef,
    placement: "below",
    resolveBoundaryElement: sectionMenuBoundaryElement
  });
  // #1365/#1368: the menu is a non-modal popup with no backdrop, so the shared
  // owner dismisses it on an outside press and keeps one section menu open at a
  // time. The press is not swallowed: it still selects the room under it.
  useOutsidePressDismissal({
    open: menuOpen,
    kind: "sidebar-section-menu",
    contentRef: menuRef,
    triggerRef: menuTriggerRef,
    onDismiss: () => setMenuOpen(false)
  });
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
  // Focus enters the menu on open, so the arrow keys rove it like the shared
  // context menu; Escape returns focus to the trigger (above). #1369: the menu is
  // `visibility: hidden` until the floating layer has placed it, and a hidden
  // element cannot take focus, so this waits for the resolved placement instead
  // of reading an unplaced menu. It runs once per opening, never on a later
  // placement change, so a scroll cannot pull the roving focus back to the top.
  const menuFocusEntered = useRef(false);
  useEffect(() => {
    if (!menuOpen) {
      menuFocusEntered.current = false;
      return;
    }
    if (!menuPlacement || menuFocusEntered.current) return;
    menuFocusEntered.current = true;
    menuRef.current
      ?.querySelector<HTMLElement>('[role^="menuitem"]')
      ?.focus();
  }, [menuOpen, menuPlacement]);

  // #542: Recent is the product order and Name stays locale-aware. The
  // attention-first `activity` value is still deserialized for a settings
  // document an older build persisted, but it is no longer selectable.
  const sortOptions: Array<[RoomListSort, string]> = [
    [{ kind: "recentFirst" }, t("roomList.sortRecent")],
    [{ kind: "normalLocale" }, t("roomList.sortName")]
  ];

  return (
    <div
      className="section-title"
    >
      {/* #1486: folding an expanded section back to its bound is a view
          control, like the heading's own collapse toggle, so the two sit
          together at the start of the row. The right group keeps the actions
          that create content or open the section menu. */}
      <div className="section-title-lead">
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
        {foldControl && onToggleExpanded ? (
          // #1486: the control sits at the start of the heading, so its bubble
          // opens towards the rows instead of over the space rail.
          <Tooltip label={t("roomList.showFewer")} placement="below" align="start">
            {(tooltipProps) => (
              <button
                className="section-title-action"
                type="button"
                aria-controls={`${sectionId}-room-list`}
                aria-expanded={true}
                aria-label={t("roomList.showFewerInSection", { section: label })}
                onClick={(event) => onToggleExpanded(false, event.currentTarget)}
                {...tooltipProps}
              >
                <ChevronsDownUp size={ICON_SIZE.compact} aria-hidden="true" />
              </button>
            )}
          </Tooltip>
        ) : null}
      </div>
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
          <Tooltip label={createActionLabel} placement="below" align="end">
            {(tooltipProps) => (
              <button
                className="section-title-action"
                type="button"
                aria-label={createActionLabel}
                onClick={onCreate}
                {...tooltipProps}
              >
                <Plus size={ICON_SIZE.compact} aria-hidden="true" />
              </button>
            )}
          </Tooltip>
        ) : null}
        {onSelectSort && selectedSort ? (
          <span>
            <Tooltip
              label={t("roomList.sectionOptions", { section: label })}
              placement="below"
              align="end"
            >
              {(tooltipProps) => (
                <button
                  ref={menuTriggerRef}
                  className="section-title-action"
                  type="button"
                  aria-label={t("roomList.sectionOptions", { section: label })}
                  aria-expanded={menuOpen}
                  onClick={() => setMenuOpen((open) => !open)}
                  {...tooltipProps}
                >
                  <MoreHorizontal size={ICON_SIZE.compact} aria-hidden="true" />
                </button>
              )}
            </Tooltip>
            {menuOpen ? (
              <FloatingLayer>
                <div
                  className="section-menu"
                  ref={menuRef}
                  role="menu"
                  aria-label={t("roomList.sectionOptions", { section: label })}
                  style={floatingPlacementStyle(menuPlacement, false)}
                  onKeyDown={(event) => onMenuKeyDown(event, event.currentTarget)}
                >
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
                  {/* A menu's own label is not a menu item. */}
                  <div className="section-menu-title" role="presentation">{t("roomList.sort")}</div>
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
              </FloatingLayer>
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
  onSelectRoom: (roomId: string, input?: import("../backend/desktopApi").NavigationInput) => void;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const sourceRoom = roomById.get(room.room_id);
  const dmUserIds = sourceRoom?.dm_user_ids ?? [];
  const dmUserId =
    kind === "dm" && sourceRoom?.is_dm && dmUserIds.length === 1
      ? dmUserIds[0]
      : null;
  const isOnlineDm = dmUserId ? presence[dmUserId] === "online" : false;
  const displayCount = room.display_count ?? room.unread_count;
  // #1327: a joined ordinary room row shows its own access condition; while it
  // has not been projected the row says so instead of guessing. A DM row shows
  // neither an encryption nor an access indicator, and lanes without a joined
  // condition (invitations, not-joined) render none. (#1166)
  const access =
    kind === "room"
      ? roomAccessIndicator(room.access_join_rule, room.access_restricted_conditions, {
          spaceMembersRoute: room.access_space_members_route,
          allowedRoomNames: room.access_allowed_room_names
        }) ?? ROOM_ACCESS_CHECKING
      : null;
  const roomLabel = roomListItemLabel(room);
  // #1166: the condition is announced as the row's *description*, so the row's
  // accessible name stays exactly the room label other surfaces and tests match
  // on. The visible icon stays the sighted affordance.
  // #1327: the row is the one tab stop, so its own hover and focus reveal the
  // popup and no extra tab stop is added inside it.
  const accessDescriptionId = useId();
  const accessDescription = access
    ? roomAccessTooltipLabel(
        access.descriptionMessageId,
        access.descriptionAllowedRoomNames,
        access.descriptionSpaceName
      )
    : null;
  // #1410: a not-joined child the Space marks as suggested carries a marker
  // before its name; like the access condition, it is announced as the row's
  // description so the row's accessible name stays the room label.
  const suggestedDescriptionId = useId();
  const suggested = kind === "notJoined" && room.suggested === true;
  const rowDescriptionIds = [
    access ? accessDescriptionId : null,
    suggested ? suggestedDescriptionId : null
  ].filter((id): id is string => id !== null);
  const tooltipLabel = accessDescription ?? (suggested ? t("roomList.suggestedDescription") : null);

  function renderRow(triggerProps?: TooltipTriggerProps) {
    return (
    <button
      className={`room-item ${room.room_id === activeRoomId ? "is-active" : ""}`}
      aria-label={sourceRoom?.marked_unread && displayCount === 0 ? `${roomLabel}, ${t("room.manuallyMarkedUnread")}` : roomLabel}
      data-access={access?.glyph ?? undefined}
      data-room-kind={kind}
      data-testid="room-item"
      type="button"
      onClick={(event) => {
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
        onSelectRoom(room.room_id, !event.isTrusted ? "programmatic" : event.detail === 0 ? "keyboard" : "pointer");
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
            dmUserIds: dmUserId ? [dmUserId] : [],
            canMarkRoomAsRead:
              ((room.main_unread_count ?? room.unread_count) > 0 || sourceRoom?.marked_unread === true) &&
              Boolean(sourceRoom?.latest_event?.event_id) &&
              !sourceRoom?.latest_event?.is_redacted &&
              !sourceRoom?.latest_event?.relation_type &&
              !sourceRoom?.latest_event?.thread_root_event_id
          })
        );
      }}
      {...triggerProps}
      // Always the full condition text: an open tooltip renders the same
      // sentence, and the description stays reachable after the popup closes.
      aria-describedby={rowDescriptionIds.length > 0 ? rowDescriptionIds.join(" ") : undefined}
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
          {accessDescription}
        </span>
      ) : null}
      {suggested ? (
        <span className="sr-only" id={suggestedDescriptionId}>
          {t("roomList.suggestedDescription")}
        </span>
      ) : null}
      {/* #1166: the name and the access icon share the grid's name cell so the
          avatar and the trailing unread area keep their columns.
          #1247: the access icon follows the name so every name keeps one left
          edge regardless of whether an indicator is present.
          #1327: the row shows exactly one compact access glyph after the name;
          the full localized label stays its accessible name, and the popup and
          the row description carry the explanation. */}
      <span className="room-name-shell">
        {suggested ? (
          <span
            className="room-suggested-icon"
            data-testid="room-suggested-icon"
            role="img"
            aria-label={t("roomList.suggested")}
          >
            <Sparkles size={ICON_SIZE.access} aria-hidden="true" />
          </span>
        ) : null}
        <span className="room-name" dir="auto">{roomLabel}</span>
        {access ? (
          <span
            className="room-access-icon"
            data-room-access={access.glyph}
            role="img"
            aria-label={t(access.labelMessageId)}
          >
            <RoomAccessGlyphIcon glyph={access.glyph} />
          </span>
        ) : null}
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
        {displayCount > 0 ? (
          <span
            className="room-count"
            title={t(room.is_muted ? "room.mutedUnreadBreakdown" : "room.unreadBreakdown", {
              main: room.main_unread_count ?? 0,
              threads: room.thread_unread_count ?? 0,
            })}
          >
            {displayCount}
          </span>
        ) : null}
        {displayCount === 0 && sourceRoom?.marked_unread ? (
          <span className="room-unread-dot" title={t("room.manuallyMarkedUnread")} />
        ) : null}
      </span>
    </button>
    );
  }

  if (!tooltipLabel) {
    return renderRow();
  }
  return (
    <Tooltip className="room-item-host" label={tooltipLabel}>
      {(triggerProps) => renderRow(triggerProps)}
    </Tooltip>
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
