//! #1219: the account tab and that account's Home aggregate must share one
//! Rust-owned per-account attention policy.
//!
//! The tab badge is not the native OS-notification or Dock-badge policy: it is
//! the account's actionable attention, so it must count the same rooms and
//! pending invites the Home rail counts, including for background accounts.

use std::collections::HashMap;

use koushi_state::{
    AppState, InvitePreview, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomNotificationMode,
    RoomNotificationSettings, RoomSummary, RoomTags, SessionInfo, SessionState,
    compose_sidebar_for_state, native_attention_state_from_rooms,
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

fn invite(room_id: &str) -> InvitePreview {
    InvitePreview {
        room_id: room_id.to_owned(),
        display_name: "Invited Room".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        inviter_display_name: None,
        inviter_user_id: None,
        is_dm: false,
        is_space: false,
    }
}

fn available_capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

/// The account tab's per-account attention, as the Tauri snapshot projects it.
fn account_tab_attention(state: &AppState) -> u64 {
    koushi_state::account_attention_summary_for_state(state).attention_count
}

/// The persistent Dock/taskbar badge, a separate Rust policy.
fn dock_badge_count(state: &AppState) -> u64 {
    let room_notification_modes: HashMap<String, RoomNotificationMode> = state
        .room_notification_settings
        .iter()
        .map(|(room_id, settings)| (room_id.clone(), settings.mode))
        .collect();
    native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &state.rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &room_notification_modes,
        ignored_user_ids: &state.profile.ignored_user_ids,
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        message_previews: false,
        capabilities: available_capabilities(),
    })
    .summary
    .badge_count
}

#[test]
fn unread_dm_account_tab_matches_the_home_aggregate() {
    // A non-muted DM with raw unread content but zero notification/highlight
    // counters: Home/DM count 1, the tab must too.
    let mut state = ready_state();
    state
        .rooms
        .push(room("!dm:example.invalid", "Direct", true, 1, 0, 0));

    let home = compose_sidebar_for_state(&state).account_home;
    assert_eq!(home.unread_count, 1);
    assert_eq!(account_tab_attention(&state), home.attention_count);
}

#[test]
fn invite_only_account_tab_matches_the_home_aggregate() {
    // Home counts the pending invite; an invite-only account must still badge.
    let mut state = ready_state();
    state.invites.push(invite("!invited:example.invalid"));

    let home = compose_sidebar_for_state(&state).account_home;
    assert_eq!(home.attention_count, 1);
    assert_eq!(account_tab_attention(&state), home.attention_count);
}

#[test]
fn muted_and_low_priority_rooms_follow_the_home_aggregate() {
    let mut state = ready_state();
    state
        .rooms
        .push(room("!counted:example.invalid", "Counted", false, 4, 4, 0));
    let mut low = room("!low:example.invalid", "Low", false, 6, 6, 0);
    low.tags.low_priority = Some(koushi_state::RoomTagInfo { order: None });
    state.rooms.push(low);
    state
        .rooms
        .push(room("!muted:example.invalid", "Muted", false, 5, 5, 0));
    state.room_notification_settings.insert(
        "!muted:example.invalid".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mute,
            ..RoomNotificationSettings::default()
        },
    );

    let home = compose_sidebar_for_state(&state).account_home;
    assert_eq!(home.unread_count, 4, "only the counted room contributes");
    assert_eq!(account_tab_attention(&state), home.attention_count);
}

#[test]
fn mentions_only_room_keeps_the_tab_exclusion_and_home_follows_it() {
    let mut state = ready_state();
    state.rooms.push(room(
        "!mentions:example.invalid",
        "Mentions",
        false,
        3,
        3,
        0,
    ));
    state.room_notification_settings.insert(
        "!mentions:example.invalid".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mentions,
            ..RoomNotificationSettings::default()
        },
    );

    let sidebar = compose_sidebar_for_state(&state);
    assert_eq!(
        sidebar.account_home.unread_count, 0,
        "a mentions-only room without a highlight is not actionable attention"
    );
    assert_eq!(sidebar.space_unread_count, 0);
    assert_eq!(sidebar.dm_unread_count, 0);
    assert_eq!(
        account_tab_attention(&state),
        sidebar.account_home.attention_count
    );
}

#[test]
fn mentions_only_room_with_a_highlight_still_contributes() {
    let mut state = ready_state();
    state
        .rooms
        .push(room("!mention:example.invalid", "Mention", false, 3, 3, 1));
    state.room_notification_settings.insert(
        "!mention:example.invalid".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mentions,
            ..RoomNotificationSettings::default()
        },
    );

    let home = compose_sidebar_for_state(&state).account_home;
    assert_eq!(home.unread_count, 3);
    assert_eq!(home.highlight_count, 1);
    assert_eq!(account_tab_attention(&state), home.attention_count);
}

#[test]
fn background_account_attention_never_leaks_into_another_tab() {
    let mut alice = ready_state();
    alice
        .rooms
        .push(room("!alice:example.invalid", "Alice Room", false, 2, 2, 0));
    let mut bob = ready_state();
    bob.invites.push(invite("!bob-invite:example.invalid"));

    let alice_home = compose_sidebar_for_state(&alice).account_home;
    let bob_home = compose_sidebar_for_state(&bob).account_home;
    assert_eq!(account_tab_attention(&alice), alice_home.attention_count);
    assert_eq!(account_tab_attention(&bob), bob_home.attention_count);
    assert_ne!(alice_home.attention_count, 0);
    assert_ne!(bob_home.attention_count, 0);
    // Each tab reads only its own account's rooms and invites.
    assert!(
        alice
            .rooms
            .iter()
            .all(|room| room.room_id != "!bob-invite:example.invalid")
    );
    assert!(
        bob.invites
            .iter()
            .all(|invite| invite.room_id != "!alice:example.invalid")
    );
}

#[test]
fn dock_badge_policy_stays_separate_from_the_account_tab() {
    // The Dock/taskbar badge keeps its own intended count: a mentions-only room
    // still contributes its raw unread there, while the account tab treats it as
    // non-actionable.
    let mut state = ready_state();
    state.rooms.push(room(
        "!mentions:example.invalid",
        "Mentions",
        false,
        3,
        0,
        0,
    ));
    state.room_notification_settings.insert(
        "!mentions:example.invalid".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mentions,
            ..RoomNotificationSettings::default()
        },
    );

    assert_eq!(account_tab_attention(&state), 0);
    assert_eq!(dock_badge_count(&state), 3);
}
