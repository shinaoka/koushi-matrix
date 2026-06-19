use koushi_state::{
    CatalogLocale, DisplayPlatform, LocaleDirection, LocaleSettings, PseudoLocaleMode,
    TextDirectionPreference, cjk_display_sort_key, normalize_cjk_search_text,
    resolve_locale_display_profile,
};
use serde_json::json;

fn locale_settings(language_tag: &str, text_direction: TextDirectionPreference) -> LocaleSettings {
    LocaleSettings {
        language_tag: Some(language_tag.to_owned()),
        text_direction,
    }
}

#[test]
fn default_locale_resolves_to_english_ltr_with_platform_modifier_labels() {
    let profile =
        resolve_locale_display_profile(&LocaleSettings::default(), DisplayPlatform::Linux);

    assert_eq!(profile.lang, "en");
    assert_eq!(profile.dir, LocaleDirection::Ltr);
    assert_eq!(profile.catalog_locale, CatalogLocale::En);
    assert_eq!(profile.pseudo_locale, PseudoLocaleMode::None);
    assert_eq!(profile.platform, DisplayPlatform::Linux);
    assert_eq!(profile.modifier_labels.primary, "Ctrl");

    let mac = resolve_locale_display_profile(&LocaleSettings::default(), DisplayPlatform::Macos);
    let windows =
        resolve_locale_display_profile(&LocaleSettings::default(), DisplayPlatform::Windows);
    assert_eq!(mac.modifier_labels.primary, "Cmd");
    assert_eq!(windows.modifier_labels.primary, "Ctrl");
}

#[test]
fn supported_cjk_locale_resolves_to_catalog_language_without_raw_region_branching() {
    let profile = resolve_locale_display_profile(
        &locale_settings("ja-JP", TextDirectionPreference::Auto),
        DisplayPlatform::Macos,
    );

    assert_eq!(profile.lang, "ja");
    assert_eq!(profile.dir, LocaleDirection::Ltr);
    assert_eq!(profile.catalog_locale, CatalogLocale::Ja);
    assert_eq!(profile.pseudo_locale, PseudoLocaleMode::None);
}

#[test]
fn cjk_search_normalization_folds_width_case_and_kana_variants() {
    assert_eq!(normalize_cjk_search_text("ＡＢＣ１２３"), "abc123");
    assert_eq!(
        normalize_cjk_search_text("ﾊﾝｶｸ"),
        normalize_cjk_search_text("ハンカク")
    );
    assert_eq!(normalize_cjk_search_text("ﾊﾞﾅﾅ"), "バナナ");
    assert_eq!(normalize_cjk_search_text("ﾊﾟﾝ"), "パン");
    assert_eq!(normalize_cjk_search_text("ｳﾞｨｵﾗ"), "ヴィオラ");
    assert_eq!(normalize_cjk_search_text("ハ\u{3099}ナナ"), "バナナ");
}

#[test]
fn cjk_room_and_people_sort_keys_are_deterministic_for_mixed_scripts() {
    assert_eq!(
        cjk_display_sort_key("Alpha"),
        cjk_display_sort_key("ａｌｐｈａ")
    );
    assert_eq!(cjk_display_sort_key("アイ"), cjk_display_sort_key("ｱｲ"));
    assert!(cjk_display_sort_key("会議2") < cjk_display_sort_key("会議10"));

    let mut names = vec!["会議10", "ａｌｐｈａ", "アイ", "会議2", "Alpha", "ｱｲ"];
    names.sort_by_key(|name| cjk_display_sort_key(name));

    assert_eq!(
        names,
        vec!["ａｌｐｈａ", "Alpha", "アイ", "ｱｲ", "会議2", "会議10"]
    );
}

#[test]
fn english_region_locale_resolves_to_english_catalog_not_cjk_fallback() {
    let profile = resolve_locale_display_profile(
        &locale_settings("en-US", TextDirectionPreference::Auto),
        DisplayPlatform::Windows,
    );

    assert_eq!(profile.lang, "en");
    assert_eq!(profile.dir, LocaleDirection::Ltr);
    assert_eq!(profile.catalog_locale, CatalogLocale::En);
    assert_eq!(profile.pseudo_locale, PseudoLocaleMode::None);
}

