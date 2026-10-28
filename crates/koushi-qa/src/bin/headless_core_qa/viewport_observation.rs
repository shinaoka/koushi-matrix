//! A headless renderer reports facts against the same published window as React.
use super::event_wait::{QaEventDeadline, QaEventSource};
use super::registry::EVENT_TIMEOUT;
use super::{
    CoreCommand, CoreConnection, CoreEvent, RequestId, TimelineCommand, TimelineEvent,
    TimelineItem, TimelineItemId, TimelineKey, TimelineViewportObservation,
};

pub(super) async fn fresh_viewport_observation_for_qa(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    observation: TimelineViewportObservation,
) -> Result<TimelineViewportObservation, String> {
    let deadline = QaEventDeadline::after(EVENT_TIMEOUT);
    let cause = conn.next_request_id();
    tokio::time::timeout_at(
        deadline.instant,
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: cause,
            key: key.clone(),
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        })),
    )
    .await
    .map_err(|_| "QA viewport replay admission timed out".to_owned())?
    .map_err(|_| "QA viewport replay admission failed".to_owned())?;
    observation_from_replay(conn, key, cause, observation, deadline).await
}

fn event_id(item: &TimelineItem) -> Option<&str> {
    if item.is_hidden {
        return None;
    }
    match &item.display_metadata {
        Some(metadata) => metadata.activity_event_id.as_deref(),
        None => match &item.id {
            TimelineItemId::Event { event_id } => Some(event_id.as_str()),
            _ => None,
        },
    }
}

async fn observation_from_replay<S: QaEventSource + ?Sized>(
    source: &mut S,
    key: &TimelineKey,
    cause: RequestId,
    mut observation: TimelineViewportObservation,
    deadline: QaEventDeadline,
) -> Result<TimelineViewportObservation, String> {
    let mut represented = None;
    loop {
        let event = deadline
            .recv(source)
            .await
            .map_err(|_| "QA viewport replay timed out".to_owned())?
            .map_err(|_| "QA viewport replay stream closed or lagged".to_owned())?;
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                cause_request_id: Some(request),
                generation,
                items,
                ..
            }) if event_key == *key && request == cause => {
                represented = Some((generation, items));
            }
            CoreEvent::Timeline(TimelineEvent::InitialItems { key: event_key, .. })
            | CoreEvent::Timeline(TimelineEvent::ResyncRequired { key: event_key, .. })
                if event_key == *key =>
            {
                represented = None;
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: event_key,
                generation,
                diffs,
                ..
            }) if event_key == *key => {
                if let Some((current_generation, items)) = represented.as_mut() {
                    if *current_generation != generation {
                        represented = None;
                        continue;
                    }
                    for diff in &diffs {
                        super::scenario_timeline::apply_timeline_diff(items, diff);
                    }
                }
            }
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
                key: event_key,
                snapshot,
            }) if event_key == *key => {
                let (Some((_, items)), Some(window)) =
                    (represented.as_ref(), snapshot.display_window.as_ref())
                else {
                    continue;
                };
                if items.iter().find_map(event_id) != window.first_event_id.as_deref()
                    || items.iter().rev().find_map(event_id) != window.last_event_id.as_deref()
                {
                    continue;
                }
                if [
                    &observation.first_visible_event_id,
                    &observation.last_visible_event_id,
                ]
                .into_iter()
                .flatten()
                .any(|id| !items.iter().any(|item| event_id(item) == Some(id.as_str())))
                {
                    return Err(
                        "QA viewport target is outside the replayed display window".to_owned()
                    );
                }
                observation.display_window_revision = Some(window.revision);
                observation.at_bottom &= window.reaches_live_end;
                return Ok(observation);
            }
            CoreEvent::OperationFailed { request_id, .. } if request_id == cause => {
                return Err("QA viewport replay failed".to_owned());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "viewport_observation_tests.rs"]
mod tests;
