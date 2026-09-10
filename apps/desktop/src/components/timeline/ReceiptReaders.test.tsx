// @vitest-environment jsdom

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  subscribeReceiptReader: vi.fn(),
  receiveReceiptReader: vi.fn(),
  ackReceiptReader: vi.fn(),
  updateReceiptReaderWindow: vi.fn(),
  observeReceiptReaderAvatars: vi.fn().mockResolvedValue(undefined),
  readReceiptReaderResource: vi.fn(),
  closeReceiptReader: vi.fn().mockResolvedValue(undefined)
}));

vi.mock("../../backend/appRuntime", () => ({ api }));

import { ReceiptReaders } from "./ReceiptReaders";
import type { ReceiptSourceRef } from "../../domain/coreEvents";

function source(account: string, generation: string): ReceiptSourceRef {
  return {
    key: {
      account_key: account,
      kind: { Room: { room_id: "!room:example.org" } }
    },
    projection_request_id: { connection_id: "1", sequence: "2" },
    generation,
    event_id: "$event:example.org"
  };
}

function renderReaders(receiptSource: ReceiptSourceRef) {
  return render(
    <ReceiptReaders
      overflowCount={0}
      receipts={[]}
      source={receiptSource}
      totalCount={1}
    />
  );
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  api.subscribeReceiptReader.mockReset();
  api.receiveReceiptReader.mockReset();
  api.receiveReceiptReader.mockImplementation(() => new Promise(() => undefined));
  api.ackReceiptReader.mockReset();
  api.updateReceiptReaderWindow.mockReset();
  api.observeReceiptReaderAvatars.mockReset().mockResolvedValue(undefined);
  api.readReceiptReaderResource.mockReset();
  api.closeReceiptReader.mockReset().mockResolvedValue(undefined);
});

