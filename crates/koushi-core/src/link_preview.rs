use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;

use koushi_sdk::MatrixClientSession;
use koushi_state::AvatarThumbnailState;
use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::MxcUri;
use matrix_sdk::ruma::events::room::MediaSource as SdkMediaSource;
use regex::Regex;
use url::Url;

use crate::event::{LinkPreview, LinkPreviewImage, LinkPreviewState, TimelineLinkRange};
use crate::event::{TimelineFormattedBody, TimelineMediaSource};
use crate::renderable_thumbnail::{RenderableThumbnailKind, store_renderable_thumbnail};

pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r##"https?://[^\s<>"{}|\\^`\[\]]+"##).expect("valid url regex"))
}

fn href_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"href=["'](https?://[^"']+)["']"#).expect("valid href regex"))
}

fn is_link_preview_candidate(url: &str) -> bool {
    let is_matrix_permalink = Url::parse(url).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && parsed
                .host_str()
                .is_some_and(|host| host.eq_ignore_ascii_case("matrix.to"))
            && parsed.port().is_none()
    });
    !is_matrix_permalink
}

#[derive(Clone, Eq, PartialEq)]
pub struct LinkPreviewContext {
    pub unencrypted_global_enabled: bool,
    pub encrypted_global_enabled: bool,
    pub room_enabled: Option<bool>,
    pub hidden_event_ids: BTreeSet<String>,
    pub cache: HashMap<String, LinkPreview>,
    pub room_overrides: BTreeMap<String, bool>,
}

impl Default for LinkPreviewContext {
    fn default() -> Self {
        Self {
            unencrypted_global_enabled: true,
            encrypted_global_enabled: true,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
        }
    }
}

impl LinkPreviewContext {
    /// Build a context from persisted application settings. Per-room overrides
    /// are runtime state and are supplied by policy broadcasts.
    pub fn from_settings(values: &koushi_state::SettingsValues) -> Self {
        Self {
            unencrypted_global_enabled: values.display.url_previews_enabled,
            encrypted_global_enabled: values.display.encrypted_url_previews_enabled,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
        }
    }

    /// Produce a room-scoped view of this context, resolving the room override
    /// into `room_enabled`.
    pub fn for_room(&self, room_id: &str) -> Self {
        Self {
            unencrypted_global_enabled: self.unencrypted_global_enabled,
            encrypted_global_enabled: self.encrypted_global_enabled,
            room_enabled: self.room_overrides.get(room_id).copied(),
            hidden_event_ids: self.hidden_event_ids.clone(),
            cache: self.cache.clone(),
            room_overrides: self.room_overrides.clone(),
        }
    }

    /// Update only the policy fields that can change from a settings broadcast,
    /// preserving cached previews and the hidden-event set.
    pub fn apply_policy_delta(
        &mut self,
        unencrypted_global_enabled: bool,
        encrypted_global_enabled: bool,
        room_enabled: Option<bool>,
    ) {
        self.unencrypted_global_enabled = unencrypted_global_enabled;
        self.encrypted_global_enabled = encrypted_global_enabled;
        self.room_enabled = room_enabled;
    }
}

impl fmt::Debug for LinkPreviewContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinkPreviewContext")
            .field(
                "unencrypted_global_enabled",
                &self.unencrypted_global_enabled,
            )
            .field("encrypted_global_enabled", &self.encrypted_global_enabled)
            .field("room_enabled", &self.room_enabled)
            .field("room_override_count", &self.room_overrides.len())
            .field("hidden_event_ids_count", &self.hidden_event_ids.len())
            .field("cache_entry_count", &self.cache.len())
            .finish()
    }
}

/// Punctuation that terminates a URL from within the match. This prevents CJK
/// sentence punctuation (`、` `。` `，` etc.) from being swallowed into the URL
/// while keeping ASCII URL path/query punctuation such as `?`, `&`, `=`, and
/// balanced parentheses.
fn is_url_stop_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{3001}'..='\u{3003}' // 、 。 〃
        | '\u{3008}'..='\u{3011}' // 〈 《  〉 》
        | '\u{3014}'..='\u{301F}' // 〔 etc
        | '\u{FF08}'..='\u{FF09}' // （ ）
        | '\u{FF0C}' | '\u{FF0E}' | '\u{FF1A}' | '\u{FF1B}' | '\u{FF1F}' | '\u{FF01}' // 全角标点
        | '\u{2018}'..='\u{201F}' // smart quotes
        | '\u{2026}' // …
    )
}

