import { UserPlus, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type MutableRefObject, type RefObject } from "react";

import type {
  InviteTargetCandidate,
  SpaceInviteAvailabilityReason,
  SpaceInviteCancellationAvailabilityReason,
  SpaceMemberEntry,
  SpaceMemberRoleOption,
  SpaceMembersState,
  UserProfile
} from "../domain/types";
import { contextMenuItems } from "../domain/contextMenus";
import { t } from "../i18n/messages";
import { ICON_SIZE, type OpenContextMenu } from "../app/uiShared";
import { ImeTextField } from "./ImeTextControl";
import { EntityAvatar } from "./Shell";

const noopSearchInviteTargets = async (): Promise<InviteTargetCandidate[]> => [];

export type {
  SpaceInviteAvailabilityReason,
  SpaceInviteCancellationAvailabilityReason
} from "../domain/types";

export interface SpaceMembersPanelProps {
  state: SpaceMembersState;
  canInvite: boolean;
  onClose?: () => void;
  profileUsers?: Record<string, UserProfile>;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
  childRoomLabels?: ReadonlyMap<string, string>;
  onInviteUser: (userId: string) => void;
  /** Invite a brand-new user to the Space (space-only membership, #508). */
  onInviteSearchCandidate?: (userId: string) => void;
  onSearchInviteTargets?: (query: string) => Promise<InviteTargetCandidate[]>;
  /** Resets the shared Rust-owned invite-workflow state the space search uses. */
  onResetInviteSearch?: () => void;
  onCancelInvite?: (userId: string) => void;
  onUpdateRole?: (userId: string, option: SpaceMemberRoleOption) => void;
  onReloadRoles?: () => void;
  onOpenProfile: (userId: string) => void;
  onOpenContextMenu?: OpenContextMenu;
  onDiagnostic?: (message: string) => void;
  inviteAvailabilityReason?: SpaceInviteAvailabilityReason;
  canCancelInvite?: boolean;
  cancelAvailabilityReason?: SpaceInviteCancellationAvailabilityReason;
  cancelInviteFailure?: boolean;
  roleUpdateFailure?: boolean;
}

interface SpaceMembersSection {
  id: "joined" | "invited" | "child-only";
  label: string;
  entries: SpaceMemberEntry[];
}

function memberInitials(entry: SpaceMemberEntry): string {
  const words = entry.display_label.trim().split(/\s+/).filter(Boolean);
  const initials = words.length > 1 ? `${words[0]?.[0] ?? ""}${words[1]?.[0] ?? ""}` : words[0]?.slice(0, 2);
  return (initials || "?").toUpperCase();
}

function memberRoleLabel(role: SpaceMemberEntry["role"]): string | null {
  switch (role) {
    case "creator":
      return t("room.roleCreator");
    case "administrator":
      return t("room.roleAdministrator");
    default:
      return null;
  }
}

function roleOptionLabel(role: SpaceMemberEntry["role"]): string {
  switch (role) {
    case "creator":
      return t("room.roleCreator");
    case "administrator":
      return t("room.roleAdministrator");
    case "moderator":
      return t("room.roleModerator");
    case "user":
      return t("room.roleUser");
  }
}

function matchesSearch(entry: SpaceMemberEntry, query: string): boolean {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  if (!normalizedQuery) {
    return true;
  }

  return [entry.display_label, entry.display_name, entry.original_display_label, entry.user_id]
    .filter((value): value is string => Boolean(value))
    .some((value) => value.toLocaleLowerCase().includes(normalizedQuery));
}

function hasPendingOperation(state: SpaceMembersState): boolean {
  return (
    state.operation.kind === "loading" ||
    state.operation.kind === "inviting" ||
    state.operation.kind === "cancellingInvite" ||
    state.operation.kind === "updatingRole"
  );
}

function inviteIsDisabled(
  state: SpaceMembersState,
  entry: SpaceMemberEntry,
  canInvite: boolean
): boolean {
  const inFlightInviteTarget =
    state.operation.kind === "inviting" && state.operation.user_id === entry.user_id;
  return !canInvite || entry.invite_pending || inFlightInviteTarget || hasPendingOperation(state);
}

