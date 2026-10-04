//! #1110: the consumer export carries the whole row-visibility policy.
//!
//! The export used to assign `is_hidden` from the redaction preference and the
//! ignored-sender set alone. That dropped the deliberate content suppression of
//! bodyless technical state events, so they came back as blank timeline rows.
//! These probes exercise the production projection function directly.

use koushi_protocol::event::TimelineItem;
use koushi_state::AppState;

use super::project_timeline_item_display_labels;

/// Minimal `TimelineItem` fixture. Every other field has a serde default.
fn item_fixture(event_id: &str, body: Option<&str>, sender: &str) -> TimelineItem {
    serde_json::from_value(serde_json::json!({
        "id": { "Event": { "event_id": event_id } },
        "sender": sender,
        "body": body,
        "timestamp_ms": 1_800_000_000_000_u64,
        "in_reply_to_event_id": null,
    }))
    .expect("timeline item fixture")
}

fn state_with(hide_redacted: bool, ignored: &[&str]) -> AppState {
    let mut state = AppState::default();
    state.settings.values.display.hide_redacted = hide_redacted;
    state.profile.ignored_user_ids = ignored.iter().map(|id| (*id).to_owned()).collect();
    state
}

/// Probe 1: a suppressed bodyless event must stay hidden on export.
#[test]
fn bodyless_technical_event_stays_hidden_on_export() {
    let state = state_with(true, &[]);
    let mut acl = item_fixture("$acl:example.invalid", None, "@moderator:example.invalid");
    // The SDK-to-Core projection marks a bodyless technical state event hidden.
    acl.is_hidden = true;

    project_timeline_item_display_labels(&mut acl, &state);

    assert!(
        acl.is_hidden,
        "the export must preserve the content-suppression reason"
    );
}

#[test]
fn ordinary_message_stays_visible_on_export() {
    let state = state_with(true, &[]);
    let mut message = item_fixture(
        "$message:example.invalid",
        Some("Synthetic"),
        "@member:example.invalid",
    );

    project_timeline_item_display_labels(&mut message, &state);

    assert!(!message.is_hidden);
}

/// The redaction preference stays reversible without revealing anything else.
#[test]
fn redaction_preference_is_reversible_for_redacted_rows_only() {
    let mut redacted = item_fixture("$redacted:example.invalid", None, "@member:example.invalid");
    redacted.is_redacted = true;
    let visible = item_fixture(
        "$visible:example.invalid",
        Some("Synthetic"),
        "@member:example.invalid",
    );

    let mut shown = redacted.clone();
    project_timeline_item_display_labels(&mut shown, &state_with(true, &[]));
    assert!(shown.is_hidden, "hide_redacted=true hides the redacted row");

    let mut shown_again = redacted.clone();
    project_timeline_item_display_labels(&mut shown_again, &state_with(false, &[]));
    assert!(
        !shown_again.is_hidden,
        "hide_redacted=false reveals the redacted placeholder"
    );

    let mut still_visible = visible.clone();
    project_timeline_item_display_labels(&mut still_visible, &state_with(false, &[]));
    assert!(!still_visible.is_hidden);
}

/// Ignoring a sender must not depend on the previous exported value, and must
/// keep the content reason of an unrelated item.
#[test]
fn ignored_sender_reason_is_added_and_removed_without_losing_content_reason() {
    let message = item_fixture(
        "$message:example.invalid",
        Some("Synthetic"),
        "@peer:example.invalid",
    );
    let acl = item_fixture("$acl:example.invalid", None, "@peer:example.invalid");

    let mut ignored_message = message.clone();
    let mut ignored_acl = acl.clone();
    let ignored_state = state_with(true, &["@peer:example.invalid"]);
    project_timeline_item_display_labels(&mut ignored_message, &ignored_state);
    project_timeline_item_display_labels(&mut ignored_acl, &ignored_state);
    assert!(ignored_message.is_hidden, "an ignored sender is hidden");
    assert!(ignored_acl.is_hidden);

    let mut unignored_message = ignored_message.clone();
    project_timeline_item_display_labels(&mut unignored_message, &state_with(true, &[]));
    assert!(
        !unignored_message.is_hidden,
        "unignore must restore an ordinary message"
    );

    // The suppressed bodyless event keeps its content reason after unignore.
    let mut unignored_acl = ignored_acl.clone();
    project_timeline_item_display_labels(&mut unignored_acl, &state_with(true, &[]));
    assert!(
        unignored_acl.is_hidden,
        "unignore must not reveal a bodyless technical event"
    );
}
