import { readdirSync, readFileSync } from "node:fs";
import * as ts from "typescript";
import { describe, expect, test } from "vitest";
import { contextMenuItems } from "../domain/contextMenus";
import { elementShortcutParity, keyboardShortcutGroups } from "../domain/shortcuts";
import {
  MESSAGE_PLACEHOLDER_PATTERN,
  catalogs,
  localeLanguageTag,
  pseudoLocalize,
  setActiveLocaleProfile,
  t,
  tCount,
  type CountMessageId,
  type Locale,
  type MessageId
} from "./messages";

describe("i18n message catalog", () => {
  test("the assistant attachments approval covers sending and downloading (#1533)", () => {
    expect(t("mcp.approveAttachmentsTitle", { client: "Agent" }, "en", "none")).toBe(
      "Allow Agent to send and download attachments?"
    );
    expect(catalogs.en["mcp.wantsAttachments"]).toContain("send and download");
    expect(catalogs.en["mcp.attachmentsExplain"]).toContain("send files");
    expect(catalogs.en["mcp.allowAttachments"]).not.toContain("download");
    expect(catalogs.ja["mcp.approveAttachmentsTitle"]).toContain("送信とダウンロード");
    expect(catalogs.ko["mcp.approveAttachmentsTitle"]).toContain("보내고 다운로드");
    expect(catalogs["zh-Hans"]["mcp.approveAttachmentsTitle"]).toContain("发送和下载");
    expect(catalogs["zh-Hant"]["mcp.approveAttachmentsTitle"]).toContain("傳送和下載");
  });

  test("count messages pick the singular for exactly one (#1546)", () => {
    expect(tCount("people.memberCount", 1)).toBe("1 member");
    expect(tCount("people.memberCount", 2)).toBe("2 members");
    expect(tCount("people.memberCount", 0)).toBe("0 members");
    expect(tCount("directory.memberCount", 1)).toBe("1 member");
    expect(tCount("directory.memberCount", 1200, { count: "1,200" })).toBe("1,200 members");
    expect(tCount("spaceMembers.joinedCount", 1)).toBe("1 joined member");
    expect(tCount("spaceMembers.joinedCount", 3)).toBe("3 joined members");
    expect(tCount("threads.replyCount", 1)).toBe("1 reply");
    expect(tCount("threads.replyCount", 2)).toBe("2 replies");
    // The Threads list and the timeline chip read the same for one reply.
    for (const locale of ["en", "ja", "ko", "zh-Hans", "zh-Hant"] as const) {
      expect(t("threads.replyCountOne", { count: 1 }, locale, "none")).toBe(
        t("timeline.threadReplyCountOne", {}, locale, "none")
      );
      expect(t("threads.replyCount", { count: 5 }, locale, "none")).toBe(
        t("timeline.threadReplyCountMany", { count: 5 }, locale, "none")
      );
    }
  });

  test("member counts use one phrasing per language and do not repeat the panel title (#1546)", () => {
    const countIds: CountMessageId[] = [
      "people.memberCount",
      "directory.memberCount",
      "spaceMembers.joinedCount"
    ];
    for (const locale of ["ja", "ko", "zh-Hans", "zh-Hant"] as const) {
      const catalog = catalogs[locale];
      const people = catalog["panel.people"];
      expect(catalog["people.memberCount"]).not.toContain(people);
      const counter = { ja: " 人", ko: "명", "zh-Hans": " 人", "zh-Hant": " 人" }[locale];
      for (const id of countIds) {
        expect(catalog[id]).toContain(`{count}${counter}`);
        expect(catalog[`${id}One`]).toContain(`{count}${counter}`);
      }
      expect(catalog["spaceMembers.navAccessible"]).toContain(`{joined}${counter}`);
      expect(catalog["spaceMembers.navAccessible"]).toContain(`{childOnly}${counter}`);
    }
    // ko names the People panel with the same word its member counts use.
    expect(catalogs.ko["panel.people"]).toBe("멤버");
    expect(catalogs.ko["directory.memberCount"]).toContain("멤버");
  });

  test("names the sidebar sort selector Recent and Name only (#542)", () => {
    expect(catalogs.en["roomList.sortRecent"]).toBe("Recent");
    expect(catalogs.ja["roomList.sortRecent"]).toBe("新着順");
    expect(catalogs.en["roomList.sortName"]).toBeTruthy();
    expect(catalogs.ja["roomList.sortName"]).toBeTruthy();
    // The attention-first choice is legacy-only; it must not be offered and
    // must not leave a stale "Active" label behind.
    for (const locale of Object.keys(catalogs) as Locale[]) {
      expect("roomList.sortActive" in catalogs[locale]).toBe(false);
      expect("roomList.sortAttention" in catalogs[locale]).toBe(false);
    }
  });

  test("ships secure backup gate copy in both locales, including the account-wide warning", () => {
    const secureBackupIds = [
      "gate.secureBackupTitle",
      "gate.secureBackupChecking",
      "gate.secureBackupInactive",
      "gate.secureBackupNeedsRecovery",
      "gate.secureBackupRecoveryKey",
      "gate.secureBackupRecover",
      "gate.secureBackupSetupTitle",
      "gate.secureBackupSetupCopy",
      "gate.secureBackupPassphrase",
      "gate.secureBackupRecoveryKeyDestination",
      "gate.secureBackupRecoveryKeyTitle",
      "gate.secureBackupRecoveryKeyCopy",
      "gate.secureBackupCopyRecoveryKey",
      "gate.secureBackupRecoveryKeyCopied",
      "gate.secureBackupRecoveryKeyCopyFailed",
      "gate.secureBackupSaveRecoveryKeyToFile",
      "gate.secureBackupRecoveryKeySavedToFile",
      "gate.secureBackupRecoveryKeySaveFailed",
      "gate.secureBackupRecoveryKeyLost",
      "gate.secureBackupResetRecoveryKey",
      "gate.secureBackupResetRecoveryKeyTitle",
      "gate.secureBackupResetRecoveryKeyWarning",
      "gate.secureBackupResetRecoveryKeyConfirm",
      "gate.secureBackupDeliveryTitle",
      "gate.secureBackupSetup",
      "gate.secureBackupExplicitDisabledTitle",
      "gate.secureBackupExplicitDisabledCopy",
      "gate.secureBackupReenable",
      "gate.secureBackupReenableConfirm",
      "gate.secureBackupCreating",
      "gate.secureBackupDeliveryRequired",
      "gate.secureBackupUploading",
      "gate.secureBackupPendingZero",
      "gate.secureBackupPendingOne",
      "gate.secureBackupPendingTwoToTen",
      "gate.secureBackupPendingElevenToOneHundred",
      "gate.secureBackupPendingOverOneHundred",
      "gate.secureBackupPendingUnknown",
      "gate.secureBackupRetrying",
      "gate.secureBackupFailureNetwork",
      "gate.secureBackupFailureRateLimited",
      "gate.secureBackupFailureInvalidRecoveryKey",
      "gate.secureBackupFailureBackupKeyMismatch",
      "gate.secureBackupFailureSecretStorageIncomplete",
      "gate.secureBackupFailureArtifactDelivery",
      "gate.secureBackupFailureForbidden",
      "gate.secureBackupFailureTimeout",
      "gate.secureBackupFailureSdk",
      "gate.secureBackupRetry",
      "gate.secureBackupDiagnostics",
      "gate.secureBackupCommandFailed",
      "gate.secureBackupRuntimeDegraded"
    ] as const satisfies readonly MessageId[];

    for (const id of secureBackupIds) {
      expect(catalogs.en[id]).toBeTruthy();
      expect(catalogs.ja[id]).toBeTruthy();
    }
    expect(t("gate.secureBackupExplicitDisabledCopy")).toMatch(/other Matrix clients/i);
    expect(t("gate.secureBackupExplicitDisabledCopy", {}, "ja")).toMatch(/他の Matrix クライアント/);
  });

  test("all locales expose the same message ids", () => {
    const locales = Object.keys(catalogs) as Locale[];
    const baseline = Object.keys(catalogs.en).sort();
    for (const locale of locales) {
      expect(Object.keys(catalogs[locale]).sort()).toEqual(baseline);
    }
  });

  test("interpolates named values", () => {
    expect(t("composer.placeholder", { roomName: "Synthetic Room" })).toBe(
      "Message Synthetic Room"
    );
  });

  test("localizes structured room name notices with plain international text", () => {
    const oldName = "研究室 🧪 العربية";
    const newName = "<新しい部屋>";

    expect(t("timeline.notice.roomNameSet", { newName })).toBe(
      "set the room name to <新しい部屋>"
    );
    expect(t("timeline.notice.roomNameChanged", { oldName, newName })).toBe(
      "changed the room name from 研究室 🧪 العربية to <新しい部屋>"
    );
    expect(t("timeline.notice.roomNameRemoved")).toBe("removed the room name");
    expect(t("timeline.notice.roomNameChangedGeneric")).toBe("changed the room name");
    expect(t("timeline.notice.roomNameSet", { newName }, "ja")).toBe(
      "ルーム名を「<新しい部屋>」に設定しました"
    );
    expect(t("timeline.notice.roomNameChanged", { oldName, newName }, "ja")).toBe(
      "ルーム名を「研究室 🧪 العربية」から「<新しい部屋>」に変更しました"
    );
    expect(t("timeline.notice.roomNameRemoved", {}, "ja")).toBe("ルーム名を削除しました");
    expect(t("timeline.notice.roomNameChangedGeneric", {}, "ja")).toBe(
      "ルーム名が変更されました"
    );
  });

  test("localizes the single pin and unpin notices (#1591)", () => {
    expect(t("timeline.notice.roomPinnedEventPinned")).toBe("pinned a message");
    expect(t("timeline.notice.roomPinnedEventUnpinned")).toBe("unpinned a message");
    expect(t("timeline.notice.roomPinnedEvents")).toBe("updated pinned messages");
    expect(t("timeline.notice.roomPinnedEventPinned", {}, "ja")).toBe(
      "メッセージをピン留めしました"
    );
    expect(t("timeline.notice.roomPinnedEventUnpinned", {}, "ja")).toBe(
      "メッセージのピン留めを解除しました"
    );
  });

  test("Japanese catalog localizes shipped message ids except the named allowlist", () => {
    const identicalMessageIds = (Object.keys(catalogs.en) as MessageId[]).filter(
      (id) => catalogs.ja[id] === catalogs.en[id] && !japaneseIdenticalMessageAllowlist.has(id)
    );

    expect(identicalMessageIds).toEqual([]);
  });

  test("Japanese catalog explicitly overrides every English message id except the named allowlist", () => {
    // `ja` spreads `en`, so a missing translation silently keeps the English
    // text at runtime (#1058). Only the source literal shows which ids the
    // Japanese catalog overrides explicitly.
    const url = new URL("./messages.ts", import.meta.url);
    const sourceFile = ts.createSourceFile(
      "messages.ts",
      readFileSync(url, "utf8"),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TS
    );
    let jaLiteral: ts.ObjectLiteralExpression | undefined;
    function visit(node: ts.Node): void {
      if (
        ts.isVariableDeclaration(node) &&
        ts.isIdentifier(node.name) &&
        node.name.text === "ja" &&
        node.initializer &&
        ts.isObjectLiteralExpression(node.initializer)
      ) {
        jaLiteral = node.initializer;
        return;
      }
      ts.forEachChild(node, visit);
    }
    visit(sourceFile);
    expect(jaLiteral).toBeDefined();

    const explicitJapaneseIds = new Set<string>();
    for (const property of jaLiteral?.properties ?? []) {
      if (ts.isPropertyAssignment(property) && ts.isStringLiteral(property.name)) {
        explicitJapaneseIds.add(property.name.text);
      }
    }
    const inheritedIds = Object.keys(catalogs.en).filter(
      (id) => !explicitJapaneseIds.has(id) && !japaneseInheritedMessageAllowlist.has(id as MessageId)
    );

    expect(inheritedIds).toEqual([]);
    for (const id of japaneseInheritedMessageAllowlist) {
      expect(explicitJapaneseIds.has(id)).toBe(false);
    }
  });

  test("product branding uses Koushi in English and Japanese", () => {
    expect(t("app.title")).toBe("Koushi");
    expect(t("window.title")).toBe("Koushi");
    expect(t("auth.matrixDesktop")).toBe("Koushi");
    expect(t("app.title", {}, "ja")).toBe("Koushi（光子・格子）");
    expect(t("window.title", {}, "ja")).toBe("Koushi（光子・格子）");
    expect(t("auth.matrixDesktop", {}, "ja")).toBe("Koushi（光子・格子）");
  });

  test("direct-message room list surfaces are labeled consistently", () => {
    expect(t("roomList.filterPeople")).toBe("Direct Messages");
    expect(t("workspace.people")).toBe("Direct Messages");
    expect(t("roomList.filterPeople", {}, "ja")).toBe("Direct Messages");
    expect(t("workspace.people", {}, "ja")).toBe("Direct Messages");
  });

  test("Space member audit labels are localized and keep the exact search wording", () => {
    expect(t("spaceMembers.search")).toBe("Search space members");
    expect(t("spaceMembers.sectionChildOnly")).toBe("Not in Space");
    expect(t("spaceMembers.invite", {}, "ja")).toBe("スペースに招待");
    expect(t("spaceMembers.search", {}, "ja")).toBe("スペースのメンバーを検索");
  });

  test("keeps the Japanese child-room count fallback natural for one and many", () => {
    expect(t("spaceMembers.childRoomCount", { count: 1 }, "ja")).toBe(
      "参加中の子ルーム 1 個"
    );
    expect(t("spaceMembers.childRoomCount", { count: 3 }, "ja")).toBe(
      "参加中の子ルーム 3 個"
    );
  });

  test("localizes Space invite cancellation controls and failure copy", () => {
    expect(t("spaceMembers.cancelInvite")).toBe("Cancel invitation");
    expect(t("spaceMembers.cancelInvitePending")).toBe("Cancelling…");
    expect(t("spaceMembers.cancelInviteFailed")).toBe(
      "Could not cancel the invitation. Try again."
    );
    expect(t("spaceMembers.cancelInvite", {}, "ja")).toBe("招待を取り消す");
    expect(t("spaceMembers.cancelInvitePending", {}, "ja")).toBe("取消中…");
    expect(t("spaceMembers.cancelInviteFailed", {}, "ja")).toBe(
      "招待を取り消せませんでした。もう一度お試しください。"
    );
  });

  test("explains the three-state user trust model in shipped locales", () => {
    expect(t("help.userTrust.title")).toBe("User trust model");
    expect(t("help.userTrust.unverifiedTitle")).toBe("Unverified");
    expect(t("help.userTrust.verifiedTitle")).toBe("Verified");
    expect(t("help.userTrust.identityResetTitle")).toBe("Identity reset");
    expect(t("help.userTrust.title", {}, "ja")).toBe("ユーザー信頼モデル");
    expect(t("help.userTrust.unverifiedTitle", {}, "ja")).toBe("未検証");
    expect(t("help.userTrust.verifiedTitle", {}, "ja")).toBe("検証済み");
    expect(t("help.userTrust.identityResetTitle", {}, "ja")).toBe("IDリセット");
  });

  test("Japanese catalog provides representative localized labels", () => {
    expect(t("composer.replying", {}, "ja")).toBe("返信中");
    expect(t("action.send", {}, "ja")).toBe("送信");
    expect(t("settings.threadRootLatestReply")).toBe(
      "Place threaded conversations at their latest reply"
    );
    expect(t("settings.threadRootLatestReply", {}, "ja")).toBe(
      "スレッドを最新の返信位置に表示する"
    );
  });

  test("localizes the current-session status surface without changing protocol tokens", () => {
    expect(t("sessionStatus.title")).toBe("Current session");
    expect(t("sessionStatus.failureTimedOut")).toBe(
      "Could not check this session before the connection timed out"
    );
    expect(t("sessionStatus.failureConnectivityUnavailable")).toBe(
      "Could not check this session while the connection was unavailable"
    );
    expect(t("sessionStatus.title", {}, "ja")).toBe("現在のセッション");
    expect(t("sessionStatus.failureTimedOut", {}, "ja")).toBe(
      "接続がタイムアウトする前にこのセッションを確認できませんでした"
    );
    expect(t("sessionStatus.authOauth", {}, "ja")).toBe("OAuth 認証");
  });

  test("distinguishes ordinary and threaded reply labels in shipped locales", () => {
    expect(t("timeline.replyToMessage")).toBe("Reply to message");
    expect(t("timeline.replyInThread")).toBe("Reply in thread");
    expect(t("timeline.replyToMessage", {}, "ja")).toBe("メッセージに返信");
    expect(t("timeline.replyInThread", {}, "ja")).toBe("スレッドで返信");
  });

  test("localizes thread notification count copy", () => {
    expect(t("timeline.threadNotificationCount", { count: 3 })).toBe(
      "Thread unread · 3"
    );
    expect(t("timeline.threadNotificationCount", { count: 3 }, "ja")).toBe(
      "スレッドの未読 · 3"
    );
  });

  test("pseudo locale expands labels while preserving interpolation placeholders", () => {
    const pseudo = pseudoLocalize("Message {roomName}");

    expect(pseudo).toContain("{roomName}");
    expect(pseudo.length).toBeGreaterThan("Message {roomName}".length);
    expect(pseudo).not.toContain("roomName roomName");
  });

  test("pseudo catalog expansion handles RTL, CJK, and combining mark samples", () => {
    const sample = "Cafe\u0301 日本語 العربية {roomName}";
    const pseudo = pseudoLocalize(sample);

    expect(pseudo).toContain("\u0301");
    expect(pseudo).toContain("日本語");
    expect(pseudo).toContain("العربية");
    expect(pseudo).toContain("{roomName}");
    expect(pseudo.length).toBeGreaterThan(sample.length);
  });

  test("bidi pseudo mode is distinguishable from accented pseudo mode", () => {
    const sample = "Message {roomName}";
    const accented = pseudoLocalize(sample, "accented");
    const bidi = pseudoLocalize(sample, "bidi");

    expect(bidi).toContain("{roomName}");
    expect(bidi).not.toBe(accented);
    expect(bidi).toContain("\u202e");
    expect(bidi).toContain("\u202c");
  });

  test("runtime pseudo translation keeps interpolated values private-data-owned by caller", () => {
    const pseudo = t("workspace.searchPlaceholder", { spaceName: "Synthetic Space" }, "pseudo");

    expect(pseudo).toContain("Synthetic Space");
    expect(pseudo.length).toBeGreaterThan(
      t("workspace.searchPlaceholder", { spaceName: "Synthetic Space" }, "en").length
    );
  });

  test("active Rust-owned locale profile selects bidi pseudo catalog rendering", () => {
    setActiveLocaleProfile("pseudo", "bidi");
    try {
      const label = t("action.send");

      expect(label).toContain("\u202e");
      expect(label).toContain("\u202c");
      expect(label).not.toBe(t("action.send", {}, "en"));
    } finally {
      setActiveLocaleProfile("en", "none");
    }
  });

  test("product components do not embed raw user-visible strings", () => {
    const componentUrls = [
      new URL("../App.tsx", import.meta.url),
      ...readdirSync(new URL("../components", import.meta.url))
        .filter((name) => name.endsWith(".tsx"))
        .map((name) => new URL(`../components/${name}`, import.meta.url))
    ];
    const findings: string[] = [];

    for (const url of componentUrls) {
      const source = readFileSync(url, "utf8");
      const file = url.pathname.split("/").slice(-2).join("/");
      const sourceFile = ts.createSourceFile(
        file,
        source,
        ts.ScriptTarget.Latest,
        true,
        ts.ScriptKind.TSX
      );

      function visit(node: ts.Node): void {
        if (ts.isJsxText(node)) {
          const text = node.getText(sourceFile).trim().replace(/\s+/g, " ");
          if (text && text !== "koushi-desktop" && /[A-Za-z]/.test(text)) {
            findings.push(`${file}:${lineNumberAt(sourceFile, node)}: literal JSX text "${text}"`);
          }
        }

        if (
          ts.isJsxAttribute(node) &&
          ["aria-label", "placeholder", "title", "alt"].includes(node.name.getText(sourceFile)) &&
          node.initializer &&
          ts.isStringLiteral(node.initializer) &&
          // alt="" marks a decorative image; it is not user-visible text.
          !(node.name.getText(sourceFile) === "alt" && node.initializer.text === "")
        ) {
          findings.push(
            `${file}:${lineNumberAt(sourceFile, node)}: literal ${node.name.getText(sourceFile)} "${node.initializer.text}"`
          );
        }

        ts.forEachChild(node, visit);
      }

      visit(sourceFile);
    }

    expect(findings).toEqual([]);
  });

  test("structured UI registries reference catalog ids, not prose", () => {
    const messageIds = new Set(Object.keys(catalogs.en));
    const ids = [
      ...keyboardShortcutGroups.map((group) => group.categoryMessageId),
      ...elementShortcutParity().flatMap((shortcut) => [
        shortcut.labelMessageId,
        ...(shortcut.noteMessageId ? [shortcut.noteMessageId] : [])
      ]),
      ...contextMenuItems({
        kind: "message",
        canManage: true,
        canEdit: true,
        canReply: true,
        hasThread: true,
        senderUserId: "@a:example.invalid",
        currentUserId: "@a:example.invalid",
        roomId: "!room:example.invalid",
        eventId: "$event:example.invalid",
        isIgnored: false
      }).map((item) => item.labelMessageId),
      ...contextMenuItems({ kind: "room", roomId: "!room:example.invalid" }).map(
        (item) => item.labelMessageId
      ),
      ...contextMenuItems({ kind: "space" }).map((item) => item.labelMessageId),
      ...contextMenuItems({ kind: "account" }).map((item) => item.labelMessageId),
      ...(["ready", "error", "signedOut"] as const).flatMap((status) =>
        contextMenuItems({ kind: "accountTab", status }).map((item) => item.labelMessageId)
      )
    ];

    expect(ids.every((id) => messageIds.has(id))).toBe(true);
  });
});