function inviteAvailabilityReasonForEntry(
  state: SpaceMembersState,
  entry: SpaceMemberEntry,
  canInvite: boolean,
  availabilityReason: SpaceInviteAvailabilityReason | undefined
): SpaceInviteAvailabilityReason {
  if (!canInvite) {
    return availabilityReason ?? "permission_denied";
  }
  if (entry.invite_pending) {
    return "invite_pending";
  }
  if (hasPendingOperation(state)) {
    return "operation_pending";
  }
  return availabilityReason ?? "available";
}

function cancelIsDisabled(state: SpaceMembersState, canCancelInvite: boolean): boolean {
  return !canCancelInvite || hasPendingOperation(state);
}

function cancelAvailabilityReasonForEntry(
  state: SpaceMembersState,
  canCancelInvite: boolean,
  availabilityReason: SpaceInviteCancellationAvailabilityReason | undefined
): SpaceInviteCancellationAvailabilityReason {
  if (!canCancelInvite) {
    return availabilityReason ?? "permission_denied";
  }
  if (hasPendingOperation(state)) {
    return "operation_pending";
  }
  return availabilityReason ?? "available";
}

function cancellationFailureIsVisible(state: SpaceMembersState): boolean {
  const operation = state.operation;
  if (operation.kind !== "failed" || operation.user_id === null) {
    return false;
  }
  return state.space_invited.some((entry) => entry.user_id === operation.user_id);
}

function childRoomContext(
  entry: SpaceMemberEntry,
  childRoomLabels: ReadonlyMap<string, string>
): string {
  const labels = entry.child_room_ids
    .map((roomId) => childRoomLabels.get(roomId)?.trim() ?? "")
    .filter(Boolean);
  if (entry.child_room_ids.length <= 2 && labels.length === entry.child_room_ids.length) {
    return t("spaceMembers.childRoomContext", { rooms: labels.join(", ") });
  }
  return t(
    entry.child_room_ids.length === 1
      ? "spaceMembers.childRoomCountOne"
      : "spaceMembers.childRoomCount",
    { count: entry.child_room_ids.length }
  );
}

