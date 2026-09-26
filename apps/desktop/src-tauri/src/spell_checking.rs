//! Spell checking for the Linux WebKitGTK webview.
//!
//! WKWebView and WebView2 check spelling through the operating system without
//! any setup. WebKitGTK ships with spell checking disabled and checks nothing
//! until the embedder enables it and names languages, so the Linux adapter does
//! both once the main window exists. Words are looked up through Enchant, which
//! needs an installed Hunspell (or other provider) dictionary per language.
//!
//! Diagnostics record only counts and booleans, never language names or text.

// The language policy is only called from the Linux adapter and its tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use tauri::AppHandle;

/// Always checked in addition to the system languages: this is the product's
/// default catalog locale, and CJK locales have no Hunspell dictionary.
const FALLBACK_SPELL_CHECKING_LANGUAGE: &str = "en_US";

/// Turns the ordered locale names from `g_get_language_names` (for example
/// `en_US.UTF-8`, `en_US`, `en.UTF-8`, `en`, `C`) into the `ll` / `ll_CC` tags
/// Enchant resolves to dictionaries, keeping the user's preference order and
/// ending with the fallback language when no English variant is present.
pub(crate) fn spell_checking_languages<'a>(
    locale_names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    for name in locale_names {
        if is_dictionary_language_tag(name) && !languages.iter().any(|known| known == name) {
            languages.push(name.to_owned());
        }
    }
    if !languages
        .iter()
        .any(|language| language == "en" || language.starts_with("en_"))
    {
        languages.push(FALLBACK_SPELL_CHECKING_LANGUAGE.to_owned());
    }
    languages
}

/// `ll`, `lll`, `ll_CC`, or `lll_CC`. Codeset and modifier variants are
/// skipped because the language-names list already carries their bare forms.
fn is_dictionary_language_tag(name: &str) -> bool {
    let (language, region) = match name.split_once('_') {
        Some((language, region)) => (language, Some(region)),
        None => (name, None),
    };
    (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_lowercase())
        && region.is_none_or(|region| {
            region.len() == 2 && region.bytes().all(|byte| byte.is_ascii_uppercase())
        })
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpellCheckingReport {
    pub(crate) enabled: bool,
    pub(crate) requested_languages: usize,
    /// Languages WebKit found a dictionary for; zero means nothing is checked.
    pub(crate) loaded_languages: usize,
}

#[cfg(target_os = "linux")]
pub(crate) fn configure_spell_checking(
    context: &webkit2gtk::WebContext,
    languages: &[String],
) -> SpellCheckingReport {
    use webkit2gtk::WebContextExt;

    let languages: Vec<&str> = languages.iter().map(String::as_str).collect();
    context.set_spell_checking_languages(&languages);
    context.set_spell_checking_enabled(true);
    SpellCheckingReport {
        enabled: context.is_spell_checking_enabled(),
        requested_languages: languages.len(),
        loaded_languages: context.spell_checking_languages().len(),
    }
}

/// Best-effort: a missing window or webview leaves the composer usable without
/// spell checking, and the diagnostic records which case happened.
#[cfg(target_os = "linux")]
pub(crate) fn enable_for_main_window(app: &AppHandle) {
    use tauri::Manager;
    use webkit2gtk::WebViewExt;

    let Some(window) = app.get_webview_window("main") else {
        record_diagnostic(None);
        return;
    };
    let languages = spell_checking_languages(
        gtk::glib::language_names()
            .iter()
            .map(gtk::glib::GString::as_str),
    );
    let applied = window.with_webview(move |platform_webview| {
        let report = platform_webview
            .inner()
            .context()
            .map(|context| configure_spell_checking(&context, &languages));
        record_diagnostic(report);
    });
    if applied.is_err() {
        record_diagnostic(None);
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn enable_for_main_window(_app: &AppHandle) {}

#[cfg(target_os = "linux")]
fn record_diagnostic(report: Option<SpellCheckingReport>) {
    use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};

    let event = match report {
        Some(report) => DiagnosticEvent::new(
            if report.enabled && report.loaded_languages > 0 {
                DiagnosticLevel::Info
            } else {
                DiagnosticLevel::Warn
            },
            "desktop.spell_checking",
            "configured",
        )
        .field(DiagnosticField::boolean("enabled", report.enabled))
        .field(DiagnosticField::count(
            "requested_languages",
            report.requested_languages as u64,
        ))
        .field(DiagnosticField::count(
            "loaded_languages",
            report.loaded_languages as u64,
        )),
        None => DiagnosticEvent::new(
            DiagnosticLevel::Warn,
            "desktop.spell_checking",
            "webview_unavailable",
        ),
    };
    koushi_diagnostics::record(event);
}

#[cfg(test)]
mod tests {
    use super::spell_checking_languages;

    #[test]
    fn system_languages_keep_order_and_drop_codesets_and_the_c_locale() {
        let names = ["de_DE.UTF-8", "de_DE", "de.UTF-8", "de", "C"];

        assert_eq!(spell_checking_languages(names), ["de_DE", "de", "en_US"]);
    }

    #[test]
    fn an_english_system_language_is_not_followed_by_the_fallback() {
        let names = ["en_GB.UTF-8", "en_GB", "en.UTF-8", "en", "C"];

        assert_eq!(spell_checking_languages(names), ["en_GB", "en"]);
    }

    #[test]
    fn modifiers_duplicates_and_unrecognized_names_are_skipped() {
        let names = [
            "sr_RS@latin",
            "sr_RS",
            "sr",
            "sr",
            "POSIX",
            "EN",
            "e",
            "ja_JP",
        ];

        assert_eq!(
            spell_checking_languages(names),
            ["sr_RS", "sr", "ja_JP", "en_US"]
        );
    }

    #[test]
    fn the_c_locale_alone_still_checks_english() {
        assert_eq!(spell_checking_languages(["C"]), ["en_US"]);
    }

    /// WebKitGTK starts with spell checking off; the adapter must turn it on.
    /// Needs a display for GTK, so a headless runner without one skips it.
    #[cfg(target_os = "linux")]
    #[test]
    fn configuring_a_webkit_context_enables_spell_checking() {
        use webkit2gtk::WebContextExt;

        if gtk::init().is_err() {
            eprintln!("skipped: GTK needs a display");
            return;
        }
        let context = webkit2gtk::WebContext::new();
        assert!(!context.is_spell_checking_enabled());

        let report = super::configure_spell_checking(&context, &["en_US".to_owned()]);

        assert!(context.is_spell_checking_enabled());
        assert!(report.enabled);
        assert_eq!(report.requested_languages, 1);
    }
}
