// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { TimelineView, clearTimelineViewportSessionMemoryForTests } from "../components/TimelineView";
import { KEY, baseTransport, message } from "../components/timelineViewTestSupport";
import type { CoreEventPayload } from "../domain/coreEvents";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../backend/appRuntime", async () => {
  const { TauriDesktopApi } = await import("../backend/client");
  return { api: new TauriDesktopApi() };
});

afterEach(() => {
  cleanup();
  clearTimelineViewportSessionMemoryForTests();
  vi.unstubAllGlobals();
  vi.resetAllMocks();
});

it.each([true, false])("shows reader timestamps on hover (committed source: %s)", async (hasSource) => {
  const timestamp = 1_800_000_000_000;
  vi.stubGlobal("__TAURI_INTERNALS__", {});
  let received = false;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "subscribe_receipt_reader") {
      const source = (args as { source: { projection_request_id: Record<string, unknown> } }).source;
      // Mirrors the strict DecimalRequestId decoder in koushi-protocol/view.rs.
      if (typeof source.projection_request_id.connection_id !== "string" ||
          typeof source.projection_request_id.sequence !== "string") {
        throw new Error("invalid type: expected a decimal string");
      }
      return "1";
    }
    if (command === "receive_receipt_reader") {
      if (received) return new Promise(() => undefined);
      received = true;
      return { kind: "model", scope: "1", revision: "1", model: {
        kind: "readerReady", total_count: 1, start: 0, window_sequence: "0",
        rows: [{ user_id: "@reader:example.invalid", display_label: "Reader One",
          original_display_label: "Reader One", initials: "RO", timestamp: { unix_ms: String(timestamp), locale: "en" }, avatar: null }]
      } };
    }
    return undefined;
  });
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid"
    transport={baseTransport({ listenCoreEvents(listener) { emit = listener; return () => undefined; } })}
    liveSignals={{ presence: {}, rooms: { "!room:example.invalid": {
      fully_read_event_id: null, typing_user_ids: [], typing_users: [],
      receipts_by_event: { "$seen": { total_count: 1, overflow_count: 0, readers: [{
        user_id: "@reader:example.invalid", display_name: "Reader One",
        original_display_label: "Reader One", avatar: null, timestamp_ms: timestamp
      }] } }
    } } }} onReply={vi.fn()} />);
  act(() => emit({ kind: "Timeline", event: { InitialItems: {
    request_id: hasSource ? { connection_id: 1, sequence: 2 } : null, key: KEY, generation: 3,
    items: [message("$seen", "Synthetic message")]
  } } }));
  await waitFor(() => expect(document.querySelector(".message-receipts")).not.toBeNull());
  fireEvent.mouseEnter(document.querySelector(".message-receipts")!);
  expect((await screen.findByRole("listitem")).textContent).toContain("Reader One");
  const date = new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" }).format(timestamp);
  await waitFor(() => expect(screen.getByRole("listitem").textContent).toContain(date));
  if (!hasSource) {
    expect(invoke).not.toHaveBeenCalledWith("subscribe_receipt_reader", expect.anything());
    return;
  }
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("ack_receipt_reader", { scope: "1", revision: "1" }));
  expect(invoke).toHaveBeenCalledWith("subscribe_receipt_reader", {
    source: { key: KEY, projection_request_id: { connection_id: "1", sequence: "2" },
      generation: "3", event_id: "$seen" }, start: 0, limit: 256
  });
});
