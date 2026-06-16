import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { KeyboardSettingsPanel } from "./KeyboardSettingsPanel";

describe("KeyboardSettingsPanel", () => {
  test("renders Element-like shortcut groups and parity statuses", () => {
    const markup = renderToStaticMarkup(
      <KeyboardSettingsPanel
        labelProfile={{ platform: "windows", modLabel: "Ctrl" }}
        settings={{
          values: {
            locale: { language_tag: null, text_direction: "auto" },
            appearance: { theme: "system" },
            typography: { font: "system", emoji: "system" },
            keyboard: { composer_send_shortcut: "enter" },
            notifications: { desktop_notifications: true, sound: true, badges: true },
            display: { code_block_wrap: true, hide_redacted: false },
            media: { image_upload_compression: "never" }
          },
          persistence: { kind: "idle" }
        }}
        onUpdateSettings={() => undefined}
      />
    );

    expect(markup).toContain("Keyboard");
    expect(markup).toContain("Composer");
    expect(markup).toContain("Room List");
    expect(markup).toContain("Ctrl/Cmd");
    expect(markup).toContain("Keyboard settings");
    expect(markup).toContain("same");
    expect(markup).toContain("deferred");
  });

  test("renders the Rust-owned composer send shortcut selection", () => {
    const markup = renderToStaticMarkup(
      <KeyboardSettingsPanel
        labelProfile={{ platform: "macos", modLabel: "Cmd" }}
        settings={{
          values: {
            locale: { language_tag: null, text_direction: "auto" },
            appearance: { theme: "system" },
            typography: { font: "system", emoji: "system" },
            keyboard: { composer_send_shortcut: "modEnter" },
            notifications: { desktop_notifications: true, sound: true, badges: true },
            display: { code_block_wrap: true, hide_redacted: false },
            media: { image_upload_compression: "never" }
          },
          persistence: { kind: "saving", request_id: 7 }
        }}
        onUpdateSettings={() => undefined}
      />
    );

    expect(markup).toContain("Composer send shortcut");
    expect(markup).toContain("Enter sends");
    expect(markup).toContain("Cmd+Enter sends");
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain("Saving");
  });
});