// Ids the Japanese catalog deliberately inherits from `en` via the spread:
// font names, a format-only string, and protocol acronyms.
const japaneseInheritedMessageAllowlist = new Set<MessageId>([
  "settings.fontInter",
  "settings.twemojiColr",
  "timeline.mediaUploadProgress",
  "auth.flowOidc",
  "auth.flowToken"
]);

const japaneseIdenticalMessageAllowlist = new Set<MessageId>([
  // #1166: the access-condition summary uses the same em dash in both locales,
  // matching the bilingual example in the issue ("スペース例 — 招待制。...").
  "access.conditionSummarySeparator",
  // Scale fractions and image-format names are not prose: "1/2" and "JPEG"
  // read the same in both catalogs, and translating them would be wrong.
  "upload.resizeHalf",
  "upload.resizeQuarter",
  "upload.resizeEighth",
  "upload.previewActualSize",
  "upload.formatWebp",
  // #1135: part of a generated, locale-independent filename rather than prose.
  "timeline.downloadDefaultImageName",
  "upload.formatJpeg",
  "upload.formatPng",
  "auth.flowOidc",
  "auth.flowToken",
  // #1488: an example email address, not prose.
  "settings.notificationDeliverySinkToPlaceholder",
  // "Matrix ID" is the protocol's product term in both locales (Element X ja).
  "auth.matrixId",
  // #1460: "SendKey" is ServerChan's own name for the key on its Chinese site.
  "settings.notificationDeliverySecretServerChan",
  "roomList.filterPeople",
  "space.directMessages",
  "settings.fontInter",
  "settings.twemojiColr",
  // #1221: the language dropdown shows each language's self-name in every
  // locale, so "English" and "日本語" read the same in both catalogs.
  "settings.languageEnglish",
  "settings.languageJapanese",
  "settings.languageSimplifiedChinese",
  "settings.languageTraditionalChinese",
  "settings.languageKorean",
  "timeline.mediaUploadProgress",
  "workspace.people",
  // Templates combine a localized reply count with user-provided sender/body text.
  "timeline.threadSummaryWithBody",
  "timeline.threadSummaryWithPreview",
  "timeline.threadSummaryWithSender"
]);

