use std::collections::{BTreeMap, BTreeSet, HashMap};

use regex::Regex;

use matrix_desktop_state::AvatarThumbnailState;

use crate::event::{LinkPreview, LinkPreviewImage, LinkPreviewState};
use crate::event::{TimelineFormattedBody, TimelineMediaSource};

pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LinkPreviewContext {
    pub global_enabled: bool,
    pub room_enabled: Option<bool>,
    pub hidden_event_ids: BTreeSet<String>,
    pub cache: HashMap<String, LinkPreview>,
}

pub fn extract_urls(body: Option<&str>, formatted: Option<&TimelineFormattedBody>) -> Vec<String> {
    let mut urls = Vec::new();
    let url_re = Regex::new(r##"https?://[^\s<>"{}|\\^`\[\]]+"##).expect("valid url regex");

    let mut collect = |text: &str| {
        for mat in url_re.find_iter(text) {
            let url = mat
                .as_str()
                .trim_end_matches(|c| r##".,;:!?)"'>"##.contains(c));
            if !urls.contains(&url.to_owned()) {
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
        let href_re = Regex::new(r#"href=["'](https?://[^"']+)["']"#).expect("valid href regex");
        for cap in href_re.captures_iter(&formatted.html) {
            if let Some(url) = cap.get(1) {
                let url = url.as_str();
                if !urls.contains(&url.to_owned()) {
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
}
