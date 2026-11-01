// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ThreadsListView } from "./ThreadsListView";

afterEach(cleanup);

describe("ThreadsListView", () => {
  it("keeps a known unread thread openable when metadata loading fails", () => {
    const onOpenThread = vi.fn();
    render(
      <ThreadsListView
        scope={{ kind: "room", room_id: "!room:example.invalid" }}
        threadsList={{
          kind: "failed", room_id: "!room:example.invalid", request_id: 1,
          failure_kind: "network",
          items: [{
            room_id: "!room:example.invalid", root_event_id: "$known:example.invalid",
            root_sender: "", root_sender_label: null, root_body_preview: null,
            root_timestamp_ms: null, latest_event_id: null, latest_sender: null,
            latest_sender_label: null, latest_body_preview: null,
            latest_timestamp_ms: null, reply_count: 0, unread_count: 1
          }]
        }}
        onClose={() => undefined}
        onOpenThread={onOpenThread}
        onPaginate={() => undefined}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Open unread thread|未読スレッドを開く/ }));
    expect(onOpenThread).toHaveBeenCalledWith("!room:example.invalid", "$known:example.invalid", "existingThread");
  });

  it("lets users open a counted root while its metadata page is pending", () => {
    const onOpenThread = vi.fn();
    render(
      <ThreadsListView
        scope={{ kind: "room", room_id: "!room:example.invalid" }}
        threadsList={{
          kind: "open", room_id: "!room:example.invalid", request_id: 1,
          items: [{
            room_id: "!room:example.invalid", root_event_id: "$older:example.invalid",
            root_sender: "", root_sender_label: null, root_body_preview: null,
            root_timestamp_ms: null, latest_event_id: null, latest_sender: null,
            latest_sender_label: null, latest_body_preview: null,
            latest_timestamp_ms: null, reply_count: 0, unread_count: 3
          }],
          is_paginating: false, end_reached: false
        }}
        onClose={() => undefined}
        onOpenThread={onOpenThread}
        onPaginate={() => undefined}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /Open unread thread|未読スレッドを開く/ }));
    expect(onOpenThread).toHaveBeenCalledWith("!room:example.invalid", "$older:example.invalid", "existingThread");
    expect(screen.getByLabelText(/3/)).toBeTruthy();
  });
  it("uses friendly fallbacks instead of raw Matrix ids for sender metadata", () => {
    render(
      <ThreadsListView
        scope={{ kind: "room", room_id: "!room:example.invalid" }}
        threadsList={{
          kind: "open",
          room_id: "!room:example.invalid",
          request_id: 1,
          items: [
            {
              room_id: "!room:example.invalid",
              root_event_id: "$root:example.invalid",
              root_sender: "@private-root:example.invalid",
              root_sender_label: null,
              root_body_preview: "Root",
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: "$reply:example.invalid",
              latest_sender: "@private-reply:example.invalid",
              latest_sender_label: null,
              latest_body_preview: "Reply",
              latest_timestamp_ms: 1_800_000_000_100,
              reply_count: 1,
              unread_count: 0,
            }
          ],
          is_paginating: false,
          end_reached: true
        }}
        onClose={() => undefined}
        onOpenThread={() => undefined}
        onPaginate={() => undefined}
      />
    );

    expect(screen.getAllByText(/Unknown user/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/@private-(root|reply):example\.invalid/)).toBeNull();
    // #1546: a single reply uses the singular, as the timeline thread chip does.
    expect(document.body.textContent).toContain("1 reply");
    expect(document.body.textContent).not.toContain("1 replies");
  });

  it("resolves each remote string independently in mixed-direction rows", () => {
    const rootPreview = "שורש השרשור 5!";
    const rootSender = "דוד שולח 42!";
    const latestSender = "Alice";
    const latestPreview = "Latest reply 99!";
    const { container } = render(
      <ThreadsListView
        scope={{ kind: "home" }}
        threadsList={{
          kind: "open",
          room_id: "home",
          request_id: 3,
          items: [
            {
              room_id: "!room:example.invalid",
              root_event_id: "$root:example.invalid",
              root_sender: "@root:example.invalid",
              root_sender_label: rootSender,
              root_body_preview: rootPreview,
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: "$reply:example.invalid",
              latest_sender: "@reply:example.invalid",
              latest_sender_label: latestSender,
              latest_body_preview: latestPreview,
              latest_timestamp_ms: 1_800_000_000_100,
              reply_count: 2,
              unread_count: 0,
            }
          ],
          is_paginating: false,
          end_reached: true
        }}
        onClose={() => undefined}
        onOpenThread={() => undefined}
        onPaginate={() => undefined}
      />
    );

    const main = container.querySelector(".threads-list-row-main");
    expect(main?.getAttribute("dir")).not.toBe("auto");
    const directionalText = Array.from(main!.querySelectorAll<HTMLElement>("[dir=auto]"));
    expect(directionalText.map((element) => element.textContent)).toEqual([
      rootPreview,
      rootSender,
      latestSender,
      latestPreview
    ]);
  });

  it("opens an aggregate row in the room that owns its root", () => {
    const onOpenThread = vi.fn();
    render(
      <ThreadsListView
        scope={{ kind: "home" }}
        threadsList={{
          kind: "open",
          room_id: "home",
          request_id: 2,
          items: [
            {
              room_id: "!room-b:example.invalid",
              root_event_id: "$root-b:example.invalid",
              root_sender: "@sender:example.invalid",
              root_sender_label: "Sender",
              root_body_preview: "Room B thread",
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: null,
              latest_sender: null,
              latest_sender_label: null,
              latest_body_preview: null,
              latest_timestamp_ms: null,
              reply_count: 1,
              unread_count: 0,
            }
          ],
          is_paginating: false,
          end_reached: true
        }}
        onClose={() => undefined}
        onOpenThread={onOpenThread}
        onPaginate={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Room B thread/ }));
    expect(onOpenThread).toHaveBeenCalledWith(
      "!room-b:example.invalid",
      "$root-b:example.invalid",
      "existingThread"
    );
  });
});

