use std::fmt;

use koushi_sdk::{MatrixClientSession, MatrixTimelineError, MatrixTimelineItem};
use koushi_state::{ActivityRow, OperationFailureKind};

use crate::messages_backpressure::MessagesBackpressure;

const PAGE_SIZE: u16 = 50;
const MAX_PAGES: usize = 32;
const MAX_ATTEMPTS: usize = 3;

#[derive(Default)]
pub(crate) struct ActivityResolutionOutcome {
    pub(crate) rows: Vec<ActivityRow>,
    pub(crate) unresolved_room_count: u32,
    pub(crate) failure_kind: Option<OperationFailureKind>,
}

impl ActivityResolutionOutcome {
    fn record(&mut self, result: Result<Vec<ActivityRow>, OperationFailureKind>) {
        match result {
            Ok(rows) => self.rows.extend(rows),
            Err(kind) => {
                self.unresolved_room_count = self.unresolved_room_count.saturating_add(1);
                self.failure_kind.get_or_insert(kind);
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ActivityResolutionRequest {
    pub(crate) room_id: String,
    pub(crate) fully_read_event_id: Option<String>,
    pub(crate) minimum_unread_count: u64,
}

impl fmt::Debug for ActivityResolutionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivityResolutionRequest")
            .field("room_id", &"RoomId(..)")
            .field(
                "fully_read_event_id",
                &self.fully_read_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field("minimum_unread_count", &self.minimum_unread_count)
            .finish()
    }
}

pub(crate) async fn resolve_activity_requests(
    session: &MatrixClientSession,
    requests: &[ActivityResolutionRequest],
    backpressure: &MessagesBackpressure,
) -> ActivityResolutionOutcome {
    let mut outcome = ActivityResolutionOutcome::default();
    for request in requests {
        outcome.record(resolve_activity_request(session, request, backpressure).await);
    }
    outcome
}

async fn resolve_activity_request(
    session: &MatrixClientSession,
    request: &ActivityResolutionRequest,
    backpressure: &MessagesBackpressure,
) -> Result<Vec<ActivityRow>, OperationFailureKind> {
    let mut subscription = None;
    for _ in 0..MAX_ATTEMPTS {
        match koushi_sdk::subscribe_room_timeline(session, &request.room_id).await {
            Ok(value) => {
                subscription = Some(value);
                break;
            }
            Err(MatrixTimelineError::InvalidRoomId | MatrixTimelineError::RoomUnavailable) => {
                return Err(OperationFailureKind::NotFound);
            }
            Err(MatrixTimelineError::Sdk) => {}
        }
    }
    let mut subscription = subscription.ok_or(OperationFailureKind::Sdk)?;
    let pagination = subscription.pagination_handle();
    let mut reached_start = false;
    for page in 0..=MAX_PAGES {
        let items = subscription.current_items().await;
        if let Some(selected) = select_unread_items(&items, request, reached_start) {
            return Ok(selected.into_iter().map(activity_row).collect());
        }
        if reached_start {
            return Ok(Vec::new());
        }
        if page == MAX_PAGES {
            return Err(OperationFailureKind::Timeout);
        }

        let mut page_result = None;
        for _ in 0..MAX_ATTEMPTS {
            let permit = backpressure.acquire_timeline().await;
            let result = pagination.paginate_backwards(PAGE_SIZE).await;
            drop(permit);
            match result {
                Ok(value) => {
                    page_result = Some(value);
                    break;
                }
                Err(MatrixTimelineError::InvalidRoomId | MatrixTimelineError::RoomUnavailable) => {
                    return Err(OperationFailureKind::NotFound);
                }
                Err(MatrixTimelineError::Sdk) => {}
            }
        }
        reached_start = page_result.ok_or(OperationFailureKind::Network)?;
    }
    Err(OperationFailureKind::Timeout)
}

fn select_unread_items<'a>(
    items: &'a [MatrixTimelineItem],
    request: &ActivityResolutionRequest,
    reached_start: bool,
) -> Option<Vec<&'a MatrixTimelineItem>> {
    let mut ordered = items.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.timestamp_ms
            .cmp(&right.timestamp_ms)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    if let Some(marker) = request.fully_read_event_id.as_deref() {
        if let Some(index) = ordered.iter().position(|item| item.event_id == marker) {
            return Some(ordered.into_iter().skip(index + 1).collect());
        }
        return reached_start.then(|| newest_items(ordered, request.minimum_unread_count));
    }
    let enough = ordered.len() as u64 >= request.minimum_unread_count.max(1);
    (enough || reached_start).then(|| newest_items(ordered, request.minimum_unread_count))
}

fn newest_items(
    ordered: Vec<&MatrixTimelineItem>,
    minimum_unread_count: u64,
) -> Vec<&MatrixTimelineItem> {
    let count = usize::try_from(minimum_unread_count.max(1)).unwrap_or(usize::MAX);
    let skip = ordered.len().saturating_sub(count);
    ordered.into_iter().skip(skip).collect()
}

fn activity_row(item: &MatrixTimelineItem) -> ActivityRow {
    ActivityRow::event(
        item.room_id.clone(),
        item.event_id.clone(),
        Some(item.sender.clone()),
        String::new(),
        Some(item.sender.clone()),
        Some(item.body.clone()),
        item.timestamp_ms,
        false,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(event_id: &str, timestamp_ms: u64) -> MatrixTimelineItem {
        MatrixTimelineItem {
            room_id: "!private:example.invalid".to_owned(),
            event_id: event_id.to_owned(),
            sender: "@private:example.invalid".to_owned(),
            timestamp_ms,
            body: "private body".to_owned(),
        }
    }

    #[test]
    fn marker_selects_only_later_rows_and_debug_is_redacted() {
        let items = vec![item("$later", 30), item("$marker", 20), item("$older", 10)];
        let request = ActivityResolutionRequest {
            room_id: "!private:example.invalid".to_owned(),
            fully_read_event_id: Some("$marker".to_owned()),
            minimum_unread_count: 1,
        };
        let selected = select_unread_items(&items, &request, false).expect("marker resolved");
        assert_eq!(
            selected
                .iter()
                .map(|item| item.event_id.as_str())
                .collect::<Vec<_>>(),
            ["$later"]
        );
        assert!(!format!("{request:?}").contains("private"));
    }

    #[test]
    fn missing_marker_waits_for_start_then_uses_notification_bound() {
        let items = vec![item("$new", 30), item("$old", 10)];
        let request = ActivityResolutionRequest {
            room_id: "!private:example.invalid".to_owned(),
            fully_read_event_id: Some("$missing".to_owned()),
            minimum_unread_count: 1,
        };
        assert!(select_unread_items(&items, &request, false).is_none());
        let selected = select_unread_items(&items, &request, true).expect("start is authoritative");
        assert_eq!(selected[0].event_id, "$new");
    }

    #[test]
    fn partial_resolution_keeps_successful_rows_and_failure_count() {
        let mut outcome = ActivityResolutionOutcome::default();
        outcome.record(Ok(vec![activity_row(&item("$resolved", 10))]));
        outcome.record(Err(OperationFailureKind::Network));

        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.unresolved_room_count, 1);
        assert_eq!(outcome.failure_kind, Some(OperationFailureKind::Network));
    }
}
