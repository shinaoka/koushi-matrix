import { describe, expect, test } from "vitest";

import {
  MAX_AVATAR_THUMBNAIL_ATTEMPTS,
  planSnapshotAvatarThumbnailRequests
} from "./avatarThumbnails";
import type { AvatarImage, DesktopSnapshot, UserProfile } from "./types";

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
        // roomAvatar: duplicate notRequested
        avatar("mxc://matrix.org/duplicate", { kind: "notRequested" }),
        // spaceAvatar: duplicate ready — the 'ready' entry wins and evicts the notRequested one
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

  test("flood guard: profile.users member avatars are NOT bulk-requested by the snapshot planner", () => {
    // Build a snapshot with 500 member avatars (profile.users), plus one own avatar,
    // one room avatar, one space avatar, and one invite avatar — all not-yet-downloaded.
    // The planner must emit ZERO requests for the member mxc URIs, while still
    // requesting the own/room/space/invite avatars (#116 Stage F1a flood guard).
    const memberCount = 500;
    const memberUsers: Record<string, UserProfile> = {};
    for (let i = 0; i < memberCount; i++) {
      const uid = `@member${i}:example.invalid`;
      memberUsers[uid] = {
        user_id: uid,
        display_name: null,
        display_label: `Member ${i}`,
        original_display_label: `Member ${i}`,
        mention_search_terms: [],
        avatar: {
          mxc_uri: `mxc://matrix.org/member-avatar-${i}`,
          thumbnail: { kind: "notRequested" }
        }
      };
    }

    const snap = {
      state: {
        domain: {
          profile: {
            own: {
              display_name: null,
              avatar: avatar("mxc://matrix.org/own-avatar", { kind: "notRequested" })
            },
            users: memberUsers,
            local_aliases: {},
            local_alias_update: { kind: "idle" },
            ignored_user_ids: [],
            ignored_user_update: { kind: "idle" },
            update: { kind: "idle" }
          },
          rooms: [{ avatar: avatar("mxc://matrix.org/room-avatar", { kind: "notRequested" }) }],
          spaces: [{ avatar: avatar("mxc://matrix.org/space-avatar", { kind: "notRequested" }) }],
          invites: [{ avatar: avatar("mxc://matrix.org/invite-avatar", { kind: "notRequested" }) }]
        }
      }
    } as unknown as DesktopSnapshot;

    const plan = planSnapshotAvatarThumbnailRequests(snap, new Set(), new Map());

    // own / room / space / invite avatars must be requested
    expect(plan.requestMxcUris).toContain("mxc://matrix.org/own-avatar");
    expect(plan.requestMxcUris).toContain("mxc://matrix.org/room-avatar");
    expect(plan.requestMxcUris).toContain("mxc://matrix.org/space-avatar");
    expect(plan.requestMxcUris).toContain("mxc://matrix.org/invite-avatar");
    expect(plan.requestMxcUris).toHaveLength(4);

    // None of the 500 member mxc URIs must appear in the plan
    for (let i = 0; i < memberCount; i++) {
      expect(plan.requestMxcUris).not.toContain(`mxc://matrix.org/member-avatar-${i}`);
    }
  });
});

function avatar(mxcUri: string, thumbnail: AvatarImage["thumbnail"]): AvatarImage {
  return { mxc_uri: mxcUri, thumbnail };
}

/**
 * Helper: builds a minimal DesktopSnapshot with avatars placed at the four
 * positions that the snapshot planner now covers (own, room, space, invite).
 * profile.users is intentionally left empty — member avatars are no longer
 * bulk-requested by the planner (#116 Stage F1a).
 */
function snapshotWithAvatars(avatars: AvatarImage[]): DesktopSnapshot {
  const [ownAvatar, roomAvatar, spaceAvatar, inviteAvatar] = avatars;
  return {
    state: {
      domain: {
        profile: {
          own: { display_name: null, avatar: ownAvatar ?? null },
          users: {},
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
