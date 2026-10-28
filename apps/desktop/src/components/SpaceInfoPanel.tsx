import { ChevronRight, FileText, MailPlus, Settings, Users } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";

import { t } from "../i18n/messages";
import { operationFailureLabel } from "../app/uiShared";
import { roomDisplayLabel, spaceChildLabel } from "../domain/roomDisplayLabel";
import type {
  HistoryExportState,
  RoomJoinRule,
  RoomManagementState,
  RoomNotificationSettings,
  RoomSummary,
  SpaceChildMembership,
  SpaceChildSuggestedEditState,
  SpaceChildSummary,
  SpaceSummary
} from "../domain/types";
import { HistoryExportSection, type HistoryExportControls } from "./HistoryExportDialog";
import { SpaceAccessSection } from "./SpaceAccessSection";
import { SpaceDescriptionSection } from "./SpaceDescriptionSection";
import { SpaceDescriptionPreview } from "./SpaceDescriptionPreview";
import {
  InlineTextPropertyEditor,
  SettingsPropertyCard,
  type PropertySaveStatus
} from "./SettingsPropertyCard";

/** Mirrors Rust `MAX_LOCAL_SPACE_NAME_SCALARS` / `MAX_LOCAL_SPACE_ICON_SCALARS`. */
const MAX_LOCAL_NAME_LENGTH = 128;
const MAX_LOCAL_ICON_LENGTH = 12;

type LocalPresentationField = "name" | "icon";

