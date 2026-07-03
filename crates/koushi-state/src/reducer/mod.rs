use crate::{
    action::AppAction,
    effect::{AppEffect, UiEvent},
    state::{
        AccountManagementCapabilities, AccountManagementState, ActivityState, AppState,
        DeviceSessionListState, DirectoryState, E2eeKeyManagementState, E2eeTrustState,
        FilesViewState, FocusedContextState, LocalEncryptionState, NavigationState, QrLoginState,
        SearchState, SessionState, SoftLogoutReauthState, ThreadAttentionState, ThreadPaneState,
        ThreadsListState, TimelinePaneState, compute_room_list_projection,
    },
};

use std::collections::{BTreeMap, BTreeSet};

mod account;
mod activity;
mod avatar;
mod basic_operation;
mod directory;
mod e2ee;
mod live_signals;
mod local_encryption;
mod native_attention;
mod navigation;
mod profile;
mod room;
mod room_management;
mod search;
mod session;
mod settings;
mod sync;
mod thread;
mod timeline;

pub(crate) fn visible_invites_for_ignored_users(
    invites: &[crate::state::InvitePreview],
    ignored_user_ids: &std::collections::BTreeSet<String>,
) -> Vec<crate::state::InvitePreview> {
    invites
        .iter()
        .filter(|invite| {
            invite
                .inviter_user_id
                .as_deref()
                .map(|id| !ignored_user_ids.contains(id))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

pub(crate) fn recompute_room_list_projection(state: &mut AppState) {
    let visible_invites =
        visible_invites_for_ignored_users(&state.invites, &state.profile.ignored_user_ids);
    state.room_list = compute_room_list_projection(
        state.room_list.active_filter,
        state.settings.values.room_list_sort,
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
        &visible_invites,
    );
}

pub fn reduce(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    match action {
        AppAction::AppStarted => session::handle_app_started(state),
        AppAction::RestoreSessionSucceeded(info) | AppAction::LoginSucceeded(info) => {
            session::handle_restore_or_login_succeeded(state, info)
        }
        AppAction::E2eeRecoveryRequired { info, methods } => {
            e2ee::handle_e2ee_recovery_required(state, info, methods)
        }
        AppAction::E2eeRecoverySubmitted(request) => {
            e2ee::handle_e2ee_recovery_submitted(state, request)
        }
        AppAction::E2eeRecoverySucceeded => e2ee::handle_e2ee_recovery_succeeded(state),
        AppAction::E2eeRecoveryFailed { message } => {
            e2ee::handle_e2ee_recovery_failed(state, message)
        }
        AppAction::E2eeRecoveryStateChanged {
            state: recovery_state,
            methods,
        } => e2ee::handle_e2ee_recovery_state_changed(state, recovery_state, methods),
        AppAction::VerificationRequested { request_id, target } => {
            e2ee::handle_verification_requested(state, request_id, target)
        }
        AppAction::VerificationAccepted { request_id } => {
            e2ee::handle_verification_accepted(state, request_id)
        }
        AppAction::VerificationSasPresented { request_id, emojis } => {
            e2ee::handle_verification_sas_presented(state, request_id, emojis)
        }
        AppAction::VerificationConfirmed { request_id } => {
            e2ee::handle_verification_confirmed(state, request_id)
        }
        AppAction::VerificationCancelled { request_id, reason } => {
            e2ee::handle_verification_cancelled(state, request_id, reason)
        }
        AppAction::VerificationCompleted { request_id } => {
            e2ee::handle_verification_completed(state, request_id)
        }
        AppAction::VerificationFailed { request_id, kind } => {
            e2ee::handle_verification_failed(state, request_id, kind)
        }
        AppAction::CrossSigningStatusChanged { status } => {
            e2ee::handle_cross_signing_status_changed(state, status)
        }
        AppAction::BootstrapCrossSigningRequested { request_id } => {
            e2ee::handle_bootstrap_cross_signing_requested(state, request_id)
        }
        AppAction::BootstrapCrossSigningFailed { request_id, kind } => {
            e2ee::handle_bootstrap_cross_signing_failed(state, request_id, kind)
        }
        AppAction::EnableKeyBackupRequested { request_id } => {
            e2ee::handle_enable_key_backup_requested(state, request_id)
        }
        AppAction::KeyBackupEnabled {
            request_id,
            version,
        } => e2ee::handle_key_backup_enabled(state, request_id, version),
        AppAction::KeyBackupFailed { request_id, kind } => {
            e2ee::handle_key_backup_failed(state, request_id, kind)
        }
        AppAction::RestoreKeyBackupRequested {
            request_id,
            version,
        } => e2ee::handle_restore_key_backup_requested(state, request_id, version),
        AppAction::KeyBackupRestoreProgress {
            request_id,
            restored_rooms,
            total_rooms,
        } => {
            e2ee::handle_key_backup_restore_progress(state, request_id, restored_rooms, total_rooms)
        }
        AppAction::KeyBackupRestored {
            request_id,
            version,
        } => e2ee::handle_key_backup_restored(state, request_id, version),
        AppAction::ResetIdentityRequested { request_id } => {
            e2ee::handle_reset_identity_requested(state, request_id)
        }
        AppAction::ResetIdentityAuthRequired {
            request_id,
            auth_type,
        } => e2ee::handle_reset_identity_auth_required(state, request_id, auth_type),
        AppAction::ResetIdentityAuthSubmitted { request_id } => {
            e2ee::handle_reset_identity_auth_submitted(state, request_id)
        }
        AppAction::ResetIdentityCompleted { request_id } => {
            e2ee::handle_reset_identity_completed(state, request_id)
        }
        AppAction::ResetIdentityFailed { request_id, kind } => {
            e2ee::handle_reset_identity_failed(state, request_id, kind)
        }
        AppAction::RoomKeyExportRequested { request_id } => {
            e2ee::handle_room_key_export_requested(state, request_id)
        }
        AppAction::RoomKeyExported {
            request_id,
            exported_sessions,
        } => e2ee::handle_room_key_exported(state, request_id, exported_sessions),
        AppAction::RoomKeyExportFailed { request_id, kind } => {
            e2ee::handle_room_key_export_failed(state, request_id, kind)
        }
        AppAction::RoomKeyImportRequested { request_id } => {
            e2ee::handle_room_key_import_requested(state, request_id)
        }
        AppAction::RoomKeyImported {
            request_id,
            imported_count,
            total_count,
        } => e2ee::handle_room_key_imported(state, request_id, imported_count, total_count),
        AppAction::RoomKeyImportFailed { request_id, kind } => {
            e2ee::handle_room_key_import_failed(state, request_id, kind)
        }
        AppAction::SecureBackupSetupRequested { request_id } => {
            e2ee::handle_secure_backup_setup_requested(state, request_id)
        }
        AppAction::SecureBackupRecoveryKeyReady {
            request_id,
            delivery,
        } => e2ee::handle_secure_backup_recovery_key_ready(state, request_id, delivery),
        AppAction::SecureBackupSetupEnabled { request_id } => {
            e2ee::handle_secure_backup_setup_enabled(state, request_id)
        }
        AppAction::SecureBackupSetupFailed { request_id, kind } => {
            e2ee::handle_secure_backup_setup_failed(state, request_id, kind)
        }
        AppAction::SecureBackupPassphraseChangeRequested { request_id } => {
            e2ee::handle_secure_backup_passphrase_change_requested(state, request_id)
        }
        AppAction::SecureBackupPassphraseChanged {
            request_id,
            delivery,
        } => e2ee::handle_secure_backup_passphrase_changed(state, request_id, delivery),
        AppAction::SecureBackupPassphraseChangeFailed { request_id, kind } => {
            e2ee::handle_secure_backup_passphrase_change_failed(state, request_id, kind)
        }
        AppAction::QrLoginCapabilityCheckRequested { request_id } => {
            e2ee::handle_qr_login_capability_check_requested(state, request_id)
        }
        AppAction::QrLoginUnavailable { request_id } => {
            e2ee::handle_qr_login_unavailable(state, request_id)
        }
        AppAction::QrLoginDisplayRequested { request_id } => {
            e2ee::handle_qr_login_display_requested(state, request_id)
        }
        AppAction::QrLoginScanStarted { request_id } => {
            e2ee::handle_qr_login_scan_started(state, request_id)
        }
        AppAction::QrLoginVerified { request_id } => {
            e2ee::handle_qr_login_verified(state, request_id)
        }
        AppAction::QrLoginFailed { request_id, kind } => {
            e2ee::handle_qr_login_failed(state, request_id, kind)
        }
        AppAction::RestoreSessionNotFound => session::handle_restore_session_not_found(state),
        AppAction::RestoreSessionFailed { message } => {
            session::handle_restore_session_failed(state, message)
        }
        AppAction::LoginSubmitted(request) => session::handle_login_submitted(state, request),
        AppAction::LoginFailed { message } => session::handle_login_failed(state, message),
        AppAction::LoginDiscoveryRequested { homeserver } => {
            session::handle_login_discovery_requested(state, homeserver)
        }
        AppAction::LoginDiscoverySucceeded {
            homeserver,
            flows,
            delegated,
        } => session::handle_login_discovery_succeeded(state, homeserver, flows, delegated),
        AppAction::LoginDiscoveryFailed { homeserver, kind } => {
            session::handle_login_discovery_failed(state, homeserver, kind)
        }
        AppAction::SessionPersistenceFailed { message } => {
            session::handle_session_persistence_failed(state, message)
        }
        AppAction::SessionLocked => session::handle_session_locked(state),
        AppAction::LogoutRequested => session::handle_logout_requested(state),
        AppAction::LogoutFinished => session::handle_logout_finished(state),
        AppAction::SwitchAccountRequested { info } => {
            session::handle_switch_account_requested(state, info)
        }
        AppAction::SoftLogoutReauthRequested { request_id } => {
            session::handle_soft_logout_reauth_requested(state, request_id)
        }
        AppAction::SoftLogoutReauthSucceeded { request_id } => {
            session::handle_soft_logout_reauth_succeeded(state, request_id)
        }
        AppAction::SoftLogoutReauthFailed { request_id, kind } => {
            session::handle_soft_logout_reauth_failed(state, request_id, kind)
        }
        AppAction::DeviceSessionsLoadRequested { request_id } => {
            account::handle_device_sessions_load_requested(state, request_id)
        }
        AppAction::DeviceSessionsLoaded {
            request_id,
            devices,
        } => account::handle_device_sessions_loaded(state, request_id, devices),
        AppAction::DeviceSessionsLoadFailed { request_id, kind } => {
            account::handle_device_sessions_load_failed(state, request_id, kind)
        }
        AppAction::AccountManagementRequested {
            request_id,
            operation,
        } => account::handle_account_management_requested(state, request_id, operation),
        AppAction::AccountManagementUiaRequired {
            request_id,
            flow_id,
            operation,
        } => account::handle_account_management_uia_required(state, request_id, flow_id, operation),
        AppAction::AccountManagementSucceeded {
            request_id,
            operation,
        } => account::handle_account_management_succeeded(state, request_id, operation),
        AppAction::AccountManagementFailed {
            request_id,
            operation,
            kind,
        } => account::handle_account_management_failed(state, request_id, operation, kind),
        AppAction::AccountManagementAuthSubmitted {
            request_id,
            flow_id,
        } => account::handle_account_management_auth_submitted(state, request_id, flow_id),
        AppAction::AccountManagementCapabilitiesLoadRequested => {
            account::handle_account_management_capabilities_load_requested(state)
        }
        AppAction::AccountManagementCapabilitiesLoaded { change_password } => {
            account::handle_account_management_capabilities_loaded(state, change_password)
        }
        AppAction::AccountManagementCapabilitiesLoadFailed => {
            account::handle_account_management_capabilities_load_failed(state)
        }
        AppAction::SettingsLoaded { values } => settings::handle_settings_loaded(state, values),
        AppAction::SettingsLoadFailed { message } => {
            settings::handle_settings_load_failed(state, message)
        }
        AppAction::SettingsUpdateRequested { request_id, patch } => {
            settings::handle_settings_update_requested(state, request_id, patch)
        }
        AppAction::SettingsPersisted { request_id } => {
            settings::handle_settings_persisted(state, request_id)
        }
        AppAction::SettingsPersistFailed {
            request_id,
            message,
        } => settings::handle_settings_persist_failed(state, request_id, message),
        AppAction::RoomUrlPreviewOverrideSet {
            request_id,
            room_id,
            enabled,
        } => settings::handle_room_url_preview_override_set(state, request_id, room_id, enabled),
        AppAction::RoomPreferencesLoaded { preferences } => {
            settings::handle_room_preferences_loaded(state, preferences)
        }
        AppAction::RoomNotificationModeSet {
            request_id,
            room_id,
            mode,
        } => settings::handle_room_notification_mode_set(state, request_id, room_id, mode),
        AppAction::RoomNotificationModeCompleted {
            request_id,
            room_id,
        } => settings::handle_room_notification_mode_completed(state, request_id, room_id),
        AppAction::RoomNotificationModeFailed {
            request_id,
            room_id,
            kind,
        } => settings::handle_room_notification_mode_failed(state, request_id, room_id, kind),
        AppAction::OwnProfileUpdated { profile } => {
            profile::handle_own_profile_updated(state, profile)
        }
        AppAction::UserProfilesUpdated { profiles } => {
            profile::handle_user_profiles_updated(state, profiles)
        }
        AppAction::LocalUserAliasesLoaded { aliases } => {
            profile::handle_local_user_aliases_loaded(state, aliases)
        }
        AppAction::LocalUserAliasUpdateRequested {
            request_id,
            user_id,
            alias,
        } => profile::handle_local_user_alias_update_requested(state, request_id, user_id, alias),
        AppAction::LocalUserAliasUpdateSucceeded { request_id } => {
            profile::handle_local_user_alias_update_succeeded(state, request_id)
        }
        AppAction::LocalUserAliasUpdateFailed {
            request_id,
            message,
        } => profile::handle_local_user_alias_update_failed(state, request_id, message),
        AppAction::IgnoredUsersLoaded { user_ids } => {
            profile::handle_ignored_users_loaded(state, user_ids)
        }
        AppAction::IgnoredUserUpdateRequested {
            request_id,
            user_id,
            ignored,
        } => profile::handle_ignored_user_update_requested(state, request_id, user_id, ignored),
        AppAction::IgnoredUserUpdateSucceeded { request_id } => {
            profile::handle_ignored_user_update_succeeded(state, request_id)
        }
        AppAction::IgnoredUserUpdateFailed {
            request_id,
            user_id,
            ignored,
            message,
        } => {
            profile::handle_ignored_user_update_failed(state, request_id, user_id, ignored, message)
        }
        AppAction::ProfileUpdateRequested {
            request_id,
            request,
        } => profile::handle_profile_update_requested(state, request_id, request),
        AppAction::ProfileUpdateSucceeded {
            request_id,
            profile,
        } => profile::handle_profile_update_succeeded(state, request_id, profile),
        AppAction::ProfileUpdateFailed {
            request_id,
            message,
        } => profile::handle_profile_update_failed(state, request_id, message),
        AppAction::AvatarThumbnailUpdated { mxc_uri, thumbnail } => {
            profile::handle_avatar_thumbnail_updated(state, mxc_uri, thumbnail)
        }
        AppAction::SyncStarted => sync::handle_sync_started(state),
        AppAction::SyncFailed { reason } => sync::handle_sync_failed(state, reason),
        AppAction::SyncReconnecting { reason } => sync::handle_sync_reconnecting(state, reason),
        AppAction::SyncRecovered => sync::handle_sync_recovered(state),
        AppAction::SyncStopped => sync::handle_sync_stopped(state),
        AppAction::SyncModeChanged { mode } => sync::handle_sync_mode_changed(state, mode),
        AppAction::RoomListUpdated { spaces, rooms } => {
            room::handle_room_list_updated(state, spaces, rooms)
        }
        AppAction::RoomListFilterSelected { filter } => {
            room::handle_room_list_filter_selected(state, filter)
        }
        AppAction::RoomListFilterApplied { projection } => {
            room::handle_room_list_filter_applied(state, projection)
        }
        AppAction::RoomTagsUpdated { room_id, tags } => {
            room::handle_room_tags_updated(state, room_id, tags)
        }
        AppAction::RoomTagSet { room_id, tag, info } => {
            room::handle_room_tag_set(state, room_id, tag, info)
        }
        AppAction::RoomTagRemoved { room_id, tag } => {
            room::handle_room_tag_removed(state, room_id, tag)
        }
        AppAction::RoomPinnedEventsUpdated { room_id, pinned } => {
            room::handle_room_pinned_events_updated(state, room_id, pinned)
        }
        AppAction::PinEventRequested {
            request_id,
            room_id,
            event_id,
        } => room::handle_pin_event_requested(state, request_id, room_id, event_id),
        AppAction::PinEventCompleted {
            request_id,
            room_id,
        } => room::handle_pin_event_completed(state, request_id, room_id),
        AppAction::PinEventFailed {
            request_id,
            room_id,
            kind,
        } => room::handle_pin_event_failed(state, request_id, room_id, kind),
        AppAction::UnpinEventRequested {
            request_id,
            room_id,
            event_id,
        } => room::handle_unpin_event_requested(state, request_id, room_id, event_id),
        AppAction::UnpinEventCompleted {
            request_id,
            room_id,
        } => room::handle_unpin_event_completed(state, request_id, room_id),
        AppAction::UnpinEventFailed {
            request_id,
            room_id,
            kind,
        } => room::handle_unpin_event_failed(state, request_id, room_id, kind),
        AppAction::RoomMarkedAsReadRequested {
            request_id,
            room_id,
            event_id,
        } => room::handle_room_marked_as_read_requested(state, request_id, room_id, event_id),
        AppAction::RoomMarkedAsReadSucceeded {
            request_id,
            room_id,
        } => room::handle_room_marked_as_read_succeeded(state, request_id, room_id),
        AppAction::RoomMarkedAsReadFailed {
            request_id,
            room_id,
            kind,
        } => room::handle_room_marked_as_read_failed(state, request_id, room_id, kind),
        AppAction::RoomMarkedAsUnreadRequested {
            request_id,
            room_id,
            unread,
        } => room::handle_room_marked_as_unread_requested(state, request_id, room_id, unread),
        AppAction::RoomMarkedAsUnreadSucceeded {
            request_id,
            room_id,
            unread,
        } => room::handle_room_marked_as_unread_succeeded(state, request_id, room_id, unread),
        AppAction::RoomMarkedAsUnreadFailed {
            request_id,
            room_id,
            kind,
        } => room::handle_room_marked_as_unread_failed(state, request_id, room_id, kind),
        AppAction::DirectoryQueryRequested { request_id, query } => {
            directory::handle_directory_query_requested(state, request_id, query)
        }
        AppAction::DirectoryQuerySucceeded {
            request_id,
            query,
            rooms,
            next_batch,
        } => {
            directory::handle_directory_query_succeeded(state, request_id, query, rooms, next_batch)
        }
        AppAction::DirectoryQueryFailed {
            request_id,
            query,
            kind,
        } => directory::handle_directory_query_failed(state, request_id, query, kind),
        AppAction::DirectoryJoinRequested {
            request_id,
            alias,
            via_server,
        } => directory::handle_directory_join_requested(state, request_id, alias, via_server),
        AppAction::DirectoryJoinSucceeded {
            request_id,
            room_id,
        } => directory::handle_directory_join_succeeded(state, request_id, room_id),
        AppAction::DirectoryJoinFailed {
            request_id,
            alias,
            via_server,
            kind,
        } => directory::handle_directory_join_failed(state, request_id, alias, via_server, kind),
        AppAction::RoomSettingsSnapshotLoaded { room_id, settings } => {
            room_management::handle_room_settings_snapshot_loaded(state, room_id, settings)
        }
        AppAction::RoomSettingUpdateRequested {
            request_id,
            room_id,
            change: _,
        } => room_management::handle_room_setting_update_requested(state, request_id, room_id),
        AppAction::RoomSettingUpdateSucceeded {
            request_id,
            room_id,
            settings,
        } => room_management::handle_room_setting_update_succeeded(
            state, request_id, room_id, settings,
        ),
        AppAction::RoomSettingUpdateFailed {
            request_id,
            room_id,
            kind,
        } => room_management::handle_room_setting_update_failed(state, request_id, room_id, kind),
        AppAction::RoomModerationRequested {
            request_id,
            room_id,
            target_user_id: _,
            action,
            reason: _,
        } => room_management::handle_room_moderation_requested(state, request_id, room_id, action),
        AppAction::RoomModerationSucceeded {
            request_id,
            room_id,
            target_user_id,
            action,
        } => room_management::handle_room_moderation_succeeded(
            state,
            request_id,
            room_id,
            target_user_id,
            action,
        ),
        AppAction::RoomModerationFailed {
            request_id,
            room_id,
            target_user_id: _,
            action: _,
            kind,
        } => room_management::handle_room_moderation_failed(state, request_id, room_id, kind),
        AppAction::RoomMemberRoleUpdateRequested {
            request_id,
            room_id,
            target_user_id: _,
            power_level: _,
        } => room_management::handle_room_member_role_update_requested(state, request_id, room_id),
        AppAction::RoomMemberRoleUpdateSucceeded {
            request_id,
            room_id,
            target_user_id,
            power_level,
        } => room_management::handle_room_member_role_update_succeeded(
            state,
            request_id,
            room_id,
            target_user_id,
            power_level,
        ),
        AppAction::RoomMemberRoleUpdateFailed {
            request_id,
            room_id,
            target_user_id: _,
            kind,
        } => {
            room_management::handle_room_member_role_update_failed(state, request_id, room_id, kind)
        }
        AppAction::ActivityOpened { request_id } => {
            activity::handle_activity_opened(state, request_id)
        }
        AppAction::ActivityClosed => activity::handle_activity_closed(state),
        AppAction::ActivitySnapshotLoaded {
            request_id,
            active_tab,
            recent,
            unread,
            excluded_room_ids,
        } => activity::handle_activity_snapshot_loaded(
            state,
            request_id,
            active_tab,
            recent,
            unread,
            excluded_room_ids,
        ),
        AppAction::ActivityRowsObserved { .. } => activity::handle_activity_rows_observed(state),
        AppAction::ActivityRowsUpdated {
            recent,
            unread,
            excluded_room_ids,
        } => activity::handle_activity_rows_updated(state, recent, unread, excluded_room_ids),
        AppAction::ActivityTabSelected { tab } => {
            activity::handle_activity_tab_selected(state, tab)
        }
        AppAction::ActivityMarkReadRequested { request_id, target } => {
            activity::handle_activity_mark_read_requested(state, request_id, target)
        }
        AppAction::ActivityMarkReadSucceeded {
            request_id,
            cleared_event_ids,
        } => activity::handle_activity_mark_read_succeeded(state, request_id, cleared_event_ids),
        AppAction::ActivityMarkReadFailed {
            request_id,
            target,
            kind,
        } => activity::handle_activity_mark_read_failed(state, request_id, target, kind),
        AppAction::LocalEncryptionProbeRequested { request_id } => {
            local_encryption::handle_local_encryption_probe_requested(state, request_id)
        }
        AppAction::LocalEncryptionHealthChanged { request_id, health } => {
            local_encryption::handle_local_encryption_health_changed(state, request_id, health)
        }
        AppAction::ResetLocalDataRequested { request_id } => {
            local_encryption::handle_reset_local_data_requested(state, request_id)
        }
        AppAction::ResetLocalDataCompleted { request_id } => {
            local_encryption::handle_reset_local_data_completed(state, request_id)
        }
        AppAction::ResetLocalDataFailed { request_id } => {
            local_encryption::handle_reset_local_data_failed(state, request_id)
        }
        AppAction::NativeAttentionUpdated { attention } => {
            native_attention::handle_native_attention_updated(state, attention)
        }
        AppAction::JapaneseCatalogProfileChanged { profile } => {
            native_attention::handle_japanese_catalog_profile_changed(state, profile)
        }
        AppAction::InviteListUpdated { invites } => {
            navigation::handle_invite_list_updated(state, invites)
        }
        AppAction::NavigationLoaded { navigation } => {
            navigation::handle_navigation_loaded(state, navigation)
        }
        AppAction::TimelineScrollAnchorUpdated { room_id, anchor } => {
            navigation::handle_timeline_scroll_anchor_updated(state, room_id, anchor)
        }
        AppAction::EnterAnchoredTimeline { room_id, event_id } => {
            navigation::handle_enter_anchored_timeline(state, room_id, event_id)
        }
        AppAction::ReturnMainTimelineToLive { room_id } => {
            navigation::handle_return_main_timeline_to_live(state, room_id)
        }
        AppAction::SelectSpace { space_id } => navigation::handle_select_space(state, space_id),
        AppAction::ReorderSpaces { space_ids } => {
            navigation::handle_reorder_spaces(state, space_ids)
        }
        AppAction::SelectRoom { room_id } => navigation::handle_select_room(state, room_id),
        AppAction::TimelineSubscribed { room_id } => {
            timeline::handle_timeline_subscribed(state, room_id)
        }
        AppAction::TimelineSubscriptionFailed {
            room_id,
            message: _,
        } => timeline::handle_timeline_subscription_failed(state, room_id),
        AppAction::TimelineBackPaginationRequested { room_id } => {
            timeline::handle_timeline_back_pagination_requested(state, room_id)
        }
        AppAction::TimelineBackPaginationFinished { room_id } => {
            timeline::handle_timeline_back_pagination_finished(state, room_id)
        }
        AppAction::ScheduledSendCapabilityChanged { capability } => {
            timeline::handle_scheduled_send_capability_changed(state, capability)
        }
        AppAction::ScheduledSendsLoaded { scheduled_sends } => {
            timeline::handle_scheduled_sends_loaded(state, scheduled_sends)
        }
        AppAction::ScheduledSendCreated { item } => {
            timeline::handle_scheduled_send_created(state, item)
        }
        AppAction::ScheduledSendRescheduled {
            scheduled_id,
            send_at_ms,
            handle,
        } => timeline::handle_scheduled_send_rescheduled(state, scheduled_id, send_at_ms, handle),
        AppAction::ScheduledSendCancelled { scheduled_id }
        | AppAction::ScheduledSendDispatched { scheduled_id } => {
            timeline::handle_scheduled_send_cancelled_or_dispatched(state, scheduled_id)
        }
        AppAction::UploadStagingChanged { room_id, items } => {
            timeline::handle_upload_staging_changed(state, room_id, items)
        }
        AppAction::UploadStagingCaptionChanged { staged_id, caption } => {
            timeline::handle_upload_staging_caption_changed(state, staged_id, caption)
        }
        AppAction::UploadStagingCompressionChanged {
            staged_id,
            compression_choice,
        } => timeline::handle_upload_staging_compression_changed(
            state,
            staged_id,
            compression_choice,
        ),
        AppAction::UploadStagingCleared { room_id } => {
            timeline::handle_upload_staging_cleared(state, room_id)
        }
        AppAction::MediaGalleryUpdated { room_id, items } => {
            timeline::handle_media_gallery_updated(state, room_id, items)
        }
        AppAction::MediaDownloadUpdated {
            room_id,
            event_id,
            state: download_state,
        } => timeline::handle_media_download_updated(state, room_id, event_id, download_state),
        AppAction::ComposerDraftsLoaded { drafts } => {
            timeline::handle_composer_drafts_loaded(state, drafts)
        }
        AppAction::ComposerDraftChanged { room_id, draft } => {
            timeline::handle_composer_draft_changed(state, room_id, draft)
        }
        AppAction::SendTextSubmitted {
            room_id,
            transaction_id,
            body,
        } => timeline::handle_send_text_submitted(state, room_id, transaction_id, body),
        AppAction::SendTextFinished {
            room_id,
            transaction_id,
        } => timeline::handle_send_text_finished(state, room_id, transaction_id),
        AppAction::SendTextFailed {
            room_id,
            transaction_id,
            message,
        } => timeline::handle_send_text_failed(state, room_id, transaction_id, message),
        AppAction::ComposerReplyTargetSelected { room_id, event_id } => {
            timeline::handle_composer_reply_target_selected(state, room_id, event_id)
        }
        AppAction::ComposerReplyCancelled => timeline::handle_composer_reply_cancelled(state),
        AppAction::ThreadComposerDraftChanged {
            room_id,
            root_event_id,
            draft,
        } => thread::handle_thread_composer_draft_changed(state, room_id, root_event_id, draft),
        AppAction::ThreadReplySubmitted {
            room_id,
            root_event_id,
            transaction_id,
            body: _,
        } => thread::handle_thread_reply_submitted(state, room_id, root_event_id, transaction_id),
        AppAction::ThreadReplyFinished {
            room_id,
            root_event_id,
            transaction_id,
        } => thread::handle_thread_reply_finished(state, room_id, root_event_id, transaction_id),
        AppAction::ThreadReplyFailed {
            room_id,
            root_event_id,
            transaction_id,
            message,
        } => thread::handle_thread_reply_failed(
            state,
            room_id,
            root_event_id,
            transaction_id,
            message,
        ),
        AppAction::OpenThread {
            room_id,
            root_event_id,
        } => thread::handle_open_thread(state, room_id, root_event_id),
        AppAction::ThreadSubscribed {
            room_id,
            root_event_id,
        } => thread::handle_thread_subscribed(state, room_id, root_event_id),
        AppAction::ThreadAttentionUpdated {
            room_id,
            root_event_id,
            notification_count,
            highlight_count,
            live_event_marker_count,
        } => thread::handle_thread_attention_updated(
            state,
            room_id,
            root_event_id,
            notification_count,
            highlight_count,
            live_event_marker_count,
        ),
        AppAction::CloseThread => thread::handle_close_thread(state),
        AppAction::OpenFocusedContext { room_id, event_id } => {
            thread::handle_open_focused_context(state, room_id, event_id)
        }
        AppAction::FocusedContextSubscribed { room_id, event_id } => {
            thread::handle_focused_context_subscribed(state, room_id, event_id)
        }
        AppAction::CloseFocusedContext => thread::handle_close_focused_context(state),
        AppAction::SearchEdited { query, scope } => {
            search::handle_search_edited(state, query, scope)
        }
        AppAction::SearchSubmitted {
            request_id,
            query,
            scope,
        } => search::handle_search_submitted(state, request_id, query, scope),
        AppAction::SearchSucceeded {
            request_id,
            results,
        } => search::handle_search_succeeded(state, request_id, results),
        AppAction::SearchFailed {
            request_id,
            message,
        } => search::handle_search_failed(state, request_id, message),
        AppAction::SearchClosed => search::handle_search_closed(state),
        AppAction::SearchIndexRebuildRequested { request_id: _ } => {
            search::handle_search_index_rebuild_requested(state)
        }
        AppAction::HistoryCrawlStarted {
            request_id: _,
            room_id,
            timestamp_ms,
        } => search::handle_history_crawl_started(state, room_id, timestamp_ms),
        AppAction::HistoryCrawlProgress {
            room_id,
            processed,
            indexed,
            timestamp_ms,
        } => {
            search::handle_history_crawl_progress(state, room_id, processed, indexed, timestamp_ms)
        }
        AppAction::HistoryCrawlCompleted {
            room_id,
            indexed,
            timestamp_ms,
        } => search::handle_history_crawl_completed(state, room_id, indexed, timestamp_ms),
        AppAction::HistoryCrawlFailed {
            room_id,
            kind,
            timestamp_ms,
        } => search::handle_history_crawl_failed(state, room_id, kind, timestamp_ms),
        AppAction::FilesViewOpened {
            request_id,
            scope,
            filter,
            sort,
        } => search::handle_files_view_opened(state, request_id, scope, filter, sort),
        AppAction::FilesViewClosed => search::handle_files_view_closed(state),
        AppAction::FilesViewQueryRequested {
            request_id,
            scope,
            filter,
            sort,
        } => search::handle_files_view_query_requested(state, request_id, scope, filter, sort),
        AppAction::FilesViewQuerySucceeded { request_id, items } => {
            search::handle_files_view_query_succeeded(state, request_id, items)
        }
        AppAction::FilesViewQueryFailed {
            request_id,
            message,
        } => search::handle_files_view_query_failed(state, request_id, message),
        AppAction::FilesViewSelectionChanged { event_id } => {
            search::handle_files_view_selection_changed(state, event_id)
        }
        AppAction::OpenThreadsList {
            request_id,
            room_id,
        } => thread::handle_open_threads_list(state, request_id, room_id),
        AppAction::ThreadsListOpened {
            request_id,
            room_id,
            items,
            end_reached,
        } => thread::handle_threads_list_opened(state, request_id, room_id, items, end_reached),
        AppAction::ThreadsListUpdated {
            request_id,
            room_id,
            items,
            is_paginating,
            end_reached,
        } => thread::handle_threads_list_updated(
            state,
            request_id,
            room_id,
            items,
            is_paginating,
            end_reached,
        ),
        AppAction::ThreadsListPaginationCompleted {
            request_id,
            room_id,
            items,
            end_reached,
        } => thread::handle_threads_list_pagination_completed(
            state,
            request_id,
            room_id,
            items,
            end_reached,
        ),
        AppAction::ThreadsListFailed {
            request_id,
            room_id,
            failure_kind,
        } => thread::handle_threads_list_failed(state, request_id, room_id, failure_kind),
        AppAction::PaginateThreadsList {
            request_id,
            room_id,
        } => thread::handle_paginate_threads_list(state, request_id, room_id),
        AppAction::CloseThreadsList => thread::handle_close_threads_list(state),
        AppAction::ClearError { code } => basic_operation::handle_clear_error(state, code),
        AppAction::BasicOperationRequested {
            request_id,
            request,
        } => basic_operation::handle_basic_operation_requested(state, request_id, request),
        AppAction::BasicOperationSucceeded { request_id } => {
            basic_operation::handle_basic_operation_succeeded(state, request_id)
        }
        AppAction::BasicOperationFailed {
            request_id,
            message,
        } => basic_operation::handle_basic_operation_failed(state, request_id, message),
        AppAction::LiveRoomReceiptsUpdated {
            room_id,
            receipts_by_event,
        } => live_signals::handle_live_room_receipts_updated(state, room_id, receipts_by_event),
        AppAction::FullyReadMarkerUpdated { room_id, event_id } => {
            live_signals::handle_fully_read_marker_updated(state, room_id, event_id)
        }
        AppAction::TypingUsersUpdated { room_id, user_ids } => {
            live_signals::handle_typing_users_updated(state, room_id, user_ids)
        }
        AppAction::PresenceUpdated { user_id, presence } => {
            live_signals::handle_presence_updated(state, user_id, presence)
        }
    }
}

pub(crate) fn is_session_ready(state: &AppState) -> bool {
    matches!(
        state.session,
        SessionState::Ready(_)
            | SessionState::NeedsRecovery { .. }
            | SessionState::Recovering { .. }
    )
}

pub(crate) fn clear_login_failed_errors(state: &mut AppState) -> bool {
    let previous_len = state.errors.len();
    state.errors.retain(|error| error.code != "login_failed");
    state.errors.len() != previous_len
}

pub(crate) fn session_user_id(state: &AppState) -> Option<&str> {
    match &state.session {
        SessionState::Ready(info)
        | SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. } => Some(info.user_id.as_str()),
        _ => None,
    }
}

pub(crate) fn current_session_info(state: &AppState) -> Option<crate::state::SessionInfo> {
    match &state.session {
        SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info) => Some(info.clone()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    }
}

pub(crate) fn clear_session_views(state: &mut AppState) -> Vec<AppEffect> {
    let previous_room_id = state.timeline.room_id.clone();
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;
    let had_search = state.search != SearchState::Closed;
    let had_e2ee_trust = state.e2ee_trust != E2eeTrustState::default();
    let had_e2ee_key_management =
        state.e2ee_trust.key_management != E2eeKeyManagementState::default();
    let had_device_sessions = state.device_sessions != DeviceSessionListState::Idle;
    let had_account_management = state.account_management != AccountManagementState::Idle;
    let had_account_management_capabilities =
        state.account_management_capabilities != AccountManagementCapabilities::default();
    let had_soft_logout_reauth = state.soft_logout_reauth != SoftLogoutReauthState::Idle;
    let had_qr_login = state.qr_login != QrLoginState::Idle;
    let had_live_signals = state.live_signals != Default::default();
    let had_profile = state.profile != Default::default();
    let had_room_interactions = !state.room_interactions.is_empty();
    let had_directory = state.directory != DirectoryState::default();
    let had_activity = state.activity != ActivityState::Closed;
    let had_room_management = state.room_management != Default::default();
    let had_local_encryption = state.local_encryption != LocalEncryptionState::Unknown;
    let had_native_attention = state.native_attention != Default::default();
    let had_files_view = state.files_view != FilesViewState::Closed;
    let had_threads_list = state.threads_list != ThreadsListState::Closed;
    let had_link_preview_settings = !state.link_preview_settings.room_overrides.is_empty();
    let had_room_preferences = !state.room_preferences.rooms.is_empty();
    let had_room_notification_settings = !state.room_notification_settings.is_empty();

    state.navigation = NavigationState::default();
    state.link_preview_settings = Default::default();
    state.room_preferences = Default::default();
    state.spaces.clear();
    state.rooms.clear();
    state.invites.clear();
    state.room_interactions.clear();
    state.composer_drafts = Default::default();
    state.scheduled_sends = Default::default();
    state.upload_staging = Default::default();
    state.media_gallery = Default::default();
    state.directory = DirectoryState::default();
    state.activity = ActivityState::Closed;
    state.room_management = Default::default();
    state.profile = Default::default();
    state.timeline = Default::default();
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.focused_context = FocusedContextState::Closed;
    state.search = SearchState::Closed;
    state.files_view = FilesViewState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.e2ee_trust = E2eeTrustState::default();
    state.device_sessions = DeviceSessionListState::Idle;
    state.account_management = AccountManagementState::Idle;
    state.account_management_capabilities = AccountManagementCapabilities::default();
    state.soft_logout_reauth = SoftLogoutReauthState::Idle;
    state.qr_login = QrLoginState::Idle;
    state.live_signals = Default::default();
    state.local_encryption = LocalEncryptionState::Unknown;
    state.native_attention = Default::default();
    state.room_notification_settings.clear();

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if let Some(room_id) = previous_room_id {
        effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
    }
    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_search {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchChanged));
    }
    if had_e2ee_trust {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged));
    }
    if had_e2ee_key_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged));
    }
    if had_device_sessions {
        effects.push(AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged));
    }
    if had_account_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged));
    }
    if had_account_management_capabilities {
        effects.push(AppEffect::EmitUiEvent(
            UiEvent::AccountManagementCapabilitiesChanged,
        ));
    }
    if had_soft_logout_reauth {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged));
    }
    if had_qr_login {
        effects.push(AppEffect::EmitUiEvent(UiEvent::QrLoginChanged));
    }
    if had_live_signals {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged));
    }
    if had_profile {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ProfileChanged));
    }
    if had_room_interactions {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged));
    }
    if had_directory {
        effects.push(AppEffect::EmitUiEvent(UiEvent::DirectoryChanged));
    }
    if had_activity {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ActivityChanged));
    }
    if had_room_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged));
    }
    if had_local_encryption {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged));
    }
    if had_native_attention {
        effects.push(AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged));
    }
    if had_files_view {
        effects.push(AppEffect::EmitUiEvent(UiEvent::FilesViewChanged));
    }
    if had_threads_list {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
    }
    if had_link_preview_settings || had_room_preferences {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LinkPreviewSettingsChanged));
    }
    if had_room_notification_settings {
        effects.push(AppEffect::EmitUiEvent(
            UiEvent::RoomNotificationSettingsChanged,
        ));
    }
    effects
}

