use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;

use koushi_sdk::MatrixClientSession;
use koushi_state::AvatarThumbnailState;
use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::MxcUri;
use matrix_sdk::ruma::events::room::MediaSource as SdkMediaSource;
use matrix_sdk::ruma::html::Html;
use regex::Regex;
use url::Url;

use crate::renderable_thumbnail::{RenderableThumbnailKind, store_renderable_thumbnail};
use koushi_protocol::event::{LinkPreview, LinkPreviewImage, LinkPreviewState, TimelineLinkRange};
use koushi_protocol::event::{TimelineFormattedBody, TimelineMediaSource};

pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewImageDownloadError {
    Network,
    TooLarge,
}

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

/// Block-level Matrix HTML elements at whose boundaries a line separator breaks
/// a URL candidate. Inline runs (`strong`, `em`, `code`, `a`, ...) stay
/// contiguous so URLs split across inline formatting are still detected.
fn is_block_element(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "pre"
            | "br"
            | "hr"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "th"
            | "td"
            | "caption"
            | "details"
            | "summary"
    )
}

/// Text of `html` with a `\n` between block-level elements and `<br>`, keeping
/// inline-concatenated runs intact. Used only for URL boundary detection; the
/// stored formatted plain text and its offsets are left untouched.
fn plain_text_with_block_separators(html: &Html) -> String {
    fn collect(nodes: impl Iterator<Item = matrix_sdk::ruma::html::NodeRef>, out: &mut String) {
        for node in nodes {
            if let Some(text) = node.as_text() {
                out.push_str(&text.borrow());
                continue;
            }
            let block = node
                .as_element()
                .is_some_and(|element| is_block_element(element.name.local.as_ref()));
            if block && !out.is_empty() {
                out.push('\n');
            }
            collect(node.children(), out);
            if block && !out.is_empty() {
                out.push('\n');
            }
        }
    }

    let mut text = String::new();
    collect(html.children(), &mut text);
    text
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
        // Scan the sanitized formatted HTML with paragraph and <br> boundaries
        // preserved instead of the concatenated plain text, so a URL followed by
        // a new paragraph cannot swallow the next word into a phantom URL (#870).
        collect(&plain_text_with_block_separators(&Html::parse(
            &formatted.html,
        )));
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
) -> Result<AvatarThumbnailState, PreviewImageDownloadError> {
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
        .map_err(|_| PreviewImageDownloadError::Network)?;

    store_renderable_thumbnail(RenderableThumbnailKind::LinkPreview, url, bytes)
        .map_err(|_| PreviewImageDownloadError::TooLarge)
}

#[cfg(test)]
mod tests;
