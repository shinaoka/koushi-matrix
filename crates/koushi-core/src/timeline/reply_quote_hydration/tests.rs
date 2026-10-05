use super::*;
use koushi_protocol::event::TimelineItemId;

use crate::timeline::outbound_send::pending_send_item;

const ORIGINAL: &str = "$original:example.invalid";

fn original_item(event_id: &str, body: &str) -> TimelineItem {
    let mut item = pending_send_item("unused", body, None, None, Some("@alice:example.invalid"));
    item.id = TimelineItemId::Event {
        event_id: event_id.to_owned(),
    };
    item.sender_label = Some("Alice".to_owned());
    item
}

fn reply_item(transaction_id: &str, original: &str) -> TimelineItem {
    pending_send_item(
        transaction_id,
        "reply",
        Some(original.to_owned()),
        None,
        Some("@bob:example.invalid"),
    )
}

fn start(event_id: &str, attempt: u32) -> HydrationStep {
    HydrationStep::Start {
        event_id: event_id.to_owned(),
        attempt,
    }
}

#[test]
fn pending_reply_starts_loading_not_missing() {
    let item = reply_item("txn", ORIGINAL);
    let quote = item.reply_quote.expect("pending reply carries a quote");
    assert_eq!(quote.event_id, ORIGINAL);
    assert_eq!(quote.state, ReplyQuoteState::Loading);
}

#[test]
fn known_original_resolves_pending_reply_immediately() {
    let canonical = vec![original_item(ORIGINAL, "hello world")];
    let mut pending = vec![reply_item("txn", ORIGINAL)];
    let targets = unresolved_targets(pending.iter());
    let originals = known_originals(&targets, &[], &canonical);
    let changed = overlay_reply_quotes(
        pending.iter_mut(),
        |event_id| originals.get(event_id).cloned(),
        &ReplyQuoteHydration::default(),
    );
    assert!(changed);
    let quote = pending[0].reply_quote.as_ref().expect("quote");
    assert_eq!(quote.state, ReplyQuoteState::Ready);
    assert_eq!(quote.body_preview.as_deref(), Some("hello world"));
    assert_eq!(quote.sender.as_deref(), Some("@alice:example.invalid"));
    assert_eq!(quote.sender_label.as_deref(), Some("Alice"));
}

#[test]
fn redacted_original_projects_redacted_quote() {
    let mut original = original_item(ORIGINAL, "gone");
    original.is_redacted = true;
    let quote = reply_quote_from_timeline_item(ORIGINAL, &original).expect("quote");
    assert_eq!(quote.state, ReplyQuoteState::Redacted);
    assert_eq!(quote.body_preview, None);
}

#[test]
fn settled_ledger_entry_resolves_reply_without_canonical_original() {
    let mut hydration = ReplyQuoteHydration::default();
    assert_eq!(hydration.request([ORIGINAL]), vec![start(ORIGINAL, 1)]);
    let steps = hydration.complete(
        ORIGINAL,
        1,
        OriginalLookupOutcome::Loaded(Box::new(original_item(ORIGINAL, "root body"))),
    );
    assert!(steps.is_empty());
    let mut items = vec![reply_item("txn", ORIGINAL)];
    assert!(overlay_reply_quotes(items.iter_mut(), |_| None, &hydration));
    assert_eq!(
        items[0].reply_quote.as_ref().map(|quote| quote.state),
        Some(ReplyQuoteState::Ready)
    );
}

#[test]
fn transient_failures_retry_with_backoff_then_settle_failed() {
    let mut hydration = ReplyQuoteHydration::default();
    hydration.request([ORIGINAL]);
    let steps = hydration.complete(
        ORIGINAL,
        1,
        OriginalLookupOutcome::Failed(OperationFailureKind::Network),
    );
    assert_eq!(
        steps,
        vec![HydrationStep::ScheduleRetry {
            event_id: ORIGINAL.to_owned(),
            attempt: 2,
            delay: Duration::from_secs(2),
        }]
    );
    assert_eq!(hydration.settled_quote(ORIGINAL), None);
    assert_eq!(hydration.retry_due(ORIGINAL, 2), vec![start(ORIGINAL, 2)]);
    let steps = hydration.complete(ORIGINAL, 2, OriginalLookupOutcome::TimedOut);
    assert_eq!(
        steps,
        vec![HydrationStep::ScheduleRetry {
            event_id: ORIGINAL.to_owned(),
            attempt: 3,
            delay: Duration::from_secs(10),
        }]
    );
    assert_eq!(hydration.retry_due(ORIGINAL, 3), vec![start(ORIGINAL, 3)]);
    assert!(
        hydration
            .complete(ORIGINAL, 3, OriginalLookupOutcome::TimedOut)
            .is_empty()
    );
    assert_eq!(
        hydration.settled_quote(ORIGINAL).map(|quote| quote.state),
        Some(ReplyQuoteState::Failed)
    );
    // Exhausted entries are not looked up again by this actor.
    assert!(hydration.request([ORIGINAL]).is_empty());
}

