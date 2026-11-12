// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ReplyQuote } from "../domain/coreEvents";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import type { TimelineMediaDownloadState } from "../domain/types";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { TimelineStoreContext } from "./timelineStoreContext";
import { clearTimelineViewportSessionMemoryForTests, TimelineView } from "./TimelineView";
import { KEY, baseTransport, message } from "./timelineViewTestSupport";

// Reply quotes of an image show the picture as a large inline preview (the
// quote's content), not a 40 px trailing chip next to a redundant "Image"
// kind label.

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.restoreAllMocks();
});

const QUOTED = "$picture-root:example.invalid";

function imageQuote(overrides: Partial<ReplyQuote> = {}): ReplyQuote {
  return {
    event_id: QUOTED,
    sender: "@alice:example.invalid",
    sender_label: "Alice",
    body_preview: null,
    media: {
      kind: "image",
      animated: false,
      width: 640,
      height: 480,
      has_thumbnail_source: false
    },
    permalink:
      "https://matrix.to/#/!room%3Aexample.invalid/%24picture-root%3Aexample.invalid",
    can_open: true,
    state: "ready",
    ...overrides
  };
}

function renderReply(
  quote: ReplyQuote,
  thumbnail: TimelineMediaDownloadState | undefined
) {
  const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
    InitialItems: {
      request_id: null,
      key: KEY,
      generation: 1,
      items: [
        {
          ...message("$reply", "What does this axis mean?"),
          in_reply_to_event_id: quote.event_id,
          reply_quote: quote
        }
      ]
    }
  });
  render(
    <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={baseTransport({ requestReplyQuoteThumbnail: vi.fn(async () => undefined) })}
        replyQuoteThumbnails={thumbnail ? { [quote.event_id]: thumbnail } : {}}
        onReply={vi.fn()}
        onOpenThread={vi.fn()}
      />
    </TimelineStoreContext.Provider>
  );
  const row = screen.getByText("What does this axis mean?").closest("article")!;
  return row.querySelector<HTMLElement>(".reply-quote")!;
}

const READY: TimelineMediaDownloadState = {
  kind: "ready",
  source_ref: "download/1234567890123456/6543210987654321/96x96",
  width: 96,
  height: 96,
  mime_type: "image/jpeg"
};

describe("reply quote image preview", () => {
  it("shows a captionless image as the quote content without a kind label", () => {
    const quote = renderReply(imageQuote(), READY);

    expect(quote.textContent).toContain("Alice");
    expect(quote.textContent).not.toContain(t("timeline.replyQuoteKindImage"));

    const preview = screen.getByRole("button", { name: "Open image from Alice" });
    // The preview is the quote's body, under the sender, not a trailing chip.
    expect(quote.querySelector(".reply-quote-copy")?.contains(preview)).toBe(true);
    expect(quote.classList.contains("has-thumbnail")).toBe(false);
    expect(preview.querySelector("img")).not.toBeNull();
  });

  it("does not show the kind label in other locales either", () => {
    setActiveLocaleProfile("zh-Hans", "none");
    const quote = renderReply(imageQuote(), READY);

    expect(quote.textContent).not.toContain(t("timeline.replyQuoteKindImage"));
  });

  it("keeps a caption next to the preview", () => {
    const quote = renderReply(imageQuote({ body_preview: "Energy versus coupling" }), READY);

    expect(quote.textContent).toContain("Energy versus coupling");
    expect(
      screen.getByRole("button", { name: "Open image from Alice" }).querySelector("img")
    ).not.toBeNull();
  });

  it.each([
    ["pending", { kind: "pending", progress: null } as TimelineMediaDownloadState],
    ["failed", { kind: "failed", failure_kind: "network" } as TimelineMediaDownloadState],
    ["ready", READY]
  ])("reserves the original aspect ratio while %s so the quote does not jump", (_label, state) => {
    renderReply(imageQuote(), state);

    const preview = screen.getByRole("button", { name: "Open image from Alice" });
    expect(preview.style.aspectRatio.replace(/\s/g, "")).toBe("640/480");
  });
});
