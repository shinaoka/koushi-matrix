// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DiagnosticDialog } from "./dialogs";

afterEach(cleanup);

describe("DiagnosticDialog", () => {
  it("shows copyable diagnostics", () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(
      <DiagnosticDialog
        report={"Koushi diagnostics\nDownloading messages from 1 room(s)"}
        onClose={vi.fn()}
      />
    );

    expect(screen.getByRole("dialog", { name: "Diagnostics" })).toBeTruthy();
    expect(screen.getByText(/Downloading messages from 1 room/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));

    expect(writeText).toHaveBeenCalledWith(
      "Koushi diagnostics\nDownloading messages from 1 room(s)"
    );
  });
});
