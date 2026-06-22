// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  roomTimelineKey,
  threadTimelineKey,
  type CoreEventPayload,
  type TimelineItem
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import { TimelineView, type TimelineTransport } from "./TimelineView";

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
});

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
    downloadAvatarThumbnail: async () => undefined,
    loadMessageSource: async () => undefined,
    requestRoomKey: async () => undefined,
    forwardMessage: async () => undefined,
    loadLinkPreviews: async () => undefined,
    hideLinkPreview: async () => undefined,
    updateScrollAnchor: async () => undefined,
    ...overrides
  };
}

function mockTimelineRects(
  rects: Record<string, { top: number; height: number }>,
  container: { top?: number; height?: number } = {}
) {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const eventId = this.getAttribute("data-event-id");
    const testId = this.getAttribute("data-testid");
    const top = testId === "timeline-view" ? container.top ?? 0 : rects[eventId ?? ""]?.top ?? 0;
    const height =
      testId === "timeline-view"
        ? container.height ?? 600
        : rects[eventId ?? ""]?.height ?? 0;
    const bottom = top + height;
    return {
      x: 0,
      y: top,
      top,
      left: 0,
      right: 0,
      width: 0,
      height,
      bottom,
      toJSON: () => ({})
    } as DOMRect;
  });
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

  it("emits safe timestamped timeline event diagnostics for thread timelines", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onDiagnosticLogEntry = vi.fn();
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={threadKey}
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
          key: threadKey,
          generation: 3,
          items: [message("$root:example.invalid", "Thread root")]
        }
      }
    });
    emit({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key: threadKey,
          direction: "Backward",
          state: "EndReached"
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.event",
          message: "kind=thread initial items=1 generation=3"
        })
      );
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.event",
          message: "kind=thread pagination direction=Backward state=EndReached"
        })
      );
    });
    expect(onDiagnosticLogEntry.mock.calls.map(([entry]) => entry.message).join("\n")).not.toContain(
      "$root"
    );
  });

  it("paginates older history when the user scrolls to the top even if prefetch is disabled", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages={false}
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
          items: [message("$latest", "Latest")]
        }
      }
    });

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", { value: 0, configurable: true });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(KEY);
    });
  });

  it("throttles room scroll anchor captures to once per second per room", async () => {
    vi.useFakeTimers();
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });

    mockTimelineRects({
      "$first:example.invalid": { top: 120, height: 48 },
      "$second:example.invalid": { top: 420, height: 48 }
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
    });

    const timeline = screen.getByTestId("timeline-view");

    act(() => {
      fireEvent.scroll(timeline);
    });

    act(() => {
      vi.advanceTimersByTime(500);
      fireEvent.scroll(timeline);
    });

    act(() => {
      vi.advanceTimersByTime(499);
    });
    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(updateScrollAnchor).toHaveBeenCalledTimes(2);
    expect(updateScrollAnchor).toHaveBeenCalledWith(
      "!room:example.invalid",
      expect.objectContaining({
        event_id: "$first:example.invalid",
        offset_px: 120,
        updated_at_ms: expect.any(Number)
      })
    );
  });

  it("restores a persisted room anchor when the event is already rendered", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    mockTimelineRects(
      {
        "$anchor:example.invalid": { top: 500, height: 48 },
        "$after:example.invalid": { top: 560, height: 48 }
      },
      { top: 0, height: 600 }
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        roomScrollAnchor={{
          event_id: "$anchor:example.invalid",
          offset_px: 50,
          updated_at_ms: Date.now()
        }}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [
            message("$first:example.invalid", "First"),
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(450);
    });
  });

  it("requests a live anchor restore once and restores when the anchor enters live items", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const roomId = "!room:example.invalid";
    const anchorEventId = "$anchor:example.invalid";
    const restoreTimelineAnchor = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      restoreTimelineAnchor
    });

    mockTimelineRects(
      {
        "$live-top:example.invalid": { top: 120, height: 48 },
        "$live-bottom:example.invalid": { top: 560, height: 48 },
        [anchorEventId]: { top: 500, height: 48 }
      },
      { top: 0, height: 600 }
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId={roomId}
        transport={transport}
        roomScrollAnchor={{
          event_id: anchorEventId,
          offset_px: 50,
          updated_at_ms: Date.now()
        }}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$live-top:example.invalid", "Live top"),
              message("$live-bottom:example.invalid", "Live bottom")
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Live top")).toBeTruthy();
      expect(timeline.getAttribute("data-timeline-generation")).toBe("1");
      expect(restoreTimelineAnchor).toHaveBeenCalledTimes(1);
      expect(restoreTimelineAnchor).toHaveBeenCalledWith(
        KEY,
        anchorEventId,
        expect.any(Number),
        expect.any(Number)
      );
    });

    Object.defineProperty(timeline, "scrollTop", {
      value: 550,
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
            generation: 2,
            items: [
              message("$live-top:example.invalid", "Live top"),
              message(anchorEventId, "Live anchor visible"),
              message("$live-bottom:example.invalid", "Live bottom")
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.queryByText("Bootstrap anchor")).toBeNull();
      expect(screen.queryByText("Focused bootstrap context")).toBeNull();
      expect(screen.getByText("Live anchor visible")).toBeTruthy();
      expect(timeline.getAttribute("data-timeline-generation")).toBe("2");
      expect(timeline.scrollTop).toBe(1000);
    });
  });

  it("falls back to the live edge when live anchor restore exhausts its budget", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const roomId = "!room:example.invalid";
    const anchorEventId = "$anchor:example.invalid";
    const restoreTimelineAnchor = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      restoreTimelineAnchor
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId={roomId}
        transport={transport}
        roomScrollAnchor={{
          event_id: anchorEventId,
          offset_px: 50,
          updated_at_ms: Date.now()
        }}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
              message("$live-top:example.invalid", "Live top"),
              message("$live-bottom:example.invalid", "Live bottom")
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(restoreTimelineAnchor).toHaveBeenCalledTimes(1);
      expect(timeline.scrollTop).toBe(0);
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          AnchorRestoreFinished: {
            request_id: { connection_id: 1, sequence: 99 },
            key: KEY,
            status: "BudgetExhausted"
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(2000);
    });
    expect(restoreTimelineAnchor).toHaveBeenCalledTimes(1);
  });

  it("restores the live edge after a same-key timeline resync generation arrives", async () => {
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

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
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
            items: [message("$first", "First generation")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(2000);
    });

    timeline.scrollTop = 100;

    act(() => {
      emit({ kind: "ResyncMarker" });
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 2,
            items: [message("$second", "Second generation")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Second generation")).toBeTruthy();
      expect(timeline.scrollTop).toBe(2000);
    });
  });

  it("requests visible sender avatar thumbnails that are not yet downloaded", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
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
              ...message("$avatar", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/avatar");
    });
    expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(1);
  });

  it("emits timestamped avatar diagnostics for request, success, and retryable failure", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
        enableAvatarThumbnailDownloads={true}
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
              ...message("$avatar-retry", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-retry",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/avatar-retry");
    });
    expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail request queued"
      })
    );

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 3 },
          mxc_uri: "mxc://matrix.org/avatar-retry",
          thumbnail: {
            kind: "failed",
            request_id: 3,
            failureKind: "network"
          }
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(2);
    });
    expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail failed kind=network"
      })
    );

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 4 },
          mxc_uri: "mxc://matrix.org/avatar-retry",
          thumbnail: {
            kind: "ready",
            source_url: "file:///tmp/avatar-retry.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.avatar",
          message: "avatar thumbnail ready"
        })
      );
    });
    expect(onDiagnosticLogEntry.mock.calls.every(([entry]) => Number.isFinite(entry.timestampMs)))
      .toBe(true);
  });

  it("requests profile avatar thumbnails when the timeline item has no sender avatar", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        profileUsers={{
          "@bob:example.invalid": {
            user_id: "@bob:example.invalid",
            display_name: "Bob",
            display_label: "Bob",
            original_display_label: "Bob",
            mention_search_terms: ["bob"],
            avatar: {
              mxc_uri: "mxc://matrix.org/profile-avatar",
              thumbnail: { kind: "notRequested" }
            }
          }
        }}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={true}
      />
    );

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [message("$profile-avatar", "Profile avatar row")]
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith("mxc://matrix.org/profile-avatar");
    });
    expect(downloadAvatarThumbnail).toHaveBeenCalledTimes(1);
  });

  it("does NOT call downloadAvatarThumbnail when enableAvatarThumbnailDownloads is explicitly false (kill-switch)", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });

    // Explicitly disable via the kill-switch prop (#116 Stage F1a: default is now ON).
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        enableAvatarThumbnailDownloads={false}
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
              ...message("$avatar-gated", "Avatar row (kill-switch off)"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-gated",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });

    // Give React time to flush any effects that might fire.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(downloadAvatarThumbnail).not.toHaveBeenCalled();
  });

  it("renders a downloaded sender avatar thumbnail from account events", async () => {
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
              ...message("$avatar-ready", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/avatar",
          thumbnail: {
            kind: "ready",
            source_url: "file:///tmp/avatar.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      const image = document.querySelector<HTMLImageElement>(".message .avatar img");
      expect(image?.getAttribute("src")).toBe("file:///tmp/avatar.bin");
    });
  });

  it("ignores avatar thumbnail events that are not relevant to the mounted timeline", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onDiagnosticLogEntry = vi.fn();
    const onDiagnosticsChange = vi.fn();
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
        onDiagnosticsChange={onDiagnosticsChange}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
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
              ...message("$avatar-relevant", "Avatar row"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/relevant-avatar",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    await waitFor(() =>
      expect(onDiagnosticsChange).toHaveBeenCalledWith(
        expect.objectContaining({
          avatarMxcItems: 1,
          avatarPendingItems: 1,
          visibleItems: 1
        })
      )
    );
    onDiagnosticLogEntry.mockClear();
    onDiagnosticsChange.mockClear();

    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/unrelated-avatar",
          thumbnail: {
            kind: "ready",
            source_url: "file:///tmp/unrelated-avatar.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(onDiagnosticLogEntry).not.toHaveBeenCalledWith(
      expect.objectContaining({
        source: "timeline.avatar",
        message: "avatar thumbnail ready"
      })
    );
    expect(onDiagnosticsChange).not.toHaveBeenCalled();
    expect(document.querySelector(".message .avatar img")).toBeNull();
  });

  it("renders downloaded thumbnails for multiple visible sender avatars", async () => {
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
              ...message("$avatar-ready-a", "Avatar row A"),
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-a",
                thumbnail: { kind: "notRequested" }
              }
            },
            {
              ...message("$avatar-ready-b", "Avatar row B"),
              sender: "@carol:example.invalid",
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-b",
                thumbnail: { kind: "notRequested" }
              }
            }
          ]
        }
      }
    });
    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 2 },
          mxc_uri: "mxc://matrix.org/avatar-a",
          thumbnail: {
            kind: "ready",
            source_url: "file:///tmp/avatar-a.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });
    emit({
      kind: "Account",
      event: {
        AvatarThumbnailDownloaded: {
          request_id: { connection_id: 1, sequence: 3 },
          mxc_uri: "mxc://matrix.org/avatar-b",
          thumbnail: {
            kind: "ready",
            source_url: "file:///tmp/avatar-b.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      const firstImage = document.querySelector<HTMLImageElement>(
        '[data-event-id="$avatar-ready-a"] .avatar img'
      );
      const secondImage = document.querySelector<HTMLImageElement>(
        '[data-event-id="$avatar-ready-b"] .avatar img'
      );
      expect(firstImage?.getAttribute("src")).toBe("file:///tmp/avatar-a.bin");
      expect(secondImage?.getAttribute("src")).toBe("file:///tmp/avatar-b.bin");
    });
  });

  it("falls back to sender initials when a downloaded sender avatar image is broken", async () => {
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
              ...message("$avatar-broken", "Avatar row"),
              sender_label: "Ken Inayoshi",
              sender_avatar: {
                mxc_uri: "mxc://matrix.org/avatar-broken",
                thumbnail: {
                  kind: "ready",
                  source_url: "asset://missing-avatar.bin",
                  width: null,
                  height: null,
                  mime_type: null
                }
              }
            }
          ]
        }
      }
    });

    const image = await waitFor(() => {
      const element = document.querySelector<HTMLImageElement>(".message .avatar img");
      expect(element?.getAttribute("src")).toBe("asset://missing-avatar.bin");
      return element!;
    });
    fireEvent.error(image);

    expect(document.querySelector(".message .avatar img")).toBeNull();
    expect(document.querySelector(".message .avatar")?.textContent).toBe("KE");
  });

  it("retries a transiently broken sender avatar image URL", async () => {
    vi.useFakeTimers();
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
                ...message("$avatar-retry-render", "Avatar row"),
                sender_label: "Ken Inayoshi",
                sender_avatar: {
                  mxc_uri: "mxc://matrix.org/avatar-retry-render",
                  thumbnail: {
                    kind: "ready",
                    source_url: "asset://transient-avatar.bin",
                    width: null,
                    height: null,
                    mime_type: null
                  }
                }
              }
            ]
          }
        }
      });
    });

    const image = document.querySelector<HTMLImageElement>(".message .avatar img");
    expect(image).not.toBeNull();
    expect(image?.getAttribute("src")).toBe("asset://transient-avatar.bin");
    fireEvent.error(image!);
    expect(document.querySelector(".message .avatar img")).toBeNull();

    act(() => {
      vi.advanceTimersByTime(10_000);
    });

    expect(document.querySelector<HTMLImageElement>(".message .avatar img")?.getAttribute("src")).toBe(
      "asset://transient-avatar.bin"
    );
  });

  it("jumps to an unread event outside the mounted virtual window", async () => {
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    try {
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
                can_jump_to_bottom: false,
                first_unread_event_id: "$virtual-500:example.invalid",
                newer_event_count: 0,
                read_marker_event_id: null,
                unread_event_count: 3,
                unread_position: "belowViewport"
              }
            }
          }
        });
      });

      await waitFor(() => {
        expect(timeline.getAttribute("data-virtualized")).toBe("true");
        expect(screen.getByRole("button", { name: /Jump to first unread/ })).toBeTruthy();
        expect(document.querySelector('[data-event-id="$virtual-500:example.invalid"]')).toBeNull();
      });

      fireEvent.click(screen.getByRole("button", { name: /Jump to first unread/ }));

      expect(timeline.scrollTop).toBeGreaterThan(30_000);
      await waitFor(() => expect(scrollIntoView).toHaveBeenCalled());
    } finally {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    }
  });

  it("backfills an empty thread timeline even when the first Core generation is zero", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={threadKey}
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
          key: threadKey,
          generation: 0,
          items: []
        }
      }
    });

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(threadKey);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });

  it("renders timeline notice i18n keys in the active locale", async () => {
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
              notice_i18n_key: "timeline.notice.roomCreate",
              message_kind: "notice"
            }
          ]
        }
      }
    });

    expect(await screen.findByText("ルームを作成しました")).toBeTruthy();
    expect(screen.queryByText("created the room")).toBeNull();
  });

  it("paginates an empty thread timeline once after initial items arrive", async () => {
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards
    });

    render(
      <TimelineView
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    expect(paginateBackwards).not.toHaveBeenCalled();

    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: threadKey,
          generation: 1,
          items: []
        }
      }
    });

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(threadKey);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
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
        can_request_keys: true
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

    expect(requestRoomKey).toHaveBeenCalledWith("!room:example.invalid", "$encrypted");
  });
});
