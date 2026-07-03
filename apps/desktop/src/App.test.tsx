import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, test, vi } from "vitest";

import { createBrowserFakeApi } from "./backend/browserFakeApi";
import { MessageSourceDialog, TimelineItemRow } from "./components/TimelineView";
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
    onExportRoomKeys: () => undefined,
    onImportRoomKeys: () => undefined,
    onBootstrapSecureBackup: () => undefined,
    onChangeSecureBackupPassphrase: () => undefined,
    onOpenRecovery: () => undefined,
    onProbeLocalEncryption: () => undefined,
    onResetLocalData: () => undefined,
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
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Home"');
    expect(markup).not.toContain('aria-label="Activity"');
    expect(markup).toContain('role="separator"');
    expect(markup).toContain('aria-label="Create space"');
  });

  test("workspace rail renders Rust-projected space attention counts", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { WorkspaceRail } = await import("./App");
    const api = createBrowserFakeApi();
    const snapshot = await api.getSnapshot();
    snapshot.sidebar.space_rail = [
      {
        space_id: "!ops:example.invalid",
        display_name: "Ops Space",
        avatar: null,
        unread_count: 13,
        highlight_count: 2,
        is_active: false
      }
    ];

    const markup = renderToStaticMarkup(
      <WorkspaceRail
        snapshot={snapshot}
        onCreateSpace={() => undefined}
        onOpenContextMenu={() => undefined}
        onOpenUserSettings={() => undefined}
        onReorderSpaces={() => undefined}
        onSelectSpace={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Ops Space"');
    expect(markup).toContain("draggable");
    expect(markup).toContain("compact-label");
    expect(markup).toContain("Ops Space");
    expect(markup).toContain('data-count="13"');
    expect(markup).not.toContain('data-mention-count="2"');
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
          is_hidden: false,
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
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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

  test("TimelineItemRow renders mention pills from Rust-owned profile data", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Event: { event_id: "$event:example.invalid" } },
          sender: "@alice:example.invalid",
          body: "Hello @Alice Alias",
          timestamp_ms: 1_800_000_000_000,
          in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: null,
          can_react: true,
          is_redacted: false,
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: true,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
        mentionProfileUsers={{
          "@alice:example.invalid": {
            user_id: "@alice:example.invalid",
            display_name: "Alice Upstream",
            display_label: "Alice Alias",
            original_display_label: "Alice Upstream",
            mention_search_terms: ["Alice Alias", "Alice Upstream", "@alice:example.invalid"],
            avatar: null
          }
        }}
      />
    );

    expect(markup).toContain('class="message-mention-pill"');
    expect(markup).toContain('data-mention-user-id="@alice:example.invalid"');
    expect(markup).toContain("@Alice Alias");
  });

  test("TimelineItemRow renders thread summary from Rust-owned row data", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Event: { event_id: "$root:example.invalid" } },
          sender: "@alice:example.invalid",
          body: "Root message",
          timestamp_ms: 1_800_000_000_000,
          in_reply_to_event_id: null,
          thread_root: null,
          thread_summary: {
            reply_count: 3,
            latest_sender: "@bob:example.invalid",
            latest_body_preview: "Latest thread reply",
            latest_timestamp_ms: 1_800_000_100_000
          },
          can_react: true,
          is_redacted: false,
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: true,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
        onOpenThread={() => undefined}
      />
    );

    expect(markup).toContain('class="thread-summary-chip"');
    expect(markup).toContain("3 replies");
    expect(markup).toContain("@bob:example.invalid: Latest thread reply");
    expect(markup).toContain('aria-label="Open thread, 3 replies');
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
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: false,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: false,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: true,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('class="message-media"');
    expect(markup).toContain("release-notes.pdf");
    expect(markup).toContain("application/pdf");
    expect(markup).toContain("1 KB");
    expect(markup).toContain('aria-label="Download release-notes.pdf"');
    expect(markup.indexOf('class="message-media"')).toBeLessThan(markup.indexOf("Project notes"));
    expect(markup).not.toContain("mxc://example.invalid/private-file");
  });

  test("TimelineItemRow renders Rust-owned formatted bodies and code block controls", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={{
          id: { Event: { event_id: "$formatted:example.invalid" } },
          sender: "@alice:example.invalid",
          body: "plain fallback should not render when formatted exists",
          timestamp_ms: 1_800_000_000_000,
          in_reply_to_event_id: null,
          formatted: {
            html:
              '<strong>Bold body</strong><blockquote>Quoted body</blockquote><ul><li>List item</li></ul><a href="https://example.invalid/path">safe link</a><pre><code class="language-rust">fn main() {}</code></pre>',
            plain_text: "Bold bodyQuoted bodyList itemsafe linkfn main() {}",
            code_blocks: [{ language: "rust", body: "fn main() {}" }]
          },
          thread_root: null,
          thread_summary: null,
          can_react: true,
          is_redacted: false,
          is_hidden: false,
          can_redact: false,
          is_edited: false,
          can_edit: true,
          reactions: []
        }}
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('class="message-body message-formatted-body"');
    expect(markup).toContain("<strong>Bold body</strong>");
    expect(markup).toContain("<blockquote>Quoted body</blockquote>");
    expect(markup).toContain("<li>List item</li>");
    expect(markup).toContain('href="https://example.invalid/path"');
    expect(markup).toContain('class="message-code-block"');
    expect(markup).toContain('data-code-block-wrap="true"');
    expect(markup).toContain('class="language-rust"');
    expect(markup).toContain('aria-label="Copy code"');
    expect(markup).not.toContain("plain fallback should not render");
  });

  test("TimelineItemRow renders Rust-owned message kind and spoiler contracts", () => {
    const baseItem = {
      id: { Event: { event_id: "$message-types:example.invalid" } },
      sender: "@alice:example.invalid",
      sender_label: "Alice Alias",
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: true,
      reactions: []
    } satisfies Partial<TimelineItem>;
    const renderRow = (item: TimelineItem) =>
      renderToStaticMarkup(
        <TimelineItemRow
          item={item}
          roomId="!room:example.invalid"
          onReply={() => undefined}
          onSendReaction={() => undefined}
          onRedactReaction={() => undefined}
          onEdit={() => undefined}
          onRedact={() => undefined}
        />
      );

    const emoteMarkup = renderRow({
      ...baseItem,
      body: "waves",
      message_kind: "emote"
    } as TimelineItem);
    expect(emoteMarkup).toContain('data-message-kind="emote"');
    expect(emoteMarkup).toContain('class="message-emote-prefix"');
    expect(emoteMarkup).toContain("Alice Alias");
    expect(emoteMarkup).toContain("waves");

    const noticeMarkup = renderRow({
      ...baseItem,
      body: "bot notice",
      message_kind: "notice"
    } as TimelineItem);
    expect(noticeMarkup).toContain('data-message-kind="notice"');
    expect(noticeMarkup).toContain("message-notice");

    const spoilerMarkup = renderRow({
      ...baseItem,
      body: "keep secret hidden",
      spoiler_spans: [{ start_utf16: 5, end_utf16: 11 }],
      message_kind: "text"
    } as TimelineItem);
    expect(spoilerMarkup).toContain('class="message-spoiler"');
    expect(spoilerMarkup).toContain('data-revealed="false"');
    expect(spoilerMarkup).toContain(t("timeline.spoiler"));
    expect(spoilerMarkup).not.toContain("secret");

    const formattedSpoilerMarkup = renderRow({
      ...baseItem,
      body: "plain fallback",
      formatted: {
        html: 'keep <span data-mx-spoiler="reason">secret</span> hidden',
        plain_text: "keep secret hidden",
        code_blocks: []
      },
      spoiler_spans: [{ start_utf16: 5, end_utf16: 11, reason: "reason" }],
      message_kind: "text"
    } as TimelineItem);
    expect(formattedSpoilerMarkup).toContain('data-spoiler-reason="reason"');
    expect(formattedSpoilerMarkup).not.toContain("secret");
  });

  test("TimelineItemRow reflects the Rust-owned code block wrap preference", () => {
    const item = {
      id: { Event: { event_id: "$formatted-nowrap:example.invalid" } },
      sender: "@alice:example.invalid",
      body: "plain fallback",
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      formatted: {
        html: '<pre><code class="language-rust">let long_line = "value";</code></pre>',
        plain_text: 'let long_line = "value";',
        code_blocks: [{ language: "rust", body: 'let long_line = "value";' }]
      },
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: true,
      reactions: []
    } as TimelineItem;

    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={item}
        roomId="!room:example.invalid"
        codeBlockWrap={false}
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('data-code-block-wrap="false"');
  });

  test("TimelineItemRow preserves search highlighting over formatted text", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$formatted-search:example.invalid" } },
            sender: "@alice:example.invalid",
            body: "plain fallback",
            timestamp_ms: 1_800_000_000_000,
            in_reply_to_event_id: null,
            formatted: {
              html: "<strong>Formatted keyword body</strong>",
              plain_text: "Formatted keyword body",
              code_blocks: []
            },
            thread_root: null,
            thread_summary: null,
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        searchQuery="keyword"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain("<strong>Formatted <mark>keyword</mark> body</strong>");
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
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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
            is_hidden: false,
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
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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
            is_hidden: false,
            can_redact: true,
            is_edited: true,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
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
        activeRoom={snapshot.state.domain.rooms[0] ?? null}
        activeSpace={snapshot.state.domain.spaces[0] ?? null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode="search"
        recoverySecretFilled={false}
        recoverySecretInputRef={{ current: null }}
        savedSessions={[]}
        searchQuery="Alpha"
        searchResults={
          snapshot.state.domain.search.kind === "results" ? snapshot.state.domain.search.results : []
        }
        snapshot={snapshot}
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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
    snapshot.state.ui.focused_context = {
      kind: "open",
      room_id: snapshot.state.domain.search.kind === "results" ? snapshot.state.domain.search.results[0]?.room_id ?? "!room:example.invalid" : "!room:example.invalid",
      event_id:
        snapshot.state.domain.search.kind === "results"
          ? snapshot.state.domain.search.results[0]?.event_id ?? "$focused:example.invalid"
          : "$focused:example.invalid",
      is_subscribed: true
    };

    const markup = renderToStaticMarkup(
      <ContextualRightPanel
        activeRoom={snapshot.state.domain.rooms[0] ?? null}
        activeSpace={snapshot.state.domain.spaces[0] ?? null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode="search"
        recoverySecretFilled={false}
        recoverySecretInputRef={{ current: null }}
        savedSessions={[]}
        searchQuery="Alpha"
        searchResults={
          snapshot.state.domain.search.kind === "results"
            ? snapshot.state.domain.search.results
          : []
        }
        snapshot={snapshot}
        timelineTransport={
          {
            listenCoreEvents: () => () => undefined,
            paginateBackwards: async (timelineKey) => {
              void timelineKey;
            },
            sendReaction: async () => undefined,
            retrySend: async () => undefined,
            cancelSend: async () => undefined,
            redactReaction: async () => undefined,
            sendReadReceipt: async () => undefined,
            setFullyRead: async () => undefined,
            setTyping: async () => undefined,
            editMessage: async () => undefined,
            redactMessage: async () => undefined,
            pinEvent: async () => undefined,
            unpinEvent: async () => undefined,
            downloadMedia: async () => undefined,
            loadMessageSource: async () => undefined,
            requestRoomKey: async () => undefined,
            forwardMessage: async () => undefined,
            loadLinkPreviews: async () => undefined,
            hideLinkPreview: async () => undefined
          } as const
        }
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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
    snapshot.state.ui.focused_context = {
      kind: "open",
      room_id: "!room-alpha:example.invalid",
      event_id: "$focused:example.invalid",
      is_subscribed: true
    };

    const markup = renderToStaticMarkup(
      <ContextualRightPanel
        activeRoom={snapshot.state.domain.rooms[0] ?? null}
        activeSpace={snapshot.state.domain.spaces[0] ?? null}
        activeSpaceName="Home"
        isRecoveryBusy={false}
        mode="focusedContext"
        recoverySecretFilled={false}
        recoverySecretInputRef={{ current: null }}
        savedSessions={[]}
        searchQuery="Alpha"
        searchResults={
          snapshot.state.domain.search.kind === "results"
            ? snapshot.state.domain.search.results
            : []
        }
        snapshot={snapshot}
        timelineTransport={
          {
            listenCoreEvents: () => () => undefined,
            paginateBackwards: async () => undefined,
            sendReaction: async () => undefined,
            retrySend: async () => undefined,
            cancelSend: async () => undefined,
            redactReaction: async () => undefined,
            sendReadReceipt: async () => undefined,
            setFullyRead: async () => undefined,
            setTyping: async () => undefined,
            editMessage: async () => undefined,
            redactMessage: async () => undefined,
            pinEvent: async () => undefined,
            unpinEvent: async () => undefined,
            downloadMedia: async () => undefined,
            loadMessageSource: async () => undefined,
            requestRoomKey: async () => undefined,
            forwardMessage: async () => undefined,
            loadLinkPreviews: async () => undefined,
            hideLinkPreview: async () => undefined
          } as const
        }
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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
    snapshot.state.ui.thread = {
      kind: "open",
      room_id: snapshot.state.domain.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      is_subscribed: true,
      composer: { pending_transaction_id: null, draft: "", mode: "Plain" }
    };
    snapshot.timeline = [
      {
        room_id: snapshot.state.domain.rooms[0]?.room_id ?? "!room:example.invalid",
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
        activeRoom={snapshot.state.domain.rooms[0] ?? null}
        activeSpace={snapshot.state.domain.spaces[0] ?? null}
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
            sendReaction: async () => undefined,
            retrySend: async () => undefined,
            cancelSend: async () => undefined,
            redactReaction: async () => undefined,
            sendReadReceipt: async () => undefined,
            setFullyRead: async () => undefined,
            setTyping: async () => undefined,
            editMessage: async () => undefined,
            redactMessage: async () => undefined,
            pinEvent: async () => undefined,
            unpinEvent: async () => undefined,
            downloadMedia: async () => undefined,
            loadMessageSource: async () => undefined,
            requestRoomKey: async () => undefined,
            forwardMessage: async () => undefined,
            loadLinkPreviews: async () => undefined,
            hideLinkPreview: async () => undefined
          } as const
        }
        onClosePanel={() => undefined}
        onCloseThread={() => undefined}
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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
    snapshot.state.ui.thread = {
      kind: "open",
      room_id: snapshot.state.domain.rooms[0]?.room_id,
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
        activeRoom={snapshot.state.domain.rooms[0] ?? null}
        activeSpace={snapshot.state.domain.spaces[0] ?? null}
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
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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
    snapshot.state.ui.thread = {
      kind: "open",
      room_id: snapshot.state.domain.rooms[0]?.room_id,
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
        activeRoom={snapshot.state.domain.rooms[0] ?? null}
        activeSpace={snapshot.state.domain.spaces[0] ?? null}
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
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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

  test("thread render path keeps Tauri transport ahead of browser fixture fallback", () => {
    const source = readFileSync(new URL("./components/rightPanel.tsx", import.meta.url), "utf8");
    const threadBranchStart = source.indexOf("const threadState = snapshot.state.ui.thread;");
    const threadBranchEnd = source.indexOf("function PanelHeader", threadBranchStart);
    const threadBranch = source.slice(threadBranchStart, threadBranchEnd);
    const transportOffset = threadBranch.indexOf("threadTimelineKeyValue && threadRoomId && timelineTransport");
    const fallbackOffset = threadBranch.indexOf("browserThreadSnapshot ?");

    expect(threadBranch).toContain("threadTimelineKey(");
    expect(threadBranch).toContain("!timelineTransport");
    expect(threadBranch).toContain("snapshot.thread");
    expect(threadBranch).toContain("threadReplyToTimelineMessage(reply)");
    expect(threadBranch).not.toContain("snapshot.timeline");
    expect(transportOffset).toBeGreaterThanOrEqual(0);
    expect(fallbackOffset).toBeGreaterThan(transportOffset);
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

  test("TimelinePane no longer renders the older messages header control", () => {
    const source = readFileSync(new URL("./components/panes.tsx", import.meta.url), "utf8");
    const roomPaneStart = source.indexOf("export function TimelinePane");
    const roomPane = source.slice(roomPaneStart);

    expect(roomPane).toContain("roomTimelineKey(currentUserId, timelineRoomId)");
    expect(roomPane).not.toContain('aria-label={t("timeline.olderMessages")}');
    expect(roomPane).not.toContain("timelineTransport.paginateBackwards");
    expect(roomPane).not.toContain("<ArrowUp");
    expect(roomPane).not.toContain("canPaginateOlderMessages");
  });

  test("Tauri timeline ensure waits for the webview CoreEvent listener registration", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const transportStart = source.indexOf("const tauriTimelineTransport");
    const transportEnd = source.indexOf("const tauriNotificationTransport", transportStart);
    const transportBranch = source.slice(transportStart, transportEnd);

    expect(source).toContain("let tauriCoreEventListenerReady");
    expect(transportBranch).toContain("tauriCoreEventListenerReady = listen<CoreEventPayload>");
    expect(transportBranch).toContain("async ensureSubscribed");
    expect(transportBranch).toContain("await tauriCoreEventListenerReady");
    expect(transportBranch).toContain("ensure_timeline_subscribed");
  });

  test("room composer draft input stays local and persists to Rust with debounce", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).toContain("const localComposerDraftsRef = useRef<Record<string, string>>({});");
    expect(source).toContain("composerDraftPersistTimer");
    expect(source).toContain("queueComposerDraftPersist(roomId, value)");
    expect(source).toContain("updateComposerTypingSignal(roomId, value)");
    expect(source).toContain("localComposerDraftsRef.current[roomId] = value;");
    expect(source).not.toContain("if (value) {\n      localComposerDraftsRef.current[roomId] = value;");
    expect(source).toContain("window.setTimeout");
    expect(source).toContain("async function sendText(bodyOverride?: string)");
    expect(source).not.toContain("setSnapshot(await api.setComposerDraft(roomId, value))");
    expect(source).not.toContain("setLocalComposerDrafts");
  });

  test("desktop api exposes a search index rebuild command", () => {
    const source = readFileSync(new URL("./backend/client.ts", import.meta.url), "utf8");
    const fakeSource = readFileSync(new URL("./backend/browserFakeApi.ts", import.meta.url), "utf8");

    expect(source).toContain("rebuildSearchIndex(): Promise<DesktopSnapshot>");
    expect(source).toContain('invoke<DesktopSnapshot>("rebuild_search_index"');
    expect(fakeSource).toContain("async rebuildSearchIndex(): Promise<DesktopSnapshot>");
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
        onOpenThread={() => undefined}
        onOpenFiles={() => undefined}
        onRefreshFilesView={() => undefined}
        onPaginateThreadsList={() => undefined}
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
  test("applies state deltas and keeps full snapshot refresh as fallback", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).toContain("applyAppStoreDelta");
    expect(source).toContain('event.payload.kind !== "StateDelta"');
    expect(source).toContain("generation: event.payload.generation");
    expect(source).toContain("if (!applied)");
    expect(source).toContain("STATE_EVENT_NAME");
    expect(source).toContain("listen<string>(STATE_EVENT_NAME");
    expect(source).toContain("STATE_EVENT_REFRESH_DEBOUNCE_MS");
    expect(source).toContain("stateRefreshTimerRef");
    expect(source).toContain("window.setTimeout");
    expect(source).toContain("void refresh()");
  });

  test("browser fixture messages use a natural-flow wrapper", () => {
    const source = readFileSync(new URL("./components/panes.tsx", import.meta.url), "utf8");
    const fallbackStart = source.indexOf("Browser fixture preview only");
    const fallbackEnd = source.indexOf("</div>", fallbackStart);
    const fallbackSource = source.slice(fallbackStart, fallbackEnd);
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

    expect(fallbackStart).toBeGreaterThanOrEqual(0);
    expect(fallbackSource).toContain('className="message-fixture-list"');
    expect(fallbackSource).toContain("snapshot.timeline.map");
    expect(styles).toContain(".message-fixture-list");
    expect(styles).toContain("@media (min-width: 761px) and (max-width: 1180px)");
    expect(styles).toContain(".app-grid.right-panel-open .thread-pane");
  });

  test("room header has one info/overflow action that toggles room info, not thread state", () => {
    const source = readFileSync(new URL("./components/panes.tsx", import.meta.url), "utf8");
    const paneStart = source.indexOf("export function TimelinePane");
    const paneEnd = source.indexOf("function TimelineComposer", paneStart);
    const paneSource = source.slice(paneStart, paneEnd);

    expect(paneSource).toContain('aria-label={t("room.roomInfo")}');
    expect(paneSource).toContain("onToggleRoomInfoStable");
    expect(paneSource).not.toContain("snapshot.state.ui.thread.kind");
    expect(paneSource).toContain("<MoreHorizontal");
    expect((paneSource.match(/<MoreHorizontal/g) ?? []).length).toBe(1);
  });

  test("room header wires People and media actions and conditionally shows threads", () => {
    const source = readFileSync(new URL("./components/panes.tsx", import.meta.url), "utf8");
    const paneStart = source.indexOf("export function TimelinePane");
    const paneEnd = source.indexOf("function TimelineComposer", paneStart);
    const paneSource = source.slice(paneStart, paneEnd);

    expect(paneSource).toContain('aria-label={t("panel.people")}');
    expect(paneSource).toContain("onOpenPeopleStable");
    expect(paneSource).toContain('aria-label={t("mediaGallery.open")}');
    expect(paneSource).toContain("showThreadsHeader");
    expect(paneSource).toContain("onOpenThreadsStable");
  });

  test("room creation links the new room into the active space", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const createStart = source.indexOf("async function submitCreateDialog");
    const createEnd = source.indexOf("async function setComposerReplyTarget", createStart);
    const createSource = source.slice(createStart, createEnd);

    expect(createSource).toContain("activeSpaceIdForCreatedRoom");
    expect(createSource).toContain("createRoomRequestFromDraft");
    expect(createSource).toContain("api.createRoom(createRoomRequest");
    expect(createSource).toContain("serverNameFromRoomId(createdRoomId)");
    expect(createSource).toContain("api.setSpaceChild(");
  });

  test("accepting an invite returns to the timeline view", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const acceptStart = source.indexOf("async function acceptInvite");
    const acceptEnd = source.indexOf("async function declineInvite", acceptStart);
    const acceptSource = source.slice(acceptStart, acceptEnd);

    expect(acceptSource).toContain("api.acceptInvite(roomId)");
    expect(acceptSource).toContain("api.selectRoom(roomId)");
    expect(acceptSource).toContain('setPrimaryView("timeline")');
  });

  test("joining a directory room shows the backend-selected timeline", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const joinStart = source.indexOf("async function joinDirectoryRoom");
    const joinEnd = source.indexOf("function openCreateDialog", joinStart);
    const joinSource = source.slice(joinStart, joinEnd);

    expect(joinSource).toContain("api.joinDirectoryRoom(alias, serverNameFromAlias(alias))");
    expect(joinSource).toContain('setPrimaryView("timeline")');
    expect(joinSource).toContain("setSnapshot(nextSnapshot)");
    expect(joinSource).not.toContain("previousRoomIds");
    expect(joinSource).not.toContain("api.selectRoom(");
  });

  test("room mark-as-read prefers the room latest event over stale markers", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const actionStart = source.indexOf('case "markRoomAsRead"');
    const actionEnd = source.indexOf('case "markRoomAsUnread"', actionStart);
    const actionSource = source.slice(actionStart, actionEnd);

    expect(actionStart).toBeGreaterThanOrEqual(0);
    expect(actionSource).toContain("fully_read_event_id");
    expect(actionSource).toContain("room?.latest_event?.event_id");
    expect(actionSource.indexOf("room?.latest_event?.event_id")).toBeLessThan(
      actionSource.indexOf("fully_read_event_id")
    );
    expect(actionSource).toContain("eventId.trim().length > 0");
    expect(actionSource).toContain("api.markRoomAsRead(target.roomId, eventId)");
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
    expect(selectSearchResultSource).toContain('setPrimaryView("timeline")');
    expect(selectSearchResultSource).not.toContain("selectRoom(");
    expect(selectSearchResultSource).not.toContain('setSearchQuery("")');
    expect(selectSearchResultSource).not.toContain("document.querySelector");
    expect(selectSearchResultSource).not.toContain("scrollIntoView");
    expect(selectSearchResultSource).not.toContain("cssEscape");
  });

  test("message context menu normal reply targets the room composer", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const actionStart = source.indexOf("function runContextMenuAction");
    const actionEnd = source.indexOf("async function runSearch", actionStart);
    const actionSource = source.slice(actionStart, actionEnd);
    const replyIndex = actionSource.indexOf('case "replyToMessage"');
    const threadIndex = actionSource.indexOf('case "openThread"');

    expect(replyIndex).toBeGreaterThanOrEqual(0);
    expect(threadIndex).toBeGreaterThan(replyIndex);
    expect(actionSource).toContain(
      "void setComposerReplyTarget(target.message.room_id, target.message.event_id);"
    );
  });

  test("search close clears Rust search state instead of promoting inline results", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const closeSearchStart = source.indexOf("async function closeSearchPanel");
    const closeSearchEnd = source.indexOf("function openActivityRow", closeSearchStart);
    const closeSearchSource = source.slice(closeSearchStart, closeSearchEnd);

    expect(closeSearchStart).toBeGreaterThanOrEqual(0);
    expect(closeSearchSource).toContain("api.closeSearch()");
    expect(closeSearchSource).toContain('setSearchQuery("")');
    expect(closeSearchSource).toContain('setRightPanelMode("closed")');
    expect(source).not.toContain('showSearchResults={effectiveRightPanelMode !== "search"}');
    expect(source).toContain("showSearchResults={false}");
  });

  test("activity row selection navigates to the event without opening focused context", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const openActivityRowStart = source.indexOf("function openActivityRow");
    const openActivityRowEnd = source.indexOf("function selectSearchResult");
    const openActivityRowSource = source.slice(openActivityRowStart, openActivityRowEnd);
    const activityRenderStart = source.indexOf("<ActivityPane");
    const activityRenderEnd = source.indexOf("</ActivityPane>", activityRenderStart);
    const activityRenderSource = source.slice(activityRenderStart, activityRenderEnd);

    expect(openActivityRowStart).toBeGreaterThanOrEqual(0);
    expect(openActivityRowSource).toContain(".openActivityEvent(roomId, eventId)");
    expect(openActivityRowSource).not.toContain(".selectSearchResult(roomId, eventId)");
    expect(openActivityRowSource).toContain('setRightPanelMode("closed")');
    expect(openActivityRowSource).not.toContain('setRightPanelMode("focusedContext")');
    expect(openActivityRowSource).not.toContain('setRightPanelMode("search")');
    expect(activityRenderSource).toContain("openActivityRow(row.room_id, row.event_id)");
    expect(activityRenderSource).toContain('row.kind === "roomUnread"');
    expect(activityRenderSource).toContain("openActivityRoom(row.room_id)");
    expect(activityRenderSource).not.toContain("selectSearchResult(row.room_id, row.event_id)");
  });

  test("home rail button resets Home to Activity Recent instead of restoring the saved Home pane", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const sidebarRenderStart = source.indexOf("<Sidebar");
    const sidebarRenderEnd = source.indexOf("</Sidebar>", sidebarRenderStart);
    const sidebarRenderSource = source.slice(sidebarRenderStart, sidebarRenderEnd);
    const onOpenHomeStart = sidebarRenderSource.indexOf("onOpenHome={() =>");
    const onOpenHomeEnd = sidebarRenderSource.indexOf("onOpenInvites", onOpenHomeStart);
    const onOpenHomeSource = sidebarRenderSource.slice(onOpenHomeStart, onOpenHomeEnd);

    expect(onOpenHomeStart).toBeGreaterThanOrEqual(0);
    expect(onOpenHomeSource).toContain("openHomeActivityView()");
    expect(onOpenHomeSource).not.toContain("selectSpace(null)");
  });

  test("initial Home selection does not override an already selected room", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const effectStart = source.indexOf("initialHomeSelectionApplied.current ||");
    const effectEnd = source.indexOf("initialHomeSelectionApplied.current = true", effectStart);
    const effectGuardSource = source.slice(effectStart, effectEnd);

    expect(effectStart).toBeGreaterThanOrEqual(0);
    expect(effectGuardSource).toContain(
      "snapshot.state.ui.navigation.active_room_id !== null"
    );
  });

  test("recovery submit trims pasted outer whitespace without altering the secret variable", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const submitRecoveryStart = source.indexOf("async function submitRecovery");
    const submitRecoveryEnd = source.indexOf("async function restartSync", submitRecoveryStart);
    const submitRecoverySource = source.slice(submitRecoveryStart, submitRecoveryEnd);

    expect(submitRecoveryStart).toBeGreaterThanOrEqual(0);
    expect(submitRecoverySource).toContain("recoverySecretRef.current?.value.trim() ??");
    expect(submitRecoverySource).toContain("api.submitRecovery(secret)");
  });

  test("timeline date jump navigates the main timeline without opening the right panel", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const transportStart = source.indexOf("const appTimelineTransport");
    const transportEnd = source.indexOf("const attentionSummary", transportStart);
    const transportSource = source.slice(transportStart, transportEnd);

    expect(transportStart).toBeGreaterThanOrEqual(0);
    expect(transportSource).toContain("api.openTimelineAtTimestamp(roomId, timestampMs)");
    expect(transportSource).toContain("setSnapshot(nextSnapshot)");
    expect(transportSource).toContain('setPrimaryView("timeline")');
    // #161: jump-to-date must NOT open the right panel — it explicitly closes it
    // so an already-open focused-context/search panel does not linger over the
    // anchored main timeline. The focused timeline renders in the MAIN pane,
    // marked by navigation.main_timeline_anchor.
    expect(transportSource).not.toContain('setRightPanelMode("focusedContext")');
    expect(transportSource).toContain('setRightPanelMode("closed")');
    expect(source).toContain("timelineTransport={appTimelineTransport}");

    // The main pane switches to the focused timeline key when anchored.
    const panesSource = readFileSync(
      new URL("./components/panes.tsx", import.meta.url),
      "utf8"
    );
    expect(panesSource).toContain("main_timeline_anchor");
    expect(panesSource).toContain("focusedTimelineKey");

    // #161: the anchored main pane exposes a return-to-live control that closes
    // the focused context (which clears the anchor in Rust → main pane re-renders
    // the live timeline).
    expect(source).toContain("onReturnToLive");
    expect(source).toContain("api.closeFocusedContext()");
    expect(panesSource).toContain("isAnchored={Boolean(mainTimelineAnchorEventId)}");
    expect(panesSource).toContain("onReturnToLive={onReturnToLive}");
    const timelineViewSource = readFileSync(
      new URL("./components/TimelineView.tsx", import.meta.url),
      "utf8"
    );
    expect(timelineViewSource).toContain("isAnchored && onReturnToLive");
  });

  test("anchored timeline header latest button returns to live instead of scrolling focused history", () => {
    const panesSource = readFileSync(
      new URL("./components/panes.tsx", import.meta.url),
      "utf8"
    );
    const headerNavigationStart = panesSource.indexOf('className="timeline-header-navigation"');
    const headerNavigationEnd = panesSource.indexOf("</nav>", headerNavigationStart);
    const headerNavigationSource = panesSource.slice(headerNavigationStart, headerNavigationEnd);

    expect(headerNavigationStart).toBeGreaterThanOrEqual(0);
    expect(headerNavigationSource).toContain("mainTimelineAnchorEventId");
    expect(headerNavigationSource).toContain("onReturnToLive");
    expect(headerNavigationSource).toContain("jumpToLatestRef.current?.()");
    expect(headerNavigationSource).not.toContain("closest<HTMLElement>(");
    expect(headerNavigationSource).not.toContain("scrollTop");
    expect(headerNavigationSource.indexOf("onReturnToLive")).toBeLessThan(
      headerNavigationSource.indexOf("jumpToLatestRef.current?.()")
    );
    expect(panesSource).toContain("onRegisterJumpToLatest={registerJumpToLatest}");
    expect(panesSource).toContain("jumpToLatestRef.current = handler");
  });

  test("activity room-unread placeholders open rooms without forcing live edge", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const openActivityRoomStart = source.indexOf("async function openActivityRoom");
    const openActivityRoomEnd = source.indexOf("function selectSearchResult", openActivityRoomStart);
    const openActivityRoomSource = source.slice(openActivityRoomStart, openActivityRoomEnd);
    const openRowStart = source.indexOf("onOpenRow={(row)");
    const openRowEnd = source.indexOf("onSetTab={(tab)", openRowStart);
    const openRowSource = source.slice(openRowStart, openRowEnd);

    expect(openActivityRoomStart).toBeGreaterThanOrEqual(0);
    expect(openRowSource).toContain('row.kind === "roomUnread"');
    expect(openRowSource).toContain("openActivityRoom(row.room_id)");
    expect(openActivityRoomSource).toContain("api.closeFocusedContext()");
    expect(openActivityRoomSource).toContain("api.selectRoom(roomId)");
    expect(openActivityRoomSource).not.toContain("setTimelineLiveEdgeReset");
    expect(openActivityRoomSource).not.toContain("timelineLiveEdgeReset");

    const coreEventsSource = readFileSync(
      new URL("./domain/coreEvents.ts", import.meta.url),
      "utf8"
    );
    const unreadTypeStart = coreEventsSource.indexOf("export interface ActivityRoomUnreadRow");
    const unreadTypeEnd = coreEventsSource.indexOf("export type ActivityRow", unreadTypeStart);
    const unreadTypeSource = coreEventsSource.slice(unreadTypeStart, unreadTypeEnd);
    expect(unreadTypeSource).toContain("event_id: null");
  });

  test("member-panel avatar thumbnail requests respect the global avatar download gate", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const renderStart = source.indexOf("<ContextualRightPanel");
    const renderEnd = source.indexOf("</ContextualRightPanel>", renderStart);
    const panelPropsSource = source.slice(renderStart, renderEnd);

    expect(panelPropsSource).toMatch(
      /onRequestMemberAvatarThumbnail=\{\s*AVATAR_THUMBNAIL_DOWNLOADS_ENABLED\s*\?\s*tauriTimelineTransport\?\.downloadAvatarThumbnail\s*:\s*undefined\s*\}/
    );
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
    expect(panelPropsSource).toContain("onTimelineDiagnosticLogEntry={appendDiagnosticLog}");
  });

  test("feeds Rust-owned native attention into window title and notification adapters", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    const summaryStart = source.indexOf("const attentionSummary");
    const summaryEnd = source.indexOf("function handleShortcutAction", summaryStart);
    const summarySource = source.slice(summaryStart, summaryEnd);
    expect(summarySource).toContain("desktopAttentionSummary(snapshot.state.domain.native_attention)");
    expect(summarySource).not.toContain("snapshot.state.domain.rooms");
    expect(summarySource).not.toContain("navigation.active_room_id");

    const notificationStart = source.indexOf("const candidate = desktopAttentionNotificationCandidate");
    const notificationEnd = source.indexOf("void sendDesktopAttentionNotification", notificationStart);
    const notificationSource = source.slice(notificationStart, notificationEnd);
    expect(notificationSource).toContain("snapshot.state.domain.native_attention");
    expect(notificationSource).not.toContain("previousAttentionInput");
    expect(notificationSource).not.toContain("snapshot.state.domain.rooms");

    const notificationEffectEnd = source.indexOf("]);", notificationStart);
    const notificationEffectSource = source.slice(notificationStart, notificationEffectEnd);
    expect(notificationEffectSource).toContain("void dispatchDesktopAttentionTransientEffects");
    expect(notificationEffectSource).toContain("snapshot.state.domain.native_attention.summary.capabilities");
    expect(notificationEffectSource).not.toContain("snapshot.state.domain.rooms");

    const clearStart = source.indexOf("safeAttentionSummary.badgeCount !== 0");
    const clearEnd = source.indexOf("const message = qaSendSmokeMessage", clearStart);
    const clearSource = source.slice(clearStart, clearEnd);
    expect(clearSource).toContain("safeAttentionSummary.badgeCount !== 0");
    expect(clearSource).toContain("void clearDesktopAttentionNotifications");
    expect(clearSource).toContain("tauriNotificationTransport");

    expect(source).toContain("desktopAttentionWindowTitle");
    expect(source).toContain("sendDesktopAttentionNotification");
    expect(source).toContain("dispatchDesktopAttentionTransientEffects");
    expect(source).toContain("applyDesktopAttentionToWindow");
    expect(source).toContain("qaWindowTitle(");
    expect(source).toContain("effectiveRightPanelModeForSnapshot");
    expect(source).toContain("rightPanelMode");
    expect(source).toContain("qaSendStatus");
    expect(source).toContain("getCurrentWindow()");
    expect(source).toContain("snapshot?.state.domain.native_attention.summary.capabilities");
    expect(source).toContain("document.title = title");
    expect(source).toContain("desktopAttentionWindowTitle");
  });

  test("room selection appends private-data-free transition diagnostics around the API call", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const fnStart = source.indexOf("async function selectRoom(roomId: string)");
    const fnEnd = source.indexOf("async function openHomeActivityView", fnStart);
    expect(fnStart).toBeGreaterThanOrEqual(0);
    expect(fnEnd).toBeGreaterThan(fnStart);
    const selectRoomSource = source.slice(fnStart, fnEnd);

    const startOffset = selectRoomSource.indexOf("stage=select_start");
    const apiOffset = selectRoomSource.indexOf("api.selectRoom(roomId)");
    const doneOffset = selectRoomSource.indexOf("stage=select_done");

    expect(selectRoomSource).toContain('source: "room.transition"');
    expect(selectRoomSource).toContain("target_known=");
    expect(selectRoomSource).toContain("same_active=");
    expect(selectRoomSource).toContain("timeline_matches=");
    expect(startOffset).toBeGreaterThanOrEqual(0);
    expect(apiOffset).toBeGreaterThan(startOffset);
    expect(doneOffset).toBeGreaterThan(apiOffset);
  });
});

