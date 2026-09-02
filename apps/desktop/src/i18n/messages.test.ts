import { readdirSync, readFileSync } from "node:fs";
import * as ts from "typescript";
import { describe, expect, test } from "vitest";
import { contextMenuItems } from "../domain/contextMenus";
import { elementShortcutParity, keyboardShortcutGroups } from "../domain/shortcuts";
import {
  catalogs,
  pseudoLocalize,
  setActiveLocaleProfile,
  t,
  type Locale,
  type MessageId
} from "./messages";

describe("i18n message catalog", () => {
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
      "gate.secureBackupChooseDestination",
      "gate.secureBackupDestinationSelected",
      "gate.secureBackupDestinationNotSelected",
      "gate.secureBackupDestinationSelectionFailed",
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

  test("Japanese catalog localizes shipped message ids except the named allowlist", () => {
    const identicalMessageIds = (Object.keys(catalogs.en) as MessageId[]).filter(
      (id) => catalogs.ja[id] === catalogs.en[id] && !japaneseIdenticalMessageAllowlist.has(id)
    );

    expect(identicalMessageIds).toEqual([]);
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
      "Thread notifications · 3"
    );
    expect(t("timeline.threadNotificationCount", { count: 3 }, "ja")).toBe(
      "スレッド通知 · 3"
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
          ts.isStringLiteral(node.initializer)
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
      ...contextMenuItems({ kind: "account" }).map((item) => item.labelMessageId)
    ];

    expect(ids.every((id) => messageIds.has(id))).toBe(true);
  });
});

const japaneseIdenticalMessageAllowlist = new Set<MessageId>([
  "auth.failureForbidden",
  // Scale fractions and image-format names are not prose: "1/2" and "JPEG"
  // read the same in both catalogs, and translating them would be wrong.
  "upload.resizeHalf",
  "upload.resizeQuarter",
  "upload.resizeEighth",
  "upload.previewActualSize",
  "upload.formatWebp",
  "upload.formatJpeg",
  "upload.formatPng",
  "auth.failureNetwork",
  "auth.failureSdk",
  "auth.failureTimeout",
  "auth.failureUnsupported",
  "auth.flowOidc",
  "auth.flowPassword",
  "auth.flowSso",
  "auth.flowToken",
  "auth.flowUnknown",
  "roomList.filterPeople",
  "space.directMessages",
  "settings.fontInter",
  "settings.twemojiColr",
  "timeline.mediaUploadProgress",
  "workspace.people"
]);

function lineNumberAt(sourceFile: ts.SourceFile, node: ts.Node): number {
  return sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
}
