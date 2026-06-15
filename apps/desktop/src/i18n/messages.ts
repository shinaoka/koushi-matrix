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
  | "composer.attachFile"
  | "composer.attachFileInput"
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
  | "dialog.invitePeopleTitle"
  | "dialog.matrixUserId"
  | "dialog.newDmTitle"
  | "dialog.sendInvite"
  | "dialog.roomName"
  | "dialog.spaceName"
  | "dialog.startDm"
  | "dialog.submitCreateRoom"
  | "dialog.submitCreateSpace"
  | "invite.accept"
  | "invite.decline"
  | "invite.fromInviter"
  | "invite.noPending"
  | "invite.noTopic"
  | "invite.pendingInvites"
  | "invite.preview"
  | "invite.summary"
  | "invite.tabs"
  | "invite.topic"
  | "invite.unknownInviter"
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
  | "room.invitePeople"
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
  | "settings.emojiFont"
  | "settings.fontInter"
  | "settings.fontSystem"
  | "settings.general"
  | "settings.homeserver"
  | "settings.keyboard"
  | "settings.keyboardDescription"
  | "settings.localStore"
  | "settings.localStoreLabel"
  | "settings.matrixAccount"
  | "settings.notRestored"
  | "settings.preferences"
  | "settings.profile"
  | "settings.profileAvatar"
  | "settings.profileDisplayName"
  | "settings.profileDisplayNamePlaceholder"
  | "settings.profileSavingAvatar"
  | "settings.profileSavingDisplayName"
  | "settings.profileUpdate"
  | "settings.profileUploadAvatar"
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
  | "settings.twemojiColr"
  | "settings.typography"
  | "settings.uiFont"
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
  | "timeline.presenceAway"
  | "timeline.presenceOffline"
  | "timeline.presenceOnline"
  | "timeline.addReaction"
  | "timeline.reactionSummary"
  | "timeline.reactionPicker"
  | "timeline.reactionOption"
  | "timeline.readBy"
  | "timeline.readMarker"
  | "timeline.olderMessages"
  | "timeline.saveEdit"
  | "timeline.cancelEdit"
  | "timeline.redactMessage"
  | "timeline.redactedMessage"
  | "timeline.replyToMessage"
  | "timeline.threadComposer"
  | "timeline.threadPlaceholder"
  | "timeline.unsent"
  | "timeline.sending"
  | "timeline.notSent"
  | "timeline.cancelledSend"
  | "timeline.resendSend"
  | "timeline.deleteSend"
  | "timeline.cancelSend"
  | "timeline.unsentBar"
  | "timeline.resendAll"
  | "timeline.cancelAll"
  | "timeline.downloadMedia"
  | "timeline.encryptedMedia"
  | "timeline.mediaUploadProgress"
  | "timeline.threadReplyCountOne"
  | "timeline.threadReplyCountMany"
  | "timeline.threadRoot"
  | "timeline.threadSummaryWithBody"
  | "timeline.threadSummaryWithPreview"
  | "timeline.threadSummaryWithSender"
  | "timeline.typingMany"
  | "timeline.typingOne"
  | "timeline.openThreadSummary"
  | "timeline.viewReplies"
  | "workspace.createSpace"
  | "workspace.home"
  | "workspace.invites"
  | "workspace.newDm"
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
  "composer.attachFile": "Attach file",
  "composer.attachFileInput": "Attach file input",
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
  "dialog.invitePeopleTitle": "Invite people to {name}",
  "dialog.matrixUserId": "Matrix user ID",
  "dialog.newDmTitle": "New DM",
  "dialog.sendInvite": "Send invite",
  "dialog.roomName": "Room name",
  "dialog.spaceName": "Space name",
  "dialog.startDm": "Start DM",
  "dialog.submitCreateRoom": "Submit create room",
  "dialog.submitCreateSpace": "Submit create space",
  "invite.accept": "Accept invite",
  "invite.decline": "Decline invite",
  "invite.fromInviter": "From {inviter}",
  "invite.noPending": "No pending invites",
  "invite.noTopic": "No topic",
  "invite.pendingInvites": "Pending invites",
  "invite.preview": "Invite preview",
  "invite.summary": "Invite summary",
  "invite.tabs": "Invite views",
  "invite.topic": "Topic",
  "invite.unknownInviter": "Unknown inviter",
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
  "room.invitePeople": "Invite people",
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
  "settings.emojiFont": "Emoji font",
  "settings.fontInter": "Inter",
  "settings.fontSystem": "System",
  "settings.general": "General",
  "settings.homeserver": "Homeserver",
  "settings.keyboard": "Keyboard",
  "settings.keyboardDescription": "Element-compatible shortcuts for implemented desktop actions.",
  "settings.localStore": "Separate encrypted namespace",
  "settings.localStoreLabel": "Local store",
  "settings.matrixAccount": "Matrix account",
  "settings.notRestored": "Not restored",
  "settings.preferences": "Preferences",
  "settings.profile": "Profile",
  "settings.profileAvatar": "Avatar",
  "settings.profileDisplayName": "Display name",
  "settings.profileDisplayNamePlaceholder": "Display name",
  "settings.profileSavingAvatar": "Uploading",
  "settings.profileSavingDisplayName": "Saving",
  "settings.profileUpdate": "Update",
  "settings.profileUploadAvatar": "Upload",
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
  "settings.twemojiColr": "Twemoji COLR",
  "settings.typography": "Typography",
  "settings.uiFont": "UI font",
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
  "timeline.presenceAway": "Away",
  "timeline.presenceOffline": "Offline",
  "timeline.presenceOnline": "Online",
  "timeline.addReaction": "Add reaction",
  "timeline.reactionSummary": "Reaction {key}, count {count}",
  "timeline.reactionPicker": "Choose reaction",
  "timeline.reactionOption": "React with {emoji}",
  "timeline.readBy": "Read by {count}",
  "timeline.readMarker": "Read up to here",
  "timeline.olderMessages": "Older messages",
  "timeline.saveEdit": "Save edit",
  "timeline.cancelEdit": "Cancel edit",
  "timeline.redactMessage": "Redact message",
  "timeline.redactedMessage": "Message redacted",
  "timeline.replyToMessage": "Reply to message",
  "timeline.threadComposer": "Thread composer",
  "timeline.threadPlaceholder": "Reply",
  "timeline.unsent": "Unsent",
  "timeline.sending": "Sending",
  "timeline.notSent": "Not sent",
  "timeline.cancelledSend": "Cancelled",
  "timeline.resendSend": "Resend",
  "timeline.deleteSend": "Delete",
  "timeline.cancelSend": "Cancel send",
  "timeline.unsentBar": "Some messages haven't been sent",
  "timeline.resendAll": "Resend all",
  "timeline.cancelAll": "Cancel all",
  "timeline.downloadMedia": "Download {filename}",
  "timeline.encryptedMedia": "Encrypted",
  "timeline.mediaUploadProgress": "{percent}%",
  "timeline.threadReplyCountOne": "1 reply",
  "timeline.threadReplyCountMany": "{count} replies",
  "timeline.threadRoot": "Thread root {eventId}",
  "timeline.threadSummaryWithBody": "{count} · {preview}",
  "timeline.threadSummaryWithPreview": "{count} · {sender}: {preview}",
  "timeline.threadSummaryWithSender": "{count} · {sender}",
  "timeline.typingMany": "{count} people are typing",
  "timeline.typingOne": "{user} is typing",
  "timeline.openThreadSummary": "Open thread, {summary}",
  "timeline.viewReplies": "View new replies · {count}",
  "workspace.createSpace": "Create space",
  "workspace.home": "Home",
  "workspace.invites": "Invites",
  "workspace.newDm": "New DM",
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

