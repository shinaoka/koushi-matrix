// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { roomTimelineKey, type CoreEventPayload, type TimelineItem } from "../domain/coreEvents";
import { TimelineView, type TimelineTransport } from "./TimelineView";

afterEach(cleanup);

const KEY = roomTimelineKey("@alice:example.invalid", "!room:example.invalid");

function message(eventId: string, body: string): TimelineItem {
  return {
    id: { Event: { event_id: eventId } },
    sender: "@bob:example.invalid",
    body,
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    thread_root: null,
    thread_summary: null,
    can_react: true,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: []
  };
}

function baseTransport(
  overrides: Partial<TimelineTransport>
): TimelineTransport {
  return {
    listenCoreEvents: () => () => undefined,
    paginateBackwards: async () => undefined,
    sendReaction: async () => undefined,
    retrySend: async () => undefined,
    cancelSend: async () => undefined,
    redactReaction: async () => undefined,
    sendReadReceipt: async () => undefined,
    setFullyRead: async () => undefined,
    setTyping: async () => undefined,
    editMessage: async () => undefined,
    redactMessage: async () => undefined,
    pinEvent: async () => undefined,
    unpinEvent: async () => undefined,
    downloadMedia: async () => undefined,
    loadMessageSource: async () => undefined,
    forwardMessage: async () => undefined,
    loadLinkPreviews: async () => undefined,
    hideLinkPreview: async () => undefined,
    ...overrides
  };
}

describe("TimelineView", () => {
  it("ensures the timeline subscription after registering the CoreEvent listener", async () => {
    const calls: string[] = [];
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        calls.push("listen");
        listener = nextListener;
        return () => undefined;
      },
      async ensureSubscribed(timelineKey) {
        calls.push("ensure");
        expect(timelineKey).toEqual(KEY);
        listener?.({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key: KEY,
              generation: 1,
              items: [message("$latest", "Latest after listener")]
            }
          }
        });
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Latest after listener")).toBeTruthy();
    });
    expect(calls).toEqual(["listen", "ensure"]);
  });
});
