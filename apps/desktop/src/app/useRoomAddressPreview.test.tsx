// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { useRoomAddressPreview } from "./useRoomAddressPreview";
import type { RoomAddressPreview } from "../domain/types";

test("returns only Rust previews for the current account and raw draft", async () => {
  const pending: Array<(value: RoomAddressPreview) => void> = [];
  const api = { previewRoomAddress: vi.fn(() => new Promise<RoomAddressPreview>(resolve => pending.push(resolve))) };
  const { result, rerender } = renderHook(({ name, alias, account }) =>
    useRoomAddressPreview(api, name, alias, account, true),
    { initialProps: { name: "First", alias: null as string | null, account: "account-a" } });
  rerender({ name: "Second", alias: "manual", account: "account-a" });
  await act(async () => pending[0]!({ localpart: "first", full_alias: "#first:example.invalid", error: null }));
  expect(result.current).toBeNull();
  await act(async () => pending[1]!({ localpart: "manual", full_alias: "#manual:example.invalid", error: null }));
  await waitFor(() => expect(result.current?.localpart).toBe("manual"));
  expect(api.previewRoomAddress).toHaveBeenLastCalledWith("Second", "manual");
  rerender({ name: "Second", alias: "manual", account: "account-b" });
  expect(result.current).toBeNull();
});
