use crate::{
    action::AppAction,
    effect::{AppEffect, UiEvent},
    state::{
        AccountManagementCapabilities, AccountManagementState, ActivityState, AppState,
        DirectoryState, E2eeKeyManagementState, E2eeTrustState, FilesViewState,
        FocusedContextState, InviteWorkflowState, LocalEncryptionState, NavigationState,
        QrLoginState, SearchState, SessionState, SoftLogoutReauthState, SpaceConversationSurface,
        SpaceNavigationSelection, ThreadAttentionState, ThreadPaneState, ThreadsListState,
        TimelinePaneState, VerificationFlowState, compute_room_list_projection,
    },
};

use std::collections::{BTreeMap, BTreeSet};

mod account;
mod activity;
mod avatar;
mod basic_operation;
mod directory;
mod e2ee;
mod invite_workflow;
mod live_signals;
mod local_encryption;
mod mention;
mod native_attention;
mod navigation;
mod profile;
mod room;
mod room_management;
mod search;
mod session;
mod session_status;
mod settings;
mod sliding_sync;
mod space_members;
mod submission;
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
    let readiness = state.room_list.readiness;
    let visible_invites =
        visible_invites_for_ignored_users(&state.invites, &state.profile.ignored_user_ids);
    state.room_list = compute_room_list_projection(
        state.room_list.active_filter,
        state.settings.values.room_list_sort,
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
        &state.room_notification_settings,
        &visible_invites,
    );
    state.room_list.readiness = readiness;
}

pub(crate) fn clear_stale_verification_flow(state: &mut AppState) -> bool {
    if matches!(state.e2ee_trust.verification, VerificationFlowState::Idle) {
        return false;
    }
    state.e2ee_trust.verification = VerificationFlowState::Idle;
    true
}

