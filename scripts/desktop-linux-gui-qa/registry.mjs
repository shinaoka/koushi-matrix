#!/usr/bin/env node
import { assertSdkSubmoduleSynced } from "../lib/sdk-submodule-status.mjs";
import { guiScenario,repoRoot } from "./options.mjs";
import { runLocalInvitesDmScenario,runLocalLoginScenario,runLocalLogoutReloginScenario,runSignedOutScenario } from "./scenarios/auth.mjs";
import { runLocalImageCompressionScenario,runLocalMediaScenario,runLocalMessageTypesScenario } from "./scenarios/media.mjs";
import { runLocalActivityScenario,runLocalAliasScenario,runLocalCjkScenario,runLocalComposerScenario,runLocalCreateRoomScenario,runLocalCreateSpaceScenario,runLocalExploreScenario,runLocalMessageActionsScenario,runLocalPinsScenario,runLocalReceiptReadersScenario,runLocalReplyScenario,runLocalRichFormattingScenario,runLocalRoomManagementScenario,runLocalRoomTagsScenario,runLocalScheduledSendScenario,runLocalSendScenario,runLocalSpacesNavScenario,runLocalTimelineNavigationScenario } from "./scenarios/rooms-timeline.mjs";
import { runLocalE2eeKeyManagementScenario,runLocalSettingsScenario } from "./scenarios/settings-security.mjs";

export const checks = [
  "scenario signed-out",
  "scenario local-login",
  "scenario local-send",
  "scenario local-create-room",
  "scenario local-create-space",
  "scenario local-logout-relogin",
  "scenario local-spaces-nav",
  "scenario local-invites-dm",
  "scenario local-reply",
  "scenario local-media",
  "scenario local-image-compression",
  "scenario local-room-tags",
  "scenario local-room-management",
  "scenario local-activity",
  "scenario local-explore",
  "scenario local-message-actions",
  "scenario local-receipt-readers",
  "scenario local-pins",
  "scenario local-message-types",
  "scenario local-composer",
  "scenario local-scheduled-send",
  "scenario local-timeline-navigation",
  "scenario local-rich-formatting",
  "scenario local-alias",
  "scenario local-cjk",
  "scenario local-settings",
  "scenario local-e2ee-key-management",
  "verify local-settings trust section",
  "verify local-e2ee-key-management tokens",
  "verify Xvfb virtual display",
  "verify tauri-driver and WebKitWebDriver",
  "verify debug Tauri build",
  "drive WebdriverIO session",
  "exercise real IPC and DOM smoke",
  "optional local homeserver login via FIFO",
  "clean process teardown"
];

export async function run() {
  assertSdkSubmoduleSynced({ repoRoot });

  if (guiScenario === "signed-out") {
    await runSignedOutScenario();
    return;
  }
  if (guiScenario === "local-login") {
    await runLocalLoginScenario();
    return;
  }
  if (guiScenario === "local-send") {
    await runLocalSendScenario();
    return;
  }
  if (guiScenario === "local-create-room") {
    await runLocalCreateRoomScenario();
    return;
  }
  if (guiScenario === "local-create-space") {
    await runLocalCreateSpaceScenario();
    return;
  }
  if (guiScenario === "local-logout-relogin") {
    await runLocalLogoutReloginScenario();
    return;
  }
  if (guiScenario === "local-spaces-nav") {
    await runLocalSpacesNavScenario();
    return;
  }
  if (guiScenario === "local-invites-dm") {
    await runLocalInvitesDmScenario();
    return;
  }
  if (guiScenario === "local-reply") {
    await runLocalReplyScenario();
    return;
  }
  if (guiScenario === "local-media") {
    await runLocalMediaScenario();
    return;
  }
  if (guiScenario === "local-image-compression") {
    await runLocalImageCompressionScenario();
    return;
  }
  if (guiScenario === "local-room-tags") {
    await runLocalRoomTagsScenario();
    return;
  }
  if (guiScenario === "local-room-management") {
    await runLocalRoomManagementScenario();
    return;
  }
  if (guiScenario === "local-activity") {
    await runLocalActivityScenario();
    return;
  }
  if (guiScenario === "local-explore") {
    await runLocalExploreScenario();
    return;
  }
  if (guiScenario === "local-message-actions") {
    await runLocalMessageActionsScenario();
    return;
  }
  if (guiScenario === "local-receipt-readers") {
    await runLocalReceiptReadersScenario();
    return;
  }
  if (guiScenario === "local-pins") {
    await runLocalPinsScenario();
    return;
  }
  if (guiScenario === "local-message-types") {
    await runLocalMessageTypesScenario();
    return;
  }
  if (guiScenario === "local-composer") {
    await runLocalComposerScenario();
    return;
  }
  if (guiScenario === "local-scheduled-send") {
    await runLocalScheduledSendScenario();
    return;
  }
  if (guiScenario === "local-timeline-navigation") {
    await runLocalTimelineNavigationScenario();
    return;
  }
  if (guiScenario === "local-rich-formatting") {
    await runLocalRichFormattingScenario();
    return;
  }
  if (guiScenario === "local-alias") {
    await runLocalAliasScenario();
    return;
  }
  if (guiScenario === "local-cjk") {
    await runLocalCjkScenario();
    return;
  }
  if (guiScenario === "local-settings") {
    await runLocalSettingsScenario();
    return;
  }
  if (guiScenario === "local-e2ee-key-management") {
    await runLocalE2eeKeyManagementScenario();
    return;
  }
  throw new Error(`unsupported --scenario: ${guiScenario}`);
}
