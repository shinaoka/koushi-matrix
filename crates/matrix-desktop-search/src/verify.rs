use matrix_desktop_state::{SearchMatchField, SearchMatchKind, SearchResult, TextRange};
use thiserror::Error;

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
    let start_byte = haystack.find(needle)?;
    let end_byte = start_byte + needle.len();

    Some(TextRange {
        start_utf16: utf16_len(&haystack[..start_byte]),
        end_utf16: utf16_len(&haystack[..end_byte]),
    })
}

fn utf16_len(value: &str) -> u32 {
    value.encode_utf16().count() as u32
}