pub fn reduce(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    match action {
        AppAction::AppStarted => session::handle_app_started(state),
        AppAction::RestoreSessionRequested => session::handle_restore_session_requested(state),
        action @ (AppAction::SlidingSyncCapabilityCheckStarted { .. }
        | AppAction::SlidingSyncCapabilityCheckCompleted { .. }
        | AppAction::SlidingSyncCapabilityRetryAccepted { .. }
        | AppAction::SlidingSyncCapabilityRevalidationStarted { .. }
        | AppAction::SlidingSyncCapabilityRevalidationCompleted { .. }) => {
            sliding_sync::reduce(state, action)
        }
        AppAction::RestoreSessionSucceeded(info) => {
            session::handle_restore_session_succeeded(state, info)
        }
        AppAction::LoginSucceeded { attempt_id, info } => {
            session::handle_login_succeeded(state, attempt_id, info)
        }
        AppAction::CurrentDeviceTrustChanged(trust) => {
            if matches!(state.session, SessionState::Ready(_))
                && matches!(
                    trust,
                    crate::state::CurrentDeviceTrustState::Unknown
                        | crate::state::CurrentDeviceTrustState::Unverified
                )
            {
                session_status::reset(state);
            }
            session::handle_current_device_trust_changed(state, trust)
        }
        AppAction::SecureBackupGateChanged(gate) => {
            session::handle_secure_backup_gate_changed(state, gate)
        }
        AppAction::AuthoritativeDeviceTrustChanged { trust, .. } => {
            if matches!(state.session, SessionState::Ready(_))
                && matches!(
                    trust,
                    crate::state::CurrentDeviceTrustState::Unknown
                        | crate::state::CurrentDeviceTrustState::Unverified
                )
            {
                session_status::reset(state);
            }
            session::handle_authoritative_device_trust_changed(state, trust)
        }
        AppAction::VerificationMethodsDiscovered(gate) => {
            session::handle_verification_methods_discovered(state, gate)
        }
        AppAction::VerificationMethodDiscoveryFailed { kind, .. } => {
            session::handle_verification_method_discovery_failed(state, kind)
        }
        AppAction::VerificationMethodDiscoveryRetryStarted { .. } => {
            session::handle_verification_method_discovery_retry_started(state)
        }
        AppAction::VerificationMethodSubmitted { method, flow_id } => {
            session::handle_verification_method_submitted(state, method, flow_id)
        }
        AppAction::GateSasPresented { flow_id, emojis } => {
            e2ee::handle_gate_sas_presented(state, flow_id, emojis)
        }
        AppAction::VerificationGateAttemptFailed { flow_id, kind } => {
            session::handle_verification_gate_attempt_failed(state, flow_id, kind)
        }
        AppAction::DeviceCleanupStartRequested { request_id } => {
            session::handle_device_cleanup_start_requested(state, request_id)
        }
        AppAction::DeviceCleanupRemoteStarted {
            request_id,
            auth_mode,
        } => session::handle_device_cleanup_remote_started(state, request_id, auth_mode),
        AppAction::DeviceCleanupUiaRequired {
            request_id,
            flow_id,
        } => session::handle_device_cleanup_uia_required(state, request_id, flow_id),
        AppAction::DeviceCleanupUiaSubmitted {
            request_id,
            flow_id,
        } => session::handle_device_cleanup_uia_submitted(state, request_id, flow_id),
        AppAction::DeviceCleanupRemoteSettled {
            request_id,
            outcome,
        } => session::handle_device_cleanup_remote_settled(state, request_id, outcome),
        AppAction::DeviceCleanupRemoteFailed {
            request_id,
            auth_mode,
            kind,
        } => session::handle_device_cleanup_remote_failed(state, request_id, auth_mode, kind),
        AppAction::DeviceCleanupEraseLocalAnywayRequested { request_id } => {
            session::handle_device_cleanup_erase_local_anyway_requested(state, request_id)
        }
        AppAction::DeviceCleanupLocalResetFailed { request_id, kind } => {
            session::handle_device_cleanup_local_reset_failed(state, request_id, kind)
        }
        AppAction::DeviceCleanupCompleted { request_id } => {
            session::handle_device_cleanup_completed(state, request_id)
        }
        AppAction::VerificationSessionRejected { reason } => {
            session::handle_verification_session_rejected(state, reason)
        }
        AppAction::BootstrapRecoveryKeyDelivered { flow_id } => {
            session::handle_bootstrap_recovery_key_delivered(state, flow_id)
        }
        AppAction::BootstrapRecoveryKeyDeliveryFailed { flow_id, kind } => {
            session::handle_bootstrap_recovery_key_delivery_failed(state, flow_id, kind)
        }
        AppAction::BootstrapRecoverySavedConfirmed { flow_id } => {
            session::handle_bootstrap_recovery_saved_confirmed(state, flow_id)
        }
        AppAction::ProvisionalSessionDiscarded => {
            session_status::reset(state);
            session::handle_provisional_session_discarded(state)
        }
        AppAction::E2eeRecoveryRequired { info, methods } => {
            e2ee::handle_e2ee_recovery_required(state, info, methods)
        }
        AppAction::E2eeRecoverySubmitted { flow_id, request } => {
            e2ee::handle_e2ee_recovery_submitted(state, flow_id, request)
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
        AppAction::ResetIdentityCancelled { request_id } => {
            e2ee::handle_reset_identity_cancelled(state, request_id)
        }
        AppAction::ResetIdentityTimedOut { request_id } => {
            e2ee::handle_reset_identity_timed_out(state, request_id)
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
        AppAction::SecureBackupSetupRequested { request_id, intent } => {
            e2ee::handle_secure_backup_setup_requested(state, request_id, intent)
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
        AppAction::LoginSubmitted {
            attempt_id,
            request,
        } => session::handle_login_submitted(state, attempt_id, request),
        AppAction::AuthenticationStarted {
            attempt_id,
            homeserver,
        } => session::handle_authentication_started(state, attempt_id, homeserver),
        AppAction::LoginFailed {
            attempt_id,
            message,
        } => session::handle_login_failed(state, attempt_id, message),
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
        AppAction::ActiveSessionAccountManagementUrlResolved { info, url } => {
            session::handle_active_session_account_management_url_resolved(state, info, url)
        }
        AppAction::SessionPersistenceFailed { message } => {
            session::handle_session_persistence_failed(state, message)
        }
        AppAction::SessionLocked => {
            if matches!(state.session, SessionState::Ready(_)) {
                session_status::reset(state);
            }
            session::handle_session_locked(state)
        }
        AppAction::SessionAuthenticationInvalidated { soft_logout } => {
            if matches!(state.session, SessionState::Ready(_)) {
                session_status::reset(state);
            }
            session::handle_session_authentication_invalidated(state, soft_logout)
        }
        AppAction::LogoutRequested => {
            session_status::reset(state);
            session::handle_logout_requested(state)
        }
        AppAction::LogoutFinished => session::handle_logout_finished(state),
        AppAction::SwitchAccountRequested { info } => {
            session_status::reset(state);
            session::handle_switch_account_requested(state, info)
        }
        AppAction::SoftLogoutReauthRequested { request_id } => {
            session::handle_soft_logout_reauth_requested(state, request_id)
        }
        AppAction::SoftLogoutReauthSucceeded { request_id } => {
            session::handle_soft_logout_reauth_succeeded(state, request_id)
        }
        AppAction::SoftLogoutReauthSessionInstalled { request_id, info } => {
            session::handle_soft_logout_reauth_session_installed(state, request_id, info)
        }
        AppAction::SoftLogoutReauthFailed { request_id, kind } => {
            session::handle_soft_logout_reauth_failed(state, request_id, kind)
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
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id,
            trigger,
        } => session_status::handle_refresh_requested(state, request_id, trigger),
        AppAction::CurrentSessionStatusRefreshed {
            request_id,
            details,
        } => session_status::handle_refreshed(state, request_id, details),
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id,
            kind,
            checked_at_ms,
        } => session_status::handle_refresh_failed(state, request_id, kind, checked_at_ms),
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
        AppAction::SpaceMembersLoadRequested {
            request_id,
            space_id,
            generation,
        } => space_members::handle_load_requested(state, request_id, space_id, generation),
        AppAction::SpaceMembersLoaded {
            request_id,
            projection,
        } => space_members::handle_loaded(state, request_id, projection),
        AppAction::SpaceMembersProfilesObserved {
            request_id,
            profiles,
        } => space_members::handle_profiles_observed(state, request_id, profiles),
        AppAction::SpaceMembersProjectionReconciled {
            request_id,
            projection,
            profiles,
        } => space_members::handle_projection_reconciled(state, request_id, projection, profiles),
        AppAction::SpaceMembersBackgroundProjectionReconciled {
            request_id,
            space_id,
            generation,
            projection,
            profiles,
        } => space_members::handle_background_projection_reconciled(
            state, request_id, space_id, generation, projection, profiles,
        ),
        AppAction::SpaceMemberRoleUpdateRequested {
            request_id,
            space_id,
            user_id,
            generation,
            expected_power_levels_revision,
            expected_power_level,
            power_level,
            confirmed,
        } => space_members::handle_role_update_requested(
            state,
            request_id,
            space_id,
            user_id,
            generation,
            expected_power_levels_revision,
            expected_power_level,
            power_level,
            confirmed,
        ),
        AppAction::SpaceMemberRoleUpdateSettled {
            request_id,
            space_id,
            user_id,
            generation,
            outcome,
            sent_revision,
            projection,
        } => space_members::handle_role_update_settled(
            state,
            request_id,
            space_id,
            user_id,
            generation,
            outcome,
            sent_revision,
            projection,
        ),
        AppAction::SpaceMembersLoadFailed {
            request_id,
            space_id,
            generation,
            kind,
        } => space_members::handle_load_failed(state, request_id, space_id, generation, kind),
        AppAction::SpaceMemberInviteRequested {
            request_id,
            space_id,
            user_id,
            generation,
        } => {
            space_members::handle_invite_requested(state, request_id, space_id, user_id, generation)
        }
        AppAction::SpaceMemberInviteSettled {
            request_id,
            space_id,
            user_id,
            generation,
            outcome,
        } => space_members::handle_invite_settled(
            state, request_id, space_id, user_id, generation, outcome,
        ),
        AppAction::SpaceMemberInviteCancellationRequested {
            request_id,
            space_id,
            user_id,
            generation,
        } => space_members::handle_cancellation_requested(
            state, request_id, space_id, user_id, generation,
        ),
        AppAction::SpaceMemberInviteCancellationSettled {
            request_id,
            space_id,
            user_id,
            generation,
            outcome,
        } => space_members::handle_cancellation_settled(
            state, request_id, space_id, user_id, generation, outcome,
        ),
        AppAction::MentionCandidatesDemanded {
            request_id,
            generation,
            room_id,
            surface,
            query,
        } => mention::handle_demanded(state, request_id, generation, room_id, surface, query),
        AppAction::MentionCandidatesProjected {
            request_id,
            generation,
            room_id,
            surface,
            query,
            completeness,
            candidates,
            room_mention_allowed,
        } => mention::handle_projected(
            state,
            request_id,
            generation,
            room_id,
            surface,
            query,
            completeness,
            candidates,
            room_mention_allowed,
        ),
        AppAction::MentionCandidatesFailed {
            request_id,
            generation,
            room_id,
            surface,
            query,
            kind,
        } => mention::handle_failed(state, request_id, generation, room_id, surface, query, kind),
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
        AppAction::SyncStatusChanged { generation, status } => {
            sync::handle_sync_status_changed(state, generation, status)
        }
        AppAction::RoomListUpdated { spaces, rooms } => {
            room::handle_room_list_updated(state, spaces, rooms)
        }
        AppAction::RoomListBootstrapStarted { generation, source } => {
            room::handle_room_list_bootstrap_started(state, generation, source)
        }
        AppAction::RoomListSnapshotProvisional {
            generation,
            source,
            spaces,
            rooms,
            invites,
        } => room::handle_room_list_snapshot_provisional(
            state, generation, source, spaces, rooms, invites,
        ),
        AppAction::RoomListSnapshotAuthoritative {
            generation,
            source,
            spaces,
            rooms,
            invites,
        } => room::handle_room_list_snapshot_authoritative(
            state, generation, source, spaces, rooms, invites,
        ),
        AppAction::RoomListBootstrapFailed {
            generation,
            source,
            kind,
        } => room::handle_room_list_bootstrap_failed(state, generation, source, kind),
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
        AppAction::DirectoryPreviewRequested {
            request_id,
            room_id_or_alias,
            via_servers,
        } => directory::handle_directory_preview_requested(
            state,
            request_id,
            room_id_or_alias,
            via_servers,
        ),
        AppAction::DirectoryPreviewLoaded { request_id, room } => {
            directory::handle_directory_preview_loaded(state, request_id, room)
        }
        AppAction::DirectoryPreviewFailed {
            request_id,
            room_id_or_alias,
            via_servers,
            kind,
        } => directory::handle_directory_preview_failed(
            state,
            request_id,
            room_id_or_alias,
            via_servers,
            kind,
        ),
        AppAction::DirectoryPreviewDismissed => {
            directory::handle_directory_preview_dismissed(state)
        }
        AppAction::DirectoryJoinRequested {
            request_id,
            room_id_or_alias,
            via_servers,
        } => directory::handle_directory_join_requested(
            state,
            request_id,
            room_id_or_alias,
            via_servers,
        ),
        AppAction::DirectoryJoinSucceeded {
            request_id,
            room_id,
        } => directory::handle_directory_join_succeeded(state, request_id, room_id),
        AppAction::DirectoryJoinFailed {
            request_id,
            room_id_or_alias,
            via_servers,
            kind,
        } => directory::handle_directory_join_failed(
            state,
            request_id,
            room_id_or_alias,
            via_servers,
            kind,
        ),
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
        AppAction::CanonicalActivityWindowReconciled { .. } => {
            activity::handle_canonical_activity_window_reconciled(state)
        }
        AppAction::ActivityResolutionRowsObserved { .. } => {
            activity::handle_activity_rows_observed(state)
        }
        AppAction::ActivityRowsUpdated {
            recent,
            unread,
            excluded_room_ids,
        } => activity::handle_activity_rows_updated(state, recent, unread, excluded_room_ids),
        AppAction::ActivityResolutionStarted {
            generation,
            unresolved_room_count,
        } => activity::handle_activity_resolution_started(state, generation, unresolved_room_count),
        AppAction::ActivityResolutionSucceeded { generation } => {
            activity::handle_activity_resolution_succeeded(state, generation)
        }
        AppAction::ActivityResolutionFailed {
            generation,
            unresolved_room_count,
            kind,
        } => activity::handle_activity_resolution_failed(
            state,
            generation,
            unresolved_room_count,
            kind,
        ),
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
        AppAction::NativeWindowFocusChanged {
            focused,
            observation_generation,
        } => native_attention::handle_native_window_focus_changed(
            state,
            focused,
            observation_generation,
        ),
        AppAction::NativeAttentionDispatchStarted { dispatch_id } => {
            native_attention::handle_dispatch_started(state, dispatch_id)
        }
        AppAction::NativeAttentionDispatchSettled {
            dispatch_id,
            outcome,
        } => native_attention::handle_dispatch_settled(state, dispatch_id, outcome),
        AppAction::JapaneseCatalogProfileChanged { profile } => {
            native_attention::handle_japanese_catalog_profile_changed(state, profile)
        }
        AppAction::InviteListUpdated { invites } => {
            navigation::handle_invite_list_updated(state, invites)
        }
        AppAction::InviteWorkflowOpened { room_id } => {
            invite_workflow::handle_invite_workflow_opened(state, room_id)
        }
        AppAction::InviteWorkflowClosed => invite_workflow::handle_invite_workflow_closed(state),
        AppAction::InviteTargetQueryChanged { room_id, query } => {
            invite_workflow::handle_invite_target_query_changed(state, room_id, query)
        }
        AppAction::InviteScopeSelected { room_id, scope } => {
            invite_workflow::handle_invite_scope_selected(state, room_id, scope)
        }
        AppAction::InviteTargetSelected { room_id, user_id } => {
            invite_workflow::handle_invite_target_selected(state, room_id, user_id)
        }
        AppAction::InviteTargetRemoved { user_id } => {
            invite_workflow::handle_invite_target_removed(state, user_id)
        }
        AppAction::InviteBatchRequested {
            request_id,
            room_id,
            user_ids,
            scope,
        } => invite_workflow::handle_invite_batch_requested(
            state, request_id, room_id, user_ids, scope,
        ),
        AppAction::InviteBatchCompleted {
            request_id,
            room_id,
            results,
        } => invite_workflow::handle_invite_batch_completed(state, request_id, room_id, results),
        AppAction::InviteBatchFailed {
            request_id,
            room_id,
            kind,
        } => invite_workflow::handle_invite_batch_failed(state, request_id, room_id, kind),
        AppAction::NavigationLoaded { navigation } => {
            navigation::handle_navigation_loaded(state, navigation)
        }
        AppAction::NavigationPreferenceUpdated { update } => {
            navigation::handle_navigation_preference_updated(state, update)
        }
        AppAction::EventNavigationStarted { source } => {
            navigation::handle_event_navigation_started(state, source)
        }
        AppAction::EventNavigationAnchored { generation } => {
            navigation::handle_event_navigation_anchored(state, generation)
        }
        AppAction::EventNavigationLiveFallback { generation } => {
            navigation::handle_event_navigation_live_fallback(state, generation)
        }
        AppAction::EventNavigationFailed { generation, kind } => {
            navigation::handle_event_navigation_failed(state, generation, kind)
        }
        AppAction::EventNavigationCleared => navigation::handle_event_navigation_cleared(state),
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
        AppAction::SpaceOrderPreferenceRemoved { space_id } => {
            navigation::handle_space_order_preference_removed(state, space_id)
        }
        AppAction::SelectRoom { room_id } => navigation::handle_select_room(state, room_id),
        AppAction::TimelineSubscribed { room_id } => {
            timeline::handle_timeline_subscribed(state, room_id)
        }
        AppAction::TimelineSubscriptionFailed {
            room_id,
            message: _,
        } => timeline::handle_timeline_subscription_failed(state, room_id),
        AppAction::TimelineContinuityInspectionStarted {
            room_id,
            generation,
        } => timeline::handle_timeline_continuity_inspection_started(state, room_id, generation),
        AppAction::TimelineContinuityInspected {
            room_id,
            generation,
            inspection,
        } => timeline::handle_timeline_continuity_inspected(state, room_id, generation, inspection),
        AppAction::TimelineGapRepairStarted {
            room_id,
            generation,
            gap_count,
        } => timeline::handle_timeline_gap_repair_started(state, room_id, generation, gap_count),
        AppAction::TimelineGapRepairProgressed {
            room_id,
            generation,
            gap_count,
            batches_processed,
            minimum_batch_id,
        } => timeline::handle_timeline_gap_repair_progressed(
            state,
            room_id,
            generation,
            gap_count,
            batches_processed,
            minimum_batch_id,
        ),
        AppAction::TimelineGapRepairFailed {
            room_id,
            generation,
            gap_count,
            batches_processed,
            kind,
        } => timeline::handle_timeline_gap_repair_failed(
            state,
            room_id,
            generation,
            gap_count,
            batches_processed,
            kind,
        ),
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
        AppAction::ScheduledSendCreatedAtRevision {
            item,
            draft_revision,
        } => timeline::handle_scheduled_send_created_at_revision(state, item, draft_revision),
        AppAction::ScheduledSendDispatchStarted { scheduled_id } => {
            timeline::handle_scheduled_send_dispatch_started(state, scheduled_id)
        }
        AppAction::ScheduledSendDispatchFailed {
            scheduled_id,
            retry_at_ms,
        } => timeline::handle_scheduled_send_dispatch_failed(state, scheduled_id, retry_at_ms),
        AppAction::ScheduledSendRescheduled {
            scheduled_id,
            body,
            send_at_ms,
            handle,
        } => timeline::handle_scheduled_send_rescheduled(
            state,
            scheduled_id,
            body,
            send_at_ms,
            handle,
        ),
        AppAction::ScheduledSendCancelled { scheduled_id }
        | AppAction::ScheduledSendDispatched { scheduled_id } => {
            timeline::handle_scheduled_send_cancelled_or_dispatched(state, scheduled_id)
        }
        AppAction::UploadStagingChanged { target, items } => {
            timeline::handle_upload_staging_changed(state, target, items)
        }
        AppAction::UploadStagingCaptionChanged {
            target,
            staged_id,
            caption,
        } => timeline::handle_upload_staging_caption_changed(state, target, staged_id, caption),
        AppAction::UploadStagingCompressionChanged {
            target,
            staged_id,
            compression_choice,
        } => timeline::handle_upload_staging_compression_changed(
            state,
            target,
            staged_id,
            compression_choice,
        ),
        AppAction::UploadStagingOutputSelected {
            target,
            staged_id,
            selection,
        } => timeline::handle_upload_staging_output_selected(state, target, staged_id, selection),
        AppAction::UploadStagingCleared { target } => {
            timeline::handle_upload_staging_cleared(state, target)
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
        AppAction::ComposerDraftChanged { room_id, document } => {
            let Ok(revision) = crate::ComposerDraftRevision::checked_successor(
                state.composer_drafts.room_revision(&room_id),
                crate::ComposerDraftRevision::ZERO,
            ) else {
                return Vec::new();
            };
            timeline::handle_composer_draft_changed(state, room_id, document, revision)
        }
        AppAction::ComposerDraftChangedAtRevision {
            room_id,
            document,
            revision,
        } => timeline::handle_composer_draft_changed(state, room_id, document, revision),
        AppAction::SendTextSubmitted {
            room_id,
            transaction_id,
            body,
        } => {
            let draft_revision = state.composer_drafts.room_revision(&room_id);
            timeline::handle_send_text_submitted(
                state,
                room_id,
                transaction_id,
                body,
                draft_revision,
            )
        }
        AppAction::SendTextSubmittedAtRevision {
            room_id,
            transaction_id,
            body,
            draft_revision,
        } => timeline::handle_send_text_submitted(
            state,
            room_id,
            transaction_id,
            body,
            draft_revision,
        ),
        AppAction::SendTextFinished {
            room_id,
            transaction_id,
        } => timeline::handle_send_text_finished(state, room_id, transaction_id),
        AppAction::SendTextFailed {
            room_id,
            transaction_id,
            message,
        } => timeline::handle_send_text_failed(state, room_id, transaction_id, message),
        AppAction::ComposerSubmissionAccepted {
            submission_id,
            room_id,
            transaction_id,
            body,
        } => {
            let draft_revision = state.composer_drafts.room_revision(&room_id);
            timeline::handle_composer_submission_accepted(
                state,
                submission_id,
                room_id,
                transaction_id,
                body,
                draft_revision,
            )
        }
        AppAction::ComposerSubmissionAcceptedAtRevision {
            submission_id,
            room_id,
            transaction_id,
            body,
            draft_revision,
        } => timeline::handle_composer_submission_accepted(
            state,
            submission_id,
            room_id,
            transaction_id,
            body,
            draft_revision,
        ),
        AppAction::ComposerSubmissionFinished {
            submission_id,
            room_id,
            transaction_id,
        } => timeline::handle_composer_submission_finished(
            state,
            submission_id,
            room_id,
            transaction_id,
        ),
        AppAction::ComposerSubmissionSettled {
            submission_id,
            transaction_id,
            target,
            outcome,
        } => submission::handle_settled(state, submission_id, transaction_id, target, outcome),
        AppAction::ComposerReplyTargetSelected { room_id, event_id } => {
            timeline::handle_composer_reply_target_selected(state, room_id, event_id)
        }
        AppAction::ComposerReplyCancelled => timeline::handle_composer_reply_cancelled(state),
        AppAction::ThreadComposerDraftChanged {
            room_id,
            root_event_id,
            document,
        } => {
            let Ok(revision) = crate::ComposerDraftRevision::checked_successor(
                state
                    .composer_drafts
                    .thread_revision(&room_id, &root_event_id),
                crate::ComposerDraftRevision::ZERO,
            ) else {
                return Vec::new();
            };
            thread::handle_thread_composer_draft_changed(
                state,
                room_id,
                root_event_id,
                document,
                revision,
            )
        }
        AppAction::ThreadComposerDraftChangedAtRevision {
            room_id,
            root_event_id,
            document,
            revision,
        } => thread::handle_thread_composer_draft_changed(
            state,
            room_id,
            root_event_id,
            document,
            revision,
        ),
        AppAction::ThreadReplySubmitted {
            room_id,
            root_event_id,
            transaction_id,
            body: _,
        } => {
            let draft_revision = state
                .composer_drafts
                .thread_revision(&room_id, &root_event_id);
            thread::handle_thread_reply_submitted(
                state,
                room_id,
                root_event_id,
                transaction_id,
                draft_revision,
            )
        }
        AppAction::ThreadReplySubmittedAtRevision {
            room_id,
            root_event_id,
            transaction_id,
            body: _,
            draft_revision,
        } => thread::handle_thread_reply_submitted(
            state,
            room_id,
            root_event_id,
            transaction_id,
            draft_revision,
        ),
        AppAction::ThreadSubmissionAccepted {
            submission_id,
            room_id,
            root_event_id,
            transaction_id,
            body: _,
        } => {
            let draft_revision = state
                .composer_drafts
                .thread_revision(&room_id, &root_event_id);
            thread::handle_thread_submission_accepted(
                state,
                submission_id,
                room_id,
                root_event_id,
                transaction_id,
                draft_revision,
            )
        }
        AppAction::ThreadSubmissionAcceptedAtRevision {
            submission_id,
            room_id,
            root_event_id,
            transaction_id,
            body: _,
            draft_revision,
        } => thread::handle_thread_submission_accepted(
            state,
            submission_id,
            room_id,
            root_event_id,
            transaction_id,
            draft_revision,
        ),
        AppAction::ComposerDraftAccepted {
            target,
            submitted_revision,
        } => timeline::handle_composer_draft_accepted(state, target, submitted_revision),
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
            intent,
        } => thread::handle_open_thread(state, room_id, root_event_id, intent),
        AppAction::ThreadSubscribed {
            room_id,
            root_event_id,
        } => thread::handle_thread_subscribed(state, room_id, root_event_id),
        AppAction::ThreadSubscriptionFailed {
            room_id,
            root_event_id,
            message,
        } => thread::handle_thread_subscription_failed(state, room_id, root_event_id, message),
        AppAction::ThreadActivityObserved {
            room_id,
            root_event_id,
        } => thread::handle_thread_activity_observed(state, room_id, root_event_id),
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
        AppAction::FocusedContextSubscriptionFailed {
            room_id,
            event_id,
            message,
        } => thread::handle_focused_context_subscription_failed(state, room_id, event_id, message),
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
            query,
            scope,
            results,
        } => search::handle_search_succeeded(state, request_id, query, scope, results),
        AppAction::SearchFailed {
            request_id,
            query,
            scope,
            message,
        } => search::handle_search_failed(state, request_id, query, scope, message),
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
        AppAction::HistoryCrawlStopped { room_id } => {
            search::handle_history_crawl_stopped(state, room_id)
        }
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
        AppAction::ThreadRootProjectionObserved {
            room_id,
            root_event_id,
            activity_event_id,
            activity_timestamp_ms,
        } => thread::handle_thread_root_projection_observed(
            state,
            room_id,
            root_event_id,
            activity_event_id,
            activity_timestamp_ms,
        ),
        AppAction::ThreadRootProjectionReady {
            room_id,
            root_event_id,
            activity_event_id,
            activity_timestamp_ms,
        } => thread::handle_thread_root_projection_ready(
            state,
            room_id,
            root_event_id,
            activity_event_id,
            activity_timestamp_ms,
        ),
        AppAction::ThreadRootProjectionFailed {
            room_id,
            root_event_id,
            activity_event_id,
            activity_timestamp_ms,
            failure_kind,
        } => thread::handle_thread_root_projection_failed(
            state,
            room_id,
            root_event_id,
            activity_event_id,
            activity_timestamp_ms,
            failure_kind,
        ),
        AppAction::ThreadRootProjectionCleared {
            room_id,
            root_event_id,
        } => thread::handle_thread_root_projection_cleared(state, room_id, root_event_id),
        AppAction::ThreadRootProjectionsCleared { room_id } => {
            thread::handle_thread_root_projections_cleared(state, room_id)
        }
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
        AppAction::LiveRoomProfilesObserved { room_id, profiles } => {
            live_signals::handle_live_room_profiles_observed(state, room_id, profiles)
        }
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id,
            scoped_event_ids,
            receipts_by_event,
        } => live_signals::handle_live_room_receipts_window_reconciled(
            state,
            room_id,
            scoped_event_ids,
            receipts_by_event,
        ),
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
    matches!(state.session, SessionState::Ready(_))
}

pub(crate) fn has_session_projection_context(state: &AppState) -> bool {
    matches!(
        state.session,
        SessionState::Ready(_) | SessionState::Locked(_) | SessionState::SwitchingAccount { .. }
    )
}

pub(crate) fn has_verification_gate_projection_context(state: &AppState) -> bool {
    matches!(
        state.session,
        SessionState::Provisional { .. }
            | SessionState::AwaitingVerification { .. }
            | SessionState::Verifying { .. }
            | SessionState::AwaitingBootstrapConfirmation { .. }
            | SessionState::Rejecting { .. }
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
        | SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Locked(info)
        | SessionState::CapabilityBlocked { info, .. }
        | SessionState::SwitchingAccount { info } => Some(info.user_id.as_str()),
        _ => None,
    }
}

pub(crate) fn current_session_info(state: &AppState) -> Option<crate::state::SessionInfo> {
    match &state.session {
        SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. }
        | SessionState::AwaitingBootstrapConfirmation { info, .. }
        | SessionState::Rejecting { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info) => Some(info.clone()),
        SessionState::CapabilityBlocked { info, .. } => Some(info.clone()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    }
}

pub(crate) fn clear_session_views(state: &mut AppState) -> Vec<AppEffect> {
    let previous_room_id = state.timeline.room_id.clone();
    let had_invite_workflow = state.invite_workflow != InviteWorkflowState::default();
    let had_focused_context = state.focused_context != FocusedContextState::Closed;
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;
    let had_search = state.search != SearchState::Closed;
    let had_e2ee_trust = state.e2ee_trust != E2eeTrustState::default();
    let had_e2ee_key_management =
        state.e2ee_trust.key_management != E2eeKeyManagementState::default();
    let had_account_management = state.account_management != AccountManagementState::Idle;
    let had_account_management_capabilities =
        state.account_management_capabilities != AccountManagementCapabilities::default();
    let had_soft_logout_reauth = state.soft_logout_reauth != SoftLogoutReauthState::Idle;
    let had_qr_login = state.qr_login != QrLoginState::Idle;
    let had_live_signals = state.live_signals != Default::default();
    let had_profile = state.profile != Default::default();
    let had_room_interactions = !state.room_interactions.is_empty();
    let had_directory = state.directory != DirectoryState::default();
    let had_activity = !matches!(state.activity, ActivityState::Closed { .. });
    let had_room_management = state.room_management != Default::default();
    let had_mention_candidates = state.mention_candidates != Default::default();
    let had_local_encryption = state.local_encryption != LocalEncryptionState::Unknown;
    let had_native_attention = state.native_attention != Default::default();
    let had_files_view = state.files_view != FilesViewState::Closed;
    let had_threads_list = state.threads_list != ThreadsListState::Closed;
    let had_link_preview_settings = !state.link_preview_settings.room_overrides.is_empty();
    let had_room_preferences = !state.room_preferences.rooms.is_empty();
    let had_room_notification_settings = !state.room_notification_settings.is_empty();
    let had_search_crawler = state.search_crawler != Default::default();
    let had_space_members = state.space_members != Default::default();

    state.navigation = NavigationState::default();
    state.link_preview_settings = Default::default();
    state.room_preferences = Default::default();
    state.spaces.clear();
    state.rooms.clear();
    state.invites.clear();
    state.room_list = Default::default();
    state.room_interactions.clear();
    state.composer_drafts = Default::default();
    state.scheduled_sends = Default::default();
    state.upload_staging = Default::default();
    state.media_gallery = Default::default();
    state.directory = DirectoryState::default();
    state.activity = ActivityState::default();
    state.room_management = Default::default();
    state.mention_candidates = Default::default();
    state.profile = Default::default();
    state.timeline = Default::default();
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.focused_context = FocusedContextState::Closed;
    state.search = SearchState::Closed;
    state.search_crawler = Default::default();
    state.files_view = FilesViewState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.e2ee_trust = E2eeTrustState::default();
    state.account_management_url = None;
    state.account_management = AccountManagementState::Idle;
    state.account_management_capabilities = AccountManagementCapabilities::default();
    state.soft_logout_reauth = SoftLogoutReauthState::Idle;
    state.qr_login = QrLoginState::Idle;
    state.live_signals = Default::default();
    state.local_encryption = LocalEncryptionState::Unknown;
    state.native_attention = Default::default();
    state.invite_workflow = Default::default();
    state.space_members = Default::default();
    state.basic_operation = Default::default();
    state.room_notification_settings.clear();

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if had_invite_workflow {
        effects.push(AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged));
    }
    if let Some(room_id) = previous_room_id {
        effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
    }
    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_focused_context {
        effects.push(AppEffect::EmitUiEvent(UiEvent::FocusedContextChanged));
    }
    if had_search {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchChanged));
    }
    if had_search_crawler {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged));
    }
    if had_e2ee_trust {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged));
    }
    if had_e2ee_key_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged));
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
    if had_mention_candidates {
        effects.push(AppEffect::EmitUiEvent(UiEvent::MentionCandidatesChanged));
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
    if had_space_members {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
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
    live_signals_changed: bool,
    space_members_changed: bool,
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
    if live_signals_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged));
    }
    if space_members_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
    }
    effects
}

