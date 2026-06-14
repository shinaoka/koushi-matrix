import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi } from "./backend/browserFakeApi";
import { TimelineItemRow } from "./components/TimelineView";
import type { TimelineItem } from "./domain/coreEvents";
import type { DesktopSnapshot } from "./domain/types";
import type { RightPanelMode } from "./domain/rightPanel";

describe("ContextualRightPanel", () => {
  test("composer disables sending while a transaction is pending", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { Composer } = await import("./App");

    const markup = renderToStaticMarkup(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={true}
        roomName="Room Alpha"
        value="hello"
        onCancelReply={() => undefined}
        onSend={() => undefined}
        onValueChange={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Sending"');
    expect(markup).toContain("disabled");
  });

  test("workspace rail exposes create space control", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { WorkspaceRail } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();

    const markup = renderToStaticMarkup(
      <WorkspaceRail
        snapshot={snapshot}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Create space"');
  });

  test("composer renders reply mode from snapshot state", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { Composer } = await import("./App");

    const markup = renderToStaticMarkup(
      <Composer
        composerMode={{ kind: "reply", in_reply_to_event_id: "$root" }}
        isSending={false}
        roomName="QA Room"
        value="reply"
        onCancelReply={() => undefined}
        onSend={() => undefined}
        onValueChange={() => undefined}
      />
    );

    expect(markup).toContain("Replying");
    expect(markup).toContain('aria-label="Cancel reply"');
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

  test("renders thread panel from Rust-owned state when legacy thread snapshot is null", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    snapshot.state.thread = {
      kind: "open",
      room_id: snapshot.state.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      is_subscribed: true,
      composer: { pending_transaction_id: null, draft: "", mode: "Plain" }
    };
    snapshot.thread = null;

    const markup = renderToStaticMarkup(
      <ContextualRightPanel
        activeRoom={snapshot.state.rooms[0] ?? null}
        activeSpace={snapshot.state.spaces[0] ?? null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode="thread"
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

    expect(markup).toContain("Thread");
    expect(markup).toContain("$root:example.invalid");
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

  test("feeds the effective right panel mode into the QA window title", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).toContain("desktopAttentionSummary");
    expect(source).toContain("desktopAttentionWindowTitle");
    expect(source).toContain("desktopAttentionNotificationCandidate");
    expect(source).toContain("sendDesktopAttentionNotification");
    expect(source).toContain("applyDesktopAttentionToWindow");
    expect(source).toContain("qaWindowTitle(");
    expect(source).toContain("effectiveRightPanelModeForSnapshot");
    expect(source).toContain("rightPanelMode");
    expect(source).toContain("qaSendStatus");
    expect(source).toContain("getCurrentWindow()");
    expect(source).toContain("document.title = title");
    expect(source).toContain("desktopAttentionWindowTitle");
  });
});

describe("TopBar sync state rendering", () => {
  test("renders reconnecting and failed states with a restart control", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { TopBar } = await import("./App");
    const baseProps = {
      activeSpaceName: "Matrix",
      isBusy: false,
      searchInputRef: { current: null },
      searchQuery: "",
      searchScope: "allRooms" as const,
      onOpenKeyboardSettings: () => undefined,
      onRestartSync: () => undefined,
      onSearchQueryChange: () => undefined,
      onSearchScopeChange: () => undefined
    };

    const reconnectingMarkup = renderToStaticMarkup(
      <TopBar
        {...baseProps}
        sync={
          {
            reconnecting: "sync service is unavailable"
          } as DesktopSnapshot["state"]["sync"]
        }
      />
    );
    expect(reconnectingMarkup).toContain("Reconnecting");
    expect(reconnectingMarkup).toContain("sync service is unavailable");
    expect(reconnectingMarkup).toContain('aria-label="Restart sync"');

    const failedMarkup = renderToStaticMarkup(
      <TopBar
        {...baseProps}
        sync={
          {
            failed: "transport error"
          } as DesktopSnapshot["state"]["sync"]
        }
      />
    );
    expect(failedMarkup).toContain("Failed");
    expect(failedMarkup).toContain("transport error");
    expect(failedMarkup).toContain('aria-label="Restart sync"');
  });
});

describe("Timeline item row rendering", () => {
  test("marks Transaction timeline items as unsent local echoes", () => {
    const localEcho = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Transaction: { transaction_id: "desktop-1" } },
            sender: "@me:example.invalid",
            body: "queued message",
            timestamp_ms: 1_820_000_000_000,
            in_reply_to_event_id: null
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
      />
    );

    expect(localEcho).toContain('data-send-state="unsent"');
    expect(localEcho).toContain("Unsent");

    const remoteEvent = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$remote" } },
            sender: "@me:example.invalid",
            body: "sent message",
            timestamp_ms: 1_820_000_000_100,
            in_reply_to_event_id: null
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
      />
    );

    expect(remoteEvent).not.toContain('data-send-state="unsent"');
    expect(remoteEvent).not.toContain("Unsent");
  });
});
