use koushi_state::{
    AppAction, AppEffect, AppState, DisplaySettings, SettingsPatch, SettingsValues, UiEvent, reduce,
};

fn ready_state_with_room(room_id: &str) -> AppState {
    use koushi_state::{RoomSummary, RoomTags, SessionInfo, SessionState};

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
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        ..AppState::default()
    }
}

#[test]
fn display_settings_default_enables_url_previews() {
    let values = SettingsValues::default();
    assert!(values.display.url_previews_enabled);
    assert!(values.display.encrypted_url_previews_enabled);
    assert!(
        AppState::default()
            .link_preview_settings
            .room_overrides
            .is_empty()
    );
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
                    encrypted_url_previews_enabled: false,
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
fn settings_update_toggles_encrypted_global_url_previews() {
    let mut state = ready_state_with_room("!room:example.invalid");
    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 4,
            patch: SettingsPatch {
                display: Some(DisplaySettings {
                    code_block_wrap: true,
                    hide_redacted: false,
                    url_previews_enabled: true,
                    encrypted_url_previews_enabled: true,
                }),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(state.settings.values.display.encrypted_url_previews_enabled);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::PersistSettings { request_id: 4, .. }))
    );
}

#[test]
fn per_room_override_updates_runtime_state_without_persisting_settings() {
    let mut state = ready_state_with_room("!room:example.invalid");

    let effects = reduce(
        &mut state,
        AppAction::RoomUrlPreviewOverrideSet {
            request_id: 2,
            room_id: "!room:example.invalid".to_owned(),
            enabled: false,
        },
    );

    assert_eq!(
        state
            .link_preview_settings
            .room_overrides
            .get("!room:example.invalid"),
        Some(&false)
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::EmitUiEvent(UiEvent::LinkPreviewSettingsChanged)
    )));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::PersistSettings { .. }))
    );
    let persisted = serde_json::to_string(&state.settings.values).unwrap();
    assert!(!persisted.contains("room_url_previews"));
    assert!(!persisted.contains("!room:example.invalid"));
}

#[test]
fn default_room_override_removes_runtime_entry() {
    let mut state = ready_state_with_room("!room:example.invalid");
    state
        .link_preview_settings
        .room_overrides
        .insert("!room:example.invalid".to_owned(), false);

    reduce(
        &mut state,
        AppAction::RoomUrlPreviewOverrideSet {
            request_id: 3,
            room_id: "!room:example.invalid".to_owned(),
            enabled: true,
        },
    );

    assert!(
        !state
            .link_preview_settings
            .room_overrides
            .contains_key("!room:example.invalid")
    );
}

#[test]
fn encrypted_room_override_uses_encrypted_global_default() {
    let mut state = ready_state_with_room("!room:example.invalid");
    state.rooms[0].is_encrypted = true;
    state.settings.values.display.encrypted_url_previews_enabled = true;
    state
        .link_preview_settings
        .room_overrides
        .insert("!room:example.invalid".to_owned(), false);

    reduce(
        &mut state,
        AppAction::RoomUrlPreviewOverrideSet {
            request_id: 5,
            room_id: "!room:example.invalid".to_owned(),
            enabled: true,
        },
    );

    assert!(
        !state
            .link_preview_settings
            .room_overrides
            .contains_key("!room:example.invalid")
    );
}
