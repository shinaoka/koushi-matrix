//! Incremental AppState slice deltas.

use std::collections::{BTreeMap, HashMap};

use koushi_state::{
    AccountManagementCapabilities, AccountManagementState, ActivityState, AppError, AppState,
    AuthDiscoveryState, BasicOperationState, CjkTextPolicyState, DeviceSessionListState,
    DirectoryState, E2eeTrustState, FilesViewState, FocusedContextState, InvitePreview,
    InviteWorkflowState, LinkPreviewSettingsState, LiveSignalsState, LocalEncryptionState,
    NativeAttentionState, NavigationState, ProfileState, QrLoginState, RoomInteractionState,
    RoomListProjection, RoomManagementState, RoomNotificationSettings, RoomPreferencesState,
    RoomSummary, SearchCrawlerState, SearchState, SessionState, SettingsState, SidebarModel,
    SoftLogoutReauthState, SpaceSummary, SyncMode, SyncState, ThreadAttentionState,
    ThreadPaneState, ThreadsListState, TimelinePaneState,
    compose_sidebar_with_room_notification_settings,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDelta {
    pub generation: u64,
    pub changed: StateDeltaChangedSlices,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateDeltaChangedSlices {
    pub session: Option<SessionState>,
    pub auth: Option<AuthDiscoveryState>,
    pub device_sessions: Option<DeviceSessionListState>,
    pub account_management: Option<AccountManagementState>,
    pub account_management_capabilities: Option<AccountManagementCapabilities>,
    pub soft_logout_reauth: Option<SoftLogoutReauthState>,
    pub qr_login: Option<QrLoginState>,
    pub settings: Option<SettingsState>,
    pub link_preview_settings: Option<LinkPreviewSettingsState>,
    pub room_preferences: Option<RoomPreferencesState>,
    pub profile: Option<ProfileState>,
    pub sync: Option<SyncState>,
    pub sync_mode: Option<SyncMode>,
    pub navigation: Option<NavigationState>,
    pub spaces: Option<Vec<SpaceSummary>>,
    pub rooms: Option<Vec<RoomSummary>>,
    pub invites: Option<Vec<InvitePreview>>,
    pub invite_workflow: Option<InviteWorkflowState>,
    pub room_list: Option<RoomListProjection>,
    pub room_notification_settings: Option<HashMap<String, RoomNotificationSettings>>,
    pub room_interactions: Option<BTreeMap<String, RoomInteractionState>>,
    pub directory: Option<DirectoryState>,
    pub room_management: Option<RoomManagementState>,
    pub activity: Option<ActivityState>,
    pub timeline: Option<TimelinePaneState>,
    pub thread: Option<ThreadPaneState>,
    pub thread_attention: Option<ThreadAttentionState>,
    pub threads_list: Option<ThreadsListState>,
    pub focused_context: Option<FocusedContextState>,
    pub search: Option<SearchState>,
    pub search_crawler: Option<SearchCrawlerState>,
    pub files_view: Option<FilesViewState>,
    pub basic_operation: Option<BasicOperationState>,
    pub live_signals: Option<LiveSignalsState>,
    pub e2ee_trust: Option<E2eeTrustState>,
    pub local_encryption: Option<LocalEncryptionState>,
    pub native_attention: Option<NativeAttentionState>,
    pub cjk_text_policy: Option<CjkTextPolicyState>,
    pub errors: Option<Vec<AppError>>,
    pub sidebar: Option<SidebarModel>,
}

impl StateDeltaChangedSlices {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

pub fn build_state_delta(
    generation: u64,
    previous: &AppState,
    next: &AppState,
) -> Option<StateDelta> {
    audit_app_state_delta_slices(previous);
    audit_app_state_delta_slices(next);

    let mut changed = StateDeltaChangedSlices::default();

    macro_rules! changed_slice {
        ($field:ident) => {
            if previous.$field != next.$field {
                changed.$field = Some(next.$field.clone());
            }
        };
    }

    changed_slice!(session);
    changed_slice!(auth);
    changed_slice!(device_sessions);
    changed_slice!(account_management);
    changed_slice!(account_management_capabilities);
    changed_slice!(soft_logout_reauth);
    changed_slice!(qr_login);
    changed_slice!(settings);
    changed_slice!(link_preview_settings);
    changed_slice!(room_preferences);
    changed_slice!(profile);
    changed_slice!(sync);
    changed_slice!(sync_mode);
    changed_slice!(navigation);
    changed_slice!(spaces);
    changed_slice!(rooms);
    changed_slice!(invites);
    changed_slice!(invite_workflow);
    changed_slice!(room_list);
    changed_slice!(room_notification_settings);
    changed_slice!(room_interactions);
    changed_slice!(directory);
    changed_slice!(room_management);
    changed_slice!(activity);
    changed_slice!(timeline);
    changed_slice!(thread);
    changed_slice!(thread_attention);
    changed_slice!(threads_list);
    changed_slice!(focused_context);
    changed_slice!(search);
    changed_slice!(search_crawler);
    changed_slice!(files_view);
    changed_slice!(basic_operation);
    changed_slice!(live_signals);
    changed_slice!(e2ee_trust);
    changed_slice!(local_encryption);
    changed_slice!(native_attention);
    changed_slice!(cjk_text_policy);
    changed_slice!(errors);

    if previous.navigation.active_space_id != next.navigation.active_space_id
        || previous.spaces != next.spaces
        || previous.rooms != next.rooms
        || previous.room_notification_settings != next.room_notification_settings
    {
        let previous_sidebar = compose_sidebar_with_room_notification_settings(
            previous.navigation.active_space_id.as_deref(),
            &previous.spaces,
            &previous.rooms,
            &previous.room_notification_settings,
        );
        let next_sidebar = compose_sidebar_with_room_notification_settings(
            next.navigation.active_space_id.as_deref(),
            &next.spaces,
            &next.rooms,
            &next.room_notification_settings,
        );
        if previous_sidebar != next_sidebar {
            changed.sidebar = Some(next_sidebar);
        }
    }

    if changed.is_empty() {
        return None;
    }

    Some(StateDelta {
        generation,
        changed,
    })
}

fn audit_app_state_delta_slices(state: &AppState) {
    let AppState {
        session: _,
        auth: _,
        device_sessions: _,
        account_management: _,
        account_management_capabilities: _,
        soft_logout_reauth: _,
        qr_login: _,
        settings: _,
        link_preview_settings: _,
        room_preferences: _,
        profile: _,
        sync: _,
        sync_mode: _,
        navigation: _,
        spaces: _,
        rooms: _,
        invites: _,
        invite_workflow: _,
        room_list: _,
        room_notification_settings: _,
        room_interactions: _,
        composer_drafts: _,
        scheduled_sends: _,
        upload_staging: _,
        media_gallery: _,
        directory: _,
        room_management: _,
        activity: _,
        timeline: _,
        thread: _,
        thread_attention: _,
        threads_list: _,
        focused_context: _,
        search: _,
        search_crawler: _,
        files_view: _,
        basic_operation: _,
        live_signals: _,
        e2ee_trust: _,
        local_encryption: _,
        native_attention: _,
        cjk_text_policy: _,
        errors: _,
    } = state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::SearchCrawlerRoomState;

    #[test]
    fn state_delta_contains_only_changed_slices_and_sidebar_projection() {
        let previous = AppState::default();
        let mut next = previous.clone();
        next.search_crawler.rooms.insert(
            "!room:example.invalid".to_owned(),
            SearchCrawlerRoomState::Queued,
        );

        let delta = build_state_delta(1, &previous, &next).expect("state changed");

        assert_eq!(delta.generation, 1);
        assert!(delta.changed.search_crawler.is_some());
        assert!(delta.changed.session.is_none());
        assert!(delta.changed.sidebar.is_none());
    }

    #[test]
    fn state_delta_omits_unchanged_state() {
        assert!(build_state_delta(1, &AppState::default(), &AppState::default()).is_none());
    }

    #[test]
    fn state_delta_omits_sidebar_when_navigation_change_does_not_change_sidebar_projection() {
        let previous = AppState::default();
        let mut next = previous.clone();
        next.navigation.active_room_id = Some("!room:example.invalid".to_owned());

        let delta = build_state_delta(1, &previous, &next).expect("navigation changed");

        assert!(delta.changed.navigation.is_some());
        assert!(delta.changed.sidebar.is_none());
    }
}
