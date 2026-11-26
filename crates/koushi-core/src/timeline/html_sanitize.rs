//! Matrix `formatted_body` sanitizing shared by the timeline projection and
//! the history export pages.

use matrix_sdk::ruma::html::{Html, ListBehavior, PropertiesNames, SanitizerConfig};

/// Elements removed with their content in every sanitized body.
const REMOVED_ELEMENTS: [&str; 2] = ["script", "style"];

/// Sanitize with ruma's compatibility allow-list, dropping reply fallbacks,
/// scripts and styles. `extra_removed` names further elements to drop with
/// their content.
pub(crate) fn sanitize_matrix_html(body: &str, extra_removed: &[&'static str]) -> String {
    let html = Html::parse(body);
    html.sanitize_with(
        &SanitizerConfig::compat()
            .remove_reply_fallback()
            .remove_elements(REMOVED_ELEMENTS.iter().chain(extra_removed).copied()),
    );
    html.to_string()
}

/// Heading levels the document view's table of contents links to (#2097).
const DOCUMENT_HEADING_ELEMENTS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

/// Sanitize the HTML of a rendered Markdown attachment (#2097).
///
/// This is the message path's allow-list plus exactly one attribute, `id` on
/// headings, so the table of contents can link to them; the Matrix element list
/// already covers tables. Images are dropped (an `mxc` image is not the
/// document's to fetch). The message path above is unchanged.
pub(crate) fn sanitize_markdown_document_html(body: &str) -> String {
    let html = Html::parse(body);
    html.sanitize_with(
        &SanitizerConfig::compat()
            .remove_reply_fallback()
            .remove_elements(REMOVED_ELEMENTS.iter().chain(&["img"]).copied())
            .allow_attributes(
                DOCUMENT_HEADING_ELEMENTS.map(|parent| PropertiesNames {
                    parent,
                    properties: &["id"],
                }),
                ListBehavior::Add,
            ),
    );
    html.to_string()
}
