// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setActiveLocaleProfile, t } from "../i18n/messages";
import { DiagnosticDialog } from "./dialogs";

afterEach(() => {
  cleanup();
  setActiveLocaleProfile("en", "none");
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
});
