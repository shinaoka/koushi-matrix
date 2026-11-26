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
        mcp: Some(McpSettings {
            enabled: false,
            ..McpSettings::default()
        }),
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
        mcp: Some(McpSettings {
            enabled: false,
            ..McpSettings::default()
        }),
        ..SettingsPatch::default()
    });
    assert!(values.mcp.enabled);
}

#[test]
fn the_allow_list_is_derived_from_the_signed_in_accounts_and_editable_both_ways() {
    use koushi_state::MentionAllowedSenders;
    let signed_in = ["@a:x", "@b:x"];
    let default = MentionAllowedSenders::default();
    assert_eq!(
        default.effective(signed_in),
        ["@a:x", "@b:x"].map(str::to_owned).into()
    );
    // An account added later is covered without editing anything.
    assert!(default.effective(["@a:x", "@b:x", "@c:x"]).contains("@c:x"));
    // Additive on top of the derived default, and the derived one can be removed.
    let edited = MentionAllowedSenders {
        added: vec!["@guest:x".to_owned()],
        removed: vec!["@b:x".to_owned()],
    };
    assert_eq!(
        edited.effective(signed_in),
        ["@a:x", "@guest:x"].map(str::to_owned).into()
    );
}

#[test]
fn canonicalizing_the_allow_list_trims_dedups_and_lets_added_win() {
    use koushi_state::MentionAllowedSenders;
    let mut edited = MentionAllowedSenders {
        added: vec![" @a:x ".to_owned(), "@a:x".to_owned(), String::new()],
        removed: vec!["@a:x".to_owned(), "@b:x".to_owned()],
    };
    edited.canonicalize();
    assert_eq!(edited.added, ["@a:x"]);
    assert_eq!(edited.removed, ["@b:x"]);
}
