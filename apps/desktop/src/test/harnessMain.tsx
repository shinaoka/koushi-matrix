/**
 * Playwright harness entry: mounts the REAL TimelineView component against
 * the mock IPC transport, and exposes a small control surface on `window`
 * so the headless-Chromium spec (e2e/timeline-scrollback.spec.ts) can push
 * fake CoreEvent payloads and read recorded command invocations.
 *
 * Loaded only by harness.html (never part of the production index.html
 * bundle). No Tauri, no network: everything flows through TauriIpcMock.
 */

import { createRoot } from "react-dom/client";

import type { CoreEventPayload } from "../domain/coreEvents";
import { roomTimelineKey } from "../domain/coreEvents";
import {
  TimelineView,
  type TimelineTransport
} from "../components/TimelineView";
import { TauriIpcMock, type IpcInvocation } from "./tauriIpcMock";

declare global {
  interface Window {
    __harness: {
      pushCoreEvent(event: CoreEventPayload): void;
      invocations(): readonly IpcInvocation[];
      invocationsOf(command: string): IpcInvocation[];
    };
  }
}

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const autoLoadOlderMessages =
  new URLSearchParams(window.location.search).get("autoLoadOlderMessages") === "true";

const ipc = new TauriIpcMock();

window.__harness = {
  pushCoreEvent: (event) => ipc.emitCoreEvent(event),
  invocations: () => ipc.recordedInvocations(),
  invocationsOf: (command) => ipc.invocationsOf(command)
};

const transport: TimelineTransport = {
  listenCoreEvents(listener) {
    return ipc.listen("koushi-desktop://event", (envelope) => {
      listener((envelope as { payload: CoreEventPayload }).payload);
    });
  },
  paginateBackwards(timelineKey) {
    if ("Room" in timelineKey.kind) {
      return ipc.invoke("paginate_timeline_backwards", {
        roomId: timelineKey.kind.Room.room_id
      });
    }
    if ("Thread" in timelineKey.kind) {
      return ipc.invoke("paginate_thread_timeline_backwards", {
        roomId: timelineKey.kind.Thread.room_id,
        rootEventId: timelineKey.kind.Thread.root_event_id
      });
    }
    return Promise.resolve();
  },
  sendReaction(roomId, eventId, reactionKey) {
    return ipc.invoke("send_reaction", { roomId, eventId, reactionKey });
  },
  retrySend(roomId, transactionId) {
    return ipc.invoke("retry_send", { roomId, transactionId });
  },
  cancelSend(roomId, transactionId) {
    return ipc.invoke("cancel_send", { roomId, transactionId });
  },
  redactReaction(roomId, eventId, reactionKey, reactionEventId) {
    return ipc.invoke("redact_reaction", {
      roomId,
      eventId,
      reactionKey,
      reactionEventId
    });
  },
  sendReadReceipt(roomId, eventId) {
    return ipc.invoke("send_read_receipt", { roomId, eventId });
  },
  setFullyRead(roomId, eventId) {
    return ipc.invoke("set_fully_read", { roomId, eventId });
  },
  setTyping(roomId, isTyping) {
    return ipc.invoke("set_typing", { roomId, isTyping });
  },
  editMessage(roomId, eventId, body) {
    return ipc.invoke("edit_message", { roomId, eventId, body });
  },
  redactMessage(roomId, eventId) {
    return ipc.invoke("redact_message", { roomId, eventId });
  },
  pinEvent(roomId, eventId) {
    return ipc.invoke("pin_event", { roomId, eventId });
  },
  unpinEvent(roomId, eventId) {
    return ipc.invoke("unpin_event", { roomId, eventId });
  },
  downloadMedia(roomId, eventId) {
    return ipc.invoke("download_media", { roomId, eventId });
  },
  loadMessageSource(roomId, eventId) {
    return ipc.invoke("load_message_source", { roomId, eventId });
  },
  requestRoomKey(roomId, eventId) {
    return ipc.invoke("request_room_key", { roomId, eventId });
  },
  forwardMessage(roomId, sourceEventId, destinationRoomId) {
    return ipc.invoke("forward_message", { roomId, sourceEventId, destinationRoomId });
  },
  loadLinkPreviews(roomId, eventId) {
    return ipc.invoke("load_link_previews", { roomId, eventId });
  },
  hideLinkPreview(roomId, eventId) {
    return ipc.invoke("hide_link_preview", { roomId, eventId });
  },
  observeViewport(roomId, firstVisibleEventId, lastVisibleEventId, atBottom) {
    return ipc.invoke("observe_timeline_viewport", {
      roomId,
      firstVisibleEventId,
      lastVisibleEventId,
      atBottom
    });
  },
  openAtTimestamp(roomId, timestampMs) {
    return ipc.invoke("open_timeline_at_timestamp", { roomId, timestampMs });
  }
};

const root = document.getElementById("harness-root");
if (!root) {
  throw new Error("harness root element missing");
}

createRoot(root).render(
  <TimelineView
    roomId={ROOM_ID}
    timelineKey={roomTimelineKey(ACCOUNT_KEY, ROOM_ID)}
    transport={transport}
    autoLoadOlderMessages={autoLoadOlderMessages}
    onReply={(roomId, eventId) => {
      void ipc.invoke("set_composer_reply_target", { roomId, eventId });
    }}
  />
);
