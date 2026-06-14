export type Locale = "en" | "ja" | "pseudo";

export type MessageId =
  | "action.add"
  | "action.back"
  | "action.cancel"
  | "action.close"
  | "action.createRoom"
  | "action.createSpace"
  | "action.forward"
  | "action.history"
  | "action.more"
  | "action.restartSync"
  | "action.recover"
  | "action.recovering"
  | "action.send"
  | "action.sending"
  | "auth.checking"
  | "auth.checkLoginMethods"
  | "auth.connecting"
  | "auth.continue"
  | "auth.deviceName"
  | "auth.encryptionRecovery"
  | "auth.matrixAccount"
  | "auth.matrixDesktop"
  | "auth.noLoginMethods"
  | "auth.notChecked"
  | "auth.password"
  | "auth.recoveryKey"
  | "auth.recoverySecret"
  | "auth.securityPhrase"
  | "auth.sessionLocked"
  | "auth.signIn"
  | "auth.supportedRecoveryMethods"
  | "auth.usernameOrMatrixId"
  | "composer.bold"
  | "composer.code"
  | "composer.emoji"
  | "composer.italic"
  | "composer.link"
  | "composer.list"
  | "composer.mention"
  | "composer.messageComposer"
  | "composer.placeholder"
  | "composer.replying"
  | "composer.cancelReply"
  | "context.editMessage"
  | "context.openKeyboardSettings"
  | "context.openRoomInfo"
  | "context.openSpaceInfo"
  | "context.openThread"
  | "context.openUserSettings"
  | "context.redactMessage"
  | "context.searchInRoom"
  | "context.selectRoom"
  | "context.selectSpace"
  | "context.switchAccount"
  | "dialog.cancelCreate"
  | "dialog.createRoomTitle"
  | "dialog.createSpaceTitle"
  | "dialog.roomName"
  | "dialog.spaceName"
  | "dialog.submitCreateRoom"
  | "dialog.submitCreateSpace"
  | "panel.context"
  | "panel.keyboard"
  | "panel.focusedContext"
  | "panel.recovery"
  | "panel.roomInfo"
  | "panel.search"
  | "panel.spaceInfo"
  | "panel.thread"
  | "panel.userSettings"
  | "room.members"
  | "room.directMessage"
  | "room.dmList"
  | "room.exactVerifiedResults"
  | "room.files"
  | "room.globalDmList"
  | "room.noRoomSelected"
  | "room.noSpaces"
  | "room.notifications"
  | "room.people"
  | "room.roomInfo"
  | "room.roomScoped"
  | "room.roomSettings"
  | "room.searchIndex"
  | "room.spaces"
  | "room.subscribed"
  | "room.summary"
  | "room.tabs"
  | "room.threadToggle"
  | "room.timeline"
  | "room.type"
  | "room.unread"
  | "room.unreadCount"
  | "search.noExactMatches"
  | "search.matchAttachmentFileName"
  | "search.matchMessage"
  | "search.resultCountMany"
  | "search.resultCountOne"
  | "search.scopeAll"
  | "search.scopeDm"
  | "search.scopeRoom"
  | "search.scopeSpace"
  | "settings.accounts"
  | "settings.appearance"
  | "settings.accountSwitcher"
  | "settings.current"
  | "settings.device"
  | "settings.general"
  | "settings.homeserver"
  | "settings.keyboard"
  | "settings.keyboardDescription"
  | "settings.localStore"
  | "settings.localStoreLabel"
  | "settings.matrixAccount"
  | "settings.notRestored"
  | "settings.preferences"
  | "settings.searchIndex"
  | "settings.security"
  | "settings.securityPrivacy"
  | "settings.session"
  | "settings.sessionSecretLabel"
  | "settings.sessionSecret"
  | "settings.saving"
  | "settings.switch"
  | "settings.userId"
  | "settings.theme"
  | "settings.themeDark"
  | "settings.themeLight"
  | "settings.themeSystem"
  | "trust.acceptVerification"
  | "trust.closeVerification"
  | "trust.confirmSas"
  | "trust.continueIdentityReset"
  | "trust.crossSigning"
  | "trust.declineVerification"
  | "trust.deviceBlocked"
  | "trust.deviceCount"
  | "trust.deviceOrdinal"
  | "trust.deviceUnknown"
  | "trust.deviceUnverified"
  | "trust.deviceVerified"
  | "trust.devices"
  | "trust.enableKeyBackup"
  | "trust.encryption"
  | "trust.failureCancelled"
  | "trust.failureForbidden"
  | "trust.failureMismatch"
  | "trust.failureNetwork"
  | "trust.failureSdk"
  | "trust.failureTimeout"
  | "trust.identityReset"
  | "trust.identityResetAuthUnknown"
  | "trust.identityResetPassword"
  | "trust.keyBackup"
  | "trust.noDevices"
  | "trust.resetIdentity"
  | "trust.sasEmoji"
  | "trust.sasEmojiList"
  | "trust.setupCrossSigning"
  | "trust.statusAwaitingAuth"
  | "trust.statusBootstrapping"
  | "trust.statusConfirming"
  | "trust.statusDisabled"
  | "trust.statusEnabled"
  | "trust.statusEnabling"
  | "trust.statusFailed"
  | "trust.statusFailedReason"
  | "trust.statusIdle"
  | "trust.statusInProgress"
  | "trust.statusMissing"
  | "trust.statusNeedsAttention"
  | "trust.statusNotTrusted"
  | "trust.statusResetting"
  | "trust.statusRestoringBackup"
  | "trust.statusRestoringBackupOpen"
  | "trust.statusSasPresented"
  | "trust.statusTrusted"
  | "trust.statusUnknown"
  | "trust.statusVerificationAccepted"
  | "trust.statusVerificationRequested"
  | "trust.statusVerified"
  | "trust.verification"
  | "space.allRooms"
  | "space.childRooms"
  | "space.directMessages"
  | "space.home"
  | "space.invite"
  | "space.noUnread"
  | "space.preferences"
  | "space.roomMembership"
  | "space.spacePreferences"
  | "space.spaceSettings"
  | "space.summary"
  | "sync.failed"
  | "sync.failedWithReason"
  | "sync.reconnecting"
  | "sync.reconnectingWithReason"
  | "sync.running"
  | "sync.starting"
  | "sync.stopped"
  | "shortcut.activateButton"
  | "shortcut.cancelAutocomplete"
  | "shortcut.cancelReplyOrEdit"
  | "shortcut.categoryAccessibility"
  | "shortcut.categoryAutocomplete"
  | "shortcut.categoryComposer"
  | "shortcut.categoryNavigation"
  | "shortcut.categoryRoom"
  | "shortcut.categoryRoomList"
  | "shortcut.closeDialogOrMenu"
  | "shortcut.collapseRoomListSection"
  | "shortcut.composerSendShortcut"
  | "shortcut.enterSends"
  | "shortcut.expandRoomListSection"
  | "shortcut.filterRooms"
  | "shortcut.formatBold"
  | "shortcut.formatCode"
  | "shortcut.formatItalics"
  | "shortcut.formatLink"
  | "shortcut.goHome"
  | "shortcut.jumpToFirstMessage"
  | "shortcut.jumpToLatestMessage"
  | "shortcut.jumpToOldestUnread"
  | "shortcut.modEnterSends"
  | "shortcut.newLine"
  | "shortcut.nextAutocompleteSelection"
  | "shortcut.nextLandmark"
  | "shortcut.nextRoomInList"
  | "shortcut.nextVisitedRoomOrSpace"
  | "shortcut.noteCallsDeferred"
  | "shortcut.noteGoHomeAdapted"
  | "shortcut.noteUploadUiDeferred"
  | "shortcut.openUserSettings"
  | "shortcut.parityAdapted"
  | "shortcut.parityDeferred"
  | "shortcut.parityNotApplicable"
  | "shortcut.paritySame"
  | "shortcut.previousAutocompleteSelection"
  | "shortcut.previousLandmark"
  | "shortcut.previousRoomInList"
  | "shortcut.previousVisitedRoomOrSpace"
  | "shortcut.searchInRoom"
  | "shortcut.selectNextRoom"
  | "shortcut.selectNextUnreadRoom"
  | "shortcut.selectPreviousRoom"
  | "shortcut.selectPreviousUnreadRoom"
  | "shortcut.selectRoomInRoomList"
  | "shortcut.sendMessage"
  | "shortcut.shortcutKeys"
  | "shortcut.showKeyboardSettings"
  | "shortcut.scrollTimelineDown"
  | "shortcut.scrollTimelineUp"
  | "shortcut.toggleMicrophone"
  | "shortcut.toggleRightPanel"
  | "shortcut.toggleSpacePanel"
  | "shortcut.uploadFile"
  | "timeline.conversation"
  | "timeline.conversationStart"
  | "timeline.editedMessage"
  | "timeline.editMessage"
  | "timeline.editBody"
  | "timeline.loading"
  | "timeline.messagesTab"
  | "timeline.openingThread"
  | "timeline.addReaction"
  | "timeline.reactionSummary"
  | "timeline.reactionPicker"
  | "timeline.reactionOption"
  | "timeline.olderMessages"
  | "timeline.saveEdit"
  | "timeline.cancelEdit"
  | "timeline.redactMessage"
  | "timeline.redactedMessage"
  | "timeline.replyToMessage"
  | "timeline.threadComposer"
  | "timeline.threadPlaceholder"
  | "timeline.unsent"
  | "timeline.threadReplyCountOne"
  | "timeline.threadReplyCountMany"
  | "timeline.threadRoot"
  | "timeline.threadSummaryWithBody"
  | "timeline.threadSummaryWithPreview"
  | "timeline.threadSummaryWithSender"
  | "timeline.openThreadSummary"
  | "timeline.viewReplies"
  | "workspace.createSpace"
  | "workspace.home"
  | "workspace.invites"
  | "workspace.people"
  | "workspace.rooms"
  | "workspace.search"
  | "workspace.searchPlaceholder"
  | "workspace.searchScope"
  | "workspace.spaceInfoSettings"
  | "workspace.threads"
  | "workspace.userSettings"
  | "workspace.workspaces";

