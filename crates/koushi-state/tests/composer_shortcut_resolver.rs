use koushi_state::{
    ComposerKey, ComposerKeyEvent, ComposerKeyModifiers, ComposerResolvedAction,
    ComposerResolverContext, ComposerSendShortcut, ComposerSurface, resolve_composer_key_action,
};

fn key_event(key: ComposerKey) -> ComposerKeyEvent {
    ComposerKeyEvent {
        key,
        modifiers: ComposerKeyModifiers::default(),
        is_composing: false,
        selection: None,
    }
}

fn enter_with(mut modifiers: ComposerKeyModifiers) -> ComposerKeyEvent {
    modifiers.alt = false;
    ComposerKeyEvent {
        key: ComposerKey::Enter,
        modifiers,
        is_composing: false,
        selection: None,
    }
}

fn context(surface: ComposerSurface) -> ComposerResolverContext {
    ComposerResolverContext {
        surface,
        send_shortcut: ComposerSendShortcut::Enter,
        autocomplete_open: false,
        send_enabled: true,
    }
}

#[test]
fn composer_enter_mode_sends_plain_enter_on_every_surface() {
    for surface in [
        ComposerSurface::Main,
        ComposerSurface::Thread,
        ComposerSurface::Edit,
    ] {
        assert_eq!(
            resolve_composer_key_action(key_event(ComposerKey::Enter), context(surface)),
            ComposerResolvedAction::Send
        );
    }
}

#[test]
fn composer_shift_enter_always_inserts_newline() {
    let mut event = key_event(ComposerKey::Enter);
    event.modifiers.shift = true;

    assert_eq!(
        resolve_composer_key_action(event, context(ComposerSurface::Main)),
        ComposerResolvedAction::InsertNewline
    );
}

#[test]
fn composer_mod_enter_mode_keeps_plain_enter_as_newline_and_sends_on_platform_modifier() {
    let mut plain_context = context(ComposerSurface::Main);
    plain_context.send_shortcut = ComposerSendShortcut::ModEnter;
    assert_eq!(
        resolve_composer_key_action(key_event(ComposerKey::Enter), plain_context),
        ComposerResolvedAction::InsertNewline
    );

    let mut ctrl_context = context(ComposerSurface::Thread);
    ctrl_context.send_shortcut = ComposerSendShortcut::ModEnter;
    assert_eq!(
        resolve_composer_key_action(
            enter_with(ComposerKeyModifiers {
                ctrl: true,
                ..ComposerKeyModifiers::default()
            }),
            ctrl_context,
        ),
        ComposerResolvedAction::Send
    );

    let mut meta_context = context(ComposerSurface::Edit);
    meta_context.send_shortcut = ComposerSendShortcut::ModEnter;
    assert_eq!(
        resolve_composer_key_action(
            enter_with(ComposerKeyModifiers {
                meta: true,
                ..ComposerKeyModifiers::default()
            }),
            meta_context,
        ),
        ComposerResolvedAction::Send
    );
}

#[test]
fn composer_autocomplete_acceptance_precedes_send_shortcut() {
    let mut open_context = context(ComposerSurface::Main);
    open_context.autocomplete_open = true;

    assert_eq!(
        resolve_composer_key_action(key_event(ComposerKey::Enter), open_context),
        ComposerResolvedAction::AcceptAutocomplete
    );
}

#[test]
fn composer_disabled_send_or_ime_composition_never_submits() {
    let mut disabled_context = context(ComposerSurface::Main);
    disabled_context.send_enabled = false;
    assert_eq!(
        resolve_composer_key_action(key_event(ComposerKey::Enter), disabled_context),
        ComposerResolvedAction::Noop
    );

    let mut composing = key_event(ComposerKey::Enter);
    composing.is_composing = true;
    assert_eq!(
        resolve_composer_key_action(composing, context(ComposerSurface::Main)),
        ComposerResolvedAction::CommitImeCandidate
    );
}

#[test]
fn composer_escape_cancels_reply_or_edit_modes() {
    assert_eq!(
        resolve_composer_key_action(
            key_event(ComposerKey::Escape),
            context(ComposerSurface::Edit)
        ),
        ComposerResolvedAction::Cancel
    );
}
