import { invoke } from "@tauri-apps/api/core";

import { isRendererSelectedAccountTabId } from "./client";
import { desktopEventPort } from "./desktopEventRuntime";
import { saveReadyMediaFile } from "./linkMediaRuntime";
import { isTauriRuntime } from "./runtimeEnvironment";
import type { TimelineTransport } from "../components/timeline/TimelineTransport";
import type {
  CoreEventPayload,
  TimelineBottomArrival,
  TimelineGapId,
  TimelineKey
} from "../domain/coreEvents";
import type { ComposerDocument, TimelineScrollAnchor } from "../domain/types";

let tauriCoreEventListenerReady: Promise<void> = Promise.resolve();

/**
 * Tauri transport for the event-driven timeline (Async rule 4: timeline data
 * flows ONLY as CoreEvent diffs over `koushi-desktop://event`; AppState
 * snapshots never embed item lists). Null in browser preview mode, where the
 * fixture snapshot rendering below is used instead.
 */
export function createTauriTimelineTransport(accountTabId?: string): TimelineTransport | null {
  if (!isTauriRuntime()) return null;

  const invokeAccount = <T = unknown>(
    command: string,
    args?: Record<string, unknown>
  ): Promise<T> => {
    if (accountTabId !== undefined && !isRendererSelectedAccountTabId(accountTabId)) {
      return Promise.reject(new Error("account tab is no longer selected"));
    }
    if (accountTabId === undefined) {
      return args === undefined ? invoke<T>(command) : invoke<T>(command, args);
    }
    return invoke<T>(command, { ...args, accountTabId });
  };

  return {
    listenCoreEvents(listener: (payload: CoreEventPayload) => void) {
      let disposed = false;
      let unlisten: (() => void) | null = null;
      tauriCoreEventListenerReady = desktopEventPort.listenCoreEvents((payload) => {
        if (!disposed) listener(payload);
      }).then((dispose) => {
        if (disposed) {
          dispose();
        } else {
          unlisten = dispose;
        }
      });
      void tauriCoreEventListenerReady;
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
    async ensureSubscribed(timelineKey: TimelineKey) {
      await tauriCoreEventListenerReady;
      await invokeAccount("ensure_timeline_subscribed", { timelineKey });
    },
    async paginateBackwards(timelineKey: TimelineKey) {
      if ("Room" in timelineKey.kind) {
        await invokeAccount("paginate_timeline_backwards", {
          roomId: timelineKey.kind.Room.room_id
        });
        return;
      }
      if ("Thread" in timelineKey.kind) {
        await invokeAccount("paginate_thread_timeline_backwards", {
          roomId: timelineKey.kind.Thread.room_id,
          rootEventId: timelineKey.kind.Thread.root_event_id
        });
      }
    },
    async repairTimeline(roomId: string) {
      await invokeAccount("repair_room_timeline", { roomId });
    },
    async sendReaction(roomId: string, eventId: string, reactionKey: string) {
      await invokeAccount("send_reaction", { roomId, eventId, reactionKey });
    },
    async retrySend(roomId: string, transactionId: string) {
      await invokeAccount("retry_send", { roomId, transactionId });
    },
    async cancelSend(roomId: string, transactionId: string) {
      await invokeAccount("cancel_send", { roomId, transactionId });
    },
    async redactReaction(
      roomId: string,
      eventId: string,
      reactionKey: string,
      reactionEventId: string
    ) {
      await invokeAccount("redact_reaction", {
        roomId,
        eventId,
        reactionKey,
        reactionEventId
      });
    },
    async sendReadReceipt(roomId: string, eventId: string, threadRootEventId?: string | null) {
      await invokeAccount("send_read_receipt", { roomId, eventId, threadRootEventId });
    },
    async setFullyRead(roomId: string, eventId: string) {
      await invokeAccount("set_fully_read", { roomId, eventId });
    },
    async setTyping(roomId: string, isTyping: boolean) {
      await invokeAccount("set_typing", { roomId, isTyping });
    },
    async editMessage(roomId: string, eventId: string, document: ComposerDocument) {
      await invokeAccount("edit_message", { roomId, eventId, document });
    },
    async redactMessage(roomId: string, eventId: string) {
      await invokeAccount("redact_message", { roomId, eventId });
    },
    async pinEvent(roomId: string, eventId: string) {
      await invokeAccount("pin_event", { roomId, eventId });
    },
    async unpinEvent(roomId: string, eventId: string) {
      await invokeAccount("unpin_event", { roomId, eventId });
    },
    async downloadMedia(roomId: string, eventId: string) {
      await invokeAccount("download_media", { roomId, eventId });
    },
    async saveMediaFile(sourceUrl: string, filename: string) {
      await saveReadyMediaFile(sourceUrl, filename, accountTabId);
    },
    async downloadAvatarThumbnail(mxcUri: string): Promise<string> {
      return invokeAccount<string>("download_avatar_thumbnail", { mxcUri });
    },
    async cancelAvatarThumbnail(mxcUri: string, requestSequence: string) {
      await invokeAccount("cancel_avatar_thumbnail", { mxcUri, requestSequence });
    },
    async loadMessageSource(roomId: string, eventId: string) {
      await invokeAccount("load_message_source", { roomId, eventId });
    },
    async requestRoomKey(
      roomId: string,
      eventId: string,
      origin: "user" | "automatic",
      timelineKey?: TimelineKey
    ) {
      await invokeAccount("request_room_key", { roomId, eventId, origin, timelineKey });
    },
    async forwardMessage(
      roomId: string,
      sourceEventId: string,
      destinationRoomId: string
    ) {
      await invokeAccount("forward_message", { roomId, sourceEventId, destinationRoomId });
    },
    async loadLinkPreviews(roomId: string, eventId: string) {
      await invokeAccount("load_link_previews", { roomId, eventId });
    },
    async hideLinkPreview(roomId: string, eventId: string) {
      await invokeAccount("hide_link_preview", { roomId, eventId });
    },
    async observeViewport(
      roomId: string,
      firstVisibleEventId: string | null,
      lastVisibleEventId: string | null,
      visibleGapIds: TimelineGapId[],
      atBottom: boolean,
      bottomArrival: TimelineBottomArrival,
      threadRootEventId: string | null
    ) {
      await invokeAccount("observe_timeline_viewport", {
        roomId,
        firstVisibleEventId,
        lastVisibleEventId,
        visibleGapIds,
        atBottom,
        bottomArrival,
        threadRootEventId
      });
    },
    async updateScrollAnchor(roomId: string, anchor: TimelineScrollAnchor) {
      await invokeAccount("update_navigation_scroll_anchor", { roomId, anchor });
    },
    async openAtTimestamp(roomId: string, timestampMs: number) {
      await invokeAccount("open_timeline_at_timestamp", { roomId, timestampMs });
    }
  };
}
