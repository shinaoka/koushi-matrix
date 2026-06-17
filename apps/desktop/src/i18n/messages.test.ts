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

  test("Japanese catalog localizes shipped message ids except the named allowlist", () => {
    const identicalMessageIds = (Object.keys(catalogs.en) as MessageId[]).filter(
      (id) => catalogs.ja[id] === catalogs.en[id] && !japaneseIdenticalMessageAllowlist.has(id)
    );

    expect(identicalMessageIds).toEqual([]);
  });

  test("Japanese catalog provides representative localized labels", () => {
    expect(t("composer.replying", {}, "ja")).toBe("返信中");
    expect(t("action.send", {}, "ja")).toBe("送信");
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
          if (text && text !== "matrix-desktop" && /[A-Za-z]/.test(text)) {
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
      ...contextMenuItems({ kind: "message", canManage: true, hasThread: true }).map(
        (item) => item.labelMessageId
      ),
      ...contextMenuItems({ kind: "room" }).map((item) => item.labelMessageId),
      ...contextMenuItems({ kind: "space" }).map((item) => item.labelMessageId),
      ...contextMenuItems({ kind: "account" }).map((item) => item.labelMessageId)
    ];

    expect(ids.every((id) => messageIds.has(id))).toBe(true);
  });
});

const japaneseIdenticalMessageAllowlist = new Set<MessageId>([
  "settings.fontInter",
  "settings.twemojiColr",
  "timeline.mediaUploadProgress"
]);

function lineNumberAt(sourceFile: ts.SourceFile, node: ts.Node): number {
  return sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
}
