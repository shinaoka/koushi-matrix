// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { AppearanceControls } from "./AppearanceControls";

const baseProps = {
  displayDensity: "comfortable" as const,
  selectedEmoji: "system" as const,
  selectedFont: "system" as const,
  selectedTheme: "dark" as const,
  selectedLocale: { language_tag: null, text_direction: "auto" as const },
  onDisplayDensityChange: vi.fn(),
  onUpdateSettings: vi.fn()
};

describe("AppearanceControls language", () => {
  test("shows Default (English), English, and Japanese and persists a complete locale patch", () => {
    const onUpdateSettings = vi.fn();
    render(<AppearanceControls {...baseProps} onUpdateSettings={onUpdateSettings} />);

    expect(screen.getByRole("group", { name: "Language" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Default (English)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "English" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Japanese" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Japanese" }));

    expect(onUpdateSettings).toHaveBeenCalledWith({
      locale: { language_tag: "ja-JP", text_direction: "auto" }
    });
  });
});
