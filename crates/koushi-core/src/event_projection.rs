use koushi_protocol::event::{
    RoomEvent, TimelineDiff, TimelineDisplayLabelUpdate, TimelineEvent, TimelineItem,
    TimelineItemId, TimelineMessageActions, TimelineMessageSource,
};
use koushi_state::{
    AppState, ProfileState, SessionState, resolve_optional_user_display_name,
    resolve_user_display_name,
};

pub(crate) fn timeline_projection_own_user_id(state: &AppState) -> Option<&str> {
    match &state.session {
        SessionState::Ready(info) => Some(info.user_id.as_str()),
        _ => None,
    }
}

pub(crate) fn project_room_event_display_labels(event: &mut RoomEvent, state: &AppState) {
    match event {
        RoomEvent::RoomSettingsLoaded { settings, .. }
        | RoomEvent::RoomSettingUpdated { settings, .. } => {
            koushi_state::refresh_room_settings_member_display_projection(
                settings,
                &state.profile,
                timeline_projection_own_user_id(state),
            );
        }
        _ => {}
    }
}

pub(crate) fn project_timeline_event_display_labels(event: &mut TimelineEvent, state: &AppState) {
    match event {
        TimelineEvent::InitialItems { items, .. } => {
            for item in items {
                project_timeline_item_display_labels(item, state);
            }
        }
        TimelineEvent::ItemsUpdated { diffs, .. } => {
            for diff in diffs {
                project_timeline_diff_display_labels(diff, state);
            }
        }
        _ => {}
    }
}

pub(crate) fn project_timeline_item_display_labels(item: &mut TimelineItem, state: &AppState) {
    item.sender_label =
        timeline_sender_label(item.sender.as_deref(), item.sender_label.as_deref(), state);
    item.is_hidden = (state.settings.values.display.hide_redacted && item.is_redacted)
        || koushi_state::is_ignored_user(&state.profile, item.sender.as_deref());
    if let Some(reply_quote) = item.reply_quote.as_mut() {
        reply_quote.sender_label = timeline_sender_label(
            reply_quote.sender.as_deref(),
            reply_quote.sender_label.as_deref(),
            state,
        );
    }
    if let Some(thread_summary) = item.thread_summary.as_mut() {
        thread_summary.latest_sender_label = timeline_sender_label(
            thread_summary.latest_sender.as_deref(),
            thread_summary.latest_sender_label.as_deref(),
            state,
        );
    }
    for reaction in &mut item.reactions {
        for sender in &mut reaction.sender_preview {
            sender.display_label = timeline_sender_label(
                Some(sender.user_id.as_str()),
                sender.display_label.as_deref(),
                state,
            );
        }
    }
}

fn project_timeline_diff_display_labels(diff: &mut TimelineDiff, state: &AppState) {
    match diff {
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => project_timeline_item_display_labels(item, state),
        TimelineDiff::Reset { items } => {
            for item in items {
                project_timeline_item_display_labels(item, state);
            }
        }
        TimelineDiff::Remove { .. } | TimelineDiff::Truncate { .. } | TimelineDiff::Clear => {}
    }
}

fn timeline_sender_label(
    sender: Option<&str>,
    upstream_display_label: Option<&str>,
    state: &AppState,
) -> Option<String> {
    let sender = sender?;
    resolve_optional_user_display_name(
        &state.profile,
        sender,
        upstream_display_label,
        timeline_projection_own_user_id(state),
    )
}

pub(crate) fn derive_display_label_updates_for_user_ids<'a>(
    profile: &ProfileState,
    own_user_id: Option<&str>,
    user_ids: impl IntoIterator<Item = &'a str>,
) -> Vec<TimelineDisplayLabelUpdate> {
    let mut seen = std::collections::BTreeSet::new();
    let mut updates = Vec::new();
    let mut push = |user_id: &str| {
        if !seen.insert(user_id.to_owned()) {
            return;
        }
        updates.push(TimelineDisplayLabelUpdate {
            user_id: user_id.to_owned(),
            display_label: resolve_user_display_name(profile, user_id, None, own_user_id),
        });
    };
    for uid in user_ids {
        push(uid);
    }
    updates
}

pub(crate) fn message_actions_for_timeline_item(
    room_id: &str,
    item_id: &TimelineItemId,
    body: Option<&str>,
    has_media: bool,
    is_redacted: bool,
) -> TimelineMessageActions {
    let TimelineItemId::Event { event_id } = item_id else {
        return TimelineMessageActions::default();
    };
    let has_body = body.is_some_and(|body| !body.is_empty());
    let permalink = matrix_to_event_permalink(room_id, event_id);
    TimelineMessageActions {
        can_copy: has_body && !is_redacted,
        can_forward: has_body && !is_redacted,
        can_reply: !is_redacted && !event_id.trim().is_empty() && (body.is_some() || has_media),
        can_permalink: permalink.is_some(),
        can_view_source: !event_id.trim().is_empty(),
        permalink,
        editable_document: None,
    }
}

pub(crate) fn matrix_to_event_permalink(room_id: &str, event_id: &str) -> Option<String> {
    if room_id.trim().is_empty() || event_id.trim().is_empty() {
        return None;
    }
    Some(format!(
        "https://matrix.to/#/{}/{}",
        percent_encode_matrix_to_component(room_id),
        percent_encode_matrix_to_component(event_id),
    ))
}

fn percent_encode_matrix_to_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'!') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => unreachable!(),
    }
}

pub(crate) fn message_source_for_timeline_item(
    item: &TimelineItem,
) -> Option<TimelineMessageSource> {
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    Some(TimelineMessageSource {
        event_id: event_id.clone(),
        sender: item.sender.clone(),
        timestamp_ms: item.timestamp_ms,
        body: item.body.clone(),
        in_reply_to_event_id: item.in_reply_to_event_id.clone(),
        thread_root: item.thread_root.clone(),
        is_redacted: item.is_redacted,
        is_edited: item.is_edited,
        has_media: item.media.is_some(),
        megolm_session_fingerprint: None,
        megolm_message_index: None,
        megolm_session_rotation_reason: None,
        original_json: None,
    })
}

#[cfg(test)]
mod tests;
