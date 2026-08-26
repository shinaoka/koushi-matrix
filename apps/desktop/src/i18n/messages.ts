export type Locale = "en" | "ja" | "pseudo";

export type MessageId =
  | "action.add"
  | "action.back"
  | "action.cancel"
  | "action.close"
  | "action.continue"
  | "action.createRoom"
  | "action.createSpace"
  | "action.done"
  | "action.more"
  | "action.restartSync"
  | "action.recover"
  | "action.recovering"
  | "action.report"
  | "action.send"
  | "action.sending"
  | "activity.highlightBadge"
  | "activity.loadMore"
  | "activity.loading"
  | "activity.resolvingUnread"
  | "activity.resolveFailed"
  | "activity.retryResolution"
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
  | "auth.createAccount"
  | "auth.deviceName"
  | "auth.encryptionRecovery"
  | "app.about"
  | "app.title"
  | "app.versionMismatch.title"
  | "app.versionMismatch.detail"
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
  | "auth.loginFailureUsernameHint"
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
  | "auth.username"
  | "auth.usernameHelp"
  | "auth.usernameOrMatrixId"
  | "auth.usernamePlaceholder"
  | "composer.attachedFile"
  | "composer.attachmentFallback"
  | "composer.attachFile"
  | "composer.attachFileInput"
  | "composer.bold"
  | "composer.code"
  | "composer.dropFiles"
  | "composer.emoji"
  | "composer.emojiSearch"
  | "composer.emojiRecent"
  | "composer.italic"
  | "composer.link"
  | "composer.list"
  | "composer.mathMode"
  | "composer.mathModeOff"
  | "composer.mathModeOn"
  | "composer.mention"
  | "composer.inlineMention"
  | "composer.mentionLoading"
  | "composer.mentionRoomNotification"
  | "composer.mentionRoomNotificationDescription"
  | "composer.mentionSuggestions"
  | "composer.mentionUsers"
  | "composer.messageComposer"
  | "composer.imageCompressionCompressed"
  | "composer.imageCompressionOriginal"
  | "composer.imageCompressionPreviewAlt"
  | "composer.imageCompressionSaveDefault"
  | "composer.imageCompressionTitle"
  | "composer.placeholder"
  | "composer.replying"
  | "composer.slashCommandUnavailable"
  | "composer.removeAttachment"
  | "composer.cancelReply"
  | "upload.captionForFile"
  | "upload.ask"
  | "upload.clear"
  | "upload.compressed"
  | "upload.dialogTitle"
  | "upload.original"
  | "upload.preparing"
  | "upload.preparationFailed"
  | "upload.retryPreparation"
  | "upload.useOriginal"
  | "upload.previewAlt"
  | "upload.savings"
  | "upload.sizeChoice"
  | "upload.resizeChoice"
  | "upload.formatChoice"
  | "upload.resizeOriginal"
  | "upload.resizeHalf"
  | "upload.resizeQuarter"
  | "upload.resizeEighth"
  | "upload.formatKeep"
  | "upload.formatWebp"
  | "upload.formatJpeg"
  | "upload.formatPng"
  | "upload.outputSummary"
  | "upload.recompressing"
  | "upload.outputReady"
  | "upload.sendAttachments"
  | "window.title"
  | "context.editMessage"
  | "context.addToFavourites"
  | "context.addToLowPriority"
  | "context.ignoreUser"
  | "context.leaveSpace"
  | "context.leaveRoom"
  | "context.leaveConversation"
  | "room.leaveConfirmTitle"
  | "room.leaveConfirmTitleDm"
  | "room.leaveConfirmCopy"
  | "room.leaveConfirmCopyDm"
  | "room.leaveConfirmAction"
  | "room.leaveConfirmActionDm"
  | "context.openKeyboardSettings"
  | "context.openRoomInfo"
  | "context.openSpaceInfo"
  | "context.openThread"
  | "context.openUserInfo"
  | "context.openUserSettings"
  | "context.redactMessage"
  | "context.removeFromFavourites"
  | "context.removeFromLowPriority"
  | "context.searchInRoom"
  | "context.reportContent"
  | "context.reportRoom"
  | "context.reportUser"
  | "context.selectRoom"
  | "context.selectSpace"
  | "context.switchAccount"
  | "context.unignoreUser"
  | "dialog.cancel"
  | "dialog.cancelCreate"
  | "dialog.createRoomTitle"
  | "dialog.createSpaceTitle"
  | "dialog.encryptedRoom"
  | "dialog.inviteCandidates"
  | "dialog.inviteHistory"
  | "dialog.inviteHistoryCurrent"
  | "dialog.inviteHistoryCurrentBadge"
  | "dialog.inviteHistoryEncryptedShared"
  | "dialog.inviteHistoryNonRetroactive"
  | "dialog.inviteHistoryRecovery"
  | "dialog.inviteHistoryWorldReadableWarning"
  | "dialog.inviteInvalidMatrixId"
  | "dialog.invitePeopleTitle"
  | "dialog.inviteScope"
  | "dialog.inviteSearch"
  | "dialog.inviteSelectedTargets"
  | "dialog.openRoomInfo"
  | "dialog.matrixUserId"
  | "dialog.newDmTitle"
  | "dialog.privateRoom"
  | "dialog.publicRoom"
  | "dialog.roomAddress"
  | "dialog.roomTopic"
  | "dialog.roomVisibility"
  | "dialog.sendInvite"
  | "dialog.removeInviteTarget"
  | "dialog.roomName"
  | "dialog.spaceName"
  | "dialog.standardRoomInSpace"
  | "dialog.startDm"
  | "dialog.reportReasonLabel"
  | "dialog.reportReasonPlaceholder"
  | "dialog.reportReasonTitle"
  | "dialog.submitCreateRoom"
  | "dialog.submitCreateSpace"
  | "diagnostics.copied"
  | "diagnostics.copy"
  | "diagnostics.copyFailed"
  | "diagnostics.copying"
  | "diagnostics.open"
  | "diagnostics.title"
  | "emoji.category.people"
  | "emoji.category.nature"
  | "emoji.category.foods"
  | "emoji.category.activity"
  | "emoji.category.places"
  | "emoji.category.objects"
  | "emoji.category.symbols"
  | "emoji.category.flags"
  | "emoji.noResults"
  | "files.empty"
  | "files.error"
  | "files.filterKinds"
  | "files.filterPlaceholder"
  | "files.kind.audio"
  | "files.kind.file"
  | "files.kind.image"
  | "files.kind.sticker"
  | "files.kind.video"
  | "files.loading"
  | "files.sort.filename"
  | "files.sort.newestFirst"
  | "files.sort.oldestFirst"
  | "files.sort.sender"
  | "files.sortLabel"
  | "files.title"
  | "help.learnMore"
  | "help.userTrust.deviceStateBody"
  | "help.userTrust.deviceStateTitle"
  | "help.userTrust.effectiveTrustBody"
  | "help.userTrust.effectiveTrustTitle"
  | "help.userTrust.explain"
  | "help.userTrust.identityResetBody"
  | "help.userTrust.identityResetTitle"
  | "help.userTrust.title"
  | "help.userTrust.unverifiedBody"
  | "help.userTrust.unverifiedTitle"
  | "help.userTrust.verifiedBody"
  | "help.userTrust.verifiedTitle"
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
  | "directory.previewAlreadyJoined"
  | "directory.previewCancel"
  | "directory.previewFailed"
  | "directory.previewInviteOnly"
  | "directory.previewLoading"
  | "directory.previewOpenRoom"
  | "directory.previewRestricted"
  | "directory.previewTitle"
  | "directory.previewUnknownJoinRule"
  | "directory.noResults"
  | "directory.results"
  | "directory.search"
  | "directory.addressHelper"
  | "directory.addressIsUser"
  | "directory.addressLabel"
  | "directory.addressNotRecognized"
  | "directory.addressPlaceholder"
  | "directory.joinSectionTitle"
  | "directory.preview"
  | "directory.roomBadge"
  | "directory.searchSectionTitle"
  | "directory.searchServerHelper"
  | "directory.searchTermLabel"
  | "directory.searchFailed"
  | "directory.searching"
  | "directory.searchPlaceholder"
  | "directory.searchServer"
  | "directory.searchServerPlaceholder"
  | "directory.spaceBadge"
  | "directory.unnamedRoom"
  | "directory.unnamedSpace"
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
  | "mediaGallery.empty"
  | "mediaGallery.next"
  | "mediaGallery.noPreview"
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
  | "panel.people"
  | "panel.profile"
  | "people.memberCount"
  | "people.memberActions"
  | "people.openProfile"
  | "people.searchMembers"
  | "people.noSearchResults"
  | "people.you"
  | "people.sendMessage"
  | "people.setAlias"
  | "people.unknownUser"
  | "room.members"
  | "room.ban"
  | "room.banMember"
  | "room.avatarUrl"
  | "room.accessAndHistory"
  | "room.accessAndHistoryHint"
  | "room.currentAvatar"
  | "room.currentTopic"
  | "room.copyShareLink"
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
  | "room.historyInvitedDescription"
  | "room.historyJoinedDescription"
  | "room.historySharedDescription"
  | "room.historySharedEncryptedHint"
  | "room.historyNonRetroactive"
  | "room.historyRecoveryRequired"
  | "room.historyWorldReadableDescription"
  | "room.historyWorldReadableWarning"
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
  | "room.messageMember"
  | "room.aliasDialogTitle"
  | "room.aliasInput"
  | "room.clearAlias"
  | "room.clearAliasForMember"
  | "room.editAlias"
  | "room.editAliasForMember"
  | "room.memberOriginalName"
  | "room.setAlias"
  | "room.setAliasForMember"
  | "room.noMembers"
  | "room.noTopic"
  | "room.noRoomSelected"
  | "room.noSpaces"
  | "room.notifications"
  | "room.notifyModeAll"
  | "room.notifyModeMentions"
  | "room.notifyModeMute"
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
  | "room.repair"
  | "room.repairTimeline"
  | "room.repairTimelineHint"
  | "room.status"
  | "room.statusEncrypted"
  | "room.statusHistoryLimited"
  | "room.statusHistoryShared"
  | "room.statusHistoryWorldReadable"
  | "room.statusNotEncrypted"
  | "room.statusPrivate"
  | "room.statusPublic"
  | "room.reshareRoomKeys"
  | "room.reshareRoomKeysHint"
  | "room.reshareRoomKeysPending"
  | "room.reshareRoomKeysSuccess"
  | "room.reshareRoomKeysNoSession"
  | "room.reshareRoomKeysNoRecipients"
  | "room.reshareRoomKeysStaleSession"
  | "room.reshareRoomKeysError"
  | "room.dangerousEncryptionDebugging"
  | "room.dangerousEncryptionDebuggingWarning"
  | "room.forceNewEncryptionSession"
  | "room.shareIndex0Key"
  | "room.resendIndex0Key"
  | "room.forceNewEncryptionSessionConfirm"
  | "room.shareIndex0KeyConfirm"
  | "room.resendIndex0KeyConfirm"
  | "room.confirm"
  | "room.cancel"
  | "room.encryptionDebugOutcomeCompleted"
  | "room.encryptionDebugOutcomeRefusedNotEncrypted"
  | "room.encryptionDebugOutcomeRefusedIndexAdvanced"
  | "room.encryptionDebugOutcomeNoSession"
  | "room.encryptionDebugOutcomeNoRecipients"
  | "room.encryptionDebugOutcomeInboundSessionMissing"
  | "room.encryptionDebugOutcomeInboundIndexAdvanced"
  | "room.encryptionDebugOutcomeOriginalLedgerMissing"
  | "room.encryptionDebugOutcomeStaleIdentityRefused"
  | "room.encryptionDebugOutcomeCancelledStale"
  | "room.encryptionDebugOutcomePolicyBlocked"
  | "room.encryptionDebugOutcomeDeadline"
  | "room.encryptionDebugOutcomeFailed"
  | "room.saveAccess"
  | "room.saveHistoryVisibility"
  | "room.saveJoinRule"
  | "room.saveAvatar"
  | "room.saveName"
  | "room.saveTopic"
  | "room.searchIndex"
  | "room.settingsLoading"
  | "room.spaces"
  | "room.subscribed"
  | "room.summary"
  | "room.tabs"
  | "room.rightPanelToggle"
  | "room.returnToInvite"
  | "room.timeline"
  | "room.topic"
  | "room.type"
  | "room.unban"
  | "room.unbanMember"
  | "room.unread"
  | "room.unreadCount"
  | "roomList.category"
  | "roomList.categoryDms"
  | "roomList.categoryRooms"
  | "roomList.categorySummary"
  | "roomList.categorySummaryWithHighlights"
  | "roomList.filterRooms"
  | "roomList.filterUnread"
  | "roomList.filterPeople"
  | "roomList.filterFavourites"
  | "roomList.filterInvites"
  | "roomList.filterDmsPlaceholder"
  | "roomList.filterRoomsPlaceholder"
  | "roomList.clearFilter"
  | "roomList.noMatchingDms"
  | "roomList.noMatchingRooms"
  | "roomList.sort"
  | "roomList.sortLabel"
  | "roomList.sortActive"
  | "roomList.sortName"
  | "roomList.loading"
  | "roomList.failed"
  | "room.markAsRead"
  | "room.markAsUnread"
  | "search.indexingPending"
  | "search.noExactMatches"
  | "search.matchAttachmentFileName"
  | "search.matchMessage"
  | "search.resultCountMany"
  | "search.resultCountOne"
  | "search.searching"
  | "search.searchingFor"
  | "search.tooShort"
  | "search.scopeAll"
  | "search.scopeRoom"
  | "search.scopeSpace"
  | "sessionStatus.authentication"
  | "sessionStatus.authOauth"
  | "sessionStatus.authPassword"
  | "sessionStatus.authSso"
  | "sessionStatus.authToken"
  | "sessionStatus.backupDisabled"
  | "sessionStatus.backupReady"
  | "sessionStatus.checking"
  | "sessionStatus.copyDeviceId"
  | "sessionStatus.crossSigned"
  | "sessionStatus.deviceId"
  | "sessionStatus.deviceName"
  | "sessionStatus.failed"
  | "sessionStatus.failureSdk"
  | "sessionStatus.failureTimedOut"
  | "sessionStatus.failureUnavailable"
  | "sessionStatus.homeserver"
  | "sessionStatus.identity"
  | "sessionStatus.identityMissing"
  | "sessionStatus.identityUnverified"
  | "sessionStatus.identityVerified"
  | "sessionStatus.keyBackup"
  | "sessionStatus.lastChecked"
  | "sessionStatus.manageAccount"
  | "sessionStatus.notChecked"
  | "sessionStatus.notCrossSigned"
  | "sessionStatus.open"
  | "sessionStatus.openWithRuntimeWarning"
  | "sessionStatus.openWithRuntimeWarnings"
  | "sessionStatus.ownerCrossSigning"
  | "sessionStatus.recheck"
  | "sessionStatus.retry"
  | "sessionStatus.runtimeAlertSecureBackup"
  | "sessionStatus.runtimeWarningCount"
  | "sessionStatus.runtimeWarnings"
  | "sessionStatus.runtimeWarningsCount"
  | "sessionStatus.sync"
  | "sessionStatus.syncError"
  | "sessionStatus.syncRunning"
  | "sessionStatus.syncStarting"
  | "sessionStatus.syncStopped"
  | "sessionStatus.title"
  | "sessionStatus.unavailable"
  | "sessionStatus.unverified"
  | "sessionStatus.unknown"
  | "sessionStatus.userId"
  | "sessionStatus.verification"
  | "sessionStatus.verified"
  | "settings.accounts"
  | "settings.appearance"
  | "settings.accountSwitcher"
  | "settings.current"
  | "settings.autoLoadOlderMessages"
  | "settings.autoLoadOlderMessagesDescription"
  | "settings.threadRootLatestReply"
  | "settings.threadRootLatestReplyDescription"
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
  | "settings.timeline"
  | "settings.display"
  | "settings.displayDensity"
  | "settings.densityCompact"
  | "settings.densityDefault"
  | "settings.densityComfortable"
  | "settings.codeBlockWrap"
  | "settings.hideRedacted"
  | "settings.notificationBadges"
  | "settings.notificationDesktop"
  | "settings.notificationSound"
  | "settings.sendReadReceipts"
  | "settings.sendTypingNotifications"
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
  | "settings.messagingPrivacy"
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
  | "settings.accountManagement"
  | "settings.manageAccount"
  | "settings.manageAccountHint"
  | "settings.changePassword"
  | "settings.changePasswordLabel"
  | "settings.changePasswordConfirm"
  | "settings.changePasswordMismatch"
  | "settings.passwordChanged"
  | "settings.deactivateAccount"
  | "settings.deactivateAccountErase"
  | "settings.deactivateAccountConfirm"
  | "settings.accountDeactivated"
  | "settings.accountManagementFailed"
  | "settings.signOut"
  | "settings.keyManagement"
  | "settings.roomKeyExport"
  | "settings.roomKeyExportDestination"
  | "settings.roomKeyExportIdle"
  | "settings.roomKeyExporting"
  | "settings.roomKeyExportedUnknown"
  | "settings.roomKeyExportedCount"
  | "settings.roomKeyExportFailed"
  | "settings.exportRoomKeys"
  | "settings.chooseRoomKeyExportFile"
  | "settings.roomKeyImport"
  | "settings.roomKeyImportSource"
  | "settings.roomKeyImportIdle"
  | "settings.roomKeyImporting"
  | "settings.roomKeyImportedCount"
  | "settings.roomKeyImportFailed"
  | "settings.importRoomKeys"
  | "settings.chooseRoomKeyImportFile"
  | "settings.roomKeyPassphrasePromptExport"
  | "settings.roomKeyPassphrasePromptImport"
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
  | "settings.resetLocalDataConfirm"
  | "gate.resetForNewDevice"
  | "gate.resetForNewDeviceConfirm"
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
  | "scheduled.bodyInput"
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
  | "trust.cancelIdentityReset"
  | "trust.continueIdentityReset"
  | "trust.crossSigning"
  | "trust.declineVerification"
  | "trust.deviceBlocked"
  | "trust.deviceCount"
  | "trust.deviceOrdinal"
  | "trust.deviceUnknown"
  | "trust.deviceUnverified"
  | "trust.deviceVerified"
  | "trust.deviceCrossSigned"
  | "trust.deviceNotCrossSigned"
  | "trust.devices"
  | "trust.enableKeyBackup"
  | "trust.encryption"
  | "trust.failureCancelled"
  | "trust.failureForbidden"
  | "trust.failureInvalidPassphrase"
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
  | "trust.userIdentityReset"
  | "trust.userUnverified"
  | "trust.userVerified"
  | "trust.verification"
  | "gate.title"
  | "gate.sessionExpired"
  | "gate.sessionExpiredCopy"
  | "gate.verifying"
  | "gate.finishing"
  | "gate.preparing"
  | "gate.checking"
  | "gate.discovering"
  | "gate.rejecting"
  | "gate.locked"
  | "gate.otherDevice"
  | "gate.verifyRecoveryKey"
  | "gate.noRecoveryKeyTitle"
  | "gate.noRecoveryKeyCopy"
  | "gate.deviceVerificationDialogTitle"
  | "gate.deviceVerificationDialogCopy"
  | "gate.useRecoveryKey"
  | "gate.tryDeviceVerificationAnyway"
  | "gate.match"
  | "gate.mismatch"
  | "gate.recoverySecret"
  | "gate.recover"
  | "gate.destination"
  | "gate.passphrase"
  | "gate.bootstrap"
  | "gate.saved"
  | "gate.retry"
  | "gate.cleanupOffer"
  | "gate.cleanupDialogTitle"
  | "gate.cleanupDialogCopy"
  | "gate.cleanupConfirm"
  | "gate.cleanupResolving"
  | "gate.cleanupRemoving"
  | "gate.cleanupAccountPassword"
  | "gate.cleanupContinue"
  | "gate.cleanupRemoteFailed"
  | "gate.cleanupRetryRemote"
  | "gate.cleanupEraseAnywayOffer"
  | "gate.cleanupEraseAnywayTitle"
  | "gate.cleanupEraseAnywayCopy"
  | "gate.cleanupEraseAnywayConfirm"
  | "gate.cleanupResettingLocal"
  | "gate.cleanupErasingLocal"
  | "gate.cleanupLocalFailed"
  | "gate.cleanupRetryLocal"
  | "gate.cleanupCommandFailed"
  | "gate.signOut"
  | "gate.secureBackupTitle"
  | "gate.secureBackupChecking"
  | "gate.secureBackupInactive"
  | "gate.secureBackupNeedsRecovery"
  | "gate.secureBackupRecoveryKey"
  | "gate.secureBackupRecover"
  | "gate.secureBackupSetupTitle"
  | "gate.secureBackupSetupCopy"
  | "gate.secureBackupPassphrase"
  | "gate.secureBackupRecoveryKeyDestination"
  | "gate.secureBackupChooseDestination"
  | "gate.secureBackupDestinationSelected"
  | "gate.secureBackupDestinationNotSelected"
  | "gate.secureBackupDestinationSelectionFailed"
  | "gate.secureBackupSetup"
  | "gate.secureBackupExplicitDisabledTitle"
  | "gate.secureBackupExplicitDisabledCopy"
  | "gate.secureBackupReenable"
  | "gate.secureBackupReenableConfirm"
  | "gate.secureBackupCreating"
  | "gate.secureBackupDeliveryRequired"
  | "gate.secureBackupUploading"
  | "gate.secureBackupPendingZero"
  | "gate.secureBackupPendingOne"
  | "gate.secureBackupPendingTwoToTen"
  | "gate.secureBackupPendingElevenToOneHundred"
  | "gate.secureBackupPendingOverOneHundred"
  | "gate.secureBackupPendingUnknown"
  | "gate.secureBackupRetrying"
  | "gate.secureBackupFailureNetwork"
  | "gate.secureBackupFailureRateLimited"
  | "gate.secureBackupFailureInvalidRecoveryKey"
  | "gate.secureBackupFailureBackupKeyMismatch"
  | "gate.secureBackupFailureSecretStorageIncomplete"
  | "gate.secureBackupFailureArtifactDelivery"
  | "gate.secureBackupFailureForbidden"
  | "gate.secureBackupFailureTimeout"
  | "gate.secureBackupFailureSdk"
  | "gate.secureBackupRetry"
  | "gate.secureBackupDiagnostics"
  | "gate.secureBackupCommandFailed"
  | "gate.secureBackupRuntimeDegraded"
  | "slidingSync.blockedTitle"
  | "slidingSync.unsupported"
  | "slidingSync.unreachable"
  | "slidingSync.invalidResponse"
  | "slidingSync.changeHomeserver"
  | "gate.failureCancelled"
  | "gate.failureForbidden"
  | "gate.failureNoProof"
  | "gate.failureSdk"
  | "gate.verificationCommandFailed"
  | "gate.rejectNoProof"
  | "gate.rejectUser"
  | "space.allRooms"
  | "space.childRooms"
  | "space.directMessages"
  | "space.home"
  | "space.invite"
  | "space.localIcon"
  | "space.localIconPlaceholder"
  | "space.localName"
  | "space.localNamePlaceholder"
  | "space.localPresentation"
  | "space.resetLocalPresentation"
  | "space.noUnread"
  | "space.preferences"
  | "space.roomMembership"
  | "space.spacePreferences"
  | "space.spaceSettings"
  | "space.summary"
  | "sync.failed"
  | "sync.failedWithReason"
  | "sync.reasonAuth"
  | "sync.reasonHttp"
  | "sync.reasonInternal"
  | "sync.reasonNetworkError"
  | "sync.reasonNetworkOffline"
  | "sync.reasonStore"
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
  | "shortcut.toggleFullscreen"
  | "shortcut.toggleMicrophone"
  | "shortcut.toggleRightPanel"
  | "shortcut.toggleSpacePanel"
  | "shortcut.uploadFile"
  | "timeline.conversation"
  | "timeline.conversationStart"
  | "timeline.notice.roomCreate"
  | "timeline.notice.roomPowerLevels"
  | "timeline.notice.roomGuestAccess"
  | "timeline.notice.roomEncryption"
  | "timeline.notice.spaceParent"
  | "timeline.notice.roomJoinRules"
  | "timeline.notice.roomHistoryVisibility"
  | "timeline.notice.roomPinnedEvents"
  | "timeline.notice.roomNameSet"
  | "timeline.notice.roomNameChanged"
  | "timeline.notice.roomNameRemoved"
  | "timeline.notice.roomNameChangedGeneric"
  | "timeline.editedMessage"
  | "timeline.editMessage"
  | "timeline.editBody"
  | "timeline.loading"
  | "timeline.gapRepairing"
  | "timeline.gapRepairFailed"
  | "timeline.threadRootLoading"
  | "timeline.threadRootUnavailable"
  | "timeline.messagesTab"
  | "timeline.openingThread"
  | "timeline.presenceAway"
  | "timeline.presenceOffline"
  | "timeline.presenceOnline"
  | "timeline.addReaction"
  | "timeline.reactionSummary"
  | "timeline.reactionPicker"
  | "timeline.reactionOption"
  | "timeline.reactionTooltip"
  | "timeline.reactionSenderOverflow"
  | "timeline.reactionSenderUnknown"
  | "timeline.readBy"
  | "timeline.readReceiptOverflow"
  | "timeline.readMarker"
  | "timeline.readStateSyncing"
  | "timeline.readStateNotSynced"
  | "timeline.readStateReasonAuthentication"
  | "timeline.readStateReasonRateLimited"
  | "timeline.readStateReasonTimeout"
  | "timeline.readStateReasonTransport"
  | "timeline.readStateReasonServer"
  | "timeline.readStateReasonCapacity"
  | "timeline.readStateReasonSdk"
  | "timeline.unreadMarker"
  | "timeline.jumpToFirstUnread"
  | "timeline.jumpToBottom"
  | "timeline.olderMessages"
  | "timeline.latest"
  | "timeline.navigation"
  | "timeline.saveEdit"
  | "timeline.cancelEdit"
  | "timeline.redactMessage"
  | "timeline.redactedMessage"
  | "timeline.revealSpoiler"
  | "timeline.pinMessage"
  | "timeline.spoiler"
  | "timeline.unpinMessage"
  | "timeline.pinnedMessages"
  | "timeline.pinnedMessagesCount"
  | "timeline.pinnedMessage"
  | "timeline.pinnedMessagesEmpty"
  | "timeline.pinnedEventUnableToDecrypt"
  | "timeline.pinnedEventUnavailable"
  | "timeline.pinnedNavigationLoading"
  | "timeline.pinnedNavigationFailed"
  | "timeline.pinnedNavigationRetry"
  | "timeline.replyQuoteMissing"
  | "timeline.replyQuoteUnavailable"
  | "timeline.replyQuoteUnknownSender"
  | "timeline.replyQuoteUnsupported"
  | "timeline.replyToMessage"
  | "timeline.replyInThread"
  | "timeline.messageActions"
  | "timeline.copyMessage"
  | "timeline.copyCode"
  | "timeline.copyPermalink"
  | "timeline.viewSource"
  | "timeline.removeMessage"
  | "timeline.recoveryCheckingLocal"
  | "timeline.recoveryCheckingBackup"
  | "timeline.recoveryRequestingDevices"
  | "timeline.recoveryRepairingOlm"
  | "timeline.recoveryWaitingForKey"
  | "timeline.recoveryRetrying"
  | "timeline.recoveryRecovered"
  | "timeline.recoveryTemporarilyFailed"
  | "timeline.recoveryExhausted"
  | "timeline.recoveryUnrecoverable"
  | "timeline.recoveryGuidanceAnotherDevice"
  | "timeline.recoveryGuidanceBackup"
  | "timeline.recoveryGuidanceSenderMessage"
  | "timeline.recoveryGuidanceRepost"
  | "timeline.keyRequestAwaiting"
  | "timeline.keyRequestStillWaiting"
  | "timeline.keyRequestWithheld"
  | "timeline.keyRequestUnavailable"
  | "timeline.keyRequestUnauthorised"
  | "timeline.keyRequestUnverified"
  | "timeline.keyRequestBlacklisted"
  | "timeline.keyRequestRecovered"
  | "timeline.keyRequestToast"
  | "timeline.requestRoomKey"
  | "timeline.forwardMessage"
  | "timeline.messageSource"
  | "timeline.closeMessageSource"
  | "timeline.sourceEventId"
  | "timeline.copyEventId"
  | "timeline.encryptionDetails"
  | "timeline.megolmSessionFingerprint"
  | "timeline.copyMegolmSessionFingerprint"
  | "timeline.megolmRotationReason"
  | "timeline.megolmReasonInitial"
  | "timeline.megolmReasonExpiredTime"
  | "timeline.megolmReasonExpiredMessageCount"
  | "timeline.megolmReasonMembershipOrDeviceChange"
  | "timeline.megolmReasonEncryptionSettingsChanged"
  | "timeline.megolmReasonExplicitDiscard"
  | "timeline.megolmReasonFullMemberListReload"
  | "timeline.megolmReasonRoomSubscription"
  | "timeline.megolmReasonLimitedSyncResponse"
  | "timeline.megolmReasonKeyShareFailure"
  | "timeline.megolmReasonStoreMissing"
  | "timeline.megolmReasonInvalidated"
  | "timeline.megolmReasonUnknown"
  | "timeline.megolmReasonNotRetained"
  | "timeline.originalEventSource"
  | "timeline.copyOriginalEventSource"
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
  | "timeline.sent"
  | "timeline.resendSend"
  | "timeline.deleteSend"
  | "timeline.cancelSend"
  | "timeline.unsentBar"
  | "timeline.resendAll"
  | "timeline.cancelAll"
  | "timeline.downloadMedia"
  | "timeline.encryptedMedia"
  | "timeline.mediaUploadProgress"
  | "timeline.mediaDownloadPending"
  | "timeline.mediaDownloadFailed"
  | "timeline.mediaDownloadRetry"
  | "timeline.mediaOpenFile"
  | "timeline.mediaDetails"
  | "timeline.mediaDetailsTitle"
  | "timeline.closeMediaDetails"
  | "timeline.mediaViewer"
  | "timeline.threadReplyCountOne"
  | "timeline.threadReplyCountMany"
  | "timeline.threadRoot"
  | "timeline.threadSummaryWithBody"
  | "timeline.threadSummaryWithPreview"
  | "timeline.threadSummaryWithSender"
  | "timeline.threadNotificationCount"
  | "timeline.typingMany"
  | "timeline.typingOne"
  | "timeline.openThreadSummary"
  | "timeline.viewReplies"
  | "workspace.activity"
  | "workspace.createSpace"
  | "workspace.explore"
  | "workspace.homeAttention"
  | "workspace.filters"
  | "workspace.home"
  | "workspace.invites"
  | "workspace.favourites"
  | "workspace.lowPriority"
  | "workspace.newDm"
  | "workspace.notJoined"
  | "workspace.people"
  | "workspace.rooms"
  | "workspace.resizeRoomList"
  | "workspace.resizeRightPanel"
  | "workspace.search"
  | "workspace.searchEverywhere"
  | "workspace.searchInRoom"
  | "workspace.searchPlaceholder"
  | "workspace.searchScope"
  | "workspace.spaceInfoSettings"
  | "workspace.threads"
  | "workspace.userSettings"
  | "workspace.workspaces"
  | "spaceMembers.title"
  | "spaceMembers.search"
  | "spaceMembers.navLabel"
  | "spaceMembers.navCount"
  | "spaceMembers.navAccessible"
  | "spaceMembers.joinedCount"
  | "spaceMembers.sectionJoined"
  | "spaceMembers.sectionInvited"
  | "spaceMembers.sectionChildOnly"
  | "spaceMembers.childRoomContext"
  | "spaceMembers.childRoomCountOne"
  | "spaceMembers.childRoomCount"
  | "spaceMembers.invite"
  | "spaceMembers.invitePending"
  | "spaceMembers.inviteFailed"
  | "spaceMembers.cancelInvite"
  | "spaceMembers.cancelInvitePending"
  | "spaceMembers.cancelInviteFailed"
  | "spaceMembers.loadFailed"
  | "spaceMembers.syncIncomplete"
  | "spaceMembers.noResults"
  | "spaceMembers.roleSelect"
  | "spaceMembers.roleUpdateFailed"
  | "spaceMembers.roleReload"
  | "spaceMembers.roleConfirmTitle"
  | "spaceMembers.roleConfirmCopy"
  | "spaceMembers.roleConfirmAction"
  | "threads.empty"
  | "threads.error"
  | "threads.loading"
  | "threads.open"
  | "threads.replyCount"
  | "threads.title"
  | "settings.searchHistory"
  | "settings.searchHistoryCrawler"
  | "settings.searchHistoryPause"
  | "settings.searchHistoryResume"
  | "settings.searchHistoryRebuild"
  | "settings.searchHistoryRebuildConfirm"
  | "settings.searchHistorySpeed"
  | "settings.searchHistorySpeedStandard"
  | "settings.searchHistorySpeedFast"
  | "settings.searchHistorySpeedSlow"
  | "settings.searchHistorySpeedPaused"
  | "settings.searchHistorySpeedCurrent"
  | "settings.searchHistoryIncludeCaptions"
  | "settings.searchHistoryIncludeFilenames"
  | "settings.searchHistoryActivity"
  | "settings.searchHistoryIndexingProgress"
  | "settings.searchHistoryPausedProgress"
  | "settings.searchHistoryActivitySummary"
  | "settings.searchHistoryActivityIdle"
  | "settings.searchHistoryActivityLastIndexed"
  | "settings.searchHistoryActivityJustNow"
  | "settings.searchHistoryActivityMinutesAgo"
  | "settings.searchHistoryActivityHoursAgo"
  | "settings.searchHistoryActivityDaysAgo"
  | "settings.searchHistoryActivityHint"
  | "settings.searchHistoryRoomStatus"
  | "settings.searchHistoryRoomIdle"
  | "settings.searchHistoryRoomQueued"
  | "settings.searchHistoryRoomRunning"
  | "settings.searchHistoryRoomCompleted"
  | "settings.searchHistoryRoomFailed"
  | "settings.searchHistoryStartRoom"
  | "settings.searchHistoryStopRoom"
  | "settings.searchHistoryRoomUnknown"
  | "settings.urlPreviews"
  | "settings.urlPreviewsEnabled"
  | "settings.urlPreviewsDescription"
  | "settings.urlPreviewsUnencrypted"
  | "settings.urlPreviewsUnencryptedDescription"
  | "settings.urlPreviewsEncrypted"
  | "settings.urlPreviewsEncryptedDescription"
  | "settings.urlPreviewsEncryptedNotice"
  | "settings.urlPreviewsEnabledForRoom"
  | "timeline.linkPreviewHide"
  | "timeline.linkPreviewFailed"
  | "timeline.linkPreviewLoading";

