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
    CloseAutocomplete,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposerFormattingOptions {
    #[serde(default = "default_math_mode")]
    pub math_mode: bool,
}

impl Default for ComposerFormattingOptions {
    fn default() -> Self {
        Self { math_mode: true }
    }
}

fn default_math_mode() -> bool {
    true
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
        ComposerKey::Escape => {
            if context.autocomplete_open {
                ComposerResolvedAction::CloseAutocomplete
            } else {
                ComposerResolvedAction::Cancel
            }
        }
        ComposerKey::Other => ComposerResolvedAction::Noop,
        ComposerKey::Enter => resolve_enter_key(event.modifiers, context),
    }
}

pub fn build_formatted_message_draft(
    body: impl Into<String>,
    mentions: MentionIntent,
) -> FormattedMessageDraft {
    build_formatted_message_draft_with_options(body, mentions, ComposerFormattingOptions::default())
}

pub fn build_formatted_message_draft_with_options(
    body: impl Into<String>,
    mentions: MentionIntent,
    options: ComposerFormattingOptions,
) -> FormattedMessageDraft {
    let plain_body = body.into();
    FormattedMessageDraft {
        formatted_body: format_markdown_subset(&plain_body, options),
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
        // Issue #450: only implemented/recognized commands are commands.
        // Unknown leading-slash text is ordinary content and is sent
        // literally with the slash preserved (the // escape remains for
        // compatibility).
        _ => SlashCommandIntent::PlainText {
            body: body.to_owned(),
        },
    }
}

pub fn resolve_composer_send_intent(body: &str, mentions: MentionIntent) -> ComposerSendIntent {
    resolve_composer_send_intent_with_options(body, mentions, ComposerFormattingOptions::default())
}

