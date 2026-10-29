//! #1668: an event jump anchors the main pane on a `Focused` timeline whose
//! `/context` page ends before the present. Forward pagination of that key is
//! the only way to reach newer history, so it must extend the focused
//! projection until the SDK reports the live end.
//!
//! #1675: the same key reaches older history around the jump target through
//! backward pagination.

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
const PAGE_EVENTS: u16 = 20;
const MAX_PAGES: usize = 10;
/// The backward jump target: its `/context` page holds about ten earlier
/// events, so the first seeded message is out of reach without paginating.
pub(super) const BACKWARD_ANCHOR_INDEX: usize = 25;

/// One seeded message: its body and its event id.
pub(super) struct SeededMessage {
    pub(super) body: String,
    pub(super) event_id: String,
}

/// Sends newer history after the jump target, oldest first.
pub(super) async fn seed_newer_history(
    conn: &mut CoreConnection,
    key: &TimelineKey,
) -> Result<Vec<SeededMessage>, String> {
    let mut seeded = Vec::with_capacity(NEWER_HISTORY_MESSAGES);
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
        let (_, event_id) =
            wait_for_send_completed(conn, request_id, key, "anchored forward message").await?;
        seeded.push(SeededMessage { body, event_id });
    }
    Ok(seeded)
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

/// Whether a diff adds a row carrying `body`. A backward page proves itself
/// only by adding the row, never by a reset that could replay the initial
/// `/context` page.
fn diff_adds_body(diff: &TimelineDiff, body: &str) -> bool {
    match diff {
        TimelineDiff::PushFront { item } | TimelineDiff::Insert { item, .. } => {
            item.body.as_deref() == Some(body)
        }
        _ => false,
    }
}

/// Paginates the anchored focused timeline backward until a page adds the
/// row carrying `oldest_body`, which its `/context` page does not hold.
pub(super) async fn paginate_anchored_backward_to(
    conn: &mut CoreConnection,
    anchored_key: &TimelineKey,
    oldest_body: &str,
) -> Result<usize, String> {
    paginate_anchored(
        conn,
        anchored_key,
        PaginationDirection::Backward,
        oldest_body,
    )
    .await
}

/// Paginates the anchored focused timeline forward until `EndReached` and
/// requires the newest message to enter its projection on the way.
pub(super) async fn paginate_anchored_forward_to_live_end(
    conn: &mut CoreConnection,
    anchored_key: &TimelineKey,
    newest_body: &str,
) -> Result<usize, String> {
    paginate_anchored(
        conn,
        anchored_key,
        PaginationDirection::Forward,
        newest_body,
    )
    .await
}

/// Forward pages run to `EndReached` and need `body` projected on the way;
/// backward pages stop once a page adds `body`, or fail at the room start
/// without it.
async fn paginate_anchored(
    conn: &mut CoreConnection,
    anchored_key: &TimelineKey,
    direction: PaginationDirection,
    body: &str,
) -> Result<usize, String> {
    let label = match direction {
        PaginationDirection::Forward => "forward",
        PaginationDirection::Backward => "backward",
    };
    if !matches!(anchored_key.kind, TimelineKind::Focused { .. }) {
        return Err(format!("anchored {label} pagination needs a focused key"));
    }
    let projected = |diff: &TimelineDiff| match direction {
        PaginationDirection::Forward => diff_contains_body(diff, body),
        PaginationDirection::Backward => diff_adds_body(diff, body),
    };
    let mut body_projected = false;
    let mut skipped_pages = 0;
    let mut released_waits = 0;
    for page in 1..=MAX_PAGES {
        let mut paginating = false;
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: anchored_key.clone(),
            direction,
            event_count: PAGE_EVENTS,
        }))
        .await
        .map_err(|e| format!("submit anchored {label} page {page}: {e}"))?;
        let state = tokio::time::timeout(EVENT_TIMEOUT, async {
            loop {
                match conn.recv_event().await {
                    Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. }))
                        if &key == anchored_key =>
                    {
                        body_projected |= diffs.iter().any(projected);
                    }
                    Ok(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                        request_id: Some(id),
                        key,
                        direction: event_direction,
                        state,
                        ..
                    })) if id == request_id
                        && &key == anchored_key
                        && event_direction == direction =>
                    {
                        if state == PaginationState::Paginating {
                            paginating = true;
                        } else {
                            return Ok(state);
                        }
                    }
                    Ok(CoreEvent::OperationFailed {
                        request_id: id,
                        failure,
                    }) if id == request_id => {
                        return Err(format!("anchored {label} page {page} failed: {failure:?}"));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        return Err(format!("anchored {label} page {page}: stream: {error:?}"));
                    }
                }
            }
        })
        .await
        .map_err(|_| format!("anchored {label} page {page}: timed out"))??;
        match state {
            PaginationState::EndReached => {
                // The final page's diffs may follow the terminal event.
                if !body_projected {
                    wait_for_projection(conn, anchored_key, &projected, label).await?;
                }
                println!("anchored_{label}_declined_pages={skipped_pages}");
                return Ok(page);
            }
            PaginationState::Idle
                if paginating && direction == PaginationDirection::Backward && body_projected =>
            {
                println!("anchored_{label}_declined_pages={skipped_pages}");
                return Ok(page);
            }
            // An Idle terminal without Paginating is Core declining the page
            // while gap repair owns the timeline; retry after its release.
            PaginationState::Idle if !paginating => {
                skipped_pages += 1;
                if wait_for_gap_repair_release(conn, anchored_key, &projected, &mut body_projected)
                    .await
                {
                    released_waits += 1;
                }
            }
            PaginationState::Idle => {}
            other => return Err(format!("anchored {label} page {page}: {other:?}")),
        }
    }
    Err(format!(
        "anchored {label} pagination did not finish in {MAX_PAGES} pages \
         (skipped={skipped_pages} released={released_waits} projected={body_projected})"
    ))
}

async fn wait_for_projection(
    conn: &mut CoreConnection,
    anchored_key: &TimelineKey,
    projected: &impl Fn(&TimelineDiff) -> bool,
    label: &str,
) -> Result<(), String> {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            match conn.recv_event().await {
                Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. }))
                    if &key == anchored_key && diffs.iter().any(projected) =>
                {
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) => return Err(format!("anchored {label} end: stream: {error:?}")),
            }
        }
    })
    .await
    .map_err(|_| format!("anchored {label} pagination reached the end without its message"))?
}

/// Returns whether gap repair reported its release before the timeout. An
/// earlier accepted page's diffs can arrive meanwhile; they still count.
async fn wait_for_gap_repair_release(
    conn: &mut CoreConnection,
    anchored_key: &TimelineKey,
    projected: &impl Fn(&TimelineDiff) -> bool,
    body_projected: &mut bool,
) -> bool {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        loop {
            match conn.recv_event().await {
                Ok(CoreEvent::Timeline(TimelineEvent::GapRepairReleased { key, .. }))
                    if &key == anchored_key =>
                {
                    return true;
                }
                Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. }))
                    if &key == anchored_key =>
                {
                    *body_projected |= diffs.iter().any(projected);
                }
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}
