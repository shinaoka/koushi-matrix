// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { roomTimelineKey, threadTimelineKey, type CoreEventPayload, type TimelineItem } from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { KEY, baseTransport, message, mockTimelineRects } from "./timelineViewTestSupport";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "./TimelineView";
import type { LiveSignalsState } from "../domain/types";
import type { RoomKeyRequestStateDto, RoomKeyRequestWithheldCode } from "../domain/coreEvents";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("TimelineView", () => {

  it("reports the latest visible room event without sending read state from the view", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const sendReadReceipt = vi.fn().mockResolvedValue(undefined);
    const setFullyRead = vi.fn().mockResolvedValue(undefined);
    const observeViewport = vi.fn().mockResolvedValue(undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      sendReadReceipt,
      setFullyRead,
      observeViewport
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$older:example.invalid": { top: 40, height: 80 },
        "$latest:example.invalid": { top: 140, height: 80 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 2_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 0,
        writable: true,
        configurable: true
      });

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key: KEY,
              generation: 1,
              items: [
                message("$older:example.invalid", "Older visible message"),
                message("$latest:example.invalid", "Latest visible message")
              ]
            }
          }
        });
      });

      timeline.scrollTop = 0;
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);

      await waitFor(() => {
        expect(observeViewport).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$older:example.invalid",
          "$latest:example.invalid",
          [],
          true,
          null
        );
      });
      expect(sendReadReceipt).not.toHaveBeenCalled();
      expect(setFullyRead).not.toHaveBeenCalled();
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("reports the latest visible thread event without sending read state from the view", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const sendReadReceipt = vi.fn().mockResolvedValue(undefined);
    const setFullyRead = vi.fn().mockResolvedValue(undefined);
    const observeViewport = vi.fn().mockResolvedValue(undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      sendReadReceipt,
      setFullyRead,
      observeViewport
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectSpy = mockTimelineRects(
      {
        "$thread-reply:example.invalid": { top: 140, height: 80 }
      },
      { top: 0, height: 500 },
      scrollContainerRef
    );

    try {
      render(
        <TimelineView
          timelineKey={threadKey}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      scrollContainerRef.current = timeline;
      Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
      Object.defineProperty(timeline, "scrollHeight", { value: 2_000, configurable: true });
      Object.defineProperty(timeline, "scrollTop", {
        value: 0,
        writable: true,
        configurable: true
      });

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key: threadKey,
              generation: 1,
              items: [message("$thread-reply:example.invalid", "Thread reply")]
            }
          }
        });
      });

      timeline.scrollTop = 0;
      fireEvent.wheel(timeline, { deltaY: 1 });
      fireEvent.scroll(timeline);

      await waitFor(() => {
        expect(sendReadReceipt).not.toHaveBeenCalled();
        expect(setFullyRead).not.toHaveBeenCalled();
      });
      expect(observeViewport).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$thread-reply:example.invalid",
        "$thread-reply:example.invalid",
        [],
        true,
        "$root:example.invalid"
      );
    } finally {
      rectSpy.mockRestore();
    }
  });


  it("renders pending and failed read-state status from Rust snapshots", async () => {
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

    const snapshot = {
      read_marker_event_id: "$server:example.invalid",
      read_marker_display_event_id: "$local:example.invalid",
      first_unread_event_id: null,
      unread_event_count: 0,
      unread_position: "none" as const,
      newer_event_count: 0,
      can_jump_to_bottom: false,
      local_viewed_event_id: "$local:example.invalid",
      server_confirmed_read_event_id: "$server:example.invalid"
    };
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: { ...snapshot, read_state_sync: "pending" }
          }
        }
      });
    });
    expect(
      (await screen.findByTestId("timeline-read-state-status")).textContent
    ).toContain("Syncing read state");

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              ...snapshot,
              read_state_sync: { failed: { kind: "timeout" } }
            }
          }
        }
      });
    });
    expect(
      (await screen.findByTestId("timeline-read-state-status")).textContent
    ).toContain("Read state not synced");
    expect(screen.getByTestId("timeline-read-state-status").textContent).toContain("timed out");
  });

  it("renders read receipts as a compact avatar stack without an inline text label", async () => {
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
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              typing_users: [],
              receipts_by_event: {
                "$seen": {
                  total_count: 2,
                  overflow_count: 0,
                  readers: [
                    {
                      user_id: "@ken:example.invalid",
                      display_name: "Ken Inayoshi",
                      original_display_label: "Ken Inayoshi",
                      avatar: null,
                      timestamp_ms: null
                    },
                    {
                      user_id: "@satoshi:example.invalid",
                      display_name: "Satoshi Terasaki",
                      original_display_label: "Satoshi Terasaki",
                      avatar: null,
                      timestamp_ms: null
                    }
                  ]
                }
              }
            }
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
            items: [message("$seen", "Seen message")]
          }
        }
      });
    });

    await waitFor(() => {
      const receipts = document.querySelector(".message-receipts");
      expect(receipts).not.toBeNull();
      expect(receipts?.textContent).toContain("KE");
      expect(receipts?.textContent).toContain("SA");
      expect(receipts?.textContent).not.toContain("Read by 2");
      expect(receipts?.getAttribute("aria-label")).toContain("Read by 2");
      expect(receipts?.getAttribute("title")).toBe("Ken Inayoshi\nSatoshi Terasaki");
    });
  });


  it("opens the reader popup in the floating layer so a clipped pane cannot cut it", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    // A narrow, overflow-clipped container stands in for the thread pane.
    const pane = document.createElement("div");
    pane.className = "thread-pane";
    pane.style.overflow = "hidden";
    pane.style.width = "320px";
    document.body.appendChild(pane);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        presentationContext="thread"
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              typing_users: [],
              receipts_by_event: {
                "$seen": {
                  total_count: 1,
                  overflow_count: 0,
                  readers: [
                    {
                      user_id: "@ken:example.invalid",
                      display_name: "Ken Inayoshi",
                      original_display_label: "Ken Inayoshi",
                      avatar: null,
                      timestamp_ms: 1_800_000_000_000
                    }
                  ]
                }
              }
            }
          }
        }}
        onReply={vi.fn()}
      />,
      { container: pane }
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [message("$seen", "Seen message")]
          }
        }
      });
    });

    const receipts = await waitFor(() => {
      const node = pane.querySelector<HTMLElement>(".message-receipts");
      expect(node).not.toBeNull();
      return node!;
    });

    // Closed by default; the details are not a row-local always-rendered child.
    expect(document.querySelector('[role="tooltip"]')).toBeNull();

    fireEvent.focus(receipts);
    const tooltip = await waitFor(() => {
      const node = document.querySelector<HTMLElement>('[role="tooltip"]');
      expect(node).not.toBeNull();
      return node!;
    });
    expect(tooltip.textContent).toContain("Ken Inayoshi");
    // The popup must escape the clipped pane, so it cannot be a descendant.
    expect(pane.contains(tooltip)).toBe(false);
    expect(tooltip.parentElement).toBe(document.body);

    fireEvent.blur(receipts);
    await waitFor(() => {
      expect(document.querySelector('[role="tooltip"]')).toBeNull();
    });

    pane.remove();
  });


  it("places reactions and read receipts in one status row", async () => {
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
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              typing_users: [],
              receipts_by_event: {
                "$reacted-seen": {
                  total_count: 1,
                  overflow_count: 0,
                  readers: [
                    {
                      user_id: "@ken:example.invalid",
                      display_name: "Ken Inayoshi",
                      original_display_label: "Ken Inayoshi",
                      avatar: null,
                      timestamp_ms: null
                    }
                  ]
                }
              }
            }
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
                ...message("$reacted-seen", "Reacted and seen"),
                reactions: [
                  {
                    key: "✈️",
                    count: 1,
                    reacted_by_me: false,
                    my_reaction_event_id: null,
                    sender_preview: [
                      { user_id: "@ken:example.invalid", display_label: "Ken Alias" }
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
      const reactions = document.querySelector(".message-reactions");
      const receipts = document.querySelector(".message-receipts");
      const statusRow = document.querySelector(".message-status-row");

      expect(reactions).not.toBeNull();
      expect(receipts).not.toBeNull();
      expect(statusRow).not.toBeNull();
      expect(reactions?.parentElement).toBe(statusRow);
      expect(receipts?.parentElement).toBe(statusRow);
      expect(Array.from(statusRow?.children ?? [])).toEqual([reactions, receipts]);
    });
  });


  it("hides the first-unread pill while keeping the unread marker and bottom pill", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const items = Array.from({ length: 650 }, (_, index) =>
      message(`$virtual-${index}:example.invalid`, `Virtual message ${index}`)
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 650 * 72, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              can_jump_to_bottom: true,
              first_unread_event_id: "$virtual-5:example.invalid",
              newer_event_count: 2,
              read_marker_display_event_id: "$virtual-2:example.invalid",
              read_marker_event_id: "$virtual-2:example.invalid",
              local_viewed_event_id: "$virtual-2:example.invalid",
              server_confirmed_read_event_id: "$virtual-2:example.invalid",
              read_state_sync: "synced",
              unread_event_count: 3,
              unread_position: "belowViewport"
            }
          }
        }
      });
    });

    timeline.scrollTop = 0;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(timeline.getAttribute("data-virtualized")).toBe("true");
      expect(screen.queryByRole("button", { name: /Jump to first unread/ })).toBeNull();
      expect(screen.getByRole("button", { name: /Jump to bottom/ })).toBeTruthy();
      expect(screen.getByRole("separator", { name: "Unread messages" })).toBeTruthy();
    });
  });


  it("lets users request missing room keys from undecryptable events", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const requestRoomKey = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey
    });
    const encrypted = {
      ...message("$encrypted", "Unable to decrypt message"),
      unable_to_decrypt: {
        session_id: "session-1",
        reason: "missingRoomKey" as const,
        can_request_keys: true,
        recovery_stage: null,
        recovery_guidance: null
      }
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
          items: [encrypted]
        }
      }
    });

    const button = await screen.findByRole("button", { name: "Request keys and retry" });
    fireEvent.click(button);

    expect(requestRoomKey).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$encrypted",
      "user",
      KEY
    );
  });


  it("renders Rust-owned automatic request state without dispatching automatic commands", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const requestRoomKey = vi.fn(async () => undefined);
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey
    });
    const encrypted = {
      ...message("$encrypted-thread-reply:example.invalid", "Unable to decrypt message"),
      thread_root: "$thread-root:example.invalid",
      unable_to_decrypt: {
        session_id: "session-1",
        reason: "missingRoomKey" as const,
        can_request_keys: true,
        recovery_stage: null,
        recovery_guidance: null
      },
      request_state: { stage: "automatic", withheldCode: null } as const
    };

    render(
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        presentationContext="thread"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: threadKey,
          generation: 1,
          items: [encrypted]
        }
      }
    });

    // Automatic admission is Rust-owned: the frontend dispatches nothing and
    // only renders the Rust-published request state (awaiting copy).
    await waitFor(() => {
      expect(requestRoomKey).not.toHaveBeenCalled();
    });
    expect(screen.queryByText("Waiting for the decryption key…")).toBeTruthy();
  });


  it("does not classify room-key request failures in React", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const privateEventId = "$private-event:example.invalid";
    const privateBody = "secret message body";
    const rawError = [
      "raw SDK error",
      "/Users/member/private/store",
      "https://private.example.invalid/room",
      "access_token=private-token"
    ].join(" ");
    const requestRoomKey = vi.fn(async () => {
      throw new Error(rawError);
    });
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey
    });
    const encrypted = {
      ...message(privateEventId, privateBody),
      unable_to_decrypt: {
        session_id: "private-session-id",
        reason: "missingRoomKey" as const,
        can_request_keys: true,
        recovery_stage: null,
        recovery_guidance: null
      }
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [encrypted]
        }
      }
    });

    fireEvent.click(await screen.findByRole("button", { name: "Request keys and retry" }));

    await waitFor(() => expect(requestRoomKey).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
    const diagnosticText = JSON.stringify(onDiagnosticLogEntry.mock.calls);
    expect(diagnosticText).not.toContain("operation=request_keys stage=failed kind=transport");

    for (const privateValue of [
      "!room:example.invalid",
      privateEventId,
      privateBody,
      "private-session-id",
      rawError,
      "/Users/member/private/store",
      "private.example.invalid",
      "private-token"
    ]) {
      expect(diagnosticText).not.toContain(privateValue);
    }
  });


  it("renders the read marker after the Rust-derived display anchor for own messages after the marker", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const ownMessage = (eventId: string): TimelineItem => ({
      ...message(eventId, "own"),
      sender: "@alice:example.invalid"
    });
    const other = message("$other:example.invalid", "hello");
    const own1 = ownMessage("$own1:example.invalid");
    const own2 = ownMessage("$own2:example.invalid");

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
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            read_marker_event_id: "$other:example.invalid",
            read_marker_display_event_id: "$own2:example.invalid",
            first_unread_event_id: null,
            local_viewed_event_id: "$own2:example.invalid",
            server_confirmed_read_event_id: "$other:example.invalid",
            read_state_sync: "synced",
            unread_event_count: 0,
            unread_position: "none",
            newer_event_count: 0,
            can_jump_to_bottom: false
          }
        }
      }
    });
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [other, own1, own2]
        }
      }
    });

    const marker = await screen.findByRole("separator", { name: "Read up to here" });
    expect(marker.previousElementSibling?.getAttribute("data-event-id")).toBe(
      "$own2:example.invalid"
    );
  });


  it("renders the read marker after the current user's latest own message when the marker starts on an own message", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const ownMessage = (eventId: string): TimelineItem => ({
      ...message(eventId, "own"),
      sender: "@alice:example.invalid"
    });
    const own1 = ownMessage("$own1:example.invalid");
    const own2 = ownMessage("$own2:example.invalid");

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
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            read_marker_event_id: "$own1:example.invalid",
            read_marker_display_event_id: "$own2:example.invalid",
            first_unread_event_id: null,
            local_viewed_event_id: "$own2:example.invalid",
            server_confirmed_read_event_id: "$own1:example.invalid",
            read_state_sync: "synced",
            unread_event_count: 0,
            unread_position: "none",
            newer_event_count: 0,
            can_jump_to_bottom: false
          }
        }
      }
    });
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [own1, own2]
        }
      }
    });

    const marker = await screen.findByRole("separator", { name: "Read up to here" });
    expect(marker.previousElementSibling?.getAttribute("data-event-id")).toBe(
      "$own2:example.invalid"
    );
  });


  it("renders the unread marker before the first unread event", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const other = message("$other:example.invalid", "hello");
    const unread = message("$unread:example.invalid", "new message");
    const own1 = { ...message("$own1:example.invalid", "own"), sender: "@alice:example.invalid" };

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
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            read_marker_event_id: "$other:example.invalid",
            read_marker_display_event_id: null,
            first_unread_event_id: "$unread:example.invalid",
            local_viewed_event_id: null,
            server_confirmed_read_event_id: "$other:example.invalid",
            read_state_sync: "pending",
            unread_event_count: 1,
            unread_position: "insideViewport",
            newer_event_count: 0,
            can_jump_to_bottom: false
          }
        }
      }
    });
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [other, unread, own1]
        }
      }
    });

    const marker = await screen.findByRole("separator", { name: "Unread messages" });
    expect(marker.nextElementSibling?.getAttribute("data-event-id")).toBe(
      "$unread:example.invalid"
    );
  });


  it("keeps reactions and read receipts in one footer status row", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const item: TimelineItem = {
      ...message("$reacted:example.invalid", "hello"),
      reactions: [
        {
          key: "👍",
          count: 1,
          reacted_by_me: false,
          my_reaction_event_id: null,
          sender_preview: [{ user_id: "@bob:example.invalid", display_label: "Bob" }]
        }
      ],
      can_react: true
    };
    const liveSignals: LiveSignalsState = {
      rooms: {
        "!room:example.invalid": {
          receipts_by_event: {
            "$reacted:example.invalid": {
              readers: [
                {
                  user_id: "@bob:example.invalid",
                  display_name: "Bob",
                  original_display_label: "Bob",
                  avatar: null,
                  timestamp_ms: 1_800_000_000_000
                }
              ],
              total_count: 1,
              overflow_count: 0
            }
          },
          fully_read_event_id: null,
          typing_user_ids: [],
          typing_users: []
        }
      },
      presence: {}
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        liveSignals={liveSignals}
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

    const statusRow = await waitFor(() => {
      const row = document.querySelector(".message-status-row");
      if (!row) {
        throw new Error("message-status-row not found");
      }
      return row;
    });
    expect(statusRow.querySelector(".message-reactions")).toBeTruthy();
    expect(statusRow.querySelector(".message-receipts")).toBeTruthy();
  });


  it("renders typing indicators with room display labels when available", async () => {
    const transport = baseTransport({});
    const liveSignals: LiveSignalsState = {
      rooms: {
        "!room:example.invalid": {
          receipts_by_event: {},
          fully_read_event_id: null,
          typing_user_ids: ["@hironeishida:matrix.org"],
          typing_users: [
            {
              user_id: "@hironeishida:matrix.org",
              display_label: "Hirone Ishida"
            }
          ]
        }
      },
      presence: {}
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        liveSignals={liveSignals}
        profileUsers={{
          "@hironeishida:matrix.org": {
            user_id: "@hironeishida:matrix.org",
            display_name: "Hirone Ishida",
            display_label: "Hirone Ishida",
            original_display_label: "Hirone Ishida",
            mention_search_terms: [],
            avatar: null
          }
        }}
      />
    );

    expect(screen.getByText("Hirone Ishida is typing")).toBeTruthy();
    expect(screen.queryByText("@hironeishida:matrix.org is typing")).toBeNull();
  });


  it("uses a friendly fallback for typing indicators without a projected label", async () => {
    const transport = baseTransport({});
    const liveSignals: LiveSignalsState = {
      rooms: {
        "!room:example.invalid": {
          receipts_by_event: {},
          fully_read_event_id: null,
          typing_user_ids: ["@unknown:example.invalid"],
          typing_users: [
            {
              user_id: "@unknown:example.invalid",
              display_label: null
            }
          ]
        }
      },
      presence: {}
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        liveSignals={liveSignals}
      />
    );

    expect(screen.getByText("Unknown user is typing")).toBeTruthy();
    expect(screen.queryByText("@unknown:example.invalid is typing")).toBeNull();
  });

});