describe("TopBar sync state rendering", () => {
  test("uses native macOS titlebar overlay instead of a separate titlebar row", () => {
    const config = JSON.parse(
      readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")
    );
    const mainWindow = config.app.windows[0];

    expect(mainWindow.decorations ?? true).toBe(true);
    expect(mainWindow.titleBarStyle).toBe("Overlay");
    expect(mainWindow.hiddenTitle).toBe(true);
    expect(mainWindow.trafficLightPosition).toEqual({ x: 18, y: 16 });
  });

  test("does not draw duplicate macOS traffic light controls", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { TopBar } = await import("./App");
    const markup = renderToStaticMarkup(
      <TopBar
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    expect(markup).not.toContain('class="traffic"');
    expect(markup).not.toContain("dot red");
    expect(markup).not.toContain("dot yellow");
    expect(markup).not.toContain("dot green");
  });

  test("marks the overlay titlebar top edge as a Tauri window drag region", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { TopBar } = await import("./App");
    const markup = renderToStaticMarkup(
      <TopBar
        activeSpaceName="Matrix"
        isBusy={false}
        searchInputRef={{ current: null }}
        searchQuery=""
        searchScope="allRooms"
        sync="running"
        onOpenKeyboardSettings={() => undefined}
        onRestartSync={() => undefined}
        onSearchQueryChange={() => undefined}
        onSearchScopeChange={() => undefined}
      />
    );

    expect(markup).toContain('class="titlebar"');
    expect(markup).toContain('data-tauri-drag-region=""');
    expect(markup).not.toContain("titlebar-drag-strip");
  });

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
          } as DesktopSnapshot["state"]["domain"]["sync"]
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
          } as DesktopSnapshot["state"]["domain"]["sync"]
        }
      />
    );
    expect(failedMarkup).toContain("Failed");
    expect(failedMarkup).toContain("transport error");
    expect(failedMarkup).toContain('aria-label="Restart sync"');
  });
});

