use std::collections::BTreeMap;

use matrix_desktop_state::{
    AppAction, AppEffect, AppState, DisplaySettings, SettingsPatch, SettingsValues, UiEvent, reduce,
};

fn ready_state_with_room(room_id: &str) -> AppState {
    use matrix_desktop_state::{RoomSummary, RoomTags, SessionInfo, SessionState};

    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        }),
        rooms: vec![RoomSummary {
            room_id: room_id.to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: Vec::new(),
            is_encrypted: false,
        }],
        ..AppState::default()
    }
}

#[test]
fn display_settings_default_enables_url_previews() {
    let values = SettingsValues::default();
    assert!(values.display.url_previews_enabled);
    assert!(values.room_url_previews.is_empty());
}

#[test]
fn settings_update_toggles_global_url_previews() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: false,
                }),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(!state.settings.values.display.url_previews_enabled);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::PersistSettings { request_id: 1, .. }))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::EmitUiEvent(UiEvent::SettingsChanged)))
    );
}

#[test]
fn per_room_override_merges_into_settings() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let mut overrides = BTreeMap::new();
    overrides.insert("!room:example.invalid".to_owned(), true);

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 2,
            patch: SettingsPatch {
                room_url_previews: Some(overrides),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(
        state
            .settings
            .values
            .room_url_previews
            .get("!room:example.invalid"),
        Some(&true)
    );
}

#[test]
fn false_override_removes_room_entry() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let mut overrides = BTreeMap::new();
    overrides.insert("!room:example.invalid".to_owned(), true);
    state.settings.values.room_url_previews = overrides;

    let mut remove = BTreeMap::new();
    remove.insert("!room:example.invalid".to_owned(), false);
    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 3,
            patch: SettingsPatch {
                room_url_previews: Some(remove),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(
        !state
            .settings
            .values
            .room_url_previews
            .contains_key("!room:example.invalid")
    );
}
