import { describe, expect, test } from "vitest";

import { createBrowserFakeApi } from "./browserFakeApi";

describe("BrowserFakeApi settings preview", () => {
  test("applies the Rust-shaped settings patch to the fixture snapshot", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.updateSettings({
      appearance: { theme: "dark" },
      keyboard: { composer_send_shortcut: "modEnter" }
    });

    expect(snapshot.state.settings.values.appearance.theme).toBe("dark");
    expect(snapshot.state.settings.values.keyboard.composer_send_shortcut).toBe("modEnter");
    expect(snapshot.state.settings.persistence).toEqual({ kind: "idle" });
  });

  test("resolves composer key actions from the Rust-shaped settings snapshot", async () => {
    const api = createBrowserFakeApi();

    await expect(
      api.resolveComposerKeyAction(
        "main",
        {
          key: "enter",
          modifiers: { ctrl: false, meta: false, shift: false, alt: false },
          is_composing: false
        },
        { autocomplete_open: false, send_enabled: true }
      )
    ).resolves.toBe("send");

    await api.updateSettings({
      keyboard: { composer_send_shortcut: "modEnter" }
    });

    await expect(
      api.resolveComposerKeyAction(
        "thread",
        {
          key: "enter",
          modifiers: { ctrl: false, meta: false, shift: false, alt: false },
          is_composing: false
        },
        { autocomplete_open: false, send_enabled: true }
      )
    ).resolves.toBe("insertNewline");

    await expect(
      api.resolveComposerKeyAction(
        "thread",
        {
          key: "enter",
          modifiers: { ctrl: true, meta: false, shift: false, alt: false },
          is_composing: false
        },
        { autocomplete_open: false, send_enabled: true }
      )
    ).resolves.toBe("send");
  });

  test("updates the Rust-shaped locale display profile from locale settings", async () => {
    const api = createBrowserFakeApi();

    const snapshot = await api.updateSettings({
      locale: { language_tag: "ar-XB", text_direction: "auto" }
    });

    expect(snapshot.state.locale_profile).toMatchObject({
      lang: "ar-XB",
      dir: "rtl",
      catalog_locale: "pseudo",
      pseudo_locale: "bidi",
      platform: "linux",
      modifier_labels: { primary: "Ctrl" }
    });
  });
});
