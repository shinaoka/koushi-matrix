use koushi_state::{
    ComposerFormattingOptions, ComposerKey, ComposerKeyFacts, ComposerKeyModifiers,
    ComposerResolvedAction, ComposerResolverContext, ComposerSelection, ComposerSendIntent,
    ComposerSendShortcut, ComposerSurface, FormattedMessageDraft, MentionIntent, MentionTarget,
    SlashCommandIntent, build_formatted_message_draft, build_formatted_message_draft_with_options,
    resolve_composer_key_action, resolve_composer_send_intent,
};

fn key_facts(
    surface: ComposerSurface,
    is_composing: bool,
) -> (ComposerKeyFacts, ComposerResolverContext) {
    (
        ComposerKeyFacts {
            key: ComposerKey::Enter,
            modifiers: ComposerKeyModifiers::default(),
            is_composing,
            selection: Some(ComposerSelection { start: 0, end: 0 }),
        },
        ComposerResolverContext {
            surface,
            send_shortcut: ComposerSendShortcut::Enter,
            autocomplete_open: true,
            send_enabled: true,
        },
    )
}

#[test]
fn composer_composing_enter_commits_ime_candidate_and_never_sends() {
    let (event, context) = key_facts(ComposerSurface::Main, true);

    assert_eq!(
        resolve_composer_key_action(event, context),
        ComposerResolvedAction::CommitImeCandidate
    );
}

#[test]
fn composer_main_thread_and_edit_surfaces_share_the_same_key_facts_model() {
    for surface in [
        ComposerSurface::Main,
        ComposerSurface::Thread,
        ComposerSurface::Edit,
    ] {
        let (event, mut context) = key_facts(surface, false);
        context.autocomplete_open = false;

        assert_eq!(
            resolve_composer_key_action(event, context),
            ComposerResolvedAction::Send
        );
    }
}

#[test]
fn composer_mention_intent_preserves_structured_candidate_targets() {
    let intent = MentionIntent {
        targets: vec![
            MentionTarget::User {
                user_id: "@alice:example.test".to_owned(),
                display_label: "Alice".to_owned(),
            },
            MentionTarget::Room {
                room_id: "!room:example.test".to_owned(),
                display_label: "Project".to_owned(),
            },
            MentionTarget::RoomMention {
                display_label: "@room".to_owned(),
            },
        ],
    };

    assert_eq!(intent.targets.len(), 3);
    assert_eq!(intent.user_ids(), vec!["@alice:example.test".to_owned()]);
    assert!(intent.mentions_room());
}

#[test]
fn composer_markdown_send_request_keeps_plain_body_plus_formatted_body() {
    let draft =
        build_formatted_message_draft("hello **world** and `code`", MentionIntent::default());

    assert_eq!(
        draft,
        FormattedMessageDraft {
            plain_body: "hello **world** and `code`".to_owned(),
            formatted_body: Some("hello <strong>world</strong> and <code>code</code>".to_owned()),
            mentions: MentionIntent::default(),
        }
    );
}

#[test]
fn composer_spoiler_markdown_is_rust_owned_formatted_body() {
    let draft = build_formatted_message_draft("keep ||secret|| hidden", MentionIntent::default());

    assert_eq!(draft.plain_body, "keep ||secret|| hidden");
    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("keep <span data-mx-spoiler>secret</span> hidden")
    );
}

#[test]
fn composer_markdown_unordered_lists_are_rust_owned_formatted_body() {
    let draft = build_formatted_message_draft("- first\n- **second**", MentionIntent::default());

    assert_eq!(draft.plain_body, "- first\n- **second**");
    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>first</li><li><strong>second</strong></li></ul>")
    );
}

#[test]
fn composer_nested_list_two_space_indentation() {
    let draft = build_formatted_message_draft("- A\n  - B", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>A<ul><li>B</li></ul></li></ul>")
    );
}

#[test]
fn composer_nested_list_four_space_indentation() {
    let draft = build_formatted_message_draft("- A\n    - B", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>A<ul><li>B</li></ul></li></ul>")
    );
}

