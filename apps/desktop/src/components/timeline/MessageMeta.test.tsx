// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MessageMeta } from "./MessageMeta";

afterEach(cleanup);

describe("MessageMeta sender profile action", () => {
  const displayLabel = ["Duplicate", "Name"].join(" ");
  const baseProps = {
    senderDisplayLabel: displayLabel,
    timestampMs: 1_800_000_000_000,
    isEdited: false,
    isRedacted: false,
    sendStateKind: null
  };

  it("uses a native labelled button and contains activation", () => {
    const onOpenSenderProfile = vi.fn();
    const onParentClick = vi.fn();
    render(
      <div onClick={onParentClick}>
        <MessageMeta {...baseProps} onOpenSenderProfile={onOpenSenderProfile} />
      </div>
    );

    const sender = screen.getByRole("button", {
      name: `Open profile for ${displayLabel}`
    });
    expect(sender.textContent).toBe(displayLabel);
    expect(sender.getAttribute("type")).toBe("button");
    fireEvent.click(sender);
    expect(onOpenSenderProfile).toHaveBeenCalledTimes(1);
    expect(onParentClick).not.toHaveBeenCalled();
  });

  it("keeps the sender as plain text without a bound stable-id action", () => {
    const { container } = render(<MessageMeta {...baseProps} />);
    expect(screen.queryByRole("button", { name: /Open profile for/ })).toBeNull();
    expect(container.querySelector("span.sender")?.textContent).toBe(displayLabel);
  });
});
