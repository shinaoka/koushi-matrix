import { describe, expect, test, vi } from "vitest";

import { selectJoinedRoomIfPresent } from "./joinedRoomNavigation";

const rooms = [{ room_id: "!joined:example.invalid" }];

describe("joined-room navigation", () => {
  test("does not dispatch before Rust projects the joined room", async () => {
    const selectRoom = vi.fn(async () => true);

    await expect(
      selectJoinedRoomIfPresent(rooms, "!missing:example.invalid", selectRoom)
    ).resolves.toBe(false);
    expect(selectRoom).not.toHaveBeenCalled();
  });

  test("returns the shared room-selection settlement unchanged", async () => {
    const refused = vi.fn(async () => false);
    await expect(
      selectJoinedRoomIfPresent(rooms, "!joined:example.invalid", refused)
    ).resolves.toBe(false);
    expect(refused).toHaveBeenCalledWith("!joined:example.invalid");

    const committed = vi.fn(async () => true);
    await expect(
      selectJoinedRoomIfPresent(rooms, "!joined:example.invalid", committed)
    ).resolves.toBe(true);
    expect(committed).toHaveBeenCalledWith("!joined:example.invalid");

    const failure = new Error("synthetic selection failure");
    const rejected = vi.fn(async () => {
      throw failure;
    });
    await expect(
      selectJoinedRoomIfPresent(rooms, "!joined:example.invalid", rejected)
    ).rejects.toBe(failure);
  });
});
