// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ReactionSenders } from "./ReactionSenders";
import type { ReactionSender } from "../../domain/coreEvents";

function senders(...labels: string[]): ReactionSender[] {
  return labels.map((label, index) => ({
    user_id: `@reactor-${index}:example.invalid`,
    display_label: label
  }));
}

function renderSenders({
  preview,
  totalCount = preview.length,
  currentUserId = "@reactor-0:example.invalid",
  onOpenProfile = vi.fn(),
  onClose = vi.fn()
}: {
  preview: ReactionSender[];
  totalCount?: number;
  currentUserId?: string;
  onOpenProfile?: (userId: string) => void;
  onClose?: () => void;
}) {
  const anchorRef = { current: document.createElement("span") };
  document.body.appendChild(anchorRef.current);
  const view = render(
    <ReactionSenders
      anchorRef={anchorRef}
      currentUserId={currentUserId}
      open
      profiles={{}}
      reactionLabel="🎉"
      senders={preview}
      totalCount={totalCount}
      onClose={onClose}
      onOpenProfile={onOpenProfile}
    />
  );
  return { ...view, anchorRef, onOpenProfile, onClose };
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("ReactionSenders", () => {
  it("lists every reactor the preview names and marks the signed-in user", () => {
    renderSenders({
      preview: senders("Alice", "Bob", "Carol", "Dana", "Erin"),
      currentUserId: "@reactor-2:example.invalid"
    });

    const dialog = screen.getByRole("dialog", { name: "Reacted with 🎉" });
    const names = Array.from(dialog.querySelectorAll(".receipt-reader-name")).map(
      (name) => name.textContent
    );
    expect(names).toEqual(["Alice", "Bob", "Carol", "Dana", "Erin"]);
    // Only the signed-in user carries the marker.
    expect(
      Array.from(dialog.querySelectorAll(".reaction-sender-self")).map((marker) => marker.textContent)
    ).toEqual(["You"]);
    // A named group is complete, so there is no "more" row.
    expect(dialog.textContent).not.toContain("more");
  });

  it("says how many people the preview left out instead of pretending to list them", () => {
    renderSenders({ preview: senders("Alice", "Bob", "Carol"), totalCount: 60 });

    const dialog = screen.getByRole("dialog", { name: "Reacted with 🎉" });
    expect(dialog.textContent).toContain("57 more");
    expect(Array.from(dialog.querySelectorAll('[role="listitem"]'))).toHaveLength(3);
  });

  it("opens the focused person's profile and closes the list", async () => {
    const { onOpenProfile, onClose } = renderSenders({ preview: senders("Alice", "Bob") });

    const row = screen.getByRole("button", { name: "Bob" });
    fireEvent.click(row);

    expect(onOpenProfile).toHaveBeenCalledWith("@reactor-1:example.invalid");
    expect(onClose).toHaveBeenCalled();
  });

  it("moves a roving focus with the arrow keys and closes on Escape", async () => {
    const { onClose } = renderSenders({ preview: senders("Alice", "Bob", "Carol") });

    const rows = screen.getAllByRole("listitem");
    const buttons = screen.getAllByRole("button", { name: /Alice|Bob|Carol/ });
    // Exactly one row is in the tab order: the list is reachable once.
    expect(buttons.map((button) => button.tabIndex)).toEqual([0, -1, -1]);

    fireEvent.keyDown(rows[0], { key: "ArrowDown" });
    await waitFor(() => expect(document.activeElement).toBe(buttons[1]));
    fireEvent.keyDown(rows[1], { key: "End" });
    await waitFor(() => expect(document.activeElement).toBe(buttons[2]));
    fireEvent.keyDown(rows[2], { key: "ArrowUp" });
    await waitFor(() => expect(document.activeElement).toBe(buttons[1]));

    fireEvent.keyDown(rows[1], { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps the dialog reachable when the preview arrives without names", () => {
    renderSenders({ preview: [], totalCount: 7 });

    const dialog = screen.getByRole("dialog", { name: "Reacted with 🎉" });
    expect(dialog.textContent).toContain("7 more");
  });
});
