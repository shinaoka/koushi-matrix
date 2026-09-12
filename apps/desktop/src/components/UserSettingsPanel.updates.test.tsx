// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { DesktopUpdateControls } from "./UserSettingsPanel";

afterEach(cleanup);

describe("DesktopUpdateControls", () => {
  test("dispatches the Rust-shaped automatic update setting", () => {
    const onSelect = vi.fn();
    render(
      <DesktopUpdateControls
        current={{ auto_check: true }}
        state={{ kind: "idle" }}
        onSelect={onSelect}
        onRestart={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("switch", { name: "Automatically check for updates" }));
    expect(onSelect).toHaveBeenCalledWith({ updates: { auto_check: false } });
  });

  test("offers restart only after a verified update is ready", () => {
    const onRestart = vi.fn();
    const { rerender } = render(
      <DesktopUpdateControls
        current={{ auto_check: true }}
        state={{ kind: "downloading", version: "1.2.3" }}
        onSelect={() => undefined}
        onRestart={onRestart}
      />
    );
    expect(screen.queryByRole("button", { name: "Restart to install" })).toBeNull();

    rerender(
      <DesktopUpdateControls
        current={{ auto_check: true }}
        state={{ kind: "ready", version: "1.2.3" }}
        onSelect={() => undefined}
        onRestart={onRestart}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Restart to install" }));
    expect(onRestart).toHaveBeenCalledOnce();
  });
});