pub(crate) fn refresh_open_room_settings_member_display_projection(
    state: &mut AppState,
    own_user_id: Option<&str>,
) -> bool {
    let Some(settings) = state.room_management.settings.as_mut() else {
        return false;
    };
    crate::state::refresh_room_settings_member_display_projection(
        settings,
        &state.profile,
        own_user_id,
    )
}

pub(crate) fn refresh_open_room_summary_display_projection(
    state: &mut AppState,
    own_user_id: Option<&str>,
) -> bool {
    crate::state::refresh_room_summary_display_projection(
        &mut state.rooms,
        &state.profile,
        own_user_id,
    )
}

pub(crate) fn refresh_native_attention_candidate_display_projection(state: &mut AppState) -> bool {
    let Some(candidate) = state.native_attention.summary.candidate.as_mut() else {
        return false;
    };
    let Some(display_label) = state
        .rooms
        .iter()
        .filter(|room| room.tags.low_priority.is_none())
        .filter_map(|room| {
            crate::state::room_attention_summary(
                room.display_label.clone(),
                room.is_dm,
                room.notification_count,
                room.highlight_count,
                room.unread_count,
            )
        })
        .filter(|summary| {
            summary.kind == candidate.kind
                && summary.unread_count == candidate.unread_count
                && summary.highlight_count == candidate.highlight_count
        })
        .map(|summary| summary.room_display_name)
        .min()
    else {
        return false;
    };
    if candidate.room_display_name == display_label {
        return false;
    }
    candidate.room_display_name = display_label;
    true
}

