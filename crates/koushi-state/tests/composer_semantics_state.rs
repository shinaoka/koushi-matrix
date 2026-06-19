use koushi_state::{
    ComposerKey, ComposerKeyFacts, ComposerKeyModifiers, ComposerResolvedAction,
    ComposerResolverContext, ComposerSelection, ComposerSendIntent, ComposerSendShortcut,
    ComposerSurface, FormattedMessageDraft, MentionIntent, MentionTarget, SlashCommandIntent,
    build_formatted_message_draft, resolve_composer_key_action, resolve_composer_send_intent,
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
fn composer_unknown_slash_command_returns_structured_local_failure() {
    assert_eq!(
        resolve_composer_send_intent("/shrug nope", MentionIntent::default()),
        ComposerSendIntent::LocalFailure {
            command: SlashCommandIntent::Unsupported {
                command: "shrug".to_owned(),
                argument: "nope".to_owned(),
            },
        }
    );
}
