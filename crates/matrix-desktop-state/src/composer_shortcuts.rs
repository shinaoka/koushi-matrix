use serde::{Deserialize, Serialize};

use crate::state::ComposerSendShortcut;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposerSurface {
    Main,
    Thread,
    Edit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposerKey {
    Enter,
    Escape,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerKeyModifiers {
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerKeyEvent {
    pub key: ComposerKey,
    pub modifiers: ComposerKeyModifiers,
    pub is_composing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerResolverContext {
    pub surface: ComposerSurface,
    pub send_shortcut: ComposerSendShortcut,
    pub autocomplete_open: bool,
    pub send_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposerResolvedAction {
    Send,
    InsertNewline,
    AcceptAutocomplete,
    Cancel,
    Ignore,
}

pub fn resolve_composer_key_action(
    event: ComposerKeyEvent,
    context: ComposerResolverContext,
) -> ComposerResolvedAction {
    if event.is_composing {
        return ComposerResolvedAction::Ignore;
    }

    match event.key {
        ComposerKey::Escape => ComposerResolvedAction::Cancel,
        ComposerKey::Other => ComposerResolvedAction::Ignore,
        ComposerKey::Enter => resolve_enter_key(event.modifiers, context),
    }
}

fn resolve_enter_key(
    modifiers: ComposerKeyModifiers,
    context: ComposerResolverContext,
) -> ComposerResolvedAction {
    if modifiers.shift || modifiers.alt {
        return ComposerResolvedAction::InsertNewline;
    }

    if context.autocomplete_open {
        return ComposerResolvedAction::AcceptAutocomplete;
    }

    let wants_send = match context.send_shortcut {
        ComposerSendShortcut::Enter => true,
        ComposerSendShortcut::ModEnter => modifiers.ctrl || modifiers.meta,
    };

    if wants_send {
        if context.send_enabled {
            ComposerResolvedAction::Send
        } else {
            ComposerResolvedAction::Ignore
        }
    } else {
        ComposerResolvedAction::InsertNewline
    }
}
