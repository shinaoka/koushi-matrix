use super::*;
use koushi_protocol::event::{
    TimelineItemId, TimelineUnableToDecrypt, TimelineUnableToDecryptReason,
};

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

fn undecryptable_item(event_id: &str) -> TimelineItem {
    let mut item = original_item(event_id, "");
    item.body = None;
    item.unable_to_decrypt = Some(TimelineUnableToDecrypt {
        session_id: None,
        reason: TimelineUnableToDecryptReason::MissingRoomKey,
        can_request_keys: true,
        recovery_stage: None,
        recovery_guidance: None,
    });
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

fn ready_quote(event_id: &str, body: &str) -> ReplyQuote {
    let mut quote = placeholder_quote(event_id, ReplyQuoteState::Ready);
    quote.body_preview = Some(body.to_owned());
    quote
}

/// `start(id, token)` uses token = 1, 2, … in issuance order.
fn start(event_id: &str, token: HydrationToken) -> HydrationStep {
    HydrationStep::Start {
        event_id: event_id.to_owned(),
        token,
    }
}

fn retry(event_id: &str, token: HydrationToken, delay: Duration) -> HydrationStep {
    HydrationStep::ScheduleRetry {
        event_id: event_id.to_owned(),
        token,
        delay,
    }
}

fn no_refresh() -> HashSet<String> {
    HashSet::new()
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
    let mut pending = [reply_item("txn", ORIGINAL)];
    let targets = quote_targets(pending.iter())
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let originals = known_originals(&targets, &[], &canonical);
    let changed = overlay_reply_quotes(
        pending.iter_mut(),
        |event_id| originals.get(event_id).cloned(),
        &ReplyQuoteHydration::default(),
        &no_refresh(),
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
    let mut items = [reply_item("txn", ORIGINAL)];
    assert!(overlay_reply_quotes(
        items.iter_mut(),
        |_| None,
        &hydration,
        &no_refresh()
    ));
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
    assert_eq!(steps, vec![retry(ORIGINAL, 2, Duration::from_secs(2))]);
    assert_eq!(hydration.settled_quote(ORIGINAL), None);
    assert_eq!(hydration.retry_due(ORIGINAL, 2), vec![start(ORIGINAL, 3)]);
    let steps = hydration.complete(ORIGINAL, 3, OriginalLookupOutcome::TimedOut);
    assert_eq!(steps, vec![retry(ORIGINAL, 4, Duration::from_secs(10))]);
    assert_eq!(hydration.retry_due(ORIGINAL, 4), vec![start(ORIGINAL, 5)]);
    assert!(
        hydration
            .complete(ORIGINAL, 5, OriginalLookupOutcome::TimedOut)
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
fn undecryptable_originals_back_off_slowly_then_settle_failed() {
    let mut hydration = ReplyQuoteHydration::default();
    assert_eq!(hydration.request([ORIGINAL]), vec![start(ORIGINAL, 1)]);
    let mut in_flight = 1;
    let mut next_retry = 2;
    // Room keys arrive late: the undecryptable budget is separate from the
    // transient one, so seven retries use the slow delays before Failed.
    for seconds in [15u64, 60, 180, 300, 300, 300, 300] {
        let steps = hydration.complete(
            ORIGINAL,
            in_flight,
            OriginalLookupOutcome::Loaded(Box::new(undecryptable_item(ORIGINAL))),
        );
        assert_eq!(
            steps,
            vec![retry(ORIGINAL, next_retry, Duration::from_secs(seconds))]
        );
        assert_eq!(
            hydration.retry_due(ORIGINAL, next_retry),
            vec![start(ORIGINAL, next_retry + 1)]
        );
        in_flight = next_retry + 1;
        next_retry = in_flight + 1;
    }
    assert!(
        hydration
            .complete(
                ORIGINAL,
                in_flight,
                OriginalLookupOutcome::Loaded(Box::new(undecryptable_item(ORIGINAL))),
            )
            .is_empty()
    );
    assert_eq!(
        hydration.settled_quote(ORIGINAL).map(|quote| quote.state),
        Some(ReplyQuoteState::Failed)
    );
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
fn stale_tokens_are_fenced() {
    let mut hydration = ReplyQuoteHydration::default();
    hydration.request([ORIGINAL]);
    // Token 2 is not the live lookup token, so its result is ignored.
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
    // A retry wake for a token that is not waiting is ignored too.
    assert!(hydration.retry_due(ORIGINAL, 1).is_empty());
    // The live token still settles.
    assert!(
        hydration
            .complete(
                ORIGINAL,
                1,
                OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
            )
            .is_empty()
    );
    assert_eq!(
        hydration.settled_quote(ORIGINAL).map(|quote| quote.state),
        Some(ReplyQuoteState::Missing)
    );
}

#[test]
fn evicted_and_re_requested_original_fences_the_old_token() {
    let mut hydration = ReplyQuoteHydration::default();
    assert_eq!(hydration.request([ORIGINAL]), vec![start(ORIGINAL, 1)]);
    assert!(
        hydration
            .complete(
                ORIGINAL,
                1,
                OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
            )
            .is_empty()
    );
    // Fill the ledger so the settled entry is evicted.
    for index in 0..REPLY_QUOTE_LEDGER_MAX_ENTRIES {
        let event_id = format!("$f{index}:example.invalid");
        hydration.learn(ready_quote(&event_id, "body"));
    }
    assert!(!hydration.tracks(ORIGINAL));
    // A fresh request issues the next token; the evicted entry's token can no
    // longer settle the new lookup.
    let steps = hydration.request([ORIGINAL]);
    assert_eq!(steps, vec![start(ORIGINAL, 2)]);
    assert!(
        hydration
            .complete(
                ORIGINAL,
                1,
                OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
            )
            .is_empty()
    );
    assert_eq!(hydration.settled_quote(ORIGINAL), None);
    assert!(
        hydration
            .complete(
                ORIGINAL,
                2,
                OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
            )
            .is_empty()
    );
    assert_eq!(
        hydration.settled_quote(ORIGINAL).map(|quote| quote.state),
        Some(ReplyQuoteState::Missing)
    );
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
    assert_eq!(steps, vec![start(&ids[REPLY_QUOTE_MAX_IN_FLIGHT], 5)]);
}

#[test]
fn ledger_is_bounded_and_evicts_settled_entries_first() {
    let mut hydration = ReplyQuoteHydration::default();
    for index in 0..REPLY_QUOTE_LEDGER_MAX_ENTRIES + 10 {
        let event_id = format!("$e{index}:example.invalid");
        hydration.learn(ready_quote(&event_id, "body"));
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
fn resolved_quotes_are_not_overwritten_without_a_refresh() {
    let mut hydration = ReplyQuoteHydration::default();
    hydration.learn(ready_quote(ORIGINAL, "learned"));
    let mut item = reply_item("txn", ORIGINAL);
    item.reply_quote = Some(placeholder_quote(ORIGINAL, ReplyQuoteState::Redacted));
    assert!(!overlay_reply_quotes(
        std::iter::once(&mut item),
        |_| None,
        &hydration,
        &no_refresh()
    ));
    assert_eq!(
        item.reply_quote.map(|quote| quote.state),
        Some(ReplyQuoteState::Redacted)
    );
}

#[test]
fn changed_originals_refresh_resolved_dependent_quotes() {
    let mut hydration = ReplyQuoteHydration::default();
    hydration.learn(ready_quote(ORIGINAL, "before edit"));

    // A settled edit or redaction is reported as `updated`, so dependents
    // must be re-derived even though their quote already resolved.
    let mut redacted = placeholder_quote(ORIGINAL, ReplyQuoteState::Redacted);
    redacted.sender = Some("@alice:example.invalid".to_owned());
    let outcome = hydration.learn(redacted);
    assert!(outcome.updated);
    assert!(!outcome.superseded_task);

    let mut item = reply_item("txn", ORIGINAL);
    item.reply_quote = Some(ready_quote(ORIGINAL, "before edit"));
    let mut refresh = HashSet::new();
    refresh.insert(ORIGINAL.to_owned());
    assert!(overlay_reply_quotes(
        std::iter::once(&mut item),
        |_| None,
        &hydration,
        &refresh
    ));
    assert_eq!(
        item.reply_quote.as_ref().map(|quote| quote.state),
        Some(ReplyQuoteState::Redacted)
    );
    assert_eq!(
        item.reply_quote
            .as_ref()
            .and_then(|quote| quote.body_preview.clone()),
        None
    );
}

#[test]
fn learning_an_edit_supersedes_an_in_flight_lookup() {
    let mut hydration = ReplyQuoteHydration::default();
    assert_eq!(hydration.request([ORIGINAL]), vec![start(ORIGINAL, 1)]);
    let outcome = hydration.learn(ready_quote(ORIGINAL, "edited"));
    assert!(outcome.superseded_task);
    assert!(!outcome.updated);
    // The superseded lookup cannot overwrite the learned observation, even
    // with its original token.
    assert!(
        hydration
            .complete(
                ORIGINAL,
                1,
                OriginalLookupOutcome::Failed(OperationFailureKind::NotFound),
            )
            .is_empty()
    );
    assert_eq!(
        hydration.settled_quote(ORIGINAL).map(|quote| quote.state),
        Some(ReplyQuoteState::Ready)
    );
}
