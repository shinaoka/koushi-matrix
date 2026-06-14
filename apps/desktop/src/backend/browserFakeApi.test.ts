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
});