describe("Chinese and Korean catalogs (#1461)", () => {
  const translatedLocales = ["zh-Hans", "zh-Hant", "ko"] as const;
  const placeholders = (template: string) =>
    [...template.matchAll(MESSAGE_PLACEHOLDER_PATTERN)].map((match) => match[0]).sort();

  test.each(translatedLocales)("%s catalog has exactly the English message ids", (locale) => {
    expect(Object.keys(catalogs[locale]).sort()).toEqual(Object.keys(catalogs.en).sort());
    for (const id of Object.keys(catalogs.en) as MessageId[]) {
      expect(typeof catalogs[locale][id], id).toBe("string");
      expect(catalogs[locale][id].trim(), id).not.toBe("");
    }
  });

  test.each(translatedLocales)("%s catalog preserves every named placeholder", (locale) => {
    const mismatches = (Object.keys(catalogs.en) as MessageId[]).filter(
      (id) => placeholders(catalogs[locale][id]).join() !== placeholders(catalogs.en[id]).join()
    );
    expect(mismatches).toEqual([]);
  });

  test.each(translatedLocales)(
    "%s catalog translates every message id except the named allowlist",
    (locale) => {
      const identical = (Object.keys(catalogs.en) as MessageId[]).filter(
        (id) => catalogs[locale][id] === catalogs.en[id] && !translatedLocaleIdenticalAllowlist.has(id)
      );
      expect(identical).toEqual([]);
    }
  );

  test("Traditional Chinese is a Taiwan-vocabulary catalog, not a copy of Simplified Chinese", () => {
    const sameAsSimplified = (Object.keys(catalogs.en) as MessageId[]).filter(
      (id) =>
        catalogs["zh-Hant"][id] === catalogs["zh-Hans"][id] &&
        !translatedLocaleIdenticalAllowlist.has(id)
    );
    // Short CJK labels can legitimately coincide; whole-catalog identity cannot.
    expect(sameAsSimplified.length).toBeLessThan(Object.keys(catalogs.en).length / 10);
    expect(t("settings.appSettings", {}, "zh-Hant")).toContain("設定");
    expect(t("settings.appSettings", {}, "zh-Hans")).toContain("设置");
    expect(t("settings.accountSettings", {}, "zh-Hant")).toContain("帳號");
    expect(t("timeline.messagesTab", {}, "zh-Hant")).toContain("訊息");
  });

  test("language self-names read the same in every catalog", () => {
    for (const locale of Object.keys(catalogs) as Locale[]) {
      if (locale === "pseudo") continue;
      expect(catalogs[locale]["settings.languageEnglish"]).toBe("English");
      expect(catalogs[locale]["settings.languageJapanese"]).toBe("日本語");
      expect(catalogs[locale]["settings.languageSimplifiedChinese"]).toBe("简体中文");
      expect(catalogs[locale]["settings.languageTraditionalChinese"]).toBe("繁體中文");
      expect(catalogs[locale]["settings.languageKorean"]).toBe("한국어");
    }
  });

  test("interpolates caller-owned mixed-script values unchanged in each new locale", () => {
    const roomName = "研究室 🧪 العربية <한국어>";
    for (const locale of translatedLocales) {
      const rendered = t("composer.placeholder", { roomName }, locale);
      expect(rendered).toContain(roomName);
      expect(rendered).not.toContain("{roomName}");
      expect(rendered).not.toBe(t("composer.placeholder", { roomName }, "en"));
    }
  });

  test("presentation language tags match the Rust catalog ids", () => {
    expect(localeLanguageTag("zh-Hans")).toBe("zh-Hans");
    expect(localeLanguageTag("zh-Hant")).toBe("zh-Hant");
    expect(localeLanguageTag("ko")).toBe("ko");
    expect(localeLanguageTag("pseudo")).toBe("en");
  });
});

