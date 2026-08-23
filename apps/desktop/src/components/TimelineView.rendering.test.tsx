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
      const row = document.querySelector<HTMLElement>(
        '[data-event-id="$virtual-8:example.invalid"]'
      );
      expect(row).not.toBeNull();
      expect(row?.classList.contains("is-continuation")).toBe(true);
    });
    expect(timeline.getAttribute("data-total-items")).toBe("601");
    expect(screen.getByText("Virtual 8")).toBeTruthy();
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

  it("shows conversation start only with Rust-owned authoritative continuity", async () => {
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

    rerender(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        continuity={{ kind: "healthy", generation: 2, authoritative_start: true }}
      />
    );
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
