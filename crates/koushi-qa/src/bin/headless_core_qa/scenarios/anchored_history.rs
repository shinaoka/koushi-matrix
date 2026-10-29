//! #1668: an event jump anchors the main pane on a `Focused` timeline whose
//! `/context` page ends before the present. Forward pagination of that key is
//! the only way to reach newer history, so it must extend the focused
//! projection until the SDK reports the live end.

use koushi_protocol::CoreEvent;
use koushi_protocol::event::{PaginationState, TimelineDiff, TimelineEvent};

use super::event_wait::wait_for_send_completed;
use super::registry::EVENT_TIMEOUT;
use super::{
    ComposerDocument, CoreCommand, CoreConnection, PaginationDirection, TimelineCommand,
    TimelineKey, TimelineKind,
};

/// More newer events than one SDK `/context` page (20 events around the
/// target) can hold, so the focused timeline cannot start at the live end.
const NEWER_HISTORY_MESSAGES: usize = 30;
const FORWARD_PAGE_EVENTS: u16 = 20;
const MAX_FORWARD_PAGES: usize = 10;

/// Sends newer history after the jump target and returns the newest body.
pub(super) async fn seed_newer_history(
    conn: &mut CoreConnection,
    key: &TimelineKey,
) -> Result<String, String> {
    let mut newest = String::new();
    for index in 0..NEWER_HISTORY_MESSAGES {
        let body = format!("Anchored forward QA message {index}");
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id,
            key: key.clone(),
            transaction_id: format!("qa-anchored-forward-txn-{index}"),
            document: ComposerDocument::from_plain_text(body.clone()),
        }))
        .await
        .map_err(|e| format!("submit anchored forward message {index}: {e}"))?;
        wait_for_send_completed(conn, request_id, key, "anchored forward message").await?;
        newest = body;
    }
    Ok(newest)
}

fn diff_contains_body(diff: &TimelineDiff, body: &str) -> bool {
    let has = |item: &koushi_protocol::event::TimelineItem| item.body.as_deref() == Some(body);
    match diff {
        TimelineDiff::PushBack { item }
        | TimelineDiff::PushFront { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => has(item),
        TimelineDiff::Reset { items } => items.iter().any(has),
        _ => false,
    }
}

/// Paginates the anchored focused timeline forward until `EndReached` and
/// requires the newest message to enter its projection on the way.
pub(super) async fn paginate_anchored_forward_to_live_end(
    conn: &mut CoreConnection,
    anchored_key: &TimelineKey,
    newest_body: &str,
) -> Result<usize, String> {
    if !matches!(anchored_key.kind, TimelineKind::Focused { .. }) {
        return Err("anchored forward pagination needs a focused key".to_owned());
    }
    let mut newest_projected = false;
    for page in 1..=MAX_FORWARD_PAGES {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: anchored_key.clone(),
            direction: PaginationDirection::Forward,
            event_count: FORWARD_PAGE_EVENTS,
        }))
        .await
        .map_err(|e| format!("submit anchored forward page {page}: {e}"))?;
        let state = tokio::time::timeout(EVENT_TIMEOUT, async {
            loop {
                match conn.recv_event().await {
                    Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. }))
                        if &key == anchored_key =>
                    {
                        newest_projected |= diffs
                            .iter()
                            .any(|diff| diff_contains_body(diff, newest_body));
                    }
                    Ok(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                        request_id: Some(id),
                        key,
                        direction: PaginationDirection::Forward,
                        state,
                        ..
                    })) if id == request_id
                        && &key == anchored_key
                        && state != PaginationState::Paginating =>
                    {
                        return Ok(state);
                    }
                    Ok(CoreEvent::OperationFailed {
                        request_id: id,
                        failure,
                    }) if id == request_id => {
                        return Err(format!("anchored forward page {page} failed: {failure:?}"));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        return Err(format!("anchored forward page {page}: stream: {error:?}"));
                    }
                }
            }
        })
        .await
        .map_err(|_| format!("anchored forward page {page}: timed out"))??;
        match state {
            PaginationState::EndReached if newest_projected => return Ok(page),
            PaginationState::EndReached => {
                return Err(
                    "anchored forward pagination reached the end without the newest message"
                        .to_owned(),
                );
            }
            PaginationState::Idle => {}
            other => return Err(format!("anchored forward page {page}: {other:?}")),
        }
    }
    Err(format!(
        "anchored forward pagination did not reach the end in {MAX_FORWARD_PAGES} pages"
    ))
}
