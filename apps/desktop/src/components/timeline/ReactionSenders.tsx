import { useMemo } from "react";

import { avatarInitial, peopleFacingLabel } from "../../app/uiShared";
import { PersonListPopover, personListBoundaryElement, type PersonListRow } from "./PersonListPopover";
import { t } from "../../i18n/messages";
import type { AvatarImage } from "../../domain/types";
import type { ReactionSender } from "../../domain/coreEvents";

/** What the renderer knows about one reaction sender beyond the preview. */
export type ReactionSenderProfile = {
  label?: string | null;
  avatar?: AvatarImage | null;
};

/**
 * #2111: the fixed list of everyone who reacted with one key.
 *
 * A tooltip disappears as soon as the pointer moves, so the chip's right-click
 * (and long-press) needs its own surface. It is the #2098 read-receipt popover
 * rather than a copy: same content-sized box, one person per row, follow-focus
 * keyboard movement. Core names at most
 * `REACTION_SENDER_PREVIEW_LIMIT` senders, so a group past that limit reports
 * the rest as a count instead of pretending the list is complete.
 */
export function ReactionSenders({
  anchorRef,
  currentUserId,
  open,
  reactionLabel,
  senders,
  profiles,
  totalCount,
  onClose,
  onOpenProfile,
  onRequestAvatarThumbnail
}: {
  anchorRef: React.RefObject<Element | null>;
  currentUserId?: string | null;
  open: boolean;
  /** The reaction key as the chip paints it, used in the dialog's label. */
  reactionLabel: string;
  senders: readonly ReactionSender[];
  profiles: Readonly<Record<string, ReactionSenderProfile>>;
  totalCount: number;
  /** `false` closes without returning the focus to the chip (a profile opens). */
  onClose: (restoreFocus?: boolean) => void;
  onOpenProfile?: (userId: string) => void;
  onRequestAvatarThumbnail?: (mxcUri: string) => void | Promise<void | (() => void)>;
}) {
  const rows = useMemo<PersonListRow[]>(
    () =>
      senders.map((sender) => {
        const profile = profiles[sender.user_id];
        const displayName = peopleFacingLabel(sender.display_label, profile?.label);
        const isSelf = currentUserId !== undefined && currentUserId === sender.user_id;
        return {
          user_id: sender.user_id,
          display_name: displayName,
          initials: avatarInitial(displayName),
          avatar: profile?.avatar ?? null,
          meta: isSelf ? t("people.you") : null,
          meta_class_name: isSelf ? "receipt-reader-time reaction-sender-self" : undefined
        };
      }),
    [currentUserId, profiles, senders]
  );
  const label = t("timeline.reactionSenders", { key: reactionLabel });
  const overflowCount = Math.max(0, totalCount - senders.length);
  return (
    <PersonListPopover
      anchorRef={anchorRef}
      focusOnOpen
      label={label}
      open={open}
      overflowCount={overflowCount}
      overflowText={t("timeline.reactionSenderOverflow", { count: overflowCount })}
      resolveBoundaryElement={personListBoundaryElement}
      rows={rows}
      totalCount={totalCount}
      onActivate={
        onOpenProfile
          ? (row) => {
              // The profile panel takes the focus; restoring the chip's would
              // yank the reader back into the message list.
              onClose(false);
              onOpenProfile(row.user_id);
            }
          : undefined
      }
      onClose={onClose}
      onRequestAvatarThumbnail={onRequestAvatarThumbnail}
    />
  );
}
