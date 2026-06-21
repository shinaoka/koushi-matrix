// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Composer } from "./composer";

afterEach(() => {
  cleanup();
});

describe("Composer", () => {
  it("keeps typed text local and sends it before parent state catches up", () => {
    const onSend = vi.fn();
    const onValueChange = vi.fn();

    const { container } = render(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Direct room"
        value=""
        onCancelReply={() => undefined}
        onSend={onSend}
        onValueChange={onValueChange}
      />
    );

    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    fireEvent.change(textarea!, {
      target: { value: "pasted text that should appear immediately" }
    });

    expect(textarea!.value).toBe("pasted text that should appear immediately");
    expect(onValueChange).toHaveBeenCalledWith("pasted text that should appear immediately");

    fireEvent.click(screen.getByLabelText("Send"));

    expect(onSend).toHaveBeenCalledWith("pasted text that should appear immediately");
  });
});
