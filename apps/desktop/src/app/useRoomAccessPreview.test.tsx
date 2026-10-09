// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";

import type { RoomAccessDraftScope, RoomAccessPreview } from "../domain/types";
import { useRoomAccessPreview } from "./useRoomAccessPreview";

const scope: RoomAccessDraftScope = { kind: "room", roomId: "!room:example.invalid" };

function preview(joinMessageId: RoomAccessPreview["outcome"]["join"]["messageId"]): RoomAccessPreview {
  return {
    scope,
    context: "access",
    confirmed: false,
    outcome: {
      join: { messageId: joinMessageId },
      history: { messageId: "room.accessOutcomeHistoryShared" },
      encryption: { messageId: "room.accessOutcomeNotEncrypted" },
      directory: { messageId: "room.accessOutcomeDirectoryPrivate" },
      nonRetroactive: { messageId: "room.historyNonRetroactive" }
    }
  };
}

test("an older preview result never replaces newer details (#1177)", async () => {
  const pending: Array<(value: RoomAccessPreview) => void> = [];
  const api = {
    previewRoomAccess: vi.fn(() => new Promise<RoomAccessPreview>((resolve) => pending.push(resolve)))
  };
  const { result, rerender } = renderHook(
    ({ generation }) => useRoomAccessPreview(api, scope, "access", generation, true),
    { initialProps: { generation: 1 } }
  );
  // A second identity (a confirmed-property advance with no draft change)
  // starts a newer request, and its effect cleanup retires the first.
  rerender({ generation: 2 });
  await act(async () => {
    pending[1]!(preview("room.accessOutcomeJoinPublic"));
  });
  await waitFor(() =>
    expect(result.current?.outcome.join.messageId).toBe("room.accessOutcomeJoinPublic")
  );
  // The older result resolves last; it must not overwrite the newer details.
  await act(async () => {
    pending[0]!(preview("room.accessOutcomeJoinInvite"));
  });
  expect(result.current?.outcome.join.messageId).toBe("room.accessOutcomeJoinPublic");
});

test("leaving the panel clears the preview", async () => {
  const api = {
    previewRoomAccess: vi.fn(() => Promise.resolve(preview("room.accessOutcomeJoinPublic")))
  };
  const { result, rerender } = renderHook(
    ({ enabled }) => useRoomAccessPreview(api, scope, "access", 1, enabled),
    { initialProps: { enabled: true } }
  );
  await waitFor(() =>
    expect(result.current?.outcome.join.messageId).toBe("room.accessOutcomeJoinPublic")
  );
  rerender({ enabled: false });
  expect(result.current).toBeNull();
});
