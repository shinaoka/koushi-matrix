import { describe, expect, test } from "vitest";

import {
  MAX_AVATAR_THUMBNAIL_ATTEMPTS,
  planSnapshotAvatarThumbnailRequests
} from "./avatarThumbnails";
import type { AvatarImage, DesktopSnapshot } from "./types";

describe("planSnapshotAvatarThumbnailRequests", () => {
  test("requests not-yet-downloaded snapshot avatars", () => {
    const plan = planSnapshotAvatarThumbnailRequests(
      snapshotWithAvatars([avatar("mxc://matrix.org/profile", { kind: "notRequested" })]),
      new Set(),
      new Map()
    );

    expect(plan.requestMxcUris).toEqual(["mxc://matrix.org/profile"]);
    expect(plan.requestedMxcUris.has("mxc://matrix.org/profile")).toBe(true);
    expect(plan.retryCounts.get("mxc://matrix.org/profile")).toBe(1);
  });

  test("retries transient failed snapshot avatars after an in-flight request settles", () => {
    const plan = planSnapshotAvatarThumbnailRequests(
      snapshotWithAvatars([
        avatar("mxc://matrix.org/profile-retry", {
          kind: "failed",
          request_id: 7,
          failureKind: "network"
        })
      ]),
      new Set(["mxc://matrix.org/profile-retry"]),
      new Map([["mxc://matrix.org/profile-retry", 1]])
    );

    expect(plan.requestMxcUris).toEqual(["mxc://matrix.org/profile-retry"]);
    expect(plan.requestedMxcUris.has("mxc://matrix.org/profile-retry")).toBe(true);
    expect(plan.retryCounts.get("mxc://matrix.org/profile-retry")).toBe(2);
  });

  test("stops retrying snapshot avatars after the bounded retry budget", () => {
    const plan = planSnapshotAvatarThumbnailRequests(
      snapshotWithAvatars([
        avatar("mxc://matrix.org/profile-retry", {
          kind: "failed",
          request_id: 8,
          failureKind: "sdk"
        })
      ]),
      new Set(["mxc://matrix.org/profile-retry"]),
      new Map([["mxc://matrix.org/profile-retry", MAX_AVATAR_THUMBNAIL_ATTEMPTS]])
    );

    expect(plan.requestMxcUris).toEqual([]);
    expect(plan.requestedMxcUris.has("mxc://matrix.org/profile-retry")).toBe(false);
    expect(plan.retryCounts.get("mxc://matrix.org/profile-retry")).toBe(
      MAX_AVATAR_THUMBNAIL_ATTEMPTS
    );
  });

  test("does not retry permanent failures or avatars that already have a ready duplicate", () => {
    const plan = planSnapshotAvatarThumbnailRequests(
      snapshotWithAvatars([
        avatar("mxc://matrix.org/permanent", {
          kind: "failed",
          request_id: 9,
          failureKind: "forbidden"
        }),
        avatar("mxc://matrix.org/duplicate", { kind: "notRequested" }),
        avatar("mxc://matrix.org/duplicate", {
          kind: "ready",
          source_url: "asset://localhost/avatar",
          width: null,
          height: null,
          mime_type: null
        })
      ]),
      new Set(["mxc://matrix.org/permanent", "mxc://matrix.org/duplicate"]),
      new Map([
        ["mxc://matrix.org/permanent", 1],
        ["mxc://matrix.org/duplicate", 1]
      ])
    );

    expect(plan.requestMxcUris).toEqual([]);
    expect(plan.requestedMxcUris.has("mxc://matrix.org/permanent")).toBe(false);
    expect(plan.requestedMxcUris.has("mxc://matrix.org/duplicate")).toBe(false);
    expect(plan.retryCounts.has("mxc://matrix.org/permanent")).toBe(false);
    expect(plan.retryCounts.has("mxc://matrix.org/duplicate")).toBe(false);
  });
});

function avatar(mxcUri: string, thumbnail: AvatarImage["thumbnail"]): AvatarImage {
  return { mxc_uri: mxcUri, thumbnail };
}

function snapshotWithAvatars(avatars: AvatarImage[]): DesktopSnapshot {
  const [ownAvatar, userAvatar, roomAvatar, spaceAvatar, inviteAvatar] = avatars;
  return {
    state: {
      domain: {
        profile: {
          own: { display_name: null, avatar: ownAvatar ?? null },
          users: userAvatar
            ? {
                "@user:example.invalid": {
                  user_id: "@user:example.invalid",
                  display_name: null,
                  display_label: "User",
                  original_display_label: "User",
                  mention_search_terms: [],
                  avatar: userAvatar
                }
              }
            : {},
          local_aliases: {},
          local_alias_update: { kind: "idle" },
          ignored_user_ids: [],
          ignored_user_update: { kind: "idle" },
          update: { kind: "idle" }
        },
        rooms: roomAvatar ? [{ avatar: roomAvatar }] : [],
        spaces: spaceAvatar ? [{ avatar: spaceAvatar }] : [],
        invites: inviteAvatar ? [{ avatar: inviteAvatar }] : []
      }
    }
  } as unknown as DesktopSnapshot;
}
