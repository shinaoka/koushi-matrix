// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { DesktopSnapshot } from "../domain/types";
import { EntityAvatar, Sidebar } from "./Shell";

afterEach(() => {
  cleanup();
});

describe("EntityAvatar", () => {
  it("falls back to initials when a ready avatar image fails to render", () => {
    render(
      <EntityAvatar
        avatar={{
          mxc_uri: "mxc://matrix.org/avatar",
          thumbnail: {
            kind: "ready",
            source_url: "asset://avatar.bin",
            width: null,
            height: null,
            mime_type: null
          }
        }}
        className="room-avatar"
        fallback="AL"
      />
    );

    const image = document.querySelector<HTMLImageElement>(".room-avatar img");
    expect(image?.getAttribute("src")).toBe("asset://avatar.bin");
    fireEvent.error(image!);

    expect(document.querySelector(".room-avatar img")).toBeNull();
    expect(screen.getByText("AL")).toBeTruthy();
  });
});

describe("Sidebar", () => {
  it("does not render the duplicate horizontal room-list filter tabs", () => {
    render(
      <Sidebar
        activeRoomId={null}
        activeView="timeline"
        snapshot={minimalSidebarSnapshot()}
        onCreateRoom={() => undefined}
        onNewDm={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenExplore={() => undefined}
        onOpenHome={() => undefined}
        onOpenInvites={() => undefined}
        onOpenSpaceInfo={() => undefined}
        onOpenThreads={() => undefined}
        onSelectRoom={() => undefined}
      />
    );

    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("tab", { name: "Rooms" })).toBeNull();
    expect(screen.getByRole("region", { name: "DMs" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Rooms" })).toBeTruthy();
  });
});

function minimalSidebarSnapshot(): DesktopSnapshot {
  return {
    sidebar: {
      account_home: {
        display_name: "Home",
        is_active: true,
        unread_count: 0,
        highlight_count: 0
      },
      space_rail: [],
      space_rooms: [],
      global_dms: [],
      global_rooms: [],
      space_unread_count: 0,
      space_highlight_count: 0
    },
    timeline: [],
    thread: null,
    state: {
      schema_version: 2,
      domain: {
        spaces: [],
        rooms: [],
        invites: [],
        thread_attention: { kind: "closed" }
      },
      ui: {
        navigation: {
          active_room_id: null,
          active_space_id: null
        },
        room_list: {
          active_filter: { kind: "rooms" },
          sort: "activity",
          items: []
        }
      }
    }
  } as unknown as DesktopSnapshot;
}
