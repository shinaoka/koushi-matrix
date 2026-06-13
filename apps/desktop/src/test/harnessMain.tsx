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
  paginateBackwards(roomId) {
    return ipc.invoke("paginate_timeline_backwards", { roomId });
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