// #1259: a row shows the same per-root unread count as the room timeline chip.
describe("ThreadsListView unread thread replies", () => {
  it("updates each row from its projected unread count", () => {
    const list = (firstUnread: number, secondUnread: number) => (
      <ThreadsListView
        scope={{ kind: "room", room_id: "!room:example.invalid" }}
        threadsList={{
          kind: "open",
          room_id: "!room:example.invalid",
          request_id: 1,
          items: [
            {
              room_id: "!room:example.invalid",
              root_event_id: "$root:example.invalid",
              root_sender: "@alice:example.invalid",
              root_sender_label: "Alice",
              root_body_preview: "Root",
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: "$reply:example.invalid",
              latest_sender: "@bob:example.invalid",
              latest_sender_label: "Bob",
              latest_body_preview: "Reply",
              latest_timestamp_ms: 1_800_000_000_100,
              reply_count: 2,
              unread_count: firstUnread
            },
            {
              room_id: "!room:example.invalid",
              root_event_id: "$read-root:example.invalid",
              root_sender: "@alice:example.invalid",
              root_sender_label: "Alice",
              root_body_preview: "Read root",
              root_timestamp_ms: 1_800_000_000_000,
              latest_event_id: null,
              latest_sender: null,
              latest_sender_label: null,
              latest_body_preview: null,
              latest_timestamp_ms: null,
              reply_count: 1,
              unread_count: secondUnread
            }
          ],
          is_paginating: false,
          end_reached: true
        }}
        onClose={() => undefined}
        onOpenThread={() => undefined}
        onPaginate={() => undefined}
      />
    );
    const { rerender } = render(list(2, 0));

    expect(screen.getByLabelText("Thread unread · 2").textContent).toBe("2");
    expect(document.querySelectorAll(".threads-list-row-unread")).toHaveLength(1);

    rerender(list(2, 104));
    expect(screen.getByLabelText("Thread unread · 2").textContent).toBe("2");
    expect(screen.getByLabelText("Thread unread · 104").textContent).toBe("99+");
    rerender(list(0, 0));
    expect(document.querySelectorAll(".threads-list-row-unread")).toHaveLength(0);
  });
});