type MessageValues = Record<string, string | number>;
type Catalog = Record<MessageId, string>;
export type PseudoLocaleMode = "accented" | "bidi";
type ActivePseudoLocaleMode = PseudoLocaleMode | "none";

let activeLocale: Locale = "en";
let activePseudoLocale: ActivePseudoLocaleMode = "none";

export function setActiveLocaleProfile(
  locale: Locale,
  pseudoLocale: ActivePseudoLocaleMode = "none"
): void {
  activeLocale = locale;
  activePseudoLocale = pseudoLocale;
}

const pseudoAccentMap: Record<string, string> = {
  A: "Å",
  B: "ß",
  C: "Ç",
  D: "Ð",
  E: "É",
  F: "Ƒ",
  G: "Ĝ",
  H: "Ħ",
  I: "Î",
  J: "Ĵ",
  K: "Ķ",
  L: "Ŀ",
  M: "Ṁ",
  N: "Ñ",
  O: "Ø",
  P: "Þ",
  Q: "Ǫ",
  R: "Ŕ",
  S: "Š",
  T: "Ŧ",
  U: "Û",
  V: "Ṽ",
  W: "Ŵ",
  X: "Ẋ",
  Y: "Ý",
  Z: "Ž",
  a: "å",
  b: "ƀ",
  c: "ç",
  d: "ð",
  e: "é",
  f: "ƒ",
  g: "ĝ",
  h: "ħ",
  i: "î",
  j: "ĵ",
  k: "ķ",
  l: "ŀ",
  m: "ṁ",
  n: "ñ",
  o: "ø",
  p: "þ",
  q: "ǫ",
  r: "ŕ",
  s: "š",
  t: "ŧ",
  u: "û",
  v: "ṽ",
  w: "ŵ",
  x: "ẋ",
  y: "ý",
  z: "ž"
};