#[test]
fn composer_nested_list_siblings_then_root_sibling_close_in_order() {
    let draft = build_formatted_message_draft("- A\n  - B\n  - C\n- D", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>A<ul><li>B</li><li>C</li></ul></li><li>D</li></ul>")
    );
}

#[test]
fn composer_nested_list_three_levels_outdent_to_open_levels() {
    let draft =
        build_formatted_message_draft("- A\n  - B\n    - C\n  - D\n- E", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>A<ul><li>B<ul><li>C</li></ul></li><li>D</li></ul></li><li>E</li></ul>")
    );
}

#[test]
fn composer_nested_list_unmatched_outdent_clamps_to_root() {
    let draft = build_formatted_message_draft("- A\n    - B\n  - C", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>A<ul><li>B</li></ul></li><li>C</li></ul>")
    );
}

#[test]
fn composer_nested_list_indented_root_seeds_siblings_and_nesting() {
    let draft =
        build_formatted_message_draft("  - A\n  - B\n    - C\n  - D", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<ul><li>A</li><li>B<ul><li>C</li></ul></li><li>D</li></ul>")
    );
}

#[test]
fn composer_nested_list_keeps_inline_formatting_and_escaping() {
    let draft = build_formatted_message_draft(
        "- **root** & <root>\n  - *nested* & <nested> `code`",
        MentionIntent::default(),
    );

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some(
            "<ul><li><strong>root</strong> &amp; &lt;root&gt;<ul><li><em>nested</em> &amp; &lt;nested&gt; <code>code</code></li></ul></li></ul>"
        )
    );
}

#[test]
fn composer_inline_markdown_uses_explicit_html_breaks() {
    let draft = build_formatted_message_draft("**first**\nsecond", MentionIntent::default());

    assert_eq!(draft.plain_body, "**first**\nsecond");
    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<strong>first</strong><br>second")
    );
}

#[test]
fn plain_multiline_text_still_uses_only_the_plain_body() {
    let draft = build_formatted_message_draft("first\n\nsecond", MentionIntent::default());

    assert_eq!(draft.plain_body, "first\n\nsecond");
    assert_eq!(draft.formatted_body, None);
}

#[test]
fn composer_block_boundaries_preserve_only_authored_blank_lines() {
    let cases = [
        (
            "before\n- one\n- two",
            "before\n<ul><li>one</li><li>two</li></ul>",
        ),
        (
            "- one\n- two\nafter",
            "<ul><li>one</li><li>two</li></ul>\nafter",
        ),
        ("before\n\n- one", "before<br>\n<ul><li>one</li></ul>"),
        ("- one\n\nafter", "<ul><li>one</li></ul>\n<br>after"),
        (
            "before\n$$\nx\ny\n$$",
            "before\n<div data-mx-maths=\"x\ny\">x\ny</div>",
        ),
        (
            "$$\nx\ny\n$$\nafter",
            "<div data-mx-maths=\"x\ny\">x\ny</div>\nafter",
        ),
        (
            "before\n\n$$\nx\ny\n$$",
            "before<br>\n<div data-mx-maths=\"x\ny\">x\ny</div>",
        ),
        (
            "$$\nx\ny\n$$\n\nafter",
            "<div data-mx-maths=\"x\ny\">x\ny</div>\n<br>after",
        ),
    ];

    for (body, expected) in cases {
        let draft = build_formatted_message_draft(body, MentionIntent::default());
        assert_eq!(draft.formatted_body.as_deref(), Some(expected), "{body:?}");
    }
}

#[test]
fn composer_math_markdown_uses_matrix_math_html_by_default() {
    let draft = build_formatted_message_draft(
        "Energy $E=mc^2$\n$$\n\\int_0^1 x dx\n$$",
        MentionIntent::default(),
    );

    assert_eq!(draft.plain_body, "Energy $E=mc^2$\n$$\n\\int_0^1 x dx\n$$");
    assert_eq!(
        draft.formatted_body.as_deref(),
        Some(
            "Energy <span data-mx-maths=\"E=mc^2\">E=mc^2</span>\n<div data-mx-maths=\"\\int_0^1 x dx\">\\int_0^1 x dx</div>"
        )
    );
}

