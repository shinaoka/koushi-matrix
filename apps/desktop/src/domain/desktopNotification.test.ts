import { beforeEach, describe, expect, test, vi } from "vitest";

import {
  clearDesktopAttentionNotifications,
  desktopNotificationTargetPlan,
  sendDesktopAttentionNotification,
  type DesktopNotificationActivation,
  type DesktopNotificationTransport
} from "./desktopNotification";

function transport(overrides: Partial<DesktopNotificationTransport> = {}) {
  const show = vi.fn().mockResolvedValue(undefined);
  const clear = vi.fn().mockResolvedValue(undefined);
  const onActivated = vi.fn(() => () => undefined);
  return { transport: { show, clear, onActivated, ...overrides }, show, clear };
}

describe("desktop notification dispatch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // The banner text, the preview gate, and the navigation target are
  // Rust-owned; the webview only decides when to ask for a banner. The body and
  // preview assertions live in the Rust attention tests.
  test("asks the adapter to show the notification Rust owns", async () => {
    const { transport: port, show } = transport();

    await sendDesktopAttentionNotification(port);

    expect(show).toHaveBeenCalledOnce();
  });

  test("swallows notification failures with a fixed diagnostic token", async () => {
    const { transport: port } = transport({
      show: vi.fn().mockRejectedValue(new Error("private raw failure"))
    });
    const diagnostic = vi.fn();

    await expect(sendDesktopAttentionNotification(port, diagnostic)).resolves.toBeUndefined();

    expect(diagnostic).toHaveBeenCalledWith("attention_notification_failed");
    expect(diagnostic).not.toHaveBeenCalledWith(expect.stringContaining("private raw failure"));
  });

  test("clears native notifications through a mockable adapter", async () => {
    const { transport: port, clear } = transport();

    await clearDesktopAttentionNotifications(port);

    expect(clear).toHaveBeenCalledOnce();
  });

  test("reports native notification clear failure with a fixed token", async () => {
    const { transport: port } = transport({
      clear: vi.fn().mockRejectedValue(new Error("private raw failure"))
    });
    const diagnostic = vi.fn();

    await expect(clearDesktopAttentionNotifications(port, diagnostic)).resolves.toBeUndefined();

    expect(diagnostic).toHaveBeenCalledWith("attention_notification_clear_failed");
    expect(diagnostic).not.toHaveBeenCalledWith(expect.stringContaining("private raw failure"));
  });
});

describe("desktop notification activation plan", () => {
  const target = (overrides: Partial<DesktopNotificationActivation>) => ({
    room_id: "!room:example.invalid",
    event_id: "$event:example.invalid",
    thread_root_event_id: null,
    ...overrides
  });

  test("opens the thread anchored at the triggering reply", () => {
    expect(
      desktopNotificationTargetPlan(
        target({ thread_root_event_id: "$root:example.invalid" })
      )
    ).toEqual({
      kind: "thread",
      roomId: "!room:example.invalid",
      rootEventId: "$root:example.invalid",
      eventId: "$event:example.invalid"
    });
  });

  test("navigates the main timeline for a room event", () => {
    expect(desktopNotificationTargetPlan(target({}))).toEqual({
      kind: "event",
      roomId: "!room:example.invalid",
      eventId: "$event:example.invalid"
    });
  });

  test("still opens the room when the notification has no event", () => {
    expect(desktopNotificationTargetPlan(target({ event_id: null }))).toEqual({
      kind: "room",
      roomId: "!room:example.invalid"
    });
  });

  // A thread root without its reply cannot be pinned, so it degrades to the
  // room instead of opening a thread the user did not click.
  test("degrades a thread target without an event to the room", () => {
    expect(
      desktopNotificationTargetPlan(
        target({ event_id: null, thread_root_event_id: "$root:example.invalid" })
      )
    ).toEqual({ kind: "room", roomId: "!room:example.invalid" });
  });
});
