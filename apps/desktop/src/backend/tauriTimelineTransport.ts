import { invoke } from "@tauri-apps/api/core";

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
const tauriTimelineTransport: TimelineTransport | null = isTauriRuntime()
  ? {
      listenCoreEvents(listener: (payload: CoreEventPayload) => void) {
        let disposed = false;
        let unlisten: (() => void) | null = null;
        tauriCoreEventListenerReady = desktopEventPort.listenCoreEvents(listener).then((dispose) => {
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
        await invoke("ensure_timeline_subscribed", { timelineKey });
      },
      async paginateBackwards(timelineKey: TimelineKey) {
        if ("Room" in timelineKey.kind) {
          await invoke("paginate_timeline_backwards", {
            roomId: timelineKey.kind.Room.room_id
          });
          return;
        }
        if ("Thread" in timelineKey.kind) {
          await invoke("paginate_thread_timeline_backwards", {
            roomId: timelineKey.kind.Thread.room_id,
            rootEventId: timelineKey.kind.Thread.root_event_id
          });
        }
      },
      async repairTimeline(roomId: string) {
        await invoke("repair_room_timeline", { roomId });
      },
      async sendReaction(roomId: string, eventId: string, reactionKey: string) {
        await invoke("send_reaction", { roomId, eventId, reactionKey });
      },
      async retrySend(roomId: string, transactionId: string) {
        await invoke("retry_send", { roomId, transactionId });
      },
      async cancelSend(roomId: string, transactionId: string) {
        await invoke("cancel_send", { roomId, transactionId });
      },
      async redactReaction(
        roomId: string,
        eventId: string,
        reactionKey: string,
        reactionEventId: string
      ) {
        await invoke("redact_reaction", {
          roomId,
          eventId,
          reactionKey,
          reactionEventId
        });
      },
      async sendReadReceipt(roomId: string, eventId: string, threadRootEventId?: string | null) {
        await invoke("send_read_receipt", { roomId, eventId, threadRootEventId });
      },
      async setFullyRead(roomId: string, eventId: string) {
        await invoke("set_fully_read", { roomId, eventId });
      },
      async setTyping(roomId: string, isTyping: boolean) {
        await invoke("set_typing", { roomId, isTyping });
      },
      async editMessage(
        roomId: string,
        eventId: string,
        document: ComposerDocument
      ) {
        await invoke("edit_message", { roomId, eventId, document });
      },
      async redactMessage(roomId: string, eventId: string) {
        await invoke("redact_message", { roomId, eventId });
      },
      async pinEvent(roomId: string, eventId: string) {
        await invoke("pin_event", { roomId, eventId });
      },
      async unpinEvent(roomId: string, eventId: string) {
        await invoke("unpin_event", { roomId, eventId });
      },
      async downloadMedia(roomId: string, eventId: string) {
        await invoke("download_media", { roomId, eventId });
      },
      async saveMediaFile(sourceUrl: string, filename: string) {
        await saveReadyMediaFile(sourceUrl, filename);
      },
      async downloadAvatarThumbnail(mxcUri: string): Promise<string> {
        return invoke<string>("download_avatar_thumbnail", { mxcUri });
      },
      async cancelAvatarThumbnail(mxcUri: string, requestSequence: string) {
        await invoke("cancel_avatar_thumbnail", { mxcUri, requestSequence });
      },
      async loadMessageSource(roomId: string, eventId: string) {
        await invoke("load_message_source", { roomId, eventId });
      },
      async requestRoomKey(
        roomId: string,
        eventId: string,
        origin: "user" | "automatic",
        timelineKey?: TimelineKey
      ) {
        await invoke("request_room_key", { roomId, eventId, origin, timelineKey });
      },
      async forwardMessage(
        roomId: string,
        sourceEventId: string,
        destinationRoomId: string
      ) {
        await invoke("forward_message", { roomId, sourceEventId, destinationRoomId });
      },
      async loadLinkPreviews(roomId: string, eventId: string) {
        await invoke("load_link_previews", { roomId, eventId });
      },
      async hideLinkPreview(roomId: string, eventId: string) {
        await invoke("hide_link_preview", { roomId, eventId });
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
        await invoke("observe_timeline_viewport", {
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
        await invoke("update_navigation_scroll_anchor", { roomId, anchor });
      },
      async openAtTimestamp(roomId: string, timestampMs: number) {
        await invoke("open_timeline_at_timestamp", { roomId, timestampMs });
      }
    }
  : null;

export { tauriTimelineTransport };
