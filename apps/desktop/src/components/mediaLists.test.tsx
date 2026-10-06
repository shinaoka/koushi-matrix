// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  MessageArticle,
  PinnedEventsList,
  PinnedMessagesEntry,
  ScheduledMessagesList
} from "./mediaLists";

afterEach(cleanup);

describe("people-facing media list labels", () => {
  it("renders pinned-event sender_label and hides the raw sender id", () => {
    render(
      <PinnedEventsList
        roomId="!room:example.invalid"
        pinnedEvents={[
          {
            event_id: "$pinned:example.invalid",
            sender: "@private:example.invalid",
            sender_label: "Pinned Alias",
            body_preview: "Pinned body",
            redacted: false
          }
        ]}
        onUnpin={() => undefined}
      />
    );

    expect(screen.getByText("Pinned Alias")).toBeTruthy();
    expect(screen.queryByText("@private:example.invalid")).toBeNull();
  });

  it("navigates by the owning room and exact event ID from a pinned row", () => {
    const onOpen = vi.fn();
    render(
      <PinnedEventsList
        roomId="!room:example.invalid"
        pinnedEvents={[
          {
            event_id: "$pinned:example.invalid",
            sender: "@private:example.invalid",
            sender_label: "Pinned Alias",
            body_preview: "Pinned body",
            redacted: false,
            timestamp_ms: 1_800_000_000_000,
            state: "ready",
            thread_root_event_id: null
          }
        ]}
        onOpen={onOpen}
        onUnpin={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Pinned body/ }));

    expect(onOpen).toHaveBeenCalledWith(
      "!room:example.invalid",
      "$pinned:example.invalid",
      null
    );
  });

  it("opens the pinned panel from the compact timeline entry", () => {
    const onOpen = vi.fn();
    render(<PinnedMessagesEntry count={3} onOpen={onOpen} />);

    fireEvent.click(screen.getByRole("button", { name: "Pinned · 3" }));

    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("renders a profile label in fixture message metadata and fails closed when absent", () => {
    const message = {
      room_id: "!room:example.invalid",
      event_id: "$message:example.invalid",
      sender: "@private:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      body: "Body",
      attachment_filename: null,
      reply_count: 0
    };
    const props = {
      currentUserId: null,
      message,
      highlights: [],
      onOpenThread: vi.fn(),
      onRedactMessage: vi.fn(),
      isIgnored: false
    };
    const { rerender } = render(
      <MessageArticle
        {...props}
        profileUsers={{
          [message.sender]: {
            user_id: message.sender,
            display_name: "Profile Name",
            display_label: "Local Alias",
            original_display_label: "Profile Name",
            mention_search_terms: [],
            avatar: null
          }
        }}
      />
    );

    expect(screen.getByText("Local Alias")).toBeTruthy();
    expect(screen.queryByText(message.sender)).toBeNull();

    rerender(<MessageArticle {...props} profileUsers={{}} />);
    expect(screen.getByText("Unknown user")).toBeTruthy();
    expect(screen.queryByText(message.sender)).toBeNull();
  });
});

describe("scheduled message editing", () => {
  it("edits the body and time through the shared composer controls", () => {
    const onReschedule = vi.fn();
    render(
      <ScheduledMessagesList
        capability="localFallback"
        items={[
          {
            scheduled_id: "scheduled-1",
            room_id: "!room:example.invalid",
            body: "Original body",
            send_at_ms: Date.UTC(2030, 0, 1, 12, 0),
            handle: { kind: "local" }
          }
        ]}
        onCancel={() => undefined}
        onReschedule={onReschedule}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit scheduled send" }));
    const body = screen.getByRole("textbox", { name: "Scheduled message" });
    expect(body.textContent).toBe("Original body");
    body.textContent = "Edited **body**";
    fireEvent.input(body);
    expect(screen.getByRole("button", { name: "Save scheduled send" }).textContent).toBe("Save");
    fireEvent.click(screen.getByRole("button", { name: "Save scheduled send" }));

    expect(onReschedule).toHaveBeenCalledWith("scheduled-1", "Edited **body**", expect.any(Number));
  });
});
