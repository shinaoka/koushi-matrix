import { sendReadMarkers, sendRoomMessage } from "../../lib/local-homeserver-qa.mjs";
import { parseQaTitle,safeTimestamp,timestamp } from "../evidence.mjs";
import { cleanupLocalGuiScenario,recordLocalGuiEvidence,startLocalGuiScenario,waitForAuthScreen,waitForComposerSendSettled,waitForLocalLoginReady,waitForLocalSendSuccess,writeLocalLoginPipe } from "../local-session.mjs";
import { timeoutMs } from "../options.mjs";
import { sleep } from "../runtime.mjs";
import { MESSAGE_COMPOSER_SELECTOR,clickLatestMessageRedactButtonByText,clickMenuItemByText,clickRoomMemberAliasClear,clickVisibleButtonByAriaLabelInElement,clickVisibleButtonByTextPrefix,clickVisibleMenuItemByText,clickWorkspaceButton,driveTimelineToBottom,elementCount,getRoomEvent,localDatetimeInputValue,openRoomContextMenu,scrollTimelineToTop,selectComposerText,selectRoomByName,setDatetimeLocalValue,timelineDateJumpDiagnostics,waitForActiveRoomName,waitForCjkVisualContract,waitForDocumentText,waitForElementAttribute,waitForElementCount,waitForElementCountGreaterThan,waitForLatestEventMessageRow,waitForLatestMessageActionButton,waitForMessageSourceDialog,waitForPinnedRegionCleared,waitForPinnedRegionVisible,waitForQaTitle,waitForReplyLanded,waitForRichFormattedTimeline,waitForRoomInSection,waitForRoomManagementTopic,waitForRoomMemberAlias,waitForRoomMemberRole,waitForEditableValue,waitForTimelineAwayFromBottom,waitForTimelineFocusedContextReady,waitForTimelineScrollable,waitForTimelineScrolledToBottom,waitForTimelineSenderLabel,waitForTimelineViewMounted,waitForWorkspaceActive,waitForWorkspaceButton } from "../webdriver.mjs";

