use matrix_desktop_state::{
    SearchMatchField, SearchMatchKind, SearchResult, TextRange, normalize_cjk_search_text,
};
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

use crate::{SearchCandidate, SearchableEvent};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SearchVerificationError {
    #[error("candidate event is not available")]
    MissingCandidate,
}

pub fn verify_candidate(
    candidate: &SearchCandidate,
    event: &SearchableEvent,
    query: &str,
) -> Option<SearchResult> {
    if query.is_empty() || candidate.room_id != event.room_id {
        return None;
    }

    if let Some(body) = &event.body
        && let Some(highlight) = exact_range(body.as_str(), query)
    {
        return Some(result(
            candidate,
            event,
            body.as_str(),
            SearchMatchField::MessageBody,
            highlight,
        ));
    }

    if let Some(filename) = &event.attachment_filename
        && let Some(highlight) = exact_range(filename.as_str(), query)
    {
        return Some(result(
            candidate,
            event,
            filename.as_str(),
            SearchMatchField::AttachmentFileName,
            highlight,
        ));
    }

    None
}

fn result(
    candidate: &SearchCandidate,
    event: &SearchableEvent,
    snippet: &str,
    match_field: SearchMatchField,
    highlight: TextRange,
) -> SearchResult {
    SearchResult {
        room_id: event.room_id.clone(),
        event_id: event.event_id.clone(),
        sender: event.sender.clone(),
        timestamp_ms: event.timestamp_ms,
        score_millis: candidate.score_millis,
        snippet: snippet.to_owned(),
        match_field,
        highlights: vec![highlight],
        match_kind: SearchMatchKind::Exact,
    }
}

fn exact_range(haystack: &str, needle: &str) -> Option<TextRange> {
    direct_exact_range(haystack, needle).or_else(|| normalized_exact_range(haystack, needle))
}

fn direct_exact_range(haystack: &str, needle: &str) -> Option<TextRange> {
    let start_byte = haystack.find(needle)?;
    let end_byte = start_byte + needle.len();

    Some(TextRange {
        start_utf16: utf16_len(&haystack[..start_byte]),
        end_utf16: utf16_len(&haystack[..end_byte]),
    })
}

fn normalized_exact_range(haystack: &str, needle: &str) -> Option<TextRange> {
    let normalized_needle = normalize_cjk_search_text(needle);
    if normalized_needle.is_empty() {
        return None;
    }

    let normalized_haystack = NormalizedHaystack::from_original(haystack);
    let start_byte = normalized_haystack.text.find(&normalized_needle)?;
    let end_byte = start_byte + normalized_needle.len();
    let start_char = normalized_haystack.text[..start_byte].chars().count();
    let end_char = normalized_haystack.text[..end_byte].chars().count();
    if start_char >= end_char || end_char > normalized_haystack.source_ranges.len() {
        return None;
    }

    let source_start_byte = normalized_haystack.source_ranges[start_char].0;
    let source_end_byte = normalized_haystack.source_ranges[end_char - 1].1;

    Some(TextRange {
        start_utf16: utf16_len(&haystack[..source_start_byte]),
        end_utf16: utf16_len(&haystack[..source_end_byte]),
    })
}

struct NormalizedHaystack {
    text: String,
    source_ranges: Vec<(usize, usize)>,
}

impl NormalizedHaystack {
    fn from_original(value: &str) -> Self {
        let mut text = String::new();
        let mut source_ranges = Vec::new();

        for (start, grapheme) in value.grapheme_indices(true) {
            let end = start + grapheme.len();
            let normalized = normalize_cjk_search_text(grapheme);
            for normalized_ch in normalized.chars() {
                text.push(normalized_ch);
                source_ranges.push((start, end));
            }
        }

        Self {
            text,
            source_ranges,
        }
    }
}

fn utf16_len(value: &str) -> u32 {
    value.encode_utf16().count() as u32
}