pub(crate) fn profile_changed_effects(
    room_management_changed: bool,
    room_list_changed: bool,
    native_attention_changed: bool,
) -> Vec<AppEffect> {
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)];
    if room_list_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    if room_management_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged));
    }
    if native_attention_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged));
    }
    effects
}

pub(crate) fn room_exists(state: &AppState, room_id: &str) -> bool {
    state.rooms.iter().any(|room| room.room_id == room_id)
}

pub(crate) fn retain_navigation_room_memory(state: &mut AppState) {
    let valid_pairs = state
        .spaces
        .iter()
        .flat_map(|space| {
            space
                .child_room_ids
                .iter()
                .map(|room_id| (space.space_id.clone(), room_id.clone()))
        })
        .filter(|(_, room_id)| {
            state
                .rooms
                .iter()
                .any(|room| room.room_id == *room_id && !room.is_dm)
        })
        .collect::<BTreeSet<_>>();

    state
        .navigation
        .last_room_by_space_id
        .retain(|space_id, room_id| valid_pairs.contains(&(space_id.clone(), room_id.clone())));
}

pub(crate) fn active_room_left_selected_space(state: &AppState, active_room_id: &str) -> bool {
    let Some(active_space_id) = state.navigation.active_space_id.as_deref() else {
        return false;
    };
    let Some(active_room) = state
        .rooms
        .iter()
        .find(|room| room.room_id == active_room_id)
    else {
        return false;
    };
    if active_room.is_dm {
        return false;
    }

    state
        .spaces
        .iter()
        .find(|space| space.space_id == active_space_id)
        .is_some_and(|space| {
            !space
                .child_room_ids
                .iter()
                .any(|room_id| room_id == active_room_id)
        })
}

