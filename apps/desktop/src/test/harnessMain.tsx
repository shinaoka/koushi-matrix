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
import type { TimelineScrollDiagnostics } from "../domain/timelineScrollDiagnostics";
import type { TimelineMediaDownloadState } from "../domain/types";
import {
  TimelineView,
  type TimelineTransport
} from "../components/TimelineView";
import { TauriIpcMock, type IpcInvocation } from "./tauriIpcMock";

declare global {
  interface Window {
    __harness: {
      pushCoreEvent(event: CoreEventPayload): void;
      setMediaDownloadState(eventId: string, state: TimelineMediaDownloadState): void;
      invocations(): readonly IpcInvocation[];
      invocationsOf(command: string): IpcInvocation[];
      scrollDiagnostics(): TimelineScrollDiagnostics | null;
      resetScrollDiagnostics(): void;
    };
  }
}

const ACCOUNT_KEY = "@harness-user:example.invalid";
const ROOM_ID = "!harness-room:example.invalid";
const autoLoadOlderMessages =
  new URLSearchParams(window.location.search).get("autoLoadOlderMessages") === "true";
const variableHeights =
  new URLSearchParams(window.location.search).get("variableHeights") === "true";

if (variableHeights) {
  document.body.dataset.variableHeights = "true";
}

const ipc = new TauriIpcMock();
let latestScrollDiagnostics: TimelineScrollDiagnostics | null = null;
let scrollDiagnosticsBaseline: TimelineScrollDiagnostics | null = null;
let mediaDownloads: Record<string, TimelineMediaDownloadState> = {};
let renderTimeline: () => void = () => undefined;

function cloneScrollDiagnostics(
  diagnostics: TimelineScrollDiagnostics
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    scrollWrites: { ...diagnostics.scrollWrites },
    latestFrame: diagnostics.latestFrame ? { ...diagnostics.latestFrame } : null,
    estimateBuckets: Object.fromEntries(
      Object.entries(diagnostics.estimateBuckets).map(([kind, bucket]) => [
        kind,
        { ...bucket }
      ])
    ) as TimelineScrollDiagnostics["estimateBuckets"]
  };
}

function subtractCounter(current: number, baseline: number): number {
  return Math.max(0, current - baseline);
}

function diagnosticsSinceBaseline(
  current: TimelineScrollDiagnostics,
  baseline: TimelineScrollDiagnostics | null
): TimelineScrollDiagnostics {
  const snapshot = cloneScrollDiagnostics(current);
  if (!baseline) {
    return snapshot;
  }
  return {
    ...snapshot,
    renderCommits: subtractCounter(current.renderCommits, baseline.renderCommits),
    scrollFrames: subtractCounter(current.scrollFrames, baseline.scrollFrames),
    activeScrollFrames: subtractCounter(
      current.activeScrollFrames,
      baseline.activeScrollFrames
    ),
    rangeCommits: subtractCounter(current.rangeCommits, baseline.rangeCommits),
    heightModelCommits: subtractCounter(
      current.heightModelCommits,
      baseline.heightModelCommits
    ),
    measurementFlushes: subtractCounter(
      current.measurementFlushes,
      baseline.measurementFlushes
    ),
    maxAnchorTopDeltaPx:
      current.maxAnchorTopDeltaPx > baseline.maxAnchorTopDeltaPx
        ? current.maxAnchorTopDeltaPx
        : 0,
    scrollWrites: Object.fromEntries(
      Object.entries(current.scrollWrites).map(([reason, count]) => [
        reason,
        subtractCounter(
          count,
          baseline.scrollWrites[reason as keyof TimelineScrollDiagnostics["scrollWrites"]]
        )
      ])
    ) as TimelineScrollDiagnostics["scrollWrites"],
    estimateBuckets: Object.fromEntries(
      Object.entries(current.estimateBuckets).map(([kind, bucket]) => {
        const baselineBucket =
          baseline.estimateBuckets[kind as keyof TimelineScrollDiagnostics["estimateBuckets"]];
        return [
          kind,
          {
            count: subtractCounter(bucket.count, baselineBucket.count),
            maxAbsErrorPx:
              bucket.maxAbsErrorPx > baselineBucket.maxAbsErrorPx
                ? bucket.maxAbsErrorPx
                : 0,
            totalAbsErrorPx: subtractCounter(
              bucket.totalAbsErrorPx,
              baselineBucket.totalAbsErrorPx
            )
          }
        ];
      })
    ) as TimelineScrollDiagnostics["estimateBuckets"]
  };
}

window.__harness = {
  pushCoreEvent: (event) => ipc.emitCoreEvent(event),
  setMediaDownloadState: (eventId, state) => {
    mediaDownloads = { ...mediaDownloads, [eventId]: state };
    renderTimeline();
  },
  invocations: () => ipc.recordedInvocations(),
  invocationsOf: (command) => ipc.invocationsOf(command),
  scrollDiagnostics: () =>
    latestScrollDiagnostics
      ? diagnosticsSinceBaseline(latestScrollDiagnostics, scrollDiagnosticsBaseline)
      : null,
  resetScrollDiagnostics: () => {
    scrollDiagnosticsBaseline = latestScrollDiagnostics
      ? cloneScrollDiagnostics(latestScrollDiagnostics)
      : null;
  }
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
  sendReadReceipt(roomId, eventId, threadRootEventId) {
    return ipc.invoke("send_read_receipt", { roomId, eventId, threadRootEventId });
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

const timelineRoot = createRoot(root);
renderTimeline = () => {
  timelineRoot.render(
    <TimelineView
      roomId={ROOM_ID}
      timelineKey={roomTimelineKey(ACCOUNT_KEY, ROOM_ID)}
      transport={transport}
      autoLoadOlderMessages={autoLoadOlderMessages}
      mediaDownloads={mediaDownloads}
      onReply={(roomId, eventId) => {
        void ipc.invoke("set_composer_reply_target", { roomId, eventId });
      }}
      onScrollDiagnosticsChange={(diagnostics) => {
        latestScrollDiagnostics = diagnostics;
      }}
    />
  );
};
renderTimeline();
