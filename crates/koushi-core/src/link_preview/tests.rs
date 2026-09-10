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
    let body = "a https://x.com b https://x.com c https://y.com d https://z.com e https://w.com";
    let urls = extract_urls(Some(body), None);
    assert_eq!(
        urls,
        vec!["https://x.com", "https://y.com", "https://z.com"]
    );
}

#[test]
fn formatted_paragraph_boundary_does_not_swallow_next_word() {
    // Regression for #870: the raw formatted plain text concatenates paragraphs
    // into ".../projectYou can try this.", which used to yield two candidates.
    let formatted = fmt_body_with_html(
        "<p>https://example.invalid/project</p><p>You can try this.</p>",
        "https://example.invalid/projectYou can try this.",
    );
    let body = "https://example.invalid/project\n\nYou can try this.";
    assert_eq!(
        extract_urls(Some(body), Some(&formatted)),
        vec!["https://example.invalid/project"]
    );
}

#[test]
fn formatted_paragraph_boundary_terminates_url_without_plain_body() {
    let formatted = fmt_body_with_html(
        "<p>https://example.invalid/project</p><p>You can try this.</p>",
        "https://example.invalid/projectYou can try this.",
    );
    assert_eq!(
        extract_urls(None, Some(&formatted)),
        vec!["https://example.invalid/project"]
    );
}

#[test]
fn formatted_br_boundary_terminates_url() {
    let formatted = fmt_body_with_html(
        "<p>Visit https://example.invalid/project<br>then next line</p>",
        "Visit https://example.invalid/projectthen next line",
    );
    assert_eq!(
        extract_urls(None, Some(&formatted)),
        vec!["https://example.invalid/project"]
    );
}

#[test]
fn formatted_inline_formatting_keeps_split_url_intact() {
    let formatted = fmt_body_with_html(
        "<p>https://example.invalid/a<strong>b</strong></p>",
        "https://example.invalid/ab",
    );
    assert_eq!(
        extract_urls(None, Some(&formatted)),
        vec!["https://example.invalid/ab"]
    );
}

#[test]
fn formatted_anchor_and_matrix_mention_yield_one_candidate() {
    let html = r##"<p><a href="https://matrix.to/#/%40alice%3Aexample.invalid">@alice</a> check <a href="https://example.invalid/project">https://example.invalid/project</a></p>"##;
    let formatted = fmt_body_with_html(html, "@alice check https://example.invalid/project");
    assert_eq!(
        extract_urls(
            Some("@alice check https://example.invalid/project"),
            Some(&formatted)
        ),
        vec!["https://example.invalid/project"]
    );
}

#[test]
fn formatted_distinct_urls_in_adjacent_paragraphs_stay_separate() {
    let formatted = fmt_body_with_html(
        "<p>https://one.example.invalid/a</p><p>https://two.example.invalid/b</p>",
        "https://one.example.invalid/ahttps://two.example.invalid/b",
    );
    assert_eq!(
        extract_urls(None, Some(&formatted)),
        vec![
            "https://one.example.invalid/a",
            "https://two.example.invalid/b"
        ]
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