pub(crate) fn retarget_active_room_for_selected_space(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    previous_room_id: String,
) {
    let next_room_id = first_room_id_in_active_space(state);
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;

    match next_room_id {
        Some(room_id) => {
            select_active_room_after_room_list_update(state, effects, room_id);
        }
        None => {
            state.navigation.active_room_id = None;
            state.thread = ThreadPaneState::Closed;
            state.thread_attention = ThreadAttentionState::Closed;
            state.timeline = Default::default();
            effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: previous_room_id,
            }));

            if had_thread {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
            }
        }
    }
}

pub(crate) fn select_active_room_after_room_list_update(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    room_id: String,
) {
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;

    state.navigation.active_room_id = Some(room_id.clone());
    state.timeline = TimelinePaneState {
        room_id: Some(room_id.clone()),
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: state.composer_drafts.composer_for_room(&room_id),
        scheduled_send_capability: state.scheduled_sends.capability.clone(),
        scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
        staged_uploads: state.upload_staging.items_for_room(&room_id),
        media_gallery: state.media_gallery.items_for_room(&room_id),
        media_downloads: Default::default(),
    };
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    effects.push(AppEffect::SubscribeTimeline {
        room_id: room_id.clone(),
    });
    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));

    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
}

pub(crate) fn select_active_room_for_navigation(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    room_id: String,
) {
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;
    let had_threads_list = state.threads_list != ThreadsListState::Closed;

    state.navigation.active_room_id = Some(room_id.clone());
    state.timeline = TimelinePaneState {
        room_id: Some(room_id.clone()),
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: state.composer_drafts.composer_for_room(&room_id),
        scheduled_send_capability: state.scheduled_sends.capability.clone(),
        scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
        staged_uploads: state.upload_staging.items_for_room(&room_id),
        media_gallery: state.media_gallery.items_for_room(&room_id),
        media_downloads: Default::default(),
    };
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.focused_context = FocusedContextState::Closed;
    // #161: switching rooms resets the main pane to the live timeline.
    state.navigation.main_timeline_anchor = None;
    effects.push(AppEffect::SubscribeTimeline {
        room_id: room_id.clone(),
    });
    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));

    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_threads_list {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
    }
}

