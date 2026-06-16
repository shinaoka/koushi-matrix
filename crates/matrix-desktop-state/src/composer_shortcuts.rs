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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerSelection {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerKeyEvent {
    pub key: ComposerKey,
    pub modifiers: ComposerKeyModifiers,
    pub is_composing: bool,
    #[serde(default)]
    pub selection: Option<ComposerSelection>,
}

pub type ComposerKeyFacts = ComposerKeyEvent;

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
    CommitImeCandidate,
    Noop,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MentionIntent {
    pub targets: Vec<MentionTarget>,
}

impl MentionIntent {
    pub fn user_ids(&self) -> Vec<String> {
        self.targets
            .iter()
            .filter_map(|target| match target {
                MentionTarget::User { user_id, .. } => Some(user_id.clone()),
                MentionTarget::Room { .. } | MentionTarget::RoomMention { .. } => None,
            })
            .collect()
    }

    pub fn room_ids(&self) -> Vec<String> {
        self.targets
            .iter()
            .filter_map(|target| match target {
                MentionTarget::Room { room_id, .. } => Some(room_id.clone()),
                MentionTarget::User { .. } | MentionTarget::RoomMention { .. } => None,
            })
            .collect()
    }

    pub fn mentions_room(&self) -> bool {
        self.targets
            .iter()
            .any(|target| matches!(target, MentionTarget::RoomMention { .. }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MentionTarget {
    User {
        user_id: String,
        display_label: String,
    },
    Room {
        room_id: String,
        display_label: String,
    },
    RoomMention {
        display_label: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FormattedMessageDraft {
    pub plain_body: String,
    pub formatted_body: Option<String>,
    pub mentions: MentionIntent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SlashCommandIntent {
    Join { room_alias: String },
    Invite { user_id: String },
    Me { body: String },
    PlainText { body: String },
    Unsupported { command: String, argument: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ComposerSendIntent {
    Message { draft: FormattedMessageDraft },
    SlashCommand { command: SlashCommandIntent },
    LocalFailure { command: SlashCommandIntent },
}

pub fn resolve_composer_key_action(
    event: ComposerKeyEvent,
    context: ComposerResolverContext,
) -> ComposerResolvedAction {
    if event.is_composing {
        return ComposerResolvedAction::CommitImeCandidate;
    }

    match event.key {
        ComposerKey::Escape => ComposerResolvedAction::Cancel,
        ComposerKey::Other => ComposerResolvedAction::Noop,
        ComposerKey::Enter => resolve_enter_key(event.modifiers, context),
    }
}

pub fn build_formatted_message_draft(
    body: impl Into<String>,
    mentions: MentionIntent,
) -> FormattedMessageDraft {
    let plain_body = body.into();
    FormattedMessageDraft {
        formatted_body: format_markdown_subset(&plain_body),
        plain_body,
        mentions,
    }
}

pub fn parse_slash_command(body: &str) -> SlashCommandIntent {
    if !body.starts_with('/') {
        return SlashCommandIntent::PlainText {
            body: body.to_owned(),
        };
    }

    if let Some(escaped) = body.strip_prefix("//") {
        return SlashCommandIntent::PlainText {
            body: format!("/{escaped}"),
        };
    }

    let command_body = &body[1..];
    let mut parts = command_body.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().to_ascii_lowercase();
    let argument = parts.next().unwrap_or_default().trim().to_owned();

    match command.as_str() {
        "join" => SlashCommandIntent::Join {
            room_alias: argument,
        },
        "invite" => SlashCommandIntent::Invite { user_id: argument },
        "me" => SlashCommandIntent::Me { body: argument },
        _ => SlashCommandIntent::Unsupported { command, argument },
    }
}

pub fn resolve_composer_send_intent(body: &str, mentions: MentionIntent) -> ComposerSendIntent {
    match parse_slash_command(body) {
        SlashCommandIntent::PlainText { body } => ComposerSendIntent::Message {
            draft: build_formatted_message_draft(body, mentions),
        },
        command @ SlashCommandIntent::Me { .. }
        | command @ SlashCommandIntent::Join { .. }
        | command @ SlashCommandIntent::Invite { .. } => {
            ComposerSendIntent::SlashCommand { command }
        }
        command @ SlashCommandIntent::Unsupported { .. } => {
            ComposerSendIntent::LocalFailure { command }
        }
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
            ComposerResolvedAction::Noop
        }
    } else {
        ComposerResolvedAction::InsertNewline
    }
}

fn format_markdown_subset(body: &str) -> Option<String> {
    let mut html = String::with_capacity(body.len());
    let mut changed = false;
    let mut index = 0;

    while index < body.len() {
        let rest = &body[index..];
        if let Some(after) = rest.strip_prefix("||")
            && let Some(end) = after.find("||")
        {
            html.push_str("<span data-mx-spoiler>");
            push_escaped_html(&mut html, &after[..end]);
            html.push_str("</span>");
            index += 2 + end + 2;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            html.push_str("<strong>");
            push_escaped_html(&mut html, &after[..end]);
            html.push_str("</strong>");
            index += 2 + end + 2;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            html.push_str("<code>");
            push_escaped_html(&mut html, &after[..end]);
            html.push_str("</code>");
            index += 1 + end + 1;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            html.push_str("<em>");
            push_escaped_html(&mut html, &after[..end]);
            html.push_str("</em>");
            index += 1 + end + 1;
            changed = true;
            continue;
        }

        let ch = rest
            .chars()
            .next()
            .expect("rest is non-empty while scanning markdown subset");
        push_escaped_html_char(&mut html, ch);
        index += ch.len_utf8();
    }

    changed.then_some(html)
}

fn push_escaped_html(output: &mut String, value: &str) {
    for ch in value.chars() {
        push_escaped_html_char(output, ch);
    }
}

fn push_escaped_html_char(output: &mut String, ch: char) {
    match ch {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        '"' => output.push_str("&quot;"),
        '\'' => output.push_str("&#39;"),
        _ => output.push(ch),
    }
}
