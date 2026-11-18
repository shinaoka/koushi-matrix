// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type CoreEventPayload, type TimelineItem } from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import type { RoomLatestEventSummary } from "../domain/types";
import { TimelineStoreContext } from "./timelineStoreContext";
import {
  clearTimelineViewportSessionMemoryForTests,
  roomLatestDisplayEventId,
  TimelineView
} from "./TimelineView";
import {
  KEY,
  baseTransport,
  message,
  navigationSnapshot
} from "./timelineViewTestSupport";

function latestEventSummary(
  overrides: Partial<RoomLatestEventSummary> = {}
): RoomLatestEventSummary {
  return {
    event_id: "$event:example.invalid",
    is_redacted: false,
    relation_type: null,
    relation_event_id: null,
    sender_id: null,
    sender_label: null,
    sender_avatar: null,
    preview: null,
    timestamp_ms: 1_800_000_000_000,
    ...overrides
  };
}

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("TimelineView", () => {

  it.each(["en", "ja"] as const)("renders content-policy notices and replacement navigation in %s", async (locale) => {
    setActiveLocaleProfile(locale, "none");
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const openTarget = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(listener) { emit = listener; return () => undefined; }
    });
    render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} onOpenMatrixTarget={openTarget} />);
    const entries: TimelineItem[] = [
      { ...message("$topic", "fallback topic"), message_kind: "notice", notice_i18n: { key: "timeline.notice.roomTopicSet", value: "<Synthetic topic>" } },
      { ...message("$malformed", "fallback malformed"), message_kind: "notice", notice_i18n: { key: "timeline.notice.malformedEvent" } },
      { ...message("$utd", "fallback decryption"), message_kind: "notice", notice_i18n: { key: "timeline.notice.unableToDecrypt" } },
      { ...message("$upgrade", "fallback upgrade"), message_kind: "notice", notice_i18n: { key: "timeline.notice.roomUpgraded", replacement_room_id: "!new:example.invalid" } },
      { ...message("$hidden", ""), body: null, is_hidden: true }
    ];
    act(() => emit({ kind: "Timeline", event: { InitialItems: { request_id: null, key: KEY, generation: 1, items: entries } } }));
    expect(await screen.findByText(locale === "en" ? "set the room topic to <Synthetic topic>" : "トピックを「<Synthetic topic>」に設定しました")).toBeTruthy();
    expect(screen.getByText(locale === "en" ? "This event could not be read." : "このイベントを読み取れませんでした。")).toBeTruthy();
    expect(screen.getByText(locale === "en" ? "Unable to decrypt message" : "メッセージを復号できません")).toBeTruthy();
    fireEvent.click(screen.getByRole("link", { name: locale === "en" ? "Open replacement room" : "新しいルームを開く" }));
    expect(openTarget).toHaveBeenCalledWith({ kind: "room", roomIdOrAlias: "!new:example.invalid", viaServers: [] });
    expect(document.querySelector('[data-event-id="$hidden"]')).toBeNull();
    expect(document.querySelector("Synthetic")).toBeNull();
  });

  it.each([
    ["ordinary event", latestEventSummary(), "$event:example.invalid"],
    ["redacted event", latestEventSummary({ is_redacted: true }), null],
    [
      "message edit",
      latestEventSummary({ relation_type: "m.replace", relation_event_id: "$target:example.invalid" }),
      null
    ],
    [
      "reaction annotation",
      latestEventSummary({ relation_type: "m.annotation", relation_event_id: "$target:example.invalid" }),
      null
    ],
    [
      "relation without a target",
      latestEventSummary({ relation_type: "m.replace", relation_event_id: null }),
      null
    ],
    [
      "other relation",
      latestEventSummary({ relation_type: "m.reference", relation_event_id: "$target:example.invalid" }),
      null
    ],
    [
      "thread reply with an identified root",
      latestEventSummary({ thread_root_event_id: "$root:example.invalid" }),
      null
    ]
  ])("only maps an ordinary room summary to a display event for %s", (_label, summary, expected) => {
    expect(roomLatestDisplayEventId(summary)).toBe(expected);
  });

  it("marks only same-sender continuation rows", async () => {
    const first = message("$first:example.invalid", "First");
    const second = {
      ...message("$second:example.invalid", "Second"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Reply",
        latest_timestamp_ms: 1_800_000_000_100
      }
    };
    const differentSender = {
      ...message("$different:example.invalid", "Different sender"),
      sender: "@carol:example.invalid"
    };
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [first, second, differentSender]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const firstRow = await screen.findByText("First");
    const secondRow = screen.getByText("Second");
    const differentSenderRow = screen.getByText("Different sender");
    expect(firstRow.closest("article")?.classList.contains("is-continuation")).toBe(false);
    expect(secondRow.closest("article")?.classList.contains("is-continuation")).toBe(true);
    expect(differentSenderRow.closest("article")?.classList.contains("is-continuation")).toBe(false);
    expect(screen.getAllByRole("article")).toHaveLength(3);
  });

  it.each([
    [
      "an unread marker on the current row",
      navigationSnapshot({
        first_unread_event_id: "$second:example.invalid",
        first_unread_display_event_id: "$second:example.invalid",
        unread_event_count: 1,
        unread_position: "insideViewport"
      })
    ],
    [
      "a read marker after the preceding row",
      navigationSnapshot({
        read_marker_event_id: "$first:example.invalid",
        read_marker_display_event_id: "$first:example.invalid"
      })
    ]
  ])("breaks continuation runs at %s", async (_label, snapshot) => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
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

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$first:example.invalid", "First"),
              message("$second:example.invalid", "Second")
            ]
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: { key: KEY, snapshot }
        }
      });
    });

    const secondRow = await screen.findByText("Second");
    expect(secondRow.closest("article")?.classList.contains("is-continuation")).toBe(false);
  });

  it("breaks continuation runs at a date divider and a timeline gap", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const dateDivider: TimelineItem = {
      ...message("$date-divider-source:example.invalid", ""),
      id: { Synthetic: { synthetic_id: "date-divider-1800000000000" } },
      sender: null,
      body: null
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$date-first:example.invalid", "Date first"),
              dateDivider,
              message("$date-second:example.invalid", "Date second")
            ]
          }
        }
      });
    });

    const dateSecond = await screen.findByText("Date second");
    expect(dateSecond.closest("article")?.classList.contains("is-continuation")).toBe(false);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          GapPositionsUpdated: {
            key: KEY,
            actor_generation: 0,
            generation: 1,
            positions: [{ id: { topology_revision: "1", ordinal: 0 }, before_item_index: 1 }]
          }
        }
      });
    });

    await waitFor(() => {
      const second = screen.getByText("Date second");
      expect(second.closest("article")?.classList.contains("is-continuation")).toBe(false);
    });
    expect(screen.getByTestId("timeline-gap-row")).toBeTruthy();
  });

  it("uses the full visible row list across a virtual window boundary", async () => {
    const items = Array.from({ length: 601 }, (_, index) =>
      message(`$virtual-${index}:example.invalid`, `Virtual ${index}`)
    );
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 601 * 72, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 5_000,
      writable: true,
      configurable: true
    });
    fireEvent.wheel(timeline, { deltaY: 1 });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(timeline.getAttribute("data-virtualized")).toBe("true");
      // The first row of the mounted window has no rendered predecessor, so
      // grouping has to come from the full visible row list, not the window.
      const firstRow = document.querySelector<HTMLElement>("[data-item-id]");
      expect(firstRow).not.toBeNull();
      expect(firstRow?.classList.contains("is-continuation")).toBe(true);
      const index = Number(
        firstRow?.getAttribute("data-event-id")?.split("-").at(-1)?.split(":")[0]
      );
      expect(index).toBeGreaterThan(0);
      expect(screen.getByText(`Virtual ${index}`)).toBeTruthy();
    });
    expect(timeline.getAttribute("data-total-items")).toBe("601");
  });

  it("preserves formatted Markdown structure inside a reply quote", () => {
    const formattedQuote = {
      event_id: "$formatted-root:example.invalid",
      sender: "@bob:example.invalid",
      sender_label: "Bob",
      body_preview: "fallback preview",
      formatted: {
        html: '<p>Opening</p><ul><li>one</li><li>two</li></ul><p><a href="https://example.com">link</a></p><pre><code class="language-rust">fn main() {}</code></pre>',
        plain_text: "Openingonetwolinkfn main() {}",
        code_blocks: [{ language: "rust", body: "fn main() {}" }]
      },
      state: "ready"
    } as unknown as NonNullable<TimelineItem["reply_quote"]>;
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$formatted-reply", "Reply with a formatted quote"),
            in_reply_to_event_id: "$formatted-root:example.invalid",
            reply_quote: formattedQuote
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Reply with a formatted quote").closest("article");
    expect(row).not.toBeNull();
    const quote = row!.querySelector<HTMLElement>(".reply-quote");
    expect(quote?.querySelector("ul")).not.toBeNull();
    expect(quote?.querySelectorAll("li")).toHaveLength(2);
    expect(quote?.querySelector('a[href="https://example.com/"]')).not.toBeNull();
    expect(quote?.querySelector(".message-code-block-pre code")?.textContent).toBe(
      "fn main() {}"
    );
    expect(quote?.textContent).not.toContain("fallback preview");
  });

  it("turns an openable reply quote into one Rust-linked control without nested anchors", () => {
    const openQuote = vi.fn();
    const permalink =
      "https://matrix.to/#/!room%3Aexample.invalid/%24openable-root%3Aexample.invalid";
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$openable-reply", "Reply with an openable quote"),
            in_reply_to_event_id: "$openable-root:example.invalid",
            reply_quote: {
              event_id: "$openable-root:example.invalid",
              sender: "@alice:example.invalid",
              sender_label: "Alice",
              body_preview: "Original quoted body",
              permalink,
              can_open: true,
              state: "ready"
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
          onOpenReplyQuote={openQuote}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Reply with an openable quote").closest("article");
    const quote = row!.querySelector<HTMLElement>(".reply-quote")!;
    expect(quote.classList.contains("is-openable")).toBe(true);
    const link = within(quote).getByRole("link", { name: "Open original message" });
    expect(link.getAttribute("href")).toBe(permalink);

    fireEvent.keyDown(link, { key: "Enter" });
    fireEvent.keyDown(link, { key: " " });
    fireEvent.click(link);
    fireEvent.click(within(quote).getByText("Original quoted body"));

    expect(openQuote).toHaveBeenCalledTimes(4);
    for (const call of openQuote.mock.calls) {
      expect(call).toEqual([
        "!room:example.invalid",
        "$openable-root:example.invalid"
      ]);
    }
    expect(quote.querySelectorAll("a a")).toHaveLength(0);
  });

  it("keeps a link inside an openable reply quote on its own target", () => {
    const openQuote = vi.fn();
    const openTarget = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$inner-link-reply", "Reply with an inner link quote"),
            in_reply_to_event_id: "$inner-link-root:example.invalid",
            reply_quote: {
              event_id: "$inner-link-root:example.invalid",
              sender: "@alice:example.invalid",
              sender_label: "Alice",
              body_preview: "Original with a link",
              permalink:
                "https://matrix.to/#/!room%3Aexample.invalid/%24inner-link-root%3Aexample.invalid",
              can_open: true,
              formatted: {
                html: '<p>Original <a href="https://matrix.to/#/%40bob%3Aexample.invalid">@Bob</a></p>',
                plain_text: "Original @Bob",
                code_blocks: []
              },
              state: "ready"
            }
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
          onOpenMatrixTarget={openTarget}
          onOpenReplyQuote={openQuote}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Reply with an inner link quote").closest("article");
    const quote = row!.querySelector<HTMLElement>(".reply-quote")!;
    fireEvent.click(within(quote).getByRole("link", { name: "@Bob" }));

    expect(openQuote).not.toHaveBeenCalled();
    expect(openTarget).toHaveBeenCalledWith({
      kind: "user",
      userId: "@bob:example.invalid"
    });
  });

  it.each(["loading", "missing", "unsupported", "failed"] as const)(
    "keeps a %s reply quote inert without link affordances",
    (state) => {
      const openQuote = vi.fn();
      const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message(`$inert-${state}-reply`, "Reply with an inert quote"),
              in_reply_to_event_id: "$inert-root:example.invalid",
              reply_quote: {
                event_id: "$inert-root:example.invalid",
                sender: "@alice:example.invalid",
                sender_label: "Alice",
                body_preview: null,
                permalink:
                  "https://matrix.to/#/!room%3Aexample.invalid/%24inert-root%3Aexample.invalid",
                can_open: false,
                state
              }
            }
          ]
        }
      });

      render(
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
            onOpenThread={vi.fn()}
            onOpenReplyQuote={openQuote}
          />
        </TimelineStoreContext.Provider>
      );

      const row = screen.getByText("Reply with an inert quote").closest("article");
      const quote = row!.querySelector<HTMLElement>(".reply-quote")!;
      expect(quote.classList.contains("is-openable")).toBe(false);
      expect(quote.querySelector(".reply-quote-link")).toBeNull();
      expect(quote.querySelector("[tabindex]")).toBeNull();
      fireEvent.click(quote);
      expect(openQuote).not.toHaveBeenCalled();
    }
  );

  it("renders a shared image thumbnail slot for every reply to one quoted picture", async () => {
    const requestThumbnail = vi.fn(async () => undefined);
    const quotedEventId = "$picture-root:example.invalid";
    const quote = {
      event_id: quotedEventId,
      sender: "@alice:example.invalid",
      sender_label: "Alice",
      body_preview: null,
      media: {
        kind: "image" as const,
        animated: false,
        width: 640,
        height: 480,
        has_thumbnail_source: false
      },
      permalink:
        "https://matrix.to/#/!room%3Aexample.invalid/%24picture-root%3Aexample.invalid",
      can_open: true,
      state: "ready" as const
    };
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$image-reply-a", "first reply"),
            in_reply_to_event_id: quotedEventId,
            reply_quote: quote
          },
          {
            ...message("$image-reply-b", "second reply"),
            in_reply_to_event_id: quotedEventId,
            reply_quote: quote
          }
        ]
      }
    });

    const view = (thumbnails: Record<string, import("../domain/types").TimelineMediaDownloadState>) => (
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ requestReplyQuoteThumbnail: requestThumbnail })}
          replyQuoteThumbnails={thumbnails}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    const { rerender } = render(view({}));
    await waitFor(() =>
      expect(requestThumbnail).toHaveBeenCalledTimes(1)
    );
    expect(requestThumbnail).toHaveBeenCalledWith(
      "!room:example.invalid",
      quotedEventId,
      KEY
    );

    rerender(
      view({
        [quotedEventId]: {
          kind: "ready",
          source_ref: "download/1234567890123456/6543210987654321/480x360",
          width: 480,
          height: 360,
          mime_type: "image/jpeg"
        }
      })
    );
    const slots = screen.getAllByRole("button", { name: "Open image from Alice" });
    expect(slots).toHaveLength(2);
    for (const slot of slots) {
      expect(slot.getAttribute("data-thumbnail-state")).toBe("ready");
      expect(slot.querySelector("img")?.getAttribute("src")).toBe(
        "koushi-media://localhost/download/jpg/1234567890123456/6543210987654321/480x360"
      );
    }
  });

  it("keeps a neutral thumbnail placeholder pending and after failure", () => {
    const quote = {
      event_id: "$placeholder-root:example.invalid",
      sender: "@alice:example.invalid",
      sender_label: "Alice",
      body_preview: "quoted caption",
      media: {
        kind: "image" as const,
        animated: false,
        width: 640,
        height: 480,
        has_thumbnail_source: false
      },
      permalink:
        "https://matrix.to/#/!room%3Aexample.invalid/%24placeholder-root%3Aexample.invalid",
      can_open: true,
      state: "ready" as const
    };
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$placeholder-reply", "reply"),
            in_reply_to_event_id: quote.event_id,
            reply_quote: quote
          }
        ]
      }
    });
    const view = (state: "pending" | "failed") => (
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          replyQuoteThumbnails={{
            [quote.event_id]:
              state === "pending"
                ? { kind: "pending", progress: null }
                : { kind: "failed", failure_kind: "network" }
          }}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    const { rerender } = render(view("pending"));
    let slot = screen.getByRole("button", { name: "Open image from Alice" });
    expect(slot.getAttribute("data-thumbnail-state")).toBe("pending");
    expect(slot.querySelector("img")).toBeNull();
    rerender(view("failed"));
    slot = screen.getByRole("button", { name: "Open image from Alice" });
    expect(slot.getAttribute("data-thumbnail-state")).toBe("failed");
    expect(slot.querySelector("img")).toBeNull();
    expect(slot.textContent).toBe("");
  });

  it("renders no thumbnail slot for a text or redacted quote", () => {
    const textQuote = {
      event_id: "$text-root:example.invalid",
      sender: "@alice:example.invalid",
      sender_label: "Alice",
      body_preview: "plain quote",
      state: "ready" as const
    };
    const redactedQuote = {
      event_id: "$redacted-root:example.invalid",
      sender: "@alice:example.invalid",
      sender_label: "Alice",
      body_preview: null,
      media: {
        kind: "image" as const,
        animated: false,
        width: 640,
        height: 480,
        has_thumbnail_source: false
      },
      state: "redacted" as const
    };
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$text-quote-reply", "reply"),
            in_reply_to_event_id: textQuote.event_id,
            reply_quote: textQuote
          },
          {
            ...message("$redacted-quote-reply", "reply"),
            in_reply_to_event_id: redactedQuote.event_id,
            reply_quote: redactedQuote
          }
        ]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onOpenThread={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    expect(screen.queryByRole("button", { name: /Open image from/ })).toBeNull();
  });

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

  it("skips the fallback timeline subscription when InitialItems arrive after listener registration", async () => {
    vi.useFakeTimers();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const ensureSubscribed = vi.fn().mockResolvedValue(undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      ensureSubscribed
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$selected-room-initial", "Initial from select_room")]
          }
        }
      });
    });
    act(() => {
      vi.advanceTimersByTime(1_000);
    });

    expect(screen.getByText("Initial from select_room")).toBeTruthy();
    expect(ensureSubscribed).not.toHaveBeenCalled();
  });

  it("renders from a prepopulated App-level store without fallback resubscribe", async () => {
    vi.useFakeTimers();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$app-store:example.invalid", "From app store")]
      }
    });
    const ensureSubscribed = vi.fn().mockResolvedValue(undefined);
    const setStore = vi.fn();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const listenCoreEvents = vi.fn((nextListener: (payload: CoreEventPayload) => void) => {
      emit = nextListener;
      return () => undefined;
    });
    const transport = baseTransport({
      listenCoreEvents,
      ensureSubscribed
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    expect(screen.getByText("From app store")).toBeTruthy();
    expect(listenCoreEvents).toHaveBeenCalledTimes(1);
    expect(ensureSubscribed).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(ensureSubscribed).not.toHaveBeenCalled();
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          MessageSourceLoaded: {
            request_id: { connection_id: 1, sequence: 1 },
            key: KEY,
            source: {
              event_id: "$source:example.invalid",
              sender: "@alice:example.invalid",
              timestamp_ms: 1_800_000_000_000,
              body: "source body",
              in_reply_to_event_id: null,
              thread_root: null,
              is_redacted: false,
              is_edited: false,
              has_media: false,
              original_json: {
                type: "m.room.message",
                content: { body: "source body", msgtype: "m.text" }
              }
            }
          }
        }
      });
    });
    expect(screen.getByText("$source:example.invalid")).toBeTruthy();
    expect(setStore).not.toHaveBeenCalled();
  });

  it("keeps hover actions out of the timestamp's flow", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
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

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$acts", "Message with actions")]
          }
        }
      });
    });

    const row = await waitFor(() => {
      const node = screen.getByText("Message with actions").closest<HTMLElement>("article");
      expect(node).not.toBeNull();
      return node!;
    });
    const actions = row.querySelector<HTMLElement>(".message-actions");
    const timestamp = row.querySelector<HTMLElement>(".message-timestamp");
    expect(actions).not.toBeNull();
    expect(timestamp).not.toBeNull();

    // The actions float over the row instead of sharing the header row with the
    // timestamp, which is what let them cover it in a narrow pane.
    expect(actions!.parentElement).toBe(row);
    expect(timestamp!.closest(".message-actions")).toBeNull();
    expect(actions!.classList.contains("message-actions-floating")).toBe(true);
  });

  it("surfaces Rust-projected reaction sender labels without profile-map repair", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        profileUsers={{
          "@ken:example.invalid": {
            user_id: "@ken:example.invalid",
            display_name: "Ken Inayoshi",
            display_label: "Ken Inayoshi",
            original_display_label: "Ken Inayoshi",
            mention_search_terms: [],
            avatar: null
          },
          "@satoshi:example.invalid": {
            user_id: "@satoshi:example.invalid",
            display_name: "Satoshi Terasaki",
            display_label: "Satoshi Terasaki",
            original_display_label: "Satoshi Terasaki",
            mention_search_terms: [],
            avatar: null
          }
        }}
        onReply={vi.fn()}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              {
                ...message("$reacted", "Reacted message"),
                reactions: [
                  {
                    key: "😢",
                    count: 2,
                    reacted_by_me: false,
                    my_reaction_event_id: null,
                    sender_preview: [
                      { user_id: "@ken:example.invalid", display_label: "Ken Alias" },
                      { user_id: "@satoshi:example.invalid", display_label: "Satoshi" }
                    ]
                  }
                ]
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("😢")).toBeTruthy();
      expect(screen.getByText("Ken Alias and Satoshi reacted with 😢")).toBeTruthy();
    });
  });

  it("falls back to the room timeline sender label for reaction previews", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
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

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              {
                ...message("$reacted-without-label", "Reacted message"),
                reactions: [
                  {
                    key: "👍",
                    count: 1,
                    reacted_by_me: false,
                    my_reaction_event_id: null,
                    sender_preview: [
                      { user_id: "@xuanzhe:example.invalid", display_label: null }
                    ]
                  }
                ]
              },
              {
                ...message("$sender-label", "Xuanzhe's message"),
                sender: "@xuanzhe:example.invalid",
                sender_label: "Xuanzhe Xia"
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Xuanzhe Xia reacted with 👍")).toBeTruthy();
    });
  });

  it("renders structured timeline notices in the active locale as plain text", async () => {
    setActiveLocaleProfile("ja", "none");
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
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

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            {
              ...message("$create", "created the room"),
              notice_i18n: {
                key: "timeline.notice.roomCreate"
              },
              message_kind: "notice"
            },
            {
              ...message("$room-name", "Unsupported event: m.room.name"),
              notice_i18n: {
                key: "timeline.notice.roomNameChanged",
                old_name: "研究室 🧪 العربية",
                new_name: "<新しい部屋>"
              },
              message_kind: "notice"
            }
          ]
        }
      }
    });

    expect(await screen.findByText("ルームを作成しました")).toBeTruthy();
    expect(
      await screen.findByText("ルーム名を「研究室 🧪 العربية」から「<新しい部屋>」に変更しました")
    ).toBeTruthy();
    expect(screen.queryByText("created the room")).toBeNull();
    expect(screen.queryByText("Unsupported event: m.room.name")).toBeNull();
    expect(document.querySelector("新しい部屋")).toBeNull();
  });

  it("keeps a canonical gap before its newer event when an earlier row is hidden", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        continuity={{
          kind: "repairing",
          generation: 3,
          gap_count: 1,
          batches_processed: 0,
          minimum_batch_id: null
        }}
      />
    );
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            actor_generation: 1,
            generation: 1,
            items: [
              message("$older:example.invalid", "Older"),
              { ...message("$hidden:example.invalid", "Hidden"), is_hidden: true },
              message("$newer:example.invalid", "Newer")
            ]
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          GapPositionsUpdated: {
            key: KEY,
            actor_generation: 1,
            generation: 3,
            positions: [
              { id: { topology_revision: "7", ordinal: 0 }, before_item_index: 2 }
            ]
          }
        }
      });
    });

    const gap = await screen.findByTestId("timeline-gap-row");
    expect(gap.parentElement?.previousElementSibling?.textContent).toContain("Older");
    expect(gap.parentElement?.nextElementSibling?.textContent).toContain("Newer");
  });

  it("shows conversation start only when the displayed window reaches the SDK start", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const { rerender } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        continuity={{ kind: "unknown" }}
      />
    );
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$latest:example.invalid", "Latest")]
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key: KEY,
            direction: "Backward",
            state: "EndReached"
          }
        }
      });
    });
    expect(screen.queryByText("Start of conversation")).toBeNull();

    // The SDK start is authoritative, but the displayed rows have not reached
    // it, so Core has not projected the timeline-start row (#1443).
    rerender(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        continuity={{ kind: "healthy", generation: 2, authoritative_start: true }}
      />
    );
    expect(screen.queryByText("Start of conversation")).toBeNull();

    // Core materialized the timeline-start row: the displayed oldest boundary
    // reaches the SDK start.
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 3,
            items: [
              {
                ...message("$start:example.invalid", ""),
                id: { Synthetic: { synthetic_id: "timeline-start" } },
                is_hidden: true
              },
              message("$latest:example.invalid", "Latest")
            ]
          }
        }
      });
    });
    expect(await screen.findByText("Start of conversation")).not.toBeNull();
  });

  it("preserves formatted HTML when adding Rust-projected link anchors", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const plainText = "fn main() {}Visit https://example.com/page";
    const item: TimelineItem = {
      ...message("$formatted-url:example.invalid", plainText),
      formatted: {
        html: "<pre><code>fn main() {}</code></pre><strong>Visit https://example.com/page</strong>",
        plain_text: plainText,
        code_blocks: [{ language: null, body: "fn main() {}" }]
      },
      link_ranges: [
        {
          url: "https://example.com/page",
          start_utf16: "fn main() {}Visit ".length,
          end_utf16: plainText.length
        }
      ]
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const link = await screen.findByRole("link", { name: "https://example.com/page" });
    expect(link.getAttribute("href")).toBe("https://example.com/page");
    expect(link.closest("strong")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Copy code" })).toBeTruthy();
  });

  it("preserves compact formatted list structure inside message bodies", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const item: TimelineItem = {
      ...message("$formatted-list:example.invalid", "Paper\nEvent and announcement\nAI\nNested"),
      formatted: {
        html: `
          <ul>
            <li>Paper</li>
            <li>Event and announcement</li>
            <li>AI
              <ol>
                <li>Nested</li>
              </ol>
            </li>
          </ul>
        `,
        plain_text: "Paper\nEvent and announcement\nAI\nNested",
        code_blocks: []
      }
    };

    const { container } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const list = await waitFor(() => {
      const next = container.querySelector("ul");
      expect(next).not.toBeNull();
      return next!;
    });
    const items = within(list).getAllByRole("listitem");
    expect(items.map((listItem) => listItem.textContent?.replace(/\s+/g, " ").trim())).toEqual([
      "Paper",
      "Event and announcement",
      "AI Nested",
      "Nested"
    ]);
    expect(container.querySelectorAll(".message-formatted-body br")).toHaveLength(0);
    for (const renderedList of container.querySelectorAll("ul, ol")) {
      expect(Array.from(renderedList.children).every((child) => child.tagName === "LI")).toBe(true);
    }
  });

  it("collapses source whitespace while preserving inline space and explicit breaks", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const item: TimelineItem = {
      ...message("$formatted-whitespace:example.invalid", "Hello world\nnext"),
      formatted: {
        html: `
          <p><strong>Hello</strong> <em>world</em><br>next</p>
        `,
        plain_text: "Hello world\nnext",
        code_blocks: []
      }
    };

    const { container } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const body = await waitFor(() => {
      const next = container.querySelector(".message-formatted-body");
      expect(next).not.toBeNull();
      return next!;
    });
    expect(body.querySelector("p")?.textContent).toBe("Hello worldnext");
    expect(body.querySelectorAll("br")).toHaveLength(1);
  });

  it("renders authored mention newlines and blank lines from explicit formatted breaks", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const item: TimelineItem = {
      ...message(
        "$formatted-mention-breaks:example.invalid",
        "@Alice\nhttps://example.invalid/pull/7\n\nFeedback from the port"
      ),
      formatted: {
        html: '<a href="https://matrix.to/#/%40alice%3Aexample.invalid">@Alice</a><br>https://example.invalid/pull/7<br><br>Feedback from the port',
        plain_text: "@Alice\nhttps://example.invalid/pull/7\n\nFeedback from the port",
        code_blocks: []
      }
    };

    const { container } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      }
    });

    const body = await waitFor(() => {
      const next = container.querySelector(".message-formatted-body");
      expect(next).not.toBeNull();
      return next!;
    });
    expect(body.querySelectorAll("br")).toHaveLength(3);
    expect(body.textContent).toBe(
      "@Alicehttps://example.invalid/pull/7Feedback from the port"
    );
  });
});