pub(crate) fn clear_active_room_for_navigation(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    previous_room_id: String,
) {
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;
    let had_threads_list = state.threads_list != ThreadsListState::Closed;

    state.navigation.active_room_id = None;
    state.timeline = Default::default();
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.focused_context = FocusedContextState::Closed;
    // #161: clearing the active room resets the main pane to the live timeline.
    state.navigation.main_timeline_anchor = None;
    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
        room_id: previous_room_id,
    }));

    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_threads_list {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
    }
}

pub(crate) fn refresh_timeline_scheduled_sends(state: &mut AppState) {
    state.timeline.scheduled_send_capability = state.scheduled_sends.capability.clone();
    state.timeline.scheduled_sends = state
        .timeline
        .room_id
        .as_deref()
        .map(|room_id| state.scheduled_sends.items_for_room(room_id))
        .unwrap_or_default();
}

pub(crate) fn refresh_timeline_upload_staging(state: &mut AppState) {
    state.timeline.staged_uploads = state
        .timeline
        .room_id
        .as_deref()
        .map(|room_id| state.upload_staging.items_for_room(room_id))
        .unwrap_or_default();
}

pub(crate) fn refresh_timeline_media_gallery(state: &mut AppState) {
    state.timeline.media_gallery = state
        .timeline
        .room_id
        .as_deref()
        .map(|room_id| state.media_gallery.items_for_room(room_id))
        .unwrap_or_default();
}

