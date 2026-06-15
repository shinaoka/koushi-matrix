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

const ipc = new TauriIpcMock();

window.__harness = {
  pushCoreEvent: (event) => ipc.emitCoreEvent(event),
  invocations: () => ipc.recordedInvocations(),
  invocationsOf: (command) => ipc.invocationsOf(command)
};

const transport: TimelineTransport = {
  listenCoreEvents(listener) {
    return ipc.listen("matrix-desktop://event", (envelope) => {
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
  downloadMedia(roomId, eventId) {
    return ipc.invoke("download_media", { roomId, eventId });
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
    onReply={(roomId, eventId) => {
      void ipc.invoke("set_composer_reply_target", { roomId, eventId });
    }}
  />
);
