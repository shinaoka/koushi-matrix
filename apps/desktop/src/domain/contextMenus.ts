import type { MessageId } from "../i18n/messages";
import type { AccountTabSummary, RoomTags } from "./types";

export type ContextMenuKind =
  | "message"
  | "room"
  | "space"
  | "spaceMember"
  | "account"
  | "accountTab";

export type ContextMenuActionId =
  | "copySelectedText"
  | "searchWebForSelectedText"
  | "replyToMessage"
  | "openThread"
  | "editMessage"
  | "redactMessage"
  | "ignoreUser"
  | "unignoreUser"
  | "reportUser"
  | "reportContent"
  | "selectRoom"
  | "openUserInfo"
  | "openRoomInfo"
  | "searchInRoom"
  | "reportRoom"
  | "setRoomFavourite"
  | "removeRoomFavourite"
  | "setRoomLowPriority"
  | "removeRoomLowPriority"
  | "markRoomAsRead"
  | "markRoomAsUnread"
  | "leaveRoom"
  | "openSpaceInfo"
  | "leaveSpace"
  | "inviteUserToSpace"
  | "openUserSettings"
  | "openKeyboardSettings"
  | "accountTabSettings"
  | "accountTabSignOut"
  | "accountTabSignOutAndRemove"
  | "accountTabSignInAgain"
  | "accountTabRemove";

export interface ContextMenuItem {
  id: ContextMenuActionId;
  labelMessageId: MessageId;
  destructive?: boolean;
}

export type ContextMenuRequest =
  | {
      kind: "message";
      canManage: boolean;
      /** Rust-owned `TimelineItem.can_edit`; gates Edit exactly like the row's
       * own action button (#1100). */
      canEdit: boolean;
      canReply: boolean;
      hasThread: boolean;
      senderUserId: string;
      currentUserId: string;
      roomId: string;
      eventId: string;
      isIgnored: boolean;
      /**
       * The visible message-text selection captured by the requesting row before
       * the menu took focus (#1155). Empty/absent means the selection actions are
       * not offered. It stays in renderer-local menu state for that menu only.
       */
      selectedText?: string;
    }
  | {
      kind: "room";
      roomId: string;
      tags?: RoomTags;
      dmUserIds?: string[];
      canMarkRoomAsRead?: boolean;
    }
  | {
      kind: "space";
    }
  | {
      kind: "spaceMember";
      spaceId: string;
      userId: string;
      generation: number;
      canInvite: boolean;
      invitePending: boolean;
      operationPending: boolean;
    }
  | {
      kind: "account";
    }
  | {
      /** #1504: right-click on an account tab; actions target that tab's account. */
      kind: "accountTab";
      status: AccountTabSummary["status"];
    };