/// Characters that may be trimmed from the end of a URL match. Closing
/// brackets are trimmed only when they are not balancing an opener already
/// present in the URL, so `https://example.com/foo(bar)` stays balanced.
fn is_trailing_url_punctuation(c: char) -> bool {
    const ASCII_TRAILING: &str = ". , ; : ! ? \" ' \u{00a0}";
    if ASCII_TRAILING.contains(c) {
        return true;
    }
    if matches!(c, ')' | ']' | '}' | '>') {
        return true;
    }
    // CJK / full-width punctuation blocks that commonly wrap a sentence.
    matches!(
        c,
        '\u{3001}'..='\u{3003}' // 、 。 〃
        | '\u{3008}'..='\u{3011}' // 〈 《  〉 》
        | '\u{3014}'..='\u{301F}' // 〔 etc
        | '\u{FF08}'..='\u{FF09}' // （ ）
        | '\u{FF0C}' | '\u{FF0E}' | '\u{FF1A}' | '\u{FF1B}' | '\u{FF1F}' | '\u{FF01}' // 全角标点
        | '\u{2018}'..='\u{201F}' // smart quotes
        | '\u{2026}' // …
    )
}

fn matching_open_bracket(c: char) -> Option<char> {
    match c {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        '>' => Some('<'),
        _ => None,
    }
}

fn trim_trailing_url_punctuation(url: &str) -> &str {
    let mut end = url.len();
    while end > 0 {
        let c = url[..end].chars().next_back().unwrap();
        if !is_trailing_url_punctuation(c) {
            break;
        }
        // Keep a closing bracket if it balances an opener in the remaining URL.
        if let Some(open) = matching_open_bracket(c) {
            let prefix = &url[..end - c.len_utf8()];
            let opens = prefix.chars().filter(|&x| x == open).count();
            let closes = prefix.chars().filter(|&x| x == c).count();
            if opens > closes {
                break;
            }
        }
        end -= c.len_utf8();
    }
    &url[..end]
}

fn truncate_at_stop_punctuation(url: &str) -> &str {
    match url
        .char_indices()
        .find(|(_, c)| is_url_stop_punctuation(*c))
    {
        Some((index, _)) => &url[..index],
        None => url,
    }
}

/// Extract clickable link ranges from plain text using the same Unicode-aware
/// URL policy as link previews. Ranges are expressed in UTF-16 code units so
/// they align with JavaScript string indices in the React renderer.
///
/// Each occurrence produces its own range so anchors can be rendered for every
/// URL in the message body. Callers that only need unique preview URLs (such as
/// link-preview fetching) should use [`extract_urls`] instead.
pub fn extract_link_ranges(text: &str) -> Vec<TimelineLinkRange> {
    let url_re = url_regex();
    let mut ranges = Vec::new();

    for mat in url_re.find_iter(text) {
        let raw = mat.as_str();
        let stopped = truncate_at_stop_punctuation(raw);
        let trimmed = trim_trailing_url_punctuation(stopped);
        if trimmed.is_empty() {
            continue;
        }

        let start_utf16 = text[..mat.start()].encode_utf16().count();
        let raw_end_utf16 = start_utf16 + raw.encode_utf16().count();
        let trailing_utf16 = raw[trimmed.len()..].encode_utf16().count();
        let end_utf16 = raw_end_utf16 - trailing_utf16;

        ranges.push(TimelineLinkRange {
            url: trimmed.to_owned(),
            start_utf16,
            end_utf16,
        });
    }

    ranges
}