describe("Timeline item row rendering", () => {
  test("MessageSourceDialog renders Element-style original event source details", () => {
    const markup = renderToStaticMarkup(
      <MessageSourceDialog
        source={{
          event_id: "$event:example.invalid",
          sender: "@alice:example.invalid",
          timestamp_ms: 1_781_841_275_583,
          body: "We are planning to release the first version in July.",
          in_reply_to_event_id: null,
          thread_root: null,
          is_redacted: false,
          is_edited: false,
          has_media: false,
          original_json: {
            unsigned: {
              age: 648,
              transaction_id: "m1781841277122.98",
              membership: "join"
            },
            content: {
              body: "We are planning to release the first version in July.",
              "m.mentions": {},
              msgtype: "m.text"
            },
            origin_server_ts: 1_781_841_275_583,
            sender: "@alice:example.invalid",
            type: "m.room.message",
            event_id: "$event:example.invalid",
            room_id: "!room:example.invalid"
          }
        }}
        onClose={() => undefined}
      />
    );

    expect(markup).toContain("Event ID:");
    expect(markup).toContain("$event:example.invalid");
    expect(markup).toContain("Original event source");
    expect(markup).toContain("&quot;unsigned&quot;");
    expect(markup).toContain("&quot;m.room.message&quot;");
    expect(markup).toContain("&quot;m.mentions&quot;");
  });

  test("renders sender surfaces from Rust-owned timeline display labels", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$reply:example.invalid" } },
            sender: "@me:example.invalid",
            sender_label: "Me Alias",
            body: "Reply body",
            timestamp_ms: 1_820_000_000_000,
            in_reply_to_event_id: "$root:example.invalid",
            reply_quote: {
              event_id: "$root:example.invalid",
              sender: "@alice:example.invalid",
              sender_label: "Alice Alias",
              body_preview: "Original quoted body",
              state: "ready"
            },
            thread_root: null,
            thread_summary: {
              reply_count: 2,
              latest_sender: "@carol:example.invalid",
              latest_sender_label: "Carol Alias",
              latest_body_preview: "latest reply",
              latest_timestamp_ms: 1_820_000_000_001
            },
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain("Me Alias");
    expect(markup).toContain("Alice Alias");
    expect(markup).toContain("Carol Alias");
    expect(markup).not.toContain("@me:example.invalid");
    expect(markup).not.toContain("@alice:example.invalid");
    expect(markup).not.toContain("@carol:example.invalid");
  });

  test("TimelineItemRow renders sender avatar from Rust-owned timeline profile data", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$avatar:example.invalid" } },
            sender: "@kamohara:matrix.org",
            sender_label: "kamohara",
            sender_avatar: {
              mxc_uri: "mxc://matrix.org/avatar",
              thumbnail: {
                kind: "ready",
                source_url: "/media/avatar.png",
                width: 96,
                height: 96,
                mime_type: "image/png"
              }
            },
            body: "23リットルにしました。",
            timestamp_ms: 1_820_000_000_000,
            in_reply_to_event_id: null,
            thread_root: null,
            thread_summary: null,
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('<img src="/media/avatar.png"');
    expect(markup).not.toContain(">KA<");
  });

  test("TimelineItemRow renders SDK date dividers without a fallback question avatar", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Synthetic: { synthetic_id: "date-divider-1781049600000" } },
            sender: null,
            body: null,
            timestamp_ms: 1_781_049_600_000,
            in_reply_to_event_id: null,
            thread_root: null,
            thread_summary: null,
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('role="separator"');
    expect(markup).toContain("Jun");
    expect(markup).not.toContain('class="avatar"');
    expect(markup).not.toContain("&gt;?&lt;");
  });

  test("renders reply quote block from Rust-owned timeline item data", () => {
    const markup = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Event: { event_id: "$reply:example.invalid" } },
            sender: "@me:example.invalid",
            body: "Reply body",
            timestamp_ms: 1_820_000_000_000,
            in_reply_to_event_id: "$root:example.invalid",
            reply_quote: {
              event_id: "$root:example.invalid",
              sender: "@alice:example.invalid",
              body_preview: "Original quoted body",
              state: "ready"
            },
            thread_root: null,
            thread_summary: null,
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(markup).toContain('class="reply-quote"');
    expect(markup).toContain("@alice:example.invalid");
    expect(markup).toContain("Original quoted body");
    expect(markup).not.toContain("$root:example.invalid");
  });

  test("renders pin or unpin row action from Rust-owned pinned state", () => {
    const item = {
      id: { Event: { event_id: "$pin-target:example.invalid" } },
      sender: "@me:example.invalid",
      body: "Pinnable message",
      timestamp_ms: 1_820_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    } as TimelineItem;

    const unpinned = renderToStaticMarkup(
      <TimelineItemRow
        item={item}
        roomId="!room:example.invalid"
        isPinned={false}
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
        onPin={() => undefined}
        onUnpin={() => undefined}
      />
    );
    expect(unpinned).toContain('aria-label="Pin message"');
    expect(unpinned).not.toContain('aria-label="Unpin message"');

    const pinned = renderToStaticMarkup(
      <TimelineItemRow
        item={item}
        roomId="!room:example.invalid"
        isPinned={true}
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
        onPin={() => undefined}
        onUnpin={() => undefined}
      />
    );
    expect(pinned).toContain('aria-label="Unpin message"');
    expect(pinned).not.toContain('aria-label="Pin message"');
  });

  test("renders send queue status from Rust-owned send_state only", () => {
    const transactionWithoutState = renderToStaticMarkup(
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
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: []
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(transactionWithoutState).not.toContain("data-send-state=");
    expect(transactionWithoutState).not.toContain("Not sent");
    expect(transactionWithoutState).not.toContain("Sending");

    const notSent = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Transaction: { transaction_id: "desktop-2" } },
            sender: "@me:example.invalid",
            body: "failed message",
            timestamp_ms: 1_820_000_000_100,
            in_reply_to_event_id: null,
            thread_root: null,
            thread_summary: null,
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: [],
            send_state: { kind: "notSent", reason: "recoverable" }
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(notSent).toContain('data-send-state="notSent"');
    expect(notSent).toContain("Not sent");
    expect(notSent).toContain("Resend");
    expect(notSent).toContain("Delete");

    const sending = renderToStaticMarkup(
      <TimelineItemRow
        item={
          {
            id: { Transaction: { transaction_id: "desktop-3" } },
            sender: "@me:example.invalid",
            body: "sending message",
            timestamp_ms: 1_820_000_000_200,
            in_reply_to_event_id: null,
            thread_root: null,
            thread_summary: null,
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            reactions: [],
            send_state: { kind: "sending" }
          } as TimelineItem
        }
        roomId="!room:example.invalid"
        onReply={() => undefined}
        onSendReaction={() => undefined}
        onRedactReaction={() => undefined}
        onEdit={() => undefined}
        onRedact={() => undefined}
      />
    );

    expect(sending).toContain('data-send-state="sending"');
    expect(sending).toContain("Sending");
    expect(sending).toContain("Cancel send");
  });

  test("room People entries load room settings before switching to people mode", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    const timelinePaneStart = source.indexOf("<TimelinePane");
    const timelinePaneEnd = source.indexOf("\n          />", timelinePaneStart);
    expect(timelinePaneStart).toBeGreaterThanOrEqual(0);
    expect(timelinePaneEnd).toBeGreaterThan(timelinePaneStart);
    const timelinePaneSource = source.slice(timelinePaneStart, timelinePaneEnd);

    expect(timelinePaneSource).toContain("api.loadRoomSettings(");
    expect(timelinePaneSource).toContain('setRightPanelModeClosingFocusedContext("people")');
    expect(timelinePaneSource.indexOf("api.loadRoomSettings(")).toBeLessThan(
      timelinePaneSource.indexOf('setRightPanelModeClosingFocusedContext("people")')
    );

    const contextualRightPanelStart = source.indexOf("<ContextualRightPanel");
    const contextualRightPanelEnd = source.indexOf("\n        />", contextualRightPanelStart);
    expect(contextualRightPanelStart).toBeGreaterThanOrEqual(0);
    expect(contextualRightPanelEnd).toBeGreaterThan(contextualRightPanelStart);
    const contextualRightPanelSource = source.slice(
      contextualRightPanelStart,
      contextualRightPanelEnd
    );

    expect(contextualRightPanelSource).toContain("api.loadRoomSettings(");
    expect(contextualRightPanelSource).toContain(
      'setRightPanelModeClosingFocusedContext("people")'
    );
    expect(contextualRightPanelSource.indexOf("api.loadRoomSettings(")).toBeLessThan(
      contextualRightPanelSource.indexOf('setRightPanelModeClosingFocusedContext("people")')
    );
  });
});
