//! Block-boundary regressions for clickable link ranges (#1596).
//!
//! The published `formatted.plain_text` concatenates block elements without a
//! separator, so scanning that text directly lets a bare URL swallow the next
//! item's label. These tests drive the real sanitizer and projection and assert
//! every offset against the published plain-text basis the renderer indexes.

use koushi_protocol::event::TimelineLinkRange;
use matrix_sdk::ruma::events::room::message::{MessageType, TextMessageEventContent};

use super::{link_ranges_for_message_projection, message_projection_from_msgtype};

fn projected_ranges(html: &str) -> (String, Vec<TimelineLinkRange>) {
    let msgtype = MessageType::Text(TextMessageEventContent::html("fallback body", html));
    let projection = message_projection_from_msgtype(&msgtype, "fallback body");
    let formatted = projection
        .formatted
        .expect("formatted body should project to a render model");
    let ranges = link_ranges_for_message_projection(projection.body.as_deref(), Some(&formatted));
    (formatted.plain_text, ranges)
}

fn expected_range(plain_text: &str, url: &str) -> (usize, usize) {
    let start = plain_text
        .find(url)
        .unwrap_or_else(|| panic!("{url:?} should be part of the plain text: {plain_text:?}"));
    let start_utf16 = plain_text[..start].encode_utf16().count();
    (start_utf16, start_utf16 + url.encode_utf16().count())
}

fn assert_urls(ranges: &[TimelineLinkRange], expected: &[&str], plain_text: &str) {
    let actual: Vec<&str> = ranges.iter().map(|range| range.url.as_str()).collect();
    assert_eq!(actual, expected, "plain_text={plain_text:?}");
    for range in ranges {
        assert_eq!(
            (range.start_utf16, range.end_utf16),
            expected_range(plain_text, &range.url),
            "the range for {:?} must address the published plain text",
            range.url
        );
    }
}

#[test]
fn adjacent_list_items_keep_each_url_inside_its_own_item() {
    let html = r#"<ul><li>Member 1: https://example.invalid/profiles/member_one?locale=ja</li><li>Member 2: https://example.invalid/profiles/member_two?locale=ja</li></ul>"#;
    let (plain_text, ranges) = projected_ranges(html);

    assert_urls(
        &ranges,
        &[
            "https://example.invalid/profiles/member_one?locale=ja",
            "https://example.invalid/profiles/member_two?locale=ja",
        ],
        &plain_text,
    );
}

#[test]
fn paragraph_and_break_boundaries_end_the_url_candidate() {
    for html in [
        "<p>Member 1: https://example.invalid/one</p><p>Member 2: https://example.invalid/two</p>",
        "<p>Member 1: https://example.invalid/one</p>\n<p>Member 2: https://example.invalid/two</p>",
        "<div>Member 1: https://example.invalid/one<br>Member 2: https://example.invalid/two</div>",
        "<div>Member 1: https://example.invalid/one<br>\nMember 2: https://example.invalid/two</div>",
    ] {
        let (plain_text, ranges) = projected_ranges(html);
        assert_urls(
            &ranges,
            &["https://example.invalid/one", "https://example.invalid/two"],
            &plain_text,
        );
    }
}

#[test]
fn block_boundaries_map_back_to_utf16_offsets_of_cjk_and_non_bmp_text() {
    let html = "<ul><li>会議😀 https://example.invalid/one</li><li>次 https://example.invalid/two</li></ul>";
    let (plain_text, ranges) = projected_ranges(html);

    assert_urls(
        &ranges,
        &["https://example.invalid/one", "https://example.invalid/two"],
        &plain_text,
    );
    // 会 and 議 are one UTF-16 unit each and 😀 is a surrogate pair, so the
    // first URL starts at unit 5 of the published plain text.
    assert_eq!(ranges[0].start_utf16, "会議😀 ".encode_utf16().count());
}

#[test]
fn inline_runs_stay_contiguous_so_a_wrapped_url_is_still_one_candidate() {
    let html = r#"<p><strong>https://example.invalid/</strong>split</p><p>https://example.invalid/other</p>"#;
    let (plain_text, ranges) = projected_ranges(html);

    assert_urls(
        &ranges,
        &[
            "https://example.invalid/split",
            "https://example.invalid/other",
        ],
        &plain_text,
    );
}
