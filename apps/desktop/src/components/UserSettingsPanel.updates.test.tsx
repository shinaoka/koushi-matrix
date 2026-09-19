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
        current={{ auto_check: true, include_prereleases: false }}
        state={{ kind: "idle" }}
        onSelect={onSelect}
        onCheck={() => undefined}
        onDownload={() => undefined}
        onRestart={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("switch", { name: "Automatically check for updates" }));
    expect(onSelect).toHaveBeenCalledWith({
      updates: { auto_check: false, include_prereleases: false }
    });
  });

  test("offers restart only after a verified update is ready", () => {
    const onRestart = vi.fn();
    const { rerender } = render(
      <DesktopUpdateControls
        current={{ auto_check: true, include_prereleases: false }}
        state={{ kind: "downloading", version: "1.2.3" }}
        onSelect={() => undefined}
        onCheck={() => undefined}
        onDownload={() => undefined}
        onRestart={onRestart}
      />
    );
    expect(screen.queryByRole("button", { name: "Restart to install" })).toBeNull();

    rerender(
      <DesktopUpdateControls
        current={{ auto_check: true, include_prereleases: false }}
        state={{ kind: "ready", version: "1.2.3" }}
        onSelect={() => undefined}
        onCheck={() => undefined}
        onDownload={() => undefined}
        onRestart={onRestart}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Restart to install" }));
    expect(onRestart).toHaveBeenCalledOnce();
  });

  test("dispatches the pre-release channel setting without changing automatic checks", () => {
    const onSelect = vi.fn();
    render(
      <DesktopUpdateControls
        current={{ auto_check: true, include_prereleases: false }}
        state={{ kind: "idle" }}
        onSelect={onSelect}
        onCheck={() => undefined}
        onDownload={() => undefined}
        onRestart={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("switch", { name: "Include pre-release versions" }));
    expect(onSelect).toHaveBeenCalledWith({
      updates: { auto_check: true, include_prereleases: true }
    });
  });

  test("asks the user to download after an update is found", () => {
    const onDownload = vi.fn();
    render(
      <DesktopUpdateControls
        current={{ auto_check: true, include_prereleases: false }}
        state={{ kind: "available", version: "1.2.3" }}
        onSelect={() => undefined}
        onCheck={() => undefined}
        onDownload={onDownload}
        onRestart={() => undefined}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: "Download update" }));
    expect(onDownload).toHaveBeenCalledOnce();
  });

  test("shows the latest version and offers another manual check", () => {
    const onCheck = vi.fn();
    render(
      <DesktopUpdateControls
        current={{ auto_check: true, include_prereleases: false }}
        state={{ kind: "up_to_date", version: "1.2.3" }}
        onSelect={() => undefined}
        onCheck={onCheck}
        onDownload={() => undefined}
        onRestart={() => undefined}
      />
    );

    expect(screen.getByText("Koushi is up to date (v1.2.3).")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(onCheck).toHaveBeenCalledOnce();
  });
});
