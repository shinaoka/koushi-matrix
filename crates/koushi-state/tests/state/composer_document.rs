use koushi_state::{ComposerDocument, ComposerInline, MentionTarget};

fn user(user_id: &str, display_label: &str) -> MentionTarget {
    MentionTarget::User {
        user_id: user_id.to_owned(),
        display_label: display_label.to_owned(),
    }
}

#[test]
fn composer_document_normalizes_text_without_losing_mention_occurrences() {
    let alice = user("@alice:example.invalid", "Same Name");
    let bob = user("@bob:example.invalid", "Same Name");
    let document = ComposerDocument::new(vec![
        ComposerInline::Text {
            text: "hello ".into(),
        },
        ComposerInline::Text {
            text: String::new(),
        },
        ComposerInline::Text {
            text: "world ".into(),
        },
        ComposerInline::Mention {
            target: alice.clone(),
            display_label: "Same Name".into(),
        },
        ComposerInline::Mention {
            target: alice.clone(),
            display_label: "Same Name".into(),
        },
        ComposerInline::Mention {
            target: bob.clone(),
            display_label: "Same Name".into(),
        },
    ]);

    assert_eq!(document.version, ComposerDocument::VERSION);
    assert_eq!(document.inlines.len(), 4);
    assert_eq!(
        document.inlines[0],
        ComposerInline::Text {
            text: "hello world ".into()
        }
    );
    assert_eq!(
        document.plain_body(),
        "hello world @Same Name@Same Name@Same Name"
    );
    assert_eq!(document.mention_intent().targets, vec![alice, bob]);
}

#[test]
fn plain_text_never_fabricates_semantic_mentions() {
    let document = ComposerDocument::from_plain_text("@Same Name and @room");

    assert_eq!(document.plain_body(), "@Same Name and @room");
    assert!(document.mention_intent().targets.is_empty());
    assert_eq!(
        serde_json::to_value(document).unwrap(),
        serde_json::json!({
            "version": 2,
            "inlines": [{ "kind": "text", "text": "@Same Name and @room" }]
        })
    );
}

#[test]
fn formatted_body_links_each_mention_identity_without_inferring_plain_text() {
    let document = ComposerDocument::new(vec![
        ComposerInline::Text {
            text: "**hello** @Same & Name ".into(),
        },
        ComposerInline::Mention {
            target: user("@alice:example.invalid", "Same & Name"),
            display_label: "Same & Name".into(),
        },
        ComposerInline::Text {
            text: " and ".into(),
        },
        ComposerInline::Mention {
            target: user("@bob:example.invalid", "Same & Name"),
            display_label: "Same & Name".into(),
        },
        ComposerInline::Text {
            text: " typed @Same & Name".into(),
        },
    ]);

    assert_eq!(
        document.formatted_body(),
        Some(
            "<strong>hello</strong> @Same &amp; Name <a href=\"https://matrix.to/#/%40alice%3Aexample.invalid\">@Same &amp; Name</a> and <a href=\"https://matrix.to/#/%40bob%3Aexample.invalid\">@Same &amp; Name</a> typed @Same &amp; Name".into()
        )
    );
}

#[test]
fn formatted_mentions_escape_cjk_labels_and_debug_redacts_content_and_identity() {
    let document = ComposerDocument::new(vec![
        ComposerInline::Text {
            text: "秘密 ".into(),
        },
        ComposerInline::Mention {
            target: user("@alice:example.invalid", "研究 <&>"),
            display_label: "研究 <&>".into(),
        },
    ]);

    assert_eq!(
        document.formatted_body().as_deref(),
        Some(
            "秘密 <a href=\"https://matrix.to/#/%40alice%3Aexample.invalid\">@研究 &lt;&amp;&gt;</a>"
        )
    );
    let debug = format!("{document:?}");
    assert!(!debug.contains("秘密"));
    assert!(!debug.contains("alice"));
    assert!(!debug.contains("研究"));
}

#[test]
fn formatted_mentions_preserve_authored_newlines_and_plain_fallback() {
    let document = ComposerDocument::new(vec![
        ComposerInline::Mention {
            target: user("@alice:example.invalid", "Alice"),
            display_label: "Alice".into(),
        },
        ComposerInline::Text {
            text: "\nhttps://example.invalid/pull/7\n\nFeedback from the port".into(),
        },
    ]);

    assert_eq!(
        document.plain_body(),
        "@Alice\nhttps://example.invalid/pull/7\n\nFeedback from the port"
    );
    assert_eq!(
        document.formatted_body().as_deref(),
        Some(
            "<a href=\"https://matrix.to/#/%40alice%3Aexample.invalid\">@Alice</a><br>https://example.invalid/pull/7<br><br>Feedback from the port"
        )
    );
}

#[test]
fn formatted_mentions_define_leading_trailing_and_repeated_newlines() {
    let document = ComposerDocument::new(vec![
        ComposerInline::Text { text: "\n".into() },
        ComposerInline::Mention {
            target: user("@alice:example.invalid", "Alice"),
            display_label: "Alice".into(),
        },
        ComposerInline::Text {
            text: "\n\n".into(),
        },
    ]);

    assert_eq!(document.plain_body(), "\n@Alice\n\n");
    assert_eq!(
        document.formatted_body().as_deref(),
        Some("<br><a href=\"https://matrix.to/#/%40alice%3Aexample.invalid\">@Alice</a><br><br>")
    );
}

#[test]
fn edit_shaped_mention_replacement_preserves_blank_line() {
    let replacement = ComposerDocument::new(vec![
        ComposerInline::Text {
            text: "**Updated** ".into(),
        },
        ComposerInline::Mention {
            target: user("@alice:example.invalid", "Alice"),
            display_label: "Alice".into(),
        },
        ComposerInline::Text {
            text: "\n\nReplacement detail".into(),
        },
    ]);

    assert_eq!(
        replacement.formatted_body().as_deref(),
        Some(
            "<strong>Updated</strong> <a href=\"https://matrix.to/#/%40alice%3Aexample.invalid\">@Alice</a><br><br>Replacement detail"
        )
    );
}

#[test]
fn room_mentions_and_room_targets_are_deduplicated_by_identity() {
    let room = MentionTarget::Room {
        room_id: "!room:example.invalid".into(),
        display_label: "Room".into(),
    };
    let room_mention = MentionTarget::RoomMention {
        display_label: "room".into(),
    };
    let document = ComposerDocument::new(vec![
        ComposerInline::Mention {
            target: room.clone(),
            display_label: "Room".into(),
        },
        ComposerInline::Mention {
            target: room.clone(),
            display_label: "Renamed room occurrence".into(),
        },
        ComposerInline::Mention {
            target: room_mention.clone(),
            display_label: "room".into(),
        },
        ComposerInline::Mention {
            target: room_mention.clone(),
            display_label: "room".into(),
        },
    ]);

    assert_eq!(document.mention_intent().targets, vec![room, room_mention]);
    assert_eq!(
        document.formatted_body().as_deref(),
        Some(
            "<a href=\"https://matrix.to/#/%21room%3Aexample.invalid\">@Room</a><a href=\"https://matrix.to/#/%21room%3Aexample.invalid\">@Renamed room occurrence</a><span data-mx-mention>@room</span><span data-mx-mention>@room</span>"
        )
    );
}