// Ids that the zh-Hans, zh-Hant, and ko catalogs keep identical to English:
// product/brand names, language self-names, protocol terms, and format-only
// templates. Everything else must be translated.
const translatedLocaleIdenticalAllowlist = new Set<MessageId>([
  // Brand names, protocol/format names, numeric or symbol-only templates,
  // language self-names, and the locale-independent download file name.
  "access.conditionSummarySeparator",
  "access.labelSeparator",
  "accountTabs.select",
  "app.title",
  "auth.flowOidc",
  "auth.matrixDesktop",
  "auth.matrixId",
  "auth.matrixIdPlaceholder",
  "settings.credentialStoreLinux",
  "settings.notificationDeliverySinkToPlaceholder",
  "settings.fontInter",
  "settings.languageEnglish",
  "settings.languageJapanese",
  "settings.languageKorean",
  "settings.languageSimplifiedChinese",
  "settings.languageTraditionalChinese",
  "settings.notificationDeliverySecretServerChan",
  "settings.twemojiColr",
  "timeline.downloadDefaultImageName",
  "timeline.mediaUploadProgress",
  "timeline.threadSummaryWithBody",
  "timeline.threadSummaryWithPreview",
  "timeline.threadSummaryWithSender",
  "upload.formatJpeg",
  "upload.formatPng",
  "upload.formatWebp",
  "upload.previewActualSize",
  "upload.resizeEighth",
  "upload.resizeHalf",
  "upload.resizeQuarter",
  "window.title"
]);

function lineNumberAt(sourceFile: ts.SourceFile, node: ts.Node): number {
  return sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
}