pub(crate) fn room_exists(state: &AppState, room_id: &str) -> bool {
    state.rooms.iter().any(|room| room.room_id == room_id)
}

pub(crate) fn retain_navigation_room_memory(state: &mut AppState, authoritative: bool) {
    // #445: a provisional or incomplete Sliding Sync projection is not evidence
    // that a remembered conversation is gone. Pruning on one erased a valid
    // selection during the window before the authoritative projection landed,
    // so only an authoritative projection may invalidate memory here.
    if !authoritative {
        return;
    }

    let known_space_ids = state
        .spaces
        .iter()
        .map(|space| space.space_id.clone())
        .collect::<BTreeSet<_>>();

    let retained_legacy = state
        .navigation
        .last_room_by_space_id
        .iter()
        .filter(|(space_id, room_id)| room_belongs_to_space(state, room_id, space_id))
        .map(|(space_id, room_id)| (space_id.clone(), room_id.clone()))
        .collect::<BTreeMap<_, _>>();

    let retained_selections = state
        .navigation
        .last_selection_by_space_id
        .iter()
        .filter(|(space_id, _)| known_space_ids.contains(space_id.as_str()))
        .map(|(space_id, selection)| {
            // A Space the user still has keeps its surface memory even when the
            // remembered conversation itself became inaccessible.
            let room_id = selection
                .room_id
                .as_deref()
                .filter(|room_id| room_belongs_to_space(state, room_id, space_id))
                .map(str::to_owned);
            (
                space_id.clone(),
                SpaceNavigationSelection {
                    surface: selection.surface,
                    room_id,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    state.navigation.last_room_by_space_id = retained_legacy;
    state.navigation.last_selection_by_space_id = retained_selections;
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
        submission_registry: state.timeline.submission_registry.clone(),
        scheduled_send_capability: state.scheduled_sends.capability.clone(),
        scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
        staged_uploads: state.upload_staging.items_for_room(&room_id),
        media_gallery: state.media_gallery.items_for_room(&room_id),
        media_downloads: Default::default(),
        continuity: Default::default(),
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
        submission_registry: state.timeline.submission_registry.clone(),
        scheduled_send_capability: state.scheduled_sends.capability.clone(),
        scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
        staged_uploads: state.upload_staging.items_for_room(&room_id),
        media_gallery: state.media_gallery.items_for_room(&room_id),
        media_downloads: Default::default(),
        continuity: Default::default(),
    };
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.focused_context = FocusedContextState::Closed;
    // #161: switching rooms resets the main pane to the live timeline.
    state.navigation.main_timeline_anchor = None;
    state.navigation.event_navigation = crate::state::EventNavigationState::Idle;
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
    state.navigation.event_navigation = crate::state::EventNavigationState::Idle;
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

pub(crate) fn normalize_space_order_preference(space_order: &mut Vec<String>) {
    let mut seen_space_ids = BTreeSet::new();
    space_order.retain(|space_id| seen_space_ids.insert(space_id.clone()));
}

pub(crate) fn merge_new_spaces_into_preference(
    space_order: &mut Vec<String>,
    spaces: &[crate::state::SpaceSummary],
) -> bool {
    normalize_space_order_preference(space_order);
    let mut changed = false;
    let mut known_space_ids = space_order.iter().cloned().collect::<BTreeSet<_>>();
    for space in spaces {
        if known_space_ids.insert(space.space_id.clone()) {
            space_order.push(space.space_id.clone());
            changed = true;
        }
    }
    changed
}

pub(crate) fn apply_space_order_preference(
    spaces: &mut [crate::state::SpaceSummary],
    space_order: &[String],
) {
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

pub(crate) fn reorder_visible_space_order(
    space_order: &mut Vec<String>,
    current_spaces: &[crate::state::SpaceSummary],
    requested_space_ids: &[String],
) -> bool {
    if !is_complete_space_order(current_spaces, requested_space_ids) {
        return false;
    }

    let mut next_space_order = space_order.clone();
    merge_new_spaces_into_preference(&mut next_space_order, current_spaces);
    let visible_space_ids = current_spaces
        .iter()
        .map(|space| space.space_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut requested_space_ids = requested_space_ids.iter();
    for space_id in &mut next_space_order {
        if visible_space_ids.contains(space_id.as_str()) {
            *space_id = requested_space_ids
                .next()
                .expect("validated visible Space reorder length")
                .clone();
        }
    }

    *space_order = next_space_order;
    true
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
    if !room_belongs_to_space(state, &room_id, &space_id) {
        return;
    }
    let is_dm = state
        .rooms
        .iter()
        .any(|room| room.room_id == room_id && room.is_dm);
    let surface = if is_dm {
        SpaceConversationSurface::Dms
    } else {
        SpaceConversationSurface::Rooms
    };
    state.navigation.last_selection_by_space_id.insert(
        space_id.clone(),
        SpaceNavigationSelection {
            surface,
            room_id: Some(room_id.clone()),
        },
    );
    if !is_dm {
        // Keep the legacy map non-DM-only so an older build reading the same
        // persisted `navigation.v1` payload behaves exactly as it did before.
        state
            .navigation
            .last_room_by_space_id
            .insert(space_id, room_id);
    }
}

/// The remembered selection for a Space, validated against what that Space can
/// currently show, with a deterministic fallback (#445).
pub(crate) fn preferred_selection_in_space(
    state: &AppState,
    space_id: &str,
) -> SpaceNavigationSelection {
    if let Some(selection) = state.navigation.last_selection_by_space_id.get(space_id) {
        if let Some(room_id) = selection.room_id.as_deref()
            && room_belongs_to_space(state, room_id, space_id)
        {
            return selection.clone();
        }
        if selection.surface == SpaceConversationSurface::Dms {
            // The remembered DM is no longer visible here, but the surface is
            // still valid memory: fall back inside that surface rather than
            // silently switching the user back to Rooms.
            return SpaceNavigationSelection {
                surface: SpaceConversationSurface::Dms,
                room_id: first_dm_room_id_in_space(state, space_id),
            };
        }
    }
    if let Some(room_id) = state
        .navigation
        .last_room_by_space_id
        .get(space_id)
        .filter(|room_id| room_belongs_to_space(state, room_id, space_id))
    {
        // Migration path: a payload persisted before `last_selection_by_space_id`
        // existed only ever recorded non-DM rooms.
        return SpaceNavigationSelection {
            surface: SpaceConversationSurface::Rooms,
            room_id: Some(room_id.clone()),
        };
    }
    SpaceNavigationSelection {
        surface: SpaceConversationSurface::Rooms,
        room_id: first_room_id_in_space(state, space_id),
    }
}

fn preferred_room_id_in_space(state: &AppState, space_id: &str) -> Option<String> {
    preferred_selection_in_space(state, space_id).room_id
}

fn first_dm_room_id_in_space(state: &AppState, space_id: &str) -> Option<String> {
    state
        .rooms
        .iter()
        .find(|room| {
            room.is_dm
                && room
                    .dm_space_ids
                    .iter()
                    .any(|candidate| candidate == space_id)
        })
        .map(|room| room.room_id.clone())
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
        // #445: a DM is not a Matrix child room of a Space, but every Space has
        // a DMs surface showing a Space-filtered DM list, so for navigation
        // memory a DM belongs to the Spaces whose DM projection shows it.
        // Returning `false` unconditionally here is why a DM selection could
        // never be remembered or restored.
        return room
            .dm_space_ids
            .iter()
            .any(|candidate| candidate == space_id);
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
mod tests;
