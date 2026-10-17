// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { OtherOwnSessionsList, UnverifiedOwnSessionsDialog, otherOwnSessions } from "./ownSessions";
import type { DeviceTrustLevel, DeviceTrustSummary } from "../domain/types";

afterEach(cleanup);

function device(
  device_id: string,
  trust_level: DeviceTrustLevel,
  last_seen_ts: number | null,
  is_current = false
): DeviceTrustSummary {
  return {
    user_id: "@user:example.invalid",
    device_id,
    trust_level,
    display_name: `Session ${device_id}`,
    last_seen_ts,
    is_current
  };
}

const devices = [
  device("CURRENT", "verified", 5_000, true),
  device("PHONE", "verified", 4_000),
  device("OLD", "unverified", 1_000),
  device("NEW", "unknown", 3_000)
];

it("orders other sessions unverified first, then by last activity, without the current one", () => {
  expect(otherOwnSessions(devices).map((d) => d.device_id)).toEqual(["NEW", "OLD", "PHONE"]);
});

it("names the account and counts in the unverified-sessions dialog", () => {
  const onReview = vi.fn();
  const onLater = vi.fn();
  render(
    <UnverifiedOwnSessionsDialog
      account="Demo User"
      userId="@user:example.invalid"
      homeserver="https://example.invalid"
      alert={{ total_sessions: 4, unverified_sessions: 2 }}
      devices={devices}
      onReview={onReview}
      onLater={onLater}
    />
  );
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText("Unverified sessions on Demo User")).toBeTruthy();
  expect(dialog.textContent).toContain("@user:example.invalid");
  expect(dialog.textContent).toContain("https://example.invalid");
  expect(dialog.textContent).toContain("Signed-in sessions: 4. Not verified: 2.");
  const list = within(dialog).getByRole("list", { name: /unverified/i });
  expect(within(list).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
    expect.stringContaining("Session NEW"),
    expect.stringContaining("Session OLD")
  ]);
  fireEvent.click(within(dialog).getByRole("button", { name: "Later" }));
  expect(onLater).toHaveBeenCalledTimes(1);
  fireEvent.click(within(dialog).getByRole("button", { name: "Review sessions" }));
  expect(onReview).toHaveBeenCalledTimes(1);
});

it("lists other sessions or explains why the list is empty", () => {
  const { rerender } = render(<OtherOwnSessionsList devices={[]} label="Other sessions" />);
  expect(screen.queryByRole("list")).toBeNull();
  rerender(<OtherOwnSessionsList devices={[devices[0]]} label="Other sessions" />);
  expect(screen.queryByRole("list")).toBeNull();
  rerender(<OtherOwnSessionsList devices={devices} label="Other sessions" />);
  expect(
    within(screen.getByRole("list", { name: "Other sessions" })).getAllByRole("listitem")
  ).toHaveLength(3);
});
