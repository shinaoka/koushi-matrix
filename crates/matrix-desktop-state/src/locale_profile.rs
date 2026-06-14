use serde::{Deserialize, Serialize};

use crate::state::{LocaleSettings, TextDirectionPreference};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocaleDisplayProfile {
    pub lang: String,
    pub dir: LocaleDirection,
    pub catalog_locale: CatalogLocale,
    pub pseudo_locale: PseudoLocaleMode,
    pub platform: DisplayPlatform,
    pub modifier_labels: ModifierLabelProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocaleDirection {
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogLocale {
    En,
    Ja,
    Pseudo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PseudoLocaleMode {
    None,
    Accented,
    Bidi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayPlatform {
    Macos,
    Windows,
    Linux,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModifierLabelProfile {
    pub primary: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedLanguage {
    primary: SupportedLanguage,
    pseudo_locale: PseudoLocaleMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SupportedLanguage {
    En,
    Ja,
    Rtl,
}

pub fn resolve_locale_display_profile(
    settings: &LocaleSettings,
    platform: DisplayPlatform,
) -> LocaleDisplayProfile {
    let parsed = settings
        .language_tag
        .as_deref()
        .and_then(parse_language_tag);
    let pseudo_locale = parsed
        .map(|language| language.pseudo_locale)
        .unwrap_or(PseudoLocaleMode::None);
    let catalog_locale = catalog_locale(parsed);
    let lang = resolved_lang(catalog_locale, pseudo_locale).to_owned();
    let dir = match settings.text_direction {
        TextDirectionPreference::Ltr => LocaleDirection::Ltr,
        TextDirectionPreference::Rtl => LocaleDirection::Rtl,
        TextDirectionPreference::Auto => auto_direction(parsed, pseudo_locale),
    };

    LocaleDisplayProfile {
        lang,
        dir,
        catalog_locale,
        pseudo_locale,
        platform,
        modifier_labels: ModifierLabelProfile {
            primary: platform.primary_modifier_label().to_owned(),
        },
    }
}

fn catalog_locale(parsed: Option<ParsedLanguage>) -> CatalogLocale {
    match parsed {
        Some(ParsedLanguage {
            pseudo_locale: PseudoLocaleMode::Accented | PseudoLocaleMode::Bidi,
            ..
        }) => CatalogLocale::Pseudo,
        Some(ParsedLanguage {
            primary: SupportedLanguage::Ja,
            pseudo_locale: PseudoLocaleMode::None,
        }) => CatalogLocale::Ja,
        _ => CatalogLocale::En,
    }
}

fn resolved_lang(catalog_locale: CatalogLocale, pseudo_locale: PseudoLocaleMode) -> &'static str {
    match pseudo_locale {
        PseudoLocaleMode::Accented => "en-XA",
        PseudoLocaleMode::Bidi => "ar-XB",
        PseudoLocaleMode::None => match catalog_locale {
            CatalogLocale::Ja => "ja",
            CatalogLocale::En | CatalogLocale::Pseudo => "en",
        },
    }
}

fn auto_direction(
    parsed: Option<ParsedLanguage>,
    pseudo_locale: PseudoLocaleMode,
) -> LocaleDirection {
    if pseudo_locale == PseudoLocaleMode::Bidi {
        return LocaleDirection::Rtl;
    }

    match parsed.map(|language| language.primary) {
        Some(SupportedLanguage::Rtl) => LocaleDirection::Rtl,
        Some(SupportedLanguage::En | SupportedLanguage::Ja) | None => LocaleDirection::Ltr,
    }
}

fn parse_language_tag(language_tag: &str) -> Option<ParsedLanguage> {
    let normalized = language_tag.trim().replace('_', "-");
    let mut subtags = normalized.split('-');
    let primary = subtags.next()?.to_ascii_lowercase();
    if !is_primary_language_subtag(&primary) {
        return None;
    }

    let rest = subtags.collect::<Vec<_>>();
    if rest.iter().any(|subtag| subtag.eq_ignore_ascii_case("x")) {
        return None;
    }
    if !rest.iter().all(|subtag| is_valid_language_subtag(subtag)) {
        return None;
    }

    let pseudo_locale = if normalized.eq_ignore_ascii_case("en-XA") {
        PseudoLocaleMode::Accented
    } else if normalized.eq_ignore_ascii_case("ar-XB") {
        PseudoLocaleMode::Bidi
    } else {
        PseudoLocaleMode::None
    };

    Some(ParsedLanguage {
        primary: supported_language(&primary)?,
        pseudo_locale,
    })
}

fn is_primary_language_subtag(subtag: &str) -> bool {
    (2..=3).contains(&subtag.len()) && subtag.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn is_valid_language_subtag(subtag: &str) -> bool {
    (1..=8).contains(&subtag.len()) && subtag.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn supported_language(primary: &str) -> Option<SupportedLanguage> {
    match primary {
        "en" => Some(SupportedLanguage::En),
        "ja" => Some(SupportedLanguage::Ja),
        "ar" | "dv" | "fa" | "he" | "ps" | "sd" | "ug" | "ur" | "yi" => {
            Some(SupportedLanguage::Rtl)
        }
        _ => None,
    }
}

impl DisplayPlatform {
    fn primary_modifier_label(self) -> &'static str {
        match self {
            DisplayPlatform::Macos => "Cmd",
            DisplayPlatform::Windows | DisplayPlatform::Linux => "Ctrl",
        }
    }
}
