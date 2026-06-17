use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;

use regex::Regex;

use matrix_desktop_state::AvatarThumbnailState;

use crate::event::{LinkPreview, LinkPreviewImage, LinkPreviewState};
use crate::event::{TimelineFormattedBody, TimelineMediaSource};

pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r##"https?://[^\s<>"{}|\\^`\[\]]+"##).expect("valid url regex"))
}

fn href_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"href=["'](https?://[^"']+)["']"#).expect("valid href regex"))
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct LinkPreviewContext {
    pub global_enabled: bool,
    pub room_enabled: Option<bool>,
    pub hidden_event_ids: BTreeSet<String>,
    pub cache: HashMap<String, LinkPreview>,
}

impl fmt::Debug for LinkPreviewContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinkPreviewContext")
            .field("global_enabled", &self.global_enabled)
            .field("room_enabled", &self.room_enabled)
            .field("hidden_event_ids_count", &self.hidden_event_ids.len())
            .field("cache_entry_count", &self.cache.len())
            .finish()
    }
}

pub fn extract_urls(body: Option<&str>, formatted: Option<&TimelineFormattedBody>) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();
    let url_re = url_regex();

    let mut collect = |text: &str| {
        for mat in url_re.find_iter(text) {
            let url = mat
                .as_str()
                .trim_end_matches(|c| r##".,;:!?)"'>"##.contains(c));
            if seen.insert(url.to_owned()) {
                urls.push(url.to_owned());
            }
        }
    };

    if let Some(body) = body {
        collect(body);
    }
    if let Some(formatted) = formatted {
        collect(&formatted.plain_text);
        // Extract hrefs from sanitized HTML without parsing the full DOM.
        let href_re = href_regex();
        for cap in href_re.captures_iter(&formatted.html) {
            if let Some(url) = cap.get(1) {
                let url = url.as_str();
                if seen.insert(url.to_owned()) {
                    urls.push(url.to_owned());
                }
            }
        }
    }

    urls.into_iter()
        .take(MAX_LINK_PREVIEWS_PER_MESSAGE)
        .collect()
}

pub fn link_previews_for_message(
    body: Option<&str>,
    formatted: Option<&TimelineFormattedBody>,
    event_id: &str,
    is_encrypted: bool,
    context: &LinkPreviewContext,
) -> Option<Vec<LinkPreview>> {
    if context.hidden_event_ids.contains(event_id) {
        return Some(Vec::new());
    }

    let effective_enabled = if is_encrypted {
        context.room_enabled.unwrap_or(false)
    } else {
        context.room_enabled.unwrap_or(context.global_enabled)
    };

    if !effective_enabled {
        return None;
    }

    let urls = extract_urls(body, formatted);
    if urls.is_empty() {
        return None;
    }

    Some(
        urls.into_iter()
            .map(|url| {
                context
                    .cache
                    .get(&url)
                    .cloned()
                    .unwrap_or_else(|| LinkPreview {
                        url,
                        title: None,
                        description: None,
                        image: None,
                        state: LinkPreviewState::Pending,
                    })
            })
            .collect(),
    )
}

pub fn effective_room_url_previews_enabled(
    room_id: &str,
    is_encrypted: bool,
    global_enabled: bool,
    room_overrides: &BTreeMap<String, bool>,
) -> bool {
    if is_encrypted {
        room_overrides.get(room_id).copied().unwrap_or(false)
    } else {
        room_overrides
            .get(room_id)
            .copied()
            .unwrap_or(global_enabled)
    }
}

