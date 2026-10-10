//! Persistent conversation badges must project the same Rust room attention.
use koushi_state::{
    AppAction, AppState, NativeAttentionCapabilities, NativeAttentionObservationKind,
    NativeAttentionProjectionInput, RoomNotificationMode, RoomNotificationSettings, RoomSummary,
    RoomTags, SessionInfo, SessionState, SpaceSummary, account_attention_summary_for_state,
    compose_sidebar_for_state, native_attention_state_from_rooms, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.invalid".to_owned(),
            user_id: "@attention:example.invalid".to_owned(),
            device_id: "ATTENTION".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn room(
    room_id: &str,
    display_name: &str,
    is_dm: bool,
    unread_count: u64,
    notification_count: u64,
    highlight_count: u64,
) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        display_label: display_name.to_owned(),
        original_display_label: display_name.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count,
        notification_count,
        highlight_count,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn space(space_id: &str, child_room_ids: Vec<String>) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: Some("Space".to_owned()),
        display_name: "Space".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids,
        parent_side_child_room_ids: Vec::new(),
    }
}

fn dock_count(state: &AppState) -> u64 {
    let modes = state
        .room_notification_settings
        .iter()
        .map(|(id, settings)| (id.clone(), settings.mode))
        .collect();
    native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &state.rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &modes,
        ignored_user_ids: &state.profile.ignored_user_ids,
        window_focused: false,
        observation: NativeAttentionObservationKind::InitialSync,
        previous_candidate: None,
        message_previews: false,
        capabilities: NativeAttentionCapabilities::default(),
    })
    .summary
    .badge_count
}

#[test]
fn dock_and_sidebar_share_notification_mention_message_and_manual_attention_counts() {
    for (unread, notification, highlight, marked, expected) in [
        (0, 1, 0, false, 1),
        (0, 0, 2, false, 2),
        (3, 1, 0, false, 3),
        (1, 4, 2, false, 4),
        (0, 0, 0, true, 1),
        (0, 0, 0, false, 0),
    ] {
        let mut state = AppState::default();
        let mut room = room(
            "!room:example.invalid",
            "Room",
            false,
            unread,
            notification,
            highlight,
        );
        room.marked_unread = marked;
        state.rooms = vec![room];
        state.spaces = vec![space(
            "!space:example.invalid",
            vec!["!room:example.invalid".into()],
        )];
        state.navigation.active_space_id = Some("!space:example.invalid".into());
        let sidebar = compose_sidebar_for_state(&state);
        assert_eq!(sidebar.sections.rooms[0].unread_count, expected);
        assert_eq!(sidebar.space_rail[0].unread_count, expected);
        assert_eq!(sidebar.account_home.unread_count, expected);
        assert_eq!(
            account_attention_summary_for_state(&state).unread_count,
            expected
        );
        assert_eq!(
            dock_count(&state),
            expected,
            "persistent badges must agree for unread={unread}, notify={notification}, highlight={highlight}, marked={marked}"
        );
    }
}

#[test]
fn native_reducer_keeps_a_notification_only_badge_until_counts_clear() {
    let mut state = ready_state();
    let rooms = vec![room("!room:example.invalid", "Room", false, 0, 1, 0)];
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms,
        },
    );
    assert_eq!(state.native_attention.summary.badge_count, 1);
    assert_eq!(
        compose_sidebar_for_state(&state).account_home.unread_count,
        1
    );
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room("!room:example.invalid", "Room", false, 0, 0, 0)],
        },
    );
    assert_eq!(state.native_attention.summary.badge_count, 0);
    assert_eq!(
        compose_sidebar_for_state(&state).account_home.unread_count,
        0
    );
}

#[test]
fn persistent_badges_share_mute_exclusions_and_keep_mentions_only_attention() {
    for (mode, expected) in [
        (RoomNotificationMode::Mute, 0),
        (RoomNotificationMode::Mentions, 1),
    ] {
        let mut state = AppState {
            rooms: vec![room("!room:example.invalid", "Room", false, 0, 1, 0)],
            ..AppState::default()
        };
        state.room_notification_settings.insert(
            "!room:example.invalid".into(),
            RoomNotificationSettings {
                mode,
                ..RoomNotificationSettings::default()
            },
        );
        assert_eq!(
            compose_sidebar_for_state(&state).account_home.unread_count,
            expected
        );
        assert_eq!(dock_count(&state), expected);
    }
}