pub fn extract_urls(body: Option<&str>, formatted: Option<&TimelineFormattedBody>) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    let mut collect = |text: &str| {
        for range in extract_link_ranges(text) {
            if is_link_preview_candidate(&range.url) && seen.insert(range.url.clone()) {
                urls.push(range.url);
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
                if is_link_preview_candidate(url) && seen.insert(url.to_owned()) {
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
        context
            .room_enabled
            .unwrap_or(context.encrypted_global_enabled)
    } else {
        context
            .room_enabled
            .unwrap_or(context.unencrypted_global_enabled)
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

#[allow(dead_code)]
pub fn effective_room_url_previews_enabled(
    room_id: &str,
    is_encrypted: bool,
    unencrypted_global_enabled: bool,
    encrypted_global_enabled: bool,
    room_overrides: &BTreeMap<String, bool>,
) -> bool {
    if is_encrypted {
        room_overrides
            .get(room_id)
            .copied()
            .unwrap_or(encrypted_global_enabled)
    } else {
        room_overrides
            .get(room_id)
            .copied()
            .unwrap_or(unencrypted_global_enabled)
    }
}

#[allow(dead_code)]
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

/// Fetch link preview metadata for `url` from the homeserver's URL preview
/// endpoint. Image thumbnails are stored only in the in-memory renderable
/// thumbnail cache.
pub async fn fetch_link_preview(
    session: &MatrixClientSession,
    url: &str,
) -> Result<LinkPreview, ()> {
    let client = session.client();
    let mut preview_url = client.homeserver();
    preview_url.set_path("/_matrix/media/v3/preview_url");
    preview_url.set_query(None);
    preview_url.query_pairs_mut().append_pair("url", url);

    let mut request = client.http_client().get(preview_url);
    if let Some(token) = client.access_token() {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request.send().await.map_err(|_| ())?;
    let bytes = response.bytes().await.map_err(|_| ())?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| ())?;

    let title = json
        .get("og:title")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let description = json
        .get("og:description")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let image_width = json.get("og:image:width").and_then(|v| v.as_u64());
    let image_height = json.get("og:image:height").and_then(|v| v.as_u64());

    let mut image = None;
    if let Some(image_url) = json.get("og:image").and_then(|v| v.as_str()) {
        let mxc = <&MxcUri>::from(image_url);
        if mxc.is_valid() {
            let uri = mxc.to_owned();
            let thumbnail = download_preview_image(session, &uri, url).await.ok();
            if let Some(thumbnail) = thumbnail {
                image = Some(LinkPreviewImage {
                    source: TimelineMediaSource {
                        mxc_uri: uri.to_string(),
                        encrypted: false,
                        encryption_version: None,
                    },
                    width: image_width,
                    height: image_height,
                    thumbnail,
                });
            }
        }
    }

    Ok(LinkPreview {
        url: url.to_owned(),
        title,
        description,
        image,
        state: LinkPreviewState::Ready,
    })
}

async fn download_preview_image(
    session: &MatrixClientSession,
    uri: &matrix_sdk::ruma::OwnedMxcUri,
    url: &str,
) -> Result<AvatarThumbnailState, ()> {
    let client = session.client();
    let bytes = client
        .media()
        .get_media_content(
            &MediaRequestParameters {
                source: SdkMediaSource::Plain(uri.clone()),
                format: MediaFormat::File,
            },
            false,
        )
        .await
        .map_err(|_| ())?;

    Ok(store_renderable_thumbnail(
        RenderableThumbnailKind::LinkPreview,
        url,
        bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn link_preview_url_policy_keeps_unicode_path_query_and_balanced_parentheses() {
        // Unicode path and query are preserved.
        let body = "Read https://tensor4all.org/blog/パス?q=日本語 for details.";
        let urls = extract_urls(Some(body), None);
        assert_eq!(urls, vec!["https://tensor4all.org/blog/パス?q=日本語"]);

        // Balanced parentheses are kept.
        let body2 = "See https://example.com/foo(bar).";
        assert_eq!(
            extract_urls(Some(body2), None),
            vec!["https://example.com/foo(bar)"]
        );

        // CJK punctuation stops the URL, not trims it.
        let body3 = "Next https://example.com/a、次の文";
        assert_eq!(
            extract_urls(Some(body3), None),
            vec!["https://example.com/a"]
        );
    }

    #[test]
    fn extract_link_ranges_use_utf16_offsets_and_strip_trailing_punctuation() {
        let body = "See https://example.com/path.";
        let ranges = extract_link_ranges(body);
        assert_eq!(ranges.len(), 1);
        let range = &ranges[0];
        assert_eq!(range.url, "https://example.com/path");
        // "See " is 4 UTF-16 code units; the URL starts at offset 4.
        assert_eq!(range.start_utf16, 4);
        // Trailing period is stripped, so the end is the length of the URL text after it.
        assert_eq!(
            range.end_utf16,
            4 + "https://example.com/path".encode_utf16().count()
        );
    }

    #[test]
    fn extract_link_ranges_supports_idn_and_cjk_punctuation() {
        // IDN domain and path, followed by a full-width period.
        let body = "https://例え.jp/テスト。";
        let ranges = extract_link_ranges(body);
        assert_eq!(ranges.len(), 1);
        let range = &ranges[0];
        assert_eq!(range.url, "https://例え.jp/テスト");
        assert_eq!(range.start_utf16, 0);
        assert_eq!(
            range.end_utf16,
            "https://例え.jp/テスト".encode_utf16().count()
        );
    }

    #[test]
    fn extract_link_ranges_keeps_repeated_url_occurrences_distinct() {
        let body = "https://a.test https://a.test https://b.test";
        let ranges = extract_link_ranges(body);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0].url, "https://a.test");
        assert_eq!(ranges[1].url, "https://a.test");
        assert_eq!(ranges[2].url, "https://b.test");
        assert!(ranges[0].end_utf16 <= ranges[1].start_utf16);
        assert!(ranges[1].end_utf16 <= ranges[2].start_utf16);
    }

    #[test]
    fn default_context_enables_encrypted_room_previews() {
        let context = LinkPreviewContext::default();
        assert!(context.unencrypted_global_enabled);
        assert!(context.encrypted_global_enabled);
    }

    #[test]
    fn encrypted_room_default_off() {
        let context = LinkPreviewContext {
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
        };
        let previews =
            link_previews_for_message(Some("https://example.com"), None, "$event", true, &context);
        assert_eq!(previews, None);
    }

    #[test]
    fn encrypted_room_global_setting_can_enable() {
        let context = LinkPreviewContext {
            unencrypted_global_enabled: true,
            encrypted_global_enabled: true,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
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
    fn encrypted_room_explicit_override_enables() {
        let context = LinkPreviewContext {
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: Some(true),
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
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
            unencrypted_global_enabled: true,
            encrypted_global_enabled: true,
            room_enabled: Some(false),
            hidden_event_ids: BTreeSet::new(),
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
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
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: None,
            hidden_event_ids: hidden,
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
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
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: None,
            hidden_event_ids: hidden,
            cache: HashMap::new(),
            room_overrides: BTreeMap::new(),
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
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: None,
            hidden_event_ids: BTreeSet::new(),
            cache,
            room_overrides: BTreeMap::new(),
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

        // Encrypted rooms follow the encrypted-room global default.
        assert!(!effective_room_url_previews_enabled(
            "!other:example.com",
            true,
            true,
            false,
            &overrides
        ));
        assert!(effective_room_url_previews_enabled(
            "!other:example.com",
            true,
            false,
            true,
            &overrides
        ));
        // Encrypted explicit override enables.
        assert!(effective_room_url_previews_enabled(
            "!room:example.com",
            true,
            false,
            false,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!disabled:example.com",
            true,
            true,
            true,
            &overrides
        ));

        // Non-encrypted follows global when no override.
        assert!(effective_room_url_previews_enabled(
            "!other:example.com",
            false,
            true,
            false,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!other:example.com",
            false,
            false,
            true,
            &overrides
        ));
        // Non-encrypted explicit overrides.
        assert!(effective_room_url_previews_enabled(
            "!room:example.com",
            false,
            false,
            false,
            &overrides
        ));
        assert!(!effective_room_url_previews_enabled(
            "!disabled:example.com",
            false,
            true,
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
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: Some(false),
            hidden_event_ids: hidden,
            cache,
            room_overrides: BTreeMap::new(),
        };
        let debug = format!("{:?}", context);
        assert!(debug.contains("unencrypted_global_enabled"));
        assert!(debug.contains("encrypted_global_enabled"));
        assert!(debug.contains("room_enabled"));
        assert!(debug.contains("room_override_count"));
        assert!(debug.contains("hidden_event_ids_count"));
        assert!(debug.contains("cache_entry_count"));
        assert!(!debug.contains("https://example.com"));
        assert!(!debug.contains("$event"));
    }

    #[test]
    fn apply_policy_delta_preserves_hidden_event_ids_and_cache() {
        let mut hidden = BTreeSet::new();
        hidden.insert("$event".to_owned());

        let ready = LinkPreview {
            url: "https://example.com".to_owned(),
            title: Some("Example".to_owned()),
            description: Some("Description".to_owned()),
            image: None,
            state: LinkPreviewState::Ready,
        };
        let mut cache = HashMap::new();
        cache.insert(ready.url.clone(), ready.clone());

        let mut context = LinkPreviewContext {
            unencrypted_global_enabled: true,
            encrypted_global_enabled: false,
            room_enabled: None,
            hidden_event_ids: hidden.clone(),
            cache: cache.clone(),
            room_overrides: BTreeMap::new(),
        };

        context.apply_policy_delta(false, true, Some(true));

        assert!(!context.unencrypted_global_enabled);
        assert!(context.encrypted_global_enabled);
        assert_eq!(context.room_enabled, Some(true));
        assert_eq!(context.hidden_event_ids, hidden);
        assert_eq!(context.cache, cache);
    }
}
