import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";

const apiSource = readFileSync(new URL("./desktopApi.ts", import.meta.url), "utf8");
const planSource = readFileSync(
  new URL("../../../../docs/superpowers/plans/2026-09-01-issue759-ordered-state-transport.md", import.meta.url),
  "utf8"
);

function apiMethodBlocks(): Array<{ name: string; source: string }> {
  const body = apiSource.slice(
    apiSource.indexOf("export interface DesktopApi"),
    apiSource.indexOf("export interface ComposerDraftAccountOwner")
  );
  const starts = [...body.matchAll(/^  (\w+)(?:\(|:)/gm)];
  return starts.map((match, index) => ({
    name: match[1],
    source: body.slice(match.index, starts[index + 1]?.index ?? body.length)
  }));
}

function apiMethods(): string[] {
  return apiMethodBlocks().map(({ name }) => name).sort();
}

function plannedMethods(): string[] {
  const table = planSource.slice(
    planSource.indexOf("### Exhaustive DesktopApi migration map"),
    planSource.indexOf("### Browser fake and tests")
  );
  return table
    .split("\n")
    .filter(
      (line) =>
        line.startsWith("| ") &&
        !line.startsWith("| Category") &&
        !line.startsWith("| removed renderer ACK")
    )
    .flatMap((line) => [...(line.split("|")[2] ?? "").matchAll(/`(\w+)`/g)])
    .map((match) => match[1])
    .sort();
}

describe("DesktopApi command contract", () => {
  test("classifies every method exactly once in the approved migration map", () => {
    const planned = plannedMethods();
    const removed = new Set([
      "setRoomListProjection",
      "reshareRoomKey",
      "forceNewOutboundSession",
      "shareIndex0RoomKey",
      "resendIndex0RoomKey"
    ]);
    const current = planned
      .filter((method) => !removed.has(method))
      // Commands added after the #759 map was written. The plan is a historical
      // record of that migration, so later commands are listed here instead of
      // being back-dated into it.
      .concat(
        "importLegacySettings",
        "updateNavigationPreference",
        "loadSpaceChildren",
        "dismissEventNavigationFailure",
        "historyExportTimeZone",
        "exportHistory",
        "stopHistoryExport",
        "retryHistoryExport",
        "openNotificationEvent",
        "loadAccountNotifications",
        "setNotificationCategory",
        "setAccountPushEnabled",
        "requestNotificationEmailToken",
        "resendNotificationEmailToken",
        "confirmNotificationEmail",
        "submitNotificationEmailUia",
        "cancelNotificationEmail",
        "enableEmailNotifications",
        "disableEmailNotifications"
      )
      .sort();
    expect(new Set(current).size).toBe(current.length);
    expect(current).toEqual(apiMethods());
  });

  test("keeps deleted renderer acknowledgements out of the interface", () => {
    expect(apiMethods()).not.toContain("acknowledgeTimelineProjection");
    expect(apiMethods()).not.toContain("acknowledgeTimelineBatchRendered");
  });

  test("allows full snapshots only for initial and explicit resync reads", () => {
    const snapshotReturns = apiMethodBlocks()
      .filter(({ source }) => source.includes("Promise<DesktopSnapshot>"))
      .map(({ name }) => name);
    expect(snapshotReturns).toEqual(["getSnapshot", "settlementSnapshot", "resyncSnapshot"]);
  });
});