pub(crate) fn reconcile_space_order(
    space_order: &mut Vec<String>,
    spaces: &[crate::state::SpaceSummary],
) {
    let available_space_ids = spaces
        .iter()
        .map(|space| space.space_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut retained_space_ids = BTreeSet::new();
    space_order.retain(|space_id| {
        available_space_ids.contains(space_id.as_str())
            && retained_space_ids.insert(space_id.clone())
    });
    for space in spaces {
        if retained_space_ids.insert(space.space_id.clone()) {
            space_order.push(space.space_id.clone());
        }
    }
}

pub(crate) fn apply_space_order(spaces: &mut [crate::state::SpaceSummary], space_order: &[String]) {
    let position_by_space_id = space_order
        .iter()
        .enumerate()
        .map(|(position, space_id)| (space_id.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    spaces.sort_by_key(|space| {
        position_by_space_id
            .get(space.space_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

pub(crate) fn is_complete_space_order(
    spaces: &[crate::state::SpaceSummary],
    space_ids: &[String],
) -> bool {
    if spaces.len() != space_ids.len() {
        return false;
    }

    let current_space_ids = spaces
        .iter()
        .map(|space| space.space_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut requested_space_ids = BTreeSet::new();
    for space_id in space_ids {
        if !requested_space_ids.insert(space_id.as_str()) {
            return false;
        }
    }

    current_space_ids == requested_space_ids
}

pub(crate) fn preferred_room_id_in_active_space(state: &AppState) -> Option<String> {
    let active_space_id = state.navigation.active_space_id.as_deref()?;
    preferred_room_id_in_space(state, active_space_id)
}

pub(crate) fn first_default_room_id(state: &AppState) -> Option<String> {
    state
        .rooms
        .iter()
        .find(|room| !room.is_dm)
        .or_else(|| state.rooms.first())
        .map(|room| room.room_id.clone())
}

pub(crate) fn remember_active_room_for_current_space(state: &mut AppState) {
    let Some(space_id) = state.navigation.active_space_id.clone() else {
        return;
    };
    let Some(room_id) = state.navigation.active_room_id.clone() else {
        return;
    };
    if room_belongs_to_space(state, &room_id, &space_id) {
        state
            .navigation
            .last_room_by_space_id
            .insert(space_id, room_id);
    }
}

fn preferred_room_id_in_space(state: &AppState, space_id: &str) -> Option<String> {
    state
        .navigation
        .last_room_by_space_id
        .get(space_id)
        .filter(|room_id| room_belongs_to_space(state, room_id, space_id))
        .cloned()
        .or_else(|| first_room_id_in_space(state, space_id))
}

fn first_room_id_in_active_space(state: &AppState) -> Option<String> {
    let active_space_id = state.navigation.active_space_id.as_deref()?;
    first_room_id_in_space(state, active_space_id)
}

fn first_room_id_in_space(state: &AppState, space_id: &str) -> Option<String> {
    let active_space = state
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)?;

    active_space
        .child_room_ids
        .iter()
        .find_map(|child_room_id| {
            state
                .rooms
                .iter()
                .find(|room| room.room_id == *child_room_id && !room.is_dm)
                .map(|room| room.room_id.clone())
        })
}

fn room_belongs_to_space(state: &AppState, room_id: &str, space_id: &str) -> bool {
    let Some(room) = state.rooms.iter().find(|room| room.room_id == room_id) else {
        return false;
    };
    if room.is_dm {
        return false;
    }

    state
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .is_some_and(|space| {
            space
                .child_room_ids
                .iter()
                .any(|child_room_id| child_room_id == room_id)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        AvatarImage, AvatarThumbnailState, LiveEventReceiptSummary, LiveEventReceipts,
        LiveReadReceipt, MediaTransferProgress, OperationFailureKind, PresenceKind,
        RoomLatestEventSummary, RoomLiveSignals, TimelineMediaDownloadState, UserProfile,
    };

    fn ready_state() -> AppState {
        let mut state = AppState::default();
        state.session = SessionState::Ready(crate::state::SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        });
        state
    }

    fn test_space(space_id: &str) -> crate::state::SpaceSummary {
        crate::state::SpaceSummary {
            space_id: space_id.to_owned(),
            display_name: space_id.to_owned(),
            avatar: None,
            child_room_ids: Vec::new(),
        }
    }

    fn test_avatar(mxc_uri: &str) -> AvatarImage {
        AvatarImage {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: AvatarThumbnailState::NotRequested,
        }
    }

    fn ready_avatar_thumbnail(label: &str) -> AvatarThumbnailState {
        AvatarThumbnailState::Ready {
            source_url: format!("file:///tmp/koushi-test-{label}.png"),
            width: Some(64),
            height: Some(64),
            mime_type: Some("image/png".to_owned()),
        }
    }

    fn test_room(room_id: &str, avatar: Option<AvatarImage>) -> crate::state::RoomSummary {
        crate::state::RoomSummary {
            room_id: room_id.to_owned(),
            display_name: room_id.to_owned(),
            display_label: room_id.to_owned(),
            original_display_label: room_id.to_owned(),
            avatar,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: crate::state::RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }
    }

    fn latest_event(event_id: &str, timestamp_ms: u64) -> RoomLatestEventSummary {
        RoomLatestEventSummary {
            event_id: event_id.to_owned(),
            sender_id: Some("@bob:example.invalid".to_owned()),
            sender_label: Some("Bob".to_owned()),
            sender_avatar: None,
            preview: Some("body".to_owned()),
            timestamp_ms,
        }
    }

    #[test]
    fn avatar_thumbnail_updates_rust_owned_snapshots() {
        let mut state = ready_state();
        let mxc_uri = "mxc://example.invalid/avatar";
        state.profile.own.avatar = Some(test_avatar(mxc_uri));
        state.profile.users.insert(
            "@bob:example.invalid".to_owned(),
            UserProfile {
                user_id: "@bob:example.invalid".to_owned(),
                display_name: Some("Bob".to_owned()),
                display_label: "Bob".to_owned(),
                original_display_label: "Bob".to_owned(),
                mention_search_terms: Vec::new(),
                avatar: Some(test_avatar(mxc_uri)),
            },
        );
        state.rooms = vec![test_room(
            "!room:example.invalid",
            Some(test_avatar(mxc_uri)),
        )];
        state.spaces = vec![crate::state::SpaceSummary {
            avatar: Some(test_avatar(mxc_uri)),
            ..test_space("!space:example.invalid")
        }];
        state.invites = vec![crate::state::InvitePreview {
            room_id: "!invite:example.invalid".to_owned(),
            display_name: "Invite".to_owned(),
            avatar: Some(test_avatar(mxc_uri)),
            topic: None,
            inviter_display_name: None,
            inviter_user_id: None,
            is_dm: false,
        }];

        let thumbnail = ready_avatar_thumbnail("avatar");
        let effects = reduce(
            &mut state,
            AppAction::AvatarThumbnailUpdated {
                mxc_uri: mxc_uri.to_owned(),
                thumbnail: thumbnail.clone(),
            },
        );

        assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::ProfileChanged)));
        assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
        assert_eq!(
            state
                .profile
                .own
                .avatar
                .as_ref()
                .map(|avatar| &avatar.thumbnail),
            Some(&thumbnail)
        );
        assert_eq!(
            state
                .profile
                .users
                .get("@bob:example.invalid")
                .and_then(|profile| profile.avatar.as_ref())
                .map(|avatar| &avatar.thumbnail),
            Some(&thumbnail)
        );
        assert_eq!(
            state.rooms[0]
                .avatar
                .as_ref()
                .map(|avatar| &avatar.thumbnail),
            Some(&thumbnail)
        );
        assert_eq!(
            state.spaces[0]
                .avatar
                .as_ref()
                .map(|avatar| &avatar.thumbnail),
            Some(&thumbnail)
        );
        assert_eq!(
            state.invites[0]
                .avatar
                .as_ref()
                .map(|avatar| &avatar.thumbnail),
            Some(&thumbnail)
        );
    }

    #[test]
    fn room_list_updates_preserve_downloaded_avatar_thumbnails() {
        let mut state = ready_state();
        let mxc_uri = "mxc://example.invalid/avatar";
        let thumbnail = ready_avatar_thumbnail("preserved");
        state.rooms = vec![test_room(
            "!room:example.invalid",
            Some(AvatarImage {
                mxc_uri: mxc_uri.to_owned(),
                thumbnail: thumbnail.clone(),
            }),
        )];

        let effects = reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![test_room(
                    "!room:example.invalid",
                    Some(test_avatar(mxc_uri)),
                )],
            },
        );

        assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
        assert_eq!(
            state.rooms[0]
                .avatar
                .as_ref()
                .map(|avatar| &avatar.thumbnail),
            Some(&thumbnail)
        );
    }

    #[test]
    fn reorder_spaces_persists_and_reapplies_to_room_list_updates() {
        let mut state = ready_state();
        state.spaces = vec![
            test_space("!space-a:example.invalid"),
            test_space("!space-b:example.invalid"),
        ];

        let effects = reduce(
            &mut state,
            AppAction::ReorderSpaces {
                space_ids: vec![
                    "!space-b:example.invalid".to_owned(),
                    "!space-a:example.invalid".to_owned(),
                ],
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        );
        assert_eq!(
            state.navigation.space_order,
            vec!["!space-b:example.invalid", "!space-a:example.invalid"]
        );
        assert_eq!(
            state
                .spaces
                .iter()
                .map(|space| space.space_id.as_str())
                .collect::<Vec<_>>(),
            vec!["!space-b:example.invalid", "!space-a:example.invalid"]
        );

        let effects = reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: vec![
                    test_space("!space-a:example.invalid"),
                    test_space("!space-b:example.invalid"),
                    test_space("!space-c:example.invalid"),
                ],
                rooms: Vec::new(),
            },
        );

        assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
        assert_eq!(
            state.navigation.space_order,
            vec![
                "!space-b:example.invalid",
                "!space-a:example.invalid",
                "!space-c:example.invalid"
            ]
        );
        assert_eq!(
            state
                .spaces
                .iter()
                .map(|space| space.space_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "!space-b:example.invalid",
                "!space-a:example.invalid",
                "!space-c:example.invalid"
            ]
        );
    }

    #[test]
    fn live_signal_actions_update_rust_owned_state() {
        let mut state = ready_state();

        let effects = reduce(
            &mut state,
            AppAction::LiveRoomReceiptsUpdated {
                room_id: "!room:example.invalid".to_owned(),
                receipts_by_event: vec![LiveEventReceipts {
                    event_id: "$event:example.invalid".to_owned(),
                    receipts: vec![LiveReadReceipt {
                        user_id: "@bob:example.invalid".to_owned(),
                        display_name: None,
                        original_display_label: String::new(),
                        avatar: None,
                        timestamp_ms: Some(1_234),
                    }],
                }],
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        assert_eq!(
            state.live_signals.rooms.get("!room:example.invalid"),
            Some(&RoomLiveSignals {
                receipts_by_event: [(
                    "$event:example.invalid".to_owned(),
                    LiveEventReceiptSummary {
                        readers: vec![LiveReadReceipt {
                            user_id: "@bob:example.invalid".to_owned(),
                            display_name: Some("@bob:example.invalid".to_owned()),
                            original_display_label: "@bob:example.invalid".to_owned(),
                            avatar: None,
                            timestamp_ms: Some(1_234),
                        }],
                        total_count: 1,
                        overflow_count: 0,
                    },
                )]
                .into(),
                fully_read_event_id: None,
                typing_user_ids: Vec::new(),
            })
        );

        let effects = reduce(
            &mut state,
            AppAction::FullyReadMarkerUpdated {
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some("$event:example.invalid".to_owned()),
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        assert_eq!(
            state
                .live_signals
                .rooms
                .get("!room:example.invalid")
                .and_then(|room| room.fully_read_event_id.as_deref()),
            Some("$event:example.invalid")
        );

        let mut unread_room = test_room("!room:example.invalid", None);
        unread_room.unread_count = 3;
        unread_room.notification_count = 2;
        unread_room.highlight_count = 1;
        unread_room.marked_unread = true;
        state.rooms = vec![unread_room];

        let effects = reduce(
            &mut state,
            AppAction::FullyReadMarkerUpdated {
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some("$event-2:example.invalid".to_owned()),
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        let room = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!room:example.invalid")
            .expect("room summary should exist");
        assert_eq!(room.unread_count, 3);
        assert_eq!(room.notification_count, 2);
        assert_eq!(room.highlight_count, 1);
        assert!(room.marked_unread);

        let effects = reduce(
            &mut state,
            AppAction::TypingUsersUpdated {
                room_id: "!room:example.invalid".to_owned(),
                user_ids: vec![
                    "@carol:example.invalid".to_owned(),
                    "@bob:example.invalid".to_owned(),
                    "@bob:example.invalid".to_owned(),
                ],
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        assert_eq!(
            state
                .live_signals
                .rooms
                .get("!room:example.invalid")
                .map(|room| room.typing_user_ids.clone()),
            Some(vec![
                "@bob:example.invalid".to_owned(),
                "@carol:example.invalid".to_owned(),
            ])
        );

        let effects = reduce(
            &mut state,
            AppAction::PresenceUpdated {
                user_id: "@bob:example.invalid".to_owned(),
                presence: PresenceKind::Away,
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        assert_eq!(
            state.live_signals.presence.get("@bob:example.invalid"),
            Some(&PresenceKind::Away)
        );
    }

    #[test]
    fn room_list_update_does_not_reintroduce_stale_unread_after_fully_read_marker() {
        let mut state = ready_state();
        let latest_event = latest_event("$latest:example.invalid", 42);
        let mut room = test_room("!room:example.invalid", None);
        room.latest_event = Some(latest_event.clone());
        room.last_activity_ms = 42;
        state.rooms = vec![room];

        reduce(
            &mut state,
            AppAction::FullyReadMarkerUpdated {
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some("$latest:example.invalid".to_owned()),
            },
        );
        reduce(
            &mut state,
            AppAction::RoomMarkedAsReadSucceeded {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
            },
        );

        let mut stale_room = test_room("!room:example.invalid", None);
        stale_room.unread_count = 2;
        stale_room.notification_count = 2;
        stale_room.highlight_count = 1;
        stale_room.marked_unread = true;
        stale_room.latest_event = Some(latest_event);
        stale_room.last_activity_ms = 42;
        reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![stale_room],
            },
        );

        let room = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!room:example.invalid")
            .expect("room summary should exist");
        assert_eq!(room.unread_count, 0);
        assert_eq!(room.notification_count, 0);
        assert_eq!(room.highlight_count, 0);
        assert!(!room.marked_unread);
    }

    #[test]
    fn room_list_update_suppresses_stale_unread_when_read_marker_event_differs_from_latest_event() {
        let mut state = ready_state();
        let latest_event = latest_event("$room-summary-latest:example.invalid", 42);
        let mut room = test_room("!room:example.invalid", None);
        room.latest_event = Some(latest_event.clone());
        room.last_activity_ms = 42;
        state.rooms = vec![room];

        reduce(
            &mut state,
            AppAction::FullyReadMarkerUpdated {
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some("$visible-read-event:example.invalid".to_owned()),
            },
        );
        reduce(
            &mut state,
            AppAction::RoomMarkedAsReadSucceeded {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
            },
        );

        let mut stale_room = test_room("!room:example.invalid", None);
        stale_room.unread_count = 1;
        stale_room.notification_count = 1;
        stale_room.latest_event = Some(latest_event);
        stale_room.last_activity_ms = 42;
        reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![stale_room],
            },
        );

        let room = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!room:example.invalid")
            .expect("room summary should exist");
        assert_eq!(room.unread_count, 0);
        assert_eq!(room.notification_count, 0);
    }

    #[test]
    fn room_list_update_preserves_unread_when_latest_event_changes_after_local_read() {
        let mut state = ready_state();
        let mut room = test_room("!room:example.invalid", None);
        room.latest_event = Some(latest_event("$old-latest:example.invalid", 42));
        room.last_activity_ms = 42;
        state.rooms = vec![room];

        reduce(
            &mut state,
            AppAction::FullyReadMarkerUpdated {
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some("$visible-read-event:example.invalid".to_owned()),
            },
        );
        reduce(
            &mut state,
            AppAction::RoomMarkedAsReadSucceeded {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
            },
        );

        let mut new_unread_room = test_room("!room:example.invalid", None);
        new_unread_room.unread_count = 1;
        new_unread_room.notification_count = 1;
        new_unread_room.latest_event = Some(latest_event("$new-latest:example.invalid", 43));
        new_unread_room.last_activity_ms = 43;
        reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![new_unread_room],
            },
        );

        let room = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!room:example.invalid")
            .expect("room summary should exist");
        assert_eq!(room.unread_count, 1);
        assert_eq!(room.notification_count, 1);
    }

    #[test]
    fn live_signals_clear_with_session_views() {
        let mut state = ready_state();
        state.live_signals.rooms.insert(
            "!room:example.invalid".to_owned(),
            RoomLiveSignals {
                fully_read_event_id: Some("$event:example.invalid".to_owned()),
                ..RoomLiveSignals::default()
            },
        );
        state
            .live_signals
            .presence
            .insert("@bob:example.invalid".to_owned(), PresenceKind::Online);

        let effects = reduce(&mut state, AppAction::LogoutRequested);

        assert!(state.live_signals.rooms.is_empty());
        assert!(state.live_signals.presence.is_empty());
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)
            ))
        );
    }

    #[test]
    fn live_read_receipts_project_reader_profiles_order_and_overflow() {
        let mut state = ready_state();
        state.profile.users.insert(
            "@alice:example.invalid".to_owned(),
            UserProfile {
                user_id: "@alice:example.invalid".to_owned(),
                display_name: Some("Alice".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.invalid/alice".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            },
        );
        state.profile.users.insert(
            "@bob:example.invalid".to_owned(),
            UserProfile {
                user_id: "@bob:example.invalid".to_owned(),
                display_name: Some("Bob".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.invalid/bob".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            },
        );
        state.profile.users.insert(
            "@carol:example.invalid".to_owned(),
            UserProfile {
                user_id: "@carol:example.invalid".to_owned(),
                display_name: Some("Carol".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            },
        );

        let effects = reduce(
            &mut state,
            AppAction::LiveRoomReceiptsUpdated {
                room_id: "!room:example.invalid".to_owned(),
                receipts_by_event: vec![LiveEventReceipts {
                    event_id: "$event:example.invalid".to_owned(),
                    receipts: vec![
                        LiveReadReceipt {
                            user_id: "@alice:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(1_000),
                        },
                        LiveReadReceipt {
                            user_id: "@bob:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(3_000),
                        },
                        LiveReadReceipt {
                            user_id: "@carol:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(2_000),
                        },
                        LiveReadReceipt {
                            user_id: "@dana:example.invalid".to_owned(),
                            display_name: Some("Dana".to_owned()),
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(4_000),
                        },
                        LiveReadReceipt {
                            user_id: "@alice:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(5_000),
                        },
                    ],
                }],
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        let summary = state
            .live_signals
            .rooms
            .get("!room:example.invalid")
            .and_then(|room| room.receipts_by_event.get("$event:example.invalid"))
            .expect("receipt projection");
        // The session user (@alice) is excluded from the readers list — own
        // receipts must never appear in the displayed readers or affect counts.
        assert_eq!(summary.total_count, 3);
        assert_eq!(summary.overflow_count, 0);
        assert_eq!(
            summary
                .readers
                .iter()
                .map(|receipt| (
                    receipt.user_id.as_str(),
                    receipt.display_name.as_deref(),
                    receipt.timestamp_ms,
                    receipt
                        .avatar
                        .as_ref()
                        .map(|avatar| avatar.mxc_uri.as_str()),
                ))
                .collect::<Vec<_>>(),
            vec![
                ("@dana:example.invalid", Some("Dana"), Some(4_000), None),
                (
                    "@bob:example.invalid",
                    Some("Bob"),
                    Some(3_000),
                    Some("mxc://example.invalid/bob"),
                ),
                ("@carol:example.invalid", Some("Carol"), Some(2_000), None),
            ]
        );
    }

    #[test]
    fn media_download_updated_stores_state_for_active_room() {
        let mut state = ready_state();
        state.timeline.room_id = Some("!r:example.invalid".to_owned());

        let effects = reduce(
            &mut state,
            AppAction::MediaDownloadUpdated {
                room_id: "!r:example.invalid".to_owned(),
                event_id: "$ev:example.invalid".to_owned(),
                state: TimelineMediaDownloadState::Pending {
                    progress: Some(MediaTransferProgress {
                        current: 3,
                        total: 10,
                    }),
                },
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "!r:example.invalid".to_owned(),
            })]
        );
        assert_eq!(state.timeline.media_downloads.len(), 1);
        let download = state
            .timeline
            .media_downloads
            .get("$ev:example.invalid")
            .expect("download entry");
        assert!(matches!(
            download,
            TimelineMediaDownloadState::Pending {
                progress: Some(MediaTransferProgress {
                    current: 3,
                    total: 10
                })
            }
        ));
    }

    #[test]
    fn media_download_updated_ignored_for_inactive_room() {
        let mut state = ready_state();
        state.timeline.room_id = Some("!r:example.invalid".to_owned());

        let effects = reduce(
            &mut state,
            AppAction::MediaDownloadUpdated {
                room_id: "!other:example.invalid".to_owned(),
                event_id: "$ev:example.invalid".to_owned(),
                state: TimelineMediaDownloadState::Ready {
                    source_url: "/tmp/x.png".to_owned(),
                    width: Some(100),
                    height: Some(100),
                    mime_type: Some("image/png".to_owned()),
                },
            },
        );

        assert!(effects.is_empty());
        assert!(state.timeline.media_downloads.is_empty());
    }

    #[test]
    fn media_download_updated_ignored_without_ready_session() {
        let mut state = AppState::default();
        state.timeline.room_id = Some("!r:example.invalid".to_owned());

        let effects = reduce(
            &mut state,
            AppAction::MediaDownloadUpdated {
                room_id: "!r:example.invalid".to_owned(),
                event_id: "$ev:example.invalid".to_owned(),
                state: TimelineMediaDownloadState::Failed {
                    failure_kind: OperationFailureKind::Network,
                },
            },
        );

        assert!(effects.is_empty());
        assert!(state.timeline.media_downloads.is_empty());
    }
}
