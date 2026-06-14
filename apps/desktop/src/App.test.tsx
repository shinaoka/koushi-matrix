import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi } from "./backend/browserFakeApi";
import { TimelineItemRow } from "./components/TimelineView";
import type { TimelineItem } from "./domain/coreEvents";
import type { DesktopSnapshot } from "./domain/types";
import type { RightPanelMode } from "./domain/rightPanel";
import { t } from "./i18n/messages";

describe("ContextualRightPanel", () => {
  const trustPanelHandlers = {
    onAcceptVerification: () => undefined,
    onBootstrapCrossSigning: () => undefined,
    onCancelVerification: () => undefined,
    onConfirmSasVerification: () => undefined,
    onEnableKeyBackup: () => undefined,
    onResetIdentity: () => undefined,
    onSubmitIdentityResetOAuth: () => undefined,
    onSubmitIdentityResetPassword: () => undefined
  };

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

  test("composer exposes an attach file control separately from text send", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { Composer } = await import("./App");

    const markup = renderToStaticMarkup(
      <Composer
        composerMode={{ kind: "plain" }}
        isSending={false}
        roomName="Room Alpha"
        value=""
        onCancelReply={() => undefined}
        onSend={() => undefined}
        onValueChange={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Attach file"');
    expect(markup).toContain('type="file"');
    expect(markup).toContain('aria-label="Attach file input"');
    expect(markup).toContain('aria-label="Send"');
  });

  test("TimelineItemRow renders reaction pills with accessible labels", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Event: { event_id: "$event:example.invalid" } },
          sender: "@alice:example.invalid",
          body: "Hello",
          timestamp_ms: 1_800_000_000_000,
          in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
          can_react: true,
          is_redacted: false,
          can_redact: false,
          is_edited: false,
          can_edit: true,
          reactions: [
            {
              key: "👍",
              count: 2,
              reacted_by_me: true,
              my_reaction_event_id: "$reaction:example.invalid",
              sender_preview: ["@alice:example.invalid"]
            }
          ]
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Reaction 👍, count 2"');
    expect(markup).toContain('class="reaction-pill"');
    expect(markup).toContain('data-reacted-by-me="true"');
    expect(markup).toContain('type="button"');
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain('dir="auto"');
  });

  test("TimelineItemRow renders add reaction affordance only for reactable events", () => {
    const reactableMarkup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Event: { event_id: "$event:example.invalid" } },
          sender: "@alice:example.invalid",
          body: "Hello",
          timestamp_ms: 1_800_000_000_000,
          in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
          can_react: true,
          is_redacted: false,
          can_redact: false,
          is_edited: false,
          can_edit: false,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    const nonReactableMarkup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Synthetic: { synthetic_id: "divider" } },
          sender: null,
          body: null,
          timestamp_ms: null,
          in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
          can_react: false,
          is_redacted: false,
          can_redact: false,
          is_edited: false,
          can_edit: false,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(reactableMarkup).toContain('aria-label="Add reaction"');
    expect(nonReactableMarkup).not.toContain('aria-label="Add reaction"');
  });

  test("TimelineItemRow renders media metadata from Rust-owned timeline DTOs", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Event: { event_id: "$media:example.invalid" } },
          sender: "@alice:example.invalid",
          body: "Project notes",
          timestamp_ms: 1_800_000_000_000,
          in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
          media: {
            kind: "File",
            filename: "release-notes.pdf",
            source: {
              mxc_uri: "mxc://example.invalid/private-file",
              encrypted: true,
              encryption_version: "v2"
            },
            mimetype: "application/pdf",
            size: 1024,
            width: null,
            height: null,
            thumbnail: null
          },
          can_react: true,
          is_redacted: false,
          can_redact: false,
          is_edited: false,
          can_edit: true,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('class="message-media"');
    expect(markup).toContain("release-notes.pdf");
    expect(markup).toContain("application/pdf");
    expect(markup).toContain("1 KB");
    expect(markup).toContain('aria-label="Download release-notes.pdf"');
    expect(markup).not.toContain("mxc://example.invalid/private-file");
  });

  test("TimelineItemRow renders redaction affordance and redacted placeholder", () => {
    const redactableMarkup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$event:example.invalid" } },
            sender: "@alice:example.invalid",
            body: "Visible message",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
            can_react: true,
            is_redacted: false,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    const redactedMarkup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$redacted:example.invalid" } },
            sender: "@alice:example.invalid",
            body: "Hidden message",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
            can_react: true,
            is_redacted: true,
            can_redact: true,
            is_edited: true,
            can_edit: true,
            reactions: [
              {
                key: "👍",
                count: 2,
                reacted_by_me: true,
                my_reaction_event_id: null,
                sender_preview: ["@alice:example.invalid"]
              }
            ]
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(redactableMarkup).toContain(`aria-label="${t("timeline.redactMessage")}"`);
    expect(redactedMarkup).toContain(t("timeline.redactedMessage"));
    expect(redactedMarkup).not.toContain("Hidden message");
    expect(redactedMarkup).not.toContain('Reaction 👍, count 2');
    expect(redactedMarkup).not.toContain('class="reaction-pill"');
    expect(redactedMarkup).not.toContain("Edited");
    expect(redactedMarkup).not.toContain(t("timeline.editedMessage"));
    expect(redactedMarkup).not.toContain(`aria-label="${t("timeline.replyToMessage")}"`);
    expect(redactedMarkup).not.toContain(`aria-label="${t("timeline.addReaction")}"`);
    expect(redactedMarkup).not.toContain(`aria-label="${t("timeline.redactMessage")}"`);
  });

  test("TimelineItemRow renders edit affordance and edited marker for editable messages", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$edit:example.invalid" } },
            sender: "@alice:example.invalid",
            body: "Visible message",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
            can_react: true,
            is_redacted: false,
            can_redact: true,
            is_edited: true,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Edit message"');
    expect(markup).toContain("Edited");
  });

  test("TimelineItemRow suppresses edit affordance for redacted rows", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$redacted-edit:example.invalid" } },
            sender: "@alice:example.invalid",
            body: "Hidden message",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
            can_react: true,
            is_redacted: true,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).not.toContain('aria-label="Edit message"');
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
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain("Search");
    expect(markup).toContain("Alpha");
    expect(markup).toContain("keyword update");
    expect(markup).toContain("search-results");
  });

  test("renders focused search context from Rust-owned snapshot state", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Alpha", "allRooms");
    snapshot.state.focused_context = {
      kind: "open",
      room_id: snapshot.state.search.kind === "results" ? snapshot.state.search.results[0]?.room_id ?? "!room:example.invalid" : "!room:example.invalid",
      event_id:
        snapshot.state.search.kind === "results"
          ? snapshot.state.search.results[0]?.event_id ?? "$focused:example.invalid"
          : "$focused:example.invalid",
      is_subscribed: true
    };

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
          snapshot.state.search.kind === "results"
            ? snapshot.state.search.results
          : []
        }
        snapshot={snapshot}
        timelineTransport={
          {
            listenCoreEvents: () => () => undefined,
            paginateBackwards: async (timelineKey) => {
              void timelineKey;
            },
            toggleReaction: async () => undefined,
            editMessage: async () => undefined,
            redactMessage: async () => undefined,
            downloadMedia: async () => undefined
          } as const
        }
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenKeyboardSettings={() => undefined}
        onRecoverySecretPresenceChange={() => undefined}
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain(t("panel.focusedContext"));
    expect(markup).toContain('data-testid="timeline-view"');
  });

  test("renders focusedContext mode as a focused TimelineView without search results", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.submitSearch("Alpha", "allRooms");
    snapshot.state.focused_context = {
      kind: "open",
      room_id: "!room-alpha:example.invalid",
      event_id: "$focused:example.invalid",
      is_subscribed: true
    };

    const markup = renderToStaticMarkup(
      <ContextualRightPanel
        activeRoom={snapshot.state.rooms[0] ?? null}
        activeSpace={snapshot.state.spaces[0] ?? null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode="focusedContext"
        recoverySecretFilled={false}
        recoverySecretInputRef={{ current: null }}
        savedSessions={[]}
        searchQuery="Alpha"
        searchResults={
          snapshot.state.search.kind === "results"
            ? snapshot.state.search.results
            : []
        }
        snapshot={snapshot}
        timelineTransport={
          {
            listenCoreEvents: () => () => undefined,
            paginateBackwards: async () => undefined,
            toggleReaction: async () => undefined,
            editMessage: async () => undefined,
            redactMessage: async () => undefined,
            downloadMedia: async () => undefined
          } as const
        }
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenKeyboardSettings={() => undefined}
        onRecoverySecretPresenceChange={() => undefined}
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain(t("panel.focusedContext"));
    expect(markup).toContain('data-testid="timeline-view"');
    expect(markup).not.toContain("search-results");
    expect(markup).not.toContain("keyword update");
    expect(markup).not.toContain('aria-label="Thread composer"');
  });

  test("renders thread panel as a keyed TimelineView from Rust-owned state", async () => {
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
    snapshot.timeline = [
      {
        room_id: snapshot.state.rooms[0]?.room_id ?? "!room:example.invalid",
        event_id: "$root:example.invalid",
        sender: "@legacy:example.invalid",
        timestamp_ms: 1_800_000_000_000,
        body: "Legacy room timeline root",
        attachment_filename: null,
        reply_count: 1
      }
    ];
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
        timelineTransport={
          {
            listenCoreEvents: () => () => undefined,
            paginateBackwards: async (timelineKey) => {
              void timelineKey;
            },
            toggleReaction: async () => undefined,
            editMessage: async () => undefined,
            redactMessage: async () => undefined,
            downloadMedia: async () => undefined
          } as const
        }
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenKeyboardSettings={() => undefined}
        onRecoverySecretPresenceChange={() => undefined}
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain(t("panel.thread"));
    expect(markup).toContain('data-testid="timeline-view"');
    expect(markup).not.toContain("Legacy room timeline root");
    expect(markup).not.toContain("$root:example.invalid");
  });

  test("thread composer renders Rust-owned draft and enables send only when not pending", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    snapshot.state.thread = {
      kind: "open",
      room_id: snapshot.state.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      is_subscribed: true,
      composer: {
        pending_transaction_id: null,
        draft: "Rust-owned draft",
        mode: "Plain"
      }
    };

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
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Thread composer"');
    expect(markup).toContain("Rust-owned draft");
    expect(markup).toContain('aria-label="Send"');
    expect(markup).not.toContain('aria-label="Sending"');
  });

  test("thread composer disables send while the Rust-owned composer is pending", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    snapshot.state.thread = {
      kind: "open",
      room_id: snapshot.state.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      is_subscribed: true,
      composer: {
        pending_transaction_id: "txn-thread-1",
        draft: "Draft blocked by pending send",
        mode: "Plain"
      }
    };

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
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Sending"');
    expect(markup).toContain("disabled");
  });

  test("thread render path does not read legacy snapshot timeline or replies", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const threadBranchStart = source.indexOf("const threadState = snapshot.state.thread;");
    const threadBranchEnd = source.indexOf("function PanelHeader", threadBranchStart);
    const threadBranch = source.slice(threadBranchStart, threadBranchEnd);

    expect(threadBranch).toContain("threadTimelineKey(");
    expect(threadBranch).not.toContain("snapshot.timeline");
    expect(threadBranch).not.toContain("snapshot.thread");
    expect(threadBranch).not.toContain(".replies");
  });

  test("Tauri timeline transport routes thread pagination by TimelineKey", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const transportStart = source.indexOf("const tauriTimelineTransport");
    const transportEnd = source.indexOf("const tauriNotificationTransport", transportStart);
    const transportBranch = source.slice(transportStart, transportEnd);

    expect(transportBranch).toContain("paginate_timeline_backwards");
    expect(transportBranch).toContain("paginate_thread_timeline_backwards");
    expect(transportBranch).toContain("rootEventId");
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
        onReply={() => undefined}
        onResultSelect={() => undefined}
        onSubmitRecovery={(event) => event.preventDefault()}
        onSwitchAccount={() => undefined}
        {...trustPanelHandlers}
        onThreadComposerDraftChange={() => undefined}
        onThreadReplySend={() => undefined}
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

  test("search result selection is snapshot-driven and does not scroll the DOM", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const selectSearchResultStart = source.indexOf("function selectSearchResult");
    const selectSearchResultEnd = source.indexOf("function runContextMenuAction");
    const selectSearchResultSource = source.slice(selectSearchResultStart, selectSearchResultEnd);

    expect(selectSearchResultSource).toContain("api.selectSearchResult(roomId, eventId)");
    expect(selectSearchResultSource).toContain('setRightPanelMode("search")');
    expect(selectSearchResultSource).not.toContain("selectRoom(");
    expect(selectSearchResultSource).not.toContain('setSearchQuery("")');
    expect(selectSearchResultSource).not.toContain("document.querySelector");
    expect(selectSearchResultSource).not.toContain("scrollIntoView");
    expect(selectSearchResultSource).not.toContain("cssEscape");
  });

  test("closing an active focused context goes through Rust before hiding the panel", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const closeFocusedContextStart = source.indexOf("async function closeFocusedContextIfHiddenBy");
    const closeFocusedContextEnd = source.indexOf("async function closeFocusedContextPanel", closeFocusedContextStart);
    const closeFocusedContextSource = source.slice(closeFocusedContextStart, closeFocusedContextEnd);

    expect(closeFocusedContextStart).toBeGreaterThanOrEqual(0);
    expect(closeFocusedContextSource).toContain("api.closeFocusedContext()");
    expect(closeFocusedContextSource).toContain("focusedContextVisibleForMode(rightPanelMode)");
    expect(closeFocusedContextSource).toContain("!focusedContextVisibleForMode(nextMode)");

    const modeHelperStart = source.indexOf("async function setRightPanelModeClosingFocusedContext");
    const modeHelperEnd = source.indexOf("async function closeFocusedContextPanel", modeHelperStart);
    const modeHelperSource = source.slice(modeHelperStart, modeHelperEnd);
    expect(modeHelperSource).toContain("await closeFocusedContextIfHiddenBy(nextMode)");
    expect(modeHelperSource).toContain("setRightPanelMode(nextMode)");

    const renderStart = source.indexOf("<ContextualRightPanel");
    const renderEnd = source.indexOf("</ContextualRightPanel>", renderStart);
    const panelPropsSource = source.slice(renderStart, renderEnd);
    expect(panelPropsSource).toContain("closeFocusedContextPanel");
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
            in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
            can_react: false,
            is_redacted: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
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
            in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
            can_react: true,
            is_redacted: false,
            can_redact: false,
            is_edited: false,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onToggleReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(remoteEvent).not.toContain('data-send-state="unsent"');
    expect(remoteEvent).not.toContain("Unsent");
  });
});
