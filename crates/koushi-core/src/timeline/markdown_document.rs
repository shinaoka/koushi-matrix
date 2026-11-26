//! Markdown attachment rendering (#2097).
//!
//! A shared `.md` file is rendered here, in Rust, to the same
//! `TimelineFormattedBody` a message carries, so the renderer draws it with the
//! message body's own HTML-to-React path. Raw HTML in the document is turned into
//! text before rendering and the result is sanitized again by the document
//! allow-list: a document can never put markup of its own on screen.

use koushi_protocol::event::TimelineFormattedBody;
use matrix_sdk::ruma::html::Html;
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, html};

use super::html_sanitize::sanitize_markdown_document_html;
use super::item_projection::{code_blocks_from_html, plain_text_from_html};

/// Id prefix of rendered headings; the table of contents links to `md-h<index>`.
const HEADING_ID_PREFIX: &str = "md-h";

/// Render the text of a shared `.md` attachment to the sanitized formatted DTO.
pub fn render_markdown_document(markdown: &str) -> TimelineFormattedBody {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;
    let mut heading_index = 0usize;
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        // Raw HTML is shown as the text it is, never emitted as markup.
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::InlineMath(latex) => Event::Html(math_markup("span", &latex)),
        Event::DisplayMath(latex) => Event::Html(math_markup("div", &latex)),
        Event::Start(Tag::Heading {
            level,
            classes,
            attrs,
            ..
        }) => {
            let id = format!("{HEADING_ID_PREFIX}{heading_index}");
            heading_index += 1;
            Event::Start(Tag::Heading {
                level,
                id: Some(CowStr::from(id)),
                classes,
                attrs,
            })
        }
        other => other,
    });
    let mut rendered = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut rendered, events);

    let sanitized = sanitize_markdown_document_html(&rendered);
    let parsed = Html::parse(&sanitized);
    TimelineFormattedBody {
        plain_text: plain_text_from_html(&parsed),
        code_blocks: code_blocks_from_html(&parsed),
        html: sanitized,
    }
}

/// The message body's math markup (`data-mx-maths`), as the composer emits it.
fn math_markup<'a>(tag: &str, latex: &str) -> CowStr<'a> {
    let mut escaped = String::with_capacity(latex.len());
    for ch in latex.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(ch),
        }
    }
    CowStr::from(format!(
        "<{tag} data-mx-maths=\"{escaped}\">{escaped}</{tag}>"
    ))
}

#[cfg(test)]
mod tests;