export function SpaceInfoPanel({
  fallbackName,
  historyExport,
  historyExportControls,
  localIcon = "",
  localName = "",
  rooms,
  roomNotificationSettings = {},
  roomManagement,
  space,
  spaceChildren = [],
  canEditSuggested = false,
  suggestedEdit,
  onAcceptInvite,
  onInvitePeople,
  onJoinRoom,
  onOpenFiles,
  onOpenMembers,
  onSetLocalPresentation,
  onSetChildSuggested,
  onUpdateJoinRule,
  onUpdateTopic
}: {
  fallbackName: string;
  historyExport?: HistoryExportState;
  historyExportControls?: HistoryExportControls;
  localIcon?: string;
  localName?: string;
  rooms: RoomSummary[];
  roomNotificationSettings?: Record<string, RoomNotificationSettings>;
  roomManagement?: RoomManagementState;
  space: SpaceSummary | null;
  /** Issue #961: every child the Space advertises, joined or not. */
  spaceChildren?: readonly SpaceChildSummary[];
  /** Issue #1410: whether this account may change the children's suggested flags. */
  canEditSuggested?: boolean;
  suggestedEdit?: SpaceChildSuggestedEditState;
  onAcceptInvite?: (roomId: string) => void;
  onInvitePeople?: () => void;
  onJoinRoom?: (roomId: string) => void;
  onOpenFiles?: () => void;
  onOpenMembers?: () => void;
  /**
   * Saves this device's presentation of the Space. The promise settles when
   * Rust admits the preference change; the confirmed value arrives as props.
   */
  onSetLocalPresentation?: (
    override: { name?: string | null; icon?: string | null } | null
  ) => void | Promise<unknown>;
  /** Issue #1410: suggest (or stop suggesting) one child to the Space's members. */
  onSetChildSuggested?: (childRoomId: string, suggested: boolean) => void;
  /** Issue #935: change who can join the selected Space. */
  onUpdateJoinRule?: (spaceId: string, joinRule: RoomJoinRule) => void | Promise<void>;
  onUpdateTopic?: (spaceId: string, topic: string | null) => void | Promise<void>;
}) {
  const accessHeadingRef = useRef<HTMLHeadingElement>(null);
  // Which local-presentation change is this panel's own, so its result shows
  // on that property's card; the confirmed value is always Rust's.
  const [localSubmission, setLocalSubmission] = useState<{
    field: LocalPresentationField;
    target: string;
    inFlight: boolean;
    rejected: boolean;
  } | null>(null);
  const localEpochRef = useRef(0);
  const childRooms = space
    ? space.child_room_ids
        .map((roomId) => rooms.find((room) => room.room_id === roomId))
        .filter((room): room is RoomSummary => Boolean(room && !room.is_dm))
    : rooms.filter((room) => !room.is_dm);
  const roomUnreadCount = (room: RoomSummary) =>
    room.unread_count + (room.thread_unread_count ?? 0);
  const unreadTotal = childRooms.reduce(
    (sum, room) =>
      sum +
      (room.tags.low_priority || roomNotificationSettings[room.room_id]?.mode.kind === "mute"
        ? 0
        : roomUnreadCount(room)),
    0
  );
  const childRoomIds = new Set([
    ...childRooms.map((room) => room.room_id),
    ...spaceChildren.map((child) => child.room_id)
  ]);
  // Joined children are already listed above from the room list, which owns
  // their labels and unread state; this is the remainder of the Space.
  const joinedRoomIds = new Set(childRooms.map((room) => room.room_id));
  const outsideChildren = spaceChildren.filter(
    (child) => !joinedRoomIds.has(child.room_id) && child.membership !== "joined"
  );
  const title = localName.trim() || space?.display_name || fallbackName;
  const loadedSpaceSettings =
    space && roomManagement?.selected_room_id === space.space_id
      ? roomManagement.settings
      : null;
  const memberCount = loadedSpaceSettings?.members.length ?? 0;

  const spaceId = space?.space_id ?? null;
  useEffect(() => {
    // A result never carries across to another Space.
    localEpochRef.current += 1;
    setLocalSubmission(null);
  }, [spaceId]);
  useEffect(
    () => () => {
      localEpochRef.current += 1;
    },
    []
  );

  const childSummaries = new Map(spaceChildren.map((child) => [child.room_id, child]));
  // The Space the toggle applies to is named in its label, so an admin of
  // several Spaces sees which one a change reaches.
  const suggestSpaceName = space?.display_name || title;
  function suggestedToggle(roomId: string, roomLabel: string) {
    const child = childSummaries.get(roomId);
    if (!space || !child || !canEditSuggested || !onSetChildSuggested) {
      return null;
    }
    return (
      <SuggestedToggle
        child={child}
        edit={suggestedEdit}
        roomLabel={roomLabel}
        spaceName={suggestSpaceName}
        onChange={(suggested) => onSetChildSuggested(child.room_id, suggested)}
      />
    );
  }

  function openAccessSettings() {
    const heading = accessHeadingRef.current;
    heading?.scrollIntoView?.({ block: "nearest" });
    heading?.focus();
  }

  function openMembers() {
    onOpenMembers?.();
  }

  /**
   * Changes one local field and keeps the other: clearing the name must not
   * drop the icon, nor the reverse. Removing both removes the Space's local
   * presentation.
   */
  function saveLocalPresentation(field: LocalPresentationField, next: string) {
    if (!onSetLocalPresentation) return;
    const epoch = ++localEpochRef.current;
    const name = field === "name" ? next : localName.trim();
    const icon = field === "icon" ? next : localIcon.trim();
    setLocalSubmission({ field, target: next, inFlight: true, rejected: false });
    const settle = (rejected: boolean) => {
      if (localEpochRef.current === epoch) {
        setLocalSubmission((previous) =>
          previous ? { ...previous, inFlight: false, rejected } : previous
        );
      }
    };
    try {
      void Promise.resolve(
        onSetLocalPresentation(name || icon ? { name: name || null, icon: icon || null } : null)
      ).then(
        () => settle(false),
        () => settle(true)
      );
    } catch {
      settle(true);
    }
  }

  function localStatus(field: LocalPresentationField): PropertySaveStatus {
    if (localSubmission?.field !== field) return null;
    if (localSubmission.inFlight) return { kind: "saving" };
    if (localSubmission.rejected) {
      return { kind: "failed", message: t("space.localPresentationFailed") };
    }
    // Saved only once Rust's value is the submitted one; a change Rust
    // rejects after admitting it leaves the old value, never a success.
    const confirmed = (field === "name" ? localName : localIcon).trim();
    return confirmed === localSubmission.target ? { kind: "saved" } : null;
  }

  return (
    <section className="settings-panel space-info-panel" aria-labelledby="space-info-title">
      <header className="settings-panel-header">
        <div>
          <h2 id="space-info-title" dir="auto">{title}</h2>
          <p dir="auto">{space?.space_id ?? t("space.allRooms")}</p>
        </div>
      </header>

      <div className="settings-summary-grid" aria-label={t("space.summary")}>
        <SummaryTile label={t("workspace.rooms")} value={String(space ? childRoomIds.size : childRooms.length)} />
        <SummaryTile label={t("room.members")} value={loadedSpaceSettings ? String(memberCount) : "-"} />
        <SummaryTile label={t("room.unread")} value={String(unreadTotal)} />
      </div>

      {space ? (
        <section className="settings-section" aria-label={t("space.names")}>
          <h3>{t("space.names")}</h3>
          {/*
            Issue #960: the canonical `m.room.name` and the local label this
            device shows are different facts. A Space with no name event has
            no canonical name — its alias or computed name is not one.
            Issue #1008: each is shown, changed and confirmed in its own card.
          */}
          <SettingsPropertyCard
            property="space-matrix-name"
            label={t("space.canonicalName")}
            hint={<p className="profile-settings-hint">{t("space.canonicalNameHint")}</p>}
          >
            <div className="settings-property-row">
              <div
                className="settings-property-value"
                dir={space.raw_name?.trim() ? "auto" : undefined}
              >
                {space.raw_name?.trim() || (
                  <span className="settings-property-empty">{t("space.nameUnset")}</span>
                )}
              </div>
            </div>
          </SettingsPropertyCard>
          <InlineTextPropertyEditor
            key={`${space.space_id}:local-name`}
            property="space-local-name"
            label={t("space.localName")}
            value={localName}
            emptyText={t("space.nameUnset")}
            inputLabel={t("space.localName")}
            editLabel={t("space.editLocalName")}
            saveLabel={t("space.saveLocalName")}
            clearLabel={t("space.clearLocalName")}
            placeholder={t("space.localNamePlaceholder")}
            maxLength={MAX_LOCAL_NAME_LENGTH}
            syncKey={`${space.space_id}:local-name`}
            canEdit={Boolean(onSetLocalPresentation)}
            busy={Boolean(localSubmission?.inFlight)}
            hint={<p className="profile-settings-hint">{t("space.localNameHint")}</p>}
            status={localStatus("name")}
            onSave={(next) => saveLocalPresentation("name", next)}
            onClear={() => saveLocalPresentation("name", "")}
          />
          <InlineTextPropertyEditor
            key={`${space.space_id}:local-icon`}
            property="space-local-icon"
            label={t("space.localIcon")}
            value={localIcon}
            emptyText={t("space.nameUnset")}
            display={<span className="settings-property-icon-preview">{localIcon.trim()}</span>}
            inputLabel={t("space.localIcon")}
            editLabel={t("space.editLocalIcon")}
            saveLabel={t("space.saveLocalIcon")}
            clearLabel={t("space.clearLocalIcon")}
            placeholder={t("space.localIconPlaceholder")}
            maxLength={MAX_LOCAL_ICON_LENGTH}
            syncKey={`${space.space_id}:local-icon`}
            canEdit={Boolean(onSetLocalPresentation)}
            busy={Boolean(localSubmission?.inFlight)}
            hint={<p className="profile-settings-hint">{t("space.localIconHint")}</p>}
            status={localStatus("icon")}
            onSave={(next) => saveLocalPresentation("icon", next)}
            onClear={() => saveLocalPresentation("icon", "")}
          />
        </section>
      ) : null}

      {space ? (
        <>
        <SpaceDescriptionSection key={`description:${space.space_id}`} spaceId={space.space_id} roomManagement={roomManagement} onUpdateTopic={onUpdateTopic} />
      <SpaceAccessSection
          // Keyed by Space so a confirmation or result never carries across.
          key={space.space_id}
          headingRef={accessHeadingRef}
          roomManagement={roomManagement}
          space={space}
          onUpdateJoinRule={onUpdateJoinRule}
        />
        </>
      ) : null}

      <section className="settings-section" aria-label={t("workspace.rooms")}>
        <h3>{t("workspace.rooms")}</h3>
        <div className="settings-detail-list">
          {childRooms.map((room) => (
            <div className="settings-detail-row" key={room.room_id}>
              <span dir="auto">{roomDisplayLabel(room)}</span>
              <small dir="auto">{roomUnreadCount(room) ? t("room.unreadCount", { count: roomUnreadCount(room) }) : room.room_id}</small>
              {suggestedToggle(room.room_id, roomDisplayLabel(room))}
            </div>
          ))}
          {/*
            Issue #961: the rest of the Space — children the account has not
            joined — with the relationship it is in, and a join action only
            where the server's own join rule allows one.
          */}
          {outsideChildren.map((child) => (
            <div className="settings-detail-row" key={child.room_id}>
              <span dir="auto">{spaceChildLabel(child)}</span>
              {/* #1661: a sub-Space introduces itself with its description. */}
              {child.is_space ? (
                <SpaceDescriptionPreview className="space-child-description" text={child.topic} />
              ) : null}
              <small className="space-child-status">
                <span className="room-membership-badge">
                  {spaceChildMembershipLabel(child.membership)}
                </span>
                {/*
                  An invitation is answered through the invite workflow, which
                  owns the account's invite list; only a room with no
                  invitation is entered with a join.
                */}
                {child.membership === "invited" && onAcceptInvite ? (
                  <button
                    className="profile-settings-action"
                    type="button"
                    onClick={() => onAcceptInvite(child.room_id)}
                  >
                    {t("invite.accept")}
                  </button>
                ) : child.can_join && onJoinRoom ? (
                  <button
                    className="profile-settings-action"
                    type="button"
                    onClick={() => onJoinRoom(child.room_id)}
                  >
                    {t("directory.join")}
                  </button>
                ) : null}
              </small>
              {suggestedToggle(child.room_id, spaceChildLabel(child))}
            </div>
          ))}
        </div>
      </section>

      <section className="settings-section" aria-label={t("space.spacePreferences")}>
        <h3>{t("space.spacePreferences")}</h3>
        <div className="settings-detail-list">
          <DetailRow label={t("space.roomMembership")} value={space ? t("space.childRooms") : t("space.allRooms")} />
          <DetailRow label={t("space.directMessages")} value={t("room.globalDmList")} />
          <DetailRow label={t("room.unread")} value={unreadTotal ? t("room.unreadCount", { count: unreadTotal }) : t("space.noUnread")} />
        </div>
      </section>

      {space && historyExport && historyExportControls ? (
        <HistoryExportSection
          key={space.space_id}
          target={{ kind: "space", spaceId: space.space_id, name: space.display_name || fallbackName }}
          exportState={historyExport}
          controls={historyExportControls}
        />
      ) : null}

      <SettingsEntryList
        entries={[
          // Issue #1008: every entry leads somewhere. Access is on this
          // panel, so its entry moves focus there and carries its name.
          ...(space
            ? [
                { icon: <Settings size={16} />, label: t("space.access"), onClick: openAccessSettings },
                { icon: <Users size={16} />, label: t("room.members"), onClick: openMembers }
              ]
            : []),
          { icon: <MailPlus size={16} />, label: t("space.invite"), onClick: onInvitePeople },
          { icon: <FileText size={16} />, label: t("room.files"), onClick: onOpenFiles }
        ]}
      />
    </section>
  );
}

