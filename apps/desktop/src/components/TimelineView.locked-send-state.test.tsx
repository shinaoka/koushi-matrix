// @vitest-environment jsdom
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CoreEventPayload, TimelineItem, TimelineItemId } from "../domain/coreEvents";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
});

const OWN = "@alice:example.invalid";

function echo(kind: "sending" | "notSent"): TimelineItem {
  return {
    ...message("$echo:example.invalid", "Queued"),
    id: { Transaction: { transaction_id: "txn-1" } } as TimelineItemId,
    sender: OWN,
    send_state: kind === "sending" ? { kind: "sending" } : { kind: "notSent", reason: "recoverable" }
  };
}

/** #2135: taking back a local message writes nothing, so the send lock must not hide it. */
function renderEcho(kind: "sending" | "notSent", canWrite: boolean) {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const transport = baseTransport({
    listenCoreEvents(listener) {
      emit = listener;
      return () => undefined;
    }
  });
  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      currentUserId={OWN}
      canWrite={canWrite}
      onReply={vi.fn()}
    />
  );
  act(() =>
    emit({
      kind: "Timeline",
      event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: [echo(kind)] } }
    })
  );
}

const has = (name: string) => screen.queryAllByRole("button", { name }).length > 0;

describe("send-state controls while sending is locked (#2135)", () => {
  it("queued row: Cancel available, never labelled Sending", () => {
    renderEcho("sending", false);
    expect(has(t("timeline.cancelSend"))).toBe(true);
    expect(screen.queryByText(t("timeline.sending"))).toBeNull();
    expect(screen.queryByText(t("timeline.sendWaiting"))).not.toBeNull();
    expect(screen.queryByText(t("timeline.notSent"))).toBeNull();
  });

  it("queued row is in the unsent bar, so row and bar agree", () => {
    renderEcho("sending", false);
    expect(has(t("timeline.cancelAll"))).toBe(true);
    expect(has(t("timeline.resendAll"))).toBe(false);
  });

  it("a queued row is not in the bar while sending is admitted", () => {
    renderEcho("sending", true);
    expect(has(t("timeline.cancelAll"))).toBe(false);
  });

  it("queued row keeps Sending and Cancel while sending is admitted", () => {
    renderEcho("sending", true);
    expect(has(t("timeline.cancelSend"))).toBe(true);
    expect(screen.queryByText(t("timeline.sending"))).not.toBeNull();
  });

  it("not-sent row: Delete available, Resend absent", () => {
    renderEcho("notSent", false);
    expect(has(t("timeline.deleteSend"))).toBe(true);
    expect(has(t("timeline.resendSend"))).toBe(false);
  });

  it("not-sent row offers both while sending is admitted", () => {
    renderEcho("notSent", true);
    expect(has(t("timeline.deleteSend"))).toBe(true);
    expect(has(t("timeline.resendSend"))).toBe(true);
  });

  it("unsent bar: Cancel all kept, Resend all only when admitted", () => {
    renderEcho("notSent", false);
    expect(has(t("timeline.cancelAll"))).toBe(true);
    expect(has(t("timeline.resendAll"))).toBe(false);
    cleanup();
    renderEcho("notSent", true);
    expect(has(t("timeline.resendAll"))).toBe(true);
  });
});