type MessageValues = Record<string, string | number>;
type Catalog = Record<MessageId, string>;
export type PseudoLocaleMode = "accented" | "bidi";
type ActivePseudoLocaleMode = PseudoLocaleMode | "none";

let activeLocale: Locale = "en";
let activePseudoLocale: ActivePseudoLocaleMode = "none";

export function getActiveLocale(): Locale {
  return activeLocale;
}

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
  "action.continue": "Continue",
  "action.createRoom": "Create room",
  "action.createSpace": "Create space",
  "action.done": "Done",
  "action.more": "More",
  "action.restartSync": "Restart sync",
  "action.recover": "Recover",
  "action.recovering": "Recovering",
  "action.report": "Report",
  "action.send": "Send",
  "action.sending": "Sending",
  "activity.highlightBadge": "Mention",
  "activity.loadMore": "Load more activity",
  "activity.loading": "Loading activity",
  "activity.resolvingUnread": "Resolving unread messages…",
  "activity.resolveFailed": "Unread messages could not be loaded.",
  "activity.retryResolution": "Retry",
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
  "auth.createAccount": "Create account",
  "auth.deviceName": "Device name",
  "auth.encryptionRecovery": "Encryption Recovery",
  "app.about": "About Koushi",
  "app.title": "Koushi",
  "app.versionMismatch.title": "Koushi needs to restart",
  "app.versionMismatch.detail":
    "Koushi couldn't load this session because its components are out of sync. Please fully quit and reopen the app.",
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
  "auth.loginFailureUsernameHint":
    "For @alice:matrix.org, enter alice here and keep matrix.org in Homeserver.",
  "auth.matrixAccount": "Matrix account",
  "auth.matrixDesktop": "Koushi",
  "auth.noLoginMethods": "No login methods",
  "auth.notChecked": "Not checked",
  "auth.password": "Password",
  "auth.recoveryKey": "Recovery key",
  "auth.recoverySecret": "Recovery key or security phrase",
  "auth.securityPhrase": "Security phrase",
  "auth.sessionLocked": "Session locked",
  "auth.signIn": "Sign in",
  "auth.supportedRecoveryMethods": "Supported recovery methods",
  "auth.username": "Username",
  "auth.usernameHelp": "Enter only the localpart. Do not include @ or the server name.",
  "auth.usernameOrMatrixId": "Username or Matrix ID",
  "auth.usernamePlaceholder": "alice",
  "composer.attachedFile": "Attached file",
  "composer.attachmentFallback": "Attachment",
  "composer.attachFile": "Attach file",
  "composer.attachFileInput": "Attach file input",
  "composer.bold": "Bold",
  "composer.code": "Code",
  "composer.dropFiles": "Drop files to attach",
  "composer.emoji": "Emoji",
  "composer.emojiSearch": "Search emoji",
  "composer.emojiRecent": "Frequently used",
  "composer.italic": "Italic",
  "composer.link": "Link",
  "composer.list": "List",
  "composer.mathMode": "Math",
  "composer.mathModeOff": "Math formatting off",
  "composer.mathModeOn": "Math formatting on",
  "composer.mention": "Mention",
  "composer.inlineMention": "Mention: {label}",
  "composer.mentionLoading": "Loading people…",
  "composer.mentionRoomNotification": "Room Notification",
  "composer.mentionRoomNotificationDescription": "Notify the whole room",
  "composer.mentionSuggestions": "Mention suggestions",
  "composer.mentionUsers": "Users",
  "composer.messageComposer": "Message composer",
  "composer.imageCompressionCompressed": "Compressed",
  "composer.imageCompressionOriginal": "Original",
  "composer.imageCompressionPreviewAlt": "Compressed image preview",
  "composer.imageCompressionSaveDefault": "Use this choice by default",
  "composer.imageCompressionTitle": "Compress image",
  "composer.placeholder": "Message {roomName}",
  "composer.replying": "Replying",
  "composer.slashCommandUnavailable": "This command is not available in this composer.",
  "composer.removeAttachment": "Remove attachment",
  "composer.cancelReply": "Cancel reply",
  "upload.captionForFile": "Caption for {filename}",
  "upload.ask": "Ask",
  "upload.clear": "Clear upload staging",
  "upload.compressed": "Compressed",
  "upload.dialogTitle": "Upload attachments",
  "upload.original": "Original",
  "upload.preparing": "Preparing attachment…",
  "upload.preparationFailed": "Attachment preparation failed",
  "upload.retryPreparation": "Retry preparation",
  "upload.useOriginal": "Use original",
  "upload.previewAlt": "Prepared attachment preview",
  "upload.savings": "{percent}% smaller",
  "upload.sizeChoice": "Upload size",
  "upload.resizeChoice": "Resize",
  "upload.formatChoice": "Format",
  "upload.resizeOriginal": "Original",
  "upload.resizeHalf": "1/2",
  "upload.resizeQuarter": "1/4",
  "upload.resizeEighth": "1/8",
  "upload.formatKeep": "Keep",
  "upload.formatWebp": "WebP",
  "upload.formatJpeg": "JPEG",
  "upload.formatPng": "PNG",
  "upload.outputSummary": "Upload result",
  "upload.recompressing": "Recompressing…",
  "upload.outputReady": "Ready",
  "upload.sendAttachments": "Send attachments",
  "window.title": "Koushi",
  "context.editMessage": "Edit",
  "context.addToFavourites": "Add to Favourites",
  "context.addToLowPriority": "Move to Low priority",
  "context.ignoreUser": "Ignore",
  "context.leaveSpace": "Leave Space",
  "context.leaveRoom": "Leave room…",
  "context.leaveConversation": "Leave conversation…",
  "room.leaveConfirmTitle": "Leave {name}?",
  "room.leaveConfirmTitleDm": "Leave conversation with {name}?",
  "room.leaveConfirmCopy":
    "This removes {name} from your joined rooms. Messages already on the homeserver are not deleted. If the room is private you may need another invitation to return.",
  "room.leaveConfirmCopyDm":
    "This removes your conversation with {name} from your joined rooms. Messages already on the homeserver are not deleted. You may need a new invitation to return.",
  "room.leaveConfirmAction": "Leave room",
  "room.leaveConfirmActionDm": "Leave conversation",
  "context.openKeyboardSettings": "Keyboard shortcuts",
  "context.openRoomInfo": "Room info",
  "context.openSpaceInfo": "Space info",
  "context.openThread": "Reply in thread",
  "context.openUserInfo": "User info",
  "context.openUserSettings": "User settings",
  "context.redactMessage": "Redact",
  "context.removeFromFavourites": "Remove from Favourites",
  "context.removeFromLowPriority": "Remove from Low priority",
  "context.searchInRoom": "Search in room",
  "context.selectRoom": "Open",
  "context.selectSpace": "Open Space",
  "context.switchAccount": "Switch account",
  "context.reportContent": "Report content",
  "context.reportRoom": "Report room",
  "context.reportUser": "Report user",
  "context.unignoreUser": "Unignore",
  "dialog.cancel": "Cancel",
  "dialog.cancelCreate": "Cancel create",
  "dialog.createRoomTitle": "Create room",
  "dialog.createSpaceTitle": "Create space",
  "dialog.encryptedRoom": "Encrypted room",
  "dialog.inviteCandidates": "Invite candidates",
  "dialog.inviteHistory": "History visibility for invitees",
  "dialog.inviteHistoryCurrent": "Current room setting. Change it in Room Info before sending the invite.",
  "dialog.inviteHistoryCurrentBadge": "Current",
  "dialog.inviteHistoryEncryptedShared": "Encrypted shared history may require sharing past room keys with invitees.",
  "dialog.inviteHistoryNonRetroactive": "This is not retroactive, and already shared events or keys cannot be revoked.",
  "dialog.inviteHistoryRecovery": "Recovery is incomplete; the invite can still proceed, but encrypted history sharing may be limited.",
  "dialog.inviteHistoryWorldReadableWarning":
    "Current advanced setting: non-members may read this room's history.",
  "dialog.inviteInvalidMatrixId": "Invalid Matrix ID",
  "dialog.invitePeopleTitle": "Invite people to {name}",
  "dialog.inviteScope": "Invite scope",
  "dialog.inviteSearch": "Name, alias, or Matrix ID",
  "dialog.inviteSelectedTargets": "Selected invite targets",
  "dialog.openRoomInfo": "Open Room Info",
  "dialog.matrixUserId": "Matrix user ID",
  "dialog.newDmTitle": "New DM",
  "dialog.privateRoom": "Private room",
  "dialog.publicRoom": "Public room",
  "dialog.roomAddress": "Room address",
  "dialog.roomTopic": "Topic",
  "dialog.roomVisibility": "Room visibility",
  "dialog.sendInvite": "Send invite",
  "dialog.removeInviteTarget": "Remove invite target",
  "dialog.roomName": "Room name",
  "dialog.spaceName": "Space name",
  "dialog.standardRoomInSpace": "Standard room in {spaceName}",
  "dialog.startDm": "Start DM",
  "dialog.reportReasonLabel": "Reason",
  "dialog.reportReasonPlaceholder": "Why are you reporting this?",
  "dialog.reportReasonTitle": "Report",
  "dialog.submitCreateRoom": "Submit create room",
  "dialog.submitCreateSpace": "Submit create space",
  "diagnostics.copied": "Diagnostics copied.",
  "diagnostics.copy": "Copy diagnostics",
  "diagnostics.copyFailed": "Could not copy diagnostics. Try again.",
  "diagnostics.copying": "Copying diagnostics…",
  "diagnostics.open": "Open diagnostics",
  "diagnostics.title": "Diagnostics",
  "emoji.category.people": "Smileys & People",
  "emoji.category.nature": "Animals & Nature",
  "emoji.category.foods": "Food & Drink",
  "emoji.category.activity": "Activities",
  "emoji.category.places": "Travel & Places",
  "emoji.category.objects": "Objects",
  "emoji.category.symbols": "Symbols",
  "emoji.category.flags": "Flags",
  "emoji.noResults": "No emojis match your search",
  "files.empty": "No files",
  "files.error": "Could not load files",
  "files.filterKinds": "File kinds",
  "files.filterPlaceholder": "Filter by filename",
  "files.kind.audio": "Audio",
  "files.kind.file": "File",
  "files.kind.image": "Image",
  "files.kind.sticker": "Sticker",
  "files.kind.video": "Video",
  "files.loading": "Loading files…",
  "files.sort.filename": "Filename",
  "files.sort.newestFirst": "Newest first",
  "files.sort.oldestFirst": "Oldest first",
  "files.sort.sender": "Sender",
  "files.sortLabel": "Sort by",
  "files.title": "Files",
  "help.learnMore": "Learn more",
  "help.userTrust.deviceStateBody":
    "Device state describes whether the device is cross-signed by its owner. This is separate from whether you have verified that user.",
  "help.userTrust.deviceStateTitle": "Device state",
  "help.userTrust.effectiveTrustBody":
    "Effective trust is Koushi's send decision after combining user trust, device state, blocked devices, and identity-reset warnings.",
  "help.userTrust.effectiveTrustTitle": "Effective trust",
  "help.userTrust.explain": "Explain user trust",
  "help.userTrust.identityResetBody":
    "This user was verified before, but their current identity is different. Verify again, or forget the previous verification and treat them as unverified.",
  "help.userTrust.identityResetTitle": "Identity reset",
  "help.userTrust.title": "User trust model",
  "help.userTrust.unverifiedBody":
    "Unverified is the normal state for people you have not checked through another channel. Encrypted messages can still be sent.",
  "help.userTrust.unverifiedTitle": "Unverified",
  "help.userTrust.verifiedBody":
    "Verified means you checked this user's identity through QR, emoji, or SAS over another channel. Use it when stronger assurance matters.",
  "help.userTrust.verifiedTitle": "Verified",
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
  "directory.previewAlreadyJoined": "You are already in this room",
  "directory.previewCancel": "Cancel",
  "directory.previewFailed": "Could not load this room: {reason}",
  "directory.previewInviteOnly": "Invite only",
  "directory.previewLoading": "Loading room details",
  "directory.previewOpenRoom": "Open",
  "directory.previewRestricted": "Members of another room only",
  "directory.previewTitle": "Join this room?",
  "directory.previewUnknownJoinRule": "The server did not report who may join",
  "directory.noResults": "No public rooms found",
  "directory.results": "Public room results",
  "directory.search": "Search",
  "directory.joinSectionTitle": "Join a room or space",
  "directory.addressLabel": "Matrix address or link",
  "directory.addressPlaceholder": "#room:server or https://matrix.to/#/...",
  "directory.addressHelper": "Room aliases, room IDs, and matrix.to links are supported.",
  "directory.preview": "Preview",
  "directory.addressIsUser": "This is a user ID. Start a DM from New DM instead.",
  "directory.addressNotRecognized": "This does not look like a Matrix address or link.",
  "directory.searchSectionTitle": "Search public rooms and spaces",
  "directory.searchTermLabel": "Search term",
  "directory.searchServerHelper": "Searches only the public directory of the selected server.",
  "directory.roomBadge": "Room",
  "directory.searchFailed": "Search failed: {reason}",
  "directory.searching": "Searching",
  "directory.searchPlaceholder": "Room name, topic, or a room link",
  "directory.searchServer": "Search on server",
  "directory.searchServerPlaceholder": "Your server",
  "directory.spaceBadge": "Space",
  "directory.unnamedRoom": "Unnamed room",
  "directory.unnamedSpace": "Unnamed space",
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
  "mediaGallery.empty": "No media in this room",
  "mediaGallery.next": "Next media",
  "mediaGallery.noPreview": "No preview",
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
  "panel.people": "People",
  "panel.profile": "Profile",
  "people.memberCount": "{count} members",
  "people.memberActions": "Member actions",
  "people.openProfile": "Open profile for {name}",
  "people.searchMembers": "Search room members",
  "people.noSearchResults": "No members match your search.",
  "people.you": "You",
  "people.sendMessage": "Send message",
  "people.setAlias": "Set alias",
  "people.unknownUser": "Unknown user",
  "room.avatarUrl": "Room avatar URL",
  "room.ban": "Ban",
  "room.banMember": "Ban {name}",
  "room.currentAvatar": "Current avatar",
  "room.currentTopic": "Current topic",
  "room.copyShareLink": "Copy room link",
  "room.members": "Members",
  "room.directMessage": "Direct message",
  "room.accessAndHistory": "Access and history",
  "room.accessAndHistoryHint": "Join rules decide who may enter; history visibility decides what they can see. These settings are independent.",
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
  "room.historyInvitedDescription": "People can see messages sent after they are invited.",
  "room.historyJoinedDescription": "People can see messages sent after they join.",
  "room.historySharedDescription": "People can see the room history that is shared with members.",
  "room.historySharedEncryptedHint": "In an encrypted room, inviting can share past room keys so eligible invitees can decrypt the shared history.",
  "room.historyNonRetroactive": "Changing this does not rewrite the past or revoke events and keys already shared.",
  "room.historyRecoveryRequired": "Device recovery is incomplete. Inviting can continue, but encrypted history sharing may be limited.",
  "room.historyWorldReadableDescription": "Anyone who can discover the room may read its history, including people who are not members.",
  "room.historyWorldReadableWarning": "Advanced setting: non-members may read the room history.",
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
  "room.messageMember": "Message {name}",
  "room.memberRole": "Member role",
  "room.memberRoleFor": "Member role for {name}",
  "room.aliasDialogTitle": "Alias for {name}",
  "room.aliasInput": "Alias",
  "room.clearAlias": "Clear alias",
  "room.clearAliasForMember": "Clear alias for {name}",
  "room.editAlias": "Edit alias",
  "room.editAliasForMember": "Edit alias for {name}",
  "room.memberOriginalName": "Original: {name}",
  "room.setAlias": "Set alias",
  "room.setAliasForMember": "Set alias for {name}",
  "room.noAvatar": "No avatar",
  "room.noMembers": "No members loaded",
  "room.noTopic": "No topic",
  "room.noRoomSelected": "No room selected",
  "room.noSpaces": "No Spaces",
  "room.notifications": "Notifications",
  "room.notifyModeAll": "All messages",
  "room.notifyModeMentions": "Mentions only",
  "room.notifyModeMute": "Mute",
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
  "room.repair": "Repair",
  "room.repairTimeline": "Repair room timeline",
  "room.repairTimelineHint": "Detect and repair missing timeline ranges without deleting existing messages.",
  "room.status": "Room status",
  "room.statusEncrypted": "Encrypted",
  "room.statusHistoryLimited": "New members do not see history",
  "room.statusHistoryShared": "New members see history",
  "room.statusHistoryWorldReadable": "Anyone can see history",
  "room.statusNotEncrypted": "Not encrypted",
  "room.statusPrivate": "Private",
  "room.statusPublic": "Public",
  "room.reshareRoomKeys": "Reshare room keys",
  "room.reshareRoomKeysHint": "Send this room's known decryption keys again to eligible, unblocked devices.",
  "room.reshareRoomKeysPending": "Resharing keys…",
  "room.reshareRoomKeysSuccess": "Room keys were resent.",
  "room.reshareRoomKeysNoSession": "No active room key is available to reshare.",
  "room.reshareRoomKeysNoRecipients": "No eligible devices need this room key.",
  "room.reshareRoomKeysStaleSession": "The active room key changed. Try again.",
  "room.reshareRoomKeysError": "Could not reshare room keys.",
  "room.dangerousEncryptionDebugging": "Dangerous encryption debugging",
  "room.dangerousEncryptionDebuggingWarning":
    "Temporary diagnostic controls. Rotation affects subsequent encrypted messages; key sharing cannot prove that a recipient received or stored the key.",
  "room.forceNewEncryptionSession": "Force new encryption session",
  "room.shareIndex0Key": "Share index-0 key with room recipients",
  "room.resendIndex0Key": "Resend index-0 recovery key",
  "room.forceNewEncryptionSessionConfirm":
    "This discards the current encryption session and creates a fresh one at message index 0. Repeated clicks may create additional sessions. Continue?",
  "room.shareIndex0KeyConfirm":
    "This shares the index-0 key of the current session with every eligible recipient device. It does not prove recipient receipt or storage. If the session has advanced past index 0, force a new session first. Continue?",
  "room.resendIndex0KeyConfirm":
    "This permanently grants decryption capability for this session to devices from the original share ledger. Shared keys cannot be revoked. This temporary control is for diagnosis only. Continue?",
  "room.confirm": "Confirm",
  "room.cancel": "Cancel",
  "room.encryptionDebugOutcomeCompleted": "Operation completed.",
  "room.encryptionDebugOutcomeRefusedNotEncrypted": "This room is not encrypted.",
  "room.encryptionDebugOutcomeRefusedIndexAdvanced":
    "The session has advanced past index 0. Force a new encryption session first.",
  "room.encryptionDebugOutcomeNoSession": "No active outbound session is available.",
  "room.encryptionDebugOutcomeNoRecipients": "No original recipient devices are eligible.",
  "room.encryptionDebugOutcomeInboundSessionMissing": "The matching inbound session is missing.",
  "room.encryptionDebugOutcomeInboundIndexAdvanced": "The inbound session cannot export index 0.",
  "room.encryptionDebugOutcomeOriginalLedgerMissing": "The original sharing proof is unavailable.",
  "room.encryptionDebugOutcomeStaleIdentityRefused": "A recipient device identity changed; sharing was refused.",
  "room.encryptionDebugOutcomeCancelledStale":
    "The operation was cancelled because the session or room changed.",
  "room.encryptionDebugOutcomePolicyBlocked":
    "Recipient policy blocked the share.",
  "room.encryptionDebugOutcomeDeadline": "The operation timed out.",
  "room.encryptionDebugOutcomeFailed": "The operation failed.",
  "room.saveAccess": "Save access",
  "room.saveHistoryVisibility": "Save history visibility",
  "room.saveJoinRule": "Save join rule",
  "room.saveAvatar": "Save avatar",
  "room.saveName": "Save room name",
  "room.saveTopic": "Save topic",
  "room.searchIndex": "Search index",
  "room.settingsLoading": "Room settings loading",
  "room.spaces": "Spaces",
  "room.subscribed": "Subscribed",
  "room.summary": "Room summary",
  "room.tabs": "Room tabs",
  "room.rightPanelToggle": "Toggle right panel",
  "room.returnToInvite": "Return to invite",
  "room.timeline": "Timeline",
  "room.topic": "Room topic",
  "room.type": "Type",
  "room.unban": "Unban",
  "room.unbanMember": "Unban {name}",
  "room.unread": "Unread",
  "room.unreadCount": "{count} unread",
  "roomList.category": "Room list category",
  "roomList.categoryDms": "DMs",
  "roomList.categoryRooms": "Rooms",
  "roomList.categorySummary": "{category}, {unread} unread, {total} total",
  "roomList.categorySummaryWithHighlights":
    "{category}, {unread} unread, {total} total, {highlights} mentions",
  "roomList.filterRooms": "Rooms",
  "roomList.filterUnread": "Unread",
  "roomList.filterPeople": "Direct Messages",
  "roomList.filterFavourites": "Favourites",
  "roomList.filterInvites": "Invites",
  "roomList.filterDmsPlaceholder": "Filter direct messages",
  "roomList.filterRoomsPlaceholder": "Filter rooms",
  "roomList.clearFilter": "Clear room list filter",
  "roomList.noMatchingDms": "No matching direct messages",
  "roomList.noMatchingRooms": "No matching rooms",
  "roomList.sort": "Room list sort",
  "roomList.sortLabel": "Sort",
  "roomList.sortActive": "Active",
  "roomList.sortName": "Name",
  "roomList.loading": "Loading rooms…",
  "roomList.failed": "Rooms could not be loaded",
  "room.markAsRead": "Mark as read",
  "room.markAsUnread": "Mark as unread",
  "search.indexingPending": "Indexing message history. More matches may appear shortly.",
  "search.noExactMatches": "No exact matches",
  "search.matchAttachmentFileName": "attachment filename",
  "search.matchMessage": "message",
  "search.resultCountMany": "{count} results for \"{query}\"",
  "search.resultCountOne": "1 result for \"{query}\"",
  "search.searching": "Searching...",
  "search.searchingFor": "Searching for \"{query}\"",
  "search.tooShort": "Search term is too short",
  "search.scopeAll": "All",
  "search.scopeRoom": "Room/DM",
  "search.scopeSpace": "Space",
  "sessionStatus.authentication": "Authentication",
  "sessionStatus.authOauth": "OAuth",
  "sessionStatus.authPassword": "Password",
  "sessionStatus.authSso": "SSO",
  "sessionStatus.authToken": "Access token",
  "sessionStatus.backupDisabled": "Disabled",
  "sessionStatus.backupReady": "Ready",
  "sessionStatus.checking": "Checking",
  "sessionStatus.copyDeviceId": "Copy Device ID",
  "sessionStatus.crossSigned": "Cross-signed",
  "sessionStatus.deviceId": "Device ID",
  "sessionStatus.deviceName": "Device name",
  "sessionStatus.failed": "Check failed",
  "sessionStatus.failureSdk": "Session check failed",
  "sessionStatus.failureTimedOut": "Session check timed out",
  "sessionStatus.failureUnavailable": "Session status is unavailable",
  "sessionStatus.homeserver": "Homeserver",
  "sessionStatus.identity": "Own identity",
  "sessionStatus.identityMissing": "Identity missing",
  "sessionStatus.identityUnverified": "Identity unverified",
  "sessionStatus.identityVerified": "Identity verified",
  "sessionStatus.keyBackup": "Key backup",
  "sessionStatus.lastChecked": "Last checked",
  "sessionStatus.manageAccount": "Manage account and devices",
  "sessionStatus.notChecked": "Not checked",
  "sessionStatus.notCrossSigned": "Not cross-signed",
  "sessionStatus.open": "Open session status",
  "sessionStatus.openWithRuntimeWarning": "Open session status, {count} runtime warning",
  "sessionStatus.openWithRuntimeWarnings": "Open session status, {count} runtime warnings",
  "sessionStatus.ownerCrossSigning": "Owner cross-signing",
  "sessionStatus.recheck": "Recheck",
  "sessionStatus.retry": "Retry",
  "sessionStatus.runtimeAlertSecureBackup": "Secure Backup unavailable",
  "sessionStatus.runtimeWarningCount": "{count} runtime warning",
  "sessionStatus.runtimeWarnings": "Runtime warnings",
  "sessionStatus.runtimeWarningsCount": "{count} runtime warnings",
  "sessionStatus.sync": "Sync",
  "sessionStatus.syncError": "Error",
  "sessionStatus.syncRunning": "Running",
  "sessionStatus.syncStarting": "Starting",
  "sessionStatus.syncStopped": "Stopped",
  "sessionStatus.title": "Current session",
  "sessionStatus.unavailable": "Unavailable",
  "sessionStatus.unverified": "Unverified",
  "sessionStatus.unknown": "Unknown",
  "sessionStatus.userId": "User ID",
  "sessionStatus.verification": "Verification",
  "sessionStatus.verified": "Verified",
  "settings.accounts": "Accounts",
  "settings.appearance": "Appearance",
  "settings.accountSwitcher": "Account switcher",
  "settings.current": "Current",
  "settings.autoLoadOlderMessages": "Automatically load older messages",
  "settings.autoLoadOlderMessagesDescription": "Prefetch room history when scrolling near the start of the loaded timeline",
  "settings.threadRootLatestReply": "Place threaded conversations at their latest reply",
  "settings.threadRootLatestReplyDescription": "Move each root message and its reply summary to the time of its latest reply",
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
  "settings.timeline": "Timeline",
  "settings.display": "Display",
  "settings.displayDensity": "Display density",
  "settings.densityCompact": "Compact",
  "settings.densityDefault": "Default",
  "settings.densityComfortable": "Comfortable",
  "settings.codeBlockWrap": "Wrap long lines in code blocks",
  "settings.hideRedacted": "Hide deleted messages",
  "settings.urlPreviews": "URL previews",
  "settings.urlPreviewsEnabled": "Show link previews",
  "settings.urlPreviewsDescription": "Load previews for links in messages",
  "settings.urlPreviewsUnencrypted": "URL previews in non-encrypted rooms",
  "settings.urlPreviewsUnencryptedDescription": "Load previews for links in non-encrypted rooms by default",
  "settings.urlPreviewsEncrypted": "URL previews in encrypted rooms",
  "settings.urlPreviewsEncryptedDescription": "Opt in to loading previews for links sent in encrypted rooms",
  "settings.urlPreviewsEncryptedNotice": "Encrypted-room previews can reveal URLs to the homeserver and destination site.",
  "settings.urlPreviewsEnabledForRoom": "Enable link previews for this room",
  "timeline.linkPreviewHide": "Hide preview",
  "timeline.linkPreviewFailed": "Could not load preview",
  "timeline.linkPreviewLoading": "Loading preview…",
  "settings.notificationBadges": "Badges",
  "settings.notificationDesktop": "Desktop notifications",
  "settings.notificationSound": "Sound",
  "settings.sendReadReceipts": "Send read receipts",
  "settings.sendTypingNotifications": "Send typing notifications",
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
  "settings.messagingPrivacy": "Messaging & privacy",
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
  "settings.accountManagement": "Account management",
  "settings.manageAccount": "Manage account & devices",
  "settings.manageAccountHint": "Opens this server's account and device management page in your browser.",
  "settings.changePassword": "Change password",
  "settings.changePasswordLabel": "New password",
  "settings.changePasswordConfirm": "Confirm new password",
  "settings.changePasswordMismatch": "Passwords do not match",
  "settings.passwordChanged": "Password changed",
  "settings.deactivateAccount": "Deactivate account",
  "settings.deactivateAccountErase": "Erase all data",
  "settings.deactivateAccountConfirm": "This cannot be undone. Confirm to proceed.",
  "settings.accountDeactivated": "Account deactivated",
  "settings.accountManagementFailed": "Account operation failed",
  "settings.signOut": "Sign out",
  "settings.keyManagement": "Key management",
  "settings.roomKeyExport": "Room key export",
  "settings.roomKeyExportDestination": "Key export destination",
  "settings.roomKeyExportIdle": "Not exported",
  "settings.roomKeyExporting": "Exporting",
  "settings.roomKeyExportedUnknown": "Exported",
  "settings.roomKeyExportedCount": "{count} sessions exported",
  "settings.roomKeyExportFailed": "Export failed: {reason}",
  "settings.exportRoomKeys": "Export room keys",
  "settings.chooseRoomKeyExportFile": "Choose export file",
  "settings.roomKeyImport": "Room key import",
  "settings.roomKeyImportSource": "Key import source",
  "settings.roomKeyImportIdle": "Not imported",
  "settings.roomKeyImporting": "Importing",
  "settings.roomKeyImportedCount": "{imported} of {total} imported",
  "settings.roomKeyImportFailed": "Import failed: {reason}",
  "settings.importRoomKeys": "Import room keys",
  "settings.chooseRoomKeyImportFile": "Choose import file",
  "settings.roomKeyPassphrasePromptExport": "Enter a passphrase for the room key export.",
  "settings.roomKeyPassphrasePromptImport": "Enter the passphrase for this room key file.",
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
  "settings.resetLocalDataConfirm": "Reset local data for this session? This removes the local Matrix store, cached history, and saved credentials on this device. This cannot be undone.",
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
  "scheduled.bodyInput": "Scheduled message",
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
  "trust.cancelIdentityReset": "Cancel identity reset",
  "trust.continueIdentityReset": "Continue",
  "trust.crossSigning": "Cross-signing",
  "trust.declineVerification": "Decline",
  "trust.deviceBlocked": "Blocked",
  "trust.deviceCount": "{count} devices",
  "trust.deviceOrdinal": "Device {index}",
  "trust.deviceUnknown": "Unknown",
  "trust.deviceUnverified": "Unverified",
  "trust.deviceVerified": "Verified",
  "trust.deviceCrossSigned": "Cross-signed",
  "trust.deviceNotCrossSigned": "Not cross-signed",
  "trust.devices": "Devices",
  "trust.enableKeyBackup": "Enable",
  "trust.encryption": "Encryption",
  "trust.failureCancelled": "Cancelled",
  "trust.failureForbidden": "Forbidden",
  "trust.failureInvalidPassphrase": "Invalid passphrase",
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
  "trust.userIdentityReset": "Identity reset",
  "trust.userUnverified": "Unverified",
  "trust.userVerified": "Verified",
  "trust.verification": "Device verification",
  "gate.title": "Verify this session",
  "gate.sessionExpired": "Session expired",
  "gate.sessionExpiredCopy": "This session has expired or was revoked. Sign in again to continue.",
  "gate.verifying": "Verifying this session…",
  "gate.finishing": "Finishing sign-in…",
  "gate.preparing": "Preparing your rooms…",
  "gate.checking": "Checking device trust…",
  "gate.discovering": "Discovering verification methods…",
  "gate.rejecting": "Rejecting unverified session…",
  "gate.locked": "This session must be verified again.",
  "gate.otherDevice": "Verify with another device",
  "gate.verifyRecoveryKey": "Verify with recovery key",
  "gate.noRecoveryKeyTitle": "No recovery key available",
  "gate.noRecoveryKeyCopy":
    "Koushi cannot verify this session without recovery material. Use a Matrix client that is already verified to set up recovery, or reset your cryptographic identity if recovery is genuinely unavailable. This device stays signed in and may appear unverified to other clients until then.",
  "gate.deviceVerificationDialogTitle": "Try device verification?",
  "gate.deviceVerificationDialogCopy": "Device-to-device verification can be unreliable if the other device is offline, slow to sync, or missing keys. We recommend verifying with your recovery key when possible.",
  "gate.useRecoveryKey": "Use recovery key",
  "gate.tryDeviceVerificationAnyway": "Try device verification anyway",
  "gate.match": "They match",
  "gate.mismatch": "They do not match",
  "gate.recoverySecret": "Recovery secret",
  "gate.recover": "Recover",
  "gate.destination": "Recovery key destination",
  "gate.passphrase": "Backup passphrase",
  "gate.bootstrap": "Create secure backup",
  "gate.saved": "I saved the recovery key",
  "gate.retry": "Retry",
  "gate.cleanupOffer": "Cancel sign-in and remove this device…",
  "gate.cleanupDialogTitle": "Cancel sign-in and remove this device",
  "gate.cleanupDialogCopy": "Koushi will remove this device from your Matrix account first, then erase local messages, encryption keys, caches, and saved credentials on this computer. Messages on your homeserver are preserved. The next sign-in creates a new Device ID. This cannot be undone.",
  "gate.cleanupConfirm": "Remove device and erase local data",
  "gate.cleanupResolving": "Checking how this device must be removed…",
  "gate.cleanupRemoving": "Removing this device from your Matrix account…",
  "gate.cleanupAccountPassword": "Account password",
  "gate.cleanupContinue": "Continue device removal",
  "gate.cleanupRemoteFailed": "The device could not be removed from your Matrix account. Your credentials and local data are still preserved.",
  "gate.cleanupRetryRemote": "Retry removing device",
  "gate.cleanupEraseAnywayOffer": "Erase local data anyway…",
  "gate.cleanupEraseAnywayTitle": "Erase local data anyway",
  "gate.cleanupEraseAnywayCopy": "The device may remain active on your Matrix account. Erasing now removes local messages, encryption keys, and saved credentials from this computer and cannot be undone.",
  "gate.cleanupEraseAnywayConfirm": "Erase local data anyway",
  "gate.cleanupResettingLocal": "Device removed. Erasing local data…",
  "gate.cleanupErasingLocal": "Erasing local data while the remote device may remain…",
  "gate.cleanupLocalFailed": "The local data could not be erased. The cleanup can be retried safely.",
  "gate.cleanupRetryLocal": "Retry erasing local data",
  "gate.cleanupCommandFailed": "Device cleanup failed. Please try again.",
  "gate.resetForNewDevice": "Reset local data and sign in as a new device",
  "gate.resetForNewDeviceConfirm": "Reset this session's local data and sign in again with a new device ID? This removes the local encrypted store, cached history, and saved credentials on this computer. It does not remove messages from the homeserver. This cannot be undone.",
  "gate.signOut": "Sign out",
  "gate.secureBackupTitle": "Secure backup required",
  "gate.secureBackupChecking": "Checking secure backup…",
  "gate.secureBackupInactive": "Secure backup status is unavailable. Encrypted messaging stays locked until it can be checked.",
  "gate.secureBackupNeedsRecovery": "Recover your existing secure backup to continue.",
  "gate.secureBackupRecoveryKey": "Secure backup recovery key",
  "gate.secureBackupRecover": "Recover secure backup",
  "gate.secureBackupSetupTitle": "Set up secure backup",
  "gate.secureBackupSetupCopy": "Create and protect a Secure Backup before encrypted messaging is available. Save the recovery key to a secure destination.",
  "gate.secureBackupPassphrase": "Secure backup passphrase",
  "gate.secureBackupRecoveryKeyDestination": "Recovery key destination",
  "gate.secureBackupChooseDestination": "Choose recovery key destination",
  "gate.secureBackupDestinationSelected": "Recovery key destination selected.",
  "gate.secureBackupDestinationNotSelected": "No recovery key destination selected.",
  "gate.secureBackupDestinationSelectionFailed": "Could not open the recovery key destination picker. Try again.",
  "gate.secureBackupSetup": "Set up secure backup",
  "gate.secureBackupExplicitDisabledTitle": "Secure backup was disabled",
  "gate.secureBackupExplicitDisabledCopy": "Re-enabling Secure Backup changes this account-wide setting and affects other Matrix clients signed in to this account. Confirm only if you want those clients to use the updated backup.",
  "gate.secureBackupReenable": "Re-enable secure backup",
  "gate.secureBackupReenableConfirm": "Confirm re-enable",
  "gate.secureBackupCreating": "Creating secure backup…",
  "gate.secureBackupDeliveryRequired": "Save the recovery key before continuing.",
  "gate.secureBackupUploading": "Uploading existing encrypted keys…",
  "gate.secureBackupPendingZero": "No encrypted keys remain to upload.",
  "gate.secureBackupPendingOne": "Uploading existing encrypted keys: 1 remaining.",
  "gate.secureBackupPendingTwoToTen": "Uploading existing encrypted keys: 2–10 remaining.",
  "gate.secureBackupPendingElevenToOneHundred": "Uploading existing encrypted keys: 11–100 remaining.",
  "gate.secureBackupPendingOverOneHundred": "Uploading existing encrypted keys: more than 100 remaining.",
  "gate.secureBackupPendingUnknown": "Uploading existing encrypted keys: remaining count unavailable.",
  "gate.secureBackupRetrying": "Retrying secure backup…",
  "gate.secureBackupFailureNetwork": "Secure backup could not reach the server.",
  "gate.secureBackupFailureRateLimited": "Secure backup requests are being limited. Try again later.",
  "gate.secureBackupFailureInvalidRecoveryKey": "That recovery key could not unlock the existing secure backup.",
  "gate.secureBackupFailureBackupKeyMismatch": "The local backup key does not match the existing secure backup.",
  "gate.secureBackupFailureSecretStorageIncomplete": "This device's secure storage is incomplete.",
  "gate.secureBackupFailureArtifactDelivery": "The recovery key could not be delivered to the selected destination.",
  "gate.secureBackupFailureForbidden": "This account is not allowed to use secure backup.",
  "gate.secureBackupFailureTimeout": "Secure backup took too long to respond.",
  "gate.secureBackupFailureSdk": "Secure backup failed.",
  "gate.secureBackupRetry": "Retry secure backup",
  "gate.secureBackupDiagnostics": "Open secure backup diagnostics",
  "gate.secureBackupCommandFailed": "Secure backup action failed. Try again.",
  "gate.secureBackupRuntimeDegraded": "Secure Backup is temporarily degraded. Sending, drafts, receiving, and local decryption remain available while Koushi retries.",
  "slidingSync.blockedTitle": "Simplified Sliding Sync is required",
  "slidingSync.unsupported": "This homeserver does not support Simplified Sliding Sync.",
  "slidingSync.unreachable": "Koushi could not reach this homeserver to verify Simplified Sliding Sync support.",
  "slidingSync.invalidResponse": "This homeserver returned an invalid Simplified Sliding Sync capability response.",
  "slidingSync.changeHomeserver": "Change homeserver",
  "gate.failureCancelled": "Verification was cancelled.",
  "gate.failureForbidden": "Verification was not permitted.",
  "gate.failureNoProof": "No proof method is available.",
  "gate.failureSdk": "Verification failed.",
  "gate.verificationCommandFailed": "Verification command failed. Please try again.",
  "gate.rejectNoProof": "This existing identity has no acceptable proof method.",
  "gate.rejectUser": "This session was rejected.",
  "space.allRooms": "All rooms",
  "space.childRooms": "Child rooms",
  "space.directMessages": "Direct Messages",
  "space.home": "Home",
  "space.invite": "Invite",
  "space.localIcon": "Local icon",
  "space.localIconPlaceholder": "Emoji or short text",
  "space.localName": "Local name",
  "space.localNamePlaceholder": "Space label on this device",
  "space.localPresentation": "Local presentation",
  "space.resetLocalPresentation": "Reset local presentation",
  "space.noUnread": "No unread",
  "space.preferences": "Preferences",
  "space.roomMembership": "Room membership",
  "space.spacePreferences": "Space preferences",
  "space.spaceSettings": "Space settings",
  "space.summary": "Space summary",
  "sync.failed": "Failed",
  "sync.failedWithReason": "Sync failed: {reason}",
  "sync.reasonAuth": "Sign-in required",
  "sync.reasonHttp": "Network issue",
  "sync.reasonInternal": "Internal error",
  "sync.reasonNetworkError": "Network issue",
  "sync.reasonNetworkOffline": "Network offline",
  "sync.reasonStore": "Local store issue",
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
  "shortcut.toggleFullscreen": "Toggle fullscreen",
  "shortcut.toggleMicrophone": "Toggle microphone in call",
  "shortcut.toggleRightPanel": "Toggle right panel",
  "shortcut.toggleSpacePanel": "Toggle space panel",
  "shortcut.uploadFile": "Upload file",
  "timeline.conversation": "Conversation timeline",
  "timeline.conversationStart": "Start of conversation",
  "timeline.notice.roomCreate": "created the room",
  "timeline.notice.roomPowerLevels": "updated room permissions",
  "timeline.notice.roomGuestAccess": "updated guest access",
  "timeline.notice.roomEncryption": "enabled room encryption",
  "timeline.notice.spaceParent": "updated the parent space",
  "timeline.notice.roomJoinRules": "updated join rules",
  "timeline.notice.roomHistoryVisibility": "updated history visibility",
  "timeline.notice.roomPinnedEvents": "updated pinned messages",
  "timeline.notice.roomNameSet": "set the room name to {newName}",
  "timeline.notice.roomNameChanged": "changed the room name from {oldName} to {newName}",
  "timeline.notice.roomNameRemoved": "removed the room name",
  "timeline.notice.roomNameChangedGeneric": "changed the room name",
  "timeline.editedMessage": "Edited",
  "timeline.editMessage": "Edit message",
  "timeline.editBody": "Edit message body",
  "timeline.loading": "Loading",
  "timeline.gapRepairing": "Repairing missing messages…",
  "timeline.gapRepairFailed": "Some messages are still missing.",
  "timeline.threadRootLoading": "Loading thread message…",
  "timeline.threadRootUnavailable": "Thread message is unavailable.",
  "timeline.messagesTab": "Messages",
  "timeline.openingThread": "Opening thread",
  "timeline.presenceAway": "Away",
  "timeline.presenceOffline": "Offline",
  "timeline.presenceOnline": "Online",
  "timeline.addReaction": "Add reaction",
  "timeline.reactionSummary": "Reaction {key}, count {count}",
  "timeline.reactionPicker": "Choose reaction",
  "timeline.reactionOption": "React with {emoji}",
  "timeline.reactionTooltip": "{names} reacted with {key}",
  "timeline.reactionSenderOverflow": "{count} more",
  "timeline.reactionSenderUnknown": "{count} people",
  "timeline.readBy": "Read by {count}",
  "timeline.readReceiptOverflow": "{count} more",
  "timeline.readMarker": "Read up to here",
  "timeline.readStateSyncing": "Syncing read state",
  "timeline.readStateNotSynced": "Read state not synced",
  "timeline.readStateReasonAuthentication": "authentication required",
  "timeline.readStateReasonRateLimited": "rate limited",
  "timeline.readStateReasonTimeout": "timed out",
  "timeline.readStateReasonTransport": "connection failed",
  "timeline.readStateReasonServer": "server failed",
  "timeline.readStateReasonCapacity": "too many pending read updates",
  "timeline.readStateReasonSdk": "local client error",
  "timeline.unreadMarker": "Unread messages",
  "timeline.jumpToFirstUnread": "Jump to first unread, {count} unread",
  "timeline.jumpToBottom": "Jump to bottom, {count} new messages",
  "timeline.olderMessages": "Older messages",
  "timeline.latest": "Latest",
  "timeline.navigation": "Timeline navigation",
  "timeline.saveEdit": "Save edit",
  "timeline.cancelEdit": "Cancel edit",
  "timeline.redactMessage": "Redact message",
  "timeline.redactedMessage": "Message redacted",
  "timeline.revealSpoiler": "Reveal spoiler",
  "timeline.pinMessage": "Pin message",
  "timeline.spoiler": "Spoiler",
  "timeline.unpinMessage": "Unpin message",
  "timeline.pinnedMessages": "Pinned messages",
  "timeline.pinnedMessagesCount": "Pinned · {count}",
  "timeline.pinnedMessage": "Pinned message",
  "timeline.pinnedMessagesEmpty": "No pinned messages",
  "timeline.pinnedEventUnableToDecrypt": "Unable to decrypt this message",
  "timeline.pinnedEventUnavailable": "Pinned message unavailable",
  "timeline.pinnedNavigationLoading": "Opening pinned message…",
  "timeline.pinnedNavigationFailed": "Could not open pinned message",
  "timeline.pinnedNavigationRetry": "Retry",
  "timeline.replyQuoteMissing": "Original message unavailable",
  "timeline.replyQuoteUnavailable": "Original message unavailable",
  "timeline.replyQuoteUnknownSender": "Unknown sender",
  "timeline.replyQuoteUnsupported": "Unsupported message",
  "timeline.replyToMessage": "Reply to message",
  "timeline.replyInThread": "Reply in thread",
  "timeline.messageActions": "Message actions",
  "timeline.copyMessage": "Copy message",
  "timeline.copyCode": "Copy code",
  "timeline.copyPermalink": "Copy permalink",
  "timeline.viewSource": "View source",
  "timeline.removeMessage": "Remove",
  "timeline.requestRoomKey": "Request keys and retry",
  "timeline.keyRequestToast": "Decryption key requested",
  "timeline.keyRequestAwaiting": "Waiting for the decryption key…",
  "timeline.keyRequestStillWaiting": "No response yet. Another device may be offline; it may decrypt later.",
  "timeline.keyRequestWithheld": "The decryption key could not be obtained.",
  "timeline.keyRequestUnavailable": "The requested device does not have this decryption key.",
  "timeline.keyRequestUnauthorised": "Sharing the decryption key was not permitted.",
  "timeline.keyRequestUnverified": "This device is unverified, so the key was not shared.",
  "timeline.keyRequestBlacklisted": "This device is excluded from key sharing.",
  "timeline.keyRequestRecovered": "Decryption key received",
  "timeline.recoveryCheckingLocal": "Checking this device for the key…",
  "timeline.recoveryCheckingBackup": "Checking your Secure Backup…",
  "timeline.recoveryRequestingDevices": "Requesting the key from your verified devices…",
  "timeline.recoveryRepairingOlm": "Repairing encryption sessions…",
  "timeline.recoveryWaitingForKey": "Waiting for the key…",
  "timeline.recoveryRetrying": "Retrying decryption…",
  "timeline.recoveryRecovered": "Recovered",
  "timeline.recoveryTemporarilyFailed": "Recovery will retry automatically.",
  "timeline.recoveryExhausted": "Automatic recovery could not get this key.",
  "timeline.recoveryUnrecoverable": "This key cannot be recovered.",
  "timeline.recoveryGuidanceAnotherDevice": "Open another verified device and retry.",
  "timeline.recoveryGuidanceBackup": "Secure Backup is unavailable or incomplete. Check your backup status and retry.",
  "timeline.recoveryGuidanceSenderMessage": "Ask the sender to send a new message in this room, then retry.",
  "timeline.recoveryGuidanceRepost": "Ask the sender to repost the unreadable content; the original message cannot be restored without its key.",
  "timeline.forwardMessage": "Forward",
  "timeline.messageSource": "Message source",
  "timeline.closeMessageSource": "Close message source",
  "timeline.sourceEventId": "Event ID:",
  "timeline.copyEventId": "Copy event ID",
  "timeline.encryptionDetails": "Encryption details",
  "timeline.megolmSessionFingerprint": "Megolm session fingerprint",
  "timeline.copyMegolmSessionFingerprint": "Copy Megolm session fingerprint",
  "timeline.megolmRotationReason": "Session change reason",
  "timeline.megolmReasonInitial": "Initial session",
  "timeline.megolmReasonExpiredTime": "Time limit reached",
  "timeline.megolmReasonExpiredMessageCount": "Message limit reached",
  "timeline.megolmReasonMembershipOrDeviceChange": "Membership or device change",
  "timeline.megolmReasonEncryptionSettingsChanged": "Encryption settings changed",
  "timeline.megolmReasonExplicitDiscard": "Manually discarded",
  "timeline.megolmReasonFullMemberListReload": "Member list reloaded",
  "timeline.megolmReasonRoomSubscription": "Room subscription changed",
  "timeline.megolmReasonLimitedSyncResponse": "Limited sync response",
  "timeline.megolmReasonKeyShareFailure": "Key sharing failed before retry",
  "timeline.megolmReasonStoreMissing": "Session record unavailable",
  "timeline.megolmReasonInvalidated": "Session invalidated",
  "timeline.megolmReasonUnknown": "Unknown reason",
  "timeline.megolmReasonNotRetained": "Reason unavailable",
  "timeline.originalEventSource": "Original event source",
  "timeline.copyOriginalEventSource": "Copy original event source",
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
  "timeline.sent": "Sent",
  "timeline.resendSend": "Resend",
  "timeline.deleteSend": "Delete",
  "timeline.cancelSend": "Cancel send",
  "timeline.unsentBar": "Some messages haven't been sent",
  "timeline.resendAll": "Resend all",
  "timeline.cancelAll": "Cancel all",
  "timeline.downloadMedia": "Download {filename}",
  "timeline.encryptedMedia": "Encrypted",
  "timeline.mediaUploadProgress": "{percent}%",
  "timeline.mediaDownloadPending": "Downloading…",
  "timeline.mediaDownloadFailed": "Download failed",
  "timeline.mediaDownloadRetry": "Retry",
  "timeline.mediaOpenFile": "Open file",
  "timeline.mediaDetails": "Show media details for {filename}",
  "timeline.mediaDetailsTitle": "Media details",
  "timeline.closeMediaDetails": "Close media details",
  "timeline.mediaViewer": "Media viewer",
  "timeline.threadReplyCountOne": "1 reply",
  "timeline.threadReplyCountMany": "{count} replies",
  "timeline.threadRoot": "Thread root {eventId}",
  "timeline.threadSummaryWithBody": "{count} · {preview}",
  "timeline.threadSummaryWithPreview": "{count} · {sender}: {preview}",
  "timeline.threadSummaryWithSender": "{count} · {sender}",
  "timeline.threadNotificationCount": "Thread notifications · {count}",
  "timeline.typingMany": "{count} people are typing",
  "timeline.typingOne": "{user} is typing",
  "timeline.openThreadSummary": "Open thread, {summary}",
  "timeline.viewReplies": "View new replies · {count}",
  "workspace.activity": "Activity",
  "workspace.createSpace": "Create space",
  "workspace.explore": "Explore",
  "workspace.homeAttention": "{name}, {unread} unread messages, {invites} invites",
  "workspace.filters": "Filters",
  "workspace.home": "Home",
  "workspace.invites": "Invites",
  "workspace.favourites": "Favourites",
  "workspace.lowPriority": "Low priority",
  "workspace.newDm": "New DM",
  "workspace.notJoined": "Not joined",
  "workspace.people": "Direct Messages",
  "workspace.rooms": "Rooms",
  "workspace.resizeRoomList": "Resize room list",
  "workspace.resizeRightPanel": "Resize right panel",
  "workspace.search": "Search",
  "workspace.searchEverywhere": "Search everywhere",
  "workspace.searchInRoom": "Search in {roomName}",
  "workspace.searchPlaceholder": "Search in {spaceName}",
  "workspace.searchScope": "Search scope",
  "workspace.spaceInfoSettings": "Space info and settings",
  "workspace.threads": "Threads",
  "workspace.userSettings": "User settings",
  "workspace.workspaces": "Workspaces",
  "spaceMembers.title": "Space members",
  "spaceMembers.search": "Search space members",
  "spaceMembers.navLabel": "Members",
  "spaceMembers.navCount": "{joined} · +{childOnly}",
  "spaceMembers.navAccessible": "Members, {joined} joined, {childOnly} only in child rooms",
  "spaceMembers.joinedCount": "{count} joined members",
  "spaceMembers.sectionJoined": "Space members",
  "spaceMembers.sectionInvited": "Invitation pending",
  "spaceMembers.sectionChildOnly": "Not in Space",
  "spaceMembers.childRoomContext": "In child rooms: {rooms}",
  "spaceMembers.childRoomCountOne": "In {count} child room",
  "spaceMembers.childRoomCount": "In {count} child rooms",
  "spaceMembers.invite": "Invite to Space",
  "spaceMembers.invitePending": "Invitation pending",
  "spaceMembers.inviteFailed": "Invite failed. Try again.",
  "spaceMembers.cancelInvite": "Cancel invitation",
  "spaceMembers.cancelInvitePending": "Cancelling…",
  "spaceMembers.cancelInviteFailed": "Could not cancel the invitation. Try again.",
  "spaceMembers.loadFailed": "Member load failed. Try again.",
  "spaceMembers.syncIncomplete": "Some child rooms are still syncing",
  "spaceMembers.noResults": "No space members found",
  "spaceMembers.roleSelect": "Role for {name}",
  "spaceMembers.roleUpdateFailed": "Could not update this member's role. Try again.",
  "spaceMembers.roleReload": "Reload roles",
  "spaceMembers.roleConfirmTitle": "Confirm role change",
  "spaceMembers.roleConfirmCopy": "Change {name}'s role to {role}?",
  "spaceMembers.roleConfirmAction": "Confirm role change",
  "threads.empty": "No threads",
  "threads.error": "Could not load threads",
  "threads.loading": "Loading threads…",
  "threads.open": "Open thread",
  "threads.replyCount": "{count} replies",
  "threads.title": "Threads",
  "settings.searchHistory": "Search history",
  "settings.searchHistoryCrawler": "Crawler",
  "settings.searchHistoryPause": "Pause crawler",
  "settings.searchHistoryResume": "Resume crawler",
  "settings.searchHistoryRebuild": "Rebuild search database",
  "settings.searchHistoryRebuildConfirm": "Rebuild the search database? This clears the local search index and re-crawls room history.",
  "settings.searchHistorySpeed": "Crawl speed",
  "settings.searchHistorySpeedStandard": "Standard",
  "settings.searchHistorySpeedFast": "Fast",
  "settings.searchHistorySpeedSlow": "Slow",
  "settings.searchHistorySpeedPaused": "Off",
  "settings.searchHistorySpeedCurrent": "Current",
  "settings.searchHistoryIncludeCaptions": "Index media captions",
  "settings.searchHistoryIncludeFilenames": "Index file names",
  "settings.searchHistoryActivity": "Search crawler activity",
  "settings.searchHistoryIndexingProgress": "Indexing message history... {completed} of {total} rooms",
  "settings.searchHistoryPausedProgress": "Message history indexing is paused. {completed} of {total} rooms indexed",
  "settings.searchHistoryActivitySummary": "{running} running, {queued} queued, {completed} complete, {failed} failed",
  "settings.searchHistoryActivityIdle": "No room is indexing right now.",
  "settings.searchHistoryActivityLastIndexed": "Last indexed {room} {age}.",
  "settings.searchHistoryActivityJustNow": "just now",
  "settings.searchHistoryActivityMinutesAgo": "{count} min ago",
  "settings.searchHistoryActivityHoursAgo": "{count} hr ago",
  "settings.searchHistoryActivityDaysAgo": "{count} d ago",
  "settings.searchHistoryActivityHint": "Processed means timeline events scanned. Indexed means searchable messages written to the local database.",
  "settings.searchHistoryRoomStatus": "Room index status",
  "settings.searchHistoryRoomIdle": "Not started",
  "settings.searchHistoryRoomQueued": "Queued",
  "settings.searchHistoryRoomRunning": "Indexing ({processed} processed, {indexed} indexed)",
  "settings.searchHistoryRoomCompleted": "Complete ({indexed} indexed)",
  "settings.searchHistoryRoomFailed": "Failed",
  "settings.searchHistoryStartRoom": "Start",
  "settings.searchHistoryStopRoom": "Stop",
  "settings.searchHistoryRoomUnknown": "Room"
};

