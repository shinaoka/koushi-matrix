use crate::{
    action::AppAction,
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, BasicOperationRequest, BasicOperationState, ComposerMode,
        CrossSigningStatus, E2eeRecoveryState, E2eeTrustState, FocusedContextState,
        IdentityResetState, KeyBackupStatus, NavigationState, PendingComposerSendKind, SasEmoji,
        SearchState, SessionState, SettingsPersistenceState, SyncState, ThreadPaneState,
        TimelinePaneState, VerificationFlowState, VerificationTarget,
    },
};

const TIMELINE_SUBSCRIPTION_FAILED_MESSAGE: &str = "Matrix timeline subscription failed";
const SETTINGS_LOAD_FAILED_MESSAGE: &str = "Settings could not be loaded";
const SETTINGS_PERSIST_FAILED_MESSAGE: &str = "Settings could not be saved";

pub fn reduce(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    match action {
        AppAction::AppStarted => {
            state.session = SessionState::Restoring;
            vec![AppEffect::RestoreSession]
        }
        AppAction::RestoreSessionSucceeded(info) | AppAction::LoginSucceeded(info) => {
            state.session = SessionState::Ready(info.clone());
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoveryRequired { info, methods } => {
            state.session = SessionState::NeedsRecovery {
                info: info.clone(),
                methods,
            };
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoverySubmitted(request) => {
            let SessionState::NeedsRecovery { info, methods } = &state.session else {
                return Vec::new();
            };
            state.session = SessionState::Recovering {
                info: info.clone(),
                methods: methods.clone(),
            };
            vec![
                AppEffect::RecoverE2ee(request),
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoverySucceeded => {
            let SessionState::Recovering { info, .. } = &state.session else {
                return Vec::new();
            };
            let info = info.clone();
            state.session = SessionState::Ready(info.clone());
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoveryFailed { message } => {
            let SessionState::Recovering { info, methods } = &state.session else {
                return Vec::new();
            };
            state.session = SessionState::NeedsRecovery {
                info: info.clone(),
                methods: methods.clone(),
            };
            state.errors.push(AppError {
                code: "e2ee_recovery_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::E2eeRecoveryStateChanged {
            state: recovery_state,
            methods,
        } => match recovery_state {
            E2eeRecoveryState::Unknown => Vec::new(),
            E2eeRecoveryState::Incomplete => {
                let SessionState::Ready(info) = &state.session else {
                    return Vec::new();
                };
                let info = info.clone();
                state.session = SessionState::NeedsRecovery { info, methods };
                vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
            }
            E2eeRecoveryState::Enabled | E2eeRecoveryState::Disabled => {
                let info = match &state.session {
                    SessionState::NeedsRecovery { info, .. }
                    | SessionState::Recovering { info, .. } => info.clone(),
                    _ => return Vec::new(),
                };
                state.session = SessionState::Ready(info.clone());
                state.sync = SyncState::Starting;
                vec![
                    AppEffect::PersistSession(info),
                    AppEffect::StartSync,
                    AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                ]
            }
        },
        AppAction::VerificationRequested { request_id, target } => {
            if !is_session_ready(state)
                || !matches!(
                    state.e2ee_trust.verification,
                    VerificationFlowState::Idle
                        | VerificationFlowState::Done { .. }
                        | VerificationFlowState::Failed { .. }
                )
            {
                return Vec::new();
            }

            state.e2ee_trust.verification = VerificationFlowState::Requested {
                request_id,
                target: target.clone(),
            };
            vec![
                AppEffect::RequestVerification { request_id, target },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::VerificationAccepted { request_id } => {
            let VerificationFlowState::Requested { target, .. } = &state.e2ee_trust.verification
            else {
                return Vec::new();
            };
            if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
                return Vec::new();
            }

            state.e2ee_trust.verification = VerificationFlowState::Accepted {
                request_id,
                target: target.clone(),
            };
            vec![
                AppEffect::AcceptVerification { request_id },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::VerificationSasPresented { request_id, emojis } => {
            if !matches!(
                state.e2ee_trust.verification,
                VerificationFlowState::Requested { .. }
                    | VerificationFlowState::Accepted { .. }
                    | VerificationFlowState::SasPresented { .. }
            ) {
                return Vec::new();
            }
            let Some(target) = verification_target(&state.e2ee_trust.verification) else {
                return Vec::new();
            };
            if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
                return Vec::new();
            }

            state.e2ee_trust.verification = VerificationFlowState::SasPresented {
                request_id,
                target,
                emojis,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::VerificationConfirmed { request_id } => {
            let VerificationFlowState::SasPresented { .. } = &state.e2ee_trust.verification else {
                return Vec::new();
            };
            let Some(target) = verification_target(&state.e2ee_trust.verification) else {
                return Vec::new();
            };
            if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
                return Vec::new();
            }
            let emojis = verification_emojis(&state.e2ee_trust.verification);

            state.e2ee_trust.verification = VerificationFlowState::Confirming {
                request_id,
                target,
                emojis,
            };
            vec![
                AppEffect::ConfirmSasVerification { request_id },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::VerificationCancelled { request_id } => {
            if !verification_is_active(&state.e2ee_trust.verification)
                || verification_request_id(&state.e2ee_trust.verification) != Some(request_id)
            {
                return Vec::new();
            }

            state.e2ee_trust.verification = VerificationFlowState::Idle;
            vec![
                AppEffect::CancelVerification { request_id },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::VerificationCompleted { request_id } => {
            if !verification_is_active(&state.e2ee_trust.verification)
                || verification_request_id(&state.e2ee_trust.verification) != Some(request_id)
            {
                return Vec::new();
            }
            let Some(target) = verification_target(&state.e2ee_trust.verification) else {
                return Vec::new();
            };

            state.e2ee_trust.verification = VerificationFlowState::Done { request_id, target };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::VerificationFailed { request_id, kind } => {
            if !verification_is_active(&state.e2ee_trust.verification)
                || verification_request_id(&state.e2ee_trust.verification) != Some(request_id)
            {
                return Vec::new();
            }
            let Some(target) = verification_target(&state.e2ee_trust.verification) else {
                return Vec::new();
            };

            state.e2ee_trust.verification = VerificationFlowState::Failed {
                request_id,
                target,
                kind,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::CrossSigningStatusChanged { status } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.e2ee_trust.cross_signing = status;
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::BootstrapCrossSigningRequested { request_id } => {
            if !is_session_ready(state)
                || matches!(
                    state.e2ee_trust.cross_signing,
                    CrossSigningStatus::Bootstrapping { .. }
                )
            {
                return Vec::new();
            }

            state.e2ee_trust.cross_signing = CrossSigningStatus::Bootstrapping { request_id };
            vec![
                AppEffect::BootstrapCrossSigning { request_id },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::BootstrapCrossSigningFailed { request_id, kind } => {
            if state.e2ee_trust.cross_signing != (CrossSigningStatus::Bootstrapping { request_id })
            {
                return Vec::new();
            }

            state.e2ee_trust.cross_signing = CrossSigningStatus::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::EnableKeyBackupRequested { request_id } => {
            if !is_session_ready(state)
                || matches!(
                    state.e2ee_trust.key_backup,
                    KeyBackupStatus::Enabling { .. } | KeyBackupStatus::Restoring { .. }
                )
            {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Enabling { request_id };
            vec![
                AppEffect::EnableKeyBackup { request_id },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::KeyBackupEnabled {
            request_id,
            version,
        } => {
            if state.e2ee_trust.key_backup != (KeyBackupStatus::Enabling { request_id }) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Enabled { version };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::KeyBackupFailed { request_id, kind } => {
            if !key_backup_request_matches(&state.e2ee_trust.key_backup, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::RestoreKeyBackupRequested {
            request_id,
            version,
        } => {
            if !is_session_ready(state)
                || matches!(
                    state.e2ee_trust.key_backup,
                    KeyBackupStatus::Enabling { .. } | KeyBackupStatus::Restoring { .. }
                )
            {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Restoring {
                request_id,
                version: version.clone(),
                restored_rooms: 0,
                total_rooms: None,
            };
            vec![
                AppEffect::RestoreKeyBackup {
                    request_id,
                    version,
                },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::KeyBackupRestoreProgress {
            request_id,
            restored_rooms,
            total_rooms,
        } => {
            let KeyBackupStatus::Restoring { version, .. } = &state.e2ee_trust.key_backup else {
                return Vec::new();
            };
            if !key_backup_request_matches(&state.e2ee_trust.key_backup, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Restoring {
                request_id,
                version: version.clone(),
                restored_rooms,
                total_rooms,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::KeyBackupRestored {
            request_id,
            version,
        } => {
            if !key_backup_restore_request_matches(&state.e2ee_trust.key_backup, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = match version {
                Some(version) => KeyBackupStatus::Enabled { version },
                None => KeyBackupStatus::Unknown,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityRequested { request_id } => {
            if !is_session_ready(state)
                || matches!(
                    state.e2ee_trust.identity_reset,
                    IdentityResetState::Resetting { .. } | IdentityResetState::AwaitingAuth { .. }
                )
            {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::Resetting { request_id };
            vec![
                AppEffect::ResetIdentity { request_id },
                AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            ]
        }
        AppAction::ResetIdentityAuthRequired {
            request_id,
            auth_type,
        } => {
            if state.e2ee_trust.identity_reset != (IdentityResetState::Resetting { request_id }) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::AwaitingAuth {
                request_id,
                auth_type,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityAuthSubmitted { request_id } => {
            if !matches!(
                state.e2ee_trust.identity_reset,
                IdentityResetState::AwaitingAuth {
                    request_id: current_request_id,
                    ..
                } if current_request_id == request_id
            ) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::Resetting { request_id };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityCompleted { request_id } => {
            if !identity_reset_request_matches(&state.e2ee_trust.identity_reset, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::Idle;
            state.e2ee_trust.verification = VerificationFlowState::Idle;
            state.e2ee_trust.cross_signing = CrossSigningStatus::Missing;
            state.e2ee_trust.key_backup = KeyBackupStatus::Disabled;
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityFailed { request_id, kind } => {
            if !identity_reset_request_matches(&state.e2ee_trust.identity_reset, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::Failed { request_id, kind };
            state.e2ee_trust.cross_signing = CrossSigningStatus::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::RestoreSessionNotFound => {
            state.session = SessionState::SignedOut;
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::RestoreSessionFailed { message } => {
            state.session = SessionState::SignedOut;
            state.errors.push(AppError {
                code: "restore_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::SwitchAccountRequested { info } => {
            if current_session_info(state).as_ref() == Some(&info) {
                return Vec::new();
            }

            state.session = SessionState::SwitchingAccount { info: info.clone() };
            state.sync = SyncState::Stopped;
            let mut effects = vec![
                AppEffect::StopSync,
                AppEffect::ClearSession,
                AppEffect::RestoreSessionFor(info),
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ];
            effects.extend(clear_session_views(state));
            effects
        }
        AppAction::LoginDiscoveryRequested { homeserver } => {
            state.auth = crate::state::AuthDiscoveryState::Discovering {
                homeserver: homeserver.clone(),
            };
            vec![
                AppEffect::DiscoverLogin { homeserver },
                AppEffect::EmitUiEvent(UiEvent::AuthChanged),
            ]
        }
        AppAction::LoginDiscoverySucceeded { homeserver, flows } => {
            state.auth = crate::state::AuthDiscoveryState::Ready { homeserver, flows };
            vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]
        }
        AppAction::LoginDiscoveryFailed {
            homeserver,
            message,
        } => {
            state.auth = crate::state::AuthDiscoveryState::Failed {
                homeserver,
                message,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]
        }
        AppAction::SettingsLoaded { values } => {
            state.settings.values = values;
            state.settings.persistence = SettingsPersistenceState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
        }
        AppAction::SettingsLoadFailed { message: _ } => {
            state.settings.persistence = SettingsPersistenceState::Idle;
            state.errors.push(AppError {
                code: "settings_load_failed".to_owned(),
                message: SETTINGS_LOAD_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::SettingsUpdateRequested { request_id, patch } => {
            state.settings.values.apply_patch(patch);
            state.settings.persistence = SettingsPersistenceState::Saving { request_id };
            vec![
                AppEffect::PersistSettings {
                    request_id,
                    values: state.settings.values.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
            ]
        }
        AppAction::SettingsPersisted { request_id } => {
            if state.settings.persistence != (SettingsPersistenceState::Saving { request_id }) {
                return Vec::new();
            }

            state.settings.persistence = SettingsPersistenceState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
        }
        AppAction::SettingsPersistFailed {
            request_id,
            message: _,
        } => {
            if state.settings.persistence != (SettingsPersistenceState::Saving { request_id }) {
                return Vec::new();
            }

            state.settings.persistence = SettingsPersistenceState::Idle;
            state.errors.push(AppError {
                code: "settings_persist_failed".to_owned(),
                message: SETTINGS_PERSIST_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::LoginSubmitted(request) => {
            state.session = SessionState::Authenticating {
                homeserver: request.homeserver.clone(),
            };
            vec![
                AppEffect::Login(request),
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::LoginFailed { message } => {
            state.session = SessionState::SignedOut;
            state.errors.push(AppError {
                code: "login_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::SessionPersistenceFailed { message } => {
            state.errors.push(AppError {
                code: "session_persistence_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::SessionLocked => {
            if let SessionState::Ready(info) = &state.session {
                state.session = SessionState::Locked(info.clone());
                state.sync = SyncState::Stopped;
                let mut effects = vec![
                    AppEffect::StopSync,
                    AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                ];
                effects.extend(clear_session_views(state));
                return effects;
            }
            Vec::new()
        }
        AppAction::LogoutRequested => {
            state.session = SessionState::LoggingOut;
            state.sync = SyncState::Stopped;
            let mut effects = vec![
                AppEffect::StopSync,
                AppEffect::ClearSession,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ];
            effects.extend(clear_session_views(state));
            effects
        }
        AppAction::LogoutFinished => {
            *state = AppState::default();
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::SyncStarted => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match state.sync {
                SyncState::Starting | SyncState::Failed { .. } | SyncState::Reconnecting { .. } => {
                    state.sync = SyncState::Running;
                    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
                }
                SyncState::Running | SyncState::Stopped => Vec::new(),
            }
        }
        AppAction::SyncFailed { reason } => {
            if !is_session_ready(state) || matches!(state.sync, SyncState::Stopped) {
                return Vec::new();
            }

            state.sync = SyncState::Failed { reason };
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
                AppEffect::StartSync,
            ]
        }
        AppAction::SyncReconnecting { reason } => {
            if !is_session_ready(state)
                || matches!(state.sync, SyncState::Stopped | SyncState::Running)
            {
                return Vec::new();
            }

            state.sync = SyncState::Reconnecting { reason };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncRecovered => {
            if !is_session_ready(state)
                || !matches!(
                    state.sync,
                    SyncState::Failed { .. } | SyncState::Reconnecting { .. }
                )
            {
                return Vec::new();
            }

            state.sync = SyncState::Running;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncStopped => {
            if matches!(state.sync, SyncState::Stopped) {
                return Vec::new();
            }

            state.sync = SyncState::Stopped;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomListUpdated { spaces, rooms } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let had_active_room_before_update = state.navigation.active_room_id.is_some();
            state.spaces = spaces;
            state.rooms = rooms;

            let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];

            if state
                .navigation
                .active_space_id
                .as_deref()
                .is_some_and(|active_space_id| {
                    !state
                        .spaces
                        .iter()
                        .any(|space| space.space_id == active_space_id)
                })
            {
                state.navigation.active_space_id = None;
            }

            if let Some(active_room_id) = state.navigation.active_room_id.clone() {
                let room_still_exists = state
                    .rooms
                    .iter()
                    .any(|room| room.room_id == active_room_id);

                if !room_still_exists {
                    state.navigation.active_room_id = None;
                    let previous_room_id = state.timeline.room_id.clone().unwrap_or(active_room_id);
                    let had_thread = state.thread != ThreadPaneState::Closed;

                    state.timeline = Default::default();
                    state.thread = ThreadPaneState::Closed;

                    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                        room_id: previous_room_id,
                    }));
                    if had_thread {
                        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
                    }
                }
            }

            if had_active_room_before_update
                && state.navigation.active_room_id.is_none()
                && state.navigation.active_space_id.is_some()
                && let Some(room_id) = first_room_id_in_active_space(state)
            {
                select_active_room_after_room_list_update(state, &mut effects, room_id);
            }

            if let Some(active_room_id) = state.navigation.active_room_id.clone()
                && active_room_left_selected_space(state, &active_room_id)
            {
                retarget_active_room_for_selected_space(state, &mut effects, active_room_id);
            }

            if !had_active_room_before_update
                && state.navigation.active_room_id.is_none()
                && let Some(room_id) = first_default_room_id(state)
            {
                state.navigation.active_room_id = Some(room_id.clone());
                state.timeline = TimelinePaneState {
                    room_id: Some(room_id.clone()),
                    is_subscribed: false,
                    is_paginating_backwards: false,
                    composer: Default::default(),
                };
                effects.push(AppEffect::SubscribeTimeline {
                    room_id: room_id.clone(),
                });
                effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
            }

            effects
        }
        AppAction::SelectSpace { space_id } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.navigation.active_space_id = space_id
                .filter(|space_id| state.spaces.iter().any(|space| space.space_id == *space_id));
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SelectRoom { room_id } => {
            if !is_session_ready(state) || !state.rooms.iter().any(|room| room.room_id == room_id) {
                return Vec::new();
            }

            let had_thread = state.thread != ThreadPaneState::Closed;
            state.navigation.active_room_id = Some(room_id.clone());
            state.timeline = TimelinePaneState {
                room_id: Some(room_id.clone()),
                is_subscribed: false,
                is_paginating_backwards: false,
                composer: Default::default(),
            };
            state.thread = ThreadPaneState::Closed;
            state.focused_context = FocusedContextState::Closed;
            let mut effects = vec![
                AppEffect::SubscribeTimeline {
                    room_id: room_id.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ];
            if had_thread {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
            }
            effects
        }
        AppAction::TimelineSubscribed { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.is_subscribed = true;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::TimelineSubscriptionFailed {
            room_id,
            message: _,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.errors.push(AppError {
                code: "timeline_subscription_failed".to_owned(),
                message: TIMELINE_SUBSCRIPTION_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::TimelineBackPaginationRequested { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.is_paginating_backwards
            {
                return Vec::new();
            }

            state.timeline.is_paginating_backwards = true;
            vec![
                AppEffect::PaginateTimelineBackwards {
                    room_id: room_id.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ]
        }
        AppAction::TimelineBackPaginationFinished { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || !state.timeline.is_paginating_backwards
            {
                return Vec::new();
            }

            state.timeline.is_paginating_backwards = false;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::ComposerDraftChanged { room_id, draft } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.composer.draft = draft;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::SendTextSubmitted {
            room_id,
            transaction_id,
            body,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.is_some()
            {
                return Vec::new();
            }

            state.timeline.composer.pending_transaction_id = Some(transaction_id.clone());
            state.timeline.composer.pending_send_kind = Some(match &state.timeline.composer.mode {
                ComposerMode::Plain => PendingComposerSendKind::Plain,
                ComposerMode::Reply {
                    in_reply_to_event_id,
                } => PendingComposerSendKind::Reply {
                    in_reply_to_event_id: in_reply_to_event_id.clone(),
                },
            });
            state.timeline.composer.draft.clear();
            vec![
                AppEffect::SendText {
                    room_id: room_id.clone(),
                    transaction_id,
                    body,
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ]
        }
        AppAction::SendTextFinished {
            room_id,
            transaction_id,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }

            let pending_send_kind = state.timeline.composer.pending_send_kind.take();
            state.timeline.composer.pending_transaction_id = None;
            if let Some(PendingComposerSendKind::Reply {
                in_reply_to_event_id,
            }) = pending_send_kind
            {
                if state.timeline.composer.mode
                    == (ComposerMode::Reply {
                        in_reply_to_event_id,
                    })
                {
                    state.timeline.composer.mode = ComposerMode::Plain;
                }
            }
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::SendTextFailed {
            room_id,
            transaction_id,
            message,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.composer.pending_transaction_id = None;
            state.timeline.composer.pending_send_kind = None;
            state.errors.push(AppError {
                code: "send_text_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::ThreadComposerDraftChanged {
            room_id,
            root_event_id,
            draft,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id && open_root_event_id == &root_event_id => {
                    composer.draft = draft;
                    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
                }
                _ => Vec::new(),
            }
        }
        AppAction::ThreadReplySubmitted {
            room_id,
            root_event_id,
            transaction_id,
            body: _body,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id
                    && open_root_event_id == &root_event_id
                    && composer.pending_transaction_id.is_none() =>
                {
                    composer.pending_transaction_id = Some(transaction_id);
                    composer.pending_send_kind = Some(PendingComposerSendKind::Reply {
                        in_reply_to_event_id: root_event_id,
                    });
                    composer.draft.clear();
                    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
                }
                _ => Vec::new(),
            }
        }
        AppAction::ThreadReplyFinished {
            room_id,
            root_event_id,
            transaction_id,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id
                    && open_root_event_id == &root_event_id
                    && composer.pending_transaction_id.as_deref()
                        == Some(transaction_id.as_str()) =>
                {
                    composer.pending_transaction_id = None;
                    composer.pending_send_kind = None;
                    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
                }
                _ => Vec::new(),
            }
        }
        AppAction::ThreadReplyFailed {
            room_id,
            root_event_id,
            transaction_id,
            message,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id
                    && open_root_event_id == &root_event_id
                    && composer.pending_transaction_id.as_deref()
                        == Some(transaction_id.as_str()) =>
                {
                    composer.pending_transaction_id = None;
                    composer.pending_send_kind = None;
                    state.errors.push(AppError {
                        code: "send_text_failed".to_owned(),
                        message,
                        recoverable: true,
                    });
                    vec![
                        AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
                        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
                    ]
                }
                _ => Vec::new(),
            }
        }
        AppAction::OpenThread {
            room_id,
            root_event_id,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Opening {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            };
            vec![
                AppEffect::OpenThreadTimeline {
                    room_id,
                    root_event_id,
                },
                AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            ]
        }
        AppAction::ThreadSubscribed {
            room_id,
            root_event_id,
        } => {
            if !is_session_ready(state)
                || !matches!(
                    &state.thread,
                    ThreadPaneState::Opening {
                        room_id: opening_room_id,
                        root_event_id: opening_root_event_id,
                    } if opening_room_id == &room_id && opening_root_event_id == &root_event_id
                )
            {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Open {
                room_id,
                root_event_id,
                is_subscribed: true,
                composer: Default::default(),
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::CloseThread => {
            if !is_session_ready(state) || state.thread == ThreadPaneState::Closed {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Closed;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::OpenFocusedContext { room_id, event_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.focused_context = FocusedContextState::Opening {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            };
            vec![AppEffect::OpenFocusedTimeline { room_id, event_id }]
        }
        AppAction::FocusedContextSubscribed { room_id, event_id } => {
            if !is_session_ready(state)
                || !matches!(
                    &state.focused_context,
                    FocusedContextState::Opening {
                        room_id: opening_room_id,
                        event_id: opening_event_id,
                    } if opening_room_id == &room_id && opening_event_id == &event_id
                )
            {
                return Vec::new();
            }

            state.focused_context = FocusedContextState::Open {
                room_id,
                event_id,
                is_subscribed: true,
            };
            Vec::new()
        }
        AppAction::CloseFocusedContext => {
            if !is_session_ready(state) || state.focused_context == FocusedContextState::Closed {
                return Vec::new();
            }

            state.focused_context = FocusedContextState::Closed;
            Vec::new()
        }
        AppAction::SearchEdited { query, scope } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.search = SearchState::Editing { query, scope };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::SearchSubmitted {
            request_id,
            query,
            scope,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.search = SearchState::Searching {
                request_id,
                query: query.clone(),
                scope: scope.clone(),
            };
            vec![
                AppEffect::SearchMessages {
                    request_id,
                    query,
                    scope,
                },
                AppEffect::EmitUiEvent(UiEvent::SearchChanged),
            ]
        }
        AppAction::SearchSucceeded {
            request_id,
            results,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, query, scope) = match &state.search {
                SearchState::Searching {
                    request_id,
                    query,
                    scope,
                } => (*request_id, query.clone(), scope.clone()),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.search = SearchState::Results {
                request_id,
                query,
                scope,
                results,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::SearchFailed {
            request_id,
            message,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, query, scope) = match &state.search {
                SearchState::Searching {
                    request_id,
                    query,
                    scope,
                } => (*request_id, query.clone(), scope.clone()),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.search = SearchState::Failed {
                request_id,
                query,
                scope,
                message,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::ClearError { code } => {
            state.errors.retain(|error| error.code != code);
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::BasicOperationRequested {
            request_id,
            request,
        } => {
            // Start transition. Guarded like the composer's pending-transaction
            // rule: a new operation is accepted only from `Idle` and only with a
            // ready session, so an in-flight operation is never clobbered.
            if !is_session_ready(state) || !state.basic_operation.is_idle() {
                return Vec::new();
            }
            state.basic_operation = match request {
                BasicOperationRequest::CreateRoom { name } => {
                    BasicOperationState::CreatingRoom { request_id, name }
                }
                BasicOperationRequest::CreateSpace { name } => {
                    BasicOperationState::CreatingSpace { request_id, name }
                }
                BasicOperationRequest::LinkSpaceChild {
                    space_id,
                    child_room_id,
                } => BasicOperationState::LinkingSpaceChild {
                    request_id,
                    space_id,
                    child_room_id,
                },
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::BasicOperationSucceeded { request_id } => {
            // Settle transition. Like search's `request_id` check, a completion is
            // applied only when it correlates to the in-flight operation; stale,
            // duplicate, or idle-state completions are ignored.
            if state.basic_operation.request_id() != Some(request_id) {
                return Vec::new();
            }
            state.basic_operation = BasicOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::BasicOperationFailed {
            request_id,
            message,
        } => {
            if state.basic_operation.request_id() != Some(request_id) {
                return Vec::new();
            }
            state.basic_operation = BasicOperationState::Idle;
            state.errors.push(AppError {
                code: "basic_operation_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::ComposerReplyTargetSelected { room_id, event_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }
            state.timeline.composer.mode = ComposerMode::Reply {
                in_reply_to_event_id: event_id,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::ComposerReplyCancelled => {
            let Some(room_id) = state.timeline.room_id.clone() else {
                return Vec::new();
            };
            if state.timeline.composer.mode == ComposerMode::Plain {
                return Vec::new();
            }
            state.timeline.composer.mode = ComposerMode::Plain;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
    }
}

fn is_session_ready(state: &AppState) -> bool {
    matches!(
        state.session,
        SessionState::Ready(_)
            | SessionState::NeedsRecovery { .. }
            | SessionState::Recovering { .. }
    )
}

fn verification_request_id(verification: &VerificationFlowState) -> Option<u64> {
    match verification {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { request_id, .. }
        | VerificationFlowState::Accepted { request_id, .. }
        | VerificationFlowState::SasPresented { request_id, .. }
        | VerificationFlowState::Confirming { request_id, .. }
        | VerificationFlowState::Done { request_id, .. }
        | VerificationFlowState::Failed { request_id, .. } => Some(*request_id),
    }
}

fn verification_target(verification: &VerificationFlowState) -> Option<VerificationTarget> {
    match verification {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { target, .. }
        | VerificationFlowState::Accepted { target, .. }
        | VerificationFlowState::SasPresented { target, .. }
        | VerificationFlowState::Confirming { target, .. }
        | VerificationFlowState::Done { target, .. }
        | VerificationFlowState::Failed { target, .. } => Some(target.clone()),
    }
}

fn verification_emojis(verification: &VerificationFlowState) -> Vec<SasEmoji> {
    match verification {
        VerificationFlowState::SasPresented { emojis, .. }
        | VerificationFlowState::Confirming { emojis, .. } => emojis.clone(),
        VerificationFlowState::Idle
        | VerificationFlowState::Requested { .. }
        | VerificationFlowState::Accepted { .. }
        | VerificationFlowState::Done { .. }
        | VerificationFlowState::Failed { .. } => Vec::new(),
    }
}

fn verification_is_active(verification: &VerificationFlowState) -> bool {
    matches!(
        verification,
        VerificationFlowState::Requested { .. }
            | VerificationFlowState::Accepted { .. }
            | VerificationFlowState::SasPresented { .. }
            | VerificationFlowState::Confirming { .. }
    )
}

fn key_backup_request_matches(key_backup: &KeyBackupStatus, request_id: u64) -> bool {
    matches!(
        key_backup,
        KeyBackupStatus::Enabling {
            request_id: current_request_id,
        } | KeyBackupStatus::Restoring {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    )
}

fn key_backup_restore_request_matches(key_backup: &KeyBackupStatus, request_id: u64) -> bool {
    matches!(
        key_backup,
        KeyBackupStatus::Restoring {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    )
}

fn identity_reset_request_matches(identity_reset: &IdentityResetState, request_id: u64) -> bool {
    matches!(
        identity_reset,
        IdentityResetState::Resetting {
            request_id: current_request_id,
        } | IdentityResetState::AwaitingAuth {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    )
}

fn first_default_room_id(state: &AppState) -> Option<String> {
    state
        .rooms
        .iter()
        .find(|room| !room.is_dm)
        .or_else(|| state.rooms.first())
        .map(|room| room.room_id.clone())
}

fn first_room_id_in_active_space(state: &AppState) -> Option<String> {
    let active_space_id = state.navigation.active_space_id.as_deref()?;
    let active_space = state
        .spaces
        .iter()
        .find(|space| space.space_id == active_space_id)?;

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

fn active_room_left_selected_space(state: &AppState, active_room_id: &str) -> bool {
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

fn retarget_active_room_for_selected_space(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    previous_room_id: String,
) {
    let next_room_id = first_room_id_in_active_space(state);
    let had_thread = state.thread != ThreadPaneState::Closed;

    match next_room_id {
        Some(room_id) => {
            select_active_room_after_room_list_update(state, effects, room_id);
        }
        None => {
            state.navigation.active_room_id = None;
            state.thread = ThreadPaneState::Closed;
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

fn select_active_room_after_room_list_update(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    room_id: String,
) {
    let had_thread = state.thread != ThreadPaneState::Closed;

    state.navigation.active_room_id = Some(room_id.clone());
    state.timeline = TimelinePaneState {
        room_id: Some(room_id.clone()),
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: Default::default(),
    };
    state.thread = ThreadPaneState::Closed;
    effects.push(AppEffect::SubscribeTimeline {
        room_id: room_id.clone(),
    });
    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));

    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
}

fn current_session_info(state: &AppState) -> Option<crate::state::SessionInfo> {
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

fn clear_session_views(state: &mut AppState) -> Vec<AppEffect> {
    let previous_room_id = state.timeline.room_id.clone();
    let had_thread = state.thread != ThreadPaneState::Closed;
    let had_search = state.search != SearchState::Closed;
    let had_e2ee_trust = state.e2ee_trust != E2eeTrustState::default();

    state.navigation = NavigationState::default();
    state.spaces.clear();
    state.rooms.clear();
    state.timeline = Default::default();
    state.thread = ThreadPaneState::Closed;
    state.focused_context = FocusedContextState::Closed;
    state.search = SearchState::Closed;
    state.e2ee_trust = E2eeTrustState::default();

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
    effects
}