describe("ReceiptReaders", () => {
  it("reports acknowledged visible rows and bounded nearby IDs without reopening on summary updates", async () => {
    const first = source("@a:example.org", "3");
    let offset = 0;
    let resized: (() => void) | undefined;
    const observeResize = vi.fn();
    const disconnectResize = vi.fn();
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { resized = callback; }
      observe = observeResize;
      disconnect = disconnectResize;
    });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("receipt-tooltip")) return new DOMRect(0, 0, 260, 100);
      if (this.classList.contains("receipt-reader-row") && this.hasAttribute("aria-posinset")) {
        return new DOMRect(0, (Number(this.getAttribute("aria-posinset")) - 1) * 20 - offset, 260, 20);
      }
      return new DOMRect(0, 0, 0, 0);
    });
    const rows = Array.from({ length: 20 }, (_, index) => ({
      user_id: `@reader${index}:example.org`, display_label: `Reader ${index}`,
      original_display_label: `Reader ${index}`, initials: "R", timestamp: null,
      avatar: { kind: "notRequested" }
    }));
    let acknowledge: (() => void) | undefined;
    api.subscribeReceiptReader.mockResolvedValue("scope-a");
    api.ackReceiptReader.mockImplementationOnce(() => new Promise<void>((resolve) => { acknowledge = resolve; }));
    api.receiveReceiptReader.mockResolvedValueOnce({
      kind: "model", scope: "scope-a", revision: "7", model: {
        kind: "readerReady", source: first, total_count: 20, start: 0, rows,
        window_sequence: "0", source_revision: "2", dependency_revision: "1",
        resolved_anchor: { kind: "notRequested" }
      }
    }).mockImplementation(() => new Promise(() => undefined));
    const view = renderReaders(first);
    fireEvent.focus(screen.getByLabelText("Read by 1"));
    const popup = await screen.findByRole("dialog");
    await waitFor(() => expect(api.ackReceiptReader).toHaveBeenCalled());
    expect(api.observeReceiptReaderAvatars).not.toHaveBeenCalled();
    await act(async () => { acknowledge?.(); });
    await waitFor(() => expect(api.observeReceiptReaderAvatars).toHaveBeenCalledWith("scope-a", {
      installed_revision: "7", sequence: "1",
      visible_user_ids: rows.slice(0, 5).map((row) => row.user_id),
      prefetch_user_ids: rows.slice(5, 13).map((row) => row.user_id)
    }));
    offset = 100;
    fireEvent.scroll(popup);
    await waitFor(() => {
      const request = api.observeReceiptReaderAvatars.mock.lastCall?.[1];
      expect(request.visible_user_ids).toEqual(rows.slice(5, 10).map((row) => row.user_id));
      expect(request.prefetch_user_ids).toHaveLength(8);
      expect(BigInt(request.sequence)).toBeGreaterThan(1n);
    });
    offset = 120;
    fireEvent(window, new Event("resize"));
    await waitFor(() => expect(api.observeReceiptReaderAvatars.mock.lastCall?.[1].visible_user_ids)
      .toEqual(rows.slice(6, 11).map((row) => row.user_id)));
    expect(observeResize).toHaveBeenCalledWith(popup);
    expect(observeResize).toHaveBeenCalledWith(screen.getAllByRole("listitem")[0]);
    offset = 140;
    resized?.();
    await waitFor(() => expect(api.observeReceiptReaderAvatars.mock.lastCall?.[1].visible_user_ids)
      .toEqual(rows.slice(7, 12).map((row) => row.user_id)));
    view.rerender(<ReceiptReaders overflowCount={0} receipts={[]} source={first} totalCount={2} />);
    expect(api.subscribeReceiptReader).toHaveBeenCalledTimes(1);
    const calls = api.observeReceiptReaderAvatars.mock.calls.length;
    offset = 300;
    fireEvent.scroll(popup);
    view.unmount();
    await waitFor(() => expect(api.closeReceiptReader).toHaveBeenCalledWith("scope-a"));
    expect(disconnectResize).toHaveBeenCalled();
    await act(async () => { await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); });
    fireEvent.scroll(popup);
    expect(api.observeReceiptReaderAvatars).toHaveBeenCalledTimes(calls);
  });
  it("requests the next bounded window from the installed revision", async () => {
    const first = source("@a:example.org", "3");
    const rows = Array.from({ length: 256 }, (_, index) => ({
      user_id: `@reader${index}:example.org`,
      display_label: `Reader ${index}`,
      original_display_label: `Reader ${index}`,
      initials: "R",
      timestamp: null,
      avatar: null
    }));
    api.subscribeReceiptReader.mockResolvedValue("scope-a");
    api.receiveReceiptReader
      .mockResolvedValueOnce({
        kind: "model",
        scope: "scope-a",
        revision: "7",
        model: {
          kind: "readerReady",
          source: first,
          total_count: 300,
          start: 0,
          rows,
          window_sequence: "0",
          source_revision: "2",
          dependency_revision: "1",
          resolved_anchor: { kind: "notRequested" }
        }
      })
      .mockImplementation(() => new Promise(() => undefined));
    const view = renderReaders(first);

    fireEvent.focus(screen.getByLabelText("Read by 1"));
    const popup = await screen.findByRole("dialog");
    await waitFor(() => expect(api.ackReceiptReader).toHaveBeenCalledWith("scope-a", "7"));
    Object.defineProperties(popup, {
      clientHeight: { configurable: true, value: 100 },
      scrollHeight: { configurable: true, value: 500 },
      scrollTop: { configurable: true, value: 400 }
    });
    fireEvent.scroll(popup);

    await waitFor(() =>
      expect(api.updateReceiptReaderWindow).toHaveBeenCalledWith("scope-a", {
        installed_revision: "7",
        sequence: "1",
        target: { kind: "index", start: "256" },
        limit: 256
      })
    );
    view.unmount();
  });

  it("uses a bounded keyboard window request for Home/End navigation", async () => {
    const first = source("@a:example.org", "3");
    api.subscribeReceiptReader.mockResolvedValue("scope-a");
    api.receiveReceiptReader
      .mockResolvedValueOnce({
        kind: "model",
        scope: "scope-a",
        revision: "7",
        model: {
          kind: "readerReady",
          source: first,
          total_count: 300,
          start: 0,
          rows: [
            {
              user_id: "@reader0:example.org",
              display_label: "Reader 0",
              original_display_label: "Reader 0",
              initials: "R",
              timestamp: null,
              avatar: null
            },
            {
              user_id: "@reader1:example.org",
              display_label: "Reader 1",
              original_display_label: "Reader 1",
              initials: "R",
              timestamp: null,
              avatar: null
            }
          ],
          window_sequence: "0",
          source_revision: "2",
          dependency_revision: "1",
          resolved_anchor: { kind: "notRequested" }
        }
      })
      .mockImplementation(() => new Promise(() => undefined));
    const view = renderReaders(first);

    fireEvent.focus(screen.getByLabelText("Read by 1"));
    const row = (await screen.findAllByRole("listitem"))[0];
    expect(row.textContent).toContain("Reader 0");
    await waitFor(() => expect(api.ackReceiptReader).toHaveBeenCalledWith("scope-a", "7"));
    row.focus();
    fireEvent.keyDown(row, { key: "End" });

    await waitFor(() =>
      expect(api.updateReceiptReaderWindow).toHaveBeenCalledWith("scope-a", {
        installed_revision: "7",
        sequence: "1",
        target: { kind: "index", start: "44" },
        limit: 256
      })
    );
    expect(row.getAttribute("aria-setsize")).toBe("300");
    expect(row.getAttribute("aria-posinset")).toBe("1");
    fireEvent.click(screen.getByRole("button", { name: "Close dialog or menu" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    view.unmount();
  });

  it("reads ready avatar bytes through the installed scope after ACK", async () => {
    const first = source("@a:example.org", "3");
    const sourceRef = "avatar/0000000000000001";
    api.subscribeReceiptReader.mockResolvedValue("scope-a");
    api.receiveReceiptReader
      .mockResolvedValueOnce({
        kind: "model",
        scope: "scope-a",
        revision: "7",
        model: {
          kind: "readerReady",
          source: first,
          total_count: 1,
          start: 0,
          rows: [
            {
              user_id: "@reader:example.org",
              display_label: "Reader",
              original_display_label: "Reader",
              initials: "R",
              timestamp: null,
              avatar: {
                kind: "ready",
                source_ref: sourceRef,
                width: 1,
                height: 1,
                mime_type: "image/gif"
              }
            }
          ],
          window_sequence: "0",
          source_revision: "2",
          dependency_revision: "1",
          resolved_anchor: { kind: "notRequested" }
        }
      })
      .mockImplementation(() => new Promise(() => undefined));
    api.readReceiptReaderResource.mockResolvedValue({
      bytes: [71, 73, 70, 56, 57, 97],
      mime_type: "image/gif"
    });
    const view = renderReaders(first);

    fireEvent.focus(screen.getByLabelText("Read by 1"));
    await waitFor(() => expect(api.ackReceiptReader).toHaveBeenCalledWith("scope-a", "7"));
    await waitFor(() =>
      expect(api.readReceiptReaderResource).toHaveBeenCalledWith("scope-a", "7", sourceRef)
    );
    view.unmount();
  });

  it("shows a bounded loading state and terminal failure without acknowledging", async () => {
    const first = source("@a:example.org", "3");
    api.subscribeReceiptReader.mockResolvedValue("scope-a");
    api.receiveReceiptReader
      .mockResolvedValueOnce({
        kind: "model",
        scope: "scope-a",
        revision: "1",
        model: { kind: "readerLoading", source: first }
      })
      .mockResolvedValueOnce({ kind: "retired", scope: "scope-a", reason: "capacity" });
    const view = renderReaders(first);

    fireEvent.focus(screen.getByLabelText("Read by 1"));
    await waitFor(() => expect(api.receiveReceiptReader).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("status")).toBeTruthy();
    expect(api.ackReceiptReader).not.toHaveBeenCalled();
    view.unmount();
  });

  it("closes the old scope and subscribes to a changed source while open", async () => {
    api.subscribeReceiptReader.mockResolvedValueOnce("scope-a").mockResolvedValueOnce("scope-b");
    const first = source("@a:example.org", "3");
    const second = source("@b:example.org", "4");
    const view = renderReaders(first);

    fireEvent.focus(screen.getByLabelText("Read by 1"));
    await waitFor(() => expect(api.subscribeReceiptReader).toHaveBeenCalledWith(first, 0, 256));

    view.rerender(
      <ReceiptReaders
        overflowCount={0}
        receipts={[]}
        source={second}
        totalCount={1}
      />
    );

    await waitFor(() => expect(api.closeReceiptReader).toHaveBeenCalledWith("scope-a"));
    await waitFor(() => expect(api.subscribeReceiptReader).toHaveBeenCalledWith(second, 0, 256));
  });
});
