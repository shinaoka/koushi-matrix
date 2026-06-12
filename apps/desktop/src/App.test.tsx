import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi } from "./backend/browserFakeApi";
import type { RightPanelMode } from "./domain/rightPanel";

describe("ContextualRightPanel", () => {
  test("composer disables sending while a transaction is pending", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { Composer } = await import("./App");

    const markup = renderToStaticMarkup(
      <Composer
        isSending={true}
        roomName="Room Alpha"
        value="hello"
        onSend={() => undefined}
        onValueChange={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Sending"');
    expect(markup).toContain("disabled");
  });

  test("renders search results as a contextual right panel mode", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Alpha", "allRooms");

    const markup = renderToStaticMarkup(
      <ContextualRightPanel
        activeRoom={snapshot.state.rooms[0] ?? null}
        activeSpace={snapshot.state.spaces[0] ?? null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode="search"
        recoverySecretFilled={false}
        recoverySecretInputRef={{ current: null }}
        savedSessions={[]}
        searchQuery="Alpha"
        searchResults={
          snapshot.state.search.kind === "results" ? snapshot.state.search.results : []
        }
        snapshot={snapshot}
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenKeyboardSettings={() => undefined}
        onRecoverySecretPresenceChange={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
      />
    );

    expect(markup).toContain("Search");
    expect(markup).toContain("Alpha");
    expect(markup).toContain("keyword update");
    expect(markup).toContain("search-results");
  });

  test("renders encryption recovery as a contextual right panel mode", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi({ session: "needsRecovery" });
    const snapshot = await api.getSnapshot();

    const markup = renderToStaticMarkup(
      <ContextualRightPanel
        activeRoom={null}
        activeSpace={null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode={"recovery" as RightPanelMode}
        recoverySecretFilled={false}
        recoverySecretInputRef={{ current: null }}
        savedSessions={[]}
        searchQuery=""
        searchResults={[]}
        snapshot={snapshot}
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenKeyboardSettings={() => undefined}
        onRecoverySecretPresenceChange={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
      />
    );

    expect(markup).toContain("Encryption Recovery");
    expect(markup).toContain("Recovery key");
    expect(markup).toContain("Security phrase");
    expect(markup).toContain("thread-pane");
    expect(markup).not.toContain("recovery-screen");
  });
});

describe("Tauri state refresh wiring", () => {
  test("listens for backend state events and refreshes the snapshot", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).toContain("STATE_EVENT_NAME");
    expect(source).toContain("listen<string>(STATE_EVENT_NAME");
    expect(source).toContain("void refresh()");
  });

  test("keeps post-login recovery in the desktop render path", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).not.toMatch(
      /snapshot\.state\.session\.kind === "needsRecovery"[\s\S]{0,240}<RecoveryScreen/
    );
    expect(source).toContain("recoveryRequired");
    expect(source).toContain('"recovery"');
  });
});
