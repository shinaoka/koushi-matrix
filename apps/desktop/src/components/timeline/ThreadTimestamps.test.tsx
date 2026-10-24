// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { message } from "../timelineViewTestSupport";
import { setActiveLocaleProfile, t, type Locale } from "../../i18n/messages";
import { TimelineItemRow } from "./TimelineItemRow";

const rootId = "$root:example.invalid";
const replyId = "$reply:example.invalid";
const posted = new Date(2026, 9, 19, 8, 49).getTime();
const replied = new Date(2026, 9, 22, 10, 20).getTime();

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 22, 12));
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  setActiveLocaleProfile("en");
});

function renderRoot(options: {
  timestamp?: number | null;
  latestTimestamp?: number | null;
  moved?: boolean;
  context?: "room" | "thread" | "focused";
  summary?: boolean;
  unread?: number;
} = {}) {
  const timestamp = options.timestamp === undefined ? posted : options.timestamp;
  const item = {
    ...message(rootId, "Synthetic thread root"),
    timestamp_ms: timestamp,
    thread_summary: options.summary === false ? null : {
      reply_count: 2,
      latest_event_id: replyId,
      latest_sender: "@member:example.invalid",
      latest_sender_label: "Member 1",
      latest_body_preview: "Synthetic reply",
      latest_timestamp_ms: options.latestTimestamp === undefined ? replied : options.latestTimestamp,
      unread_count: options.unread ?? 0
    }
  };
  const onOpenThread = vi.fn();
  const result = render(<TimelineItemRow
    item={item}
    contentEventId={rootId}
    activityEventId={options.moved === false ? rootId : replyId}
    contentTimestampMs={timestamp}
    roomId="!room:example.invalid"
    presentationContext={options.context ?? "room"}
    onReply={vi.fn()}
    onOpenThread={onOpenThread}
    onSendReaction={vi.fn()}
    onRedactReaction={vi.fn()}
    onEdit={vi.fn()}
    onRedact={vi.fn()}
  />);
  return { ...result, onOpenThread };
}

describe("thread posting and reply dates", () => {
  it("separates the original date from latest activity and keeps actions on the root", () => {
    const { container, onOpenThread } = renderRoot();
    expect(container.querySelector(".message-heading time")).toBeNull();
    const posting = container.querySelector(".message-posted-at")!;
    expect(posting?.textContent).toContain("Posted");
    expect(posting?.textContent).toContain("Oct 19");
    expect(posting?.textContent).not.toContain("Monday");
    const latest = container.querySelector(".thread-latest-reply")!;
    expect(latest?.textContent).toContain("Latest reply");
    expect(latest?.textContent).toContain("Today");
    expect(latest.querySelector("time")?.dateTime).toBe(new Date(replied).toISOString());
    const chip = container.querySelector<HTMLButtonElement>(".thread-summary-chip")!;
    expect(chip.querySelector("time")).toBeNull();
    fireEvent.click(chip);
    expect(onOpenThread).toHaveBeenCalledWith("!room:example.invalid", rootId, "existingThread");
    for (const time of [posting.querySelector("time")!, latest.querySelector("time")!]) {
      fireEvent.focus(time);
      expect(screen.getByRole("tooltip").textContent).toContain("2026");
      fireEvent.blur(time);
    }
  });

  it("also labels same-day moved roots and includes the year across New Year", () => {
    const { container, unmount } = renderRoot({ timestamp: new Date(2026, 9, 22, 8, 49).getTime() });
    expect(container.querySelector(".message-posted-at")?.textContent).toContain("Oct 22");
    unmount();
    const old = renderRoot({ timestamp: new Date(2025, 11, 31, 8, 49).getTime() });
    expect(old.container.querySelector(".message-posted-at")?.textContent).toContain("2025");
  });

  it.each(["thread", "focused"] as const)("keeps the usual heading in the %s surface", (context) => {
    const { container } = renderRoot({ context });
    expect(container.querySelector(".message-posted-at")).toBeNull();
    expect(container.querySelector(".message-heading time")).not.toBeNull();
  });

  it("keeps ordinary messages and roots at their original position compact", () => {
    const { container } = renderRoot({ moved: false, summary: false });
    expect(container.querySelector(".message-posted-at")).toBeNull();
    expect(container.querySelector(".message-heading time")).not.toBeNull();
    expect(container.querySelector(".thread-latest-reply")).toBeNull();
  });

  it("does not invent missing dates", () => {
    const { container } = renderRoot({ timestamp: null, latestTimestamp: null });
    expect(container.querySelector("time")).toBeNull();
    expect(container.querySelector(".message-posted-at")).toBeNull();
    expect(container.querySelector(".thread-latest-reply")).toBeNull();
  });

  it("localizes both labels and dates in Japanese", () => {
    setActiveLocaleProfile("ja");
    const { container } = renderRoot();
    expect(container.querySelector(".message-posted-at")?.textContent).toContain("投稿");
    expect(container.querySelector(".message-posted-at")?.textContent).toContain("10月19日");
    expect(container.querySelector(".thread-latest-reply")?.textContent).toContain("最終返信");
    expect(container.querySelector(".thread-latest-reply")?.textContent).toContain("今日");
  });

  it.each<Locale>(["en", "ja", "zh-Hans", "zh-Hant", "ko", "pseudo"])("uses the active %s catalog for both timestamps", (locale) => {
    setActiveLocaleProfile(locale);
    const { container } = renderRoot();
    expect(container.querySelector(".message-posted-at")?.textContent).toContain(t("timeline.postedAt"));
    expect(container.querySelector(".thread-latest-reply")?.textContent).toContain(t("timeline.latestReplyAt"));
    expect(container.querySelector(".thread-latest-reply time")?.textContent).toContain(t("timeline.timestampToday"));
  });
});

describe("thread unread badge", () => {
  it("overlays the unread count on the summary chip instead of a separate row", () => {
    const { container, onOpenThread } = renderRoot({ unread: 3 });
    expect(container.querySelector(".thread-new-replies-chip")).toBeNull();
    const chip = container.querySelector<HTMLButtonElement>(".thread-summary-chip")!;
    const badge = chip.querySelector(".thread-summary-unread-badge")!;
    expect(badge.textContent).toBe("3");
    expect(chip.getAttribute("aria-label")).toContain(t("timeline.threadNotificationCount", { count: 3 }));
    fireEvent.click(badge);
    expect(onOpenThread).toHaveBeenCalledWith("!room:example.invalid", rootId, "existingThread");
  });

  it("omits the badge when the thread has no unread replies", () => {
    const { container } = renderRoot();
    expect(container.querySelector(".thread-summary-unread-badge")).toBeNull();
  });
});
