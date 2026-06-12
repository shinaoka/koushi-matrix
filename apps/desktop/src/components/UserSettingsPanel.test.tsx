import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { UserSettingsPanel } from "./UserSettingsPanel";

describe("UserSettingsPanel", () => {
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
        onOpenKeyboardSettings={() => undefined}
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
    expect(markup).toContain("Separate encrypted namespace");
    expect(markup).toContain("OS credential store");
  });
});
