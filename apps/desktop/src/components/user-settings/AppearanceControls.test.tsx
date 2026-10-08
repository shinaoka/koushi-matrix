// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { setActiveLocaleProfile } from "../../i18n/messages";
import { LanguageControls } from "./AppearanceControls";

const baseProps = {
  selectedLocale: { language_tag: null, text_direction: "auto" as const },
  onUpdateSettings: vi.fn()
};

function languageSelect(): HTMLSelectElement {
  return screen.getByRole("combobox") as HTMLSelectElement;
}

describe("LanguageControls language dropdown", () => {
  beforeEach(() => {
    setActiveLocaleProfile("en");
  });

  afterEach(() => {
    cleanup();
    setActiveLocaleProfile("en");
  });

  test("is one labeled select offering exactly English and 日本語", () => {
    render(<LanguageControls {...baseProps} />);

    expect(screen.getAllByRole("combobox")).toHaveLength(1);
    expect(screen.getByRole("combobox", { name: "Language" })).toBeTruthy();
    const options = screen.getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual(["English", "日本語"]);
    expect(options.map((option) => (option as HTMLOptionElement).value)).toEqual([
      "en",
      "ja-JP"
    ]);
  });

  test("an unset language_tag displays English without rewriting the saved setting", () => {
    const onUpdateSettings = vi.fn();
    render(<LanguageControls {...baseProps} onUpdateSettings={onUpdateSettings} />);

    expect(languageSelect().value).toBe("en");
    expect(onUpdateSettings).not.toHaveBeenCalled();
  });

  test("selecting 日本語 saves the Japanese tag and preserves text_direction", () => {
    const onUpdateSettings = vi.fn();
    render(
      <LanguageControls
        selectedLocale={{ language_tag: null, text_direction: "rtl" }}
        onUpdateSettings={onUpdateSettings}
      />
    );

    fireEvent.change(languageSelect(), { target: { value: "ja-JP" } });

    expect(onUpdateSettings).toHaveBeenCalledWith({
      locale: { language_tag: "ja-JP", text_direction: "rtl" }
    });
  });

  test("a Japanese effective locale selects 日本語 and switching back persists explicit English", () => {
    const onUpdateSettings = vi.fn();
    setActiveLocaleProfile("ja");
    render(
      <LanguageControls
        selectedLocale={{ language_tag: "ja-JP", text_direction: "auto" }}
        onUpdateSettings={onUpdateSettings}
      />
    );

    expect(languageSelect().value).toBe("ja-JP");

    fireEvent.change(languageSelect(), { target: { value: "en" } });

    expect(onUpdateSettings).toHaveBeenCalledWith({
      locale: { language_tag: "en", text_direction: "auto" }
    });
  });

  test("an unsupported stored tag follows the Rust-resolved English catalog", () => {
    render(
      <LanguageControls
        selectedLocale={{ language_tag: "fr-FR", text_direction: "auto" }}
        onUpdateSettings={vi.fn()}
      />
    );

    expect(languageSelect().value).toBe("en");
  });
});
