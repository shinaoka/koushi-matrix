// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { DiagnosticDialog } from "./dialogs";

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
  vi.unstubAllGlobals();
});

describe("DiagnosticDialog", () => {
  it.each(["en", "ja"] as const)("names its close button in %s", (locale) => {
    setActiveLocaleProfile(locale, "none");
    const onClose = vi.fn();
    render(<DiagnosticDialog report="" onClose={onClose} />);
    const close = screen.getByRole("button", {
      name: t("action.close", { title: t("diagnostics.title") })
    });
    expect(close.getAttribute("aria-label")).not.toContain("{title}");
    fireEvent.click(close);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it.each(["en", "ja"] as const)(
    "states that diagnostics carry no private data and belong in an attached file in %s",
    (locale) => {
      setActiveLocaleProfile(locale, "none");
      render(<DiagnosticDialog report="Koushi diagnostics" onClose={() => undefined} />);

      const note = t("diagnostics.privacyNote");
      expect(note).not.toContain("{");
      expect(screen.getByText(note)).toBeTruthy();
    }
  );

  it("puts the privacy statement above the report it vouches for", () => {
    render(<DiagnosticDialog report="Koushi diagnostics" onClose={() => undefined} />);

    const note = screen.getByText(t("diagnostics.privacyNote"));
    const report = screen.getByText("Koushi diagnostics");

    // The statement explains why the report is safe to share, so a reader who
    // stops after the first glance must still see it: it precedes the report.
    expect(note.compareDocumentPosition(report) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // ...and it renders as the callout, not as muted fine print.
    expect(note.closest(".diagnostics-privacy-note")).not.toBeNull();
  });

  it("saves the report to a file and reports the saved state", async () => {
    const onSaveReport = vi.fn(async () => true);
    render(
      <DiagnosticDialog
        report="Koushi diagnostics"
        onClose={() => undefined}
        onSaveReport={onSaveReport}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: t("diagnostics.save") }));

    expect(onSaveReport).toHaveBeenCalledOnce();
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe(t("diagnostics.saved")));
  });

  it("returns to rest when the native save dialog is dismissed", async () => {
    const onSaveReport = vi.fn(async () => false);
    render(
      <DiagnosticDialog
        report="Koushi diagnostics"
        onClose={() => undefined}
        onSaveReport={onSaveReport}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: t("diagnostics.save") }));

    await waitFor(() => expect(onSaveReport).toHaveBeenCalledOnce());
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("surfaces a failed save next to the button", async () => {
    const onSaveReport = vi.fn(async () => {
      throw new Error("destination unavailable");
    });
    render(
      <DiagnosticDialog
        report="Koushi diagnostics"
        onClose={() => undefined}
        onSaveReport={onSaveReport}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: t("diagnostics.save") }));

    await waitFor(() =>
      expect(screen.getByRole("status").textContent).toBe(t("diagnostics.saveFailed"))
    );
  });

  it("copies the full report and reports the copied state", async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    render(<DiagnosticDialog report="Koushi diagnostics" onClose={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: t("diagnostics.copy") }));

    expect(writeText).toHaveBeenCalledWith("Koushi diagnostics");
    await waitFor(() =>
      expect(screen.getByRole("status").textContent).toBe(t("diagnostics.copied"))
    );
  });
});