export async function runLocalSendScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    const message = `Koushi GUI QA ${timestamp()}`;
    await composer.click();
    await composer.setValue(message);
    await session.browser.keys("Enter");
    await waitForLocalSendSuccess(session.browser, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_send=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalCreateRoomScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const baselineRooms = parseQaTitle(
      await session.browser.execute(() => document.title)
    ).rooms;

    const createButton = await session.browser.$('button[aria-label="Create room"]');
    await createButton.waitForDisplayed({ timeout: timeoutMs });
    await createButton.click();
    const nameInput = await session.browser.$('input[aria-label="Room name"]');
    await nameInput.waitForDisplayed({ timeout: timeoutMs });
    await nameInput.setValue(`QA Room ${timestamp()}`);
    const submit = await session.browser.$('button[aria-label="Submit create room"]');
    await submit.click();

    await waitForQaTitle(
      session.browser,
      (status) => status.rooms > baselineRooms,
      timeoutMs,
      "local GUI create room"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_create_room=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalCreateSpaceScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const baselineSpaces = parseQaTitle(
      await session.browser.execute(() => document.title)
    ).spaces;

    const createButton = await session.browser.$('button[aria-label="Create space"]');
    await createButton.waitForDisplayed({ timeout: timeoutMs });
    await createButton.click();
    const nameInput = await session.browser.$('input[aria-label="Space name"]');
    await nameInput.waitForDisplayed({ timeout: timeoutMs });
    await nameInput.setValue(`QA Space ${timestamp()}`);
    const submit = await session.browser.$('button[aria-label="Submit create space"]');
    await submit.click();

    await waitForQaTitle(
      session.browser,
      (status) => status.spaces > baselineSpaces,
      timeoutMs,
      "local GUI create space"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_create_space=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalSpacesNavScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const baselineSpaces = parseQaTitle(
      await session.browser.execute(() => document.title)
    ).spaces;
    const spaceName = `QA Nav Space ${safeTimestamp()}`;
    const createButton = await session.browser.$('button[aria-label="Create space"]');
    await createButton.waitForDisplayed({ timeout: timeoutMs });
    await createButton.click();
    const nameInput = await session.browser.$('input[aria-label="Space name"]');
    await nameInput.waitForDisplayed({ timeout: timeoutMs });
    await nameInput.setValue(spaceName);
    const submit = await session.browser.$('button[aria-label="Submit create space"]');
    await submit.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.spaces > baselineSpaces,
      timeoutMs,
      "local GUI spaces navigation create"
    );
    await waitForWorkspaceButton(session.browser, spaceName, timeoutMs, "created space");

    await clickWorkspaceButton(session.browser, "Home", timeoutMs, "local GUI spaces home");
    await waitForWorkspaceActive(session.browser, "Home", true, timeoutMs, "local GUI spaces home");
    console.log("gui_local_spaces_home=ok");

    await clickWorkspaceButton(session.browser, spaceName, timeoutMs, "local GUI spaces select");
    await waitForWorkspaceActive(
      session.browser,
      spaceName,
      true,
      timeoutMs,
      "local GUI spaces select"
    );
    console.log("gui_local_spaces_nav=ok");

    const spaceInfo = await session.browser.$('button[aria-label="Space info and settings"]');
    await spaceInfo.waitForDisplayed({ timeout: timeoutMs });
    await spaceInfo.click();
    await waitForQaTitle(
      session.browser,
      (status) => status.panel === "spaceInfo",
      timeoutMs,
      "local GUI spaces info panel"
    );
    await waitForDocumentText(
      session.browser,
      [spaceName],
      timeoutMs,
      "local GUI spaces info panel"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_spaces_info=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalReplyScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    // A reply needs a real, server-acked event to target. Send one first so a
    // timeline row with a reply affordance exists.
    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(`QA reply root ${timestamp()}`);
    await session.browser.keys("Enter");
    await waitForLocalSendSuccess(session.browser, timeoutMs);

    // The reply action sits in a hover-revealed `.message-actions` container
    // (opacity:0 until `.message:hover`/`:focus-within`), so move the pointer
    // over it before interacting. Then open reply mode and confirm the composer
    // surfaced the Rust-backed reply state (Cancel reply affordance).
    const replyButton = await session.browser.$('[aria-label="Reply to message"]');
    await replyButton.waitForExist({ timeout: timeoutMs });
    await replyButton.moveTo();
    await replyButton.waitForDisplayed({ timeout: timeoutMs });
    await replyButton.click();
    const cancelReply = await session.browser.$('[aria-label="Cancel reply"]');
    await cancelReply.waitForDisplayed({ timeout: timeoutMs });

    // Send the reply and wait for it to land (a new timeline row, or a
    // `data-reply="true"` row when the reply relation is surfaced).
    const baselineMessages = await session.browser.execute(
      () => document.querySelectorAll(".message").length
    );
    await composer.click();
    await composer.setValue(`QA reply body ${timestamp()}`);
    await session.browser.keys("Enter");
    await waitForReplyLanded(session.browser, baselineMessages, timeoutMs);
    await recordLocalGuiEvidence(session);
    console.log("gui_local_reply=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalRoomTagsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const roomName = "QA Seed Room";
    await waitForRoomInSection(
      session.browser,
      "rooms",
      roomName,
      true,
      timeoutMs,
      "local GUI room tag baseline"
    );
    await waitForRoomInSection(
      session.browser,
      "favourites",
      roomName,
      false,
      timeoutMs,
      "local GUI room tag baseline"
    );

    await openRoomContextMenu(session.browser, "rooms", roomName);
    await clickMenuItemByText(session.browser, "Add to Favourites", timeoutMs);
    await waitForRoomInSection(
      session.browser,
      "favourites",
      roomName,
      true,
      timeoutMs,
      "local GUI room tag set"
    );
    await waitForRoomInSection(
      session.browser,
      "rooms",
      roomName,
      false,
      timeoutMs,
      "local GUI room tag set"
    );
    console.log("gui_local_room_tag_set=ok");

    await openRoomContextMenu(session.browser, "favourites", roomName);
    await clickMenuItemByText(session.browser, "Remove from Favourites", timeoutMs);
    await waitForRoomInSection(
      session.browser,
      "rooms",
      roomName,
      true,
      timeoutMs,
      "local GUI room tag remove"
    );
    await waitForRoomInSection(
      session.browser,
      "favourites",
      roomName,
      false,
      timeoutMs,
      "local GUI room tag remove"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_room_tag_removed=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalRoomManagementScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const roomInfoButton = await session.browser.$('button[aria-label="Room info"]');
    await roomInfoButton.waitForDisplayed({ timeout: timeoutMs });
    await roomInfoButton.click();

    const topicInput = await session.browser.$('textarea[aria-label="Room topic"]');
    await topicInput.waitForDisplayed({ timeout: timeoutMs });
    await topicInput.setValue(session.roomManagementTopic);
    const saveTopicButton = await session.browser.$("//button[normalize-space()='Save topic']");
    await saveTopicButton.waitForDisplayed({ timeout: timeoutMs });
    await saveTopicButton.click();
    await waitForRoomManagementTopic(
      session.browser,
      session.roomManagementTopic,
      timeoutMs,
      "local GUI room management topic"
    );
    console.log("gui_local_room_topic=ok");

    await waitForElementCount(
      session.browser,
      ".room-member-row",
      1,
      timeoutMs,
      "local GUI room management member baseline"
    );
    const roleSelect = await session.browser.$('select[aria-label^="Member role for"]');
    await roleSelect.waitForDisplayed({ timeout: timeoutMs });
    await roleSelect.selectByAttribute("value", "50");
    await waitForRoomMemberRole(
      session.browser,
      "Moderator",
      "50",
      timeoutMs,
      "local GUI room management role"
    );
    console.log("gui_local_room_role=ok");

    const kickButton = await session.browser.$('.room-member-row button[data-action="kick"]');
    await kickButton.waitForDisplayed({ timeout: timeoutMs });
    await kickButton.click();
    await waitForElementCount(
      session.browser,
      ".room-member-row",
      0,
      timeoutMs,
      "local GUI room management kick"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_room_kick=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalActivityScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const activityButton = await session.browser.$('button[aria-label="Activity"]');
    await activityButton.waitForDisplayed({ timeout: timeoutMs });
    await activityButton.click();
    const activityMain = await session.browser.$('main[aria-labelledby="activity-title"]');
    await activityMain.waitForDisplayed({ timeout: timeoutMs });
    console.log("gui_local_activity_open=ok");

    const unreadTabSelector = "//button[@role='tab' and normalize-space()='Unread']";
    const unreadTab = await session.browser.$(unreadTabSelector);
    await unreadTab.waitForDisplayed({ timeout: timeoutMs });
    await unreadTab.click();
    await waitForElementAttribute(
      session.browser,
      unreadTabSelector,
      "aria-selected",
      "true",
      timeoutMs,
      "local GUI activity unread tab"
    );
    console.log("gui_local_activity_unread_tab=ok");

    const recentTabSelector = "//button[@role='tab' and normalize-space()='Recent']";
    const recentTab = await session.browser.$(recentTabSelector);
    await recentTab.waitForDisplayed({ timeout: timeoutMs });
    await recentTab.click();
    await waitForElementAttribute(
      session.browser,
      recentTabSelector,
      "aria-selected",
      "true",
      timeoutMs,
      "local GUI activity recent tab"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_activity_recent_tab=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalExploreScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const baselineRooms = parseQaTitle(await session.browser.execute(() => document.title)).rooms;
    const exploreButton = await session.browser.$('button[aria-label="Explore"]');
    await exploreButton.waitForDisplayed({ timeout: timeoutMs });
    await exploreButton.click();

    const searchInput = await session.browser.$('input[aria-label="Search term"]');
    await searchInput.waitForDisplayed({ timeout: timeoutMs });
    await searchInput.setValue(session.directoryRoomName);
    const searchButton = await session.browser.$('button[aria-label="Search"]');
    await searchButton.click();

    await waitForDocumentText(
      session.browser,
      [session.directoryRoomName],
      timeoutMs,
      "local GUI public directory query"
    );
    console.log("gui_local_explore_query=ok");

    const joinButton = await session.browser.$(
      `button[aria-label=${JSON.stringify(`Join ${session.directoryRoomName}`)}]`
    );
    await joinButton.waitForDisplayed({ timeout: timeoutMs });
    await joinButton.click();

    await waitForQaTitle(
      session.browser,
      (status) => status.rooms > baselineRooms,
      timeoutMs,
      "local GUI public directory join"
    );
    await waitForRoomInSection(
      session.browser,
      "rooms",
      session.directoryRoomName,
      true,
      timeoutMs,
      "local GUI public directory joined room"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_explore_join=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

async function waitForReceiptCount(browser, expectedCount, timeout, description) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeout) {
    const matched = await browser.execute((count) =>
      [...document.querySelectorAll('.message-receipts')].some((element) =>
        (element.getAttribute('aria-label') ?? '').startsWith(`Read by ${count}:`)
      ), expectedCount);
    if (matched) return;
    await sleep(250);
  }
  throw new Error(`${description} did not observe Read by ${expectedCount}`);
}

export async function runLocalReceiptReadersScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForDocumentText(
      session.browser,
      [session.readerSeedBody],
      timeoutMs,
      "local GUI receipt-reader seed"
    );
    // Publish one helper marker only after the previous marker is visible. This
    // is an observation barrier, not a timing delay: it proves that each
    // incremental sliding-sync receipt update is accumulated by Core.
    for (const [index, helper] of session.readerHelpers.entries()) {
      await sendReadMarkers(
        session.credentials.homeserver,
        helper.accessToken,
        session.seedRoomId,
        session.readerEventId
      );
      await waitForReceiptCount(
        session.browser,
        index + 1,
        timeoutMs,
        `local GUI receipt reader ${index + 1}`
      );
    }
    await waitForDocumentText(
      session.browser,
      ["+2"],
      timeoutMs,
      "local GUI compact receipt overflow"
    );
    const receiptAnchor = await session.browser.$(
      '.message-receipts[aria-label^="Read by 5"]'
    );
    await receiptAnchor.waitForDisplayed({ timeout: timeoutMs });
    await receiptAnchor.click();
    await waitForElementCount(
      session.browser,
      ".receipt-tooltip",
      1,
      timeoutMs,
      "local GUI receipt-reader popup"
    );
    await waitForDocumentText(
      session.browser,
      session.readerDisplayNames,
      timeoutMs,
      "local GUI receipt-reader rows"
    );
    console.log("gui_local_reader_subscribe=ok");

    await session.browser.keys("Escape");
    await waitForElementCount(
      session.browser,
      ".receipt-tooltip",
      0,
      timeoutMs,
      "local GUI receipt-reader close"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_reader_close=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalMessageActionsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForQaTitle(
      session.browser,
      (status) =>
        status.timeline_room === true &&
        status.timeline_subscribed === true,
      timeoutMs,
      "local GUI message actions timeline room"
    );
    await waitForTimelineViewMounted(session.browser, timeoutMs);
    await sleep(1000);
    const seedBaselineMessages = await elementCount(session.browser, ".message");
    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue("QA message action seed");
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI message actions seed");
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      seedBaselineMessages,
      timeoutMs,
      "local GUI message actions seed message render"
    );

    const actionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await actionButton.moveTo();
    await actionButton.waitForDisplayed({ timeout: timeoutMs });
    await actionButton.click();
    await clickVisibleMenuItemByText(session.browser, "View source", timeoutMs);
    await waitForMessageSourceDialog(session.browser, timeoutMs);
    console.log("gui_local_message_source=ok");

    const closeSource = await session.browser.$('button[aria-label="Close message source"]');
    await closeSource.waitForDisplayed({ timeout: timeoutMs });
    await closeSource.click();

    const baselineMessages = await elementCount(session.browser, ".message[data-event-id]");
    const forwardActionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await forwardActionButton.moveTo();
    await forwardActionButton.waitForDisplayed({ timeout: timeoutMs });
    await forwardActionButton.click();
    await clickVisibleMenuItemByText(session.browser, "Forward", timeoutMs);
    await clickVisibleMenuItemByText(session.browser, "QA Seed Room", timeoutMs);
    await waitForElementCountGreaterThan(
      session.browser,
      ".message[data-event-id]",
      baselineMessages,
      timeoutMs,
      "local GUI message forward"
    );
    console.log("gui_local_message_forward=ok");

    const redactedBaselineMessages = await elementCount(
      session.browser,
      '.message[data-redacted="true"]'
    );
    const hideRedactedBody = "QA hide redacted seed";
    const hideRedactedBaselineMessages = await elementCount(session.browser, ".message");
    const hideRedactedComposer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await hideRedactedComposer.waitForDisplayed({ timeout: timeoutMs });
    await hideRedactedComposer.click();
    await hideRedactedComposer.setValue(hideRedactedBody);
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(
      session.browser,
      timeoutMs,
      "local GUI hide redacted seed"
    );
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      hideRedactedBaselineMessages,
      timeoutMs,
      "local GUI hide redacted seed message render"
    );
    await clickLatestMessageRedactButtonByText(session.browser, hideRedactedBody, timeoutMs);
    await waitForElementCountGreaterThan(
      session.browser,
      '.message[data-redacted="true"]',
      redactedBaselineMessages,
      timeoutMs,
      "local GUI redacted message render"
    );
    await waitForDocumentText(
      session.browser,
      ["Message redacted"],
      timeoutMs,
      "local GUI redacted message placeholder"
    );

    const userSettings = await session.browser.$('button[aria-label="User settings"]');
    await userSettings.waitForDisplayed({ timeout: timeoutMs });
    await userSettings.click();
    const hideDeletedToggleSelector =
      '//button[@role="switch" and @aria-label="Hide deleted messages"]';
    const hideDeletedToggle = await session.browser.$(hideDeletedToggleSelector);
    await hideDeletedToggle.waitForDisplayed({ timeout: timeoutMs });
    await waitForElementAttribute(
      session.browser,
      hideDeletedToggleSelector,
      "aria-checked",
      "false",
      timeoutMs,
      "hide redacted setting before toggle"
    );
    await hideDeletedToggle.click();
    await waitForElementAttribute(
      session.browser,
      hideDeletedToggleSelector,
      "aria-checked",
      "true",
      timeoutMs,
      "hide redacted setting after toggle"
    );
    await waitForElementCount(
      session.browser,
      '.message[data-redacted="true"]',
      redactedBaselineMessages,
      timeoutMs,
      "local GUI hide redacted projection"
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_hide_redacted=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalPinsScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForQaTitle(
      session.browser,
      (status) => status.timeline_room === true && status.timeline_subscribed === true,
      timeoutMs,
      "local GUI pins timeline room"
    );
    await waitForTimelineViewMounted(session.browser, timeoutMs);

    const row = await waitForLatestEventMessageRow(
      session.browser,
      timeoutMs,
      "local GUI pin target"
    );
    await row.moveTo();
    await clickVisibleButtonByAriaLabelInElement(
      row,
      "Pin message",
      timeoutMs,
      "local GUI pin message"
    );
    await waitForPinnedRegionVisible(session.browser, timeoutMs, "local GUI pin set");
    console.log("gui_local_pin_set=ok");

    await row.moveTo();
    await clickVisibleButtonByAriaLabelInElement(
      row,
      "Unpin message",
      timeoutMs,
      "local GUI unpin message"
    );
    await waitForPinnedRegionCleared(session.browser, timeoutMs, "local GUI pin clear");

    await recordLocalGuiEvidence(session);
    console.log("gui_local_pin_removed=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalComposerScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue("@qa");

    const mentionOption = await session.browser.$('button[role="option"]');
    await mentionOption.waitForDisplayed({ timeout: timeoutMs });
    await mentionOption.click();
    await waitForElementCountGreaterThan(
      session.browser,
      ".composer-mention-pills .mention-pill",
      0,
      timeoutMs,
      "local GUI mention pill"
    );
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI mention send");
    console.log("gui_local_mention=ok");

    await composer.click();
    await composer.setValue("world");
    await selectComposerText(session.browser);
    const boldButton = await session.browser.$('button[aria-label="Bold"]');
    await boldButton.waitForDisplayed({ timeout: timeoutMs });
    await boldButton.click();
    await waitForEditableValue(
      session.browser,
      MESSAGE_COMPOSER_SELECTOR,
      "**world**",
      timeoutMs,
      "local GUI bold markdown"
    );
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI markdown send");
    console.log("gui_local_markdown=ok");

    await composer.click();
    await composer.setValue("/me waves");
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI slash send");
    await recordLocalGuiEvidence(session);
    console.log("gui_local_slash=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalScheduledSendScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);

    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(`QA scheduled body ${safeTimestamp()}`);

    const sendLater = await session.browser.$('button[aria-label="Send later"]');
    await sendLater.waitForDisplayed({ timeout: timeoutMs });
    await sendLater.click();

    const scheduleInput = await session.browser.$('input[aria-label="Scheduled send time"]');
    await scheduleInput.waitForDisplayed({ timeout: timeoutMs });
    const scheduledValue = await localDatetimeInputValue(
      session.browser,
      Date.now() + 24 * 60 * 60_000
    );
    await setDatetimeLocalValue(session.browser, scheduledValue, "Scheduled send time");
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Schedule send",
      timeoutMs,
      "local GUI scheduled send create"
    );
    await waitForDocumentText(
      session.browser,
      ["Scheduled messages", "Local fallback"],
      timeoutMs,
      "local GUI scheduled send create"
    );
    await waitForEditableValue(
      session.browser,
      MESSAGE_COMPOSER_SELECTOR,
      "",
      timeoutMs,
      "local GUI scheduled send draft clear"
    );
    console.log("gui_local_scheduled_create=ok");

    const editButton = await session.browser.$('button[aria-label="Edit scheduled send"]');
    await editButton.waitForDisplayed({ timeout: timeoutMs });
    await editButton.click();
    const editedValue = await localDatetimeInputValue(
      session.browser,
      Date.now() + 48 * 60 * 60_000
    );
    await setDatetimeLocalValue(session.browser, editedValue, "Scheduled send time");
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Save scheduled send",
      timeoutMs,
      "local GUI scheduled send reschedule"
    );
    await waitForElementCount(
      session.browser,
      'section[aria-label="Scheduled messages"]',
      1,
      timeoutMs,
      "local GUI scheduled send reschedule"
    );
    console.log("gui_local_scheduled_reschedule=ok");

    const cancelButton = await session.browser.$('button[aria-label="Cancel scheduled send"]');
    await cancelButton.waitForDisplayed({ timeout: timeoutMs });
    await cancelButton.click();
    await waitForElementCount(
      session.browser,
      'section[aria-label="Scheduled messages"]',
      0,
      timeoutMs,
      "local GUI scheduled send cancel"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_scheduled_cancel=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalTimelineNavigationScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForTimelineViewMounted(session.browser, timeoutMs);
    await waitForTimelineScrollable(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation seed"
    );

    await driveTimelineToBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation initial bottom"
    );
    const baselineMessages = await elementCount(session.browser, ".message");
    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(`QA timeline navigation baseline ${safeTimestamp()}`);
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation baseline"
    );
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      baselineMessages,
      timeoutMs,
      "local GUI timeline navigation baseline render"
    );
    await driveTimelineToBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation baseline bottom"
    );

    await scrollTimelineToTop(session.browser);
    await waitForTimelineAwayFromBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation top viewport"
    );
    const beforeUnreadMessages = await elementCount(session.browser, ".message");
    let dateJumpEventId = null;
    for (let index = 0; index < 3; index += 1) {
      const response = await sendRoomMessage(
        session.credentials.homeserver,
        session.helperAccessToken,
        session.seedRoomId,
        `QA timeline navigation unread ${index} ${safeTimestamp()}`,
        `qa-timeline-nav-${index}-${safeTimestamp()}`
      );
      dateJumpEventId = response.event_id ?? dateJumpEventId;
    }
    await waitForElementCountGreaterThan(
      session.browser,
      ".message",
      beforeUnreadMessages,
      timeoutMs,
      "local GUI timeline navigation unread render"
    );

    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Jump to first unread",
      timeoutMs,
      "local GUI timeline navigation first unread"
    );
    await waitForDocumentText(
      session.browser,
      ["Unread messages"],
      timeoutMs,
      "local GUI timeline navigation unread divider"
    );
    console.log("gui_local_timeline_unread_jump=ok");

    await scrollTimelineToTop(session.browser);
    await waitForTimelineAwayFromBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation bottom setup viewport"
    );
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Jump to bottom",
      timeoutMs,
      "local GUI timeline navigation bottom"
    );
    await waitForTimelineScrolledToBottom(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation bottom"
    );
    console.log("gui_local_timeline_bottom_jump=ok");

    if (!dateJumpEventId) {
      throw new Error("local GUI timeline navigation date setup did not capture an event id");
    }
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Jump to date",
      timeoutMs,
      "local GUI timeline navigation date dialog"
    );
    const dateInput = await session.browser.$('input[aria-label="Jump to date"]');
    await dateInput.waitForDisplayed({ timeout: timeoutMs });
    const dateJumpEvent = await getRoomEvent(
      session.credentials.homeserver,
      session.helperAccessToken,
      session.seedRoomId,
      dateJumpEventId
    );
    const localDateValue = await localDatetimeInputValue(
      session.browser,
      dateJumpEvent.origin_server_ts
    );
    await setDatetimeLocalValue(session.browser, localDateValue);
    const dateInputDiagnostics = await timelineDateJumpDiagnostics(
      session.browser,
      localDateValue
    );
    if (
      !dateInputDiagnostics.inputExists ||
      !dateInputDiagnostics.valuePresent ||
      !dateInputDiagnostics.valueMatchesExpected ||
      !dateInputDiagnostics.valid
    ) {
      throw new Error(
        `local GUI timeline navigation date input did not accept the synthetic value. Diagnostics: ${JSON.stringify(
          dateInputDiagnostics
        )}`
      );
    }
    await clickVisibleButtonByTextPrefix(
      session.browser,
      "Open date in timeline",
      timeoutMs,
      "local GUI timeline navigation date submit"
    );
    await waitForTimelineFocusedContextReady(
      session.browser,
      timeoutMs,
      "local GUI timeline navigation focused context title"
    );
    await waitForDocumentText(
      session.browser,
      ["Focused context"],
      timeoutMs,
      "local GUI timeline navigation date jump"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_timeline_date_jump=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalAliasScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);
    await waitForTimelineViewMounted(session.browser, timeoutMs);
    await waitForTimelineSenderLabel(
      session.browser,
      session.aliasMemberDisplayName,
      timeoutMs,
      "local GUI alias timeline original label"
    );

    const actionButton = await waitForLatestMessageActionButton(session.browser, timeoutMs);
    await actionButton.moveTo();
    await actionButton.waitForDisplayed({ timeout: timeoutMs });
    await actionButton.click();
    await clickVisibleMenuItemByText(
      session.browser,
      `Set alias for ${session.aliasMemberDisplayName}`,
      timeoutMs
    );
    const aliasInput = await session.browser.$('input[aria-label="Alias"]');
    await aliasInput.waitForDisplayed({ timeout: timeoutMs });
    await aliasInput.setValue(session.aliasLocalDisplayName);
    const saveAliasButton = await session.browser.$("//button[normalize-space()='Save alias']");
    await saveAliasButton.waitForDisplayed({ timeout: timeoutMs });
    await saveAliasButton.click();

    await waitForTimelineSenderLabel(
      session.browser,
      session.aliasLocalDisplayName,
      timeoutMs,
      "local GUI alias timeline set"
    );
    const roomInfoButton = await session.browser.$('button[aria-label="Room info"]');
    await roomInfoButton.waitForDisplayed({ timeout: timeoutMs });
    await roomInfoButton.click();
    await waitForRoomMemberAlias(
      session.browser,
      session.aliasLocalDisplayName,
      session.aliasMemberDisplayName,
      timeoutMs,
      "local GUI alias member set"
    );
    console.log("gui_local_alias_set=ok");

    await clickRoomMemberAliasClear(session.browser, session.aliasLocalDisplayName, timeoutMs);
    await waitForTimelineSenderLabel(
      session.browser,
      session.aliasMemberDisplayName,
      timeoutMs,
      "local GUI alias timeline clear"
    );
    await waitForRoomMemberAlias(
      session.browser,
      session.aliasMemberDisplayName,
      null,
      timeoutMs,
      "local GUI alias member clear"
    );
    await recordLocalGuiEvidence(session);
    console.log("gui_local_alias_clear=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalRichFormattingScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await selectRoomByName(session.browser, "QA Seed Room", timeoutMs);
    await waitForActiveRoomName(session.browser, "QA Seed Room", timeoutMs);

    await waitForRichFormattedTimeline(session.browser, session.richFormatted, "pre-wrap", timeoutMs);

    const userSettings = await session.browser.$('button[aria-label="User settings"]');
    await userSettings.waitForDisplayed({ timeout: timeoutMs });
    await userSettings.click();
    const wrapToggleSelector =
      '//button[@role="switch" and @aria-label="Wrap long lines in code blocks"]';
    const wrapToggle = await session.browser.$(wrapToggleSelector);
    await wrapToggle.waitForDisplayed({ timeout: timeoutMs });
    await waitForElementAttribute(
      session.browser,
      wrapToggleSelector,
      "aria-checked",
      "true",
      timeoutMs,
      "code block wrap setting before toggle"
    );
    await wrapToggle.click();
    await waitForElementAttribute(
      session.browser,
      wrapToggleSelector,
      "aria-checked",
      "false",
      timeoutMs,
      "code block wrap setting after toggle"
    );
    await waitForRichFormattedTimeline(session.browser, session.richFormatted, "pre", timeoutMs);

    await recordLocalGuiEvidence(session);
    console.log("gui_local_rich_formatting=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}

export async function runLocalCjkScenario() {
  const session = await startLocalGuiScenario();
  try {
    await waitForAuthScreen(session.browser, timeoutMs);
    await writeLocalLoginPipe(session.qaLoginPipePath, session.credentials);
    await waitForLocalLoginReady(session, timeoutMs);
    await waitForDocumentText(
      session.browser,
      [session.cjkRoomName],
      timeoutMs,
      "local GUI CJK room name"
    );

    const composer = await session.browser.$(MESSAGE_COMPOSER_SELECTOR);
    await composer.waitForDisplayed({ timeout: timeoutMs });
    await composer.click();
    await composer.setValue(session.cjkMessageBody);
    await session.browser.keys("Enter");
    await waitForComposerSendSettled(session.browser, timeoutMs, "local GUI CJK send");
    await waitForDocumentText(
      session.browser,
      [session.cjkMessageBody],
      timeoutMs,
      "local GUI CJK message render"
    );
    await waitForCjkVisualContract(
      session.browser,
      {
        roomName: session.cjkRoomName,
        messageBody: session.cjkMessageBody
      },
      timeoutMs
    );

    await recordLocalGuiEvidence(session);
    console.log("gui_local_cjk=ok");
  } finally {
    await cleanupLocalGuiScenario(session);
  }
}
