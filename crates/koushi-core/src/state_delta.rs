//! Incremental AppState slice deltas.

use koushi_protocol::state_update::{StateDelta, StateDeltaChangedSlices};
use koushi_state::{AppState, compose_sidebar_for_state};

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
    changed_slice!(session_lock_reason);
    changed_slice!(secure_backup_gate);
    changed_slice!(device_cleanup);
    changed_slice!(current_session_status);
    changed_slice!(auth);
    changed_slice!(account_management_url);
    changed_slice!(account_management);
    changed_slice!(account_management_capabilities);
    changed_slice!(soft_logout_reauth);
    changed_slice!(qr_login);
    changed_slice!(settings);
    changed_slice!(link_preview_settings);
    changed_slice!(room_preferences);
    changed_slice!(profile);
    changed_slice!(space_members);
    changed_slice!(sync);
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
    changed_slice!(mention_candidates);
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
        || previous.navigation.space_order != next.navigation.space_order
        || previous.navigation.space_local_presentations
            != next.navigation.space_local_presentations
        || previous.settings.values.room_list_sort != next.settings.values.room_list_sort
        || previous.spaces != next.spaces
        || previous.rooms != next.rooms
        || previous.invites != next.invites
        || previous.room_notification_settings != next.room_notification_settings
    {
        let previous_sidebar = compose_sidebar_for_state(previous);
        let next_sidebar = compose_sidebar_for_state(next);
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
        session_lock_reason: _,
        secure_backup_gate: _,
        sliding_sync_account_epoch: _,
        sliding_sync_capability: _,
        current_session_status: _,
        auth: _,
        account_management_url: _,
        account_management: _,
        account_management_capabilities: _,
        soft_logout_reauth: _,
        qr_login: _,
        settings: _,
        link_preview_settings: _,
        room_preferences: _,
        profile: _,
        space_members: _,
        sync: _,
        sync_generation: _,
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
        mention_candidates: _,
        activity: _,
        timeline: _,
        thread: _,
        thread_attention: _,
        threads_list: _,
        thread_root_projections: _,
        focused_context: _,
        search: _,
        search_crawler: _,
        files_view: _,
        basic_operation: _,
        live_signals: _,
        e2ee_trust: _,
        local_encryption: _,
        native_attention: _,
        native_attention_context: _,
        cjk_text_policy: _,
        errors: _,
        device_cleanup: _,
    } = state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::{
        AccountManagementUrl, DeviceCleanupOfferReason, DeviceCleanupState,
        MentionCandidatesCompleteness, MentionCandidatesTarget, MentionSurface,
        RoomMentionPermission, SearchCrawlerRoomState, SecureBackupGateState,
    };

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
    fn account_management_url_clear_is_an_explicit_delta() {
        let mut previous = AppState::default();
        previous.account_management_url = Some(AccountManagementUrl::from_validated(
            "https://account.example/devices".to_owned(),
        ));
        let next = AppState::default();

        let delta = build_state_delta(2, &previous, &next).expect("URL clear changed state");

        assert_eq!(delta.changed.account_management_url, Some(None));
    }

    #[test]
    fn state_delta_omits_unchanged_state() {
        assert!(build_state_delta(1, &AppState::default(), &AppState::default()).is_none());
    }

    #[test]
    fn session_lock_reason_delta_preserves_nested_some_and_explicit_none() {
        let mut locked = AppState::default();
        locked.session_lock_reason =
            Some(koushi_state::SessionLockReason::UnknownToken { soft_logout: true });
        let delta = build_state_delta(2, &AppState::default(), &locked).expect("reason changed");
        assert_eq!(
            delta.changed.session_lock_reason,
            Some(Some(koushi_state::SessionLockReason::UnknownToken {
                soft_logout: true,
            }))
        );

        let clear = build_state_delta(3, &locked, &AppState::default()).expect("reason cleared");
        assert_eq!(clear.changed.session_lock_reason, Some(None));
    }

    #[test]
    fn device_cleanup_state_is_an_incremental_slice() {
        let previous = AppState::default();
        let mut next = previous.clone();
        next.device_cleanup = DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::RecoveryFailed,
        };

        let delta = build_state_delta(7, &previous, &next).expect("cleanup state changed");

        assert_eq!(delta.changed.device_cleanup, Some(next.device_cleanup));
        let mut without_cleanup = delta.changed;
        without_cleanup.device_cleanup = None;
        assert!(without_cleanup.is_empty());
    }

    #[test]
    fn secure_backup_gate_is_an_incremental_slice() {
        let previous = AppState::default();
        let mut next = previous.clone();
        next.secure_backup_gate = SecureBackupGateState::Checking;

        let delta = build_state_delta(8, &previous, &next).expect("backup gate changed");

        assert_eq!(
            delta.changed.secure_backup_gate,
            Some(SecureBackupGateState::Checking)
        );
        let mut without_gate = delta.changed;
        without_gate.secure_backup_gate = None;
        assert!(without_gate.is_empty());
    }

    #[test]
    fn state_delta_emits_only_the_changed_mention_candidates_slice() {
        let previous = AppState::default();
        let mut next = previous.clone();
        next.mention_candidates
            .targets
            .push(MentionCandidatesTarget {
                room_id: "!room:example.invalid".to_owned(),
                generation: 1,
                request_id: 2,
                query: "ali".to_owned(),
                surface: MentionSurface::Main,
                completeness: MentionCandidatesCompleteness::Partial,
                candidates: Vec::new(),
                room_mention_allowed: RoomMentionPermission::Allowed,
                failure_kind: None,
            });

        let delta = build_state_delta(2, &previous, &next).expect("mention candidates changed");

        assert_eq!(
            delta.changed.mention_candidates,
            Some(next.mention_candidates)
        );
        let mut without_mentions = delta.changed;
        without_mentions.mention_candidates = None;
        assert!(without_mentions.is_empty());
    }

    #[test]
    fn navigation_delta_retains_event_navigation() {
        let previous = AppState::default();
        let mut next = previous.clone();
        next.navigation.active_room_id = Some("!room:example.invalid".to_owned());
        next.navigation.event_navigation = koushi_state::EventNavigationState::Opening {
            generation: 1,
            source: koushi_state::EventNavigationSource::Activity,
        };

        let delta = build_state_delta(1, &previous, &next).expect("navigation changed");

        assert_eq!(delta.changed.navigation, Some(next.navigation.clone()));
        assert_eq!(
            delta
                .changed
                .navigation
                .as_ref()
                .map(|navigation| navigation.event_navigation),
            Some(koushi_state::EventNavigationState::Opening {
                generation: 1,
                source: koushi_state::EventNavigationSource::Activity,
            })
        );
        assert!(delta.changed.sidebar.is_none());
    }
}
