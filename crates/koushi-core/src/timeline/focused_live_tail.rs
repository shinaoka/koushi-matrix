//! #1676: the anchored (`Focused`) view's live-tail decision.
//!
//! The SDK's event-focused cache never ingests sync events, so once forward
//! pagination reaches the end, newer live messages can never extend it. The
//! final page lands in the SDK timeline from a separate SDK task, so the
//! pagination task first waits until the SDK timeline has processed the
//! cache's last event (`Timeline::latest_event_id` covers every processed
//! remote event, rows or not). The SDK's newest row then is the final tail,
//! with no Koushi rule for which events become rows. The actor latches that
//! tail once its own projection holds it and reports `FocusedLiveTailReached`.

use std::time::Duration;

use koushi_protocol::{
    TimelineGeneration, TimelineKey,
    event::{TimelineEvent, TimelineItem},
};
use matrix_sdk::event_cache::EventFocusedCache;
use matrix_sdk_ui::timeline::Timeline;

use crate::executor;

/// Poll interval while waiting for the SDK timeline to apply the final page;
/// the SDK offers no notification for metadata-only updates.
const CONVERGENCE_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum FocusedLiveTail {
    #[default]
    Unknown,
    /// The forward end is reached and the SDK's final row is known, but
    /// Core's projection may not hold it yet.
    Pending(String),
    /// Reported for this actor generation; never cleared.
    Latched(String),
}

impl FocusedLiveTail {
    pub(super) fn expect(&mut self, tail: String) {
        if !matches!(self, Self::Latched(_)) {
            *self = Self::Pending(tail);
        }
    }

    /// Latches and returns the pending tail once `items` end with it.
    pub(super) fn commit(&mut self, items: &[TimelineItem]) -> Option<String> {
        let Self::Pending(tail) = self else {
            return None;
        };
        if newest_event_id(items) != Some(tail.as_str()) {
            return None;
        }
        let tail = tail.clone();
        *self = Self::Latched(tail.clone());
        Some(tail)
    }

    pub(super) fn latched(&self) -> Option<&str> {
        match self {
            Self::Latched(tail) => Some(tail),
            Self::Unknown | Self::Pending(_) => None,
        }
    }
}

fn newest_event_id(items: &[TimelineItem]) -> Option<&str> {
    items
        .iter()
        .rev()
        .find_map(super::item_projection::timeline_item_event_id)
}

pub(super) fn live_tail_event(
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    tail_event_id: String,
) -> TimelineEvent {
    TimelineEvent::FocusedLiveTailReached {
        key: key.clone(),
        actor_generation,
        generation,
        tail_event_id,
    }
}

/// After a forward page reached the end: waits, within `bound`, until the SDK
/// timeline processed the focused cache's last event, then returns the SDK's
/// newest row event id. `None` when the cache has no event or the SDK did not
/// converge in time; the anchored view then keeps its explicit live return.
pub(super) async fn final_tail(
    timeline: &Timeline,
    cache: &EventFocusedCache,
    bound: Duration,
) -> Option<String> {
    let events = cache.events().await.ok()?;
    let cache_last = events
        .iter()
        .rev()
        .find_map(|event| event.event_id().map(ToOwned::to_owned))?;
    executor::timeout(bound, async {
        while timeline.latest_event_id().await.as_ref() != Some(&cache_last) {
            executor::sleep(CONVERGENCE_POLL).await;
        }
    })
    .await
    .ok()?;
    let (items, _updates) = timeline.subscribe().await;
    items.iter().rev().find_map(|item| {
        item.as_event()
            .and_then(|event| event.event_id())
            .map(ToString::to_string)
    })
}

#[cfg(test)]
mod tests;
