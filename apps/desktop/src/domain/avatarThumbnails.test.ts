import { describe, expect, test, vi } from "vitest";

import {
  planSnapshotAvatarThumbnailRequests,
  resolvedAvatar,
  requestAvatarThumbnailWithDedupe
} from "./avatarThumbnails";
import { readyDesktopSnapshotFixture } from "../test/desktopApiFixture";

describe("avatar thumbnail demand discovery", () => {
  test("deduplicates not-requested snapshot avatars without owning retries", () => {
    const snapshot = readyDesktopSnapshotFixture();
    const avatar = {
      mxc_uri: "mxc://example.invalid/shared",
      thumbnail: { kind: "notRequested" as const }
    };
    snapshot.state.domain.profile.own.avatar = avatar;
    snapshot.state.domain.rooms[0].avatar = avatar;

    const first = planSnapshotAvatarThumbnailRequests(snapshot, new Set());
    expect(first.requestMxcUris).toEqual([avatar.mxc_uri]);
    expect(
      planSnapshotAvatarThumbnailRequests(snapshot, first.requestedMxcUris).requestMxcUris
    ).toEqual([]);
  });

  test("does not request room-list avatars from the account-wide snapshot", () => {
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.state.domain.profile.own.avatar = null;
    snapshot.state.domain.rooms[0]!.avatar = {
      mxc_uri: "mxc://example.invalid/offscreen-room",
      thumbnail: { kind: "notRequested" }
    };

    expect(planSnapshotAvatarThumbnailRequests(snapshot, new Set())).toEqual({
      requestMxcUris: [],
      requestedMxcUris: new Set()
    });
  });

  test.each(["loading", "ready", "failed"] as const)(
    "does not retry a %s Rust-owned terminal or in-flight state",
    (kind) => {
      const snapshot = readyDesktopSnapshotFixture();
      snapshot.state.domain.profile.own.avatar = {
        mxc_uri: "mxc://example.invalid/avatar",
        thumbnail:
          kind === "loading"
            ? { kind, request_id: 1 }
            : kind === "ready"
              ? { kind, source_ref: "asset://avatar", width: null, height: null, mime_type: null }
              : { kind, request_id: 1, failureKind: "network" }
      };

      expect(
        planSnapshotAvatarThumbnailRequests(
          snapshot,
          new Set(["mxc://example.invalid/avatar"])
        )
      ).toEqual({ requestMxcUris: [], requestedMxcUris: new Set() });
    }
  );

  test("visible-row dedupe releases only when transport admission fails", async () => {
    const visible = new Set<string>();
    const request = vi.fn().mockRejectedValueOnce(new Error("transport")).mockResolvedValue(undefined);

    await requestAvatarThumbnailWithDedupe(
      "mxc://example.invalid/member",
      new Set(),
      visible,
      request
    );
    expect(visible.size).toBe(0);

    await requestAvatarThumbnailWithDedupe(
      "mxc://example.invalid/member",
      new Set(),
      visible,
      request
    );
    await requestAvatarThumbnailWithDedupe(
      "mxc://example.invalid/member",
      new Set(),
      visible,
      request
    );
    expect(request).toHaveBeenCalledTimes(2);
    expect(visible).toEqual(new Set(["mxc://example.invalid/member"]));
  });
});

describe("ready avatar reuse", () => {
  const own = {
    mxc_uri: "mxc://example.invalid/own",
    thumbnail: {
      kind: "ready" as const,
      source_ref: "https://example.invalid/own.png",
      width: 32,
      height: 32,
      mime_type: "image/png"
    }
  };
  test("reuses the own thumbnail for the same message avatar resource", () => {
    const pending = { ...own, thumbnail: { kind: "notRequested" as const } };
    expect(resolvedAvatar(pending, undefined, own)).toBe(own);
  });
  test("does not substitute a global avatar for a different room-specific image or missing sender image", () => {
    const room = { mxc_uri: "mxc://example.invalid/room-avatar", thumbnail: { kind: "notRequested" as const } };
    expect(resolvedAvatar(room, undefined, own)).toBe(room);
    expect(resolvedAvatar(null, undefined, own)).toBeNull();
  });
  test("does not replace a ready row with an unready own thumbnail", () => {
    const pending = { ...own, thumbnail: { kind: "notRequested" as const } };
    expect(resolvedAvatar(own, undefined, pending)).toBe(own);
  });
});
