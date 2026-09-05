import { commandSnapshot } from "./test/commandReceiptFixture";
import { renderToStaticMarkup } from "react-dom/server";
import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, test, vi } from "vitest";

import {
  awaitingVerificationSnapshotFixture,
  createDesktopApiFixture
} from "./test/desktopApiFixture";
import {
  createComposerSubmissionController,
  createComposerSubmissionControllerRegistry,
  mainSubmissionTarget
} from "./domain/composerSubmission";
import { documentFromText } from "./domain/composerDocument";
import { COMPOSER_DRAFT_REVISION_ZERO } from "./domain/composerDraftRevision";
import { MessageSourceDialog, TimelineItemRow } from "./components/TimelineView";
import { focusedTimelineKey, type TimelineItem } from "./domain/coreEvents";
import { timelineStoreKeyId } from "./domain/timelineStore";
import type { DesktopSnapshot } from "./domain/types";
import type { RightPanelMode } from "./domain/rightPanel";
import { formatScheduledSendTime } from "./app/uiShared";
import { t } from "./i18n/messages";

function productionTauriImportFiles(
  directory = new URL("./", import.meta.url),
  relativeDirectory = ""
): string[] {
  const matches: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const relativePath = `${relativeDirectory}${entry.name}`;
    if (entry.isDirectory()) {
      if (entry.name !== "test") {
        matches.push(
          ...productionTauriImportFiles(
            new URL(`${entry.name}/`, directory),
            `${relativePath}/`
          )
        );
      }
      continue;
    }
    if (
      /\.tsx?$/.test(entry.name) &&
      !entry.name.includes(".test.") &&
      /from\s+["']@tauri-apps\//.test(readFileSync(new URL(entry.name, directory), "utf8"))
    ) {
      matches.push(relativePath);
    }
  }
  return matches.sort();
}

