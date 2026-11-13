// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ReplyQuote } from "../domain/coreEvents";
import type { ReplyQuoteMedia } from "../domain/coreEvents";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import type { TimelineMediaDownloadState } from "../domain/types";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { TimelineStoreContext } from "./timelineStoreContext";
import { clearTimelineViewportSessionMemoryForTests, TimelineView } from "./TimelineView";
import { replyQuotePreviewStyle } from "./timeline/ReplyQuoteThumbnail";
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

// jsdom serializes `aspect-ratio: 1.5` as `1.5 / 1`, so compare numbers.
function aspectRatioValue(value: unknown): number {
  const [width, height = "1"] = String(value).split("/");
  return Number(width) / Number(height);
}

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
  source_ref: "download/1234567890123456/6543210987654321/480x360",
  width: 480,
  height: 360,
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
    ["notRequested", undefined],
    ["pending", { kind: "pending", progress: null } as TimelineMediaDownloadState],
    ["failed", { kind: "failed", failure_kind: "network" } as TimelineMediaDownloadState],
    ["ready", READY]
  ])("reserves the original aspect ratio while %s so the quote does not jump", (_label, state) => {
    renderReply(imageQuote(), state);

    const preview = screen.getByRole("button", { name: "Open image from Alice" });
    expect(aspectRatioValue(preview.style.aspectRatio)).toBeCloseTo(640 / 480);
    expect(preview.style.inlineSize).toBe("213px");
  });

  // #1874: a video has no label either, but it keeps its play glyph so the
  // preview still reads as something to play rather than a still picture.
  it("shows a captionless video without a label and with the play glyph", () => {
    const quote = renderReply(
      imageQuote({
        media: {
          kind: "video",
          animated: false,
          width: 640,
          height: 480,
          has_thumbnail_source: false
        }
      }),
      READY
    );

    expect(quote.textContent).toContain("Alice");
    expect(quote.textContent).not.toContain(t("timeline.replyQuoteKindVideo"));
    expect(quote.querySelector(".reply-quote-thumbnail-play")).not.toBeNull();
  });
});

// #1874 removed the kind label only where a thumbnail stands in for it; file
// and audio quotes have no thumbnail, so their label or file name is the whole
// quote body.
describe("reply quote text line", () => {
  it.each(["file", "audio"] as const)(
    "keeps the kind label for a captionless %s quote",
    (kind) => {
      const quote = renderReply(
        imageQuote({
          media: {
            kind,
            animated: false,
            width: null,
            height: null,
            has_thumbnail_source: false
          }
        }),
        undefined
      );

      const label =
        kind === "file"
          ? t("timeline.replyQuoteKindFile")
          : t("timeline.replyQuoteKindAudio");
      expect(quote.querySelector(".reply-quote-body")?.textContent).toBe(label);
      expect(quote.querySelector(".reply-quote-thumbnail")).toBeNull();
    }
  );

  it("keeps a file name for a captionless file quote that has one", () => {
    const quote = renderReply(
      imageQuote({
        body_preview: "spectrum.csv",
        media: {
          kind: "file",
          animated: false,
          width: null,
          height: null,
          has_thumbnail_source: false
        }
      }),
      undefined
    );

    expect(quote.querySelector(".reply-quote-body")?.textContent).toBe("spectrum.csv");
  });
});

describe("replyQuotePreviewStyle", () => {
  function media(width: number | null, height: number | null): ReplyQuoteMedia {
    return { kind: "image", animated: false, width, height, has_thumbnail_source: false };
  }

  it.each([
    ["a 1:2 strip clamps to the minimum ratio", media(100, 400), 1 / 2, "80px"],
    ["a 3:1 panorama clamps to the maximum ratio", media(1200, 400), 3, "240px"],
    ["an in-range 4:3 image keeps its own ratio", media(640, 480), 640 / 480, "213px"],
    ["missing hints fall back to 4:3", media(null, null), 4 / 3, "213px"],
    ["zero hints fall back to 4:3", media(0, 0), 4 / 3, "213px"]
  ])("%s", (_label, hints, ratio, inlineSize) => {
    const style = replyQuotePreviewStyle(hints);

    expect(aspectRatioValue(style.aspectRatio)).toBeCloseTo(ratio);
    expect(style.inlineSize).toBe(inlineSize);
  });
});
