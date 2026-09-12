use koushi_state::{SettingsPatch, SettingsValues, UpdatesSettings};

#[test]
fn updates_default_on_patch_round_trip_and_legacy_backfill() {
    let mut values = SettingsValues::default();
    assert!(values.updates.auto_check);

    values.apply_patch(SettingsPatch {
        updates: Some(UpdatesSettings { auto_check: false }),
        ..SettingsPatch::default()
    });
    assert!(!values.updates.auto_check);

    let encoded = serde_json::to_value(&values).expect("serialize settings");
    let restored: SettingsValues =
        serde_json::from_value(encoded.clone()).expect("restore settings");
    assert!(!restored.updates.auto_check);

    let mut legacy = encoded;
    legacy
        .as_object_mut()
        .expect("settings object")
        .remove("updates");
    let backfilled: SettingsValues = serde_json::from_value(legacy).expect("backfill settings");
    assert!(backfilled.updates.auto_check);
}
