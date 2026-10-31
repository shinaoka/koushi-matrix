//! `SettingsValues.mcp` (#1536): the app-scope switch for the local MCP bridge.

use koushi_state::{
    AppSettingsValues, McpSettings, SettingsPatch, SettingsPatchScope, SettingsValues,
};

#[test]
fn the_bridge_is_on_by_default_and_for_older_settings_files() {
    assert!(SettingsValues::default().mcp.enabled);
    let older: AppSettingsValues = serde_json::from_str("{}").expect("older app settings");
    assert!(older.mcp.enabled);
    let older_block: McpSettings = serde_json::from_str("{}").expect("older block");
    assert!(older_block.enabled);
}

#[test]
fn an_app_patch_turns_it_off_and_survives_the_app_projection() {
    let mut values = SettingsValues::default();
    let patch = SettingsPatch {
        scope: Some(SettingsPatchScope::App),
        mcp: Some(McpSettings { enabled: false }),
        ..SettingsPatch::default()
    };
    assert!(!patch.affects_account_settings());
    values.apply_patch(patch.clone());
    assert!(!values.mcp.enabled);

    let mut app = SettingsValues::default().app_settings();
    app.apply_patch(&patch);
    assert!(!app.mcp.enabled);

    let mut restored = SettingsValues::default();
    restored.apply_app_settings(&app);
    assert!(!restored.mcp.enabled);
}

#[test]
fn an_account_patch_cannot_change_it() {
    let mut values = SettingsValues::default();
    values.apply_patch(SettingsPatch {
        scope: Some(SettingsPatchScope::Account),
        mcp: Some(McpSettings { enabled: false }),
        ..SettingsPatch::default()
    });
    assert!(values.mcp.enabled);
}
