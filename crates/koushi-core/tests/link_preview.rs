use koushi_core::{
    event::TimelineFormattedBody,
    link_preview::{
        LinkPreviewContext, extract_link_ranges, extract_urls, link_previews_for_message,
    },
};

fn urls(text: &str) -> Vec<String> {
    extract_link_ranges(text)
        .into_iter()
        .map(|range| range.url)
        .collect()
}

#[test]
fn link_preview_url_policy_keeps_unicode_path_and_query() {
    let text = "See https://tensor4all.org/blog/パス?q=日本語 for details";
    assert_eq!(
        urls(text),
        vec!["https://tensor4all.org/blog/パス?q=日本語"]
    );
}

#[test]
fn link_preview_url_policy_keeps_balanced_parentheses() {
    let text = "Docs: https://example.com/foo(bar)";
    assert_eq!(urls(text), vec!["https://example.com/foo(bar)"]);
}

#[test]
fn link_preview_url_policy_stops_before_cjk_punctuation() {
    let text = "https://example.com/a、次の文";
    assert_eq!(urls(text), vec!["https://example.com/a"]);
}

#[test]
fn link_preview_url_policy_trims_ascii_trailing_punctuation() {
    let text = "Visit https://example.com/page.";
    assert_eq!(urls(text), vec!["https://example.com/page"]);
}

#[test]
fn link_preview_url_policy_returns_distinct_ranges_for_repeated_url() {
    let text = "https://example.com/a and https://example.com/a again";
    let ranges = extract_link_ranges(text);
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].url, "https://example.com/a");
    assert_eq!(ranges[1].url, "https://example.com/a");
    assert!(ranges[0].end_utf16 < ranges[1].start_utf16);
}

#[test]
fn extract_urls_returns_unique_preview_urls() {
    let text = "https://example.com/a and https://example.com/a again";
    assert_eq!(
        extract_urls(Some(text), None),
        vec!["https://example.com/a"]
    );
}

#[test]
fn matrix_permalinks_are_not_link_preview_candidates() {
    let mention_url = "https://matrix.to/#/%40junya%3Aexample.invalid";
    let formatted = TimelineFormattedBody {
        html: r#"<a href="https://matrix.to/#/@junya:example.invalid">Junya Ito</a>: see <a href="https://example.com/paper">the paper</a>"#.to_owned(),
        plain_text: "Junya Ito: see the paper".to_owned(),
        code_blocks: Vec::new(),
    };

    assert_eq!(
        extract_urls(None, Some(&formatted)),
        vec!["https://example.com/paper"]
    );
    assert!(extract_urls(Some(mention_url), None).is_empty());
    assert!(extract_urls(Some("https://matrix.to/#/%23paper%3Aexample.invalid"), None).is_empty());
    assert!(extract_urls(Some("https://MATRIX.TO/#/@junya:example.invalid"), None).is_empty());
    assert!(extract_urls(Some("https://matrix.to:443/#/@junya:example.invalid"), None).is_empty());
    assert!(extract_urls(Some("http://matrix.to:80/#/@junya:example.invalid"), None).is_empty());

    for previewable in [
        "https://www.matrix.to/#/@junya:example.invalid",
        "https://matrix.to:8448/#/@junya:example.invalid",
        "http://matrix.to:443/#/@junya:example.invalid",
        "https://matrix.to.example/paper",
        "https://matrix.to@evil.example/paper",
    ] {
        assert_eq!(extract_urls(Some(previewable), None), vec![previewable]);
    }

    assert_eq!(
        urls(mention_url),
        vec![mention_url],
        "permalink exclusion must not remove the clickable link range"
    );
    assert_eq!(
        link_previews_for_message(
            Some(mention_url),
            None,
            "$mention",
            false,
            &LinkPreviewContext::default(),
        ),
        None,
        "a mention-only message must not reserve a preview skeleton"
    );
}

#[test]
fn link_preview_url_policy_reports_utf16_indices() {
    let text = "a https://example.com/パス b";
    let ranges = extract_link_ranges(text);
    assert_eq!(ranges.len(), 1);
    let range = &ranges[0];
    assert_eq!(range.url, "https://example.com/パス");
    // "a " = 2 UTF-16 code units; URL starts at index 2.
    assert_eq!(range.start_utf16, 2);
    // URL length in UTF-16: "https://example.com/" (20) + "パス" (2 code units) = 22.
    assert_eq!(range.end_utf16 - range.start_utf16, 22);
}
