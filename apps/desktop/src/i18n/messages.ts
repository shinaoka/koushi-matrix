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
  | "activity.highlightBadge"
  | "activity.loadMore"
  | "activity.loading"
  | "activity.markAllRead"
  | "activity.markReadFailed"
  | "activity.markRoomRead"
  | "activity.noPreview"
  | "activity.noRecent"
  | "activity.noUnread"
  | "activity.openItem"
  | "activity.recent"
  | "activity.tabs"
  | "activity.unread"
  | "activity.unreadBadge"
  | "auth.checking"
  | "auth.checkLoginMethods"
  | "auth.connecting"
  | "auth.continue"
  | "auth.deviceName"
  | "auth.encryptionRecovery"
  | "app.about"
  | "app.title"
  | "auth.failureForbidden"
  | "auth.failureNetwork"
  | "auth.failureSdk"
  | "auth.failureTimeout"
  | "auth.failureUnsupported"
  | "auth.flowOidc"
  | "auth.flowPassword"
  | "auth.flowSso"
  | "auth.flowToken"
  | "auth.flowUnknown"
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
  | "composer.attachedFile"
  | "composer.attachmentFallback"
  | "composer.attachFile"
  | "composer.attachFileInput"
  | "composer.bold"
  | "composer.code"
  | "composer.emoji"
  | "composer.italic"
  | "composer.link"
  | "composer.list"
  | "composer.mention"
  | "composer.mentionSuggestions"
  | "composer.messageComposer"
  | "composer.imageCompressionCompressed"
  | "composer.imageCompressionOriginal"
  | "composer.imageCompressionPreviewAlt"
  | "composer.imageCompressionTitle"
  | "composer.placeholder"
  | "composer.replying"
  | "composer.removeAttachment"
  | "composer.cancelReply"
  | "composer.selectedMentions"
  | "upload.captionForFile"
  | "upload.clear"
  | "upload.compressed"
  | "upload.dialogTitle"
  | "upload.original"
  | "upload.sizeChoice"
  | "window.title"
  | "context.editMessage"
  | "context.addToFavourites"
  | "context.addToLowPriority"
  | "context.openKeyboardSettings"
  | "context.openRoomInfo"
  | "context.openSpaceInfo"
  | "context.openThread"
  | "context.openUserSettings"
  | "context.redactMessage"
  | "context.removeFromFavourites"
  | "context.removeFromLowPriority"
  | "context.searchInRoom"
  | "context.selectRoom"
  | "context.selectSpace"
  | "context.switchAccount"
  | "dialog.cancel"
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
  | "directory.failureForbidden"
  | "directory.failureInvalid"
  | "directory.failureNetwork"
  | "directory.failureNotFound"
  | "directory.failureSdk"
  | "directory.failureTimeout"
  | "directory.guestCanJoin"
  | "directory.join"
  | "directory.joinFailed"
  | "directory.joining"
  | "directory.joinRoom"
  | "directory.memberCount"
  | "directory.noAlias"
  | "directory.noResults"
  | "directory.results"
  | "directory.search"
  | "directory.searchFailed"
  | "directory.searching"
  | "directory.searchPlaceholder"
  | "directory.searchPublicRooms"
  | "directory.worldReadable"
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
  | "mediaGallery.close"
  | "mediaGallery.encrypted"
  | "mediaGallery.next"
  | "mediaGallery.open"
  | "mediaGallery.openItem"
  | "mediaGallery.previous"
  | "mediaGallery.region"
  | "mediaGallery.viewerTitle"
  | "mediaGallery.zoomIn"
  | "mediaGallery.zoomOut"
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
  | "room.ban"
  | "room.banMember"
  | "room.avatarUrl"
  | "room.currentAvatar"
  | "room.currentTopic"
  | "room.directMessage"
  | "room.dmList"
  | "room.editRoles"
  | "room.editSettings"
  | "room.exactVerifiedResults"
  | "room.files"
  | "room.globalDmList"
  | "room.invitePeople"
  | "room.historyInvited"
  | "room.historyJoined"
  | "room.historyShared"
  | "room.historyVisibility"
  | "room.historyWorldReadable"
  | "room.joinRule"
  | "room.joinRuleInvite"
  | "room.joinRuleKnock"
  | "room.joinRulePrivate"
  | "room.joinRulePublic"
  | "room.joinRuleRestricted"
  | "room.kick"
  | "room.kickMember"
  | "room.management"
  | "room.aliasDialogTitle"
  | "room.aliasInput"
  | "room.clearAlias"
  | "room.clearAliasForMember"
  | "room.editAlias"
  | "room.editAliasForMember"
  | "room.memberOriginalName"
  | "room.saveAlias"
  | "room.setAlias"
  | "room.setAliasForMember"
  | "room.noMembers"
  | "room.noTopic"
  | "room.noRoomSelected"
  | "room.noSpaces"
  | "room.notifications"
  | "room.operationFailed"
  | "room.people"
  | "room.rolePermissions"
  | "room.memberRole"
  | "room.memberRoleFor"
  | "room.noAvatar"
  | "room.roleAdministrator"
  | "room.roleCreator"
  | "room.roleModerator"
  | "room.roleUser"
  | "room.roomInfo"
  | "room.roomScoped"
  | "room.roomSettings"
  | "room.saveAccess"
  | "room.saveAvatar"
  | "room.saveName"
  | "room.saveTopic"
  | "room.searchIndex"
  | "room.settingsLoading"
  | "room.spaces"
  | "room.subscribed"
  | "room.summary"
  | "room.tabs"
  | "room.threadToggle"
  | "room.timeline"
  | "room.topic"
  | "room.type"
  | "room.unban"
  | "room.unbanMember"
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
  | "settings.checkLocalEncryption"
  | "settings.credentialStore"
  | "settings.credentialStoreLinux"
  | "settings.credentialStoreMacos"
  | "settings.credentialStoreWindows"
  | "settings.device"
  | "settings.emojiFont"
  | "settings.fontInter"
  | "settings.fontSystem"
  | "settings.general"
  | "settings.homeserver"
  | "settings.keyboard"
  | "settings.keyboardDescription"
  | "settings.display"
  | "settings.codeBlockWrap"
  | "settings.hideRedacted"
  | "settings.media"
  | "settings.compressImages"
  | "settings.compressImagesAlways"
  | "settings.compressImagesAsk"
  | "settings.compressImagesNever"
  | "settings.notificationBadges"
  | "settings.notificationDesktop"
  | "settings.notificationSound"
  | "settings.notifications"
  | "settings.localStore"
  | "settings.localStoreLabel"
  | "settings.localData"
  | "settings.localDataResetAvailable"
  | "settings.localEncryption"
  | "settings.localEncryptionChecking"
  | "settings.localEncryptionHealthy"
  | "settings.localEncryptionLocked"
  | "settings.localEncryptionMissing"
  | "settings.localEncryptionResetRequired"
  | "settings.localEncryptionResetting"
  | "settings.localEncryptionUnavailable"
  | "settings.localEncryptionUnknown"
  | "settings.matrixAccount"
  | "settings.notRestored"
  | "settings.openRecovery"
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
  | "settings.keyManagement"
  | "settings.roomKeyExport"
  | "settings.roomKeyExportDestination"
  | "settings.roomKeyExportIdle"
  | "settings.roomKeyExporting"
  | "settings.roomKeyExportedUnknown"
  | "settings.roomKeyExportedCount"
  | "settings.roomKeyExportFailed"
  | "settings.exportRoomKeys"
  | "settings.roomKeyImport"
  | "settings.roomKeyImportSource"
  | "settings.roomKeyImportIdle"
  | "settings.roomKeyImporting"
  | "settings.roomKeyImportedCount"
  | "settings.roomKeyImportFailed"
  | "settings.importRoomKeys"
  | "settings.roomKeyPassphrase"
  | "settings.secureBackup"
  | "settings.secureBackupPassphrase"
  | "settings.recoveryKeyDestination"
  | "settings.setupSecureBackup"
  | "settings.secureBackupIdle"
  | "settings.secureBackupSettingUp"
  | "settings.secureBackupEnabled"
  | "settings.secureBackupFailed"
  | "settings.recoveryKeyReady"
  | "settings.recoveryKeySaved"
  | "settings.changeSecureBackupPassphrase"
  | "settings.oldSecureBackupSecret"
  | "settings.newSecureBackupPassphrase"
  | "settings.updateSecureBackupPassphrase"
  | "settings.passphraseChangeIdle"
  | "settings.passphraseChangeChanging"
  | "settings.passphraseChangeChanged"
  | "settings.passphraseChangeRecoveryKeySaved"
  | "settings.passphraseChangeFailed"
  | "settings.resetLocalData"
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
  | "scheduled.cancel"
  | "scheduled.edit"
  | "scheduled.localFallback"
  | "scheduled.localFallbackNotice"
  | "scheduled.save"
  | "scheduled.schedule"
  | "scheduled.sendLater"
  | "scheduled.serverDelayedEvents"
  | "scheduled.timeInput"
  | "scheduled.title"
  | "scheduled.unknownCapability"
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
  | "timeline.readReceiptOverflow"
  | "timeline.readMarker"
  | "timeline.unreadMarker"
  | "timeline.jumpToFirstUnread"
  | "timeline.jumpToBottom"
  | "timeline.jumpToDate"
  | "timeline.openDateInTimeline"
  | "timeline.olderMessages"
  | "timeline.saveEdit"
  | "timeline.cancelEdit"
  | "timeline.redactMessage"
  | "timeline.redactedMessage"
  | "timeline.revealSpoiler"
  | "timeline.pinMessage"
  | "timeline.spoiler"
  | "timeline.unpinMessage"
  | "timeline.pinnedMessages"
  | "timeline.pinnedMessage"
  | "timeline.replyQuoteMissing"
  | "timeline.replyQuoteUnavailable"
  | "timeline.replyQuoteUnknownSender"
  | "timeline.replyQuoteUnsupported"
  | "timeline.replyToMessage"
  | "timeline.messageActions"
  | "timeline.copyMessage"
  | "timeline.copyCode"
  | "timeline.copyPermalink"
  | "timeline.viewSource"
  | "timeline.forwardMessage"
  | "timeline.messageSource"
  | "timeline.closeMessageSource"
  | "timeline.sourceSender"
  | "timeline.sourceBody"
  | "timeline.sourceMetadata"
  | "timeline.sourceNoBody"
  | "timeline.sourceHasMedia"
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
  | "workspace.activity"
  | "workspace.createSpace"
  | "workspace.explore"
  | "workspace.home"
  | "workspace.invites"
  | "workspace.favourites"
  | "workspace.lowPriority"
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
  "activity.highlightBadge": "Mention",
  "activity.loadMore": "Load more activity",
  "activity.loading": "Loading activity",
  "activity.markAllRead": "Mark all read",
  "activity.markReadFailed": "Mark read failed",
  "activity.markRoomRead": "Mark room read",
  "activity.noPreview": "No message preview",
  "activity.noRecent": "No recent activity",
  "activity.noUnread": "No unread activity",
  "activity.openItem": "Open activity item {room}",
  "activity.recent": "Recent",
  "activity.tabs": "Activity views",
  "activity.unread": "Unread",
  "activity.unreadBadge": "Unread",
  "auth.checking": "Checking",
  "auth.checkLoginMethods": "Check login methods",
  "auth.connecting": "Connecting",
  "auth.continue": "Continue",
  "auth.deviceName": "Device name",
  "auth.encryptionRecovery": "Encryption Recovery",
  "app.about": "About Ruri",
  "app.title": "Ruri",
  "auth.failureForbidden": "Login methods are not available for this account",
  "auth.failureNetwork": "Could not reach the homeserver",
  "auth.failureSdk": "Could not check login methods",
  "auth.failureTimeout": "Login method check timed out",
  "auth.failureUnsupported": "Unsupported homeserver",
  "auth.flowOidc": "OIDC",
  "auth.flowPassword": "Password",
  "auth.flowSso": "Single sign-on",
  "auth.flowToken": "Token",
  "auth.flowUnknown": "Unknown method",
  "auth.matrixAccount": "Matrix account",
  "auth.matrixDesktop": "Ruri",
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
  "composer.attachedFile": "Attached file",
  "composer.attachmentFallback": "Attachment",
  "composer.attachFile": "Attach file",
  "composer.attachFileInput": "Attach file input",
  "composer.bold": "Bold",
  "composer.code": "Code",
  "composer.emoji": "Emoji",
  "composer.italic": "Italic",
  "composer.link": "Link",
  "composer.list": "List",
  "composer.mention": "Mention",
  "composer.mentionSuggestions": "Mention suggestions",
  "composer.messageComposer": "Message composer",
  "composer.imageCompressionCompressed": "Compressed",
  "composer.imageCompressionOriginal": "Original",
  "composer.imageCompressionPreviewAlt": "Compressed image preview",
  "composer.imageCompressionTitle": "Compress image",
  "composer.placeholder": "Message {roomName}",
  "composer.replying": "Replying",
  "composer.removeAttachment": "Remove attachment",
  "composer.cancelReply": "Cancel reply",
  "composer.selectedMentions": "Selected mentions",
  "upload.captionForFile": "Caption for {filename}",
  "upload.clear": "Clear upload staging",
  "upload.compressed": "Compressed",
  "upload.dialogTitle": "Upload attachments",
  "upload.original": "Original",
  "upload.sizeChoice": "Upload size",
  "window.title": "Ruri",
  "context.editMessage": "Edit",
  "context.addToFavourites": "Add to Favourites",
  "context.addToLowPriority": "Move to Low priority",
  "context.openKeyboardSettings": "Keyboard shortcuts",
  "context.openRoomInfo": "Room info",
  "context.openSpaceInfo": "Space info",
  "context.openThread": "Reply in thread",
  "context.openUserSettings": "User settings",
  "context.redactMessage": "Redact",
  "context.removeFromFavourites": "Remove from Favourites",
  "context.removeFromLowPriority": "Remove from Low priority",
  "context.searchInRoom": "Search in room",
  "context.selectRoom": "Open",
  "context.selectSpace": "Open Space",
  "context.switchAccount": "Switch account",
  "dialog.cancel": "Cancel",
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
  "directory.failureForbidden": "Forbidden",
  "directory.failureInvalid": "Invalid",
  "directory.failureNetwork": "Network",
  "directory.failureNotFound": "Not found",
  "directory.failureSdk": "SDK",
  "directory.failureTimeout": "Timeout",
  "directory.guestCanJoin": "Guest access",
  "directory.join": "Join",
  "directory.joinFailed": "Join failed: {reason}",
  "directory.joining": "Joining",
  "directory.joinRoom": "Join {name}",
  "directory.memberCount": "{count} members",
  "directory.noAlias": "No canonical alias",
  "directory.noResults": "No public rooms found",
  "directory.results": "Public room results",
  "directory.search": "Search",
  "directory.searchFailed": "Search failed: {reason}",
  "directory.searching": "Searching",
  "directory.searchPlaceholder": "Room name or topic",
  "directory.searchPublicRooms": "Search public rooms",
  "directory.worldReadable": "World readable",
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
  "mediaGallery.close": "Close media viewer",
  "mediaGallery.encrypted": "Encrypted",
  "mediaGallery.next": "Next media",
  "mediaGallery.open": "Open media gallery",
  "mediaGallery.openItem": "Open {filename}",
  "mediaGallery.previous": "Previous media",
  "mediaGallery.region": "Room media gallery",
  "mediaGallery.viewerTitle": "Media viewer",
  "mediaGallery.zoomIn": "Zoom in",
  "mediaGallery.zoomOut": "Zoom out",
  "panel.context": "Context panel",
  "panel.keyboard": "Keyboard",
  "panel.focusedContext": "Focused context",
  "panel.recovery": "Recovery",
  "panel.roomInfo": "Room info",
  "panel.search": "Search",
  "panel.spaceInfo": "Space info",
  "panel.thread": "Thread",
  "panel.userSettings": "User settings",
  "room.avatarUrl": "Room avatar URL",
  "room.ban": "Ban",
  "room.banMember": "Ban {name}",
  "room.currentAvatar": "Current avatar",
  "room.currentTopic": "Current topic",
  "room.members": "Members",
  "room.directMessage": "Direct message",
  "room.dmList": "DM list",
  "room.editRoles": "Edit roles",
  "room.editSettings": "Edit settings",
  "room.exactVerifiedResults": "Exact verified results",
  "room.files": "Files",
  "room.globalDmList": "Global DM list",
  "room.historyInvited": "Since invite",
  "room.historyJoined": "Since join",
  "room.historyShared": "Shared history",
  "room.historyVisibility": "History visibility",
  "room.historyWorldReadable": "World readable",
  "room.invitePeople": "Invite people",
  "room.joinRule": "Join rule",
  "room.joinRuleInvite": "Invite only",
  "room.joinRuleKnock": "Knock",
  "room.joinRulePrivate": "Private",
  "room.joinRulePublic": "Public",
  "room.joinRuleRestricted": "Restricted",
  "room.kick": "Kick",
  "room.kickMember": "Kick {name}",
  "room.management": "Room management",
  "room.memberRole": "Member role",
  "room.memberRoleFor": "Member role for {name}",
  "room.aliasDialogTitle": "Alias for {name}",
  "room.aliasInput": "Alias",
  "room.clearAlias": "Clear alias",
  "room.clearAliasForMember": "Clear alias for {name}",
  "room.editAlias": "Edit alias",
  "room.editAliasForMember": "Edit alias for {name}",
  "room.memberOriginalName": "Original: {name}",
  "room.saveAlias": "Save alias",
  "room.setAlias": "Set alias",
  "room.setAliasForMember": "Set alias for {name}",
  "room.noAvatar": "No avatar",
  "room.noMembers": "No members loaded",
  "room.noTopic": "No topic",
  "room.noRoomSelected": "No room selected",
  "room.noSpaces": "No Spaces",
  "room.notifications": "Notifications",
  "room.operationFailed": "Operation failed",
  "room.people": "People",
  "room.roleAdministrator": "Administrator",
  "room.roleCreator": "Creator",
  "room.roleModerator": "Moderator",
  "room.rolePermissions": "Role permissions",
  "room.roleUser": "User",
  "room.roomInfo": "Room info",
  "room.roomScoped": "Room scoped",
  "room.roomSettings": "Room settings",
  "room.saveAccess": "Save access",
  "room.saveAvatar": "Save avatar",
  "room.saveName": "Save room name",
  "room.saveTopic": "Save topic",
  "room.searchIndex": "Search index",
  "room.settingsLoading": "Room settings loading",
  "room.spaces": "Spaces",
  "room.subscribed": "Subscribed",
  "room.summary": "Room summary",
  "room.tabs": "Room tabs",
  "room.threadToggle": "Toggle thread",
  "room.timeline": "Timeline",
  "room.topic": "Room topic",
  "room.type": "Type",
  "room.unban": "Unban",
  "room.unbanMember": "Unban {name}",
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
  "settings.checkLocalEncryption": "Check local encryption",
  "settings.credentialStore": "Credential store",
  "settings.credentialStoreLinux": "Secret Service",
  "settings.credentialStoreMacos": "macOS Keychain",
  "settings.credentialStoreWindows": "Windows Credential Manager",
  "settings.device": "Device",
  "settings.emojiFont": "Emoji font",
  "settings.fontInter": "Inter",
  "settings.fontSystem": "System",
  "settings.general": "General",
  "settings.homeserver": "Homeserver",
  "settings.keyboard": "Keyboard",
  "settings.keyboardDescription": "Shortcuts compatible with common Matrix desktop clients for implemented actions.",
  "settings.display": "Display",
  "settings.codeBlockWrap": "Wrap long lines in code blocks",
  "settings.hideRedacted": "Hide deleted messages",
  "settings.media": "Media",
  "settings.compressImages": "Compress images",
  "settings.compressImagesAlways": "Always",
  "settings.compressImagesAsk": "Ask",
  "settings.compressImagesNever": "Never",
  "settings.notificationBadges": "Badges",
  "settings.notificationDesktop": "Desktop notifications",
  "settings.notificationSound": "Sound",
  "settings.notifications": "Notifications",
  "settings.localData": "Local data",
  "settings.localDataResetAvailable": "Recovery or local reset available",
  "settings.localEncryption": "Local encryption",
  "settings.localEncryptionChecking": "Checking",
  "settings.localEncryptionHealthy": "Protected",
  "settings.localEncryptionLocked": "Credential store locked",
  "settings.localEncryptionMissing": "Credential missing",
  "settings.localEncryptionResetRequired": "Reset local data required",
  "settings.localEncryptionResetting": "Resetting local data",
  "settings.localEncryptionUnavailable": "Credential store unavailable",
  "settings.localEncryptionUnknown": "Not checked",
  "settings.localStore": "Separate encrypted namespace",
  "settings.localStoreLabel": "Local store",
  "settings.matrixAccount": "Matrix account",
  "settings.notRestored": "Not restored",
  "settings.openRecovery": "Open recovery",
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
  "settings.keyManagement": "Key management",
  "settings.roomKeyExport": "Room key export",
  "settings.roomKeyExportDestination": "Key export destination",
  "settings.roomKeyExportIdle": "Not exported",
  "settings.roomKeyExporting": "Exporting",
  "settings.roomKeyExportedUnknown": "Exported",
  "settings.roomKeyExportedCount": "{count} sessions exported",
  "settings.roomKeyExportFailed": "Export failed: {reason}",
  "settings.exportRoomKeys": "Export room keys",
  "settings.roomKeyImport": "Room key import",
  "settings.roomKeyImportSource": "Key import source",
  "settings.roomKeyImportIdle": "Not imported",
  "settings.roomKeyImporting": "Importing",
  "settings.roomKeyImportedCount": "{imported} of {total} imported",
  "settings.roomKeyImportFailed": "Import failed: {reason}",
  "settings.importRoomKeys": "Import room keys",
  "settings.roomKeyPassphrase": "Room key passphrase",
  "settings.secureBackup": "Secure backup",
  "settings.secureBackupPassphrase": "Secure backup passphrase",
  "settings.recoveryKeyDestination": "Recovery key destination",
  "settings.setupSecureBackup": "Set up secure backup",
  "settings.secureBackupIdle": "Not set up",
  "settings.secureBackupSettingUp": "Setting up",
  "settings.secureBackupEnabled": "Enabled",
  "settings.secureBackupFailed": "Setup failed: {reason}",
  "settings.recoveryKeyReady": "Recovery key ready",
  "settings.recoveryKeySaved": "Recovery key saved",
  "settings.changeSecureBackupPassphrase": "Change secure backup passphrase",
  "settings.oldSecureBackupSecret": "Current recovery secret",
  "settings.newSecureBackupPassphrase": "New secure backup passphrase",
  "settings.updateSecureBackupPassphrase": "Update secure backup passphrase",
  "settings.passphraseChangeIdle": "No passphrase change",
  "settings.passphraseChangeChanging": "Changing",
  "settings.passphraseChangeChanged": "Changed",
  "settings.passphraseChangeRecoveryKeySaved": "Changed; recovery key saved",
  "settings.passphraseChangeFailed": "Change failed: {reason}",
  "settings.resetLocalData": "Reset local data",
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
  "scheduled.cancel": "Cancel scheduled send",
  "scheduled.edit": "Edit scheduled send",
  "scheduled.localFallback": "Local fallback",
  "scheduled.localFallbackNotice": "Will send only while this app is running.",
  "scheduled.save": "Save scheduled send",
  "scheduled.schedule": "Schedule send",
  "scheduled.sendLater": "Send later",
  "scheduled.serverDelayedEvents": "Server scheduled",
  "scheduled.timeInput": "Scheduled send time",
  "scheduled.title": "Scheduled messages",
  "scheduled.unknownCapability": "Checking support",
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
  "shortcut.noteGoHomeAdapted": "macOS uses Ctrl+Shift+H in some Matrix clients; this prototype keeps one cross-platform row.",
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
  "timeline.readReceiptOverflow": "{count} more",
  "timeline.readMarker": "Read up to here",
  "timeline.unreadMarker": "Unread messages",
  "timeline.jumpToFirstUnread": "Jump to first unread, {count} unread",
  "timeline.jumpToBottom": "Jump to bottom, {count} new messages",
  "timeline.jumpToDate": "Jump to date",
  "timeline.openDateInTimeline": "Open date in timeline",
  "timeline.olderMessages": "Older messages",
  "timeline.saveEdit": "Save edit",
  "timeline.cancelEdit": "Cancel edit",
  "timeline.redactMessage": "Redact message",
  "timeline.redactedMessage": "Message redacted",
  "timeline.revealSpoiler": "Reveal spoiler",
  "timeline.pinMessage": "Pin message",
  "timeline.spoiler": "Spoiler",
  "timeline.unpinMessage": "Unpin message",
  "timeline.pinnedMessages": "Pinned messages",
  "timeline.pinnedMessage": "Pinned message",
  "timeline.replyQuoteMissing": "Original message unavailable",
  "timeline.replyQuoteUnavailable": "Original message unavailable",
  "timeline.replyQuoteUnknownSender": "Unknown sender",
  "timeline.replyQuoteUnsupported": "Unsupported message",
  "timeline.replyToMessage": "Reply to message",
  "timeline.messageActions": "Message actions",
  "timeline.copyMessage": "Copy message",
  "timeline.copyCode": "Copy code",
  "timeline.copyPermalink": "Copy permalink",
  "timeline.viewSource": "View source",
  "timeline.forwardMessage": "Forward",
  "timeline.messageSource": "Message source",
  "timeline.closeMessageSource": "Close message source",
  "timeline.sourceSender": "Sender",
  "timeline.sourceBody": "Body",
  "timeline.sourceMetadata": "State",
  "timeline.sourceNoBody": "No body",
  "timeline.sourceHasMedia": "Media",
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
  "workspace.activity": "Activity",
  "workspace.createSpace": "Create space",
  "workspace.explore": "Explore",
  "workspace.home": "Home",
  "workspace.invites": "Invites",
  "workspace.favourites": "Favourites",
  "workspace.lowPriority": "Low priority",
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
  "activity.highlightBadge": "メンション",
  "activity.loadMore": "アクティビティをさらに読み込む",
  "activity.loading": "アクティビティを読み込み中",
  "activity.markAllRead": "すべて既読",
  "activity.markReadFailed": "既読化に失敗しました",
  "activity.markRoomRead": "ルームを既読",
  "activity.noPreview": "メッセージプレビューなし",
  "activity.noRecent": "最近のアクティビティはありません",
  "activity.noUnread": "未読アクティビティはありません",
  "activity.openItem": "{room}のアクティビティを開く",
  "activity.recent": "最近",
  "activity.tabs": "アクティビティ表示",
  "activity.unread": "未読",
  "activity.unreadBadge": "未読",
  "auth.checking": "確認中",
  "auth.checkLoginMethods": "ログイン方法を確認",
  "auth.connecting": "接続中",
  "auth.continue": "続行",
  "auth.deviceName": "デバイス名",
  "auth.encryptionRecovery": "暗号化リカバリ",
  "app.about": "Ruri（瑠璃）について",
  "app.title": "Ruri（瑠璃）",
  "auth.matrixAccount": "Matrixアカウント",
  "auth.matrixDesktop": "Ruri（瑠璃）",
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
  "composer.attachedFile": "添付ファイル",
  "composer.attachmentFallback": "添付",
  "composer.attachFile": "ファイルを添付",
  "composer.attachFileInput": "ファイル添付入力",
  "composer.bold": "太字",
  "composer.code": "コード",
  "composer.emoji": "絵文字",
  "composer.italic": "斜体",
  "composer.link": "リンク",
  "composer.list": "リスト",
  "composer.mention": "メンション",
  "composer.mentionSuggestions": "メンション候補",
  "composer.messageComposer": "メッセージ入力欄",
  "composer.placeholder": "{roomName}にメッセージ",
  "composer.replying": "返信中",
  "composer.removeAttachment": "添付を削除",
  "composer.cancelReply": "返信をキャンセル",
  "composer.selectedMentions": "選択中のメンション",
  "composer.imageCompressionCompressed": "圧縮済み",
  "composer.imageCompressionOriginal": "元の画像",
  "composer.imageCompressionPreviewAlt": "圧縮後の画像プレビュー",
  "composer.imageCompressionTitle": "画像を圧縮",
  "upload.captionForFile": "{filename}のキャプション",
  "upload.clear": "アップロード準備をクリア",
  "upload.compressed": "圧縮",
  "upload.dialogTitle": "添付をアップロード",
  "upload.original": "オリジナル",
  "upload.sizeChoice": "アップロードサイズ",
  "window.title": "Ruri（瑠璃）",
  "context.editMessage": "編集",
  "context.addToFavourites": "お気に入りに追加",
  "context.addToLowPriority": "低優先度に移動",
  "context.openKeyboardSettings": "キーボードショートカット",
  "context.openRoomInfo": "ルーム情報",
  "context.openSpaceInfo": "スペース情報",
  "context.openThread": "スレッドで返信",
  "context.openUserSettings": "ユーザー設定",
  "context.redactMessage": "削除",
  "context.removeFromFavourites": "お気に入りから削除",
  "context.removeFromLowPriority": "低優先度から削除",
  "context.searchInRoom": "ルーム内を検索",
  "context.selectRoom": "開く",
  "context.selectSpace": "スペースを開く",
  "context.switchAccount": "アカウントを切り替え",
  "dialog.cancel": "キャンセル",
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
  "directory.failureForbidden": "禁止",
  "directory.failureInvalid": "不正",
  "directory.failureNetwork": "ネットワーク",
  "directory.failureNotFound": "見つかりません",
  "directory.failureSdk": "SDKエラー",
  "directory.failureTimeout": "タイムアウト",
  "directory.guestCanJoin": "ゲスト参加可",
  "directory.join": "参加",
  "directory.joinFailed": "参加に失敗: {reason}",
  "directory.joining": "参加中",
  "directory.joinRoom": "{name}に参加",
  "directory.memberCount": "メンバー {count} 人",
  "directory.noAlias": "正規エイリアスなし",
  "directory.noResults": "公開ルームが見つかりません",
  "directory.results": "公開ルームの結果",
  "directory.search": "検索",
  "directory.searchFailed": "検索に失敗: {reason}",
  "directory.searching": "検索中",
  "directory.searchPlaceholder": "ルーム名またはトピック",
  "directory.searchPublicRooms": "公開ルームを検索",
  "directory.worldReadable": "誰でも閲覧可",
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
  "mediaGallery.close": "メディアビューアを閉じる",
  "mediaGallery.encrypted": "暗号化済み",
  "mediaGallery.next": "次のメディア",
  "mediaGallery.open": "メディアギャラリーを開く",
  "mediaGallery.openItem": "{filename}を開く",
  "mediaGallery.previous": "前のメディア",
  "mediaGallery.region": "ルームメディアギャラリー",
  "mediaGallery.viewerTitle": "メディアビューア",
  "mediaGallery.zoomIn": "拡大",
  "mediaGallery.zoomOut": "縮小",
  "panel.context": "コンテキストパネル",
  "panel.keyboard": "キーボード",
  "panel.focusedContext": "フォーカス中のコンテキスト",
  "panel.recovery": "復旧",
  "panel.roomInfo": "ルーム情報",
  "panel.search": "検索",
  "panel.spaceInfo": "スペース情報",
  "panel.thread": "スレッド",
  "panel.userSettings": "ユーザー設定",
  "room.avatarUrl": "ルームアバターURL",
  "room.ban": "BAN",
  "room.banMember": "{name}をBAN",
  "room.currentAvatar": "現在のアバター",
  "room.currentTopic": "現在のトピック",
  "room.members": "メンバー",
  "room.directMessage": "ダイレクトメッセージ",
  "room.dmList": "DM一覧",
  "room.editRoles": "ロールを編集",
  "room.editSettings": "設定を編集",
  "room.exactVerifiedResults": "検証済み完全一致結果",
  "room.files": "ファイル",
  "room.globalDmList": "全体DM一覧",
  "room.historyInvited": "招待以降",
  "room.historyJoined": "参加以降",
  "room.historyShared": "共有履歴",
  "room.historyVisibility": "履歴の表示範囲",
  "room.historyWorldReadable": "誰でも閲覧可",
  "room.invitePeople": "メンバーを招待",
  "room.joinRule": "参加ルール",
  "room.joinRuleInvite": "招待のみ",
  "room.joinRuleKnock": "ノック",
  "room.joinRulePrivate": "非公開",
  "room.joinRulePublic": "公開",
  "room.joinRuleRestricted": "制限付き",
  "room.kick": "キック",
  "room.kickMember": "{name}をキック",
  "room.management": "ルーム管理",
  "room.memberRole": "メンバーロール",
  "room.memberRoleFor": "{name}のメンバーロール",
  "room.aliasDialogTitle": "{name}のエイリアス",
  "room.aliasInput": "エイリアス",
  "room.clearAlias": "エイリアスを解除",
  "room.clearAliasForMember": "{name}のエイリアスを解除",
  "room.editAlias": "エイリアスを編集",
  "room.editAliasForMember": "{name}のエイリアスを編集",
  "room.memberOriginalName": "元の名前: {name}",
  "room.saveAlias": "エイリアスを保存",
  "room.setAlias": "エイリアスを設定",
  "room.setAliasForMember": "{name}のエイリアスを設定",
  "room.noAvatar": "アバターなし",
  "room.noMembers": "読み込まれたメンバーはありません",
  "room.noTopic": "トピックなし",
  "room.noRoomSelected": "ルームが選択されていません",
  "room.noSpaces": "スペースがありません",
  "room.notifications": "通知",
  "room.operationFailed": "操作に失敗しました",
  "room.people": "ユーザー",
  "room.roleAdministrator": "管理者",
  "room.roleCreator": "作成者",
  "room.roleModerator": "モデレーター",
  "room.rolePermissions": "ロール権限",
  "room.roleUser": "ユーザー",
  "room.roomInfo": "ルーム情報",
  "room.roomScoped": "ルーム内",
  "room.roomSettings": "ルーム設定",
  "room.saveAccess": "アクセス設定を保存",
  "room.saveAvatar": "アバターを保存",
  "room.saveName": "ルーム名を保存",
  "room.saveTopic": "トピックを保存",
  "room.searchIndex": "検索インデックス",
  "room.settingsLoading": "ルーム設定を読み込み中",
  "room.spaces": "スペース",
  "room.subscribed": "購読中",
  "room.summary": "ルーム概要",
  "room.tabs": "ルームタブ",
  "room.threadToggle": "スレッドを切り替え",
  "room.timeline": "タイムライン",
  "room.topic": "ルームトピック",
  "room.type": "種類",
  "room.unban": "BAN解除",
  "room.unbanMember": "{name}のBANを解除",
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
  "settings.checkLocalEncryption": "ローカル暗号化を確認",
  "settings.credentialStore": "資格情報ストア",
  "settings.credentialStoreLinux": "Linux Secret Service",
  "settings.credentialStoreMacos": "macOSキーチェーン",
  "settings.credentialStoreWindows": "Windows資格情報マネージャー",
  "settings.device": "デバイス",
  "settings.emojiFont": "絵文字フォント",
  "settings.fontSystem": "システム",
  "settings.general": "一般",
  "settings.homeserver": "ホームサーバー",
  "settings.keyboard": "キーボード",
  "settings.keyboardDescription": "実装済み操作の一般的なMatrixデスクトップクライアント互換ショートカットです。",
  "settings.display": "表示",
  "settings.codeBlockWrap": "コードブロックの長い行を折り返す",
  "settings.hideRedacted": "削除されたメッセージを非表示",
  "settings.media": "メディア",
  "settings.compressImages": "画像を圧縮",
  "settings.compressImagesAlways": "常に圧縮",
  "settings.compressImagesAsk": "毎回確認",
  "settings.compressImagesNever": "圧縮しない",
  "settings.notificationBadges": "バッジ",
  "settings.notificationDesktop": "デスクトップ通知",
  "settings.notificationSound": "サウンド",
  "settings.notifications": "通知",
  "settings.localData": "ローカルデータ",
  "settings.localDataResetAvailable": "リカバリーまたはローカルリセットを利用できます",
  "settings.localEncryption": "ローカル暗号化",
  "settings.localEncryptionChecking": "確認中",
  "settings.localEncryptionHealthy": "保護されています",
  "settings.localEncryptionLocked": "資格情報ストアがロックされています",
  "settings.localEncryptionMissing": "資格情報が見つかりません",
  "settings.localEncryptionResetRequired": "ローカルデータのリセットが必要です",
  "settings.localEncryptionResetting": "ローカルデータをリセット中",
  "settings.localEncryptionUnavailable": "資格情報ストアを利用できません",
  "settings.localEncryptionUnknown": "未確認",
  "settings.localStore": "分離された暗号化名前空間",
  "settings.localStoreLabel": "ローカルストア",
  "settings.matrixAccount": "Matrixアカウント",
  "settings.notRestored": "未復元",
  "settings.openRecovery": "リカバリーを開く",
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
  "settings.keyManagement": "鍵管理",
  "settings.roomKeyExport": "ルーム鍵エクスポート",
  "settings.roomKeyExportDestination": "鍵エクスポート先",
  "settings.roomKeyExportIdle": "未エクスポート",
  "settings.roomKeyExporting": "エクスポート中",
  "settings.roomKeyExportedUnknown": "エクスポート済み",
  "settings.roomKeyExportedCount": "{count} セッションをエクスポート済み",
  "settings.roomKeyExportFailed": "エクスポート失敗: {reason}",
  "settings.exportRoomKeys": "ルーム鍵をエクスポート",
  "settings.roomKeyImport": "ルーム鍵インポート",
  "settings.roomKeyImportSource": "鍵インポート元",
  "settings.roomKeyImportIdle": "未インポート",
  "settings.roomKeyImporting": "インポート中",
  "settings.roomKeyImportedCount": "{imported}/{total} をインポート済み",
  "settings.roomKeyImportFailed": "インポート失敗: {reason}",
  "settings.importRoomKeys": "ルーム鍵をインポート",
  "settings.roomKeyPassphrase": "ルーム鍵パスフレーズ",
  "settings.secureBackup": "セキュアバックアップ",
  "settings.secureBackupPassphrase": "セキュアバックアップのパスフレーズ",
  "settings.recoveryKeyDestination": "リカバリーキー保存先",
  "settings.setupSecureBackup": "セキュアバックアップをセットアップ",
  "settings.secureBackupIdle": "未セットアップ",
  "settings.secureBackupSettingUp": "セットアップ中",
  "settings.secureBackupEnabled": "有効",
  "settings.secureBackupFailed": "セットアップ失敗: {reason}",
  "settings.recoveryKeyReady": "リカバリーキー準備済み",
  "settings.recoveryKeySaved": "リカバリーキー保存済み",
  "settings.changeSecureBackupPassphrase": "セキュアバックアップのパスフレーズを変更",
  "settings.oldSecureBackupSecret": "現在のリカバリーシークレット",
  "settings.newSecureBackupPassphrase": "新しいセキュアバックアップのパスフレーズ",
  "settings.updateSecureBackupPassphrase": "セキュアバックアップのパスフレーズを更新",
  "settings.passphraseChangeIdle": "パスフレーズ変更なし",
  "settings.passphraseChangeChanging": "変更中",
  "settings.passphraseChangeChanged": "変更済み",
  "settings.passphraseChangeRecoveryKeySaved": "変更済み、リカバリーキー保存済み",
  "settings.passphraseChangeFailed": "変更失敗: {reason}",
  "settings.resetLocalData": "ローカルデータをリセット",
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
  "scheduled.cancel": "予約送信をキャンセル",
  "scheduled.edit": "予約送信を編集",
  "scheduled.localFallback": "ローカルフォールバック",
  "scheduled.localFallbackNotice": "このアプリが起動中のときだけ送信されます。",
  "scheduled.save": "予約送信を保存",
  "scheduled.schedule": "予約送信",
  "scheduled.sendLater": "あとで送信",
  "scheduled.serverDelayedEvents": "サーバー予約",
  "scheduled.timeInput": "予約送信日時",
  "scheduled.title": "予約メッセージ",
  "scheduled.unknownCapability": "対応状況を確認中",
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
  "shortcut.noteGoHomeAdapted": "一部のMatrixクライアントのmacOS版ではCtrl+Shift+Hですが、この試作ではクロスプラットフォームの1行に統一しています。",
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
  "timeline.readReceiptOverflow": "他 {count} 人",
  "timeline.readMarker": "ここまで既読",
  "timeline.unreadMarker": "未読メッセージ",
  "timeline.jumpToFirstUnread": "最初の未読へ移動、未読 {count} 件",
  "timeline.jumpToBottom": "最新へ移動、新着 {count} 件",
  "timeline.jumpToDate": "日時へ移動",
  "timeline.openDateInTimeline": "タイムラインで開く",
  "timeline.olderMessages": "古いメッセージ",
  "timeline.saveEdit": "編集を保存",
  "timeline.cancelEdit": "編集をキャンセル",
  "timeline.redactMessage": "メッセージを削除",
  "timeline.redactedMessage": "メッセージは削除されました",
  "timeline.revealSpoiler": "スポイラーを表示",
  "timeline.pinMessage": "メッセージをピン留め",
  "timeline.spoiler": "スポイラー",
  "timeline.unpinMessage": "メッセージのピン留めを解除",
  "timeline.pinnedMessages": "ピン留めメッセージ",
  "timeline.pinnedMessage": "ピン留めメッセージ",
  "timeline.replyQuoteMissing": "元のメッセージを利用できません",
  "timeline.replyQuoteUnavailable": "元のメッセージを利用できません",
  "timeline.replyQuoteUnknownSender": "不明な送信者",
  "timeline.replyQuoteUnsupported": "未対応のメッセージ",
  "timeline.replyToMessage": "メッセージに返信",
  "timeline.messageActions": "メッセージ操作",
  "timeline.copyMessage": "メッセージをコピー",
  "timeline.copyCode": "コードをコピー",
  "timeline.copyPermalink": "パーマリンクをコピー",
  "timeline.viewSource": "ソースを表示",
  "timeline.forwardMessage": "転送",
  "timeline.messageSource": "メッセージソース",
  "timeline.closeMessageSource": "メッセージソースを閉じる",
  "timeline.sourceSender": "送信者",
  "timeline.sourceBody": "本文",
  "timeline.sourceMetadata": "状態",
  "timeline.sourceNoBody": "本文なし",
  "timeline.sourceHasMedia": "メディア",
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
  "workspace.activity": "アクティビティ",
  "workspace.createSpace": "スペースを作成",
  "workspace.explore": "探索",
  "workspace.home": "ホーム",
  "workspace.invites": "招待",
  "workspace.favourites": "お気に入り",
  "workspace.lowPriority": "低優先度",
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