export function pseudoLocalize(input: string, mode: PseudoLocaleMode = "accented"): string {
  const chunks = input.split(/(\{[a-zA-Z0-9_]+\})/g);
  const expanded = chunks
    .map((chunk) => {
      if (/^\{[a-zA-Z0-9_]+\}$/.test(chunk)) {
        return chunk;
      }
      return Array.from(chunk)
        .map((char) => pseudoAccentMap[char] ?? char)
        .join("");
    })
    .join("");

  if (mode === "bidi") {
    return `[!! \u202e${expanded}\u202c !!]`;
  }

  return `[!! ${expanded} !!]`;
}

const en: Catalog = {
  "action.add": "Add",
  "action.back": "Back",
  "action.cancel": "Cancel",
  "action.close": "Close {title}",
  "action.createRoom": "Create room",
  "action.createSpace": "Create space",
  "action.forward": "Forward",
  "action.history": "History",
  "action.more": "More",
  "action.restartSync": "Restart sync",
  "action.recover": "Recover",
  "action.recovering": "Recovering",
  "action.send": "Send",
  "action.sending": "Sending",
  "auth.checking": "Checking",
  "auth.checkLoginMethods": "Check login methods",
  "auth.connecting": "Connecting",
  "auth.continue": "Continue",
  "auth.deviceName": "Device name",
  "auth.encryptionRecovery": "Encryption Recovery",
  "auth.matrixAccount": "Matrix account",
  "auth.matrixDesktop": "Matrix Desktop",
  "auth.noLoginMethods": "No login methods",
  "auth.notChecked": "Not checked",
  "auth.password": "Password",
  "auth.recoveryKey": "Recovery key",
  "auth.recoverySecret": "Recovery key or security phrase",
  "auth.securityPhrase": "Security phrase",
  "auth.sessionLocked": "Session locked",
  "auth.signIn": "Sign in",
  "auth.supportedRecoveryMethods": "Supported recovery methods",
  "auth.usernameOrMatrixId": "Username or Matrix ID",
  "composer.bold": "Bold",
  "composer.code": "Code",
  "composer.emoji": "Emoji",
  "composer.italic": "Italic",
  "composer.link": "Link",
  "composer.list": "List",
  "composer.mention": "Mention",
  "composer.messageComposer": "Message composer",
  "composer.placeholder": "Message {roomName}",
  "composer.replying": "Replying",
  "composer.cancelReply": "Cancel reply",
  "context.editMessage": "Edit",
  "context.openKeyboardSettings": "Keyboard shortcuts",
  "context.openRoomInfo": "Room info",
  "context.openSpaceInfo": "Space info",
  "context.openThread": "Reply in thread",
  "context.openUserSettings": "User settings",
  "context.redactMessage": "Redact",
  "context.searchInRoom": "Search in room",
  "context.selectRoom": "Open",
  "context.selectSpace": "Open Space",
  "context.switchAccount": "Switch account",
  "dialog.cancelCreate": "Cancel create",
  "dialog.createRoomTitle": "Create room",
  "dialog.createSpaceTitle": "Create space",
  "dialog.roomName": "Room name",
  "dialog.spaceName": "Space name",
  "dialog.submitCreateRoom": "Submit create room",
  "dialog.submitCreateSpace": "Submit create space",
  "panel.context": "Context panel",
  "panel.keyboard": "Keyboard",
  "panel.focusedContext": "Focused context",
  "panel.recovery": "Recovery",
  "panel.roomInfo": "Room info",
  "panel.search": "Search",
  "panel.spaceInfo": "Space info",
  "panel.thread": "Thread",
  "panel.userSettings": "User settings",
  "room.members": "Members",
  "room.directMessage": "Direct message",
  "room.dmList": "DM list",
  "room.exactVerifiedResults": "Exact verified results",
  "room.files": "Files",
  "room.globalDmList": "Global DM list",
  "room.noRoomSelected": "No room selected",
  "room.noSpaces": "No Spaces",
  "room.notifications": "Notifications",
  "room.people": "People",
  "room.roomInfo": "Room info",
  "room.roomScoped": "Room scoped",
  "room.roomSettings": "Room settings",
  "room.searchIndex": "Search index",
  "room.spaces": "Spaces",
  "room.subscribed": "Subscribed",
  "room.summary": "Room summary",
  "room.tabs": "Room tabs",
  "room.threadToggle": "Toggle thread",
  "room.timeline": "Timeline",
  "room.type": "Type",
  "room.unread": "Unread",
  "room.unreadCount": "{count} unread",
  "search.noExactMatches": "No exact matches",
  "search.matchAttachmentFileName": "attachment filename",
  "search.matchMessage": "message",
  "search.resultCountMany": "{count} results for \"{query}\"",
  "search.resultCountOne": "1 result for \"{query}\"",
  "search.scopeAll": "All",
  "search.scopeDm": "DM",
  "search.scopeRoom": "Room",
  "search.scopeSpace": "Space",
  "settings.accounts": "Accounts",
  "settings.appearance": "Appearance",
  "settings.accountSwitcher": "Account switcher",
  "settings.current": "Current",
  "settings.device": "Device",
  "settings.general": "General",
  "settings.homeserver": "Homeserver",
  "settings.keyboard": "Keyboard",
  "settings.keyboardDescription": "Element-compatible shortcuts for implemented desktop actions.",
  "settings.localStore": "Separate encrypted namespace",
  "settings.localStoreLabel": "Local store",
  "settings.matrixAccount": "Matrix account",
  "settings.notRestored": "Not restored",
  "settings.preferences": "Preferences",
  "settings.searchIndex": "Encrypted local index",
  "settings.security": "Security",
  "settings.securityPrivacy": "Security & Privacy",
  "settings.session": "Session",
  "settings.sessionSecretLabel": "Session secret",
  "settings.sessionSecret": "OS credential store",
  "settings.saving": "Saving",
  "settings.switch": "Switch",
  "settings.userId": "User ID",
  "settings.theme": "Theme",
  "settings.themeDark": "Dark",
  "settings.themeLight": "Light",
  "settings.themeSystem": "System",
  "trust.acceptVerification": "Accept",
  "trust.closeVerification": "Close",
  "trust.confirmSas": "Confirm",
  "trust.continueIdentityReset": "Continue",
  "trust.crossSigning": "Cross-signing",
  "trust.declineVerification": "Decline",
  "trust.deviceBlocked": "Blocked",
  "trust.deviceCount": "{count} devices",
  "trust.deviceOrdinal": "Device {index}",
  "trust.deviceUnknown": "Unknown",
  "trust.deviceUnverified": "Unverified",
  "trust.deviceVerified": "Verified",
  "trust.devices": "Devices",
  "trust.enableKeyBackup": "Enable",
  "trust.encryption": "Encryption",
  "trust.failureCancelled": "Cancelled",
  "trust.failureForbidden": "Forbidden",
  "trust.failureMismatch": "Mismatch",
  "trust.failureNetwork": "Network",
  "trust.failureSdk": "SDK",
  "trust.failureTimeout": "Timeout",
  "trust.identityReset": "Identity reset",
  "trust.identityResetAuthUnknown": "Authorization unavailable",
  "trust.identityResetPassword": "Password",
  "trust.keyBackup": "Key backup",
  "trust.noDevices": "No devices",
  "trust.resetIdentity": "Reset",
  "trust.sasEmoji": "SAS emoji {index}",
  "trust.sasEmojiList": "SAS emoji",
  "trust.setupCrossSigning": "Set up",
  "trust.statusAwaitingAuth": "Awaiting authorization",
  "trust.statusBootstrapping": "Setting up",
  "trust.statusConfirming": "Confirming",
  "trust.statusDisabled": "Disabled",
  "trust.statusEnabled": "Enabled",
  "trust.statusEnabling": "Enabling",
  "trust.statusFailed": "Failed",
  "trust.statusFailedReason": "Failed: {reason}",
  "trust.statusIdle": "Idle",
  "trust.statusInProgress": "In progress",
  "trust.statusMissing": "Missing",
  "trust.statusNeedsAttention": "Needs attention",
  "trust.statusNotTrusted": "Not trusted",
  "trust.statusResetting": "Resetting",
  "trust.statusRestoringBackup": "Restoring {restored}/{total}",
  "trust.statusRestoringBackupOpen": "Restoring {restored}",
  "trust.statusSasPresented": "Compare emoji",
  "trust.statusTrusted": "Trusted",
  "trust.statusUnknown": "Unknown",
  "trust.statusVerificationAccepted": "Accepted",
  "trust.statusVerificationRequested": "Request pending",
  "trust.statusVerified": "Verified",
  "trust.verification": "Device verification",
  "space.allRooms": "All rooms",
  "space.childRooms": "Child rooms",
  "space.directMessages": "Direct messages",
  "space.home": "Home",
  "space.invite": "Invite",
  "space.noUnread": "No unread",
  "space.preferences": "Preferences",
  "space.roomMembership": "Room membership",
  "space.spacePreferences": "Space preferences",
  "space.spaceSettings": "Space settings",
  "space.summary": "Space summary",
  "sync.failed": "Failed",
  "sync.failedWithReason": "Sync failed: {reason}",
  "sync.reconnecting": "Reconnecting",
  "sync.reconnectingWithReason": "Sync reconnecting: {reason}",
  "sync.running": "Running",
  "sync.starting": "Starting",
  "sync.stopped": "Stopped",
  "shortcut.activateButton": "Activate focused control",
  "shortcut.cancelAutocomplete": "Cancel autocomplete",
  "shortcut.cancelReplyOrEdit": "Cancel reply or edit",
  "shortcut.categoryAccessibility": "Accessibility",
  "shortcut.categoryAutocomplete": "Autocomplete",
  "shortcut.categoryComposer": "Composer",
  "shortcut.categoryNavigation": "Navigation",
  "shortcut.categoryRoom": "Room",
  "shortcut.categoryRoomList": "Room List",
  "shortcut.closeDialogOrMenu": "Close dialog or menu",
  "shortcut.collapseRoomListSection": "Collapse section",
  "shortcut.composerSendShortcut": "Composer send shortcut",
  "shortcut.enterSends": "Enter sends",
  "shortcut.expandRoomListSection": "Expand section",
  "shortcut.filterRooms": "Find rooms",
  "shortcut.formatBold": "Bold",
  "shortcut.formatCode": "Code",
  "shortcut.formatItalics": "Italics",
  "shortcut.formatLink": "Insert link",
  "shortcut.goHome": "Go home",
  "shortcut.jumpToFirstMessage": "Jump to first message",
  "shortcut.jumpToLatestMessage": "Jump to latest message",
  "shortcut.jumpToOldestUnread": "Jump to oldest unread",
  "shortcut.modEnterSends": "{shortcut} sends",
  "shortcut.newLine": "New line",
  "shortcut.nextAutocompleteSelection": "Next autocomplete option",
  "shortcut.nextLandmark": "Next landmark",
  "shortcut.nextRoomInList": "Next room in list",
  "shortcut.nextVisitedRoomOrSpace": "Forward",
  "shortcut.noteCallsDeferred": "Calls are out of scope for this milestone.",
  "shortcut.noteGoHomeAdapted": "macOS uses Ctrl+Shift+H in Element; this prototype keeps one cross-platform row.",
  "shortcut.noteUploadUiDeferred": "Upload UI is not implemented yet.",
  "shortcut.openUserSettings": "User settings",
  "shortcut.parityAdapted": "adapted",
  "shortcut.parityDeferred": "deferred",
  "shortcut.parityNotApplicable": "not applicable",
  "shortcut.paritySame": "same",
  "shortcut.previousAutocompleteSelection": "Previous autocomplete option",
  "shortcut.previousLandmark": "Previous landmark",
  "shortcut.previousRoomInList": "Previous room in list",
  "shortcut.previousVisitedRoomOrSpace": "Back",
  "shortcut.searchInRoom": "Search in room",
  "shortcut.selectNextRoom": "Next room",
  "shortcut.selectNextUnreadRoom": "Next unread room",
  "shortcut.selectPreviousRoom": "Previous room",
  "shortcut.selectPreviousUnreadRoom": "Previous unread room",
  "shortcut.selectRoomInRoomList": "Select room",
  "shortcut.sendMessage": "Send message",
  "shortcut.shortcutKeys": "{label} shortcut",
  "shortcut.showKeyboardSettings": "Keyboard settings",
  "shortcut.scrollTimelineDown": "Scroll timeline down",
  "shortcut.scrollTimelineUp": "Scroll timeline up",
  "shortcut.toggleMicrophone": "Toggle microphone in call",
  "shortcut.toggleRightPanel": "Toggle right panel",
  "shortcut.toggleSpacePanel": "Toggle space panel",
  "shortcut.uploadFile": "Upload file",
  "timeline.conversation": "Conversation timeline",
  "timeline.conversationStart": "Start of conversation",
  "timeline.editedMessage": "Edited",
  "timeline.editMessage": "Edit message",
  "timeline.editBody": "Edit message body",
  "timeline.loading": "Loading",
  "timeline.messagesTab": "Messages",
  "timeline.openingThread": "Opening thread",
  "timeline.addReaction": "Add reaction",
  "timeline.reactionSummary": "Reaction {key}, count {count}",
  "timeline.reactionPicker": "Choose reaction",
  "timeline.reactionOption": "React with {emoji}",
  "timeline.olderMessages": "Older messages",
  "timeline.saveEdit": "Save edit",
  "timeline.cancelEdit": "Cancel edit",
  "timeline.redactMessage": "Redact message",
  "timeline.redactedMessage": "Message redacted",
  "timeline.replyToMessage": "Reply to message",
  "timeline.threadComposer": "Thread composer",
  "timeline.threadPlaceholder": "Reply",
  "timeline.unsent": "Unsent",
  "timeline.threadReplyCountOne": "1 reply",
  "timeline.threadReplyCountMany": "{count} replies",
  "timeline.threadRoot": "Thread root {eventId}",
  "timeline.threadSummaryWithBody": "{count} · {preview}",
  "timeline.threadSummaryWithPreview": "{count} · {sender}: {preview}",
  "timeline.threadSummaryWithSender": "{count} · {sender}",
  "timeline.openThreadSummary": "Open thread, {summary}",
  "timeline.viewReplies": "View new replies · {count}",
  "workspace.createSpace": "Create space",
  "workspace.home": "Home",
  "workspace.invites": "Invites",
  "workspace.people": "People",
  "workspace.rooms": "Rooms",
  "workspace.search": "Search",
  "workspace.searchPlaceholder": "Search in {spaceName}",
  "workspace.searchScope": "Search scope",
  "workspace.spaceInfoSettings": "Space info and settings",
  "workspace.threads": "Threads",
  "workspace.userSettings": "User settings",
  "workspace.workspaces": "Workspaces"
};

const ja: Catalog = { ...en };

const pseudo: Catalog = Object.fromEntries(
  Object.entries(en).map(([id, value]) => [id, pseudoLocalize(value)])
) as Catalog;

export const catalogs: Record<Locale, Catalog> = { en, ja, pseudo };

export function t(id: MessageId, values: MessageValues = {}, locale?: Locale): string {
  const selectedLocale = locale ?? activeLocale;
  const pseudoMode = locale ? "accented" : activePseudoLocale;
  const template =
    selectedLocale === "pseudo" && pseudoMode === "bidi"
      ? pseudoLocalize(catalogs.en[id], "bidi")
      : catalogs[selectedLocale][id] ?? catalogs.en[id];
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (_, key: string) =>
    String(values[key] ?? `{${key}}`)
  );
}