export function SpaceMembersPanel({
  state,
  canInvite,
  onClose = () => undefined,
  profileUsers = {},
  onRequestAvatarThumbnail,
  childRoomLabels = new Map<string, string>(),
  onInviteUser,
  onInviteSearchCandidate = () => undefined,
  onSearchInviteTargets = noopSearchInviteTargets,
  onResetInviteSearch = () => undefined,
  onCancelInvite = () => undefined,
  onUpdateRole = () => undefined,
  onReloadRoles = () => undefined,
  onOpenProfile,
  onOpenContextMenu,
  onDiagnostic,
  inviteAvailabilityReason,
  canCancelInvite = false,
  cancelAvailabilityReason,
  cancelInviteFailure = false,
  roleUpdateFailure = false
}: SpaceMembersPanelProps) {
  const [query, setQuery] = useState("");
  // #508: space-only invite search — inviting a brand-new user to the Space
  // room (space membership only, no child-room membership).
  const [inviteMode, setInviteMode] = useState(false);
  const [inviteQuery, setInviteQuery] = useState("");
  const [inviteCandidates, setInviteCandidates] = useState<InviteTargetCandidate[]>([]);
  const [inviteSearching, setInviteSearching] = useState(false);
  const [pendingRoleChange, setPendingRoleChange] = useState<{
    userId: string;
    option: SpaceMemberRoleOption;
  } | null>(null);
  const inviteSearchRequestRef = useRef(0);
  const panelRef = useRef<HTMLElement | null>(null);
  const roleSelectRefs = useRef(new Map<string, HTMLSelectElement>());
  const previousOperationRef = useRef(state.operation);
  const sections = useMemo<SpaceMembersSection[]>(
    () => [
      {
        id: "joined",
        label: t("spaceMembers.sectionJoined"),
        entries: state.space_joined
      },
      {
        id: "invited",
        label: t("spaceMembers.sectionInvited"),
        entries: state.space_invited
      },
      {
        id: "child-only",
        label: t("spaceMembers.sectionChildOnly"),
        entries: state.child_room_only
      }
    ],
    [state.child_room_only, state.space_invited, state.space_joined]
  );
  const filteredSections = sections.map((section) => ({
    ...section,
    entries: section.entries.filter((entry) => matchesSearch(entry, query))
  }));
  const resultCount = filteredSections.reduce((count, section) => count + section.entries.length, 0);
  const hasResults = filteredSections.some((section) => section.entries.length > 0);
  const panelAvailabilityReason: SpaceInviteAvailabilityReason = !canInvite
    ? inviteAvailabilityReason ?? "permission_denied"
    : hasPendingOperation(state)
      ? "operation_pending"
      : inviteAvailabilityReason ?? "available";
  const failedOperation = state.operation.kind === "failed" ? state.operation : null;
  const roleUpdateFailed =
    state.operation.kind === "roleUpdateFailed" ? state.operation : null;

  const focusRoleSelect = (userId: string) => {
    queueMicrotask(() => roleSelectRefs.current.get(userId)?.focus());
  };

  useEffect(() => {
    const previous = previousOperationRef.current;
    if (previous.kind === "updatingRole" && state.operation.kind !== "updatingRole") {
      focusRoleSelect(previous.user_id);
    }
    previousOperationRef.current = state.operation;
  }, [state.operation]);

  useEffect(() => {
    if (
      pendingRoleChange &&
      !state.space_joined.some((entry) => entry.user_id === pendingRoleChange.userId)
    ) {
      setPendingRoleChange(null);
    }
  }, [pendingRoleChange, state.space_joined]);

  useEffect(() => {
    onDiagnostic?.(
      [
        `rendered joined=${state.space_joined.length}`,
        `invited=${state.space_invited.length}`,
        `child_only=${state.child_room_only.length}`,
        `search_active=${Boolean(query.trim())}`,
        `result_count=${resultCount}`,
        `availability_reason=${panelAvailabilityReason}`,
        `incomplete_notice=${state.incomplete_child_room_count > 0}`
      ].join(" ")
    );
  }, [
    onDiagnostic,
    panelAvailabilityReason,
    query,
    resultCount,
    state.child_room_only.length,
    state.incomplete_child_room_count,
    state.space_invited.length,
    state.space_joined.length
  ]);

  // #508: leaving the invite search (cancel, panel close, unmount) resets the
  // shared Rust-owned invite-workflow state so a later room invite dialog never
  // inherits this space search's query or candidates.
  useEffect(() => {
    return () => {
      onResetInviteSearch();
    };
  }, [onResetInviteSearch]);

  // #508: debounced invite-target search; stale responses are discarded by
  // the request counter so an older query can never overwrite newer results.
  useEffect(() => {
    const requestId = ++inviteSearchRequestRef.current;
    const trimmed = inviteQuery.trim();
    if (!inviteMode) {
      setInviteCandidates([]);
      setInviteSearching(false);
      return;
    }
    if (!trimmed) {
      setInviteCandidates([]);
      setInviteSearching(false);
      return;
    }
    setInviteSearching(true);
    const timer = window.setTimeout(() => {
      void onSearchInviteTargets(trimmed).then((candidates) => {
        if (inviteSearchRequestRef.current !== requestId) {
          return;
        }
        setInviteCandidates(candidates);
        setInviteSearching(false);
      });
    }, 250);
    return () => {
      window.clearTimeout(timer);
    };
  }, [inviteMode, inviteQuery, onSearchInviteTargets]);

  return (
    <section
      className="space-members-panel"
      ref={panelRef}
      aria-labelledby="space-members-title"
    >
      <header className="space-members-header">
        <h2 id="space-members-title">{t("spaceMembers.title")}</h2>
        <span className="space-members-count" aria-label={t("spaceMembers.joinedCount", {
          count: state.space_joined.length
        })}>
          {state.space_joined.length}
        </span>
        {canInvite && !inviteMode ? (
          <button
            className="icon-button space-members-invite-trigger"
            type="button"
            aria-label={t("room.invitePeople")}
            title={t("room.invitePeople")}
            onClick={() => setInviteMode(true)}
          >
            <UserPlus size={ICON_SIZE.control} />
          </button>
        ) : null}
        <button
          className="icon-button space-members-close"
          type="button"
          aria-label={t("action.close", { title: t("spaceMembers.title") })}
          onClick={onClose}
        >
          <X size={ICON_SIZE.control} />
        </button>
      </header>

      {inviteMode ? (
        <div className="space-members-invite" role="search">
          <label className="visually-hidden" htmlFor="space-members-invite-input">
            {t("dialog.inviteSearch")}
          </label>
          <ImeTextField
            id="space-members-invite-input"
            type="search"
            autoFocus
            aria-label={t("dialog.inviteSearch")}
            placeholder={t("dialog.inviteSearch")}
            value={inviteQuery}
            onChange={(event) => setInviteQuery(event.target.value)}
          />
          <button
            className="space-members-invite-back"
            type="button"
            onClick={() => {
              setInviteMode(false);
              setInviteQuery("");
              setInviteCandidates([]);
              onResetInviteSearch();
            }}
          >
            {t("action.cancel")}
          </button>
        </div>
      ) : (
        <div className="space-members-search" role="search">
          <label className="visually-hidden" htmlFor="space-members-search-input">
            {t("spaceMembers.search")}
          </label>
          <ImeTextField
            id="space-members-search-input"
            type="search"
            aria-label={t("spaceMembers.search")}
            placeholder={t("spaceMembers.search")}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>
      )}

      {inviteMode ? (
        <div className="space-members-invite-results" aria-label={t("dialog.inviteCandidates")}>
          {inviteSearching && inviteCandidates.length === 0 ? (
            <p className="space-members-empty" role="status">
              {t("activity.loading")}
            </p>
          ) : inviteCandidates.length === 0 && inviteQuery.trim() ? (
            <p className="space-members-empty" role="status">
              {t("spaceMembers.noResults")}
            </p>
          ) : null}
          {inviteCandidates.map((candidate) => {
            const selectable = candidate.status === "selectable";
            return (
              <button
                className="space-members-invite-candidate"
                type="button"
                key={candidate.user_id}
                disabled={!selectable || hasPendingOperation(state)}
                onClick={() => {
                  if (selectable) {
                    onInviteSearchCandidate(candidate.user_id);
                  }
                }}
              >
                <span>{candidate.display_label}</span>
                <span className="space-members-invite-candidate-id" dir="auto">
                  {candidate.user_id}
                </span>
              </button>
            );
          })}
        </div>
      ) : null}

      {failedOperation !== null || cancelInviteFailure || roleUpdateFailure || roleUpdateFailed !== null ? (
        <p className="space-members-invite-failure" role="alert">
          {roleUpdateFailure || roleUpdateFailed !== null
            ? t("spaceMembers.roleUpdateFailed")
            : cancelInviteFailure || cancellationFailureIsVisible(state)
              ? t("spaceMembers.cancelInviteFailed")
              : failedOperation !== null && failedOperation.user_id !== null
                ? t("spaceMembers.inviteFailed")
                : t("spaceMembers.loadFailed")}
        </p>
      ) : null}
      {roleUpdateFailed !== null ? (
        <button className="dialog-button" type="button" onClick={onReloadRoles}>
          {t("spaceMembers.roleReload")}
        </button>
      ) : null}

      {state.incomplete_child_room_count > 0 ? (
        <p className="space-members-sync-notice" role="status">
          {t("spaceMembers.syncIncomplete")}
        </p>
      ) : null}

      {!inviteMode ? (
        <div className="space-members-sections">
          {filteredSections.map((section) => (
            <section
              className="space-members-section"
              data-space-members-section={section.id}
              key={section.id}
              aria-labelledby={`space-members-section-${section.id}`}
            >
              <h3 id={`space-members-section-${section.id}`}>{section.label}</h3>
              {section.entries.length > 0 ? (
                <ul className="space-members-list" aria-label={section.label}>
                  {section.entries.map((entry) => (
                    <SpaceMemberRow
                      key={entry.user_id}
                      entry={entry}
                      sectionId={section.id}
                      state={state}
                      canInvite={canInvite}
                      canCancelInvite={canCancelInvite}
                      childRoomLabels={childRoomLabels}
                      profileUsers={profileUsers}
                      avatarViewportRef={panelRef}
                      onInviteUser={onInviteUser}
                      onOpenProfile={onOpenProfile}
                      onOpenContextMenu={onOpenContextMenu}
                      onDiagnostic={onDiagnostic}
                      inviteAvailabilityReason={inviteAvailabilityReason}
                      cancelAvailabilityReason={cancelAvailabilityReason}
                      onCancelInvite={onCancelInvite}
                      onUpdateRole={(entry, option) => {
                        if (option.requires_confirmation) {
                          setPendingRoleChange({ userId: entry.user_id, option });
                        } else {
                          onUpdateRole(entry.user_id, option);
                        }
                      }}
                      roleSelectRefs={roleSelectRefs}
                      onRequestAvatarThumbnail={onRequestAvatarThumbnail}
                    />
                  ))}
                </ul>
              ) : null}
            </section>
          ))}
        </div>
      ) : null}

      {!inviteMode && !hasResults ? (
        <p className="space-members-empty" role="status">
          {t("spaceMembers.noResults")}
        </p>
      ) : null}

      {pendingRoleChange ? (
        <div
          className="dialog-overlay"
          role="dialog"
          aria-modal="true"
          aria-labelledby="space-member-role-confirm-title"
        >
          <div className="dialog-box">
            <h2 id="space-member-role-confirm-title">
              {t("spaceMembers.roleConfirmTitle")}
            </h2>
            <p>
              {t("spaceMembers.roleConfirmCopy", {
                name:
                  state.space_joined.find((entry) => entry.user_id === pendingRoleChange.userId)
                    ?.display_label ?? "",
                role: roleOptionLabel(pendingRoleChange.option.role)
              })}
            </p>
            <div className="dialog-actions">
              <button
                type="button"
                className="dialog-button"
                onClick={() => {
                  const userId = pendingRoleChange.userId;
                  setPendingRoleChange(null);
                  focusRoleSelect(userId);
                }}
              >
                {t("action.cancel")}
              </button>
              <button
                type="button"
                className="dialog-button danger"
                onClick={() => {
                  const change = pendingRoleChange;
                  setPendingRoleChange(null);
                  onUpdateRole(change.userId, change.option);
                }}
              >
                {t("spaceMembers.roleConfirmAction")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}

interface SpaceMemberRowProps {
  entry: SpaceMemberEntry;
  sectionId: SpaceMembersSection["id"];
  state: SpaceMembersState;
  canInvite: boolean;
  canCancelInvite: boolean;
  childRoomLabels: ReadonlyMap<string, string>;
  profileUsers: Record<string, UserProfile>;
  avatarViewportRef: RefObject<Element | null>;
  onInviteUser: (userId: string) => void;
  onOpenProfile: (userId: string) => void;
  onOpenContextMenu?: OpenContextMenu;
  onDiagnostic?: (message: string) => void;
  inviteAvailabilityReason?: SpaceInviteAvailabilityReason;
  cancelAvailabilityReason?: SpaceInviteCancellationAvailabilityReason;
  onCancelInvite: (userId: string) => void;
  onUpdateRole: (entry: SpaceMemberEntry, option: SpaceMemberRoleOption) => void;
  roleSelectRefs: MutableRefObject<Map<string, HTMLSelectElement>>;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}

function SpaceMemberRow({
  entry,
  sectionId,
  state,
  canInvite,
  canCancelInvite,
  childRoomLabels,
  profileUsers,
  avatarViewportRef,
  onInviteUser,
  onOpenProfile,
  onOpenContextMenu,
  onDiagnostic,
  inviteAvailabilityReason,
  cancelAvailabilityReason,
  onCancelInvite,
  onUpdateRole,
  roleSelectRefs,
  onRequestAvatarThumbnail
}: SpaceMemberRowProps) {
  const rowRef = useRef<HTMLLIElement>(null);
  const requestedAvatarUriRef = useRef<string | null>(null);
  const previousAvatarStateRef = useRef<{
    mxcUri: string | null;
    kind: string | null;
    requestId: number | null;
    failureKind: string | null;
  }>({
    mxcUri: null,
    kind: null,
    requestId: null,
    failureKind: null
  });
  const avatar = profileUsers[entry.user_id]?.avatar ?? null;
  const avatarThumbnailKind = avatar?.thumbnail.kind ?? null;
  const avatarThumbnailRequestId =
    avatar?.thumbnail.kind === "loading" || avatar?.thumbnail.kind === "failed"
      ? avatar.thumbnail.request_id
      : null;
  const avatarThumbnailFailureKind =
    avatar?.thumbnail.kind === "failed" ? avatar.thumbnail.failureKind : null;
  useEffect(() => {
    const currentAvatarState = {
      mxcUri: avatar?.mxc_uri ?? null,
      kind: avatarThumbnailKind,
      requestId: avatarThumbnailRequestId,
      failureKind: avatarThumbnailFailureKind
    };
    const previousAvatarState = previousAvatarStateRef.current;
    const avatarStateChanged =
      previousAvatarState.mxcUri !== currentAvatarState.mxcUri ||
      previousAvatarState.kind !== currentAvatarState.kind ||
      previousAvatarState.requestId !== currentAvatarState.requestId ||
      previousAvatarState.failureKind !== currentAvatarState.failureKind;
    previousAvatarStateRef.current = currentAvatarState;

    const canRequestAvatar = avatarThumbnailKind === "notRequested";
    if (avatarStateChanged && canRequestAvatar) {
      requestedAvatarUriRef.current = null;
    }

    if (
      !onRequestAvatarThumbnail ||
      !avatar ||
      !canRequestAvatar ||
      !rowRef.current ||
      requestedAvatarUriRef.current === avatar.mxc_uri ||
      typeof IntersectionObserver === "undefined"
    ) {
      return;
    }

    const row = rowRef.current;
    const mxcUri = avatar.mxc_uri;
    const avatarViewport = avatarViewportRef.current;
    let disposed = false;
    let release: (() => void) | undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (
          requestedAvatarUriRef.current === mxcUri ||
          !entries.some((entry) => entry.isIntersecting)
        ) {
          return;
        }
        requestedAvatarUriRef.current = mxcUri;
        observer.disconnect();
        try {
          void Promise.resolve(onRequestAvatarThumbnail(mxcUri))
            .then((nextRelease) => {
              if (disposed) nextRelease?.();
              else if (nextRelease) release = nextRelease;
            })
            .catch(() => undefined);
        } catch {
          // Demand admission failures are represented by the Rust state/event.
        }
      },
      { root: avatarViewport }
    );
    observer.observe(row);

    return () => {
      disposed = true;
      observer.disconnect();
      release?.();
    };
  }, [
    avatar?.mxc_uri,
    avatarThumbnailFailureKind,
    avatarThumbnailKind,
    avatarThumbnailRequestId,
    avatarViewportRef,
    onRequestAvatarThumbnail
  ]);

  const roleLabel = memberRoleLabel(entry.role);

  return (
    <li
      ref={rowRef}
      className="space-members-row"
      data-user-id={entry.user_id}
      onContextMenu={(event) => {
        if (sectionId !== "child-only" || !onOpenContextMenu || !state.selected_space_id) {
          return;
        }
        onOpenContextMenu(
          event,
          {
            kind: "spaceMember",
            spaceId: state.selected_space_id,
            userId: entry.user_id,
            generation: state.generation
          },
          contextMenuItems({
            kind: "spaceMember",
            spaceId: state.selected_space_id,
            userId: entry.user_id,
            generation: state.generation,
            canInvite,
            invitePending: entry.invite_pending,
            operationPending: hasPendingOperation(state)
          })
        );
      }}
    >
      <button
        className="space-members-row-main"
        type="button"
        aria-label={t("people.openProfile", { name: entry.display_label })}
        onClick={() => onOpenProfile(entry.user_id)}
      >
        <span
          className="space-members-avatar"
          role={avatar?.thumbnail.kind === "ready" ? "img" : undefined}
          aria-label={avatar?.thumbnail.kind === "ready" ? "" : undefined}
        >
          <EntityAvatar
            avatar={avatar}
            className="space-members-avatar-content"
            colorSeed={entry.user_id}
            fallback={memberInitials(entry)}
          />
        </span>
        <span className="space-members-row-text">
          <span className="space-members-name" dir="auto">
            {entry.display_label}
            {roleLabel ? <span className="space-members-role">{roleLabel}</span> : null}
          </span>
          {sectionId === "child-only" && entry.child_room_ids.length > 0 ? (
            <span className="space-members-meta" dir="auto">
              {childRoomContext(entry, childRoomLabels)}
            </span>
          ) : null}
        </span>
      </button>
      {sectionId === "joined" && state.can_edit_roles && entry.role_options.length > 0 ? (
        <label className="space-members-role-control">
          <span className="visually-hidden">
            {t("spaceMembers.roleSelect", { name: entry.display_label })}
          </span>
          <select
            ref={(element) => {
              if (element) {
                roleSelectRefs.current.set(entry.user_id, element);
              } else {
                roleSelectRefs.current.delete(entry.user_id);
              }
            }}
            aria-label={t("spaceMembers.roleSelect", { name: entry.display_label })}
            value={entry.power_level === null ? "" : String(entry.power_level)}
            disabled={hasPendingOperation(state)}
            onChange={(event) => {
              const select = event.currentTarget;
              const option = entry.role_options.find(
                (candidate) => String(candidate.power_level) === event.target.value
              );
              if (option) {
                onUpdateRole(entry, option);
                // The visible value is Rust-owned. Restore it until the next
                // authoritative snapshot projects the requested level.
                select.value = entry.power_level === null ? "" : String(entry.power_level);
              }
            }}
          >
            {entry.power_level !== null &&
            !entry.role_options.some((option) => option.power_level === entry.power_level) ? (
              <option value={String(entry.power_level)} disabled>
                {roleOptionLabel(entry.role)}
              </option>
            ) : null}
            {entry.role_options.map((option) => (
              <option key={`${option.power_level}-${option.role}`} value={String(option.power_level)}>
                {roleOptionLabel(option.role)}
              </option>
            ))}
          </select>
        </label>
      ) : sectionId === "child-only" ? (
        <button
          className="space-members-invite"
          type="button"
          aria-label={t("spaceMembers.invite")}
          disabled={inviteIsDisabled(state, entry, canInvite)}
          onClick={() => {
            onDiagnostic?.(
              `invite trigger=inline availability_reason=${inviteAvailabilityReasonForEntry(
                state,
                entry,
                canInvite,
                inviteAvailabilityReason
              )}`
            );
            onInviteUser(entry.user_id);
          }}
        >
          {entry.invite_pending ? t("spaceMembers.invitePending") : t("spaceMembers.invite")}
        </button>
      ) : sectionId === "invited" ? (
        <button
          className="space-members-cancel"
          type="button"
          aria-label={
            state.operation.kind === "cancellingInvite" &&
            state.operation.user_id === entry.user_id
              ? t("spaceMembers.cancelInvitePending")
              : t("spaceMembers.cancelInvite")
          }
          disabled={cancelIsDisabled(state, canCancelInvite)}
          onClick={() => {
            onDiagnostic?.(
              `cancel trigger=inline availability_reason=${cancelAvailabilityReasonForEntry(
                state,
                canCancelInvite,
                cancelAvailabilityReason
              )}`
            );
            onCancelInvite(entry.user_id);
          }}
        >
          {state.operation.kind === "cancellingInvite" &&
          state.operation.user_id === entry.user_id
            ? t("spaceMembers.cancelInvitePending")
            : t("spaceMembers.cancelInvite")}
        </button>
      ) : null}
    </li>
  );
}
