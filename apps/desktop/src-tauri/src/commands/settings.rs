use super::*;
#[cfg(test)]
use crate::commands::contracts::fake_request_id;
#[cfg(test)]
use koushi_state::{AppearanceSettings, LocaleSettings, TextDirectionPreference, ThemePreference};

#[tauri::command]
pub async fn update_settings(
    patch: SettingsPatch,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_update_settings_command(request_id, patch),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn import_legacy_settings(
    patch: SettingsPatch,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_import_legacy_settings_command(request_id, patch),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn rebuild_search_index(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_rebuild_search_index_command(request_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn set_room_url_preview_override(
    room_id: String,
    enabled: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_room_url_preview_override_command(request_id, room_id, enabled),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

pub(super) fn build_update_settings_command(
    request_id: koushi_protocol::RequestId,
    patch: SettingsPatch,
) -> CoreCommand {
    CoreCommand::App(AppCommand::UpdateSettings { request_id, patch })
}

pub(super) fn build_import_legacy_settings_command(
    request_id: koushi_protocol::RequestId,
    patch: SettingsPatch,
) -> CoreCommand {
    CoreCommand::App(AppCommand::ImportLegacySettings { request_id, patch })
}

pub(super) fn build_rebuild_search_index_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::RebuildSearchIndex { request_id })
}

pub(super) fn build_set_room_url_preview_override_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    enabled: bool,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetRoomUrlPreviewOverride {
        request_id,
        room_id,
        enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_settings_command_routes_patch_to_app_update_settings() {
        let command = build_update_settings_command(
            fake_request_id(23),
            SettingsPatch {
                appearance: Some(AppearanceSettings {
                    theme: ThemePreference::Dark,
                    ..AppearanceSettings::default()
                }),
                ..SettingsPatch::default()
            },
        );

        match command {
            CoreCommand::App(AppCommand::UpdateSettings { request_id, patch }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(
                    patch.appearance.expect("appearance patch").theme,
                    ThemePreference::Dark
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let debug = format!(
            "{:?}",
            build_update_settings_command(
                fake_request_id(24),
                SettingsPatch {
                    locale: Some(LocaleSettings {
                        language_tag: Some("ja-JP-private".to_owned()),
                        text_direction: TextDirectionPreference::Auto,
                    }),
                    ..SettingsPatch::default()
                },
            )
        );
        assert!(debug.contains("locale"), "{debug}");
        assert!(!debug.contains("ja-JP-private"), "{debug}");
    }

    #[test]
    fn import_legacy_settings_command_routes_to_the_typed_app_boundary() {
        let patch = SettingsPatch {
            appearance: Some(AppearanceSettings {
                density: koushi_state::DisplayDensity::Compact,
                ..AppearanceSettings::default()
            }),
            ..SettingsPatch::default()
        };
        let command = build_import_legacy_settings_command(fake_request_id(24), patch.clone());
        assert!(matches!(
            command,
            CoreCommand::App(AppCommand::ImportLegacySettings {
                request_id,
                patch: routed
            }) if request_id == fake_request_id(24) && routed == patch
        ));
    }

    #[test]
    fn set_room_url_preview_override_command_routes_to_app_state() {
        let command = build_set_room_url_preview_override_command(
            fake_request_id(24),
            "!room:example.invalid".to_owned(),
            false,
        );

        match command {
            CoreCommand::App(AppCommand::SetRoomUrlPreviewOverride {
                request_id,
                room_id,
                enabled,
            }) => {
                assert_eq!(request_id, fake_request_id(24));
                assert_eq!(room_id, "!room:example.invalid");
                assert!(!enabled);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let debug = format!(
            "{:?}",
            build_set_room_url_preview_override_command(
                fake_request_id(25),
                "!private-room:example.invalid".to_owned(),
                true,
            )
        );
        assert!(debug.contains("SetRoomUrlPreviewOverride"), "{debug}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    }
}
