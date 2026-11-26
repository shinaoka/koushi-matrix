// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineItem, TimelineItemId } from "../domain/coreEvents";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";
import { setActiveLocaleProfile } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";

const OWN = "@alice:example.invalid";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

// #2136: the bar acts on every unsent message, so it must not live in the
// scroll content, where it scrolls away with the history.
it("renders the unsent bar outside the timeline's scroll content", async () => {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) {
      emit = listener;
      return () => undefined;
    }
  });
  render(
    <TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} currentUserId={OWN} onReply={vi.fn()} />
  );
  const scroller = await screen.findByTestId("timeline-view");
  const failed: TimelineItem = {
    ...message("$echo:example.invalid", "Failed"),
    id: { Transaction: { transaction_id: "1" } } as TimelineItemId,
    sender: OWN,
    send_state: { kind: "notSent", reason: "recoverable" }
  };
  act(() =>
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: [failed] } } as never
    })
  );
  const bar = await screen.findByTestId("timeline-send-bar");
  expect(scroller.contains(bar)).toBe(false);
});
