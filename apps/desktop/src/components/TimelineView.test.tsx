// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode, Suspense, startTransition, useEffect, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { openExternalHttpUrl } from "../domain/externalLinks";

vi.mock("../domain/externalLinks", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../domain/externalLinks")>()),
  openExternalHttpUrl: vi.fn(async () => undefined)
}));

import {
  roomTimelineKey,
  threadTimelineKey,
  type CoreEventPayload,
  type TimelineItem,
  type TimelineMessageSource
} from "../domain/coreEvents";
import { setActiveLocaleProfile } from "../i18n/messages";
import {
  applyTimelineEvent,
  createTimelineStore,
  type TimelineStoreState
} from "../domain/timelineStore";
import { TimelineStoreContext } from "./timelineStoreContext";
import {
  MessageSourceDialog,
  TimelineView,
  clearTimelineViewportSessionMemoryForTests,
  timelineMediaDisplayBoxForTests,
  type TimelineTransport
} from "./TimelineView";
import type { LiveSignalsState } from "../domain/types";

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  setActiveLocaleProfile("en", "none");
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
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

function imageMessage(eventId: string, encrypted = false): TimelineItem {
  return {
    ...message(eventId, "Image body"),
    media: {
      kind: "Image",
      filename: "photo.png",
      source: {
        mxc_uri: "mxc://example.invalid/photo",
        encrypted,
        encryption_version: encrypted ? "v2" : null
      },
      mimetype: "image/png",
      size: 416_768,
      width: 2048,
      height: 1188,
      thumbnail: null
    }
  };
}

function fileMessage(eventId: string): TimelineItem {
  return {
    ...message(eventId, "File body"),
    media: {
      kind: "File",
      filename: "notes.pdf",
      source: {
        mxc_uri: "mxc://example.invalid/notes",
        encrypted: false,
        encryption_version: null
      },
      mimetype: "application/pdf",
      size: 12_288,
      width: null,
      height: null,
      thumbnail: null
    }
  };
}

function messages(count: number, prefix = "$item"): TimelineItem[] {
  return Array.from({ length: count }, (_, index) =>
    message(`${prefix}${index}`, `message ${index}`)
  );
}

function navigationSnapshot(overrides: Partial<{
  read_marker_event_id: string | null;
  read_marker_display_event_id: string | null;
  first_unread_event_id: string | null;
  unread_event_count: number;
  unread_position: "none" | "aboveViewport" | "insideViewport" | "belowViewport" | "unknown";
  newer_event_count: number;
  can_jump_to_bottom: boolean;
}> = {}) {
  return {
    read_marker_event_id: null,
    read_marker_display_event_id: null,
    first_unread_event_id: null,
    unread_event_count: 0,
    unread_position: "none" as const,
    newer_event_count: 0,
    can_jump_to_bottom: false,
    ...overrides
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
  container: { top?: number; height?: number } = {},
  scrollContainerRef?: { current: HTMLElement | null }
) {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const eventId =
      this.getAttribute("data-event-id") ??
      this.getAttribute("data-frame-item-id") ??
      this.getAttribute("data-item-id");
    const testId = this.getAttribute("data-testid");
    const scrollTop = scrollContainerRef?.current?.scrollTop ?? 0;
    const top =
      testId === "timeline-view"
        ? container.top ?? 0
        : (rects[eventId ?? ""]?.top ?? 0) - scrollTop;
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

/**
 * Gives every rendered timeline row a deterministic block position based on
 * its presentation order. Unlike `mockTimelineRects`, this intentionally
 * follows DOM reordering so a test can observe the viewport correction that a
 * display-projection transaction must make.
 */
function mockPresentationOrderRects(
  scrollContainerRef: { current: HTMLElement | null },
  options: { rowHeight?: number; viewportHeight?: number } = {}
) {
  const rowHeight = options.rowHeight ?? 100;
  const viewportHeight = options.viewportHeight ?? 200;
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    const testId = this.getAttribute("data-testid");
    if (testId === "timeline-view") {
      return {
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 0,
        width: 0,
        height: viewportHeight,
        bottom: viewportHeight,
        toJSON: () => ({})
      } as DOMRect;
    }

    const row = this.matches("article[data-item-id]")
      ? this
      : this.querySelector<HTMLElement>("article[data-item-id]");
    if (!row) {
      return {
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 0,
        width: 0,
        height: 0,
        bottom: 0,
        toJSON: () => ({})
      } as DOMRect;
    }
    const rows = Array.from(document.querySelectorAll<HTMLElement>("article[data-item-id]"));
    const index = rows.indexOf(row);
    const top = index * rowHeight - (scrollContainerRef.current?.scrollTop ?? 0);
    return {
      x: 0,
      y: top,
      top,
      left: 0,
      right: 0,
      width: 0,
      height: rowHeight,
      bottom: top + rowHeight,
      toJSON: () => ({})
    } as DOMRect;
  });
}

function installResizeObserverMock() {
  const originalResizeObserver = window.ResizeObserver;
  const observers: Array<{ trigger: () => void }> = [];

  class MockResizeObserver {
    private readonly callback: ResizeObserverCallback;

    constructor(callback: ResizeObserverCallback) {
      this.callback = callback;
      observers.push({
        trigger: () => {
          this.callback([] as ResizeObserverEntry[], this as unknown as ResizeObserver);
        }
      });
    }

    observe = vi.fn();
    unobserve = vi.fn();
    disconnect = vi.fn();
  }

  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver
  });

  return {
    triggerAll() {
      for (const observer of observers) {
        observer.trigger();
      }
    },
    restore() {
      Object.defineProperty(window, "ResizeObserver", {
        configurable: true,
        writable: true,
        value: originalResizeObserver
      });
    }
  };
}