#[test]
fn auto_direction_uses_script_defaults_and_explicit_direction_overrides_them() {
    let rtl = resolve_locale_display_profile(
        &locale_settings("ar-EG", TextDirectionPreference::Auto),
        DisplayPlatform::Linux,
    );
    assert_eq!(rtl.lang, "en");
    assert_eq!(rtl.dir, LocaleDirection::Rtl);
    assert_eq!(rtl.catalog_locale, CatalogLocale::En);

    let forced_ltr = resolve_locale_display_profile(
        &locale_settings("ar-EG", TextDirectionPreference::Ltr),
        DisplayPlatform::Linux,
    );
    assert_eq!(forced_ltr.dir, LocaleDirection::Ltr);

    let forced_rtl = resolve_locale_display_profile(
        &locale_settings("ja-JP", TextDirectionPreference::Rtl),
        DisplayPlatform::Linux,
    );
    assert_eq!(forced_rtl.dir, LocaleDirection::Rtl);
}

#[test]
fn pseudo_locale_tags_resolve_without_feature_components_parsing_raw_tags() {
    let accented = resolve_locale_display_profile(
        &locale_settings("en-XA", TextDirectionPreference::Auto),
        DisplayPlatform::Linux,
    );
    assert_eq!(accented.lang, "en-XA");
    assert_eq!(accented.dir, LocaleDirection::Ltr);
    assert_eq!(accented.catalog_locale, CatalogLocale::Pseudo);
    assert_eq!(accented.pseudo_locale, PseudoLocaleMode::Accented);

    let bidi = resolve_locale_display_profile(
        &locale_settings("ar-XB", TextDirectionPreference::Auto),
        DisplayPlatform::Linux,
    );
    assert_eq!(bidi.lang, "ar-XB");
    assert_eq!(bidi.dir, LocaleDirection::Rtl);
    assert_eq!(bidi.catalog_locale, CatalogLocale::Pseudo);
    assert_eq!(bidi.pseudo_locale, PseudoLocaleMode::Bidi);
}

#[test]
fn locale_profile_serializes_as_the_canonical_frontend_shape() {
    let profile = resolve_locale_display_profile(
        &locale_settings("en-XA", TextDirectionPreference::Auto),
        DisplayPlatform::Macos,
    );

    assert_eq!(
        serde_json::to_value(profile).unwrap(),
        json!({
            "lang": "en-XA",
            "dir": "ltr",
            "catalog_locale": "pseudo",
            "pseudo_locale": "accented",
            "platform": "macos",
            "modifier_labels": {
                "primary": "Cmd"
            }
        })
    );
}

#[test]
fn unsupported_or_private_locale_input_does_not_leak_into_profile_diagnostics() {
    let profile = resolve_locale_display_profile(
        &locale_settings("zz-ZZ-private-room-alpha", TextDirectionPreference::Auto),
        DisplayPlatform::Linux,
    );

    assert_eq!(profile.lang, "en");
    assert_eq!(profile.dir, LocaleDirection::Ltr);
    assert_eq!(profile.catalog_locale, CatalogLocale::En);

    let debug = format!("{profile:?}");
    assert!(!debug.contains("zz-ZZ"));
    assert!(!debug.contains("private-room-alpha"));
}

#[test]
fn private_use_suffix_on_supported_locale_is_sanitized_to_default_profile() {
    let japanese_private = resolve_locale_display_profile(
        &locale_settings("ja-x-private-room-alpha", TextDirectionPreference::Auto),
        DisplayPlatform::Linux,
    );
    assert_eq!(japanese_private.lang, "en");
    assert_eq!(japanese_private.dir, LocaleDirection::Ltr);
    assert_eq!(japanese_private.catalog_locale, CatalogLocale::En);

    let rtl_private = resolve_locale_display_profile(
        &locale_settings("he-x-private-room-alpha", TextDirectionPreference::Auto),
        DisplayPlatform::Linux,
    );
    assert_eq!(rtl_private.lang, "en");
    assert_eq!(rtl_private.dir, LocaleDirection::Ltr);
    assert_eq!(rtl_private.catalog_locale, CatalogLocale::En);

    let debug = format!("{japanese_private:?} {rtl_private:?}");
    assert!(!debug.contains("private-room-alpha"));
}