export function contextMenuItems(request: ContextMenuRequest): ContextMenuItem[] {
  switch (request.kind) {
    case "message": {
      const items: ContextMenuItem[] = [];
      if (request.selectedText) {
        // Text the user aimed at is offered first, and these two actions need no
        // message ownership or server event id: only selectable visible text.
        items.push({ id: "copySelectedText", labelMessageId: "context.copySelectedText" });
        items.push({
          id: "searchWebForSelectedText",
          labelMessageId: "context.searchWebForSelectedText"
        });
      }
      if (!request.eventId || !request.senderUserId) {
        // A local echo with no server identity can still offer text selection.
        return items;
      }
      if (request.canReply) {
        items.push({ id: "replyToMessage", labelMessageId: "timeline.replyToMessage" });
      }
      if (request.hasThread) {
        items.push({ id: "openThread", labelMessageId: "context.openThread" });
      }
      if (request.canEdit) {
        items.push({ id: "editMessage", labelMessageId: "context.editMessage" });
      }
      if (request.canManage) {
        items.push({
          id: "redactMessage",
          labelMessageId: "context.redactMessage",
          destructive: true
        });
      }
      if (request.senderUserId !== request.currentUserId) {
        if (request.isIgnored) {
          items.push({ id: "unignoreUser", labelMessageId: "context.unignoreUser" });
        } else {
          items.push({ id: "ignoreUser", labelMessageId: "context.ignoreUser" });
        }
        items.push({
          id: "reportUser",
          labelMessageId: "context.reportUser",
          destructive: true
        });
        items.push({
          id: "reportContent",
          labelMessageId: "context.reportContent",
          destructive: true
        });
      }
      return items;
    }
    case "room": {
      const userInfoItem =
        request.dmUserIds?.length === 1
          ? [{ id: "openUserInfo" as const, labelMessageId: "context.openUserInfo" as const }]
          : [];
      return [
        { id: "selectRoom", labelMessageId: "context.selectRoom" },
        ...userInfoItem,
        { id: "openRoomInfo", labelMessageId: "context.openRoomInfo" },
        { id: "searchInRoom", labelMessageId: "context.searchInRoom" },
        {
          id: "reportRoom",
          labelMessageId: "context.reportRoom",
          destructive: true
        },
        request.tags?.favourite
          ? { id: "removeRoomFavourite", labelMessageId: "context.removeFromFavourites" }
          : { id: "setRoomFavourite", labelMessageId: "context.addToFavourites" },
        request.tags?.low_priority
          ? { id: "removeRoomLowPriority", labelMessageId: "context.removeFromLowPriority" }
          : { id: "setRoomLowPriority", labelMessageId: "context.addToLowPriority" },
        ...(request.canMarkRoomAsRead ? [{ id: "markRoomAsRead" as const, labelMessageId: "room.markAsRead" as const }] : []),
        { id: "markRoomAsUnread", labelMessageId: "room.markAsUnread" },
        // #373: last, destructive, and confirmation-gated by the caller. DM copy
        // differs only for readability — both ids drive the same Matrix
        // room-leave operation. Spaces keep their own `leaveSpace` action so
        // this does not change whether leaving a Space leaves child rooms.
        {
          id: "leaveRoom",
          labelMessageId:
            request.dmUserIds?.length === 1
              ? "context.leaveConversation"
              : "context.leaveRoom",
          destructive: true
        }
      ];
    }
    case "space":
      // #1218: no separate Open Space entry; the Space button's click and
      // keyboard activation already select it.
      return [
        { id: "openSpaceInfo", labelMessageId: "context.openSpaceInfo" },
        { id: "leaveSpace", labelMessageId: "context.leaveSpace", destructive: true }
      ];
    case "spaceMember":
      return request.canInvite && !request.invitePending && !request.operationPending
        ? [{ id: "inviteUserToSpace", labelMessageId: "spaceMembers.invite" }]
        : [];
    case "account":
      return [{ id: "openUserSettings", labelMessageId: "context.openUserSettings" }];
    case "accountTab":
      return accountTabMenuItems(request.status);
  }
}

function accountTabMenuItems(status: AccountTabSummary["status"]): ContextMenuItem[] {
  const signOut: ContextMenuItem[] = [
    { id: "accountTabSignOut", labelMessageId: "accountTabs.menuSignOut", destructive: true },
    {
      id: "accountTabSignOutAndRemove",
      labelMessageId: "accountTabs.menuSignOutAndRemove",
      destructive: true
    }
  ];
  switch (status) {
    case "ready":
    case "needsVerification":
      return [
        { id: "accountTabSettings", labelMessageId: "accountTabs.menuAccountSettings" },
        ...signOut
      ];
    case "error":
      return signOut;
    case "signedOut":
      return [
        { id: "accountTabSignInAgain", labelMessageId: "accountTabs.menuSignInAgain" },
        { id: "accountTabRemove", labelMessageId: "accountTabs.menuRemoveTab", destructive: true }
      ];
    // Transitional tabs (restoring, signing in, signing out) and the add-account
    // tab have no account action to offer; the add-account tab keeps its ×.
    case "addAccount":
    case "restoring":
    case "authenticating":
    case "loggingOut":
      return [];
  }
}