describe("ContextualRightPanel", () => {
  test("accepted terminal snapshot settles an unknown send before the next draft", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { reconcileComposerSubmissionSnapshot } = await import("./App");
    const ids = ["submission-1", "submission-2"];
    const registry = createComposerSubmissionControllerRegistry(
      () => createComposerSubmissionController(() => ids.shift()!)
    );
    const controller = registry.forTarget(mainSubmissionTarget("room-a"));
    const first = controller.begin()!;
    controller.capture(first, { body: "original" });
    controller.markUnknown(first, "timeout");
    const snapshot = await createDesktopApiFixture().getSnapshot();
    snapshot.state.ui.timeline.submission_registry = {
      accepted_submission_ids: [first],
      settled_submission_ids: [first]
    };
    reconcileComposerSubmissionSnapshot(registry, snapshot.state.ui.timeline);
    const nextController = registry.forTarget(mainSubmissionTarget("room-a"));
    const next = nextController.begin()!;
    nextController.capture(next, { body: "current draft" });
    expect(next).toBe("submission-2");
    expect(nextController.payload<{ body: string }>(next)?.body).toBe("current draft");
  });
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
    onCancelIdentityReset: () => undefined,
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
        document={documentFromText("hello")}
        roomName="Room Alpha"
        onCancelReply={() => undefined}
        onDocumentChange={() => undefined}
        onSend={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Sending"');
    expect(markup).toContain("disabled");
  });

  test("workspace rail exposes create space control", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { WorkspaceRail } = await import("./App");
    const api = createDesktopApiFixture();
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

    // #330: the label names unread messages and invites when either is present,
    // so match the name's start rather than the whole attribute.
    expect(markup).toContain('aria-label="Home');
    expect(markup).not.toContain('aria-label="Activity"');
    expect(markup).toContain('role="separator"');
    expect(markup).toContain('aria-label="Create space"');
  });

  test("workspace rail renders Rust-projected space attention counts", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { WorkspaceRail } = await import("./App");
    const api = createDesktopApiFixture();
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
    expect(markup).toContain("element-space");
    expect(markup).toContain(">O</span>");
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
        document={documentFromText("reply")}
        roomName="QA Room"
        onCancelReply={() => undefined}
        onDocumentChange={() => undefined}
        onSend={() => undefined}
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
        document={documentFromText("")}
        roomName="Room Alpha"
        onCancelReply={() => undefined}
        onDocumentChange={() => undefined}
        onSend={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Attach file"');
    expect(markup).toContain('type="file"');
    expect(markup).toContain('aria-label="Attach file input"');
    expect(markup).toContain('aria-label="Send"');
  });

  test("reset local data uses an in-app confirmation before deleting local state", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const resetHandler = source
      .split("async function resetLocalData()")
      .at(1)
      ?.split("async function acceptVerification")
      .at(0);

    expect(resetHandler).toBeDefined();
    expect(resetHandler).not.toContain("window.confirm");
    expect(source).toContain("resetLocalDataConfirmOpen");
    expect(source.indexOf("resetLocalDataConfirmOpen")).toBeLessThan(
      source.indexOf("async function resetLocalData()")
    );
    expect(resetHandler).toContain("api.resetLocalData()");
  });

  test("all user-facing sign-out entry points use native confirmation", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const appRuntimeSource = readFileSync(
      new URL("./backend/appRuntime.ts", import.meta.url),
      "utf8"
    );
    const confirmationHandler = source
      .split("async function requestLogout()")
      .at(1)
      ?.split("async function logout()")
      .at(0);

    expect(confirmationHandler).toBeDefined();
    expect(confirmationHandler).toContain("windowDialogPort.confirm");
    expect(confirmationHandler).toContain('t("settings.signOutConfirm")');
    expect(confirmationHandler).toContain("await logout()");
    expect(source).not.toContain("void requestLogout()");
    expect(source.match(/runInBackground\(requestLogout\(\)\)/g)?.length).toBe(4);

    const tauriImportStatements =
      source.match(/^import\s+[^;]+from\s+["']@tauri-apps\/[^"']+["'];$/gm) ?? [];
    expect(tauriImportStatements).toEqual([]);
    expect(productionTauriImportFiles()).toEqual([
      "backend/client.ts",
      "backend/tauri/desktopAttentionPort.ts",
      "backend/tauri/desktopEventPort.ts",
      "backend/tauri/linkMediaPort.ts",
      "backend/tauri/windowDialogPort.ts",
      "backend/tauriTimelineTransport.ts"
    ]);
    expect(appRuntimeSource).not.toMatch(/from\s+["']@tauri-apps\//);
  });

  test("preserves window and key-file dialog guards through the neutral port", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const exportStart = source.indexOf("async function chooseRoomKeyExportDestination");
    const backupStart = source.indexOf("async function chooseSecureBackupDestination");
    const importStart = source.indexOf("async function chooseRoomKeyImportSource");
    const importEnd = source.indexOf("async function bootstrapSecureBackup", importStart);
    const exportSource = source.slice(exportStart, backupStart);
    const backupSource = source.slice(backupStart, importStart);
    const importSource = source.slice(importStart, importEnd);

    expect(exportSource).toContain("if (!isTauriRuntime())");
    expect(exportSource).toContain("windowDialogPort.saveFile");
    expect(exportSource).toContain('defaultPath: "koushi-room-keys.txt"');
    expect(exportSource).toContain("return selected || null");
    expect(backupSource).toContain("if (!isTauriRuntime())");
    expect(backupSource).toContain("windowDialogPort.saveFile");
    expect(backupSource).toContain('defaultPath: "koushi-secure-backup-recovery-key.txt"');
    expect(importSource).toContain("if (!isTauriRuntime())");
    expect(importSource).toContain("windowDialogPort.openFile");
    expect(importSource).toContain("multiple: false");
    expect(importSource).toContain('fileAccessMode: "scoped"');
    expect(importSource).toContain('typeof selected === "string" ? selected : null');
    expect(source).toContain("runInBackground(windowDialogPort.toggleFullscreen())");
    expect(source).toContain("windowDialogPort.startDragging().catch(() => undefined)");
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
              sender_preview: [{ user_id: "@alice:example.invalid", display_label: "Alice" }]
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
            latest_event_id: "$latest-thread-reply:example.invalid",
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
    expect(markup).toContain("Unknown user: Latest thread reply");
    expect(markup).not.toContain("@bob:example.invalid: Latest thread reply");
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
              '<strong>Bold body</strong><blockquote>Quoted body</blockquote><ul><li>List item</li></ul><span data-mx-maths="E=mc^2">E=mc^2</span><a href="https://example.invalid/path">safe link</a><pre><code class="language-rust">fn main() {}</code></pre>',
            plain_text: "Bold bodyQuoted bodyList itemE=mc^2safe linkfn main() {}",
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
    expect(markup).toContain('class="message-math');
    expect(markup).toContain('class="katex');
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
        searchHighlights={[{ start_utf16: 10, end_utf16: 17 }]}
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
                sender_preview: [{ user_id: "@alice:example.invalid", display_label: "Alice" }]
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
    const api = createDesktopApiFixture();
    const snapshot = await api.getSnapshot();
    const firstSearchResult = {
      room_id: "!room-alpha:example.invalid",
      event_id: "$search:example.invalid",
      context_label: "synthetic-room",
      sender: "@alice:example.invalid",
      timestamp_ms: 1_800_000_000_000,
      score_millis: 1000,
      snippet: "Alpha keyword update",
      match_field: "messageBody" as const,
      highlights: [{ start_utf16: 0, end_utf16: 5 }],
      match_kind: "exact" as const
    };
    snapshot.state.domain.search = {
      kind: "results",
      request_id: 1,
      query: "Alpha",
      scope: "allRooms",
      results: [firstSearchResult]
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
        onThreadComposerDocumentChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain("Search");
    expect(markup).toContain("Alpha");
    expect(markup).toContain("keyword update");
    expect(markup).toContain("search-results");
    expect(markup).toContain("search-panel");
    expect(firstSearchResult).not.toBeNull();
    expect(markup).toContain(formatScheduledSendTime(firstSearchResult!.timestamp_ms));
  });

  test("search right panel keeps long result lists in a bounded scroller", () => {
    const styles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

    expect(styles).toContain(".thread-pane.search-panel > .search-results");
    expect(styles).toContain(".thread-pane.search-panel > .search-results .result-list");
    expect(styles).toContain("overflow-y: auto");
  });

  test("renders indexing-pending copy for empty search results while crawler is active", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createDesktopApiFixture();
    const snapshot = await commandSnapshot(api, api.submitSearch("NoMatch", "allRooms"));

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
        searchIndexingPending={true}
        searchQuery="NoMatch"
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
        onThreadComposerDocumentChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain("Indexing message history");
    expect(markup).not.toContain("No exact matches");
  });

  test("renders focused search context from Rust-owned snapshot state", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createDesktopApiFixture();
    const snapshot = await commandSnapshot(api, api.submitSearch("Alpha", "allRooms"));
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
        onThreadComposerDocumentChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain(t("panel.focusedContext"));
    expect(markup).toContain('data-testid="timeline-view"');
  });

  test("renders focusedContext mode as a focused TimelineView without search results", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createDesktopApiFixture();
    const snapshot = await commandSnapshot(api, api.submitSearch("Alpha", "allRooms"));
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
        onThreadComposerDocumentChange={() => undefined}
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
    const api = createDesktopApiFixture();
    const snapshot = await api.getSnapshot();
    snapshot.state.ui.thread = {
      kind: "open",
      room_id: snapshot.state.domain.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      intent: "existingThread",
      is_subscribed: true,
      composer: { accepted_submission_ids: [], pending_transaction_id: null, draft_revision: COMPOSER_DRAFT_REVISION_ZERO, last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO, draft: "", document: documentFromText(""), mode: "Plain" },
      staged_uploads: []
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
        onThreadComposerDocumentChange={() => undefined}
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
    const api = createDesktopApiFixture();
    const snapshot = await api.getSnapshot();
    snapshot.state.ui.thread = {
      kind: "open",
      room_id: snapshot.state.domain.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      intent: "existingThread",
      is_subscribed: true,
      composer: {
        accepted_submission_ids: [],
        pending_transaction_id: null,
        draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
        last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
        draft: "Rust-owned draft",
        document: documentFromText("Rust-owned draft"),
        mode: "Plain"
      },
      staged_uploads: []
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
        onThreadComposerDocumentChange={() => undefined}
        onThreadReplySend={() => undefined}
      />
    );

    expect(markup).toContain('aria-label="Thread composer"');
    expect(markup).toContain('contentEditable="true"');
    expect(markup).toContain('aria-label="Send"');
    expect(markup).not.toContain('aria-label="Sending"');
  });

  test("thread composer disables send while the Rust-owned composer is pending", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createDesktopApiFixture();
    const snapshot = await api.getSnapshot();
    snapshot.state.ui.thread = {
      kind: "open",
      room_id: snapshot.state.domain.rooms[0]?.room_id,
      root_event_id: "$root:example.invalid",
      intent: "existingThread",
      is_subscribed: true,
      composer: {
        accepted_submission_ids: [],
        pending_transaction_id: "txn-thread-1",
        draft_revision: COMPOSER_DRAFT_REVISION_ZERO,
        last_accepted_clear_revision: COMPOSER_DRAFT_REVISION_ZERO,
        draft: "Draft blocked by pending send",
        document: documentFromText("Draft blocked by pending send"),
        mode: "Plain"
      },
      staged_uploads: []
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
        onThreadComposerDocumentChange={() => undefined}
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
    const source = readFileSync(
      new URL("./backend/tauriTimelineTransport.ts", import.meta.url),
      "utf8"
    );
    const transportStart = source.indexOf("const tauriTimelineTransport");
    const transportEnd = source.indexOf("export {", transportStart);
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

  test("App consumes one ordered v1 state-update lane", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    const listenerIndex = source.indexOf("desktopEventPort.listenStateUpdates");
    const initialSnapshotIndex = source.indexOf(".then(() => api.getSnapshot())", listenerIndex);
    expect(listenerIndex).toBeGreaterThanOrEqual(0);
    expect(initialSnapshotIndex).toBeGreaterThan(listenerIndex);
    expect(source).toContain("api.resyncSnapshot");
    expect(source).toContain("applyGlobalResync");
    expect(source).toContain("pruneTimelineStore");
    expect(source).toContain("api.getSnapshot");
    expect(source).not.toContain("listenStateChanges");
    expect(source).not.toMatch(/koushi-desktop:\/\/state["']/);
    expect(source).not.toContain("stateRefreshTimerRef");
    expect(source).not.toContain("STATE_EVENT_REFRESH_DEBOUNCE_MS");
    expect(source).not.toContain('payload.kind !== "StateDelta"');
    expect(source).not.toContain('Extract<CoreEventPayload, { kind: "StateDelta" }>');
  });

  test("renderer timeline acknowledgement routes are absent", () => {
    const source = readFileSync(new URL("./components/TimelineView.tsx", import.meta.url), "utf8");
    const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const transportSource = readFileSync(
      new URL("./components/timeline/TimelineTransport.ts", import.meta.url),
      "utf8"
    );

    for (const removed of [
      "acknowledgeProjection",
      "acknowledgeRenderedBatch",
      "AcknowledgementInFlightRef",
      "timelineAcknowledgementDelivery"
    ]) {
      expect(source).not.toContain(removed);
      expect(appSource).not.toContain(removed);
      expect(transportSource).not.toContain(removed);
    }
  });

  test("Tauri timeline ensure waits for the webview CoreEvent listener registration", () => {
    const source = readFileSync(
      new URL("./backend/tauriTimelineTransport.ts", import.meta.url),
      "utf8"
    );
    const transportStart = source.indexOf("const tauriTimelineTransport");
    const transportEnd = source.indexOf("export {", transportStart);
    const transportBranch = source.slice(transportStart, transportEnd);

    expect(source).toContain("let tauriCoreEventListenerReady");
    expect(transportBranch).toContain(
      "tauriCoreEventListenerReady = desktopEventPort.listenCoreEvents"
    );
    expect(transportBranch).toContain("async ensureSubscribed");
    expect(transportBranch).toContain("await tauriCoreEventListenerReady");
    expect(transportBranch).toContain("ensure_timeline_subscribed");
  });

  test("composer lifecycle uses one lease registry and Rust-owned IME clear revisions", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).toContain("createComposerDraftLifecycleRegistry");
    expect(source).toContain("composerDraftLifecycleRegistryRef");
    expect(source).not.toContain("composerDraftRevisionsRef");
    expect(source).not.toContain("localComposerDraftRevisionsRef");
    expect(source).not.toContain("localThreadComposerDraftRevisionsRef");
    expect(source).not.toContain("composerDraftPersistTimers");
    expect(source).not.toContain("threadComposerDraftPersistTimers");
    expect(source).not.toContain("localComposerDraftClearEpochs");
    expect(source).not.toContain("threadComposerDraftClearEpochs");
    expect(source).toContain("last_accepted_clear_revision");
    expect(source).toContain('[accountOwnerKey, "main", timelineRoomId ?? "no-room"');
    expect(source).not.toContain("draft_revision].join");
    expect(source).toContain("queueComposerDraftPersist(scope, document, revision)");
    expect(source).toContain("updateComposerTypingSignal(roomId, value)");
    expect(source).toContain("setActiveOverlay(scope, document, revision)");
    expect(source).toContain("activate(scope)");
    expect(source).toContain("deactivate(scope)");
    expect(source).toContain("revokeRendererGeneration()");
    expect(source).toContain("window.setTimeout");
    expect(source).toContain("async function sendText(documentOverride?: ComposerDocument)");
    expect(source).toContain("rendererGeneration");
    expect(source).toContain("leaseId");
    expect(source).toContain("beginOperation(scope)");
    expect(source).toContain("reserveComposerAcceptedRevision(");
    expect(source).toContain("settleOperation(capture)");
    expect(source).not.toContain("composerDraftRevisionForTarget");
  });

  test("desktop api contract is neutral and exposes the search index rebuild command", () => {
    const contractSource = readFileSync(new URL("./backend/desktopApi.ts", import.meta.url), "utf8");
    const source = readFileSync(new URL("./backend/client.ts", import.meta.url), "utf8");
    const runtimeSource = readFileSync(new URL("./backend/appRuntime.ts", import.meta.url), "utf8");

    expect(contractSource).toContain("export interface DesktopApi");
    expect(contractSource).toContain("rebuildSearchIndex(): Promise<CommandAdmission>");
    expect(source).toContain('this.invokeCommand<CommandAdmission>("rebuild_search_index"');
    expect(source).not.toContain("createDesktopApi");
    expect(source).not.toContain("function isTauriRuntime");
    expect(runtimeSource).toContain("new TauriDesktopApi()");
    expect(runtimeSource).not.toContain("browserFakeApi");
  });

  test("renders encryption recovery as a contextual right panel mode", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { ContextualRightPanel } = await import("./App");
    const api = createDesktopApiFixture(awaitingVerificationSnapshotFixture());
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
        onThreadComposerDocumentChange={() => undefined}
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

describe("desktop integration source guards", () => {
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
    // #452: the overlay must take over before the inline grid runs out of room.
    // The grid minimum is 72 + 318 + 420 + 390 = 1200px, so this breakpoint moved
    // up from 1180px, which had left 1181-1199px clipping the panel off-window.
    expect(styles).toContain("@media (min-width: 761px) and (max-width: 1199.98px)");
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
    // #330: the header is the only entry point to a room's threads now, so it is
    // offered unconditionally rather than gated on unread thread attention.
    expect(paneSource).not.toContain("showThreadsHeader");
    expect(paneSource).toContain("onOpenThreadsStable");
  });

  test("room creation leaves space-child linking to the backend-created room id", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const createStart = source.indexOf("async function submitCreateDialog");
    const createEnd = source.indexOf("async function setComposerReplyTarget", createStart);
    const createSource = source.slice(createStart, createEnd);

    expect(createSource).toContain("activeSpaceIdForCreatedRoom");
    expect(createSource).toContain("createRoomRequestFromDraft");
    expect(createSource).toContain("api.createRoom(createRoomRequest");
    expect(createSource).not.toContain("active_room_id");
    expect(createSource).not.toContain("api.setSpaceChild(");
  });

  test("directory search queries the chosen homeserver, not always the user's own", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const queryStart = source.indexOf("async function queryDirectory");
    const queryEnd = source.indexOf("async function submitDirectorySearch", queryStart);
    const querySource = source.slice(queryStart, queryEnd);

    // A hardcoded null limited discovery to the user's own server directory,
    // which cannot find rooms or spaces hosted elsewhere.
    expect(querySource).toContain("server_name: directoryServerDraft.trim() || null");
    expect(querySource).not.toContain("server_name: null");
  });

  test("joining a directory room shows the backend-selected timeline", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const joinStart = source.indexOf("async function joinDirectoryRoom");
    const joinEnd = source.indexOf("function openCreateDialog", joinStart);
    const joinSource = source.slice(joinStart, joinEnd);

    // A result row goes through the one shared preview path, so a link click
    // and a directory Join cannot drift into different join behavior.
    expect(joinSource).toContain("previewJoinTarget(");
    // A public space often has no alias, so the row falls back to the room id
    // rather than becoming findable but unjoinable.
    expect(joinSource).toContain("alias ?? room.room_id");
    expect(joinSource).toContain("serverNameFromMatrixId(target)");
    expect(joinSource).not.toContain("previousRoomIds");
    expect(joinSource).not.toContain("api.selectRoom(");

    const confirmStart = source.indexOf("async function confirmDirectoryJoin");
    const confirmEnd = source.indexOf("async function dismissDirectoryPreview", confirmStart);
    const confirmSource = source.slice(confirmStart, confirmEnd);

    // The join must reuse the Rust-owned target that resolved the preview,
    // not re-derive one in React.
    expect(confirmSource).toContain("preview.room_id_or_alias");
    expect(confirmSource).toContain("preview.via_servers");
    expect(confirmSource).toContain("api.joinDirectoryRoom(");
    expect(confirmSource).toContain('setPrimaryView("timeline")');
    expect(confirmSource).toContain("settleCommand(");
    expect(confirmSource).not.toContain("setSnapshot(");
  });

  test("naming a room opens the Rust-owned preview instead of joining it outright", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const previewStart = source.indexOf("async function previewJoinTarget");
    const previewEnd = source.indexOf("async function confirmDirectoryJoin", previewStart);
    const previewSource = source.slice(previewStart, previewEnd);

    expect(previewSource).toContain("api.previewJoinTarget(");
    // Joining from here would put the user in a room they never saw.
    expect(previewSource).not.toContain("api.joinDirectoryRoom(");

    // Both entry points must reach the preview, never the join directly.
    const openStart = source.indexOf("async function openMatrixTarget");
    const openEnd = source.indexOf("async function joinDirectoryRoom", openStart);
    expect(source.slice(openStart, openEnd)).toContain("previewJoinTarget(");
  });

  test("room mark-as-read prefers the room latest event over stale markers", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const actionStart = source.indexOf('case "markRoomAsRead"');
    const actionEnd = source.indexOf('case "markRoomAsUnread"', actionStart);
    const actionSource = source.slice(actionStart, actionEnd);

    expect(actionStart).toBeGreaterThanOrEqual(0);
    expect(actionSource).toContain("fully_read_event_id");
    expect(actionSource).toContain("roomLatestDisplayEventId(room?.latest_event)");
    expect(actionSource.indexOf("roomLatestDisplayEventId(room?.latest_event)")).toBeLessThan(
      actionSource.indexOf("fully_read_event_id")
    );
    expect(actionSource).toContain("eventId.trim().length > 0");
    expect(actionSource).toContain("api.markRoomAsRead(target.roomId, eventId)");
  });

  test("renders verification states before and mutually exclusive with the desktop shell", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

    expect(source).not.toMatch(
      /snapshot\.state\.session\.kind === "needsRecovery"[\s\S]{0,240}<RecoveryScreen/
    );
    expect(source).toContain("SessionVerificationGate");
    expect(source).toContain('sessionKind !== "ready"');
    expect(source).not.toContain("recoveryRequired");
  });

  test("event navigation presentation is owned by the current Rust terminal", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const eventNavigationStart = source.indexOf("const eventNavigation =");
    const eventNavigationEffectEnd = source.indexOf("const homeSelection =", eventNavigationStart);
    const eventNavigationSource = source.slice(eventNavigationStart, eventNavigationEffectEnd);
    const openActivityStart = source.indexOf("function openActivityRow");
    const openActivityEnd = source.indexOf("function selectSearchResult", openActivityStart);
    const searchSource = source.slice(openActivityEnd, source.indexOf("function runContextMenuAction", openActivityEnd));

    expect(eventNavigationStart).toBeGreaterThanOrEqual(0);
    expect(eventNavigationSource).toContain('eventNavigation?.kind !== "anchored"');
    expect(eventNavigationSource).toContain('eventNavigation?.kind !== "liveFallback"');
    expect(source).toContain('eventNavigation?.kind === "failed"');
    expect(eventNavigationSource).toContain('setPrimaryView("timeline")');
    expect(eventNavigationSource).toContain('eventNavigation.source === "search"');
    expect(eventNavigationSource).toContain('eventNavigation.source === "activity"');
    expect(eventNavigationSource).not.toContain("navigationFailure");
    expect(eventNavigationSource).not.toContain("api.");
    expect(searchSource).not.toContain('setPrimaryView("timeline")');
    expect(searchSource).not.toContain('setRightPanelMode("search")');
  });

  test("pinned event navigation delegates failure and presentation to Rust", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const openPinnedStart = source.indexOf("async function openPinnedEvent");
    const openPinnedEnd = source.indexOf("async function closeThreadsListPanel", openPinnedStart);
    const openPinnedSource = source.slice(openPinnedStart, openPinnedEnd);

    expect(openPinnedSource).toContain("api.openPinnedEvent(roomId, eventId)");
    expect(openPinnedSource).not.toContain("setPinnedNavigation");
    expect(openPinnedSource).not.toContain('status: "failed"');
    expect(source).not.toContain("pinnedNavigation");
    expect(source).not.toContain("onRetryPinnedEvent");
  });

  test("search result selection is snapshot-driven and does not scroll the DOM", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const selectSearchResultStart = source.indexOf("function selectSearchResult");
    const selectSearchResultEnd = source.indexOf("function runContextMenuAction");
    const selectSearchResultSource = source.slice(selectSearchResultStart, selectSearchResultEnd);

    expect(selectSearchResultSource).toContain("api.selectSearchResult(roomId, eventId)");
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
      "runInBackground(setComposerReplyTarget(target.message.room_id, target.message.event_id));"
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

  test("activity row selection opens thread rows in the thread panel and room rows as anchored events", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const openActivityRowStart = source.indexOf("function openActivityRow");
    const openActivityRowEnd = source.indexOf("function selectSearchResult");
    const openActivityRowSource = source.slice(openActivityRowStart, openActivityRowEnd);
    const activityRenderStart = source.indexOf("<ActivityPane");
    const activityRenderEnd = source.indexOf("</ActivityPane>", activityRenderStart);
    const activityRenderSource = source.slice(activityRenderStart, activityRenderEnd);

    expect(openActivityRowStart).toBeGreaterThanOrEqual(0);
    expect(openActivityRowSource).toContain(
      "function openActivityRow(roomId: string, eventId: string, threadRootEventId: string | null)"
    );
    expect(openActivityRowSource).toContain("if (threadRootEventId)");
    expect(openActivityRowSource).toContain("await selectRoom(roomId)");
    expect(openActivityRowSource).toContain(
      'await openThread(roomId, threadRootEventId, "existingThread")'
    );
    expect(openActivityRowSource).toContain(".openActivityEvent(roomId, eventId)");
    expect(openActivityRowSource).not.toContain(".selectSearchResult(roomId, eventId)");
    expect(openActivityRowSource).toContain('setRightPanelMode("closed")');
    expect(openActivityRowSource).not.toContain('setRightPanelMode("focusedContext")');
    expect(openActivityRowSource).not.toContain('setRightPanelMode("search")');
    expect(activityRenderSource).toContain(
      "openActivityRow(row.room_id, row.event_id, row.thread_root_event_id)"
    );
    expect(activityRenderSource).toContain('row.kind === "roomUnread"');
    expect(activityRenderSource).toContain("openActivityRoom(row.room_id)");
    expect(activityRenderSource).not.toContain("selectSearchResult(row.room_id, row.event_id)");
  });

  test("home rail button resets Home to Activity Recent instead of restoring the saved Home pane", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const selectSpaceStart = source.indexOf("async function selectSpace(spaceId: string | null)");
    const selectSpaceEnd = source.indexOf("async function reorderSpaces", selectSpaceStart);
    const selectSpaceSource = source.slice(selectSpaceStart, selectSpaceEnd);

    expect(selectSpaceStart).toBeGreaterThanOrEqual(0);
    expect(selectSpaceSource).toContain("if (spaceId === null)");
    expect(selectSpaceSource).toContain('openHomeActivityView("home_rail")');
    expect(selectSpaceSource).not.toContain("api.selectSpace(null)");
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

  test("login submit reauthenticates the locked session instead of starting a fresh login", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const submitLoginStart = source.indexOf("async function submitLogin");
    const submitLoginEnd = source.indexOf("async function discoverLoginMethods", submitLoginStart);
    const submitLoginSource = source.slice(submitLoginStart, submitLoginEnd);

    expect(submitLoginStart).toBeGreaterThanOrEqual(0);
    expect(submitLoginSource).toContain("snapshot?.state.domain.session.kind");
    expect(submitLoginSource).toContain('sessionKind === "locked"');
    expect(submitLoginSource).toContain("api.submitSoftLogoutReauth(password)");
    expect(submitLoginSource).toContain("api.submitLogin(");
  });

  test("timeline header omits date jump while keeping anchored return-to-live", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const panesSource = readFileSync(
      new URL("./components/panes.tsx", import.meta.url),
      "utf8"
    );
    const messagesSource = readFileSync(new URL("./i18n/messages.ts", import.meta.url), "utf8");
    const stylesSource = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

    expect(panesSource).not.toContain("timeline.jumpToDate");
    expect(panesSource).not.toContain("timeline-date-jump");
    expect(panesSource).not.toContain("openAtTimestamp(");
    expect(panesSource).not.toContain("CalendarDays");
    expect(messagesSource).not.toContain("jumpToDate");
    expect(messagesSource).not.toContain("openDateInTimeline");
    expect(stylesSource).not.toContain("timeline-date-jump");

    expect(source).toContain("timelineTransport={appTimelineTransport}");
    expect(panesSource).toContain("main_timeline_anchor");
    expect(panesSource).toContain("focusedTimelineKey");
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

  test("retains the anchored main timeline key while focused context panel is closed", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { retainedTimelineStoreKeyIds } = await import("./App");
    const snapshot = await createDesktopApiFixture().getSnapshot();
    snapshot.state.ui.navigation.main_timeline_anchor = {
      event_id: "$seed-event:example.invalid"
    };
    snapshot.state.ui.focused_context = { kind: "closed" };
    const userId =
      snapshot.state.domain.session.kind === "ready"
        ? snapshot.state.domain.session.user_id!
        : "";
    const roomId = snapshot.state.ui.timeline.room_id!;

    expect(retainedTimelineStoreKeyIds(snapshot)).toContain(
      timelineStoreKeyId(
        focusedTimelineKey(
          userId,
          roomId,
          "$seed-event:example.invalid"
        )
      )
    );
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
    expect(openActivityRoomSource).toContain("await selectRoom(roomId)");
    expect(openActivityRoomSource).not.toContain("api.closeFocusedContext()");
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
    expect(unreadTypeSource).toContain("thread_root_event_id: null");
  });

  test("member-panel avatar thumbnail requests respect the global avatar download gate", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const renderStart = source.indexOf("<ContextualRightPanel");
    const renderEnd = source.indexOf("</ContextualRightPanel>", renderStart);
    const panelPropsSource = source.slice(renderStart, renderEnd);

    expect(panelPropsSource).toMatch(
      /onRequestMemberAvatarThumbnail=\{\s*AVATAR_THUMBNAIL_DOWNLOADS_ENABLED\s*\?\s*requestMemberAvatarThumbnail\s*:\s*undefined\s*\}/
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
    const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const hookSource = readFileSync(
      new URL("./app/useDesktopAttentionEffects.ts", import.meta.url),
      "utf8"
    );
    const attentionDomainSource = readFileSync(
      new URL("./domain/desktopAttention.ts", import.meta.url),
      "utf8"
    );
    const notificationDomainSource = readFileSync(
      new URL("./domain/desktopNotification.ts", import.meta.url),
      "utf8"
    );

    const summaryStart = appSource.indexOf("const attentionSummary");
    const summaryEnd = appSource.indexOf("function handleShortcutAction", summaryStart);
    const summarySource = appSource.slice(summaryStart, summaryEnd);
    expect(summarySource).toContain("desktopAttentionSummary(snapshot.state.domain.native_attention)");
    expect(summarySource).not.toContain("snapshot.state.domain.rooms");
    expect(summarySource).not.toContain("navigation.active_room_id");

    const notificationStart = hookSource.indexOf("const candidate = desktopAttentionNotificationCandidate");
    const notificationEnd = hookSource.indexOf("void sendDesktopAttentionNotification", notificationStart);
    const notificationSource = hookSource.slice(notificationStart, notificationEnd);
    expect(notificationSource).toContain("snapshot.state.domain.native_attention");
    expect(notificationSource).not.toContain("previousAttentionInput");
    expect(notificationSource).not.toContain("snapshot.state.domain.rooms");

    const notificationEffectEnd = hookSource.indexOf("]);", notificationStart);
    const notificationEffectSource = hookSource.slice(notificationStart, notificationEffectEnd);
    expect(notificationEffectSource).toContain("void dispatchDesktopAttentionTransientEffects");
    expect(notificationEffectSource).toContain("{ sound: false }");
    expect(notificationEffectSource).toContain("snapshot.state.domain.native_attention.summary.capabilities");
    expect(notificationEffectSource).not.toContain("snapshot.state.domain.rooms");

    const clearStart = hookSource.indexOf("safeAttentionSummary.badgeCount !== 0");
    const clearEnd = hookSource.indexOf("  }, [safeAttentionSummary.badgeCount]);", clearStart);
    const clearSource = hookSource.slice(clearStart, clearEnd);
    expect(clearSource).toContain("safeAttentionSummary.badgeCount !== 0");
    expect(clearSource).toContain("void clearDesktopAttentionNotifications");
    expect(clearSource).toContain("desktopAttentionPort.notifications");

    expect(appSource).toContain("desktopAttentionWindowTitle");
    expect(hookSource).toContain("sendDesktopAttentionNotification");
    expect(hookSource).toContain("createDesktopBadgeSoundDispatcher");
    expect(hookSource).toContain("desktopBadgeSoundDispatcher.observe");
    expect(hookSource).toContain("applyDesktopAttentionToWindow");
    expect(appSource).toContain("qaWindowTitle(");
    expect(appSource).toContain("effectiveRightPanelModeForSnapshot");
    expect(appSource).toContain("rightPanelMode");
    expect(appSource).toContain("qaSendStatus");
    expect(hookSource).toContain("desktopAttentionPort.currentWindow()");
    expect(hookSource).not.toContain("@tauri-apps");
    expect(attentionDomainSource).not.toContain("@tauri-apps");
    expect(notificationDomainSource).not.toContain("@tauri-apps");
    expect(hookSource).toContain("snapshot?.state.domain.native_attention.summary.capabilities");
    expect(hookSource).toContain("document.title = attentionWindowTitle");
    const windowEffectStart = hookSource.indexOf("useEffect(() => {\n    document.title");
    const windowEffectEnd = hookSource.indexOf("]);", windowEffectStart);
    const windowEffectSource = hookSource.slice(windowEffectStart, windowEffectEnd);
    expect(windowEffectSource).toContain("attentionWindowTitle");
    expect(windowEffectSource).not.toContain("\n    snapshot,");
    expect(appSource).toContain("desktopAttentionWindowTitle");
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
    expect(selectRoomSource).toContain("stage=before_composer_drain");
    expect(selectRoomSource).toContain("stage=after_composer_drain");
    expect(selectRoomSource).toContain("outcome=blocked");
    expect(selectRoomSource).toContain("outcome=continue");
    expect(selectRoomSource).toContain("stage=before_primary_view_update");
    expect(selectRoomSource).toContain("stage=after_primary_view_update");
    expect(selectRoomSource).toContain("stage=before_api_select");
    expect(selectRoomSource).toContain("stage=after_api_select");
    expect(selectRoomSource).toContain("stage=after_state_reconcile");
    expect(selectRoomSource).toContain("elapsed_ms_since_start=");
    expect(selectRoomSource).toContain("timeline_matches=");
    expect(startOffset).toBeGreaterThanOrEqual(0);
    expect(apiOffset).toBeGreaterThan(startOffset);
    expect(doneOffset).toBeGreaterThan(apiOffset);
  });

  test("home selection appends private-data-free transition diagnostics", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const fnStart = source.indexOf("const openHomeSelection = useCallback");
    const fnEnd = source.indexOf("async function selectSpace", fnStart);
    expect(fnStart).toBeGreaterThanOrEqual(0);
    expect(fnEnd).toBeGreaterThan(fnStart);
    const openHomeSelectionSource = source.slice(fnStart, fnEnd);

    expect(openHomeSelectionSource).toContain('source: "home.transition"');
    expect(openHomeSelectionSource).toContain("stage=submit");
    expect(openHomeSelectionSource).toContain("selection=");
    expect(openHomeSelectionSource).toContain("current_active_room_present=");
    expect(openHomeSelectionSource).toContain("current_timeline_present=");
    expect(openHomeSelectionSource).toContain("stage=after_composer_drain");
    expect(openHomeSelectionSource).toContain("outcome=blocked");
    expect(openHomeSelectionSource).toContain("outcome=continue");
    expect(openHomeSelectionSource).toContain("stage=after_select_space");
    expect(openHomeSelectionSource).toContain("active_room_present=");
    expect(openHomeSelectionSource).toContain("timeline_present=");
    expect(openHomeSelectionSource).toContain("stage=after_view_apply");
    expect(openHomeSelectionSource).toContain("elapsed_ms_since_start=");
  });

  test("space selection keeps transition diagnostics on structured Rust lanes", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const tauriNavigationSource = readFileSync(
      new URL("../src-tauri/src/commands/navigation.rs", import.meta.url),
      "utf8"
    );
    const runtimeSource = readFileSync(
      new URL("../../../crates/koushi-core/src/runtime.rs", import.meta.url),
      "utf8"
    );
    const fnStart = source.indexOf("async function selectSpace(spaceId: string | null)");
    const fnEnd = source.indexOf("async function reorderSpaces", fnStart);
    expect(fnStart).toBeGreaterThanOrEqual(0);
    expect(fnEnd).toBeGreaterThan(fnStart);
    const selectSpaceSource = source.slice(fnStart, fnEnd);

    const apiOffset = selectSpaceSource.indexOf("api.selectSpace(spaceId)");

    expect(apiOffset).toBeGreaterThanOrEqual(0);
    expect(selectSpaceSource).not.toContain('source: "space.transition"');
    expect(selectSpaceSource).not.toContain("stage=select_");
    expect(tauriNavigationSource).toContain('"desktop.space.transition"');
    expect(tauriNavigationSource).toContain("DiagnosticField::request_id");
    expect(tauriNavigationSource).toContain('"request_id"');
    expect(tauriNavigationSource).toContain("DiagnosticField::boolean");
    expect(runtimeSource).toContain('"core.space.transition"');
    expect(runtimeSource).toContain("DiagnosticField::boolean");
    expect(runtimeSource).toContain('"active_room_changed"');
    expect(runtimeSource).toContain("DiagnosticField::count");
    expect(runtimeSource).toContain('"rooms"');
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

    const authRequiredMarkup = renderToStaticMarkup(
      <TopBar
        {...baseProps}
        sync={
          {
            failed: "sync_failed_auth"
          } as DesktopSnapshot["state"]["domain"]["sync"]
        }
      />
    );
    expect(authRequiredMarkup).toContain("Sign-in required");
    expect(authRequiredMarkup).not.toContain('aria-label="Restart sync"');
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
          megolm_session_fingerprint: "AbCdEfGhIjKl",
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
    expect(markup).toContain("Encryption details");
    expect(markup).toContain("Megolm session fingerprint");
    expect(markup).toContain("AbCdEfGhIjKl");
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
              latest_event_id: "$latest-thread-reply:example.invalid",
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
                source_ref: "https://fixture.invalid/avatar.png",
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

    expect(markup).toContain('<img src="https://fixture.invalid/avatar.png"');
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
    expect(markup).toContain("Unknown user");
    expect(markup).not.toContain("@alice:example.invalid");
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

  test("main and thread caption edits retain bounded mounted-editor ordering", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const mainStart = source.indexOf("async function updateStagedUploadCaption(");
    const mainEnd = source.indexOf("async function selectStagedUploadOutput(", mainStart);
    const threadStart = source.indexOf("async function updateThreadStagedUploadCaption(");
    const threadEnd = source.indexOf("async function selectThreadStagedUploadOutput(", threadStart);
    const mainSource = source.slice(mainStart, mainEnd);
    const threadSource = source.slice(threadStart, threadEnd);

    expect(mainSource).toContain("applyLatestTextMutationReceipt(`caption:main:");
    expect(mainSource).toContain("api.updateStagedUploadCaption(");
    expect(threadSource).toContain("applyLatestTextMutationReceipt(`caption:thread:");
    expect(threadSource).toContain("api.updateStagedUploadCaption(");
    expect(source.match(/api\.updateStagedUploadCaption\(/g)).toHaveLength(2);
    expect(source.match(/`caption:main:/g)).toHaveLength(3);
    expect(source.match(/`caption:thread:/g)).toHaveLength(3);
    expect(source).toContain("Renderer-owned mounted-editor ordering");
  });

  test("room and Space settings effects retain renderer-local demand ownership", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    const roomEffectStart = source.indexOf('rightPanelMode !== "roomInfo"');
    const spaceEffectStart = source.indexOf('rightPanelMode !== "spaceInfo"');
    const nextEffectStart = source.indexOf("\n  useEffect(() => {", spaceEffectStart + 1);
    const roomEffect = source.slice(roomEffectStart, spaceEffectStart);
    const spaceEffect = source.slice(spaceEffectStart, nextEffectStart);

    expect(roomEffect).toContain("settleCommand(api.loadRoomSettings(activeRoomId))");
    expect(roomEffect).not.toContain("requestId");
    expect(roomEffect).toContain("roomSettingsLoadRef.current = null");
    expect(roomEffect).toContain(".catch(() => {");
    expect(spaceEffect).toContain("settleCommand(api.loadRoomSettings(activeSpaceId))");
    expect(spaceEffect).not.toContain("requestId");
    expect(spaceEffect).not.toContain("navigationRequestId");
    expect(spaceEffect).toContain("spaceSettingsLoadRef.current = null");
    expect(spaceEffect).toContain(".catch(() => {");

    const panelModeHelperStart = source.indexOf(
      "async function setRightPanelModeClosingFocusedContext"
    );
    const openSpaceMembersStart = source.indexOf("async function openSpaceMembers");
    const panelModeHelper = source.slice(panelModeHelperStart, openSpaceMembersStart);
    const openSpaceMembersEnd = source.indexOf("\n    try {", openSpaceMembersStart);
    const openSpaceMembersPrelude = source.slice(openSpaceMembersStart, openSpaceMembersEnd);
    expect(panelModeHelper.match(/spaceSettingsRequestRef\.current \+= 1/g)).toHaveLength(1);
    expect(openSpaceMembersPrelude).not.toContain("spaceSettingsRequestRef.current += 1");
    expect(openSpaceMembersPrelude).toContain(
      'setRightPanelModeClosingFocusedContext(\n      "people"'
    );
  });

  test("invite workflow Tauri commands delegate convergence to Core outcomes", () => {
    const source = readFileSync(
      new URL("../src-tauri/src/commands/room.rs", import.meta.url),
      "utf8"
    );
    const start = source.indexOf("pub async fn open_invite_workflow");
    const end = source.indexOf("pub async fn set_invite_scope", start);
    expect(start).toBeGreaterThanOrEqual(0);
    expect(end).toBeGreaterThan(start);
    const workflowCommands = source.slice(start, end);

    expect(source).toContain("INVITE_WORKFLOW_CONVERGENCE_TIMEOUT");
    expect(workflowCommands).toContain("RequestOutcomeExpectation::InviteWorkflow");
    expect(workflowCommands).not.toContain("current_snapshot(state.inner())");
    expect(workflowCommands.match(/\.wait_for_request_outcome/g)).toHaveLength(3);
  });

  test("rejected login transport refreshes authoritative gate state without rejecting", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { settleLoginTransport } = await import("./App");
    const gate = structuredClone(awaitingVerificationSnapshotFixture());
    const apply = vi.fn();
    await expect(
      settleLoginTransport(
        Promise.reject(new Error("login timeout")),
        async () => undefined,
        async () => gate,
        apply
      )
    ).resolves.toBe("Sign-in failed. Please try again.");
    expect(apply).toHaveBeenCalledWith(gate);
  });

  test("login transport does not duplicate an authoritative projected failure", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { settleLoginTransport } = await import("./App");
    const failed = structuredClone(awaitingVerificationSnapshotFixture());
    failed.state.ui.errors.push({ code: "login_failed", message: "Login failed", recoverable: true });
    await expect(settleLoginTransport(
      Promise.reject(new Error("ipc")),
      async () => undefined,
      async () => failed,
      () => undefined
    )).resolves.toBeNull();
  });

  test("unrelated projected errors do not hide a rejected login transport", async () => {
    vi.stubGlobal("window", { location: { search: "" } });
    const { settleLoginTransport } = await import("./App");
    const snapshot = structuredClone(awaitingVerificationSnapshotFixture());
    snapshot.state.ui.errors.push({ code: "media_download_failed", message: "Old media error", recoverable: true });
    await expect(
      settleLoginTransport(
        Promise.reject(new Error("ipc")),
        async () => undefined,
        async () => snapshot,
        () => undefined
      )
    ).resolves.toBe("Sign-in failed. Please try again.");
  });

  test("ready with non-running sync renders the normal shell", () => {
    const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
    expect(source).not.toContain('(sessionKind === "ready" && snapshot.state.domain.sync !== "running")');
  });
});
