import {
  type CSSProperties,
  type DragEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState
} from "react";
import {
  AlertTriangle,
  Bell,
  Bug,
  ChevronDown,
  Clock3,
  Compass,
  Edit3,
  HelpCircle,
  Home,
  MessageCircle,
  MessageSquare,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Users,
  X
} from "lucide-react";
import { t } from "../i18n/messages";
import type {
  AccountHomeItem,
  CurrentSessionStatusState,
  DesktopSnapshot,
  DisplayPlatform,
  RoomListItem,
  RoomSummary,
  SearchScopeKind,
  SessionStatusRefreshTrigger
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import { toExternalHttpUrl } from "../domain/externalLinks";
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
  elementAvatarColorIndex,
  elementAvatarInitial,
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

export type RuntimeAlertKind = "secureBackup" | "sync" | "session";

export interface RuntimeAlert {
  kind: RuntimeAlertKind;
  severity: "warning" | "error";
  title: string;
  detail: string;
  retryable: boolean;
}

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

function filterSidebarRooms(rooms: RoomListItem[], query: string): RoomListItem[] {
  const normalized = query.trim().toLocaleLowerCase();
  return normalized.length === 0
    ? rooms
    : rooms.filter((room) => room.display_name.toLocaleLowerCase().includes(normalized));
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

export function TopBar({
  accountManagementUrl,
  activeRoomName = null,
  activeSpaceName,
  currentSessionStatus = { status: "idle" },
  deviceId = null,
  homeserver,
  isBusy,
  platform = "linux",
  searchInputRef,
  searchQuery,
  searchScope,
  sync,
  userId = null,
  onManageAccount = () => undefined,
  onCopyDiagnostics = async () => undefined,
  onOpenKeyboardSettings,
  onOpenDiagnostics = () => undefined,
  onRefreshCurrentSessionStatus = () => undefined,
  onRetryRuntimeAlert = () => undefined,
  onRestartSync,
  onSearchQueryChange,
  onSearchScopeChange,
  onStartWindowDrag = () => undefined,
  runtimeAlertRetrying = false,
  runtimeAlerts = []
}: {
  accountManagementUrl?: string | null;
  activeRoomName?: string | null;
  activeSpaceName: string;
  currentSessionStatus?: CurrentSessionStatusState;
  deviceId?: string | null;
  homeserver?: string | null;
  isBusy: boolean;
  platform?: DisplayPlatform;
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchQuery: string;
  searchScope: SearchScopeKind;
  sync: DesktopSnapshot["state"]["domain"]["sync"];
  userId?: string | null;
  onManageAccount?: (safeExternalUrl: string) => void;
  onCopyDiagnostics?: () => Promise<void>;
  onOpenKeyboardSettings: () => void;
  onOpenDiagnostics?: () => void;
  onRefreshCurrentSessionStatus?: (trigger: SessionStatusRefreshTrigger) => void;
  onRetryRuntimeAlert?: (kind: RuntimeAlertKind) => void;
  onRestartSync: () => void;
  onSearchQueryChange: (value: string) => void;
  onSearchScopeChange: (value: SearchScopeKind) => void;
  onStartWindowDrag?: () => void;
  runtimeAlertRetrying?: boolean;
  runtimeAlerts?: RuntimeAlert[];
}) {
  const [sessionStatusOpen, setSessionStatusOpen] = useState(false);
  const sessionStatusHostRef = useRef<HTMLDivElement>(null);
  const sessionStatusTriggerRef = useRef<HTMLButtonElement>(null);
  const syncStatus = syncStatePresentation(sync);
  const serverLabel = matrixServerLabel(homeserver);
  const safeAccountManagementUrl = toExternalHttpUrl(accountManagementUrl);
  const syncAriaLabel = serverLabel
    ? `${serverLabel} · ${syncStatus.ariaLabel}`
    : syncStatus.ariaLabel;
  const runtimeAlertSeverity = runtimeAlerts.some((alert) => alert.severity === "error")
    ? "error"
    : "warning";
  const runtimeAlertsLabel = t(
    runtimeAlerts.length === 1
      ? "sessionStatus.runtimeWarningCount"
      : "sessionStatus.runtimeWarningsCount",
    {
      count: String(runtimeAlerts.length)
    }
  );
  const sessionStatusLabel = runtimeAlerts.length
    ? t(
        runtimeAlerts.length === 1
          ? "sessionStatus.openWithRuntimeWarning"
          : "sessionStatus.openWithRuntimeWarnings",
        {
          count: String(runtimeAlerts.length)
        }
      )
    : t("sessionStatus.open");

  function closeSessionStatus() {
    setSessionStatusOpen(false);
    sessionStatusTriggerRef.current?.focus();
  }

  function openSessionStatus() {
    setSessionStatusOpen(true);
    onRefreshCurrentSessionStatus("open");
  }

  useEffect(() => {
    if (!sessionStatusOpen) {
      return undefined;
    }
    function onKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeSessionStatus();
      }
    }
    function onPointerDown(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        !sessionStatusHostRef.current?.contains(event.target)
      ) {
        closeSessionStatus();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [sessionStatusOpen]);

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
      <label className="top-search">
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
      </select>
      <div className="top-actions">
        <div className="session-status-host" ref={sessionStatusHostRef}>
          <button
            ref={sessionStatusTriggerRef}
            className="sync-status"
            data-sync-state={syncStatus.state}
            type="button"
            aria-label={sessionStatusLabel}
            aria-expanded={sessionStatusOpen}
            aria-haspopup="dialog"
            onClick={() => {
              if (sessionStatusOpen) {
                closeSessionStatus();
              } else {
                openSessionStatus();
              }
            }}
          >
            <span
              className="sync-status-content"
              data-sync-state={syncStatus.state}
              role="status"
              aria-live="polite"
              aria-label={syncAriaLabel}
            >
              <span className={`sync-dot ${isBusy ? "busy" : ""}`} aria-hidden="true" />
              {serverLabel ? <span className="sync-status-server">{serverLabel}</span> : null}
              <span className="sync-status-label">{syncStatus.label}</span>
              {syncStatus.detail ? (
                <span className="sync-status-detail">{syncStatus.detail}</span>
              ) : null}
            </span>
            {runtimeAlerts.length ? (
              <span
                className="runtime-alert-indicator"
                data-runtime-alert-severity={runtimeAlertSeverity}
                role="img"
                aria-label={runtimeAlertsLabel}
              >
                <AlertTriangle size={ICON_SIZE.micro} aria-hidden="true" />
              </span>
            ) : null}
            <ChevronDown size={ICON_SIZE.micro} aria-hidden="true" />
          </button>
          {sessionStatusOpen ? (
            <SessionStatusPopover
              accountManagementUrl={safeAccountManagementUrl}
              currentSessionStatus={currentSessionStatus}
              deviceId={deviceId}
              homeserver={serverLabel ?? homeserver ?? null}
              userId={userId}
              onManageAccount={onManageAccount}
              onCopyDiagnostics={onCopyDiagnostics}
              onOpenDiagnostics={onOpenDiagnostics}
              onRefresh={onRefreshCurrentSessionStatus}
              runtimeAlertRetrying={runtimeAlertRetrying}
              onRetryRuntimeAlert={onRetryRuntimeAlert}
              runtimeAlerts={runtimeAlerts}
            />
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

function SessionStatusPopover({
  accountManagementUrl,
  currentSessionStatus,
  deviceId,
  homeserver,
  userId,
  onManageAccount,
  onCopyDiagnostics,
  onOpenDiagnostics,
  onRefresh,
  runtimeAlertRetrying,
  onRetryRuntimeAlert,
  runtimeAlerts
}: {
  accountManagementUrl: string | null;
  currentSessionStatus: CurrentSessionStatusState;
  deviceId: string | null;
  homeserver: string | null;
  userId: string | null;
  onManageAccount: (safeExternalUrl: string) => void;
  onCopyDiagnostics: () => Promise<void>;
  onOpenDiagnostics: () => void;
  onRefresh: (trigger: SessionStatusRefreshTrigger) => void;
  runtimeAlertRetrying: boolean;
  onRetryRuntimeAlert: (kind: RuntimeAlertKind) => void;
  runtimeAlerts: RuntimeAlert[];
}) {
  const dialogRef = useRef<HTMLElement>(null);
  const [copyState, setCopyState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const details =
    currentSessionStatus.status === "ready" ? currentSessionStatus.details : null;
  const displayedDeviceId = details?.device_id ?? deviceId;
  const checking = currentSessionStatus.status === "checking";
  const retryLabel =
    currentSessionStatus.status === "failed"
      ? t("sessionStatus.retry")
      : t("sessionStatus.recheck");

  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  async function copyDiagnostics() {
    setCopyState("copying");
    try {
      await onCopyDiagnostics();
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  return (
    <section
      ref={dialogRef}
      className="session-status-popover"
      role="dialog"
      aria-label={t("sessionStatus.title")}
      tabIndex={-1}
    >
      <div className="session-status-heading">
        <strong>{t("sessionStatus.title")}</strong>
        <span data-session-status={currentSessionStatus.status}>
          {sessionStatusVerdict(currentSessionStatus)}
        </span>
      </div>
      {currentSessionStatus.status === "failed" ? (
        <>
          <p className="session-status-failure">
            {sessionStatusFailureLabel(currentSessionStatus.kind)}
          </p>
          <dl className="session-status-facts">
            <SessionStatusFact label={t("sessionStatus.homeserver")} value={homeserver} />
            <SessionStatusFact label={t("sessionStatus.userId")} value={userId} />
            <SessionStatusFact label={t("sessionStatus.deviceId")} value={displayedDeviceId} />
          </dl>
        </>
      ) : (
        <dl className="session-status-facts">
          <SessionStatusFact label={t("sessionStatus.homeserver")} value={homeserver} />
          <SessionStatusFact label={t("sessionStatus.userId")} value={userId} />
          <SessionStatusFact
            label={t("sessionStatus.deviceName")}
            value={details?.device_display_name}
          />
          <SessionStatusFact label={t("sessionStatus.deviceId")} value={displayedDeviceId} />
          <SessionStatusFact
            label={t("sessionStatus.authentication")}
            value={details ? authenticationMethodLabel(details.authentication_method) : null}
          />
          <SessionStatusFact
            label={t("sessionStatus.sync")}
            value={details ? sessionSyncLabel(details.sync_state) : null}
          />
          <SessionStatusFact
            label={t("sessionStatus.verification")}
            value={details ? verificationLabel(details.verification) : null}
          />
          <SessionStatusFact
            label={t("sessionStatus.ownerCrossSigning")}
            value={
              details
                ? details.is_cross_signed_by_owner
                  ? t("sessionStatus.crossSigned")
                  : t("sessionStatus.notCrossSigned")
                : null
            }
          />
          <SessionStatusFact
            label={t("sessionStatus.identity")}
            value={details ? identityVerificationLabel(details.own_identity_verification) : null}
          />
          <SessionStatusFact
            label={t("sessionStatus.keyBackup")}
            value={details ? keyBackupLabel(details.key_backup) : null}
          />
          <SessionStatusFact
            label={t("sessionStatus.lastChecked")}
            value={
              details
                ? new Intl.DateTimeFormat(undefined, {
                    dateStyle: "medium",
                    timeStyle: "short"
                  }).format(details.checked_at_ms)
                : null
            }
          />
        </dl>
      )}
      {runtimeAlerts.length ? (
        <section className="runtime-alerts" aria-labelledby="runtime-warnings-title">
          <h2 id="runtime-warnings-title">{t("sessionStatus.runtimeWarnings")}</h2>
          <ul>
            {runtimeAlerts.map((alert) => (
              <li key={alert.kind} data-runtime-alert-severity={alert.severity}>
                <strong>{alert.title}</strong>
                <p>{alert.detail}</p>
                {alert.retryable ? (
                  <button
                    type="button"
                    disabled={runtimeAlertRetrying}
                    onClick={() => onRetryRuntimeAlert(alert.kind)}
                  >
                    {alert.kind === "secureBackup"
                      ? t("gate.secureBackupRetry")
                      : t("sessionStatus.retry")}
                  </button>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <div className="session-status-actions">
        <button
          type="button"
          disabled={checking}
          onClick={() => onRefresh("manual")}
        >
          {checking ? t("sessionStatus.checking") : retryLabel}
        </button>
        <button
          type="button"
          disabled={!displayedDeviceId}
          onClick={() => {
            if (displayedDeviceId) {
              void navigator.clipboard?.writeText(displayedDeviceId);
            }
          }}
        >
          {t("sessionStatus.copyDeviceId")}
        </button>
        {accountManagementUrl ? (
          <button type="button" onClick={() => onManageAccount(accountManagementUrl)}>
            {t("sessionStatus.manageAccount")}
          </button>
        ) : null}
        <button type="button" onClick={onOpenDiagnostics}>
          {t("diagnostics.open")}
        </button>
        <button type="button" disabled={copyState === "copying"} onClick={() => void copyDiagnostics()}>
          {copyState === "copying" ? t("diagnostics.copying") : t("diagnostics.copy")}
        </button>
      </div>
      {copyState !== "idle" && copyState !== "copying" ? (
        <p className="session-status-copy-feedback" aria-live="polite">
          {copyState === "copied" ? t("diagnostics.copied") : t("diagnostics.copyFailed")}
        </p>
      ) : null}
    </section>
  );
}

function SessionStatusFact({ label, value }: { label: string; value: string | null | undefined }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd dir="auto">{value ?? t("sessionStatus.unavailable")}</dd>
    </div>
  );
}

function sessionStatusVerdict(status: CurrentSessionStatusState): string {
  switch (status.status) {
    case "idle":
      return t("sessionStatus.notChecked");
    case "checking":
      return t("sessionStatus.checking");
    case "ready":
      return verificationLabel(status.details.verification);
    case "failed":
      return t("sessionStatus.failed");
  }
}

function sessionStatusFailureLabel(kind: "sdk" | "timed_out" | "unavailable"): string {
  switch (kind) {
    case "sdk":
      return t("sessionStatus.failureSdk");
    case "timed_out":
      return t("sessionStatus.failureTimedOut");
    case "unavailable":
      return t("sessionStatus.failureUnavailable");
  }
}

function authenticationMethodLabel(method: string): string {
  switch (method) {
    case "password":
      return t("sessionStatus.authPassword");
    case "sso":
      return t("sessionStatus.authSso");
    case "oauth":
      return t("sessionStatus.authOauth");
    case "token":
      return t("sessionStatus.authToken");
    default:
      return t("sessionStatus.unknown");
  }
}

function sessionSyncLabel(state: string): string {
  switch (state) {
    case "running":
      return t("sessionStatus.syncRunning");
    case "starting":
      return t("sessionStatus.syncStarting");
    case "error":
      return t("sessionStatus.syncError");
    default:
      return t("sessionStatus.syncStopped");
  }
}

function verificationLabel(state: "verified" | "unverified" | "unknown"): string {
  if (state === "unknown") return t("trust.statusUnknown");
  return state === "verified"
    ? t("sessionStatus.verified")
    : t("sessionStatus.unverified");
}

function identityVerificationLabel(state: "missing" | "unverified" | "verified"): string {
  switch (state) {
    case "verified":
      return t("sessionStatus.identityVerified");
    case "unverified":
      return t("sessionStatus.identityUnverified");
    case "missing":
      return t("sessionStatus.identityMissing");
  }
}

function keyBackupLabel(state: "ready" | "disabled" | "unknown"): string {
  switch (state) {
    case "ready":
      return t("sessionStatus.backupReady");
    case "disabled":
      return t("sessionStatus.backupDisabled");
    case "unknown":
      return t("sessionStatus.unknown");
  }
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
          <button
            className={`workspace-button workspace-system-button workspace-home-button ${
              snapshot.sidebar.account_home.is_active ? "is-active" : ""
            }`}
            data-count={snapshot.sidebar.account_home.attention_count || undefined}
            type="button"
            aria-label={accountHomeLabel(snapshot.sidebar.account_home)}
            onClick={() => onSelectSpace(null)}
          >
            <Home size={ICON_SIZE.rail} />
          </button>
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
            const fallbackName = displayName.trim() || space.space_id || "?";
            return (
            <Tooltip label={fallbackName} key={space.space_id}>
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
                    fallback={localIcon || elementAvatarInitial(fallbackName) || "?"}
                    fallbackMode={localIcon ? "compactLabel" : "elementSpace"}
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
  onOpenInvites,
  onOpenThreads = () => undefined,
  onOpenSpaceInfo,
  onOpenSpaceMembers = () => undefined,
  spaceMemberCounts,
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
  onOpenInvites: () => void;
  onOpenThreads?: () => void;
  onOpenSpaceInfo: () => void;
  onOpenSpaceMembers?: () => void;
  spaceMemberCounts?: { joined: number; childOnly: number };
  onJoinRoom?: (roomId: string) => void;
  onSelectRoom: (roomId: string) => void;
}) {
  const sections = roomListSections(
    snapshot.state.ui.room_list,
    snapshot.state.ui.navigation.active_space_id,
    snapshot.state.domain.spaces,
    snapshot.state.domain.rooms,
    snapshot.state.domain.invites,
    snapshot.state.domain.room_notification_settings
  );
  const roomListReadiness = snapshot.state.ui.room_list.readiness;
  const roomListReady = roomListReadiness.kind === "ready";
  const hasProvisionalRoomList =
    snapshot.sidebar.space_rooms.length > 0 ||
    snapshot.sidebar.global_dms.length > 0 ||
    sections.notJoined.length > 0;
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
  const [roomFilter, setRoomFilter] = useState("");
  const activeSpaceId = snapshot.state.ui.navigation.active_space_id;
  const roomCategoryRooms = roomCategory === "dms" ? snapshot.sidebar.global_dms : sections.rooms;
  const sortedCategoryRooms = sortedSidebarRooms(roomCategoryRooms, roomSort);
  const visibleCategoryRooms = filterSidebarRooms(sortedCategoryRooms, roomFilter);
  const visibleNotJoinedRooms =
    accountHomeActive || roomCategory !== "rooms"
      ? []
      : sortedSidebarRooms(sections.notJoined, roomSort);
  const visibleCategoryLabel =
    roomCategory === "dms" ? t("workspace.people") : t("workspace.rooms");
  const visibleCategoryKind = roomCategory === "dms" ? "dm" : "room";
  const visibleCategoryId = roomCategory === "dms" ? "people" : "rooms";
  const resolvedSpaceMemberCounts = spaceMemberCounts ?? {
    joined: snapshot.state.domain.space_members.space_joined.length,
    childOnly: snapshot.state.domain.space_members.child_room_only.length
  };

  useEffect(() => {
    setRoomFilter("");
  }, [activeSpaceId]);

  function selectRoomCategory(category: SidebarRoomCategory) {
    setRoomCategory(category);
    setRoomFilter("");
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
        <div className="workspace-header-title">
          <div className="workspace-name" dir="auto">
            {activeSpaceName}
          </div>
        </div>
        <div className="workspace-header-actions no-wrap">
          {activeSpace ? (
            <SpaceMembersNavButton
              childOnlyCount={resolvedSpaceMemberCounts.childOnly}
              joinedCount={resolvedSpaceMemberCounts.joined}
              onClick={onOpenSpaceMembers}
            />
          ) : null}
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
            aria-label={t("threads.title")}
            onClick={onOpenThreads}
          >
            <MessageSquare size={ICON_SIZE.control} />
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
      </div>
      <div className="sidebar-scroll">
        {/* #330: Activity, Explore, and Invites are account-global, so they live
            under Home only. A space sidebar is the room list for that space; its
            space-scoped actions are the header icons above. Room threads are
            reached from the room header, where "this room" is already implied. */}
        {accountHomeActive ? (
          <>
            <NavButton
              active={activeView === "activity"}
              icon={<Clock3 size={ICON_SIZE.control} />}
              label={t("workspace.activity")}
              onClick={onOpenActivity}
            />
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
          </>
        ) : null}
        {!roomListReady ? (
          <div className="room-list-status" role="status">
            {roomListReadiness.kind === "failed" ? t("roomList.failed") : t("roomList.loading")}
          </div>
        ) : null}
        {roomListReady || hasProvisionalRoomList ? (
          <RoomListControls
            dmTotal={snapshot.sidebar.global_dms.length}
            dmUnread={snapshot.sidebar.dm_unread_count}
            dmHighlights={snapshot.sidebar.dm_highlight_count}
            roomTotal={snapshot.sidebar.space_rooms.length}
            roomUnread={snapshot.sidebar.space_unread_count}
            roomHighlights={snapshot.sidebar.space_highlight_count}
            selectedCategory={roomCategory}
            selectedSort={roomSort}
            filter={roomFilter}
            filterPlaceholder={
              roomCategory === "dms"
                ? t("roomList.filterDmsPlaceholder")
                : t("roomList.filterRoomsPlaceholder")
            }
            onFilterChange={setRoomFilter}
            onSelectCategory={selectRoomCategory}
            onSelectSort={selectRoomSort}
          />
        ) : null}
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
        {roomCategoryRooms.length > 0 && visibleCategoryRooms.length === 0 ? (
          <div className="room-list-no-matches" role="status">
            {t(roomCategory === "dms" ? "roomList.noMatchingDms" : "roomList.noMatchingRooms")}
          </div>
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
  filter,
  filterPlaceholder,
  onFilterChange,
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
  filter: string;
  filterPlaceholder: string;
  onFilterChange: (value: string) => void;
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

function SpaceMembersNavButton({
  childOnlyCount,
  joinedCount,
  onClick
}: {
  childOnlyCount: number;
  joinedCount: number;
  onClick: () => void;
}) {
  return (
    <button
      className="icon-button space-members-nav"
      type="button"
      aria-label={t("spaceMembers.navAccessible", {
        joined: joinedCount,
        childOnly: childOnlyCount
      })}
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
  const hasUnreadContent = room.has_unread_content ?? room.unread_count > 0;
  const displayCount = room.display_count ?? room.unread_count;
  const mentionCount = room.highlight_count ?? (room.has_unread_mention ? 1 : 0);
  const attentionHighlighted = room.is_attention_highlighted ?? mentionCount;
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
  fallbackMode?: "initials" | "compactLabel" | "elementSpace";
}) {
  const sourceUrl =
    avatar?.thumbnail.kind === "ready" ? mediaSourceUrl(avatar.thumbnail.source_url) : null;
  const { displaySourceUrl, onImageError, onImageLoad } = useRecoverableImageSource(sourceUrl);
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
          "--avatar-label-length": Math.max(fallback.length, 1)
        } as CSSProperties)
      : undefined;
  const elementColor =
    fallbackMode === "elementSpace" ? elementAvatarColorIndex(colorSeed || fallback) : undefined;
  return (
    <span className={className} aria-hidden="true">
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