describe("TimelineView", () => {
  it("keeps edit live conversion DOM value and selection across timeline rerenders", () => {
    const editable = { ...message("$edit-ime", "before"), can_edit: true };
    const makeStore = (item: TimelineItem): TimelineStoreState =>
      applyTimelineEvent(createTimelineStore(), {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [item]
        }
      });
    const transport = baseTransport({});
    const view = (store: TimelineStoreState) => (
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    const { rerender } = render(view(makeStore(editable)));

    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLTextAreaElement;
    fireEvent.compositionStart(textarea);
    fireEvent.change(textarea, { target: { value: "日本語変換中" } });
    textarea.setSelectionRange(3, 5);
    rerender(view(makeStore({ ...editable, body: "stale timeline body", is_edited: true })));

    expect(textarea.value).toBe("日本語変換中");
    expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([3, 5]);
  });

  it("discards a stale deferred edit newline after newer DOM input", async () => {
    let resolveAction!: (action: "insertNewline") => void;
    const action = new Promise<"insertNewline">((resolve) => {
      resolveAction = resolve;
    });
    const editMessage = vi.fn(async () => undefined);
    const editable = { ...message("$edit-deferred", "captured"), can_edit: true };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          resolveComposerKeyAction={() => action}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLTextAreaElement;
    textarea.setSelectionRange(8, 8);
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    fireEvent.change(textarea, { target: { value: "newer edit input" } });
    await act(async () => resolveAction("insertNewline"));
    fireEvent.click(screen.getByRole("button", { name: /save edit/i }));

    expect(textarea.value).toBe("newer edit input");
    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-deferred",
      "newer edit input"
    );
  });

  it("sends the edit value captured when deferred Enter was pressed", async () => {
    let resolveAction!: (action: "send") => void;
    const action = new Promise<"send">((resolve) => {
      resolveAction = resolve;
    });
    const editMessage = vi.fn(async () => undefined);
    const editable = { ...message("$edit-send-snapshot", "intent snapshot"), can_edit: true };
    const store = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [editable]
      }
    });
    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({ editMessage })}
          resolveComposerKeyAction={() => action}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );
    fireEvent.click(screen.getByRole("button", { name: /edit message/i }));
    const textarea = screen.getByRole("textbox", { name: /edit.*body/i }) as HTMLTextAreaElement;
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter", keyCode: 13 });
    fireEvent.change(textarea, { target: { value: "later edit input" } });
    await act(async () => resolveAction("send"));

    expect(editMessage).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$edit-send-snapshot",
      "intent snapshot"
    );
  });

  it("computes a stable clamped media box for known image dimensions", () => {
    expect(timelineMediaDisplayBoxForTests(2048, 1188)).toEqual({
      inlineSize: 420,
      blockSize: 244
    });
    expect(timelineMediaDisplayBoxForTests(800, 1600)).toEqual({
      inlineSize: 130,
      blockSize: 260
    });
    expect(timelineMediaDisplayBoxForTests(null, 1600)).toBeNull();
    expect(timelineMediaDisplayBoxForTests(800, null)).toBeNull();
  });

  it("keeps the reaction emoji picker attached to its message row", async () => {
    const sendReaction = vi.fn(async () => undefined);
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react", "React here")]
      }
    });
    const transport = baseTransport({ sendReaction });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          onReply={vi.fn()}
        />
      </TimelineStoreContext.Provider>
    );

    const article = screen.getByText("React here").closest("article");
    expect(article).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(article!.contains(picker)).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: /grinning face$/i }));

    await waitFor(() => {
      expect(sendReaction).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$react",
        "😀"
      );
    });
  });

  it("opens the reaction emoji picker above when the composer-side space is insufficient", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        height = 240;
      } else if (this.classList.contains("reaction-control")) {
        top = 200;
      } else if (this.classList.contains("main-pane")) {
        height = 320;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-near-composer", "React near composer")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.classList.contains("is-above")).toBe(true);
    expect(picker.classList.contains("is-below")).toBe(false);
  });

  it("offers normal reply as an inline action next to reactions", async () => {
    const onReply = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [
          {
            ...message("$reply-inline", "Reply inline"),
            actions: {
              can_copy: true,
              can_forward: false,
              can_permalink: false,
              can_view_source: false
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
          onReply={onReply}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Reply inline").closest("article");
    expect(row).not.toBeNull();

    const actionButtons = Array.from(
      row!.querySelectorAll<HTMLButtonElement>(".message-actions .message-action")
    );
    expect(actionButtons.map((button) => button.getAttribute("aria-label"))).toEqual([
      "Add reaction",
      "Reply to message",
      "Pin message",
      "Message actions"
    ]);

    fireEvent.click(within(row!).getByRole("button", { name: "Reply to message" }));
    expect(onReply).toHaveBeenCalledWith("!room:example.invalid", "$reply-inline");

    fireEvent.click(within(row!).getByRole("button", { name: "Message actions" }));
    const menu = within(row!).getByRole("menu", { name: "Message actions" });
    expect(within(menu).queryByRole("menuitem", { name: "Reply to message" })).toBeNull();
  });

  it("autosaves sender aliases from the message action menu", () => {
    const onSetLocalUserAlias = vi.fn();
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$alias", "Alias me")]
      }
    });

    render(
      <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={vi.fn()}
          onSetLocalUserAlias={onSetLocalUserAlias}
        />
      </TimelineStoreContext.Provider>
    );

    const row = screen.getByText("Alias me").closest("article");
    expect(row).not.toBeNull();

    fireEvent.click(within(row!).getByRole("button", { name: "Message actions" }));
    fireEvent.click(
      within(row!).getByRole("menuitem", { name: "Set alias for @bob:example.invalid" })
    );

    fireEvent.change(screen.getByRole("textbox", { name: "Alias" }), {
      target: { value: "Builder Bob" }
    });

    expect(screen.queryByRole("button", { name: "Save alias" })).toBeNull();
    expect(onSetLocalUserAlias).toHaveBeenCalledWith(
      "@bob:example.invalid",
      "Builder Bob"
    );
  });

  it("shrinks the reaction emoji picker to the visible space instead of clipping it", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        top = 120;
        height = 260;
      } else if (this.classList.contains("reaction-control")) {
        top = 320;
      } else if (this.classList.contains("main-pane")) {
        top = 100;
        height = 500;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-tight-space", "React with tight space")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.style.getPropertyValue("--emoji-picker-max-block-size")).toBe("194px");
  });

  it("lets the reaction emoji picker use extra vertical room when available", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        top = 80;
        height = 720;
      } else if (this.classList.contains("reaction-control")) {
        top = 160;
      } else if (this.classList.contains("main-pane")) {
        top = 60;
        height = 760;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-roomy-space", "React with roomy space")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.classList.contains("is-below")).toBe(true);
    expect(picker.style.getPropertyValue("--emoji-picker-max-block-size")).toBe("610px");
  });

  it("updates the reaction emoji picker size when the visible space changes", async () => {
    let reactionControlTop = 320;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement
    ) {
      let top = 0;
      let height = 24;
      if (this.getAttribute("data-testid") === "timeline-view") {
        top = 120;
        height = 260;
      } else if (this.classList.contains("reaction-control")) {
        top = reactionControlTop;
      } else if (this.classList.contains("main-pane")) {
        top = 100;
        height = 500;
      }
      return {
        x: 0,
        y: top,
        top,
        left: 0,
        right: 480,
        width: 480,
        height,
        bottom: top + height,
        toJSON: () => ({})
      } as DOMRect;
    });
    const store: TimelineStoreState = applyTimelineEvent(createTimelineStore(), {
      InitialItems: {
        request_id: null,
        key: KEY,
        generation: 1,
        items: [message("$react-resized-space", "React after resize")]
      }
    });

    render(
      <div className="main-pane">
        <TimelineStoreContext.Provider value={{ store, setStore: vi.fn() }}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={baseTransport({})}
            onReply={vi.fn()}
          />
        </TimelineStoreContext.Provider>
      </div>
    );

    fireEvent.click(screen.getByRole("button", { name: /add reaction/i }));

    const picker = await screen.findByRole("dialog", { name: /emoji/i });
    expect(picker.style.getPropertyValue("--emoji-picker-max-block-size")).toBe("194px");

    reactionControlTop = 150;
    fireEvent(window, new Event("resize"));

    await waitFor(() => {
      expect(picker.classList.contains("is-below")).toBe(true);
      expect(picker.style.getPropertyValue("--emoji-picker-max-block-size")).toBe("200px");
    });
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

  it("marks the latest visible room event as read even when bottom pixels are not exact", async () => {
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
        expect(sendReadReceipt).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$latest:example.invalid"
        );
      });
      expect(setFullyRead).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$latest:example.invalid"
      );
      expect(observeViewport).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$older:example.invalid",
        "$latest:example.invalid",
        true
      );
    } finally {
      rectSpy.mockRestore();
    }
  });

  it("marks the latest visible thread event with a threaded read receipt", async () => {
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
        expect(sendReadReceipt).toHaveBeenCalledWith(
          "!room:example.invalid",
          "$thread-reply:example.invalid",
          "$root:example.invalid"
        );
      });
      expect(setFullyRead).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$thread-reply:example.invalid"
      );
      expect(observeViewport).not.toHaveBeenCalled();
    } finally {
      rectSpy.mockRestore();
    }
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

  it("emits private-data-free scroll diagnostics for the mounted timeline", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    await waitFor(() => {
      const latest = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
      expect(latest?.latestFrame?.endIndex ?? 0).toBeGreaterThan(0);
    });
    const latest = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(latest.renderCommits).toBeGreaterThan(0);
    expect(latest.scrollFrames).toBeGreaterThan(0);
    expect(JSON.stringify(latest)).not.toContain("!room:example.invalid");
    expect(JSON.stringify(latest)).not.toContain("$item");
  });

  it("defers virtual height commits during active scroll and flushes once after idle", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });
    const baselineDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineHeightModelCommits = baselineDiagnostics?.heightModelCommits ?? 0;
    const baselineMeasurementFlushes = baselineDiagnostics?.measurementFlushes ?? 0;

    fireEvent.wheel(timeline, { deltaY: 40 });
    fireEvent.scroll(timeline);

    rects.$item50 = { top: 50 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    await act(async () => {
      await Promise.resolve();
    });

    const activeDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(activeDiagnostics.heightModelCommits - baselineHeightModelCommits).toBe(0);
    expect(activeDiagnostics.pendingMeasuredRows).toBeGreaterThan(0);

    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - baselineMeasurementFlushes).toBe(1);
    expect(idleDiagnostics.heightModelCommits - baselineHeightModelCommits).toBeGreaterThan(0);
  });

  it("does not hide non-flushed changed rows in the post-flush measurement pass", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const rects: Record<string, { top: number; height: number }> = {};
    let mutateAfterFlush = false;
    let baselineMeasurementFlushes = 0;
    const onScrollDiagnosticsChange = vi.fn((diagnostics) => {
      if (
        mutateAfterFlush &&
        diagnostics.measurementFlushes > baselineMeasurementFlushes
      ) {
        mutateAfterFlush = false;
        rects.$item52 = { top: 52 * 72, height: 216 };
      }
    });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(700)
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });
    const baselineDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineHeightModelCommits = baselineDiagnostics?.heightModelCommits ?? 0;
    baselineMeasurementFlushes = baselineDiagnostics?.measurementFlushes ?? 0;

    fireEvent.wheel(timeline, { deltaY: 40 });
    fireEvent.scroll(timeline);

    rects.$item50 = { top: 50 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    await act(async () => {
      await Promise.resolve();
    });

    const activeDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(activeDiagnostics.pendingMeasuredRows).toBeGreaterThan(0);

    mutateAfterFlush = true;
    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - baselineMeasurementFlushes).toBe(1);
    expect(idleDiagnostics.heightModelCommits - baselineHeightModelCommits).toBe(2);
  });

  it("does not defer measurements from a programmatic scroll echo", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(700)
          }
        }
      });
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              unread_event_count: 1,
              newer_event_count: 2,
              can_jump_to_bottom: true
            })
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });
    await act(async () => {
      await Promise.resolve();
    });
    const jumpToBottomButton = screen.getByRole("button", { name: /Jump to bottom/i });
    const baselineDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const baselineHeightModelCommits = baselineDiagnostics?.heightModelCommits ?? 0;
    const baselineMeasurementFlushes = baselineDiagnostics?.measurementFlushes ?? 0;

    fireEvent.click(jumpToBottomButton);
    rects.$item699 = { top: 699 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 2,
            items: messages(700)
          }
        }
      });
    });
    await act(async () => {
      await Promise.resolve();
    });

    const afterEchoDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(afterEchoDiagnostics.pendingMeasuredRows).toBe(0);
    expect(afterEchoDiagnostics.heightModelCommits - baselineHeightModelCommits).toBeGreaterThan(0);

    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - baselineMeasurementFlushes).toBe(0);
  });

  it("classifies programmatic scroll writes by reason and suppresses their scroll echo", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 1000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    timeline.scrollTop = 1000;
    fireEvent.wheel(timeline, { deltaY: -40 });
    fireEvent.scroll(timeline);
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              can_jump_to_bottom: true,
              newer_event_count: 4
            })
          }
        }
      });
    });

    fireEvent.click(screen.getByRole("button", { name: /Jump to bottom/ }));
    fireEvent.scroll(timeline);

    const diagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(diagnostics.scrollWrites.jumpToBottom).toBe(1);
    expect(diagnostics.latestFrame?.userInputPending).toBe(false);
  });

  it("drops stale pending measurements after same timeline ItemsUpdated reset", async () => {
    vi.useFakeTimers();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    const rects: Record<string, { top: number; height: number }> = {};
    for (let index = 0; index < 700; index += 1) {
      rects[`$item${index}`] = { top: index * 72, height: 72 };
    }
    for (let index = 0; index < 20; index += 1) {
      rects[`$reset${index}`] = { top: index * 72, height: 72 };
    }
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        listRefCallback={(element) => {
          scrollContainerRef.current =
            element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
        }}
      />
    );

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: messages(700)
          }
        }
      });
    });

    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 3000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      writable: true,
      configurable: true
    });

    fireEvent.wheel(timeline, { deltaY: 40 });
    fireEvent.scroll(timeline);
    rects.$item50 = { top: 50 * 72, height: 180 };
    fireEvent.scroll(timeline);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(onScrollDiagnosticsChange.mock.calls.at(-1)?.[0].pendingMeasuredRows).toBeGreaterThan(0);

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 2,
            diffs: [
              {
                Reset: {
                  items: messages(20, "$reset")
                }
              }
            ]
          }
        }
      });
    });
    expect(timeline.getAttribute("data-total-items")).toBe("20");
    const resetDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    const resetHeightModelCommits = resetDiagnostics?.heightModelCommits ?? 0;
    const resetMeasurementFlushes = resetDiagnostics?.measurementFlushes ?? 0;

    act(() => {
      vi.advanceTimersByTime(100);
    });

    const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
    expect(idleDiagnostics.measurementFlushes - resetMeasurementFlushes).toBe(0);
    expect(idleDiagnostics.heightModelCommits - resetHeightModelCommits).toBe(0);
    expect(idleDiagnostics.pendingMeasuredRows).toBe(0);
  });

  it("drops pending scroll frame diagnostics after the timeline key changes", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    const flushFrames = () => {
      const queued = [...frames.entries()];
      frames.clear();
      for (const [, callback] of queued) {
        callback(0);
      }
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const renderView = (timelineKey = KEY) => (
      <TimelineView
        timelineKey={timelineKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    const { rerender } = render(renderView());
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 700 * 72, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 20_000,
      writable: true,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    await waitFor(() => expect(timeline.getAttribute("data-virtualized")).toBe("true"));
    act(() => {
      flushFrames();
    });
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      fireEvent.wheel(timeline, { deltaY: 4 });
      timeline.scrollTop += 4;
      fireEvent.scroll(timeline);
    });
    expect(frames.size).toBe(1);

    act(() => {
      rerender(renderView(threadKey));
    });
    act(() => {
      flushFrames();
    });

    const activeFrames = onScrollDiagnosticsChange.mock.calls
      .map(([diagnostics]) => diagnostics.latestFrame)
      .filter((frame) => frame?.scrollActivity === "active");
    expect(activeFrames).toEqual([]);
  });

  it("drops pending scroll frame diagnostics after same timeline items reset", async () => {
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    const flushFrames = () => {
      const queued = [...frames.entries()];
      frames.clear();
      for (const [, callback] of queued) {
        callback(0);
      }
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );
    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 700 * 72, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 20_000,
      writable: true,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });

    await waitFor(() => expect(timeline.getAttribute("data-virtualized")).toBe("true"));
    act(() => {
      flushFrames();
    });
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      fireEvent.wheel(timeline, { deltaY: 4 });
      timeline.scrollTop += 4;
      fireEvent.scroll(timeline);
    });
    expect(frames.size).toBe(1);

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 2,
            items: Array.from({ length: 20 }, (_, index) =>
              message(`$reset${index}`, `reset message ${index}`)
            )
          }
        }
      });
    });
    await waitFor(() => {
      expect(timeline.getAttribute("data-timeline-generation")).toBe("2");
      expect(timeline.getAttribute("data-total-items")).toBe("20");
    });
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      flushFrames();
    });

    const activeFrames = onScrollDiagnosticsChange.mock.calls
      .map(([diagnostics]) => diagnostics.latestFrame)
      .filter((frame) => frame?.scrollActivity === "active");
    expect(activeFrames).toEqual([]);
  });

  it("cancels delayed programmatic scroll follow-ups after timeline key changes", async () => {
    vi.useFakeTimers();
    const onScrollDiagnosticsChange = vi.fn();
    let listener: ((payload: CoreEventPayload) => void) | null = null;
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    const flushFrames = () => {
      const queued = [...frames.entries()];
      frames.clear();
      for (const [, callback] of queued) {
        callback(0);
      }
    };
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$root:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        listener = nextListener;
        return () => undefined;
      }
    });
    const renderView = (timelineKey: typeof KEY | typeof threadKey) => (
      <TimelineView
        timelineKey={timelineKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={() => undefined}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      />
    );

    const { rerender } = render(renderView(KEY));
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollTop", {
      value: 1000,
      writable: true,
      configurable: true
    });
    Object.defineProperty(timeline, "scrollHeight", {
      value: 700 * 72,
      configurable: true
    });
    Object.defineProperty(timeline, "clientHeight", {
      value: 600,
      configurable: true
    });

    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: Array.from({ length: 700 }, (_, index) =>
              message(`$item${index}`, `message ${index}`)
            )
          }
        }
      });
    });
    act(() => {
      flushFrames();
    });
    timeline.scrollTop = 1000;
    fireEvent.wheel(timeline, { deltaY: -40 });
    fireEvent.scroll(timeline);
    act(() => {
      listener?.({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              can_jump_to_bottom: true,
              newer_event_count: 4
            })
          }
        }
      });
    });

    fireEvent.click(screen.getByRole("button", { name: /Jump to bottom/ }));
    expect(frames.size).toBeGreaterThan(0);

    act(() => {
      rerender(renderView(threadKey));
    });
    timeline.scrollTop = 1000;
    onScrollDiagnosticsChange.mockClear();

    act(() => {
      flushFrames();
    });

    const jumpWritesAfterKeyChange = onScrollDiagnosticsChange.mock.calls
      .map(([diagnostics]) => diagnostics.scrollWrites.jumpToBottom)
      .filter((count) => count > 0);
    expect(jumpWritesAfterKeyChange).toEqual([]);
  });

  it("does not re-emit scroll diagnostics from parent state commits", async () => {
    const onScrollDiagnosticsChange = vi.fn();

    function Parent() {
      const [, setDiagnostics] = useState<unknown>(null);
      return (
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={baseTransport({})}
          onReply={() => undefined}
          onScrollDiagnosticsChange={(diagnostics) => {
            onScrollDiagnosticsChange(diagnostics);
            if (onScrollDiagnosticsChange.mock.calls.length <= 4) {
              setDiagnostics(diagnostics);
            }
          }}
        />
      );
    }

    render(<Parent />);

    await waitFor(() => expect(onScrollDiagnosticsChange).toHaveBeenCalled());
    await act(async () => undefined);

    expect(onScrollDiagnosticsChange.mock.calls.length).toBeLessThanOrEqual(2);
  });

  it("paginates older history when the user scrolls to the top even if prefetch is disabled", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
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

    emit({
      kind: "Timeline",
      event: {
        PaginationStateChanged: {
          request_id: null,
          key: KEY,
          direction: "Backward",
          state: "Idle"
        }
      }
    });

    await waitFor(() => {
      const backfillMessages = onDiagnosticLogEntry.mock.calls
        .map(([entry]) => entry)
        .filter((entry) => entry.source === "timeline.backfill")
        .map((entry) => entry.message);
      expect(backfillMessages[0]).toContain("stage=request trigger=scroll");
      expect(backfillMessages[0]).toContain("threshold_px=0");
      expect(backfillMessages).toEqual(
        expect.arrayContaining([expect.stringContaining("stage=complete reason=pagination_idle")])
      );
    });
  });

  it("backfills an underfilled room timeline after short initial items arrive", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
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
        autoLoadOlderMessages={true}
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 320, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });

    act(() => {
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
    });

    await waitFor(() => {
      const underfilledLogs = onDiagnosticLogEntry.mock.calls
        .map(([entry]) => entry)
        .filter((entry) => entry.source === "timeline.backfill")
        .map((entry) => entry.message)
        .filter((message) => message.includes("trigger=underfilled_initial"));
      expect(underfilledLogs).toEqual([
        expect.stringContaining("stage=request trigger=underfilled_initial")
      ]);
      expect(underfilledLogs[0]).toContain("items=1");
      expect(underfilledLogs[0]).toContain("scroll_height_px=320");
      expect(underfilledLogs[0]).toContain("client_height_px=600");
      expect(underfilledLogs[0]).toContain("overflow_px=0");
      expect(underfilledLogs[0]).toContain("auto_load=true");
      expect(underfilledLogs[0]).toContain("state=Idle");
    });
    expect(paginateBackwards).toHaveBeenCalledWith(KEY);
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });

  it("captures the bottom-most visible event as the persisted room scroll anchor", async () => {
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

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
    expect(updateScrollAnchor).toHaveBeenCalledWith(
      "!room:example.invalid",
      expect.objectContaining({
        event_id: "$second:example.invalid",
        edge: "bottom",
        offset_px: -132,
        updated_at_ms: expect.any(Number)
      })
    );

    act(() => {
      fireEvent.scroll(timeline);
    });

    expect(updateScrollAnchor).toHaveBeenCalledTimes(1);
  });

  it("persists the sent message as the room anchor after a programmatic live-edge scroll", async () => {
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
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };

    mockTimelineRects(
      {
        "$older:example.invalid": { top: 2100, height: 80 },
        "$sent:example.invalid": { top: 2320, height: 60 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2400, configurable: true });
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
            items: [message("$older:example.invalid", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                PushBack: {
                  item: {
                    ...message("$sent:example.invalid", "Message I just sent"),
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Message I just sent")).toBeTruthy();
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$sent:example.invalid",
          edge: "bottom",
          offset_px: -20,
          updated_at_ms: expect.any(Number)
        })
      );
    });
  });

  it("restores an in-session bottom-edge room anchor when the event is already rendered", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };

    mockTimelineRects(
      {
        "$anchor:example.invalid": { top: 500, height: 48 },
        "$after:example.invalid": { top: 560, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    const { unmount } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });
    timeline.scrollTop = 48;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$after:example.invalid",
          edge: "bottom",
          offset_px: -40,
          updated_at_ms: expect.any(Number)
        })
      );
    });

    unmount();
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const restoredTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = restoredTimeline;
    Object.defineProperty(restoredTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(restoredTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(restoredTimeline, "scrollTop", {
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
      expect(restoredTimeline.scrollTop).toBe(48);
    });
  });

  it("does not auto-backfill after restoring an in-session room anchor until the user scrolls", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    const paginateBackwards = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      paginateBackwards,
      updateScrollAnchor
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };

    mockTimelineRects(
      {
        "$anchor:example.invalid": { top: 500, height: 48 },
        "$after:example.invalid": { top: 560, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    const { unmount } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });
    timeline.scrollTop = 48;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);

    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$after:example.invalid",
          edge: "bottom"
        })
      );
    });
    expect(paginateBackwards).not.toHaveBeenCalled();

    unmount();
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        autoLoadOlderMessages
        onReply={vi.fn()}
        onDiagnosticLogEntry={onDiagnosticLogEntry}
      />
    );
    const restoredTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = restoredTimeline;
    Object.defineProperty(restoredTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(restoredTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(restoredTimeline, "scrollTop", {
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
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(restoredTimeline.scrollTop).toBe(48);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });

    fireEvent.scroll(restoredTimeline);

    expect(paginateBackwards).not.toHaveBeenCalled();
    expect(onDiagnosticLogEntry.mock.calls.map(([entry]) => entry.message)).toEqual(
      expect.arrayContaining([
        expect.stringContaining("reason=await_user_scroll_after_room_restore")
      ])
    );

    fireEvent.wheel(restoredTimeline, { deltaY: -120 });
    fireEvent.scroll(restoredTimeline);

    await waitFor(() => {
      expect(paginateBackwards).toHaveBeenCalledWith(KEY);
    });
    expect(paginateBackwards).toHaveBeenCalledTimes(1);
  });

  it("falls back to live edge and clears session anchor when the in-session anchor is missing", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const updateScrollAnchor = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(
      {
        "$anchor:example.invalid": { top: 500, height: 48 },
        "$after:example.invalid": { top: 560, height: 48 },
        "$live-top:example.invalid": { top: 1300, height: 48 },
        "$live-bottom:example.invalid": { top: 1900, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    const { unmount } = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });
    timeline.scrollTop = 48;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenLastCalledWith(
        "!room:example.invalid",
        expect.objectContaining({
          event_id: "$after:example.invalid",
          edge: "bottom"
        })
      );
    });

    unmount();
    const fallbackRender = render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const fallbackTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = fallbackTimeline;
    Object.defineProperty(fallbackTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(fallbackTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(fallbackTimeline, "scrollTop", {
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
            message("$live-top:example.invalid", "Live top"),
            message("$live-bottom:example.invalid", "Live bottom")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(fallbackTimeline.scrollTop).toBe(1400);
    });

    fallbackRender.unmount();
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    const liveEdgeTimeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = liveEdgeTimeline;
    Object.defineProperty(liveEdgeTimeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(liveEdgeTimeline, "clientHeight", { value: 600, configurable: true });
    Object.defineProperty(liveEdgeTimeline, "scrollTop", {
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
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(liveEdgeTimeline.scrollTop).toBe(1400);
    });
  });

  it("does not reapply a persisted room anchor across later rerenders", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rects = {
      "$anchor:example.invalid": { top: 500, height: 48 },
      "$after:example.invalid": { top: 560, height: 48 }
    };

    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    const props = {
      timelineKey: KEY,
      roomId: "!room:example.invalid",
      transport,
      roomScrollAnchor: {
        event_id: "$anchor:example.invalid",
        edge: "bottom" as const,
        offset_px: -100,
        updated_at_ms: Date.now()
      },
      onReply: vi.fn()
    };
    const { rerender } = render(<TimelineView {...props} />);

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });

    rects["$anchor:example.invalid"].top = 530;
    rerender(
      <TimelineView
        {...props}
        liveSignals={{ presence: {}, rooms: {} }}
      />
    );

    expect(timeline.scrollTop).toBe(1400);
  });

  it("does not move a free-scroll viewport when read receipts shift earlier rows", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const rects = {
      "$seen:example.invalid": { top: 440, height: 48 },
      "$anchor:example.invalid": { top: 500, height: 48 },
      "$after:example.invalid": { top: 560, height: 48 }
    };

    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

    const props = {
      timelineKey: KEY,
      roomId: "!room:example.invalid",
      transport,
      roomScrollAnchor: {
        event_id: "$anchor:example.invalid",
        edge: "bottom" as const,
        offset_px: -100,
        updated_at_ms: Date.now()
      },
      onReply: vi.fn()
    };
    const { rerender } = render(<TimelineView {...props} />);

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "scrollHeight", { value: 2000, configurable: true });
    Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });
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
            message("$seen:example.invalid", "Seen"),
            message("$anchor:example.invalid", "Anchor"),
            message("$after:example.invalid", "After")
          ]
        }
      }
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1400);
    });
    await act(async () => {
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
    });

    rects["$anchor:example.invalid"].top = 530;
    timeline.scrollTop = 58;
    fireEvent.wheel(timeline, { deltaY: -120 });
    fireEvent.scroll(timeline);
    rerender(
      <TimelineView
        {...props}
        liveSignals={{
          presence: {},
          rooms: {
            "!room:example.invalid": {
              fully_read_event_id: null,
              typing_user_ids: [],
              receipts_by_event: {
                "$seen:example.invalid": {
                  total_count: 1,
                  overflow_count: 0,
                  readers: [
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
      />
    );

    expect(timeline.scrollTop).toBe(58);
  });

  it("ignores persisted anchors on first room entry and opens at the live edge", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const roomId = "!room:example.invalid";
    const anchorEventId = "$anchor:example.invalid";
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
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
          edge: "bottom",
          offset_px: -100,
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
      expect(screen.getByText("Live top")).toBeTruthy();
      expect(timeline.getAttribute("data-timeline-generation")).toBe("1");
      expect(timeline.scrollTop).toBe(1400);
    });
  });

  it("does not chase a missing persisted anchor on first room entry", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const roomId = "!room:example.invalid";
    const anchorEventId = "$anchor:example.invalid";
    const updateScrollAnchor = vi.fn(async () => undefined);
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      updateScrollAnchor
    });
    mockTimelineRects(
      {
        "$live-top:example.invalid": { top: 1300, height: 48 },
        "$live-bottom:example.invalid": { top: 1900, height: 48 }
      },
      { top: 0, height: 600 },
      scrollContainerRef
    );

    render(
      <TimelineView
        timelineKey={KEY}
        roomId={roomId}
        transport={transport}
        roomScrollAnchor={{
          event_id: anchorEventId,
          edge: "bottom",
          offset_px: 50,
          updated_at_ms: Date.now()
        }}
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
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
      expect(timeline.scrollTop).toBe(1400);
    });
    await waitFor(() => {
      expect(updateScrollAnchor).toHaveBeenCalledWith(
        roomId,
        expect.objectContaining({
          event_id: "$live-bottom:example.invalid",
          edge: "bottom"
        })
      );
    });
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
      expect(timeline.scrollTop).toBe(1500);
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
      expect(timeline.scrollTop).toBe(1500);
    });
  });

  it("scrolls to the sent local echo even when the user was reading above the bottom", async () => {
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
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    Object.defineProperty(timeline, "scrollHeight", { value: 2400, configurable: true });
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
            items: [message("$older", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    timeline.scrollTop = 400;

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                PushBack: {
                  item: {
                    ...message("$local-echo", "Message I just sent"),
                    id: { Transaction: { transaction_id: "txn-1" } },
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Message I just sent")).toBeTruthy();
      expect(timeline.scrollTop).toBe(1800);
    });
  });

  it("keeps the live edge pinned when the read marker appears below a sent message", async () => {
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
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    let scrollHeight = 2400;
    Object.defineProperty(timeline, "scrollHeight", {
      get: () => scrollHeight,
      configurable: true
    });
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
            items: [message("$older", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                PushBack: {
                  item: {
                    ...message("$sent:example.invalid", "Test"),
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(screen.getByText("Test")).toBeTruthy();
      expect(timeline.scrollTop).toBe(1800);
    });

    scrollHeight = 2440;
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              read_marker_event_id: "$sent:example.invalid",
              read_marker_display_event_id: "$sent:example.invalid",
              first_unread_event_id: null,
              unread_event_count: 0,
              unread_position: "none",
              newer_event_count: 0,
              can_jump_to_bottom: false
            }
          }
        }
      });
    });

    expect(await screen.findByRole("separator", { name: "Read up to here" })).toBeTruthy();
    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1840);
    });
  });

  it("keeps the live edge pinned when rendered content grows without a React commit", async () => {
    const resizeObserver = installResizeObserverMock();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          currentUserId="@alice:example.invalid"
          onReply={vi.fn()}
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      let scrollHeight = 2400;
      Object.defineProperty(timeline, "scrollHeight", {
        get: () => scrollHeight,
        configurable: true
      });
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
              items: [message("$older", "Older message")]
            }
          }
        });
      });

      await waitFor(() => {
        expect(timeline.scrollTop).toBe(1800);
      });

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            ItemsUpdated: {
              key: KEY,
              generation: 1,
              batch_id: 1,
              diffs: [
                {
                  PushBack: {
                    item: {
                      ...message("$sent:example.invalid", "Test"),
                      sender: "@alice:example.invalid",
                      send_state: { kind: "sending" }
                    }
                  }
                }
              ]
            }
          }
        });
      });

      await waitFor(() => {
        expect(screen.getByText("Test")).toBeTruthy();
        expect(timeline.scrollTop).toBe(1800);
      });

      scrollHeight = 2480;
      act(() => {
        resizeObserver.triggerAll();
      });

      await waitFor(() => {
        expect(timeline.scrollTop).toBe(1880);
      });
    } finally {
      resizeObserver.restore();
    }
  });

  it("does not keep the sent-message live-edge lock after user scroll input", async () => {
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
        currentUserId="@alice:example.invalid"
        onReply={vi.fn()}
      />
    );

    const timeline = await screen.findByTestId("timeline-view");
    let scrollHeight = 2400;
    Object.defineProperty(timeline, "scrollHeight", {
      get: () => scrollHeight,
      configurable: true
    });
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
            items: [message("$older", "Older message")]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [
              {
                PushBack: {
                  item: {
                    ...message("$sent:example.invalid", "Test"),
                    sender: "@alice:example.invalid",
                    send_state: { kind: "sending" }
                  }
                }
              }
            ]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1800);
    });

    act(() => {
      fireEvent.wheel(timeline, { deltaY: -120 });
      timeline.scrollTop = 1700;
      fireEvent.scroll(timeline);
    });

    scrollHeight = 2440;
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: {
              read_marker_event_id: "$sent:example.invalid",
              read_marker_display_event_id: "$sent:example.invalid",
              first_unread_event_id: null,
              unread_event_count: 0,
              unread_position: "none",
              newer_event_count: 0,
              can_jump_to_bottom: false
            }
          }
        }
      });
    });

    expect(await screen.findByRole("separator", { name: "Read up to here" })).toBeTruthy();
    expect(timeline.scrollTop).toBe(1700);
  });

  it("drops the live-edge lock immediately on wheel input before the scroll event settles", async () => {
    const resizeObserver = installResizeObserverMock();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    try {
      render(
        <TimelineView
          timelineKey={KEY}
          roomId="!room:example.invalid"
          transport={transport}
          currentUserId="@alice:example.invalid"
          onReply={vi.fn()}
        />
      );

      const timeline = await screen.findByTestId("timeline-view");
      let scrollHeight = 2400;
      Object.defineProperty(timeline, "scrollHeight", {
        get: () => scrollHeight,
        configurable: true
      });
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
              items: [message("$older", "Older message")]
            }
          }
        });
      });

      await waitFor(() => {
        expect(timeline.scrollTop).toBe(1800);
      });

      act(() => {
        fireEvent.wheel(timeline, { deltaY: -120 });
      });

      scrollHeight = 2480;
      act(() => {
        resizeObserver.triggerAll();
      });

      expect(timeline.scrollTop).toBe(1800);
    } finally {
      resizeObserver.restore();
    }
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

  it("surfaces reaction senders in a hoverable tooltip using profile labels", async () => {
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
                    sender_preview: ["@ken:example.invalid", "@satoshi:example.invalid"]
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
      expect(screen.getByText("Ken Inayoshi and Satoshi Terasaki reacted with 😢")).toBeTruthy();
    });
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
                    sender_preview: ["@ken:example.invalid"]
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

  it("automatically requests previews for encrypted image attachments", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
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
            items: [imageMessage("$encrypted-image", true)]
          }
        }
      });
    });

    await waitFor(() => {
      expect(downloadMedia).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$encrypted-image"
      );
    });
  });

  it("renders ready image with image-first layout and hover download overlay", async () => {
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
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "appmedia://synthetic-image",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
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
            items: [imageMessage("$ready-image", false)]
          }
        }
      });
    });

    await waitFor(() => {
      const media = document.querySelector('[data-event-id="$ready-image"] .message-media');
      expect(media).not.toBeNull();
      expect(media?.getAttribute("data-download-state")).toBe("ready");
      // #163: image-first layout — the preview is the primary block. The
      // filename lives on the image (alt), not as text laid over the preview,
      // and download appears in the hover/focus action overlay.
      const image = media?.querySelector<HTMLImageElement>(".message-media-image");
      expect(image).not.toBeNull();
      expect(image?.getAttribute("alt")).toBe("photo.png");
      const actionButtons = Array.from(
        media?.querySelectorAll<HTMLButtonElement>(
          ".message-media-hover-actions .message-media-hover-action"
        ) ?? []
      );
      const actionLabels = actionButtons.map((button) => button.getAttribute("aria-label"));
      expect(actionLabels).toEqual(["Show media details for photo.png", "Download photo.png"]);
      const downloadButton = actionButtons.find(
        (button) => button.getAttribute("aria-label") === "Download photo.png"
      );
      expect(downloadButton).not.toBeNull();
      expect(downloadButton?.tagName).toBe("BUTTON");
      expect(media?.textContent).not.toContain("image/png");
      expect(media?.textContent).not.toContain("407 KB");
    });
  });

  it("renders ready file downloads as navigation-safe buttons", async () => {
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
        mediaDownloads={{
          "$ready-file": {
            kind: "ready",
            source_url: "asset://localhost/notes.pdf",
            width: null,
            height: null,
            mime_type: "application/pdf"
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
            items: [fileMessage("$ready-file")]
          }
        }
      });
    });

    await waitFor(() => {
      const downloadButton = document.querySelector<HTMLButtonElement>(
        '[data-event-id="$ready-file"] button.message-media-download'
      );
      expect(downloadButton).not.toBeNull();
      expect(downloadButton?.getAttribute("aria-label")).toBe("Download notes.pdf");
    });
  });

  it("routes ready image preview downloads through the transport when available", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const fetchMock = vi.fn(async () => new Response(new Blob(["image"], { type: "image/png" })));
    const createObjectURL = vi.fn(() => "blob:downloaded-image");
    const revokeObjectURL = vi.fn();
    const OriginalURL = URL;
    class MockURL extends OriginalURL {
      static override createObjectURL = createObjectURL;
      static override revokeObjectURL = revokeObjectURL;
    }
    const clickedAnchors: HTMLAnchorElement[] = [];
    vi.stubGlobal("fetch", fetchMock);
    vi.stubGlobal("URL", MockURL);
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
      this: HTMLAnchorElement
    ) {
      clickedAnchors.push(this);
    });
    const saveMediaFile = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      saveMediaFile
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
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
            items: [imageMessage("$ready-image", false)]
          }
        }
      });
    });

    const downloadButton = await screen.findByRole("button", { name: "Download photo.png" });
    fireEvent.click(downloadButton);

    await waitFor(() => {
      expect(saveMediaFile).toHaveBeenCalledWith(
        "asset://localhost/original-photo.png",
        "photo.png"
      );
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(createObjectURL).not.toHaveBeenCalled();
    expect(clickedAnchors).toHaveLength(0);
    expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
  });

  it("does not request encrypted image previews for off-window initial virtualized items", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadMedia = vi.fn(async () => undefined);
    const transport = baseTransport({
      downloadMedia,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const items = Array.from({ length: 700 }, (_, index) =>
      index === 350
        ? imageMessage("$offscreen-image", true)
        : message(`$plain-${index}`, `Plain ${index}`)
    );

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

    await waitFor(() => {
      const renderedItems = Number(
        screen.getByTestId("timeline-view").getAttribute("data-rendered-items")
      );
      expect(renderedItems).toBeGreaterThan(0);
      expect(renderedItems).toBeLessThan(items.length);
    });
    expect(downloadMedia).not.toHaveBeenCalledWith(
      "!room:example.invalid",
      "$offscreen-image"
    );
    expect(downloadMedia).not.toHaveBeenCalled();
  });

  it("opens ready image previews in an in-app media viewer", async () => {
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
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
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
            items: [imageMessage("$ready-image", true)]
          }
        }
      });
    });

    await waitFor(() => {
      const image = screen.getByRole("img", { name: "photo.png" });
      const previewButton = image.closest("button");
      expect(previewButton?.getAttribute("aria-label")).toBe("Open file");
      const media = document.querySelector(".message-media");
      // #163: image-first layout. The encrypted badge stays visible as a
      // security signal and the download sits in the hover overlay, but
      // filename/mimetype/size no longer occupy layout over the preview.
      expect(media?.querySelector(".message-media-image-badge")?.textContent).toContain(
        "Encrypted"
      );
      expect(media?.querySelector(".message-media-hover-actions")).not.toBeNull();
      expect(media?.textContent).not.toContain("image/png");
      expect(media?.textContent).not.toContain("407 KB");
    });

    fireEvent.click(screen.getByRole("button", { name: "Open file" }));

    const viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    expect(viewer.textContent).toContain("photo.png");
    expect(viewer.textContent).toContain("407 KB");
    expect(viewer.querySelector<HTMLImageElement>(".timeline-media-viewer-image")?.src).toContain(
      "asset://localhost/original-photo.png"
    );

    fireEvent.click(screen.getByRole("button", { name: "Close media viewer" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });
  });

  it("keeps ready image metadata behind an inline details action", async () => {
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
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
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
            items: [imageMessage("$ready-image", true)]
          }
        }
      });
    });

    const detailsButton = await screen.findByRole("button", {
      name: "Show media details for photo.png"
    });
    const media = document.querySelector(".message-media");
    expect(media?.textContent).not.toContain("image/png");
    expect(media?.textContent).not.toContain("407 KB");

    fireEvent.click(detailsButton);

    const details = await screen.findByRole("dialog", { name: "Media details" });
    expect(details.textContent).toContain("photo.png");
    expect(details.textContent).toContain("image/png");
    expect(details.textContent).toContain("407 KB");
    expect(details.textContent).toContain("2048x1188");
    expect(details.textContent).toContain("Encrypted");
  });

  it("focuses the media viewer close control and returns focus to the clicked image", async () => {
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
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
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
            items: [imageMessage("$ready-image", false)]
          }
        }
      });
    });

    const openButton = await screen.findByRole("button", { name: "Open file" });
    openButton.focus();
    fireEvent.click(openButton);

    const viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    const closeButton = within(viewer).getByRole("button", { name: "Close media viewer" });
    await waitFor(() => {
      expect(document.activeElement).toBe(closeButton);
    });

    const tabEvent = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true
    });
    document.dispatchEvent(tabEvent);
    expect(tabEvent.defaultPrevented).toBe(true);
    expect(viewer.contains(document.activeElement)).toBe(true);

    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });
    expect(document.activeElement).toBe(openButton);
  });

  it("routes media viewer message actions through the event transport", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const loadMessageSource = vi.fn(async () => undefined);
    const redactMessage = vi.fn(async () => undefined);
    const forwardMessage = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      loadMessageSource,
      redactMessage,
      forwardMessage
    });

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        mediaDownloads={{
          "$ready-image": {
            kind: "ready",
            source_url: "asset://localhost/original-photo.png",
            width: 2048,
            height: 1188,
            mime_type: "image/png"
          }
        }}
        forwardDestinations={[
          {
            room_id: "!destination:example.invalid",
            display_name: "Destination room"
          }
        ]}
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
                ...imageMessage("$ready-image", false),
                can_redact: true,
                actions: {
                  can_copy: false,
                  can_forward: true,
                  can_permalink: false,
                  can_view_source: true
                }
              }
            ]
          }
        }
      });
    });

    fireEvent.click(await screen.findByRole("button", { name: "Open file" }));
    let viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    fireEvent.click(within(viewer).getByRole("button", { name: "Message actions" }));
    expect(within(viewer).getByRole("menu", { name: "Message actions" })).not.toBeNull();
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "Forward" }));
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "Destination room" }));
    await waitFor(() => {
      expect(forwardMessage).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$ready-image",
        "!destination:example.invalid"
      );
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    fireEvent.click(within(viewer).getByRole("button", { name: "Message actions" }));
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "View source" }));
    await waitFor(() => {
      expect(loadMessageSource).toHaveBeenCalledWith("!room:example.invalid", "$ready-image");
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    viewer = await screen.findByRole("dialog", { name: "Media viewer" });
    fireEvent.click(within(viewer).getByRole("button", { name: "Message actions" }));
    fireEvent.click(within(viewer).getByRole("menuitem", { name: "Remove" }));

    await waitFor(() => {
      expect(redactMessage).toHaveBeenCalledWith("!room:example.invalid", "$ready-image");
      expect(screen.queryByRole("dialog", { name: "Media viewer" })).toBeNull();
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

  it("limits initial avatar thumbnail requests to the current viewport window", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const downloadAvatarThumbnail = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      downloadAvatarThumbnail
    });
    const items = Array.from({ length: 40 }, (_, index) => ({
      ...message(`$avatar-window-${index}`, `Avatar row ${index}`),
      sender_avatar: {
        mxc_uri: `mxc://matrix.org/avatar-window-${index}`,
        thumbnail: { kind: "notRequested" as const }
      }
    }));

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
          items
        }
      }
    });

    await waitFor(() => {
      expect(downloadAvatarThumbnail).toHaveBeenCalledWith(
        "mxc://matrix.org/avatar-window-0"
      );
    });
    expect(downloadAvatarThumbnail).not.toHaveBeenCalledWith(
      "mxc://matrix.org/avatar-window-39"
    );
    expect(downloadAvatarThumbnail.mock.calls.length).toBeLessThan(items.length);
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
            source_url: "koushi-thumbnail://localhost/avatar/retry",
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
            source_url: "koushi-thumbnail://localhost/avatar/sender",
            width: null,
            height: null,
            mime_type: null
          }
        }
      }
    });

    await waitFor(() => {
      const image = document.querySelector<HTMLImageElement>(".message .avatar img");
      expect(image?.getAttribute("src")).toBe("koushi-thumbnail://localhost/avatar/sender");
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
            source_url: "koushi-thumbnail://localhost/avatar/unrelated",
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
            source_url: "koushi-thumbnail://localhost/avatar/a",
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
            source_url: "koushi-thumbnail://localhost/avatar/b",
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
      expect(firstImage?.getAttribute("src")).toBe("koushi-thumbnail://localhost/avatar/a");
      expect(secondImage?.getAttribute("src")).toBe("koushi-thumbnail://localhost/avatar/b");
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
                read_marker_display_event_id: null,
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

  it("keeps an old-root placeholder at latest activity and replaces it without canonical pagination", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const latestReply = {
      ...message("$old-root-latest:example.invalid", "standalone old-root reply"),
      timestamp_ms: 1_800_000_010_000,
      thread_root: "$old-root:example.invalid"
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        threadRootOrder={{ kind: "latestReply" }}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key: KEY, generation: 1, items: [latestReply] }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key: KEY,
            projection: {
              root_event_id: "$old-root:example.invalid",
              activity_event_id: "$old-root-latest:example.invalid",
              activity_timestamp_ms: 1_800_000_010_000,
              state: { kind: "pending" }
            }
          }
        }
      });
    });

    const pending = await screen.findByRole("status");
    const pendingRow = pending.closest<HTMLElement>("article");
    expect(pending.textContent).toContain("Loading thread message");
    expect(pendingRow?.getAttribute("data-row-id")).toBe(
      "thread-root:$old-root:example.invalid"
    );
    expect(pendingRow?.getAttribute("data-content-event-id")).toBe("$old-root:example.invalid");
    expect(pendingRow?.getAttribute("data-activity-event-id")).toBe(
      "$old-root-latest:example.invalid"
    );
    expect(screen.queryByText("standalone old-root reply")).toBeNull();

    const loadedRoot = {
      ...message("$old-root:example.invalid", "hydrated original root"),
      timestamp_ms: 1_700_000_000_000,
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$old-root-latest:example.invalid",
        latest_sender: null,
        latest_sender_label: null,
        latest_body_preview: null,
        latest_timestamp_ms: 1_800_000_010_000
      }
    };
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key: KEY,
            projection: {
              root_event_id: "$old-root:example.invalid",
              activity_event_id: "$old-root-latest:example.invalid",
              activity_timestamp_ms: 1_800_000_010_000,
              state: { kind: "ready", item: loadedRoot }
            }
          }
        }
      });
    });

    const readyRow = await screen.findByText("hydrated original root").then((node) =>
      node.closest<HTMLElement>("article")
    );
    expect(readyRow?.getAttribute("data-row-id")).toBe(
      "thread-root:$old-root:example.invalid"
    );
    expect(readyRow?.getAttribute("data-activity-event-id")).toBe(
      "$old-root-latest:example.invalid"
    );
  });

  it("keeps a terminal old-root failure visible without restoring a reply row", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const latestReply = {
      ...message("$failed-root-latest:example.invalid", "reply must remain suppressed"),
      timestamp_ms: 1_800_000_020_000,
      thread_root: "$failed-root:example.invalid"
    };
    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        threadRootOrder={{ kind: "latestReply" }}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key: KEY, generation: 1, items: [latestReply] }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key: KEY,
            projection: {
              root_event_id: "$failed-root:example.invalid",
              activity_event_id: "$failed-root-latest:example.invalid",
              activity_timestamp_ms: 1_800_000_020_000,
              state: { kind: "failed", failure_kind: "notFound" }
            }
          }
        }
      });
    });

    const failed = await screen.findByRole("status");
    const failedRow = failed.closest<HTMLElement>("article");
    expect(failed.textContent).toContain("Thread message is unavailable");
    expect(failedRow?.getAttribute("data-thread-root-projection-state")).toBe("failed");
    expect(failedRow?.getAttribute("data-row-id")).toBe(
      "thread-root:$failed-root:example.invalid"
    );
    expect(screen.queryByText("reply must remain suppressed")).toBeNull();
  });

  it("keeps a Room root summary at its origin and suppresses canonical replies by default", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const paginateBackwards = vi.fn(async () => undefined);
    const rootTimestampMs = 1_800_000_000_000;
    const latestReplyTimestampMs = rootTimestampMs + 60_000;
    const root = {
      ...message("$default-thread-root:example.invalid", "Default root body"),
      timestamp_ms: rootTimestampMs,
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$default-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Default latest reply preview",
        latest_timestamp_ms: latestReplyTimestampMs
      }
    };
    const latestReply = {
      ...message("$default-thread-reply:example.invalid", "Default standalone reply"),
      timestamp_ms: latestReplyTimestampMs,
      thread_root: "$default-thread-root:example.invalid"
    };
    const transport = baseTransport({
      paginateBackwards,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });

    render(
      <TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [root, message("$default-between:example.invalid", "Default between"), latestReply]
          }
        }
      });
    });

    const rootRow = await screen.findByText("Default root body").then((node) =>
      node.closest<HTMLElement>("article")
    );
    expect(rootRow?.getAttribute("data-row-id")).toBe(
      "thread-root:$default-thread-root:example.invalid"
    );
    expect(rootRow?.getAttribute("data-content-event-id")).toBe("$default-thread-root:example.invalid");
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe("$default-thread-root:example.invalid");
    const latestReplyTime = new Intl.DateTimeFormat("en", { timeStyle: "short" }).format(
      new Date(latestReplyTimestampMs)
    );
    expect(rootRow?.textContent).toContain(
      `1 reply · Bob: Default latest reply preview · ${latestReplyTime}`
    );
    expect(screen.queryByText("Default standalone reply")).toBeNull();
    expect(
      Array.from(document.querySelectorAll("article[data-row-id]")).map((row) =>
        row.getAttribute("data-content-event-id")
      )
    ).toEqual(["$default-thread-root:example.invalid", "$default-between:example.invalid"]);
    expect(paginateBackwards).not.toHaveBeenCalled();
  });

  it("keeps the root but hides conversation-start chrome and its summary in thread presentation", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const root = {
      ...message("$thread-root:example.invalid", "Thread root remains visible"),
      thread_summary: {
        reply_count: 2,
        latest_event_id: "$thread-latest:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "latest reply",
        latest_timestamp_ms: 1_800_000_010_000
      }
    };

    render(
      <TimelineView
        presentationContext="thread"
        timelineKey={threadKey}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
      />
    );
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key: threadKey, generation: 1, items: [root] }
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
    });

    expect(await screen.findByText("Thread root remains visible")).not.toBeNull();
    expect(screen.queryByText("Start of conversation")).toBeNull();
    expect(screen.queryByRole("button", { name: /2 replies/i })).toBeNull();
  });

  it("moves one Room thread root and its summary to its latest reply while keeping root actions and timestamps", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onOpenThread = vi.fn();
    const onOpenContextMenu = vi.fn();
    const viewportObservations: Array<{
      roomId: string;
      firstVisibleEventId: string | null;
      lastVisibleEventId: string | null;
    }> = [];
    const observeViewport = vi.fn(
      async (
        roomId: string,
        firstVisibleEventId: string | null,
        lastVisibleEventId: string | null,
        _atBottom: boolean
      ) => {
        viewportObservations.push({ roomId, firstVisibleEventId, lastVisibleEventId });
      }
    );
    const rootTimestampMs = 1_800_000_000_000;
    const replyTimestampMs = rootTimestampMs + 60 * 60 * 1_000;
    const root = {
      ...message("$thread-root:example.invalid", "Original root body"),
      timestamp_ms: rootTimestampMs,
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Latest reply preview",
        latest_timestamp_ms: replyTimestampMs
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Standalone reply body"),
      timestamp_ms: replyTimestampMs,
      thread_root: "$thread-root:example.invalid"
    };
    const rects = {
      "$before:example.invalid": { top: -100, height: 20 },
      "$between:example.invalid": { top: -100, height: 20 },
      "$latest-thread-reply:example.invalid": { top: 20, height: 40 },
      "$after:example.invalid": { top: 700, height: 20 }
    };
    const rectMock = mockTimelineRects(rects, { top: 0, height: 600 });
    const transport = baseTransport({
      observeViewport,
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
        onOpenThread={onOpenThread}
        onOpenContextMenu={onOpenContextMenu}
        threadRootOrder={{ kind: "latestReply" }}
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
              message("$before:example.invalid", "Before"),
              root,
              message("$between:example.invalid", "Between"),
              latestReply,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    const rootRow = await screen.findByText("Original root body").then((node) =>
      node.closest<HTMLElement>("article")
    );
    expect(rootRow).not.toBeNull();
    expect(rootRow?.getAttribute("data-row-id")).toBe(
      "thread-root:$thread-root:example.invalid"
    );
    expect(rootRow?.getAttribute("data-content-event-id")).toBe("$thread-root:example.invalid");
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe(
      "$latest-thread-reply:example.invalid"
    );
    expect(rootRow?.getAttribute("data-event-id")).toBe("$latest-thread-reply:example.invalid");
    expect(rootRow?.textContent).toContain(
      new Intl.DateTimeFormat("en", { timeStyle: "short" }).format(new Date(rootTimestampMs))
    );
    expect(rootRow?.textContent).toContain("1 reply · Bob: Latest reply preview");
    expect(screen.queryByText("Standalone reply body")).toBeNull();
    expect(
      Array.from(document.querySelectorAll("article[data-row-id]")).map((row) =>
        row.getAttribute("data-content-event-id")
      )
    ).toEqual([
      "$before:example.invalid",
      "$between:example.invalid",
      "$thread-root:example.invalid",
      "$after:example.invalid"
    ]);

    fireEvent.click(screen.getByRole("button", { name: /Open thread, 1 reply/ }));
    expect(onOpenThread).toHaveBeenCalledWith("!room:example.invalid", "$thread-root:example.invalid");
    fireEvent.contextMenu(rootRow!);
    expect(onOpenContextMenu).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        kind: "message",
        message: expect.objectContaining({ event_id: "$thread-root:example.invalid" })
      }),
      expect.any(Array)
    );
    await waitFor(() => {
      expect(
        viewportObservations.some(
          ({ roomId, firstVisibleEventId, lastVisibleEventId }) =>
            roomId === "!room:example.invalid" &&
            firstVisibleEventId === "$latest-thread-reply:example.invalid" &&
            lastVisibleEventId === "$latest-thread-reply:example.invalid"
        )
      ).toBe(true);
    });
    rectMock.mockRestore();
  });

  it("keeps a replay-summary root out of the free-scroll anchor while using its activity identity", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onScrollDiagnosticsChange = vi.fn();
    const viewportObservations: Array<{
      firstVisibleEventId: string | null;
      lastVisibleEventId: string | null;
    }> = [];
    const observeViewport = vi.fn(
      async (
        _roomId: string,
        firstVisibleEventId: string | null,
        lastVisibleEventId: string | null,
        _atBottom: boolean
      ) => {
        viewportObservations.push({ firstVisibleEventId, lastVisibleEventId });
      }
    );
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const rootEventId = "$replay-summary-root:example.invalid";
    const firstActivityEventId = "$summary-activity-first:example.invalid";
    const laterActivityEventId = "$summary-activity-later:example.invalid";
    const rootTimestampMs = 1_800_000_000_000;
    const firstActivityTimestampMs = rootTimestampMs + 2_000;
    const laterActivityTimestampMs = rootTimestampMs + 4_000;
    const root = {
      ...message(rootEventId, "Replay summary root"),
      timestamp_ms: rootTimestampMs,
      thread_summary: {
        reply_count: 1,
        latest_event_id: firstActivityEventId,
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Summary-only activity",
        latest_timestamp_ms: firstActivityTimestampMs
      }
    };
    const rootWithLaterSummary = {
      ...root,
      thread_summary: {
        ...root.thread_summary,
        latest_event_id: laterActivityEventId,
        latest_timestamp_ms: laterActivityTimestampMs
      }
    };
    const before = {
      ...message("$before-summary-root:example.invalid", "Before"),
      timestamp_ms: rootTimestampMs + 1_000
    };
    const after = {
      ...message("$after-summary-root:example.invalid", "After"),
      timestamp_ms: rootTimestampMs + 3_000
    };
    const transport = baseTransport({
      observeViewport,
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        threadRootOrder={{ kind: "latestReply" }}
      />
    );
    const { rerender } = render(renderView());

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              first_unread_event_id: firstActivityEventId,
              unread_event_count: 1,
              unread_position: "insideViewport"
            })
          }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key: KEY, generation: 1, items: [before, after] }
        }
      });
      emit({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key: KEY,
            projection: {
              root_event_id: rootEventId,
              activity_event_id: firstActivityEventId,
              activity_timestamp_ms: firstActivityTimestampMs,
              retain_without_reply: true,
              source: { kind: "replayKnown", epoch: 1 },
              state: { kind: "ready", item: root }
            }
          }
        }
      });
    });

    const rootRow = await screen.findByText("Replay summary root").then((node) =>
      node.closest<HTMLElement>("article")
    );
    expect(rootRow?.getAttribute("data-content-event-id")).toBe(rootEventId);
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe(firstActivityEventId);
    expect(
      Array.from(document.querySelectorAll("article[data-row-id]")).map((row) =>
        row.getAttribute("data-row-id")
      )
    ).toEqual([
      "$before-summary-root:example.invalid",
      `thread-root:${rootEventId}`,
      "$after-summary-root:example.invalid"
    ]);
    const unreadMarker = await screen.findByRole("separator", { name: "Unread messages" });
    expect(unreadMarker.nextElementSibling).toBe(rootRow);
    await waitFor(() => {
      expect(
        viewportObservations.some(
          ({ firstVisibleEventId, lastVisibleEventId }) =>
            firstVisibleEventId === "$before-summary-root:example.invalid" &&
            lastVisibleEventId === firstActivityEventId
        )
      ).toBe(true);
    });

    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      rerender(renderView());
    });
    await waitFor(() => expect(timeline.scrollTop).toBe(800));
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ThreadRootProjection: {
            key: KEY,
            projection: {
              root_event_id: rootEventId,
              activity_event_id: laterActivityEventId,
              activity_timestamp_ms: laterActivityTimestampMs,
              retain_without_reply: true,
              source: { kind: "replayKnown", epoch: 2 },
              state: { kind: "ready", item: rootWithLaterSummary }
            }
          }
        }
      });
    });

    await waitFor(() => {
      // The unchanged normal row stays at the same pixel. If the movable
      // summary root were used as the anchor, this would instead become 290.
      expect(timeline.scrollTop).toBe(90);
      expect(screen.getByText("After").closest("article")?.getBoundingClientRect().top).toBe(10);
      expect(
        onScrollDiagnosticsChange.mock.calls.some(
          ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
        )
      ).toBe(true);
      expect(
        viewportObservations.some(
          ({ lastVisibleEventId }) => lastVisibleEventId === laterActivityEventId
        )
      ).toBe(true);
    });
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe(laterActivityEventId);
    rectMock.mockRestore();
  });

  it("uses a non-moving row, never the moved root, when latest-reply placement toggles in free scroll", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onScrollDiagnosticsChange = vi.fn();
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Standalone reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = (threadRootOrder: "rootEvent" | "latestReply") => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        threadRootOrder={{ kind: threadRootOrder }}
      />
    );
    const { rerender } = render(renderView("rootEvent"));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$before:example.invalid", "Before"),
              root,
              message("$between:example.invalid", "Between"),
              latestReply,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    await screen.findByText("Between");
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 190,
      writable: true,
      configurable: true
    });
    // Let first-entry live-edge initialization finish before the test gives
    // the viewport back to a user-controlled free-scroll position.
    act(() => {
      rerender(renderView("rootEvent"));
    });
    await waitFor(() => {
      expect(timeline.scrollTop).toBe(800);
    });
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);

    act(() => {
      rerender(renderView("latestReply"));
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(90);
      expect(
        onScrollDiagnosticsChange.mock.calls.some(
          ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
        )
      ).toBe(true);
    });
    expect(screen.getByText("Between").closest("article")?.getBoundingClientRect().top).toBe(10);
    rectMock.mockRestore();
  });

  it("keeps a committed projection compensation when StrictMode abandons a later render", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    let controls: {
      setOrder: (order: "rootEvent" | "latestReply") => void;
      setShouldSuspend: (shouldSuspend: boolean) => void;
      refresh: () => void;
    } | null = null;
    const suspended = new Promise<never>(() => undefined);
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Standalone reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    function SuspendsAfterTimeline({ shouldSuspend }: { shouldSuspend: boolean }) {
      if (shouldSuspend) {
        throw suspended;
      }
      return null;
    }
    function Harness() {
      const [order, setOrder] = useState<"rootEvent" | "latestReply">("rootEvent");
      const [shouldSuspend, setShouldSuspend] = useState(false);
      const [, setVersion] = useState(0);
      useEffect(() => {
        controls = {
          setOrder,
          setShouldSuspend,
          refresh: () => setVersion((current) => current + 1)
        };
      });
      return (
        <Suspense fallback={null}>
          <TimelineView
            timelineKey={KEY}
            roomId="!room:example.invalid"
            transport={transport}
            onReply={vi.fn()}
            threadRootOrder={{ kind: order }}
          />
          <SuspendsAfterTimeline shouldSuspend={shouldSuspend} />
        </Suspense>
      );
    }

    render(
      <StrictMode>
        <Harness />
      </StrictMode>
    );
    await waitFor(() => expect(controls).not.toBeNull());
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$before:example.invalid", "Before"),
              root,
              message("$between:example.invalid", "Between"),
              latestReply,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    const timeline = await screen.findByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      controls!.refresh();
    });
    await waitFor(() => expect(timeline.scrollTop).toBe(800));
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });

    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });

    // B commits and queues its free-scroll correction. C starts afterwards,
    // but suspends before it can commit; B remains the visible projection.
    act(() => {
      controls!.setOrder("latestReply");
    });
    expect(
      document
        .querySelector('[data-content-event-id="$thread-root:example.invalid"]')
        ?.getAttribute("data-activity-event-id")
    ).toBe("$latest-thread-reply:example.invalid");
    act(() => {
      startTransition(() => {
        controls!.setOrder("rootEvent");
        controls!.setShouldSuspend(true);
      });
    });
    expect(
      document
        .querySelector('[data-content-event-id="$thread-root:example.invalid"]')
        ?.getAttribute("data-activity-event-id")
    ).toBe("$latest-thread-reply:example.invalid");

    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        callback(0);
      }
    });

    expect(timeline.scrollTop).toBe(90);
    rectMock.mockRestore();
  });

  it("does not overwrite a user scroll that happens after projection compensation is queued", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onScrollDiagnosticsChange = vi.fn();
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Standalone reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = (threadRootOrder: "rootEvent" | "latestReply") => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        threadRootOrder={{ kind: threadRootOrder }}
      />
    );
    const { rerender } = render(renderView("rootEvent"));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$before:example.invalid", "Before"),
              root,
              message("$between:example.invalid", "Between"),
              latestReply,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    await screen.findByText("Between");
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      rerender(renderView("rootEvent"));
    });
    await waitFor(() => expect(timeline.scrollTop).toBe(800));
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });

    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    act(() => {
      rerender(renderView("latestReply"));
    });

    // A real user scroll takes ownership while the projection's frame is held.
    timeline.scrollTop = 250;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        callback(0);
      }
    });

    expect(timeline.scrollTop).toBe(250);
    expect(
      onScrollDiagnosticsChange.mock.calls.some(
        ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
      )
    ).toBe(false);
    rectMock.mockRestore();
  });

  it("does not apply queued projection compensation after a jump takes viewport ownership", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    let jumpToLatest: (() => void) | null = null;
    const onScrollDiagnosticsChange = vi.fn();
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    const rectMock = mockPresentationOrderRects(scrollContainerRef);
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Standalone reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = (threadRootOrder: "rootEvent" | "latestReply") => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onRegisterJumpToLatest={(handler) => {
          jumpToLatest = handler;
        }}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        threadRootOrder={{ kind: threadRootOrder }}
      />
    );
    const { rerender } = render(renderView("rootEvent"));

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              message("$before:example.invalid", "Before"),
              root,
              message("$between:example.invalid", "Between"),
              latestReply,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    await screen.findByText("Between");
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      rerender(renderView("rootEvent"));
    });
    await waitFor(() => expect(timeline.scrollTop).toBe(800));
    timeline.scrollTop = 190;
    fireEvent.wheel(timeline, { deltaY: -1 });

    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    act(() => {
      rerender(renderView("latestReply"));
    });

    act(() => {
      jumpToLatest?.();
    });
    expect(timeline.scrollTop).toBe(800);
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        callback(0);
      }
    });

    expect(timeline.scrollTop).toBe(800);
    expect(
      onScrollDiagnosticsChange.mock.calls.some(
        ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
      )
    ).toBe(false);
    rectMock.mockRestore();
  });

  it("renders an unread latest-reply marker before the root block that represents it", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Standalone reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
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
        threadRootOrder={{ kind: "latestReply" }}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key: KEY,
            snapshot: navigationSnapshot({
              first_unread_event_id: "$latest-thread-reply:example.invalid",
              unread_event_count: 1,
              unread_position: "insideViewport"
            })
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
            items: [message("$before:example.invalid", "Before"), root, latestReply]
          }
        }
      });
    });

    const marker = await screen.findByRole("separator", { name: "Unread messages" });
    const rootRow = marker.nextElementSibling;
    expect(rootRow?.getAttribute("data-content-event-id")).toBe("$thread-root:example.invalid");
    expect(rootRow?.getAttribute("data-activity-event-id")).toBe(
      "$latest-thread-reply:example.invalid"
    );
  });

  it("jumps to a moved root by its latest activity identity", async () => {
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    try {
      let emit: (payload: CoreEventPayload) => void = () => undefined;
      const root = {
        ...message("$thread-root:example.invalid", "Thread root"),
        thread_summary: {
          reply_count: 1,
          latest_event_id: "$latest-thread-reply:example.invalid",
          latest_sender: "@bob:example.invalid",
          latest_sender_label: "Bob",
          latest_body_preview: "Latest reply",
          latest_timestamp_ms: 1_800_000_001_000
        }
      };
      const latestReply = {
        ...message("$latest-thread-reply:example.invalid", "Standalone reply"),
        timestamp_ms: 1_800_000_001_000,
        thread_root: "$thread-root:example.invalid"
      };
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
          threadRootOrder={{ kind: "latestReply" }}
        />
      );

      act(() => {
        emit({
          kind: "Timeline",
          event: {
            NavigationUpdated: {
              key: KEY,
              snapshot: navigationSnapshot({
                first_unread_event_id: "$latest-thread-reply:example.invalid",
                unread_event_count: 1,
                unread_position: "belowViewport"
              })
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
              items: [message("$before:example.invalid", "Before"), root, latestReply]
            }
          }
        });
      });

      fireEvent.click(await screen.findByRole("button", { name: /Jump to first unread/ }));
      expect(scrollIntoView).toHaveBeenCalledTimes(1);
      const jumpedRow = scrollIntoView.mock.instances[0] as HTMLElement | undefined;
      expect(jumpedRow?.getAttribute("data-content-event-id")).toBe(
        "$thread-root:example.invalid"
      );
    } finally {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    }
  });

  it("keeps live edge pinned when a summary Set relocates its root block", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const oldRoot = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$older-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Older reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const newRoot = {
      ...oldRoot,
      thread_summary: {
        ...oldRoot.thread_summary,
        latest_event_id: "$newer-thread-reply:example.invalid",
        latest_body_preview: "Newer reply",
        latest_timestamp_ms: 1_800_000_003_000
      }
    };
    const olderReply = {
      ...message("$older-thread-reply:example.invalid", "Older reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
    const newerReply = {
      ...message("$newer-thread-reply:example.invalid", "Newer reply"),
      timestamp_ms: 1_800_000_003_000,
      thread_root: "$thread-root:example.invalid"
    };
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        threadRootOrder={{ kind: "latestReply" }}
      />
    );
    const { rerender } = render(renderView());

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [
              oldRoot,
              olderReply,
              message("$between:example.invalid", "Between"),
              newerReply
            ]
          }
        }
      });
    });

    await screen.findByText("Thread root");
    const timeline = screen.getByTestId("timeline-view");
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 1_200, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      rerender(renderView());
    });
    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1_000);
    });

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [{ Set: { index: 0, item: newRoot } }]
          }
        }
      });
    });

    await waitFor(() => {
      expect(timeline.scrollTop).toBe(1_000);
    });
  });

  it("falls back to the virtual height model when a projection anchor unmounts", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onScrollDiagnosticsChange = vi.fn();
    const rowHeight = 72;
    const normalCount = 620;
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$older-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "Older reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const updatedRoot = {
      ...root,
      thread_summary: {
        ...root.thread_summary,
        latest_event_id: "$newer-thread-reply:example.invalid",
        latest_body_preview: "Newer reply",
        latest_timestamp_ms: 1_800_000_003_000
      }
    };
    const olderReply = {
      ...message("$older-thread-reply:example.invalid", "Older reply"),
      timestamp_ms: 1_800_000_001_000,
      thread_root: "$thread-root:example.invalid"
    };
    const newerReply = {
      ...message("$newer-thread-reply:example.invalid", "Newer reply"),
      timestamp_ms: 1_800_000_003_000,
      thread_root: "$thread-root:example.invalid"
    };
    const normals = Array.from({ length: normalCount }, (_, index) =>
      message(`$normal${index}:example.invalid`, `Normal ${index}`)
    );
    const scrollContainerRef: { current: HTMLElement | null } = { current: null };
    let rootMovedToNewReply = false;
    const rectMock = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        const timeline = scrollContainerRef.current;
        if (this.getAttribute("data-testid") === "timeline-view") {
          return {
            x: 0,
            y: 0,
            top: 0,
            left: 0,
            right: 0,
            width: 0,
            height: 200,
            bottom: 200,
            toJSON: () => ({})
          } as DOMRect;
        }
        const row = this.matches(".timeline-item-frame")
          ? this
          : this.closest<HTMLElement>(".timeline-item-frame");
        const rowId =
          row?.dataset["frameItemId"] ??
          row?.querySelector<HTMLElement>("[data-item-id]")?.dataset["itemId"] ??
          "";
        let rowIndex = -1;
        if (rowId.startsWith("date-divider:")) {
          rowIndex = 0;
        } else if (rowId === "thread-root:$thread-root:example.invalid") {
          rowIndex = rootMovedToNewReply ? normalCount + 1 : 1;
        } else {
          const match = /^\$normal(\d+):example\.invalid$/.exec(rowId);
          if (match) {
            rowIndex = Number(match[1]) + (rootMovedToNewReply ? 1 : 2);
          }
        }
        const top =
          rowIndex >= 0 ? rowIndex * rowHeight - (timeline?.scrollTop ?? 0) : 0;
        return {
          x: 0,
          y: top,
          top,
          left: 0,
          right: 0,
          width: 0,
          height: rowIndex >= 0 ? rowHeight : 0,
          bottom: top + (rowIndex >= 0 ? rowHeight : 0),
          toJSON: () => ({})
        } as DOMRect;
      });
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const renderView = () => (
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onScrollDiagnosticsChange={onScrollDiagnosticsChange}
        threadRootOrder={{ kind: "latestReply" }}
      />
    );
    const { rerender } = render(renderView());

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: KEY,
            generation: 1,
            items: [root, olderReply, ...normals, newerReply]
          }
        }
      });
    });

    await screen.findByText("Thread root");
    const timeline = screen.getByTestId("timeline-view");
    scrollContainerRef.current = timeline;
    Object.defineProperty(timeline, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(timeline, "scrollHeight", { value: 50_000, configurable: true });
    Object.defineProperty(timeline, "scrollTop", {
      value: 0,
      writable: true,
      configurable: true
    });
    act(() => {
      rerender(renderView());
    });
    expect(timeline.getAttribute("data-virtualized")).toBe("true");

    // The previous presentation puts Normal 300 after a date divider and the
    // root block. Its first-visible offset is +10px.
    timeline.scrollTop = 302 * rowHeight - 10;
    fireEvent.wheel(timeline, { deltaY: -1 });
    fireEvent.scroll(timeline);
    await waitFor(() => {
      expect(
        document.querySelector('[data-content-event-id="$normal300:example.invalid"]')
      ).not.toBeNull();
    });
    vi.useFakeTimers();
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrameId = 0;
    let executedFrameCount = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      nextFrameId += 1;
      frames.set(nextFrameId, callback);
      return nextFrameId;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((frameId) => {
      frames.delete(frameId);
    });
    act(() => {
      emit({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key: KEY,
            generation: 1,
            batch_id: 1,
            diffs: [{ Set: { index: 0, item: updatedRoot } }]
          }
        }
      });
    });
    rootMovedToNewReply = true;
    const transactionFrameScheduled = frames.size > 0;

    // Model a virtual-window turnover between commit and the coalesced frame:
    // the stable anchor is no longer mounted, so DOM restoration must fail and
    // the height-model offset is the only valid correction path.
    document
      .querySelector('[data-content-event-id="$normal300:example.invalid"]')
      ?.closest(".timeline-item-frame")
      ?.remove();
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      for (const callback of queued) {
        executedFrameCount += 1;
        callback(0);
      }
    });

    expect({
      transactionFrameScheduled,
      executedFrameCount: executedFrameCount > 0,
      projectionWriteRecorded: onScrollDiagnosticsChange.mock.calls.some(
        ([diagnostics]) => diagnostics.scrollWrites.projectionCompensation > 0
      ),
      scrollTop: timeline.scrollTop
    }).toEqual({
      transactionFrameScheduled: true,
      executedFrameCount: true,
      projectionWriteRecorded: true,
      scrollTop: 301 * rowHeight - 10
    });
    rectMock.mockRestore();
  });

  it("does not reorder Thread timeline rows when latest placement is enabled", async () => {
    const threadKey = threadTimelineKey(
      "@alice:example.invalid",
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 1,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: null,
        latest_body_preview: "Latest reply",
        latest_timestamp_ms: 1_800_000_001_000
      }
    };
    const latestReply = {
      ...message("$latest-thread-reply:example.invalid", "Thread reply"),
      thread_root: "$thread-root:example.invalid"
    };
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
        threadRootOrder={{ kind: "latestReply" }}
      />
    );

    act(() => {
      emit({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key: threadKey,
            generation: 1,
            items: [root, latestReply]
          }
        }
      });
    });

    await screen.findByText("Thread reply");
    expect(
      Array.from(document.querySelectorAll("article[data-row-id]")).map((row) =>
        row.getAttribute("data-content-event-id")
      )
    ).toEqual(["$thread-root:example.invalid", "$latest-thread-reply:example.invalid"]);
  });

  it("shows new thread replies on the matching root row without moving timeline rows", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const onOpenThread = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const root = {
      ...message("$thread-root:example.invalid", "Thread root"),
      thread_summary: {
        reply_count: 4,
        latest_event_id: "$latest-thread-reply:example.invalid",
        latest_sender: "@bob:example.invalid",
        latest_sender_label: "Bob",
        latest_body_preview: "latest reply",
        latest_timestamp_ms: 1_800_000_000_500
      }
    };

    render(
      <TimelineView
        timelineKey={KEY}
        roomId="!room:example.invalid"
        transport={transport}
        onReply={vi.fn()}
        onOpenThread={onOpenThread}
        threadAttention={{
          rootEventId: "$thread-root:example.invalid",
          notificationCount: 2,
          highlightCount: 0,
          liveEventMarkerCount: 2
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
            generation: 1,
            items: [
              message("$before:example.invalid", "Before"),
              root,
              message("$after:example.invalid", "After")
            ]
          }
        }
      });
    });

    const newReplies = await screen.findByRole("button", { name: /View new replies · 2/ });
    expect(newReplies.closest("[data-event-id]")?.getAttribute("data-event-id")).toBe(
      "$thread-root:example.invalid"
    );
    const eventOrder = Array.from(document.querySelectorAll("article[data-event-id]")).map(
      (row) => row.getAttribute("data-event-id")
    );
    expect(eventOrder).toEqual([
      "$before:example.invalid",
      "$thread-root:example.invalid",
      "$after:example.invalid"
    ]);

    fireEvent.click(newReplies);
    expect(onOpenThread).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$thread-root:example.invalid"
    );
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

  it("emits fixed private-data-free diagnostics when a room-key request fails", async () => {
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
        can_request_keys: true
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

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "e2ee.room_key",
          message: "operation=request_keys stage=failed kind=transport"
        })
      );
    });
    expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "e2ee.room_key",
        message: "operation=request_keys stage=request"
      })
    );

    const diagnosticText = JSON.stringify(onDiagnosticLogEntry.mock.calls);
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

  it("shows visible copy controls in the message source dialog", () => {
    const source: TimelineMessageSource = {
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
    };

    render(<MessageSourceDialog source={source} onClose={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Copy event ID" }).textContent).toContain(
      "Copy event ID"
    );
    expect(
      screen.getByRole("button", { name: "Copy original event source" }).textContent
    ).toContain("Copy original event source");
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

  it("renders plain-text URLs as anchors from Rust-projected link ranges", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      }
    });
    const text = "Check https://example.com/page and https://example.com/page out";
    const item: TimelineItem = {
      ...message("$url:example.invalid", text),
      link_ranges: [
        {
          url: "https://example.com/page",
          start_utf16: 6,
          end_utf16: 30
        },
        {
          url: "https://example.com/page",
          start_utf16: 35,
          end_utf16: 59
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

    const links = await screen.findAllByRole("link", { name: "https://example.com/page" });
    expect(links).toHaveLength(2);
    for (const link of links) {
      expect(link.getAttribute("href")).toBe("https://example.com/page");
      expect(link.getAttribute("target")).toBe("_blank");
    }

    fireEvent.click(links[0]);
    await waitFor(() => {
      expect(openExternalHttpUrl).toHaveBeenCalledWith("https://example.com/page");
    });
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

  it("renders link preview cards as clickable anchors", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const hideLinkPreview = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      hideLinkPreview
    });
    const item: TimelineItem = {
      ...message("$preview:example.invalid", "look at this"),
      link_previews: [
        {
          url: "https://example.com/article",
          title: "An article",
          state: "ready"
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

    const card = await screen.findByRole("link", { name: /An article/ });
    expect(card.getAttribute("href")).toBe("https://example.com/article");
    fireEvent.click(card);
    await waitFor(() => {
      expect(openExternalHttpUrl).toHaveBeenCalledWith("https://example.com/article");
    });

    const hide = screen.getByRole("button", { name: "Hide preview" });
    fireEvent.click(hide);
    await waitFor(() => {
      expect(hideLinkPreview).toHaveBeenCalledWith("!room:example.invalid", "$preview:example.invalid");
    });
  });

  it("emits private-data-free diagnostics when viewport pending link previews load", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const loadLinkPreviews = vi.fn(async () => undefined);
    const onDiagnosticLogEntry = vi.fn();
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      loadLinkPreviews
    });
    const item: TimelineItem = {
      ...message("$pending-preview:example.invalid", "look at https://secret.example/article"),
      link_previews: [
        {
          url: "https://secret.example/article",
          state: "pending"
        }
      ]
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
          items: [item]
        }
      }
    });

    await waitFor(() => {
      expect(loadLinkPreviews).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$pending-preview:example.invalid"
      );
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.preview",
          message: "kind=room stage=request trigger=viewport_pending pending=1"
        })
      );
    });

    emit({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key: KEY,
          generation: 1,
          batch_id: 1,
          diffs: [
            {
              Set: {
                index: 0,
                item: {
                  ...item,
                  link_previews: [
                    {
                      url: "https://secret.example/article",
                      title: "Loaded",
                      state: "ready"
                    }
                  ]
                }
              }
            }
          ]
        }
      }
    });

    await waitFor(() => {
      expect(onDiagnosticLogEntry).toHaveBeenCalledWith(
        expect.objectContaining({
          source: "timeline.preview",
          message: "kind=room stage=update items=1 pending=0 loading=0 ready=1 failed=0"
        })
      );
    });

    const diagnosticText = onDiagnosticLogEntry.mock.calls
      .map(([entry]) => `${entry.source} ${entry.message}`)
      .join("\n");
    expect(diagnosticText).not.toContain("$pending-preview");
    expect(diagnosticText).not.toContain("secret.example");
  });

  it("limits initial link preview requests to the current viewport window", async () => {
    let emit: (payload: CoreEventPayload) => void = () => undefined;
    const loadLinkPreviews = vi.fn(async () => undefined);
    const transport = baseTransport({
      listenCoreEvents(nextListener) {
        emit = nextListener;
        return () => undefined;
      },
      loadLinkPreviews
    });
    const items = Array.from({ length: 40 }, (_, index) => ({
      ...message(`$preview-window-${index}`, `Preview row ${index}`),
      link_previews: [
        {
          url: `https://example.invalid/preview-window-${index}`,
          state: "pending" as const
        }
      ]
    }));

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
          items
        }
      }
    });

    await waitFor(() => {
      expect(loadLinkPreviews).toHaveBeenCalledWith(
        "!room:example.invalid",
        "$preview-window-0"
      );
    });
    expect(loadLinkPreviews).not.toHaveBeenCalledWith(
      "!room:example.invalid",
      "$preview-window-39"
    );
    expect(loadLinkPreviews.mock.calls.length).toBeLessThan(items.length);
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
          sender_preview: ["@bob:example.invalid"]
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
          typing_user_ids: []
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
});