#[test]
fn composer_math_accepts_latex_paren_and_bracket_delimiters() {
    // #455: `\(…\)` and `\[…\]` are what LaTeX itself defines and what people
    // paste out of papers and Overleaf. Before this they were escaped and sent
    // as literal characters with no error.
    let draft = build_formatted_message_draft(
        "Energy \\(E=mc^2\\)\n\\[ \\int_0^1 x dx \\]",
        MentionIntent::default(),
    );

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some(
            "Energy <span data-mx-maths=\"E=mc^2\">E=mc^2</span>\n<div data-mx-maths=\" \\int_0^1 x dx \"> \\int_0^1 x dx </div>"
        )
    );
}

#[test]
fn composer_math_accepts_fenced_latex_display_block() {
    let draft = build_formatted_message_draft("\\[\n\\int_0^1 x dx\n\\]", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<div data-mx-maths=\"\\int_0^1 x dx\">\\int_0^1 x dx</div>")
    );
}

#[test]
fn composer_math_renders_mid_sentence_bracket_delimiters_inline() {
    // A display block cannot be nested inside a paragraph, so a `\[…\]` that is
    // not alone on its line degrades to an inline span instead of rendering as
    // literal text.
    let draft = build_formatted_message_draft("see \\[x\\] here", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("see <span data-mx-maths=\"x\">x</span> here")
    );
}

#[test]
fn composer_math_latex_delimiters_survive_double_backslash_line_breaks() {
    // `\\` is a LaTeX line break; scanning for a bare `\` would end the formula
    // on it instead of on the real closing delimiter.
    let draft = build_formatted_message_draft("\\(a \\\\ b\\)", MentionIntent::default());

    assert_eq!(
        draft.formatted_body.as_deref(),
        Some("<span data-mx-maths=\"a \\\\ b\">a \\\\ b</span>")
    );
}

#[test]
fn composer_math_leaves_empty_latex_delimiters_literal() {
    let draft = build_formatted_message_draft("\\(\\) and \\[\\]", MentionIntent::default());

    assert_eq!(draft.formatted_body, None);
}

#[test]
fn composer_math_mode_off_leaves_latex_delimiters_literal() {
    let draft = build_formatted_message_draft_with_options(
        "Energy \\(E=mc^2\\)\n\\[ x \\]",
        MentionIntent::default(),
        ComposerFormattingOptions { math_mode: false },
    );

    assert_eq!(draft.formatted_body, None);
}

#[test]
fn composer_math_mode_off_leaves_dollar_delimiters_literal() {
    let draft = build_formatted_message_draft_with_options(
        "Energy $E=mc^2$",
        MentionIntent::default(),
        ComposerFormattingOptions { math_mode: false },
    );

    assert_eq!(draft.plain_body, "Energy $E=mc^2$");
    assert_eq!(draft.formatted_body, None);
}

#[test]
fn composer_math_requires_a_closing_delimiter_on_the_same_line() {
    let draft = build_formatted_message_draft(
        "The price is $5\nand the formula is still unclosed",
        MentionIntent::default(),
    );

    assert_eq!(draft.formatted_body, None);
}

#[test]
fn composer_escaped_dollar_is_literal_even_when_math_mode_is_on() {
    let draft = build_formatted_message_draft(r"literal \$x$ only", MentionIntent::default());

    assert_eq!(draft.plain_body, r"literal \$x$ only");
    assert_eq!(draft.formatted_body.as_deref(), Some("literal $x$ only"));
}

#[test]
fn composer_me_slash_command_returns_structured_emote_intent() {
    assert_eq!(
        resolve_composer_send_intent("/me waves", MentionIntent::default()),
        ComposerSendIntent::SlashCommand {
            command: SlashCommandIntent::Me {
                body: "waves".to_owned()
            },
        }
    );
}

