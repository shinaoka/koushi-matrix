import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

import { KeyboardSettingsPanel } from "./KeyboardSettingsPanel";

describe("KeyboardSettingsPanel", () => {
  test("renders Element-like shortcut groups and parity statuses", () => {
    const markup = renderToStaticMarkup(<KeyboardSettingsPanel />);

    expect(markup).toContain("Keyboard");
    expect(markup).toContain("Composer");
    expect(markup).toContain("Room List");
    expect(markup).toContain("Ctrl/Cmd");
    expect(markup).toContain("Keyboard settings");
    expect(markup).toContain("same");
    expect(markup).toContain("deferred");
  });
});
