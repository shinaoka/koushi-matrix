import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { UserSettingsPanel } from "./UserSettingsPanel";

describe("UserSettingsPanel", () => {
  const settings = {
    values: {
      locale: { language_tag: null, text_direction: "auto" },
      appearance: { theme: "dark" },
      typography: { font: "system", emoji: "system" },
      keyboard: { composer_send_shortcut: "enter" }
    },
    persistence: { kind: "idle" }
  } as const;

  test("renders account switch entries and keyboard settings access", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={{
          homeserver: "https://matrix.org",
          user_id: "@demo-user:example.invalid",
          device_id: "FAKEDEVICE"
        }}
        savedSessions={[
          {
            homeserver: "https://matrix.org",
            user_id: "@demo-user:example.invalid",
            device_id: "FAKEDEVICE"
          },
          {
            homeserver: "https://matrix.org",
            user_id: "@second-user:example.invalid",
            device_id: "SECONDDEVICE"
          }
        ]}
        settings={settings}
        onOpenKeyboardSettings={() => undefined}
        onUpdateSettings={() => undefined}
        onSwitchAccount={() => undefined}
      />
    );

    expect(markup).toContain("User settings");
    expect(markup).toContain("@demo-user:example.invalid");
    expect(markup).toContain("@second-user:example.invalid");
    expect(markup).toContain("Current");
    expect(markup).toContain("Switch");
    expect(markup).toContain("Keyboard");
    expect(markup).toContain("Session");
    expect(markup).toContain("Homeserver");
    expect(markup).toContain("Device");
    expect(markup).toContain("Local store");
    expect(markup).toContain("Appearance");
    expect(markup).toContain("Dark");
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain("Separate encrypted namespace");
    expect(markup).toContain("OS credential store");
  });

  test("renders saved sessions when the current session is unavailable", () => {
    const markup = renderToStaticMarkup(
      <UserSettingsPanel
        currentSession={null}
        savedSessions={[
          {
            homeserver: "https://matrix.org",
            user_id: "@second-user:example.invalid",
            device_id: "SECONDDEVICE"
          }
        ]}
        settings={settings}
        onOpenKeyboardSettings={() => undefined}
        onUpdateSettings={() => undefined}
        onSwitchAccount={() => undefined}
      />
    );

    expect(markup).toContain("Matrix account");
    expect(markup).toContain("Not restored");
    expect(markup).toContain("@second-user:example.invalid");
    expect(markup).toContain("Switch");
  });
});