const ja: Catalog = {
  ...en,
  "action.add": "追加",
  "action.back": "戻る",
  "action.cancel": "キャンセル",
  "action.close": "{title}を閉じる",
  "action.createRoom": "ルームを作成",
  "action.createSpace": "スペースを作成",
  "action.forward": "進む",
  "action.history": "履歴",
  "action.more": "その他",
  "action.restartSync": "同期を再開",
  "action.recover": "復旧",
  "action.recovering": "復旧中",
  "action.send": "送信",
  "action.sending": "送信中",
  "auth.checking": "確認中",
  "auth.checkLoginMethods": "ログイン方法を確認",
  "auth.connecting": "接続中",
  "auth.continue": "続行",
  "auth.deviceName": "デバイス名",
  "auth.encryptionRecovery": "暗号化リカバリ",
  "auth.matrixAccount": "Matrixアカウント",
  "auth.noLoginMethods": "ログイン方法がありません",
  "auth.notChecked": "未確認",
  "auth.password": "パスワード",
  "auth.recoveryKey": "リカバリキー",
  "auth.recoverySecret": "リカバリキーまたはセキュリティフレーズ",
  "auth.securityPhrase": "セキュリティフレーズ",
  "auth.sessionLocked": "セッションはロック中",
  "auth.signIn": "サインイン",
  "auth.supportedRecoveryMethods": "対応している復旧方法",
  "auth.usernameOrMatrixId": "ユーザー名またはMatrix ID",
  "composer.attachFile": "ファイルを添付",
  "composer.attachFileInput": "ファイル添付入力",
  "composer.bold": "太字",
  "composer.code": "コード",
  "composer.emoji": "絵文字",
  "composer.italic": "斜体",
  "composer.link": "リンク",
  "composer.list": "リスト",
  "composer.mention": "メンション",
  "composer.messageComposer": "メッセージ入力欄",
  "composer.placeholder": "{roomName}にメッセージ",
  "composer.replying": "返信中",
  "composer.cancelReply": "返信をキャンセル",
  "context.editMessage": "編集",
  "context.openKeyboardSettings": "キーボードショートカット",
  "context.openRoomInfo": "ルーム情報",
  "context.openSpaceInfo": "スペース情報",
  "context.openThread": "スレッドで返信",
  "context.openUserSettings": "ユーザー設定",
  "context.redactMessage": "削除",
  "context.searchInRoom": "ルーム内を検索",
  "context.selectRoom": "開く",
  "context.selectSpace": "スペースを開く",
  "context.switchAccount": "アカウントを切り替え",
  "dialog.cancelCreate": "作成をキャンセル",
  "dialog.createRoomTitle": "ルームを作成",
  "dialog.createSpaceTitle": "スペースを作成",
  "dialog.invitePeopleTitle": "{name}に招待",
  "dialog.matrixUserId": "MatrixユーザーID",
  "dialog.newDmTitle": "新しいDM",
  "dialog.sendInvite": "招待を送信",
  "dialog.roomName": "ルーム名",
  "dialog.spaceName": "スペース名",
  "dialog.startDm": "DMを開始",
  "dialog.submitCreateRoom": "ルーム作成を実行",
  "dialog.submitCreateSpace": "スペース作成を実行",
  "invite.accept": "招待を承認",
  "invite.decline": "招待を辞退",
  "invite.fromInviter": "{inviter}から",
  "invite.noPending": "保留中の招待はありません",
  "invite.noTopic": "トピックなし",
  "invite.pendingInvites": "保留中の招待",
  "invite.preview": "招待プレビュー",
  "invite.summary": "招待の概要",
  "invite.tabs": "招待ビュー",
  "invite.topic": "トピック",
  "invite.unknownInviter": "不明な招待者",
  "panel.context": "コンテキストパネル",
  "panel.keyboard": "キーボード",
  "panel.focusedContext": "フォーカス中のコンテキスト",
  "panel.recovery": "復旧",
  "panel.roomInfo": "ルーム情報",
  "panel.search": "検索",
  "panel.spaceInfo": "スペース情報",
  "panel.thread": "スレッド",
  "panel.userSettings": "ユーザー設定",
  "room.members": "メンバー",
  "room.directMessage": "ダイレクトメッセージ",
  "room.dmList": "DM一覧",
  "room.exactVerifiedResults": "検証済み完全一致結果",
  "room.files": "ファイル",
  "room.globalDmList": "全体DM一覧",
  "room.invitePeople": "メンバーを招待",
  "room.noRoomSelected": "ルームが選択されていません",
  "room.noSpaces": "スペースがありません",
  "room.notifications": "通知",
  "room.people": "ユーザー",
  "room.roomInfo": "ルーム情報",
  "room.roomScoped": "ルーム内",
  "room.roomSettings": "ルーム設定",
  "room.searchIndex": "検索インデックス",
  "room.spaces": "スペース",
  "room.subscribed": "購読中",
  "room.summary": "ルーム概要",
  "room.tabs": "ルームタブ",
  "room.threadToggle": "スレッドを切り替え",
  "room.timeline": "タイムライン",
  "room.type": "種類",
  "room.unread": "未読",
  "room.unreadCount": "未読 {count} 件",
  "search.noExactMatches": "完全一致はありません",
  "search.matchAttachmentFileName": "添付ファイル名",
  "search.matchMessage": "メッセージ",
  "search.resultCountMany": "\"{query}\" の結果 {count} 件",
  "search.resultCountOne": "\"{query}\" の結果 1 件",
  "search.scopeAll": "すべて",
  "search.scopeDm": "ダイレクト",
  "search.scopeRoom": "ルーム",
  "search.scopeSpace": "スペース",
  "settings.accounts": "アカウント",
  "settings.appearance": "外観",
  "settings.accountSwitcher": "アカウント切り替え",
  "settings.current": "現在",
  "settings.device": "デバイス",
  "settings.emojiFont": "絵文字フォント",
  "settings.fontSystem": "システム",
  "settings.general": "一般",
  "settings.homeserver": "ホームサーバー",
  "settings.keyboard": "キーボード",
  "settings.keyboardDescription": "実装済みデスクトップ操作用のElement互換ショートカットです。",
  "settings.localStore": "分離された暗号化名前空間",
  "settings.localStoreLabel": "ローカルストア",
  "settings.matrixAccount": "Matrixアカウント",
  "settings.notRestored": "未復元",
  "settings.preferences": "環境設定",
  "settings.profile": "プロフィール",
  "settings.profileAvatar": "アバター",
  "settings.profileDisplayName": "表示名",
  "settings.profileDisplayNamePlaceholder": "表示名",
  "settings.profileSavingAvatar": "アップロード中",
  "settings.profileSavingDisplayName": "保存中",
  "settings.profileUpdate": "更新",
  "settings.profileUploadAvatar": "アップロード",
  "settings.searchIndex": "暗号化ローカルインデックス",
  "settings.security": "セキュリティ",
  "settings.securityPrivacy": "セキュリティとプライバシー",
  "settings.session": "セッション",
  "settings.sessionSecretLabel": "セッションシークレット",
  "settings.sessionSecret": "OS資格情報ストア",
  "settings.saving": "保存中",
  "settings.switch": "切り替え",
  "settings.userId": "ユーザーID",
  "settings.theme": "テーマ",
  "settings.themeDark": "ダーク",
  "settings.themeLight": "ライト",
  "settings.themeSystem": "システム",
  "settings.typography": "タイポグラフィ",
  "settings.uiFont": "UIフォント",
  "trust.acceptVerification": "承認",
  "trust.closeVerification": "閉じる",
  "trust.confirmSas": "確認",
  "trust.continueIdentityReset": "続行",
  "trust.crossSigning": "クロス署名",
  "trust.declineVerification": "拒否",
  "trust.deviceBlocked": "ブロック済み",
  "trust.deviceCount": "デバイス {count} 台",
  "trust.deviceOrdinal": "デバイス {index}",
  "trust.deviceUnknown": "不明",
  "trust.deviceUnverified": "未検証",
  "trust.deviceVerified": "検証済み",
  "trust.devices": "デバイス",
  "trust.enableKeyBackup": "有効化",
  "trust.encryption": "暗号化",
  "trust.failureCancelled": "キャンセル済み",
  "trust.failureForbidden": "禁止",
  "trust.failureMismatch": "不一致",
  "trust.failureNetwork": "ネットワーク",
  "trust.failureSdk": "SDKエラー",
  "trust.failureTimeout": "タイムアウト",
  "trust.identityReset": "IDリセット",
  "trust.identityResetAuthUnknown": "認証を利用できません",
  "trust.identityResetPassword": "パスワード",
  "trust.keyBackup": "鍵バックアップ",
  "trust.noDevices": "デバイスなし",
  "trust.resetIdentity": "リセット",
  "trust.sasEmoji": "SAS絵文字 {index}",
  "trust.sasEmojiList": "SAS絵文字",
  "trust.setupCrossSigning": "セットアップ",
  "trust.statusAwaitingAuth": "認証待ち",
  "trust.statusBootstrapping": "セットアップ中",
  "trust.statusConfirming": "確認中",
  "trust.statusDisabled": "無効",
  "trust.statusEnabled": "有効",
  "trust.statusEnabling": "有効化中",
  "trust.statusFailed": "失敗",
  "trust.statusFailedReason": "失敗: {reason}",
  "trust.statusIdle": "待機中",
  "trust.statusInProgress": "進行中",
  "trust.statusMissing": "不足",
  "trust.statusNeedsAttention": "確認が必要",
  "trust.statusNotTrusted": "信頼されていません",
  "trust.statusResetting": "リセット中",
  "trust.statusRestoringBackup": "{restored}/{total} を復元中",
  "trust.statusRestoringBackupOpen": "{restored} を復元中",
  "trust.statusSasPresented": "絵文字を比較",
  "trust.statusTrusted": "信頼済み",
  "trust.statusUnknown": "不明",
  "trust.statusVerificationAccepted": "承認済み",
  "trust.statusVerificationRequested": "リクエスト待ち",
  "trust.statusVerified": "検証済み",
  "trust.verification": "デバイス検証",
  "space.allRooms": "すべてのルーム",
  "space.childRooms": "子ルーム",
  "space.directMessages": "ダイレクトメッセージ",
  "space.home": "ホーム",
  "space.invite": "招待",
  "space.noUnread": "未読なし",
  "space.preferences": "環境設定",
  "space.roomMembership": "ルーム参加状態",
  "space.spacePreferences": "スペース環境設定",
  "space.spaceSettings": "スペース設定",
  "space.summary": "スペース概要",
  "sync.failed": "失敗",
  "sync.failedWithReason": "同期失敗: {reason}",
  "sync.reconnecting": "再接続中",
  "sync.reconnectingWithReason": "同期を再接続中: {reason}",
  "sync.running": "実行中",
  "sync.starting": "開始中",
  "sync.stopped": "停止中",
  "shortcut.activateButton": "フォーカス中のコントロールを実行",
  "shortcut.cancelAutocomplete": "オートコンプリートをキャンセル",
  "shortcut.cancelReplyOrEdit": "返信または編集をキャンセル",
  "shortcut.categoryAccessibility": "アクセシビリティ",
  "shortcut.categoryAutocomplete": "オートコンプリート",
  "shortcut.categoryComposer": "入力欄",
  "shortcut.categoryNavigation": "ナビゲーション",
  "shortcut.categoryRoom": "ルーム",
  "shortcut.categoryRoomList": "ルーム一覧",
  "shortcut.closeDialogOrMenu": "ダイアログまたはメニューを閉じる",
  "shortcut.collapseRoomListSection": "セクションを折りたたむ",
  "shortcut.composerSendShortcut": "入力欄の送信ショートカット",
  "shortcut.enterSends": "Enterで送信",
  "shortcut.expandRoomListSection": "セクションを展開",
  "shortcut.filterRooms": "ルームを検索",
  "shortcut.formatBold": "太字",
  "shortcut.formatCode": "コード",
  "shortcut.formatItalics": "斜体",
  "shortcut.formatLink": "リンクを挿入",
  "shortcut.goHome": "ホームへ移動",
  "shortcut.jumpToFirstMessage": "最初のメッセージへ移動",
  "shortcut.jumpToLatestMessage": "最新メッセージへ移動",
  "shortcut.jumpToOldestUnread": "最古の未読へ移動",
  "shortcut.modEnterSends": "{shortcut}で送信",
  "shortcut.newLine": "改行",
  "shortcut.nextAutocompleteSelection": "次のオートコンプリート候補",
  "shortcut.nextLandmark": "次のランドマーク",
  "shortcut.nextRoomInList": "一覧内の次のルーム",
  "shortcut.nextVisitedRoomOrSpace": "進む",
  "shortcut.noteCallsDeferred": "通話はこのマイルストーンの範囲外です。",
  "shortcut.noteGoHomeAdapted": "ElementのmacOSではCtrl+Shift+Hですが、この試作ではクロスプラットフォームの1行に統一しています。",
  "shortcut.noteUploadUiDeferred": "アップロードUIはまだ実装されていません。",
  "shortcut.openUserSettings": "ユーザー設定",
  "shortcut.parityAdapted": "調整済み",
  "shortcut.parityDeferred": "延期",
  "shortcut.parityNotApplicable": "対象外",
  "shortcut.paritySame": "同一",
  "shortcut.previousAutocompleteSelection": "前のオートコンプリート候補",
  "shortcut.previousLandmark": "前のランドマーク",
  "shortcut.previousRoomInList": "一覧内の前のルーム",
  "shortcut.previousVisitedRoomOrSpace": "戻る",
  "shortcut.searchInRoom": "ルーム内を検索",
  "shortcut.selectNextRoom": "次のルーム",
  "shortcut.selectNextUnreadRoom": "次の未読ルーム",
  "shortcut.selectPreviousRoom": "前のルーム",
  "shortcut.selectPreviousUnreadRoom": "前の未読ルーム",
  "shortcut.selectRoomInRoomList": "ルームを選択",
  "shortcut.sendMessage": "メッセージを送信",
  "shortcut.shortcutKeys": "{label} のショートカット",
  "shortcut.showKeyboardSettings": "キーボード設定",
  "shortcut.scrollTimelineDown": "タイムラインを下へスクロール",
  "shortcut.scrollTimelineUp": "タイムラインを上へスクロール",
  "shortcut.toggleMicrophone": "通話中のマイクを切り替え",
  "shortcut.toggleRightPanel": "右パネルを切り替え",
  "shortcut.toggleSpacePanel": "スペースパネルを切り替え",
  "shortcut.uploadFile": "ファイルをアップロード",
  "timeline.conversation": "会話タイムライン",
  "timeline.conversationStart": "会話の開始",
  "timeline.editedMessage": "編集済み",
  "timeline.editMessage": "メッセージを編集",
  "timeline.editBody": "メッセージ本文を編集",
  "timeline.loading": "読み込み中",
  "timeline.messagesTab": "メッセージ",
  "timeline.openingThread": "スレッドを開いています",
  "timeline.presenceAway": "離席中",
  "timeline.presenceOffline": "オフライン",
  "timeline.presenceOnline": "オンライン",
  "timeline.addReaction": "リアクションを追加",
  "timeline.reactionSummary": "リアクション {key}、{count} 件",
  "timeline.reactionPicker": "リアクションを選択",
  "timeline.reactionOption": "{emoji}でリアクション",
  "timeline.readBy": "{count} 人が既読",
  "timeline.readMarker": "ここまで既読",
  "timeline.olderMessages": "古いメッセージ",
  "timeline.saveEdit": "編集を保存",
  "timeline.cancelEdit": "編集をキャンセル",
  "timeline.redactMessage": "メッセージを削除",
  "timeline.redactedMessage": "メッセージは削除されました",
  "timeline.replyToMessage": "メッセージに返信",
  "timeline.threadComposer": "スレッド入力欄",
  "timeline.threadPlaceholder": "返信",
  "timeline.unsent": "未送信",
  "timeline.sending": "送信中",
  "timeline.notSent": "送信されていません",
  "timeline.cancelledSend": "キャンセル済み",
  "timeline.resendSend": "再送信",
  "timeline.deleteSend": "削除",
  "timeline.cancelSend": "送信をキャンセル",
  "timeline.unsentBar": "未送信のメッセージがあります",
  "timeline.resendAll": "すべて再送信",
  "timeline.cancelAll": "すべてキャンセル",
  "timeline.downloadMedia": "{filename}をダウンロード",
  "timeline.encryptedMedia": "暗号化済み",
  "timeline.threadReplyCountOne": "返信 1 件",
  "timeline.threadReplyCountMany": "返信 {count} 件",
  "timeline.threadRoot": "スレッド元 {eventId}",
  "timeline.threadSummaryWithBody": "{count}件 · {preview}",
  "timeline.threadSummaryWithPreview": "{count}件 · {sender}: {preview}",
  "timeline.threadSummaryWithSender": "{count}件 · {sender}",
  "timeline.typingMany": "{count} 人が入力中",
  "timeline.typingOne": "{user} が入力中",
  "timeline.openThreadSummary": "スレッドを開く、{summary}",
  "timeline.viewReplies": "新しい返信を見る · {count}",
  "workspace.createSpace": "スペースを作成",
  "workspace.home": "ホーム",
  "workspace.invites": "招待",
  "workspace.newDm": "新しいDM",
  "workspace.people": "ユーザー",
  "workspace.rooms": "ルーム",
  "workspace.search": "検索",
  "workspace.searchPlaceholder": "{spaceName}内を検索",
  "workspace.searchScope": "検索範囲",
  "workspace.spaceInfoSettings": "スペース情報と設定",
  "workspace.threads": "スレッド",
  "workspace.userSettings": "ユーザー設定",
  "workspace.workspaces": "ワークスペース"
};

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
