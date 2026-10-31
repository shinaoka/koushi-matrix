use koushi_state::{
    DisplayPlatform, EmojiPreference, FontPreference, TypographyAssetStatus, TypographySettings,
    resolve_typography_display_profile,
};
use serde_json::json;

#[test]
fn default_typography_resolves_to_system_assets_on_each_platform() {
    for platform in [
        DisplayPlatform::Macos,
        DisplayPlatform::Windows,
        DisplayPlatform::Linux,
    ] {
        let profile = resolve_typography_display_profile(&TypographySettings::default(), platform);

        assert_eq!(profile.font, FontPreference::System);
        assert_eq!(profile.emoji, EmojiPreference::System);
        assert_eq!(profile.platform, platform);
        assert_eq!(profile.font_asset, TypographyAssetStatus::SystemFallback);
        assert_eq!(profile.emoji_asset, TypographyAssetStatus::SystemFallback);
    }
}

#[test]
fn bundled_preferences_request_bundled_assets_with_system_fallbacks() {
    let profile = resolve_typography_display_profile(
        &TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        },
        DisplayPlatform::Linux,
    );

    assert_eq!(profile.font, FontPreference::Inter);
    assert_eq!(profile.emoji, EmojiPreference::TwemojiColr);
    assert_eq!(profile.font_asset, TypographyAssetStatus::BundledPreferred);
    assert_eq!(profile.emoji_asset, TypographyAssetStatus::BundledPreferred);
}

#[test]
fn typography_profile_serializes_as_the_frontend_contract() {
    let profile = resolve_typography_display_profile(
        &TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        },
        DisplayPlatform::Windows,
    );

    assert_eq!(
        serde_json::to_value(profile).unwrap(),
        json!({
            "font": "inter",
            "emoji": "twemojiColr",
            "platform": "windows",
            "font_asset": "bundledPreferred",
            "emoji_asset": "bundledPreferred"
        })
    );
}
