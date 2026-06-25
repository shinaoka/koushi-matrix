use koushi_core::link_preview::extract_link_ranges;

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
    use koushi_core::link_preview::extract_urls;

    let text = "https://example.com/a and https://example.com/a again";
    assert_eq!(
        extract_urls(Some(text), None),
        vec!["https://example.com/a"]
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
