// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createBrowserFakeApi } from "../backend/browserFakeApi";
import { EntityAvatar, Sidebar } from "./Shell";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("EntityAvatar", () => {
  it("renders a ready avatar image", () => {
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

    expect(document.querySelector<HTMLImageElement>(".room-avatar img")?.getAttribute("src")).toBe(
      "asset://avatar.bin"
    );
  });

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

  it("retries the same avatar URL after a transient image load failure", () => {
    vi.useFakeTimers();
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
    fireEvent.error(image!);
    expect(document.querySelector(".room-avatar img")).toBeNull();

    act(() => {
      vi.advanceTimersByTime(10_000);
    });

    expect(document.querySelector<HTMLImageElement>(".room-avatar img")?.getAttribute("src")).toBe(
      "asset://avatar.bin"
    );
  });
});

describe("Sidebar", () => {
  it("does not render room list sections on account Home", async () => {
    const api = createBrowserFakeApi();
    const snapshot = await api.selectSpace(null);

    render(
      <Sidebar
        activeRoomId={snapshot.state.ui.navigation.active_room_id}
        activeView="timeline"
        snapshot={snapshot}
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

    expect(document.querySelector('[data-room-section="rooms"]')).toBeNull();
    expect(document.querySelector('[data-room-section="people"]')).toBeNull();
  });
});
