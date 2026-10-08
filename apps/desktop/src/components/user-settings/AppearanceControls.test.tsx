// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { getActiveLocale, setActiveLocaleProfile } from "../../i18n/messages";
import { readyDesktopSnapshotFixture } from "../../test/desktopApiFixture";
import { AppSettingsDialog } from "../UserSettingsPanel";
import { LanguageControls } from "./AppearanceControls";

const baseProps = {
  catalogLocale: "en" as const,
  selectedLocale: { language_tag: null, text_direction: "auto" as const },
  onUpdateSettings: vi.fn()
};

function languageSelect(): HTMLSelectElement {
  return screen.getByRole("combobox") as HTMLSelectElement;
}

describe("LanguageControls language dropdown", () => {
  beforeEach(() => {
    // The global catalog is deliberately a different value from the snapshot
    // resolved locale in the tests below: the control must not read it.
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
        catalogLocale="en"
        selectedLocale={{ language_tag: null, text_direction: "rtl" }}
        onUpdateSettings={onUpdateSettings}
      />
    );

    fireEvent.change(languageSelect(), { target: { value: "ja-JP" } });

    expect(onUpdateSettings).toHaveBeenCalledWith({
      locale: { language_tag: "ja-JP", text_direction: "rtl" }
    });
  });

  test("selecting English persists the explicit English tag with the unchanged direction", () => {
    const onUpdateSettings = vi.fn();
    render(
      <LanguageControls
        catalogLocale="ja"
        selectedLocale={{ language_tag: "ja-JP", text_direction: "rtl" }}
        onUpdateSettings={onUpdateSettings}
      />
    );

    expect(languageSelect().value).toBe("ja-JP");
    fireEvent.change(languageSelect(), { target: { value: "en" } });

    expect(onUpdateSettings).toHaveBeenCalledWith({
      locale: { language_tag: "en", text_direction: "rtl" }
    });
  });

  test("follows the Rust-resolved catalog locale, not the global catalog", () => {
    setActiveLocaleProfile("en");
    render(
      <LanguageControls
        catalogLocale="ja"
        selectedLocale={{ language_tag: null, text_direction: "auto" }}
        onUpdateSettings={vi.fn()}
      />
    );

    expect(languageSelect().value).toBe("ja-JP");
  });

  test("a changed resolved profile updates an already-open dialog", () => {
    const { rerender } = render(
      <LanguageControls
        catalogLocale="ja"
        selectedLocale={{ language_tag: "ja-JP", text_direction: "auto" }}
        onUpdateSettings={vi.fn()}
      />
    );
    expect(languageSelect().value).toBe("ja-JP");

    rerender(
      <LanguageControls
        catalogLocale="en"
        selectedLocale={{ language_tag: "en", text_direction: "auto" }}
        onUpdateSettings={vi.fn()}
      />
    );

    expect(languageSelect().value).toBe("en");
  });

  test("an unsupported stored tag follows the Rust-resolved English catalog", () => {
    render(
      <LanguageControls
        catalogLocale="en"
        selectedLocale={{ language_tag: "fr-FR", text_direction: "auto" }}
        onUpdateSettings={vi.fn()}
      />
    );

    expect(languageSelect().value).toBe("en");
  });
});

describe("App Settings resolved language", () => {
  afterEach(() => {
    cleanup();
    setActiveLocaleProfile("en");
  });

  test("opens with the snapshot's Japanese locale while the global catalog is English", () => {
    setActiveLocaleProfile("en");
    const snapshot = readyDesktopSnapshotFixture();
    snapshot.state.domain.locale_profile = {
      ...snapshot.state.domain.locale_profile,
      lang: "ja",
      catalog_locale: "ja"
    };

    render(
      <AppSettingsDialog snapshot={snapshot} onUpdateSettings={vi.fn()} onClose={vi.fn()} />
    );

    expect(getActiveLocale()).toBe("en");
    expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe("ja-JP");
  });
});