describe("room key request feedback (#460)", () => {
  function utdItem(eventId: string, requestState: RoomKeyRequestStateDto | null) {
    return {
      ...message(eventId, "Unable to decrypt message"),
      unable_to_decrypt: {
        session_id: "session-1",
        reason: "missingRoomKey" as const,
        can_request_keys: true,
        recovery_stage: null,
        recovery_guidance: null
      },
      request_state: requestState
    };
  }

  function renderWithItems(items: TimelineItem[]) {
    let emit: (payload: unknown) => void = () => undefined;
    const transport = {
      listenCoreEvents(nextListener: (p: unknown) => void) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey: vi.fn(async () => undefined),
      ensureSubscribed: vi.fn(async () => undefined)
    } as never;
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
            items
          }
        }
      });
    });
    return { transport };
  }

  it("renders localized copy for each closed withheld code", () => {
    const cases: Array<[RoomKeyRequestWithheldCode | null, RegExp]> = [
      ["unavailable", /The requested device does not have this decryption key/],
      ["unauthorised", /Sharing the decryption key was not permitted/],
      ["unverified", /This device is unverified, so the key was not shared/],
      ["blacklisted", /This device is excluded from key sharing/],
      [null, /The decryption key could not be obtained/]
    ];
    for (const [code, expected] of cases) {
      renderWithItems([utdItem("$w", { stage: "withheld", withheldCode: code })]);
      expect(screen.queryByText(expected)).toBeTruthy();
      cleanup();
    }
  });

  it("still_waiting shows non-terminal guidance and never a raw reason", () => {
    renderWithItems([utdItem("$s", { stage: "still_waiting", withheldCode: null })]);
    expect(
      screen.queryByText(/No response yet. Another device may be offline/)
    ).toBeTruthy();
    expect(screen.queryByText(/m\.unauthorised|refused|denied/i)).toBeNull();
  });

  it("send_failed shows the generic refusal copy instead of nothing", () => {
    renderWithItems([utdItem("$f", { stage: "send_failed", withheldCode: null })]);
    expect(
      screen.queryByText("The decryption key could not be obtained.")
    ).toBeTruthy();
    expect(screen.queryByText(/Waiting for the decryption key/)).toBeNull();
  });

  it("decryption_recovered shows success and clears the pending marker", () => {
    renderWithItems([utdItem("$r", { stage: "decryption_recovered", withheldCode: null })]);
    expect(screen.queryByText("Decryption key received")).toBeTruthy();
    expect(screen.queryByText(/Waiting for the decryption key/)).toBeNull();
  });

  it("clicking Request keys shows an immediate toast and pending copy; repeat clicks are suppressed while pending", async () => {
    let emit: (payload: unknown) => void = () => undefined;
    const requestRoomKey = vi.fn(async () => undefined);
    const transport = {
      listenCoreEvents(nextListener: (p: unknown) => void) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey,
      ensureSubscribed: vi.fn(async () => undefined)
    } as never;
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
            items: [utdItem("$click", null)]
          }
        }
      });
    });
    const button = await screen.findByRole("button", { name: "Request keys and retry" });
    // Click twice: the toast/pending marker must not duplicate, and a repeat
    // click while the request is pending dispatches no duplicate command
    // (plan: no duplicate commands while pending; Rust also coalesces).
    fireEvent.click(button);
    fireEvent.click(button);
    await waitFor(() => {
      expect(document.body.textContent).toContain("Decryption key requested");
    });
    await waitFor(() => {
      expect(document.body.textContent).toContain("Waiting for the decryption key");
    });
    expect(screen.getAllByText(/Decryption key requested/)).toHaveLength(1);
    expect(screen.getAllByText(/Waiting for the decryption key/)).toHaveLength(1);
    expect(requestRoomKey).toHaveBeenCalledTimes(1);
    expect(requestRoomKey).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$click",
      "user",
      KEY
    );
    // Suppression persists through the Rust-published pending (sent) stage:
    // a further click re-shows the toast but dispatches no new command.
    act(() => {
      emit({
        kind: "Room",
        event: {
          RoomKeyRequestStateChanged: {
            key: {
              account_key: "@alice:example.invalid",
              kind: { Room: { room_id: "!room:example.invalid" } }
            },
            event_id: "$click",
            request_id: null,
            stage: "sent",
            withheld_code: null
          }
        }
      });
    });
    await waitFor(() => {
      expect(document.body.textContent).toContain("Waiting for the decryption key");
    });
    fireEvent.click(button);
    expect(requestRoomKey).toHaveBeenCalledTimes(1);
  });

  it("keyboard activation requests keys and announces the toast in an ARIA-live status region", async () => {
    let emit: (payload: unknown) => void = () => undefined;
    const requestRoomKey = vi.fn(async () => undefined);
    const transport = {
      listenCoreEvents(nextListener: (p: unknown) => void) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey,
      ensureSubscribed: vi.fn(async () => undefined)
    } as never;
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
            items: [utdItem("$kb", null)]
          }
        }
      });
    });
    const button = await screen.findByRole("button", { name: "Request keys and retry" });
    // The action is a native <button> (browser-activated by Enter/Space);
    // jsdom does not synthesize the Enter->click translation, so activate it
    // while focused and assert the IPC payload + ARIA-live announcement.
    expect(button.tagName).toBe("BUTTON");
    button.focus();
    expect(document.activeElement).toBe(button);
    fireEvent.click(button);
    await waitFor(() => {
      expect(requestRoomKey).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$kb",
        "user",
        KEY
      );
    });
    const status = screen.getByRole("status");
    expect(status.getAttribute("aria-live")).toBe("polite");
    expect(status.textContent).toContain("Decryption key requested");
  });

  it("a Rust-published transition clears the local pending marker and shows the terminal copy", async () => {
    let emit: (payload: unknown) => void = () => undefined;
    const transport = {
      listenCoreEvents(nextListener: (p: unknown) => void) {
        emit = nextListener;
        return () => undefined;
      },
      requestRoomKey: vi.fn(async () => undefined),
      ensureSubscribed: vi.fn(async () => undefined)
    } as never;
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
            items: [utdItem("$click", null)]
          }
        }
      });
    });
    const button = await screen.findByRole("button", { name: "Request keys and retry" });
    fireEvent.click(button);
    await waitFor(() => {
      expect(document.body.textContent).toContain("Waiting for the decryption key");
    });
    // Rust settles the request as refused (withheld) via the typed event.
    act(() => {
      emit({
        kind: "Room",
        event: {
          RoomKeyRequestStateChanged: {
            key: {
              account_key: "@alice:example.invalid",
              kind: { Room: { room_id: "!room:example.invalid" } }
            },
            event_id: "$click",
            request_id: null,
            stage: "withheld",
            withheld_code: "unavailable"
          }
        }
      });
    });
    await waitFor(() => {
      expect(
        screen.queryByText(/The requested device does not have this decryption key/)
      ).toBeTruthy();
    });
    // The local pending marker is gone once the terminal state is rendered.
    expect(screen.queryByText(/Waiting for the decryption key/)).toBeNull();
  });

  it("a delayed rejection from an earlier visit does not clear the current pending marker (A->B->A)", async () => {
    let emitA: (payload: unknown) => void = () => undefined;
    let rejectFirst: (reason?: unknown) => void = () => undefined;
    let rejectSecond: (reason?: unknown) => void = () => undefined;
    const requestRoomKey = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectFirst = reject;
          })
      )
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectSecond = reject;
          })
      );
    const keyB = roomTimelineKey("@bob:example.invalid", "!room:example.invalid");
    const transport = {
      listenCoreEvents(nextListener: (p: unknown) => void) {
        emitA = nextListener;
        return () => undefined;
      },
      requestRoomKey,
      ensureSubscribed: vi.fn(async () => undefined)
    } as never;
    const { rerender } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const seed = (key: typeof KEY) =>
      act(() => {
        emitA({
          kind: "Timeline",
          event: {
            InitialItems: {
              request_id: null,
              key,
              generation: 1,
              items: [utdItem("$click", null)]
            }
          }
        });
      });
    seed(KEY);
    const button = await screen.findByRole("button", { name: "Request keys and retry" });
    fireEvent.click(button); // visit A, epoch 1
    await waitFor(() => {
      expect(document.body.textContent).toContain("Waiting for the decryption key");
    });
    // Navigate A -> B -> A (each switch bumps the view epoch).
    rerender(
      <TimelineView
        timelineKey={keyB}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    seed(keyB);
    rerender(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    seed(KEY);
    // New click in the final A visit (epoch 3) — new marker + request.
    const buttonAgain = await screen.findByRole("button", { name: "Request keys and retry" });
    fireEvent.click(buttonAgain);
    await waitFor(() => {
      expect(requestRoomKey).toHaveBeenCalledTimes(2);
    });
    // The FIRST visit's request rejects late: it must not clear the new marker.
    act(() => {
      rejectFirst(new Error("stale rejection"));
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText(/Waiting for the decryption key/)).toBeTruthy();
    // The CURRENT visit's own rejection still clears its marker.
    act(() => {
      rejectSecond(new Error("current rejection"));
    });
    await waitFor(() => {
      expect(screen.queryByText(/Waiting for the decryption key/)).toBeNull();
    });
  });
});