pub fn link_preview_image_from_mxc(mxc_uri: String) -> LinkPreviewImage {
    LinkPreviewImage {
        source: TimelineMediaSource {
            mxc_uri,
            encrypted: false,
            encryption_version: None,
        },
        width: None,
        height: None,
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt_body(text: &str) -> TimelineFormattedBody {
        TimelineFormattedBody {
            html: text.to_owned(),
            plain_text: text.to_owned(),
            code_blocks: Vec::new(),
        }
    }

    fn fmt_body_with_html(html: &str, plain: &str) -> TimelineFormattedBody {
        TimelineFormattedBody {
            html: html.to_owned(),
            plain_text: plain.to_owned(),
            code_blocks: Vec::new(),
        }
    }

    #[test]
    fn extract_urls_from_plain_text() {
        let body = "Check out https://example.com and http://test.org/page.";
        let urls = extract_urls(Some(body), None);
        assert_eq!(urls, vec!["https://example.com", "http://test.org/page"]);
    }

    #[test]
    fn deduplication_and_cap() {
        let body =
            "a https://x.com b https://x.com c https://y.com d https://z.com e https://w.com";
        let urls = extract_urls(Some(body), None);
        assert_eq!(
            urls,
            vec!["https://x.com", "https://y.com", "https://z.com"]
        );
    }

    #[test]
    fn extract_hrefs_from_formatted_html() {
        let formatted = fmt_body_with_html(
            r##"<p>See <a href="https://matrix.org">matrix</a> and <a href='https://rust-lang.org'>rust</a>.</p>"##,
            "See matrix and rust.",
        );
        let urls = extract_urls(None, Some(&formatted));
        assert_eq!(urls, vec!["https://matrix.org", "https://rust-lang.org"]);
    }

    #[test]
    fn trailing_punctuation_is_stripped() {
        let body = "Visit https://example.com.,;:!?)\"'> today.";
        let urls = extract_urls(Some(body), None);
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn encrypted_room_default_off() {
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
        };
        let previews =
            link_previews_for_message(Some("https://example.com"), None, "$event", true, &context);
        assert_eq!(previews, None);
    }

    #[test]
    fn encrypted_room_explicit_override_enables() {
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: Some(true),
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
        };
        let previews =
            link_previews_for_message(Some("https://example.com"), None, "$event", true, &context);
        assert_eq!(
            previews,
            Some(vec![LinkPreview {
                url: "https://example.com".to_owned(),
                title: None,
                description: None,
                image: None,
                state: LinkPreviewState::Pending,
            }])
        );
    }

    #[test]
    fn encrypted_room_explicit_disable_overrides_global() {
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: Some(false),
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
        };
        let previews =
            link_previews_for_message(Some("https://example.com"), None, "$event", true, &context);
        assert_eq!(previews, None);
    }

    #[test]
    fn hidden_event_returns_empty_previews() {
        let mut hidden = BTreeSet::new();
        hidden.insert("$event".to_owned());
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: None,
            hidden_event_ids: hidden,
            cache: HashMap::new(),
        };
        let previews =
            link_previews_for_message(Some("https://example.com"), None, "$event", false, &context);
        assert_eq!(previews, Some(Vec::new()));
    }

    #[test]
    fn multiple_hidden_event_ids() {
        let mut hidden = BTreeSet::new();
        hidden.insert("$alpha".to_owned());
        hidden.insert("$beta".to_owned());
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: None,
            hidden_event_ids: hidden,
            cache: HashMap::new(),
        };
        assert_eq!(
            link_previews_for_message(Some("https://example.com"), None, "$alpha", false, &context),
            Some(Vec::new())
        );
        assert_eq!(
            link_previews_for_message(Some("https://example.com"), None, "$beta", false, &context),
            Some(Vec::new())
        );
        assert!(
            link_previews_for_message(Some("https://example.com"), None, "$gamma", false, &context)
                .is_some()
        );
    }

    #[test]
    fn cache_reuse_returns_ready_preview() {
        let ready = LinkPreview {
            url: "https://example.com".to_owned(),
            title: Some("Example".to_owned()),
            description: Some("Description".to_owned()),
            image: None,
            state: LinkPreviewState::Ready,
        };
        let mut cache = HashMap::new();
        cache.insert("https://example.com".to_owned(), ready.clone());
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache,
        };
        let previews =
            link_previews_for_message(Some("https://example.com"), None, "$event", false, &context);
        assert_eq!(previews, Some(vec![ready]));
    }

    #[test]
    fn effective_room_url_previews_enabled_combinations() {
        let mut overrides = BTreeMap::new();
        overrides.insert("!room:example.com".to_owned(), true);
        overrides.insert("!disabled:example.com".to_owned(), false);

        // Encrypted rooms default to false regardless of global setting.
        assert!(!effective_room_url_previews_enabled(
            "!other:example.com",
            true,
            true,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!other:example.com",
            true,
            false,
            &overrides
        ));
        // Encrypted explicit override enables.
        assert!(effective_room_url_previews_enabled(
            "!room:example.com",
            true,
            false,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!disabled:example.com",
            true,
            true,
            &overrides
        ));

        // Non-encrypted follows global when no override.
        assert!(effective_room_url_previews_enabled(
            "!other:example.com",
            false,
            true,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!other:example.com",
            false,
            false,
            &overrides
        ));
        // Non-encrypted explicit overrides.
        assert!(effective_room_url_previews_enabled(
            "!room:example.com",
            false,
            false,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!disabled:example.com",
            false,
            true,
            &overrides
        ));
    }

    #[test]
    fn link_preview_image_from_mxc_structure() {
        let image = link_preview_image_from_mxc("mxc://example/image".to_owned());
        assert_eq!(image.source.mxc_uri, "mxc://example/image");
        assert!(!image.source.encrypted);
        assert_eq!(image.source.encryption_version, None);
        assert_eq!(image.width, None);
        assert_eq!(image.height, None);
        assert_eq!(image.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn link_preview_context_debug_hides_private_data() {
        let mut hidden = BTreeSet::new();
        hidden.insert("$event".to_owned());
        let mut cache = HashMap::new();
        cache.insert(
            "https://example.com".to_owned(),
            LinkPreview {
                url: "https://example.com".to_owned(),
                title: Some("title".to_owned()),
                description: Some("desc".to_owned()),
                image: None,
                state: LinkPreviewState::Ready,
            },
        );
        let context = LinkPreviewContext {
            global_enabled: true,
            room_enabled: Some(false),
            hidden_event_ids: hidden,
            cache,
        };
        let debug = format!("{:?}", context);
        assert!(debug.contains("global_enabled"));
        assert!(debug.contains("room_enabled"));
        assert!(debug.contains("hidden_event_ids_count"));
        assert!(debug.contains("cache_entry_count"));
        assert!(!debug.contains("https://example.com"));
        assert!(!debug.contains("$event"));
    }
}