pub fn resolve_composer_send_intent_with_options(
    body: &str,
    mentions: MentionIntent,
    options: ComposerFormattingOptions,
) -> ComposerSendIntent {
    match parse_slash_command(body) {
        SlashCommandIntent::PlainText { body } => ComposerSendIntent::Message {
            draft: build_formatted_message_draft_with_options(body, mentions, options),
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

fn format_markdown_subset(body: &str, options: ComposerFormattingOptions) -> Option<String> {
    let (html, changed) = format_markdown_subset_html(body, options);
    changed.then_some(html)
}

pub(crate) fn format_markdown_subset_html(
    body: &str,
    options: ComposerFormattingOptions,
) -> (String, bool) {
    let mut html = String::with_capacity(body.len());
    let mut changed = false;
    let lines = body.split('\n').collect::<Vec<_>>();
    let mut line_index = 0;
    let mut previous_chunk = None;

    while line_index < lines.len() {
        if lines[line_index].is_empty() {
            line_index += 1;
            continue;
        }

        if options.math_mode
            && let Some((block_body, closing_index)) = math_block_body(&lines, line_index)
                .or_else(|| latex_display_block(&lines, line_index))
        {
            push_chunk_boundary(&mut html, previous_chunk, line_index, true);
            push_math_html(&mut html, "div", &block_body);
            changed = true;
            previous_chunk = Some((closing_index, true));
            line_index = closing_index + 1;
            continue;
        }

        if let Some((root_indent, item_body)) = unordered_list_item_body(lines[line_index]) {
            push_chunk_boundary(&mut html, previous_chunk, line_index, true);
            html.push_str("<ul>");
            let mut indentation_stack = vec![root_indent];
            html.push_str("<li>");
            let _ = push_inline_markdown_subset(&mut html, item_body, options);
            line_index += 1;

            while line_index < lines.len() {
                let Some((indent, item_body)) = unordered_list_item_body(lines[line_index]) else {
                    break;
                };
                let current_indent = *indentation_stack
                    .last()
                    .expect("list has a root indentation");
                if indent > current_indent {
                    html.push_str("<ul><li>");
                    indentation_stack.push(indent);
                } else if indent == current_indent {
                    html.push_str("</li><li>");
                } else {
                    let target = indentation_stack
                        .iter()
                        .rposition(|&opened_indent| opened_indent <= indent)
                        .unwrap_or(0);
                    while indentation_stack.len() > target + 1 {
                        html.push_str("</li></ul>");
                        indentation_stack.pop();
                    }
                    html.push_str("</li><li>");
                }
                let _ = push_inline_markdown_subset(&mut html, item_body, options);
                line_index += 1;
            }

            while indentation_stack.pop().is_some() {
                html.push_str("</li></ul>");
            }
            changed = true;
            previous_chunk = Some((line_index - 1, true));
            continue;
        }

        if blockquote_line_body(lines[line_index]).is_some() {
            push_chunk_boundary(&mut html, previous_chunk, line_index, true);
            html.push_str("<blockquote>");
            let mut first_line = true;
            while line_index < lines.len() {
                let Some(quote_body) = blockquote_line_body(lines[line_index]) else {
                    break;
                };
                if !first_line {
                    html.push_str("<br>");
                }
                let _ = push_inline_markdown_subset(&mut html, quote_body, options);
                first_line = false;
                line_index += 1;
            }
            html.push_str("</blockquote>");
            changed = true;
            previous_chunk = Some((line_index - 1, true));
            continue;
        }

        push_chunk_boundary(&mut html, previous_chunk, line_index, false);
        changed = push_inline_markdown_subset(&mut html, lines[line_index], options) || changed;
        previous_chunk = Some((line_index, false));
        line_index += 1;
    }

    if let Some((previous_end, _)) = previous_chunk {
        for _ in previous_end..lines.len() - 1 {
            html.push_str("<br>");
        }
    }

    (html, changed)
}

fn push_chunk_boundary(
    html: &mut String,
    previous_chunk: Option<(usize, bool)>,
    current_start: usize,
    current_is_block: bool,
) {
    let Some((previous_end, previous_is_block)) = previous_chunk else {
        for _ in 0..current_start {
            html.push_str("<br>");
        }
        return;
    };
    let newline_count = current_start - previous_end;
    if previous_is_block {
        html.push('\n');
    }
    for _ in usize::from(previous_is_block || current_is_block)..newline_count {
        html.push_str("<br>");
    }
    if current_is_block && !previous_is_block {
        html.push('\n');
    }
}

/// One `>` level of a block quote. Leading spaces are tolerated, as in
/// `unordered_list_item_body`, so a pasted indented quote still becomes a quote;
/// `>` alone is an empty quoted line.
///
/// shortcut: one level only and inline marks only inside it, so `> > …` keeps its
/// inner marker as literal `&gt;` text and `> - item` keeps its list marker as
/// text. Upgrade to recursive blocks when a client is seen sending quotes whose
/// nested structure must round-trip.
fn blockquote_line_body(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ').strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn unordered_list_item_body(line: &str) -> Option<(usize, &str)> {
    let leading_space_count = line.len() - line.trim_start_matches(' ').len();
    let trimmed = &line[leading_space_count..];
    let body = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))?;
    Some((leading_space_count, body))
}

fn math_block_body(lines: &[&str], start_index: usize) -> Option<(String, usize)> {
    if lines.get(start_index)?.trim() != "$$" {
        return None;
    }
    let closing_index = lines
        .iter()
        .enumerate()
        .skip(start_index + 1)
        .find_map(|(index, line)| (line.trim() == "$$").then_some(index))?;
    if closing_index == start_index + 1 {
        return None;
    }
    Some((
        lines[start_index + 1..closing_index].join("\n"),
        closing_index,
    ))
}

/// Byte index of the terminating `\<closing>` sequence in `value`.
///
/// `\\` is a LaTeX line break, so it is consumed as one unit. Scanning for a
/// bare `\` would otherwise end a formula containing `x \\) y` on the line
/// break instead of on its real delimiter.
fn find_latex_close(value: &str, closing: char) -> Option<usize> {
    let mut index = 0;
    while index < value.len() {
        let mut chars = value[index..].chars();
        let ch = chars.next()?;
        if ch != '\\' {
            index += ch.len_utf8();
            continue;
        }
        match chars.next() {
            Some(next) if next == closing => return Some(index),
            Some(next) => index += ch.len_utf8() + next.len_utf8(),
            None => return None,
        }
    }
    None
}

/// `\[ … \]` written as its own paragraph, either on a single line or fenced
/// across lines. Returns the formula body and the index of its closing line.
///
/// A line that continues after `\]` is not a block: a `div` cannot be nested
/// inside a paragraph, so that case falls through to the inline scanner and
/// renders as a span instead (#455).
fn latex_display_block(lines: &[&str], start_index: usize) -> Option<(String, usize)> {
    let after = lines.get(start_index)?.trim().strip_prefix("\\[")?;
    if let Some(end) = find_latex_close(after, ']') {
        if !after.get(end + 2..)?.trim().is_empty() {
            return None;
        }
        let body = &after[..end];
        return (!body.trim().is_empty()).then(|| (body.to_owned(), start_index));
    }
    if !after.trim().is_empty() {
        return None;
    }
    let closing_index = lines
        .iter()
        .enumerate()
        .skip(start_index + 1)
        .find_map(|(index, line)| (line.trim() == "\\]").then_some(index))?;
    if closing_index == start_index + 1 {
        return None;
    }
    Some((
        lines[start_index + 1..closing_index].join("\n"),
        closing_index,
    ))
}

fn push_inline_markdown_subset(
    html: &mut String,
    body: &str,
    options: ComposerFormattingOptions,
) -> bool {
    let mut changed = false;
    let mut index = 0;

    while index < body.len() {
        let rest = &body[index..];
        if let Some(after) = rest.strip_prefix("||")
            && let Some(end) = after.find("||")
        {
            html.push_str("<span data-mx-spoiler>");
            push_escaped_html(html, &after[..end]);
            html.push_str("</span>");
            index += 2 + end + 2;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix("\\$") {
            html.push('$');
            index += rest.len() - after.len();
            changed = true;
            continue;
        }
        // #455: LaTeX's own delimiters. `\(…\)` is inline. `\[…\]` is display, but
        // a display block cannot be nested inside a paragraph, so mid-line it
        // degrades to an inline span rather than falling through to literal text;
        // on its own line `latex_display_block` has already made it a `div`.
        if options.math_mode
            && let Some((after, closing)) = rest
                .strip_prefix("\\(")
                .map(|after| (after, ')'))
                .or_else(|| rest.strip_prefix("\\[").map(|after| (after, ']')))
            && let Some(end) = find_latex_close(after, closing)
            && !after[..end].trim().is_empty()
        {
            push_math_html(html, "span", &after[..end]);
            index += 2 + end + 2;
            changed = true;
            continue;
        }
        // #991: `[text](url)`, the composer toolbar's Link output. Only
        // http(s) and mailto targets become links; anything else stays text.
        if let Some((text, url, consumed)) = markdown_link(rest) {
            html.push_str("<a href=\"");
            push_escaped_html(html, url);
            html.push_str("\">");
            push_inline_markdown_subset(html, text, options);
            html.push_str("</a>");
            index += consumed;
            changed = true;
            continue;
        }
        // #991: `_text_`, the toolbar's Italic output. Underscores inside a
        // word (`snake_case`) are not emphasis.
        if let Some((inner, consumed)) = underscore_emphasis(&body[..index], rest) {
            html.push_str("<em>");
            push_inline_markdown_subset(html, inner, options);
            html.push_str("</em>");
            index += consumed;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            html.push_str("<strong>");
            push_escaped_html(html, &after[..end]);
            html.push_str("</strong>");
            index += 2 + end + 2;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            html.push_str("<code>");
            push_escaped_html(html, &after[..end]);
            html.push_str("</code>");
            index += 1 + end + 1;
            changed = true;
            continue;
        }
        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            html.push_str("<em>");
            push_escaped_html(html, &after[..end]);
            html.push_str("</em>");
            index += 1 + end + 1;
            changed = true;
            continue;
        }
        if options.math_mode
            && let Some(after) = rest.strip_prefix('$')
            && let Some(end) = find_unescaped_math_close(after)
            && end > 0
        {
            let latex = &after[..end];
            if !latex.trim().is_empty() {
                push_math_html(html, "span", latex);
                index += 1 + end + 1;
                changed = true;
                continue;
            }
        }

        let ch = rest
            .chars()
            .next()
            .expect("rest is non-empty while scanning markdown subset");
        push_escaped_html_char(html, ch);
        index += ch.len_utf8();
    }

    changed
}

/// `[text](url)` at the start of `rest`: the link text, the URL, and the bytes
/// consumed. The text holds no `]` or line break; the URL holds no whitespace
/// or `)` and must use an http, https, or mailto scheme.
fn markdown_link(rest: &str) -> Option<(&str, &str, usize)> {
    let after_open = rest.strip_prefix('[')?;
    let text_end = after_open.find(']')?;
    let text = &after_open[..text_end];
    if text.is_empty() || text.contains(['[', '\n']) {
        return None;
    }
    let after_text = after_open[text_end + 1..].strip_prefix('(')?;
    let url_end = after_text.find(')')?;
    let url = &after_text[..url_end];
    if url.is_empty() || url.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    if !["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return None;
    }
    Some((text, url, 1 + text_end + 1 + 1 + url_end + 1))
}

/// `_text_` at the start of `rest`, not inside a word: the emphasized text and
/// the bytes consumed. The text is non-empty and neither starts nor ends with
/// whitespace.
fn underscore_emphasis<'a>(before: &str, rest: &'a str) -> Option<(&'a str, usize)> {
    let after = rest.strip_prefix('_')?;
    if before
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    let mut search_from = 0;
    while let Some(offset) = after[search_from..].find('_') {
        let end = search_from + offset;
        let inner = &after[..end];
        let next = after[end + 1..].chars().next();
        if !inner.is_empty()
            && !inner.starts_with(char::is_whitespace)
            && !inner.ends_with(char::is_whitespace)
            && !inner.contains('\n')
            && !next.is_some_and(char::is_alphanumeric)
        {
            return Some((inner, 1 + end + 1));
        }
        search_from = end + 1;
    }
    None
}