#[test]
fn not_found_and_forbidden_settle_missing_without_retry() {
    for kind in [
        OperationFailureKind::NotFound,
        OperationFailureKind::Forbidden,
    ] {
        let mut hydration = ReplyQuoteHydration::default();
        hydration.request([ORIGINAL]);
        assert!(
            hydration
                .complete(ORIGINAL, 1, OriginalLookupOutcome::Failed(kind))
                .is_empty()
        );
        assert_eq!(
            hydration.settled_quote(ORIGINAL).map(|quote| quote.state),
            Some(ReplyQuoteState::Missing)
        );
    }
}

#[test]
fn stale_attempt_results_are_fenced() {
    let mut hydration = ReplyQuoteHydration::default();
    hydration.request([ORIGINAL]);
    assert!(
        hydration
            .complete(
                ORIGINAL,
                2,
                OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
            )
            .is_empty()
    );
    assert_eq!(hydration.settled_quote(ORIGINAL), None);
    // A retry wake for an attempt that is not waiting is ignored too.
    assert!(hydration.retry_due(ORIGINAL, 1).is_empty());
}

#[test]
fn in_flight_lookups_are_bounded_and_queued_lookups_start_on_completion() {
    let ids = (0..REPLY_QUOTE_MAX_IN_FLIGHT + 2)
        .map(|index| format!("$e{index}:example.invalid"))
        .collect::<Vec<_>>();
    let mut hydration = ReplyQuoteHydration::default();
    let steps = hydration.request(ids.iter().map(String::as_str));
    assert_eq!(steps.len(), REPLY_QUOTE_MAX_IN_FLIGHT);
    let steps = hydration.complete(
        &ids[0],
        1,
        OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
    );
    assert_eq!(steps, vec![start(&ids[REPLY_QUOTE_MAX_IN_FLIGHT], 1)]);
}

#[test]
fn ledger_is_bounded_and_evicts_settled_entries_first() {
    let mut hydration = ReplyQuoteHydration::default();
    for index in 0..REPLY_QUOTE_LEDGER_MAX_ENTRIES + 10 {
        let event_id = format!("$e{index}:example.invalid");
        let mut quote = placeholder_quote(&event_id, ReplyQuoteState::Ready);
        quote.body_preview = Some("body".to_owned());
        hydration.learn(quote);
    }
    assert_eq!(hydration.len(), REPLY_QUOTE_LEDGER_MAX_ENTRIES);
    assert!(hydration.settled_quote("$e0:example.invalid").is_none());
    assert!(
        hydration
            .settled_quote(&format!(
                "$e{}:example.invalid",
                REPLY_QUOTE_LEDGER_MAX_ENTRIES + 9
            ))
            .is_some()
    );
}

#[test]
fn resolved_quotes_are_not_overwritten() {
    let mut hydration = ReplyQuoteHydration::default();
    hydration.learn({
        let mut quote = placeholder_quote(ORIGINAL, ReplyQuoteState::Ready);
        quote.body_preview = Some("learned".to_owned());
        quote
    });
    let mut item = reply_item("txn", ORIGINAL);
    item.reply_quote = Some(placeholder_quote(ORIGINAL, ReplyQuoteState::Redacted));
    assert!(!overlay_reply_quotes(
        std::iter::once(&mut item),
        |_| None,
        &hydration
    ));
    assert_eq!(
        item.reply_quote.map(|quote| quote.state),
        Some(ReplyQuoteState::Redacted)
    );
}