const ja: Catalog = {
  ...en,
  "action.add": "追加",
  "action.back": "戻る",
  "action.cancel": "キャンセル",
  "action.close": "{title}を閉じる",
  "action.continue": "続行",
  "action.createRoom": "ルームを作成",
  "action.createSpace": "スペースを作成",
  "action.done": "完了",
  "action.more": "その他",
  "action.restartSync": "同期を再開",
  "action.recover": "復旧",
  "action.recovering": "復旧中",
  "action.report": "報告",
  "action.send": "送信",
  "action.sending": "送信中",
  "activity.highlightBadge": "メンション",
  "activity.loadMore": "アクティビティをさらに読み込む",
  "activity.loading": "アクティビティを読み込み中",
  "activity.resolvingUnread": "未読メッセージを取得中…",
  "activity.resolveFailed": "未読メッセージを読み込めませんでした。",
  "activity.retryResolution": "再試行",
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
  "auth.createAccount": "アカウントを作成",
  "auth.deviceName": "デバイス名",
  "auth.encryptionRecovery": "暗号化リカバリ",
  "app.about": "Koushi（光子・格子）について",
  "app.title": "Koushi（光子・格子）",
  "app.versionMismatch.title": "Koushi の再起動が必要です",
  "app.versionMismatch.detail":
    "コンポーネントの同期が取れていないため、このセッションを読み込めませんでした。Koushi を完全に終了してから、もう一度開いてください。",
  "auth.matrixAccount": "Matrixアカウント",
  "auth.matrixDesktop": "Koushi（光子・格子）",
  "auth.noLoginMethods": "ログイン方法がありません",
  "auth.notChecked": "未確認",
  "auth.password": "パスワード",
  "auth.recoveryKey": "リカバリキー",
  "auth.recoverySecret": "リカバリキーまたはセキュリティフレーズ",
  "auth.securityPhrase": "セキュリティフレーズ",
  "auth.sessionLocked": "セッションはロック中",
  "auth.signIn": "サインイン",
  "auth.supportedRecoveryMethods": "対応している復旧方法",
  "auth.loginFailureUsernameHint":
    "例: @alice:matrix.org の場合は alice だけを入力し、matrix.org はホームサーバー欄に入れます。",
  "auth.username": "ユーザー名",
  "auth.usernameHelp":
    "ローカル部だけを入力します。先頭の @ とサーバー名は入れません。",
  "auth.usernameOrMatrixId": "ユーザー名またはMatrix ID",
  "auth.usernamePlaceholder": "例: alice",
  "composer.attachedFile": "添付ファイル",
  "composer.attachmentFallback": "添付",
  "composer.attachFile": "ファイルを添付",
  "composer.attachFileInput": "ファイル添付入力",
  "composer.bold": "太字",
  "composer.code": "コード",
  "composer.dropFiles": "ファイルをドロップして添付",
  "composer.emoji": "絵文字",
  "composer.emojiSearch": "絵文字を検索",
  "composer.emojiRecent": "よく使う",
  "composer.italic": "斜体",
  "composer.link": "リンク",
  "composer.list": "リスト",
  "composer.mathMode": "数式",
  "composer.mathModeOff": "数式モード: OFF",
  "composer.mathModeOn": "数式モード: ON",
  "composer.mention": "メンション",
  "composer.inlineMention": "メンション: {label}",
  "composer.mentionLoading": "メンバーを読み込み中…",
  "composer.mentionRoomNotification": "ルーム通知",
  "composer.mentionRoomNotificationDescription": "ルーム全体に通知",
  "composer.mentionSuggestions": "メンション候補",
  "composer.mentionUsers": "ユーザー",
  "composer.messageComposer": "メッセージ入力欄",
  "composer.placeholder": "{roomName}にメッセージ",
  "composer.replying": "返信中",
  "composer.slashCommandUnavailable": "このコマンドはこの入力欄では実行できません。",
  "composer.removeAttachment": "添付を削除",
  "composer.cancelReply": "返信をキャンセル",
  "composer.imageCompressionCompressed": "圧縮済み",
  "composer.imageCompressionOriginal": "元の画像",
  "composer.imageCompressionPreviewAlt": "圧縮後の画像プレビュー",
  "composer.imageCompressionSaveDefault": "この選択をデフォルトにする",
  "composer.imageCompressionTitle": "画像を圧縮",
  "upload.captionForFile": "{filename}のキャプション",
  "upload.ask": "確認",
  "upload.clear": "アップロード準備をクリア",
  "upload.compressed": "圧縮",
  "upload.dialogTitle": "添付をアップロード",
  "upload.original": "オリジナル",
  "upload.preparing": "添付を準備中…",
  "upload.preparationFailed": "添付の準備に失敗しました",
  "upload.retryPreparation": "準備を再試行",
  "upload.useOriginal": "オリジナルを使用",
  "upload.previewAlt": "準備済み添付のプレビュー",
  "upload.savings": "{percent}% 削減",
  "upload.sizeChoice": "アップロードサイズ",
  "upload.resizeChoice": "リサイズ",
  "upload.formatChoice": "形式",
  "upload.resizeOriginal": "元のまま",
  "upload.resizeHalf": "1/2",
  "upload.resizeQuarter": "1/4",
  "upload.resizeEighth": "1/8",
  "upload.formatKeep": "そのまま",
  "upload.formatWebp": "WebP",
  "upload.formatJpeg": "JPEG",
  "upload.formatPng": "PNG",
  "upload.outputSummary": "出力結果",
  "upload.recompressing": "再圧縮中…",
  "upload.outputReady": "準備完了",
  "upload.sendAttachments": "添付を送信",
  "window.title": "Koushi（光子・格子）",
  "context.editMessage": "編集",
  "context.addToFavourites": "お気に入りに追加",
  "context.addToLowPriority": "低優先度に移動",
  "context.ignoreUser": "無視",
  "context.leaveSpace": "スペースから退出",
  "context.leaveRoom": "ルームから退出…",
  "context.leaveConversation": "会話から退出…",
  "room.leaveConfirmTitle": "{name} から退出しますか？",
  "room.leaveConfirmTitleDm": "{name} との会話から退出しますか？",
  "room.leaveConfirmCopy":
    "{name} が参加中のルーム一覧から削除されます。ホームサーバー上のメッセージは削除されません。非公開ルームの場合、再参加には招待が必要になることがあります。",
  "room.leaveConfirmCopyDm":
    "{name} との会話が参加中のルーム一覧から削除されます。ホームサーバー上のメッセージは削除されません。再開には新しい招待が必要になることがあります。",
  "room.leaveConfirmAction": "ルームから退出",
  "room.leaveConfirmActionDm": "会話から退出",
  "context.openKeyboardSettings": "キーボードショートカット",
  "context.openRoomInfo": "ルーム情報",
  "context.openSpaceInfo": "スペース情報",
  "context.openThread": "スレッドで返信",
  "context.openUserInfo": "ユーザー情報",
  "context.openUserSettings": "ユーザー設定",
  "context.redactMessage": "削除",
  "context.removeFromFavourites": "お気に入りから削除",
  "context.removeFromLowPriority": "低優先度から削除",
  "context.searchInRoom": "ルーム内を検索",
  "context.selectRoom": "開く",
  "context.selectSpace": "スペースを開く",
  "context.switchAccount": "アカウントを切り替え",
  "context.reportContent": "コンテンツを報告",
  "context.reportRoom": "ルームを報告",
  "context.reportUser": "ユーザーを報告",
  "context.unignoreUser": "無視解除",
  "dialog.cancel": "キャンセル",
  "dialog.cancelCreate": "作成をキャンセル",
  "dialog.createRoomTitle": "ルームを作成",
  "dialog.createSpaceTitle": "スペースを作成",
  "dialog.encryptedRoom": "暗号化ルーム",
  "dialog.inviteCandidates": "招待候補",
  "dialog.inviteHistory": "招待先に見える履歴",
  "dialog.inviteHistoryCurrent": "現在のルーム設定です。招待を送る前にRoom Infoから変更できます。",
  "dialog.inviteHistoryCurrentBadge": "現在",
  "dialog.inviteHistoryEncryptedShared": "暗号化された共有履歴では、招待先へ過去のルーム鍵を共有する必要がある場合があります。",
  "dialog.inviteHistoryNonRetroactive": "この設定は過去に遡らず、すでに共有されたイベントや鍵を取り消すこともできません。",
  "dialog.inviteHistoryRecovery": "復旧が完了していません。招待は続行できますが、暗号化履歴の共有が制限される場合があります。",
  "dialog.inviteHistoryWorldReadableWarning":
    "現在の高度な設定: メンバーでない人もこのルームの履歴を閲覧できます。",
  "dialog.inviteInvalidMatrixId": "Matrix IDが正しくありません",
  "dialog.invitePeopleTitle": "{name}に招待",
  "dialog.inviteScope": "招待範囲",
  "dialog.inviteSearch": "名前、別名、Matrix ID",
  "dialog.inviteSelectedTargets": "選択中の招待先",
  "dialog.openRoomInfo": "Room Infoを開く",
  "dialog.matrixUserId": "MatrixユーザーID",
  "dialog.newDmTitle": "新しいDM",
  "dialog.privateRoom": "非公開ルーム",
  "dialog.publicRoom": "公開ルーム",
  "dialog.roomAddress": "ルームアドレス",
  "dialog.roomTopic": "トピック",
  "dialog.roomVisibility": "ルーム公開範囲",
  "dialog.sendInvite": "招待を送信",
  "dialog.removeInviteTarget": "招待先を削除",
  "dialog.roomName": "ルーム名",
  "dialog.spaceName": "スペース名",
  "dialog.standardRoomInSpace": "{spaceName}内の標準ルーム",
  "dialog.startDm": "DMを開始",
  "dialog.reportReasonLabel": "理由",
  "dialog.reportReasonPlaceholder": "報告理由を入力してください",
  "dialog.reportReasonTitle": "報告",
  "dialog.submitCreateRoom": "ルーム作成を実行",
  "dialog.submitCreateSpace": "スペース作成を実行",
  "diagnostics.copied": "診断情報をコピーしました。",
  "diagnostics.copy": "診断情報をコピー",
  "diagnostics.copyFailed": "診断情報をコピーできませんでした。もう一度お試しください。",
  "diagnostics.copying": "診断情報をコピー中…",
  "diagnostics.open": "診断情報を開く",
  "diagnostics.title": "診断情報",
  "emoji.category.people": "顔と人",
  "emoji.category.nature": "動物と自然",
  "emoji.category.foods": "食べ物と飲み物",
  "emoji.category.activity": "アクティビティ",
  "emoji.category.places": "旅行と場所",
  "emoji.category.objects": "物",
  "emoji.category.symbols": "記号",
  "emoji.category.flags": "旗",
  "emoji.noResults": "一致する絵文字がありません",
  "files.empty": "ファイルがありません",
  "files.error": "ファイルを読み込めませんでした",
  "files.filterKinds": "ファイルの種類",
  "files.filterPlaceholder": "ファイル名で絞り込み",
  "files.kind.audio": "音声",
  "files.kind.file": "ファイル",
  "files.kind.image": "画像",
  "files.kind.sticker": "ステッカー",
  "files.kind.video": "動画",
  "files.loading": "ファイルを読み込み中…",
  "files.sort.filename": "ファイル名",
  "files.sort.newestFirst": "新しい順",
  "files.sort.oldestFirst": "古い順",
  "files.sort.sender": "送信者",
  "files.sortLabel": "並び順",
  "files.title": "ファイル",
  "help.learnMore": "詳しく見る",
  "help.userTrust.deviceStateBody":
    "デバイス状態は、そのデバイスが所有者のIDからクロス署名されているかを表します。あなたがそのユーザーを検証済みかどうかとは別です。",
  "help.userTrust.deviceStateTitle": "デバイス状態",
  "help.userTrust.effectiveTrustBody":
    "実効信頼は、ユーザー信頼、デバイス状態、ブロック、IDリセット警告を合わせたKoushiの送信判断です。",
  "help.userTrust.effectiveTrustTitle": "実効信頼",
  "help.userTrust.explain": "ユーザー信頼を説明",
  "help.userTrust.identityResetBody":
    "以前検証済みだったユーザーの現在のIDが変わっています。再検証するか、以前の検証を忘れて未検証として扱ってください。",
  "help.userTrust.identityResetTitle": "IDリセット",
  "help.userTrust.title": "ユーザー信頼モデル",
  "help.userTrust.unverifiedBody":
    "未検証は、別経路で本人確認していない相手の通常状態です。暗号化メッセージは送信できます。",
  "help.userTrust.unverifiedTitle": "未検証",
  "help.userTrust.verifiedBody":
    "検証済みは、QR、絵文字、SASなどを別経路で確認した状態です。高い信頼性が必要な相手に使います。",
  "help.userTrust.verifiedTitle": "検証済み",
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
  "directory.previewAlreadyJoined": "すでに参加しています",
  "directory.previewCancel": "キャンセル",
  "directory.previewFailed": "ルーム情報を取得できません: {reason}",
  "directory.previewInviteOnly": "招待制",
  "directory.previewLoading": "ルーム情報を読み込み中",
  "directory.previewOpenRoom": "開く",
  "directory.previewRestricted": "別のルームのメンバーのみ",
  "directory.previewTitle": "このルームに参加しますか？",
  "directory.previewUnknownJoinRule": "参加条件がサーバから返されませんでした",
  "directory.noResults": "公開ルームが見つかりません",
  "directory.results": "公開ルームの結果",
  "directory.search": "検索",
  "directory.joinSectionTitle": "ルームまたはスペースに参加",
  "directory.addressLabel": "Matrix アドレスまたはリンク",
  "directory.addressPlaceholder": "#room:server または https://matrix.to/#/...",
  "directory.addressHelper": "ルームエイリアス、ルーム ID、matrix.to リンクに対応しています。",
  "directory.preview": "プレビュー",
  "directory.addressIsUser": "これはユーザー ID です。新規 DM から DM を開始してください。",
  "directory.addressNotRecognized": "Matrix アドレスまたはリンクとして認識できません。",
  "directory.searchSectionTitle": "公開ルーム・スペースを検索",
  "directory.searchTermLabel": "検索語",
  "directory.searchServerHelper": "選択したサーバーの公開ディレクトリのみを検索します。",
  "directory.roomBadge": "ルーム",
  "directory.searchFailed": "検索に失敗: {reason}",
  "directory.searching": "検索中",
  "directory.searchPlaceholder": "ルーム名・トピック・ルームのリンク",
  "directory.searchServer": "検索するサーバ",
  "directory.searchServerPlaceholder": "自分のサーバ",
  "directory.spaceBadge": "スペース",
  "directory.unnamedRoom": "名前のないルーム",
  "directory.unnamedSpace": "名前のないスペース",
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
  "mediaGallery.empty": "このルームにはメディアがありません",
  "mediaGallery.next": "次のメディア",
  "mediaGallery.noPreview": "プレビューなし",
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
  "panel.people": "メンバー",
  "panel.profile": "プロフィール",
  "people.memberCount": "メンバー {count}人",
  "people.memberActions": "メンバー操作",
  "people.openProfile": "{name}のプロフィールを開く",
  "people.searchMembers": "メンバーを検索",
  "people.noSearchResults": "該当するメンバーが見つかりません。",
  "people.you": "あなた",
  "people.sendMessage": "メッセージを送信",
  "people.setAlias": "エイリアスを設定",
  "people.unknownUser": "不明なユーザー",
  "room.avatarUrl": "ルームアバターURL",
  "room.ban": "BAN",
  "room.banMember": "{name}をBAN",
  "room.currentAvatar": "現在のアバター",
  "room.currentTopic": "現在のトピック",
  "room.copyShareLink": "ルームリンクをコピー",
  "room.members": "メンバー",
  "room.directMessage": "ダイレクトメッセージ",
  "room.accessAndHistory": "アクセスと履歴",
  "room.accessAndHistoryHint": "参加ルールは入室できる人を、履歴の表示範囲は見える内容を決めます。両者は独立しています。",
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
  "room.historyInvitedDescription": "招待された後に送信されたメッセージを閲覧できます。",
  "room.historyJoinedDescription": "参加した後に送信されたメッセージを閲覧できます。",
  "room.historySharedDescription": "メンバーに共有されたルーム履歴を閲覧できます。",
  "room.historySharedEncryptedHint": "暗号化ルームでは、共有履歴を復号できるよう、対象となる招待者へ過去のルーム鍵を共有する場合があります。",
  "room.historyNonRetroactive": "この設定を変更しても過去は書き換わらず、すでに共有されたイベントや鍵を取り消すこともできません。",
  "room.historyRecoveryRequired": "デバイスの復旧が完了していません。招待は続行できますが、暗号化履歴の共有が制限される場合があります。",
  "room.historyWorldReadableDescription": "ルームを発見できる人は、メンバーでなくても履歴を閲覧できます。",
  "room.historyWorldReadableWarning": "高度な設定: メンバーでない人もルーム履歴を閲覧できます。",
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
  "room.messageMember": "{name}にメッセージ",
  "room.memberRole": "メンバーロール",
  "room.memberRoleFor": "{name}のメンバーロール",
  "room.aliasDialogTitle": "{name}のエイリアス",
  "room.aliasInput": "エイリアス",
  "room.clearAlias": "エイリアスを解除",
  "room.clearAliasForMember": "{name}のエイリアスを解除",
  "room.editAlias": "エイリアスを編集",
  "room.editAliasForMember": "{name}のエイリアスを編集",
  "room.memberOriginalName": "元の名前: {name}",
  "room.setAlias": "エイリアスを設定",
  "room.setAliasForMember": "{name}のエイリアスを設定",
  "room.noAvatar": "アバターなし",
  "room.noMembers": "読み込まれたメンバーはありません",
  "room.noTopic": "トピックなし",
  "room.noRoomSelected": "ルームが選択されていません",
  "room.noSpaces": "スペースがありません",
  "room.notifications": "通知",
  "room.notifyModeAll": "すべてのメッセージ",
  "room.notifyModeMentions": "メンションのみ",
  "room.notifyModeMute": "ミュート",
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
  "room.repair": "修復",
  "room.repairTimeline": "ルームのタイムラインを修復",
  "room.repairTimelineHint": "既存のメッセージを削除せず、欠けている範囲を検出して修復します。",
  "room.status": "ルーム状態",
  "room.statusEncrypted": "暗号化済み",
  "room.statusHistoryLimited": "新規メンバーは履歴を閲覧不可",
  "room.statusHistoryShared": "新規メンバーも履歴を閲覧可",
  "room.statusHistoryWorldReadable": "誰でも履歴を閲覧可",
  "room.statusNotEncrypted": "未暗号化",
  "room.statusPrivate": "非公開",
  "room.statusPublic": "公開",
  "room.reshareRoomKeys": "ルーム鍵を再共有",
  "room.reshareRoomKeysHint": "このルームで保持している復号鍵を、対象となるブロックされていないデバイスへ再送します。",
  "room.reshareRoomKeysPending": "鍵を再共有中…",
  "room.reshareRoomKeysSuccess": "ルーム鍵を再送しました。",
  "room.reshareRoomKeysNoSession": "再共有できる有効なルーム鍵がありません。",
  "room.reshareRoomKeysNoRecipients": "このルーム鍵を必要とする対象デバイスはありません。",
  "room.reshareRoomKeysStaleSession": "有効なルーム鍵が変わりました。もう一度お試しください。",
  "room.reshareRoomKeysError": "ルーム鍵を再共有できませんでした。",
  "room.dangerousEncryptionDebugging": "危険な暗号化デバッグ",
  "room.dangerousEncryptionDebuggingWarning":
    "一時的な診断用コントロールです。ローテーションは以降の暗号化メッセージに影響します。鍵の共有は、受信者が鍵を受け取り・保存したことを証明するものではありません。",
  "room.forceNewEncryptionSession": "新しい暗号化セッションを作成",
  "room.shareIndex0Key": "インデックス0の鍵をルーム参加者に共有",
  "room.resendIndex0Key": "インデックス0の復旧鍵を再送",
  "room.forceNewEncryptionSessionConfirm":
    "現在の暗号化セッションを破棄し、メッセージインデックス0の新しいセッションを作成します。繰り返しクリックすると追加のセッションが作成されることがあります。続行しますか？",
  "room.shareIndex0KeyConfirm":
    "現在のセッションのインデックス0の鍵を、対象となるすべての受信デバイスに共有します。受信者の受け取り・保存の証明にはなりません。セッションがインデックス0を過ぎている場合は、先に新しいセッションを作成してください。続行しますか？",
  "room.resendIndex0KeyConfirm":
    "このセッションの復号能力を、元の共有台帳にあるデバイスへ恒久的に付与します。共有した鍵は取り消せません。この一時的な操作は診断専用です。続行しますか？",
  "room.confirm": "確認",
  "room.cancel": "キャンセル",
  "room.encryptionDebugOutcomeCompleted": "操作が完了しました。",
  "room.encryptionDebugOutcomeRefusedNotEncrypted": "このルームは暗号化されていません。",
  "room.encryptionDebugOutcomeRefusedIndexAdvanced":
    "セッションはインデックス0を過ぎています。先に新しい暗号化セッションを作成してください。",
  "room.encryptionDebugOutcomeNoSession": "有効な送信セッションがありません。",
  "room.encryptionDebugOutcomeNoRecipients": "元の受信デバイスに適格な対象がありません。",
  "room.encryptionDebugOutcomeInboundSessionMissing": "一致する受信セッションがありません。",
  "room.encryptionDebugOutcomeInboundIndexAdvanced": "受信セッションからインデックス0を取り出せません。",
  "room.encryptionDebugOutcomeOriginalLedgerMissing": "元の共有証明がありません。",
  "room.encryptionDebugOutcomeStaleIdentityRefused": "受信デバイスの本人性が変わったため共有を拒否しました。",
  "room.encryptionDebugOutcomeCancelledStale":
    "セッションまたはルームが変更されたため、操作はキャンセルされました。",
  "room.encryptionDebugOutcomePolicyBlocked": "受信者ポリシーにより共有がブロックされました。",
  "room.encryptionDebugOutcomeDeadline": "操作がタイムアウトしました。",
  "room.encryptionDebugOutcomeFailed": "操作が失敗しました。",
  "room.saveAccess": "アクセス設定を保存",
  "room.saveHistoryVisibility": "履歴の表示範囲を保存",
  "room.saveJoinRule": "参加ルールを保存",
  "room.saveAvatar": "アバターを保存",
  "room.saveName": "ルーム名を保存",
  "room.saveTopic": "トピックを保存",
  "room.searchIndex": "検索インデックス",
  "room.settingsLoading": "ルーム設定を読み込み中",
  "room.spaces": "スペース",
  "room.subscribed": "購読中",
  "room.summary": "ルーム概要",
  "room.tabs": "ルームタブ",
  "room.rightPanelToggle": "右パネルを切り替え",
  "room.returnToInvite": "招待に戻る",
  "room.timeline": "タイムライン",
  "room.topic": "ルームトピック",
  "room.type": "種類",
  "room.unban": "BAN解除",
  "room.unbanMember": "{name}のBANを解除",
  "room.unread": "未読",
  "room.unreadCount": "未読 {count} 件",
  "roomList.category": "ルームリストのカテゴリ",
  "roomList.categoryDms": "DM",
  "roomList.categoryRooms": "ルーム",
  "roomList.categorySummary": "{category}、未読 {unread} 件、合計 {total} 件",
  "roomList.categorySummaryWithHighlights":
    "{category}、未読 {unread} 件、合計 {total} 件、メンション {highlights} 件",
  "roomList.filterRooms": "ルーム",
  "roomList.filterUnread": "未読",
  "roomList.filterPeople": "Direct Messages",
  "roomList.filterFavourites": "お気に入り",
  "roomList.filterInvites": "招待",
  "roomList.filterDmsPlaceholder": "DMを絞り込む",
  "roomList.filterRoomsPlaceholder": "ルームを絞り込む",
  "roomList.clearFilter": "ルームリストの絞り込みをクリア",
  "roomList.noMatchingDms": "一致するDMがありません",
  "roomList.noMatchingRooms": "一致するルームがありません",
  "roomList.sort": "ルームリストの並び順",
  "roomList.sortLabel": "並び順",
  "roomList.sortActive": "アクティブ",
  "roomList.sortName": "名前",
  "roomList.loading": "ルームを読み込み中…",
  "roomList.failed": "ルームを読み込めませんでした",
  "room.markAsRead": "既読にする",
  "room.markAsUnread": "未読にする",
  "search.indexingPending": "メッセージ履歴をインデックス中です。まもなく追加の一致が表示される場合があります。",
  "search.noExactMatches": "完全一致はありません",
  "search.matchAttachmentFileName": "添付ファイル名",
  "search.matchMessage": "メッセージ",
  "search.resultCountMany": "\"{query}\" の結果 {count} 件",
  "search.resultCountOne": "\"{query}\" の結果 1 件",
  "search.searching": "検索中...",
  "search.searchingFor": "\"{query}\" を検索中",
  "search.tooShort": "検索語が短すぎます",
  "search.scopeAll": "すべて",
  "search.scopeRoom": "ルーム/DM",
  "search.scopeSpace": "スペース",
  "sessionStatus.authentication": "認証方法",
  "sessionStatus.authOauth": "OAuth 認証",
  "sessionStatus.authPassword": "パスワード",
  "sessionStatus.authSso": "SSO 認証",
  "sessionStatus.authToken": "アクセストークン",
  "sessionStatus.backupDisabled": "無効",
  "sessionStatus.backupReady": "準備完了",
  "sessionStatus.checking": "確認中",
  "sessionStatus.copyDeviceId": "デバイス ID をコピー",
  "sessionStatus.crossSigned": "クロス署名済み",
  "sessionStatus.deviceId": "デバイス ID",
  "sessionStatus.deviceName": "デバイス名",
  "sessionStatus.failed": "確認失敗",
  "sessionStatus.failureSdk": "セッションの確認に失敗しました",
  "sessionStatus.failureTimedOut": "セッションの確認がタイムアウトしました",
  "sessionStatus.failureUnavailable": "セッション状態を確認できません",
  "sessionStatus.homeserver": "ホームサーバー",
  "sessionStatus.identity": "自分の ID",
  "sessionStatus.identityMissing": "ID がありません",
  "sessionStatus.identityUnverified": "ID は未検証です",
  "sessionStatus.identityVerified": "ID を検証済み",
  "sessionStatus.keyBackup": "鍵バックアップ",
  "sessionStatus.lastChecked": "最終確認",
  "sessionStatus.manageAccount": "アカウントとデバイスを管理",
  "sessionStatus.notChecked": "未確認",
  "sessionStatus.notCrossSigned": "クロス署名なし",
  "sessionStatus.open": "セッション状態を開く",
  "sessionStatus.openWithRuntimeWarning": "セッション状態を開く（実行時の警告 {count} 件）",
  "sessionStatus.openWithRuntimeWarnings": "セッション状態を開く（実行時の警告 {count} 件）",
  "sessionStatus.ownerCrossSigning": "所有者のクロス署名",
  "sessionStatus.recheck": "再確認",
  "sessionStatus.retry": "再試行",
  "sessionStatus.runtimeAlertSecureBackup": "安全なバックアップを利用できません",
  "sessionStatus.runtimeWarningCount": "実行時の警告 {count} 件",
  "sessionStatus.runtimeWarnings": "実行時の警告",
  "sessionStatus.runtimeWarningsCount": "実行時の警告 {count} 件",
  "sessionStatus.sync": "同期",
  "sessionStatus.syncError": "エラー",
  "sessionStatus.syncRunning": "実行中",
  "sessionStatus.syncStarting": "開始中",
  "sessionStatus.syncStopped": "停止中",
  "sessionStatus.title": "現在のセッション",
  "sessionStatus.unavailable": "利用不可",
  "sessionStatus.unverified": "未検証",
  "sessionStatus.unknown": "不明",
  "sessionStatus.userId": "ユーザー ID",
  "sessionStatus.verification": "検証",
  "sessionStatus.verified": "検証済み",
  "settings.accounts": "アカウント",
  "settings.appearance": "外観",
  "settings.accountSwitcher": "アカウント切り替え",
  "settings.current": "現在",
  "settings.autoLoadOlderMessages": "古いメッセージを自動で読み込む",
  "settings.autoLoadOlderMessagesDescription": "読み込み済みタイムラインの先頭付近までスクロールしたらルーム履歴を先読みします",
  "settings.threadRootLatestReply": "スレッドを最新の返信位置に表示する",
  "settings.threadRootLatestReplyDescription": "ルートメッセージと返信サマリーを最新の返信時刻の位置へ移動します",
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
  "settings.timeline": "タイムライン",
  "settings.display": "表示",
  "settings.displayDensity": "表示密度",
  "settings.densityCompact": "コンパクト",
  "settings.densityDefault": "標準",
  "settings.densityComfortable": "ゆったり",
  "settings.codeBlockWrap": "コードブロックの長い行を折り返す",
  "settings.hideRedacted": "削除されたメッセージを非表示",
  "settings.urlPreviews": "URLプレビュー",
  "settings.urlPreviewsEnabled": "リンクプレビューを表示",
  "settings.urlPreviewsDescription": "メッセージ内のリンクのプレビューを読み込む",
  "settings.urlPreviewsUnencrypted": "非暗号化ルームのURLプレビュー",
  "settings.urlPreviewsUnencryptedDescription": "非暗号化ルームではリンクのプレビューをデフォルトで読み込む",
  "settings.urlPreviewsEncrypted": "暗号化ルームのURLプレビュー",
  "settings.urlPreviewsEncryptedDescription": "暗号化ルームで送信されたリンクのプレビュー読み込みを有効にする",
  "settings.urlPreviewsEncryptedNotice": "暗号化ルームのプレビューはURLをホームサーバーやアクセス先サイトに知らせる可能性があります。",
  "settings.urlPreviewsEnabledForRoom": "このルームでリンクプレビューを有効にする",
  "timeline.linkPreviewHide": "プレビューを非表示",
  "timeline.linkPreviewFailed": "プレビューを読み込めませんでした",
  "timeline.linkPreviewLoading": "プレビューを読み込み中…",
  "settings.notificationBadges": "バッジ",
  "settings.notificationDesktop": "デスクトップ通知",
  "settings.notificationSound": "サウンド",
  "settings.sendReadReceipts": "既読を送信",
  "settings.sendTypingNotifications": "入力通知を送信",
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
  "settings.messagingPrivacy": "メッセージとプライバシー",
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
  "settings.accountManagement": "アカウント管理",
  "settings.manageAccount": "アカウントとデバイスを管理",
  "settings.manageAccountHint": "このサーバーのアカウントとデバイスの管理ページをブラウザで開きます。",
  "settings.changePassword": "パスワードを変更",
  "settings.changePasswordLabel": "新しいパスワード",
  "settings.changePasswordConfirm": "新しいパスワード（確認）",
  "settings.changePasswordMismatch": "パスワードが一致しません",
  "settings.passwordChanged": "パスワードを変更しました",
  "settings.deactivateAccount": "アカウントを無効化",
  "settings.deactivateAccountErase": "すべてのデータを消去",
  "settings.deactivateAccountConfirm": "この操作は元に戻せません。続行するには確認してください。",
  "settings.accountDeactivated": "アカウントを無効化しました",
  "settings.accountManagementFailed": "アカウント操作に失敗しました",
  "settings.signOut": "サインアウト",
  "settings.keyManagement": "鍵管理",
  "settings.roomKeyExport": "ルーム鍵エクスポート",
  "settings.roomKeyExportDestination": "鍵エクスポート先",
  "settings.roomKeyExportIdle": "未エクスポート",
  "settings.roomKeyExporting": "エクスポート中",
  "settings.roomKeyExportedUnknown": "エクスポート済み",
  "settings.roomKeyExportedCount": "{count} セッションをエクスポート済み",
  "settings.roomKeyExportFailed": "エクスポート失敗: {reason}",
  "settings.exportRoomKeys": "ルーム鍵をエクスポート",
  "settings.chooseRoomKeyExportFile": "エクスポート先ファイルを選択",
  "settings.roomKeyImport": "ルーム鍵インポート",
  "settings.roomKeyImportSource": "鍵インポート元",
  "settings.roomKeyImportIdle": "未インポート",
  "settings.roomKeyImporting": "インポート中",
  "settings.roomKeyImportedCount": "{imported}/{total} をインポート済み",
  "settings.roomKeyImportFailed": "インポート失敗: {reason}",
  "settings.importRoomKeys": "ルーム鍵をインポート",
  "settings.chooseRoomKeyImportFile": "インポート元ファイルを選択",
  "settings.roomKeyPassphrasePromptExport":
    "ルーム鍵エクスポート用のパスフレーズを入力します。",
  "settings.roomKeyPassphrasePromptImport":
    "このルーム鍵ファイルのパスフレーズを入力します。",
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
  "settings.resetLocalDataConfirm": "このセッションのローカルデータをリセットしますか？このデバイス上の Matrix ストア、キャッシュ済み履歴、保存済み資格情報が削除されます。この操作は元に戻せません。",
  "gate.resetForNewDevice": "ローカルデータを削除して新しいデバイスとしてログイン",
  "gate.resetForNewDeviceConfirm": "このセッションのローカルデータを削除し、新しいデバイス ID でログインし直しますか？この端末上の暗号化ストア、キャッシュ済み履歴、保存済み資格情報が削除されます。ホームサーバー上のメッセージは削除されません。この操作は元に戻せません。",
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
  "scheduled.bodyInput": "予約メッセージ",
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
  "trust.cancelIdentityReset": "IDリセットをキャンセル",
  "trust.continueIdentityReset": "続行",
  "trust.crossSigning": "クロス署名",
  "trust.declineVerification": "拒否",
  "trust.deviceBlocked": "ブロック済み",
  "trust.deviceCount": "デバイス {count} 台",
  "trust.deviceOrdinal": "デバイス {index}",
  "trust.deviceUnknown": "不明",
  "trust.deviceUnverified": "未検証",
  "trust.deviceVerified": "検証済み",
  "trust.deviceCrossSigned": "クロス署名済み",
  "trust.deviceNotCrossSigned": "未クロス署名",
  "trust.devices": "デバイス",
  "trust.enableKeyBackup": "有効化",
  "trust.encryption": "暗号化",
  "trust.failureCancelled": "キャンセル済み",
  "trust.failureForbidden": "禁止",
  "trust.failureInvalidPassphrase": "パスフレーズが無効",
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
  "trust.userIdentityReset": "IDリセット",
  "trust.userUnverified": "未検証",
  "trust.userVerified": "検証済み",
  "trust.verification": "デバイス検証",
  "gate.title": "このセッションを検証",
  "gate.sessionExpired": "セッションの有効期限が切れました",
  "gate.sessionExpiredCopy": "このセッションは期限切れになったか、取り消されました。続行するには再度サインインしてください。",
  "gate.verifying": "このセッションを検証中…",
  "gate.finishing": "サインインを完了しています…",
  "gate.preparing": "ルームを準備しています…",
  "gate.checking": "デバイスの信頼状態を確認中…",
  "gate.discovering": "認証方法を確認中…",
  "gate.rejecting": "未検証セッションを拒否中…",
  "gate.locked": "このセッションは再検証が必要です。",
  "gate.otherDevice": "別のデバイスで検証",
  "gate.verifyRecoveryKey": "リカバリーキーで検証",
  "gate.noRecoveryKeyTitle": "リカバリーキーがありません",
  "gate.noRecoveryKeyCopy":
    "リカバリー情報がないため、Koushi はこのセッションを検証できません。すでに検証済みの Matrix クライアントでリカバリーを設定するか、リカバリーが本当に利用できない場合は暗号 ID をリセットしてください。それまでこの端末はサインインしたままで、他のクライアントには未検証として表示される場合があります。",
  "gate.deviceVerificationDialogTitle": "デバイス検証を試しますか？",
  "gate.deviceVerificationDialogCopy": "デバイス間検証は、相手のデバイスがオフライン、同期が遅い、または鍵が不足している場合に不安定になることがあります。可能な場合はリカバリーキーでの検証をおすすめします。",
  "gate.useRecoveryKey": "リカバリーキーを使う",
  "gate.tryDeviceVerificationAnyway": "それでもデバイス検証を試す",
  "gate.match": "一致しています",
  "gate.mismatch": "一致していません",
  "gate.recoverySecret": "リカバリーシークレット",
  "gate.recover": "復旧",
  "gate.destination": "リカバリーキーの保存先",
  "gate.passphrase": "バックアップ用パスフレーズ",
  "gate.bootstrap": "安全なバックアップを作成",
  "gate.saved": "リカバリーキーを保存しました",
  "gate.retry": "再試行",
  "gate.cleanupOffer": "サインインを中止してこのデバイスを削除…",
  "gate.cleanupDialogTitle": "サインインを中止してこのデバイスを削除",
  "gate.cleanupDialogCopy": "最初に Matrix アカウントからこのデバイスを削除し、その後このコンピューター上のローカルメッセージ、暗号化キー、キャッシュ、保存済み資格情報を消去します。ホームサーバー上のメッセージは保持されます。次回のサインインでは新しい Device ID が作成されます。この操作は元に戻せません。",
  "gate.cleanupConfirm": "デバイスを削除してローカルデータを消去",
  "gate.cleanupResolving": "このデバイスの削除方法を確認しています…",
  "gate.cleanupRemoving": "Matrix アカウントからこのデバイスを削除しています…",
  "gate.cleanupAccountPassword": "アカウントのパスワード",
  "gate.cleanupContinue": "デバイスの削除を続行",
  "gate.cleanupRemoteFailed": "Matrix アカウントからデバイスを削除できませんでした。資格情報とローカルデータは保持されています。",
  "gate.cleanupRetryRemote": "デバイスの削除を再試行",
  "gate.cleanupEraseAnywayOffer": "それでもローカルデータを消去…",
  "gate.cleanupEraseAnywayTitle": "それでもローカルデータを消去",
  "gate.cleanupEraseAnywayCopy": "このデバイスが Matrix アカウント上で有効なまま残る可能性があります。続行すると、このコンピューター上のローカルメッセージ、暗号化キー、保存済み資格情報が消去されます。この操作は元に戻せません。",
  "gate.cleanupEraseAnywayConfirm": "それでもローカルデータを消去",
  "gate.cleanupResettingLocal": "デバイスを削除しました。ローカルデータを消去しています…",
  "gate.cleanupErasingLocal": "リモートデバイスが残る可能性がある状態でローカルデータを消去しています…",
  "gate.cleanupLocalFailed": "ローカルデータを消去できませんでした。安全に再試行できます。",
  "gate.cleanupRetryLocal": "ローカルデータの消去を再試行",
  "gate.cleanupCommandFailed": "デバイスのクリーンアップに失敗しました。もう一度お試しください。",
  "gate.signOut": "サインアウト",
  "gate.secureBackupTitle": "安全なバックアップが必要です",
  "gate.secureBackupChecking": "安全なバックアップを確認中…",
  "gate.secureBackupInactive": "安全なバックアップの状態を確認できません。確認が完了するまで暗号化メッセージはロックされます。",
  "gate.secureBackupNeedsRecovery": "続行するには既存の安全なバックアップを復旧してください。",
  "gate.secureBackupRecoveryKey": "安全なバックアップのリカバリーキー",
  "gate.secureBackupRecover": "安全なバックアップを復旧",
  "gate.secureBackupSetupTitle": "安全なバックアップを設定",
  "gate.secureBackupSetupCopy": "暗号化メッセージを利用する前に、安全なバックアップを作成して保護してください。リカバリーキーを安全な保存先に保存します。",
  "gate.secureBackupPassphrase": "安全なバックアップのパスフレーズ",
  "gate.secureBackupRecoveryKeyDestination": "リカバリーキーの保存先",
  "gate.secureBackupChooseDestination": "リカバリーキーの保存先を選択",
  "gate.secureBackupDestinationSelected": "リカバリーキーの保存先を選択しました。",
  "gate.secureBackupDestinationNotSelected": "リカバリーキーの保存先が選択されていません。",
  "gate.secureBackupDestinationSelectionFailed": "リカバリーキーの保存先を開けませんでした。もう一度お試しください。",
  "gate.secureBackupSetup": "安全なバックアップを設定",
  "gate.secureBackupExplicitDisabledTitle": "安全なバックアップが無効になっています",
  "gate.secureBackupExplicitDisabledCopy": "安全なバックアップを再有効化すると、このアカウント全体の設定が変わり、このアカウントでサインインしている他の Matrix クライアントにも影響します。これらのクライアントで更新後のバックアップを使う場合のみ確認してください。",
  "gate.secureBackupReenable": "安全なバックアップを再有効化",
  "gate.secureBackupReenableConfirm": "再有効化を確認",
  "gate.secureBackupCreating": "安全なバックアップを作成中…",
  "gate.secureBackupDeliveryRequired": "続行する前にリカバリーキーを保存してください。",
  "gate.secureBackupUploading": "既存の暗号化キーをアップロード中…",
  "gate.secureBackupPendingZero": "アップロードする暗号化キーは残っていません。",
  "gate.secureBackupPendingOne": "既存の暗号化キーをアップロード中: 残り 1 件。",
  "gate.secureBackupPendingTwoToTen": "既存の暗号化キーをアップロード中: 残り 2～10 件。",
  "gate.secureBackupPendingElevenToOneHundred": "既存の暗号化キーをアップロード中: 残り 11～100 件。",
  "gate.secureBackupPendingOverOneHundred": "既存の暗号化キーをアップロード中: 残り 100 件超。",
  "gate.secureBackupPendingUnknown": "既存の暗号化キーをアップロード中: 残り件数は不明です。",
  "gate.secureBackupRetrying": "安全なバックアップを再試行中…",
  "gate.secureBackupFailureNetwork": "安全なバックアップのサーバーに接続できませんでした。",
  "gate.secureBackupFailureRateLimited": "安全なバックアップのリクエストが制限されています。後でもう一度お試しください。",
  "gate.secureBackupFailureInvalidRecoveryKey": "そのリカバリーキーでは既存の安全なバックアップを解除できませんでした。",
  "gate.secureBackupFailureBackupKeyMismatch": "ローカルのバックアップキーが既存の安全なバックアップと一致しません。",
  "gate.secureBackupFailureSecretStorageIncomplete": "この端末の安全な保存領域が不完全です。",
  "gate.secureBackupFailureArtifactDelivery": "選択した保存先にリカバリーキーを保存できませんでした。",
  "gate.secureBackupFailureForbidden": "このアカウントでは安全なバックアップを使用できません。",
  "gate.secureBackupFailureTimeout": "安全なバックアップの応答がタイムアウトしました。",
  "gate.secureBackupFailureSdk": "安全なバックアップに失敗しました。",
  "gate.secureBackupRetry": "安全なバックアップを再試行",
  "gate.secureBackupDiagnostics": "安全なバックアップの診断を開く",
  "gate.secureBackupCommandFailed": "安全なバックアップの操作に失敗しました。もう一度お試しください。",
  "gate.secureBackupRuntimeDegraded": "安全なバックアップが一時的に低下しています。Koushi が再試行している間も、送信、下書き、受信、端末上の復号は利用できます。",
  "slidingSync.blockedTitle": "Simplified Sliding Sync が必要です",
  "slidingSync.unsupported": "このホームサーバーは Simplified Sliding Sync に対応していません。",
  "slidingSync.unreachable": "このホームサーバーに接続できず、Simplified Sliding Sync 対応を確認できませんでした。",
  "slidingSync.invalidResponse": "このホームサーバーから Simplified Sliding Sync の無効な対応情報が返されました。",
  "slidingSync.changeHomeserver": "ホームサーバーを変更",
  "gate.failureCancelled": "検証がキャンセルされました。",
  "gate.failureForbidden": "検証は許可されませんでした。",
  "gate.failureNoProof": "利用可能な証明方法がありません。",
  "gate.failureSdk": "検証に失敗しました。",
  "gate.verificationCommandFailed": "検証コマンドに失敗しました。もう一度お試しください。",
  "gate.rejectNoProof": "既存の識別情報に利用可能な証明方法がありません。",
  "gate.rejectUser": "このセッションは拒否されました。",
  "space.allRooms": "すべてのルーム",
  "space.childRooms": "子ルーム",
  "space.directMessages": "Direct Messages",
  "space.home": "ホーム",
  "space.invite": "招待",
  "space.localIcon": "ローカルアイコン",
  "space.localIconPlaceholder": "絵文字または短い文字",
  "space.localName": "ローカル名",
  "space.localNamePlaceholder": "この端末でのスペース名",
  "space.localPresentation": "ローカル表示",
  "space.resetLocalPresentation": "ローカル表示をリセット",
  "space.noUnread": "未読なし",
  "space.preferences": "環境設定",
  "space.roomMembership": "ルーム参加状態",
  "space.spacePreferences": "スペース環境設定",
  "space.spaceSettings": "スペース設定",
  "space.summary": "スペース概要",
  "sync.failed": "失敗",
  "sync.failedWithReason": "同期失敗: {reason}",
  "sync.reasonAuth": "再ログインが必要",
  "sync.reasonHttp": "ネットワーク問題",
  "sync.reasonInternal": "内部エラー",
  "sync.reasonNetworkError": "ネットワーク問題",
  "sync.reasonNetworkOffline": "ネットワーク未接続",
  "sync.reasonStore": "ローカル保存に問題",
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
  "shortcut.toggleFullscreen": "全画面を切り替え",
  "shortcut.toggleMicrophone": "通話中のマイクを切り替え",
  "shortcut.toggleRightPanel": "右パネルを切り替え",
  "shortcut.toggleSpacePanel": "スペースパネルを切り替え",
  "shortcut.uploadFile": "ファイルをアップロード",
  "timeline.conversation": "会話タイムライン",
  "timeline.conversationStart": "会話の開始",
  "timeline.notice.roomCreate": "ルームを作成しました",
  "timeline.notice.roomPowerLevels": "ルーム権限を更新しました",
  "timeline.notice.roomGuestAccess": "ゲストアクセスを更新しました",
  "timeline.notice.roomEncryption": "ルームの暗号化を有効にしました",
  "timeline.notice.spaceParent": "親スペースを更新しました",
  "timeline.notice.roomJoinRules": "参加ルールを更新しました",
  "timeline.notice.roomHistoryVisibility": "履歴の表示範囲を更新しました",
  "timeline.notice.roomPinnedEvents": "ピン留めメッセージを更新しました",
  "timeline.notice.roomNameSet": "ルーム名を「{newName}」に設定しました",
  "timeline.notice.roomNameChanged": "ルーム名を「{oldName}」から「{newName}」に変更しました",
  "timeline.notice.roomNameRemoved": "ルーム名を削除しました",
  "timeline.notice.roomNameChangedGeneric": "ルーム名が変更されました",
  "timeline.editedMessage": "編集済み",
  "timeline.editMessage": "メッセージを編集",
  "timeline.editBody": "メッセージ本文を編集",
  "timeline.loading": "読み込み中",
  "timeline.gapRepairing": "欠けているメッセージを修復中…",
  "timeline.gapRepairFailed": "一部のメッセージを取得できませんでした。",
  "timeline.threadRootLoading": "スレッドのメッセージを読み込んでいます…",
  "timeline.threadRootUnavailable": "スレッドのメッセージを利用できません。",
  "timeline.messagesTab": "メッセージ",
  "timeline.openingThread": "スレッドを開いています",
  "timeline.presenceAway": "離席中",
  "timeline.presenceOffline": "オフライン",
  "timeline.presenceOnline": "オンライン",
  "timeline.addReaction": "リアクションを追加",
  "timeline.reactionSummary": "リアクション {key}、{count} 件",
  "timeline.reactionPicker": "リアクションを選択",
  "timeline.reactionOption": "{emoji}でリアクション",
  "timeline.reactionTooltip": "{names} が {key} でリアクションしました",
  "timeline.reactionSenderOverflow": "他 {count} 人",
  "timeline.reactionSenderUnknown": "{count} 人",
  "timeline.readBy": "{count} 人が既読",
  "timeline.readReceiptOverflow": "他 {count} 人",
  "timeline.readMarker": "ここまで既読",
  "timeline.readStateSyncing": "既読状態を同期中",
  "timeline.readStateNotSynced": "既読状態を同期できません",
  "timeline.readStateReasonAuthentication": "認証が必要です",
  "timeline.readStateReasonRateLimited": "送信制限中です",
  "timeline.readStateReasonTimeout": "時間切れです",
  "timeline.readStateReasonTransport": "接続に失敗しました",
  "timeline.readStateReasonServer": "サーバーエラーです",
  "timeline.readStateReasonCapacity": "未送信の既読更新が多すぎます",
  "timeline.readStateReasonSdk": "クライアントエラーです",
  "timeline.unreadMarker": "未読メッセージ",
  "timeline.jumpToFirstUnread": "最初の未読へ移動、未読 {count} 件",
  "timeline.jumpToBottom": "最新へ移動、新着 {count} 件",
  "timeline.olderMessages": "古いメッセージ",
  "timeline.latest": "最新",
  "timeline.navigation": "タイムライン操作",
  "timeline.saveEdit": "編集を保存",
  "timeline.cancelEdit": "編集をキャンセル",
  "timeline.redactMessage": "メッセージを削除",
  "timeline.redactedMessage": "メッセージは削除されました",
  "timeline.revealSpoiler": "スポイラーを表示",
  "timeline.pinMessage": "メッセージをピン留め",
  "timeline.spoiler": "スポイラー",
  "timeline.unpinMessage": "メッセージのピン留めを解除",
  "timeline.pinnedMessages": "ピン留めメッセージ",
  "timeline.pinnedMessagesCount": "ピン留め · {count}",
  "timeline.pinnedMessage": "ピン留めメッセージ",
  "timeline.pinnedMessagesEmpty": "ピン留めメッセージはありません",
  "timeline.pinnedEventUnableToDecrypt": "このメッセージを復号できません",
  "timeline.pinnedEventUnavailable": "ピン留めメッセージを利用できません",
  "timeline.pinnedNavigationLoading": "ピン留めメッセージを開いています…",
  "timeline.pinnedNavigationFailed": "ピン留めメッセージを開けませんでした",
  "timeline.pinnedNavigationRetry": "再試行",
  "timeline.replyQuoteMissing": "元のメッセージを利用できません",
  "timeline.replyQuoteUnavailable": "元のメッセージを利用できません",
  "timeline.replyQuoteUnknownSender": "不明な送信者",
  "timeline.replyQuoteUnsupported": "未対応のメッセージ",
  "timeline.replyToMessage": "メッセージに返信",
  "timeline.replyInThread": "スレッドで返信",
  "timeline.messageActions": "メッセージ操作",
  "timeline.copyMessage": "メッセージをコピー",
  "timeline.copyCode": "コードをコピー",
  "timeline.copyPermalink": "パーマリンクをコピー",
  "timeline.viewSource": "ソースを表示",
  "timeline.removeMessage": "削除",
  "timeline.requestRoomKey": "鍵を要求して再試行",
  "timeline.keyRequestToast": "復号鍵をリクエストしました",
  "timeline.keyRequestAwaiting": "復号鍵を待っています…",
  "timeline.keyRequestStillWaiting": "まだ応答がありません。別の端末がオフラインの場合、後で復号されることがあります。",
  "timeline.keyRequestWithheld": "復号鍵を取得できませんでした。",
  "timeline.keyRequestUnavailable": "要求先の端末はこの復号鍵を保持していません。",
  "timeline.keyRequestUnauthorised": "復号鍵の共有が許可されませんでした。",
  "timeline.keyRequestUnverified": "この端末が未検証のため、復号鍵の共有が拒否されました。",
  "timeline.keyRequestBlacklisted": "この端末は共有対象から除外されています。",
  "timeline.keyRequestRecovered": "復号鍵を受信しました",
  "timeline.recoveryCheckingLocal": "この端末の鍵を確認中…",
  "timeline.recoveryCheckingBackup": "Secure Backup を確認中…",
  "timeline.recoveryRequestingDevices": "認証済み端末に鍵をリクエスト中…",
  "timeline.recoveryRepairingOlm": "暗号化セッションを修復中…",
  "timeline.recoveryWaitingForKey": "鍵を待機中…",
  "timeline.recoveryRetrying": "復号を再試行中…",
  "timeline.recoveryRecovered": "復旧しました",
  "timeline.recoveryTemporarilyFailed": "自動復旧を再試行します。",
  "timeline.recoveryExhausted": "自動復旧ではこの鍵を取得できませんでした。",
  "timeline.recoveryUnrecoverable": "この鍵は復旧できません。",
  "timeline.recoveryGuidanceAnotherDevice": "別の認証済み端末を開いて再試行してください。",
  "timeline.recoveryGuidanceBackup": "Secure Backup が利用できないか不完全です。バックアップの状態を確認して再試行してください。",
  "timeline.recoveryGuidanceSenderMessage": "送信者にこのルームで新しいメッセージを送ってもらい、再試行してください。",
  "timeline.recoveryGuidanceRepost": "送信者に未読の内容を再投稿してもらってください。鍵がなければ元のメッセージは復元できません。",
  "timeline.forwardMessage": "転送",
  "timeline.messageSource": "メッセージソース",
  "timeline.closeMessageSource": "メッセージソースを閉じる",
  "timeline.sourceEventId": "イベントID:",
  "timeline.copyEventId": "イベントIDをコピー",
  "timeline.encryptionDetails": "暗号化情報",
  "timeline.megolmSessionFingerprint": "Megolmセッション識別子",
  "timeline.copyMegolmSessionFingerprint": "Megolmセッション識別子をコピー",
  "timeline.megolmRotationReason": "セッション変更理由",
  "timeline.megolmReasonInitial": "最初のセッション",
  "timeline.megolmReasonExpiredTime": "有効時間の上限に到達",
  "timeline.megolmReasonExpiredMessageCount": "メッセージ数の上限に到達",
  "timeline.megolmReasonMembershipOrDeviceChange": "メンバーまたは端末の変更",
  "timeline.megolmReasonEncryptionSettingsChanged": "暗号化設定の変更",
  "timeline.megolmReasonExplicitDiscard": "手動で破棄",
  "timeline.megolmReasonFullMemberListReload": "メンバー一覧の再読み込み",
  "timeline.megolmReasonRoomSubscription": "ルーム購読の変更",
  "timeline.megolmReasonLimitedSyncResponse": "制限付き同期応答",
  "timeline.megolmReasonKeyShareFailure": "鍵共有失敗後の再試行",
  "timeline.megolmReasonStoreMissing": "セッション記録を利用できません",
  "timeline.megolmReasonInvalidated": "セッションが無効化されました",
  "timeline.megolmReasonUnknown": "理由不明",
  "timeline.megolmReasonNotRetained": "理由を利用できません",
  "timeline.originalEventSource": "元イベントソース",
  "timeline.copyOriginalEventSource": "元イベントソースをコピー",
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
  "timeline.sent": "送信済み",
  "timeline.resendSend": "再送信",
  "timeline.deleteSend": "削除",
  "timeline.cancelSend": "送信をキャンセル",
  "timeline.unsentBar": "未送信のメッセージがあります",
  "timeline.resendAll": "すべて再送信",
  "timeline.cancelAll": "すべてキャンセル",
  "timeline.downloadMedia": "{filename}をダウンロード",
  "timeline.encryptedMedia": "暗号化済み",
  "timeline.mediaDownloadPending": "ダウンロード中…",
  "timeline.mediaDownloadFailed": "ダウンロードに失敗しました",
  "timeline.mediaDownloadRetry": "再試行",
  "timeline.mediaOpenFile": "ファイルを開く",
  "timeline.mediaDetails": "{filename}のメディア詳細を表示",
  "timeline.mediaDetailsTitle": "メディア詳細",
  "timeline.closeMediaDetails": "メディア詳細を閉じる",
  "timeline.mediaViewer": "メディアビューア",
  "timeline.threadReplyCountOne": "返信 1 件",
  "timeline.threadReplyCountMany": "返信 {count} 件",
  "timeline.threadRoot": "スレッド元 {eventId}",
  "timeline.threadSummaryWithBody": "{count}件 · {preview}",
  "timeline.threadSummaryWithPreview": "{count}件 · {sender}: {preview}",
  "timeline.threadSummaryWithSender": "{count}件 · {sender}",
  "timeline.threadNotificationCount": "スレッド通知 · {count}",
  "timeline.typingMany": "{count} 人が入力中",
  "timeline.typingOne": "{user} が入力中",
  "timeline.openThreadSummary": "スレッドを開く、{summary}",
  "timeline.viewReplies": "新しい返信を見る · {count}",
  "workspace.activity": "アクティビティ",
  "workspace.createSpace": "スペースを作成",
  "workspace.explore": "探索",
  "workspace.homeAttention": "{name}、未読メッセージ {unread} 件、招待 {invites} 件",
  "workspace.filters": "フィルター",
  "workspace.home": "ホーム",
  "workspace.invites": "招待",
  "workspace.favourites": "お気に入り",
  "workspace.lowPriority": "低優先度",
  "workspace.newDm": "新しいDM",
  "workspace.notJoined": "未参加",
  "workspace.people": "Direct Messages",
  "workspace.rooms": "ルーム",
  "workspace.resizeRoomList": "ルームリストの幅を変更",
  "workspace.resizeRightPanel": "右パネルの幅を変更",
  "workspace.search": "検索",
  "workspace.searchEverywhere": "すべてを検索",
  "workspace.searchInRoom": "{roomName}内を検索",
  "workspace.searchPlaceholder": "{spaceName}内を検索",
  "workspace.searchScope": "検索範囲",
  "workspace.spaceInfoSettings": "スペース情報と設定",
  "workspace.threads": "スレッド",
  "workspace.userSettings": "ユーザー設定",
  "workspace.workspaces": "ワークスペース",
  "spaceMembers.title": "スペースのメンバー",
  "spaceMembers.search": "スペースのメンバーを検索",
  "spaceMembers.navLabel": "メンバー",
  "spaceMembers.navCount": "参加 {joined} 人 · +{childOnly}",
  "spaceMembers.navAccessible": "メンバー、参加 {joined} 人、子ルームのみ {childOnly} 人",
  "spaceMembers.joinedCount": "参加メンバー {count} 人",
  "spaceMembers.sectionJoined": "スペースのメンバー",
  "spaceMembers.sectionInvited": "招待保留",
  "spaceMembers.sectionChildOnly": "スペース未参加",
  "spaceMembers.childRoomContext": "参加中の子ルーム: {rooms}",
  "spaceMembers.childRoomCountOne": "参加中の子ルーム {count} 個",
  "spaceMembers.childRoomCount": "参加中の子ルーム {count} 個",
  "spaceMembers.invite": "スペースに招待",
  "spaceMembers.invitePending": "招待保留",
  "spaceMembers.inviteFailed": "招待に失敗しました。もう一度お試しください。",
  "spaceMembers.cancelInvite": "招待を取り消す",
  "spaceMembers.cancelInvitePending": "取消中…",
  "spaceMembers.cancelInviteFailed": "招待を取り消せませんでした。もう一度お試しください。",
  "spaceMembers.loadFailed": "メンバーの読み込みに失敗しました。もう一度お試しください。",
  "spaceMembers.syncIncomplete": "一部の子ルームを同期中です",
  "spaceMembers.noResults": "スペースのメンバーが見つかりません",
  "spaceMembers.roleSelect": "{name}のロール",
  "spaceMembers.roleUpdateFailed": "このメンバーのロールを変更できませんでした。もう一度お試しください。",
  "spaceMembers.roleReload": "ロールを再読み込み",
  "spaceMembers.roleConfirmTitle": "ロール変更の確認",
  "spaceMembers.roleConfirmCopy": "{name}のロールを{role}に変更しますか？",
  "spaceMembers.roleConfirmAction": "ロールを変更",
  "threads.empty": "スレッドがありません",
  "threads.error": "スレッドを読み込めませんでした",
  "threads.loading": "スレッドを読み込み中…",
  "threads.open": "スレッドを開く",
  "threads.replyCount": "{count}件の返信",
  "threads.title": "スレッド",
  "settings.searchHistory": "検索履歴",
  "settings.searchHistoryCrawler": "クローラー",
  "settings.searchHistoryPause": "クローラーを一時停止",
  "settings.searchHistoryResume": "クローラーを再開",
  "settings.searchHistoryRebuild": "検索データベースを再構築",
  "settings.searchHistoryRebuildConfirm": "検索データベースを再構築しますか？ローカル検索インデックスを消去し、ルーム履歴を再クロールします。",
  "settings.searchHistorySpeed": "クロール速度",
  "settings.searchHistorySpeedStandard": "標準",
  "settings.searchHistorySpeedFast": "高速",
  "settings.searchHistorySpeedSlow": "低速",
  "settings.searchHistorySpeedPaused": "オフ",
  "settings.searchHistorySpeedCurrent": "選択中",
  "settings.searchHistoryIncludeCaptions": "メディアキャプションをインデックス",
  "settings.searchHistoryIncludeFilenames": "ファイル名をインデックス",
  "settings.searchHistoryActivity": "検索クローラーの動作状況",
  "settings.searchHistoryIndexingProgress": "メッセージ履歴をインデックス中... {completed}/{total}ルーム",
  "settings.searchHistoryPausedProgress": "メッセージ履歴のインデックスは一時停止中です。{completed}/{total}ルームをインデックス済み",
  "settings.searchHistoryActivitySummary": "実行中 {running}、待機 {queued}、完了 {completed}、失敗 {failed}",
  "settings.searchHistoryActivityIdle": "現在インデックス中のルームはありません。",
  "settings.searchHistoryActivityLastIndexed": "最後にインデックスしたルーム: {room}（{age}）。",
  "settings.searchHistoryActivityJustNow": "たった今",
  "settings.searchHistoryActivityMinutesAgo": "{count}分前",
  "settings.searchHistoryActivityHoursAgo": "{count}時間前",
  "settings.searchHistoryActivityDaysAgo": "{count}日前",
  "settings.searchHistoryActivityHint": "処理済みは読み取ったタイムラインイベント数、インデックス済みはローカル検索DBに登録した検索可能メッセージ数です。",
  "settings.searchHistoryRoomStatus": "ルームインデックス状況",
  "settings.searchHistoryRoomIdle": "未開始",
  "settings.searchHistoryRoomQueued": "待機中",
  "settings.searchHistoryRoomRunning": "インデックス中（処理済み: {processed}件、インデックス済み: {indexed}件）",
  "settings.searchHistoryRoomCompleted": "完了（{indexed}件インデックス済み）",
  "settings.searchHistoryRoomFailed": "失敗",
  "settings.searchHistoryStartRoom": "開始",
  "settings.searchHistoryStopRoom": "停止",
  "settings.searchHistoryRoomUnknown": "ルーム"
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