fn find_unescaped_math_close(value: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '$' {
            return Some(index);
        }
    }
    None
}

fn push_math_html(output: &mut String, tag: &str, latex: &str) {
    output.push('<');
    output.push_str(tag);
    output.push_str(" data-mx-maths=\"");
    push_escaped_html(output, latex);
    output.push_str("\">");
    push_escaped_html(output, latex);
    output.push_str("</");
    output.push_str(tag);
    output.push('>');
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

#[cfg(test)]
mod tests {
    use super::*;

    fn context(autocomplete_open: bool) -> ComposerResolverContext {
        ComposerResolverContext {
            surface: ComposerSurface::Main,
            send_shortcut: ComposerSendShortcut::Enter,
            autocomplete_open,
            send_enabled: true,
        }
    }

    fn key_event(key: ComposerKey) -> ComposerKeyEvent {
        ComposerKeyEvent {
            key,
            modifiers: ComposerKeyModifiers::default(),
            is_composing: false,
            selection: None,
        }
    }

    #[test]
    fn escape_closes_open_autocomplete_before_reply_cancel() {
        assert_eq!(
            resolve_composer_key_action(key_event(ComposerKey::Escape), context(true)),
            ComposerResolvedAction::CloseAutocomplete
        );
    }

    #[test]
    fn escape_still_cancels_when_autocomplete_is_closed() {
        assert_eq!(
            resolve_composer_key_action(key_event(ComposerKey::Escape), context(false)),
            ComposerResolvedAction::Cancel
        );
    }
}
