use serde::{Deserialize, Serialize};

use crate::{
    locale_profile::DisplayPlatform,
    state::{EmojiPreference, FontPreference, TypographySettings},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypographyDisplayProfile {
    pub font: FontPreference,
    pub emoji: EmojiPreference,
    pub platform: DisplayPlatform,
    pub font_asset: TypographyAssetStatus,
    pub emoji_asset: TypographyAssetStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypographyAssetStatus {
    SystemFallback,
    BundledPreferred,
}

pub fn resolve_typography_display_profile(
    settings: &TypographySettings,
    platform: DisplayPlatform,
) -> TypographyDisplayProfile {
    TypographyDisplayProfile {
        font: settings.font.clone(),
        emoji: settings.emoji.clone(),
        platform,
        font_asset: font_asset_status(&settings.font),
        emoji_asset: emoji_asset_status(&settings.emoji),
    }
}

fn font_asset_status(font: &FontPreference) -> TypographyAssetStatus {
    match font {
        FontPreference::System => TypographyAssetStatus::SystemFallback,
        FontPreference::Inter => TypographyAssetStatus::BundledPreferred,
    }
}

fn emoji_asset_status(emoji: &EmojiPreference) -> TypographyAssetStatus {
    match emoji {
        EmojiPreference::System => TypographyAssetStatus::SystemFallback,
        EmojiPreference::TwemojiColr => TypographyAssetStatus::BundledPreferred,
    }
}