function SuggestedToggle({
  child,
  edit,
  roomLabel,
  spaceName,
  onChange
}: {
  child: SpaceChildSummary;
  edit?: SpaceChildSuggestedEditState;
  roomLabel: string;
  spaceName: string;
  onChange: (suggested: boolean) => void;
}) {
  const saving = edit?.kind === "saving" ? edit : null;
  const failed = edit?.kind === "failed" && edit.child_room_id === child.room_id ? edit : null;
  const isSavingThis = saving?.child_room_id === child.room_id;
  // Only one change is in flight at a time; Rust rejects a second one, so
  // every toggle waits while any save runs.
  const checked = isSavingThis ? saving.suggested : child.suggested === true;
  return (
    <span className="space-child-suggest">
      <label>
        <input
          type="checkbox"
          checked={checked}
          disabled={saving !== null}
          aria-label={t("space.suggestToMembersFor", { room: roomLabel, space: spaceName })}
          onChange={(event) => onChange(event.currentTarget.checked)}
        />
        <span aria-hidden="true" dir="auto">
          {t("space.suggestToMembers", { space: spaceName })}
        </span>
      </label>
      {isSavingThis ? <small role="status">{t("space.suggestSaving")}</small> : null}
      {failed ? (
        <small className="space-child-suggest-error" role="alert">
          {t("space.suggestFailed", { reason: operationFailureLabel(failed.failure) })}
        </small>
      ) : null}
    </span>
  );
}

function spaceChildMembershipLabel(membership: SpaceChildMembership): string {
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

function DetailRow({
  label,
  value,
  userText = false
}: {
  label: string;
  value: string;
  /** Set for values that carry user-provided text, which needs `dir="auto"`. */
  userText?: boolean;
}) {
  return (
    <div className="settings-detail-row">
      <span>{label}</span>
      <small dir={userText ? "auto" : undefined}>{value}</small>
    </div>
  );
}

function SummaryTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-summary-tile">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function SettingsEntryList({
  entries
}: {
  entries: Array<{ icon: ReactNode; label: string; onClick?: () => void }>;
}) {
  return (
    <div className="settings-list">
      {entries.map((entry) => (
        <button
          className="settings-list-item"
          key={entry.label}
          type="button"
          disabled={!entry.onClick}
          onClick={entry.onClick}
        >
          <span className="settings-list-label">
            <span className="settings-list-icon" aria-hidden="true">
              {entry.icon}
            </span>
            <span>{entry.label}</span>
          </span>
          <ChevronRight size={14} />
        </button>
      ))}
    </div>
  );
}