#[test]
fn composer_unknown_slash_text_is_plain_content() {
    // Issue #450: unknown leading-slash tokens are ordinary messages and are
    // sent literally (the slash is preserved).
    assert_eq!(
        resolve_composer_send_intent("/shrug nope", MentionIntent::default()),
        ComposerSendIntent::Message {
            draft: koushi_state::build_formatted_message_draft(
                "/shrug nope",
                MentionIntent::default()
            ),
        }
    );
    assert_eq!(
        resolve_composer_send_intent("/usr/local/bin", MentionIntent::default()),
        ComposerSendIntent::Message {
            draft: koushi_state::build_formatted_message_draft(
                "/usr/local/bin",
                MentionIntent::default()
            ),
        }
    );
    assert_eq!(
        resolve_composer_send_intent("/ 文章", MentionIntent::default()),
        ComposerSendIntent::Message {
            draft: koushi_state::build_formatted_message_draft("/ 文章", MentionIntent::default()),
        }
    );
}

#[test]
fn composer_recognized_unavailable_commands_are_structured_slash_intents() {
    // Issue #450: /join and /invite remain recognized commands; the desktop
    // surfaces their rejection locally instead of sending them as text.
    assert_eq!(
        resolve_composer_send_intent("/join #room:example.invalid", MentionIntent::default()),
        ComposerSendIntent::SlashCommand {
            command: SlashCommandIntent::Join {
                room_alias: "#room:example.invalid".to_owned(),
            },
        }
    );
    assert_eq!(
        resolve_composer_send_intent("/invite @alice:example.invalid", MentionIntent::default()),
        ComposerSendIntent::SlashCommand {
            command: SlashCommandIntent::Invite {
                user_id: "@alice:example.invalid".to_owned(),
            },
        }
    );
}

fn formatted(body: &str) -> Option<String> {
    build_formatted_message_draft(body, MentionIntent::default()).formatted_body
}

#[test]
fn composer_markdown_links_become_anchors_for_safe_schemes_only() {
    assert_eq!(
        formatted("see [the docs](https://example.com/a?b=1&c=2) now").as_deref(),
        Some("see <a href=\"https://example.com/a?b=1&amp;c=2\">the docs</a> now")
    );
    assert_eq!(
        formatted("[mail](mailto:someone@example.com)").as_deref(),
        Some("<a href=\"mailto:someone@example.com\">mail</a>")
    );
    assert_eq!(
        formatted("[<b>](http://example.com/\"x)").as_deref(),
        Some("<a href=\"http://example.com/&quot;x\">&lt;b&gt;</a>")
    );
    // Unsafe or unknown schemes stay literal text.
    assert_eq!(formatted("[x](javascript:alert(1))"), None);
    assert_eq!(formatted("[x](data:text/html,hi)"), None);
    assert_eq!(formatted("[x](relative/path)"), None);
    // Not a link.
    assert_eq!(formatted("[x] (https://example.com)"), None);
    assert_eq!(formatted("[x](https://exa mple.com)"), None);
}

#[test]
fn composer_markdown_underscore_emphasis_respects_word_boundaries() {
    assert_eq!(
        formatted("an _em_ word").as_deref(),
        Some("an <em>em</em> word")
    );
    assert_eq!(formatted("snake_case_name"), None);
    assert_eq!(
        formatted("file_name.txt and _x_").as_deref(),
        Some("file_name.txt and <em>x</em>")
    );
    assert_eq!(formatted("_ not emphasis _"), None);
}

#[test]
fn composer_toolbar_link_and_italic_output_is_formatted() {
    // The Link then Italic toolbar buttons on an empty composer.
    assert_eq!(
        formatted("_[link](https://)_").as_deref(),
        Some("<em><a href=\"https://\">link</a></em>")
    );
    assert_eq!(
        formatted("[link](https://example.com) **bold** _em_ *star* `code`").as_deref(),
        Some(
            "<a href=\"https://example.com\">link</a> <strong>bold</strong> <em>em</em> <em>star</em> <code>code</code>"
        )
    );
}
