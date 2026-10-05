//! Actor-owned lifecycle for the originals that reply quotes point at (#1120).
//!
//! SDK `InReplyToDetails` are an input, not the authority: an unresolved SDK
//! quote is projected as `Loading`, and the owning `TimelineActor` resolves it
//! either from an original it already holds or through this bounded ledger.
//! The ledger is pure state; the actor executes the returned [`HydrationStep`]s
//! through `executor` and owns the resulting tasks.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use koushi_protocol::event::{TimelineDiff, TimelineItem};
use koushi_protocol::ids::{TimelineBatchId, TimelineKind};
use koushi_state::{OperationFailureKind, ReplyQuote, ReplyQuoteState};

use super::actor::{TimelineActor, TimelineActorMessage};
use super::item_projection::{item_index_for_event_id, reply_quote_from_timeline_item};
use crate::executor;

pub(super) const REPLY_QUOTE_LEDGER_MAX_ENTRIES: usize = 256;
pub(super) const REPLY_QUOTE_MAX_IN_FLIGHT: usize = 4;
pub(super) const REPLY_QUOTE_MAX_ATTEMPTS: u32 = 3;
pub(super) const REPLY_QUOTE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const REPLY_QUOTE_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];

/// Classified result of one exact-event lookup.
#[derive(Clone, Debug)]
pub(super) enum OriginalLookupOutcome {
    Loaded(Box<TimelineItem>),
    Failed(OperationFailureKind),
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HydrationStep {
    Start {
        event_id: String,
        attempt: u32,
    },
    ScheduleRetry {
        event_id: String,
        attempt: u32,
        delay: Duration,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EntryState {
    InFlight {
        attempt: u32,
    },
    RetryWait {
        attempt: u32,
    },
    /// Waiting for an in-flight slot.
    Queued {
        attempt: u32,
    },
    Settled(ReplyQuote),
}

#[derive(Debug, Default)]
pub(super) struct ReplyQuoteHydration {
    entries: HashMap<String, EntryState>,
    /// Insertion order of entries, used to evict settled entries oldest first.
    order: VecDeque<String>,
}

impl ReplyQuoteHydration {
    /// The settled quote for `original_event_id`, if the ledger has one.
    pub(super) fn settled_quote(&self, original_event_id: &str) -> Option<&ReplyQuote> {
        match self.entries.get(original_event_id)? {
            EntryState::Settled(quote) => Some(quote),
            _ => None,
        }
    }

    /// Record an original learned from an authoritative source (a `Ready` SDK
    /// detail or a projected canonical item). Never overrides an entry that is
    /// still being looked up, so stale attempts are fenced by attempt number.
    pub(super) fn learn(&mut self, quote: ReplyQuote) {
        if quote.state != ReplyQuoteState::Ready && quote.state != ReplyQuoteState::Redacted {
            return;
        }
        let event_id = quote.event_id.clone();
        match self.entries.get_mut(&event_id) {
            Some(EntryState::Settled(existing)) => *existing = quote,
            Some(EntryState::Queued { .. } | EntryState::RetryWait { .. }) => {
                self.entries.insert(event_id, EntryState::Settled(quote));
            }
            Some(EntryState::InFlight { .. }) => {
                // The in-flight result will be ignored because the entry is no
                // longer in flight with a matching attempt.
                self.entries.insert(event_id, EntryState::Settled(quote));
            }
            None => {
                if self.make_room() {
                    self.order.push_back(event_id.clone());
                    self.entries.insert(event_id, EntryState::Settled(quote));
                }
            }
        }
    }

    /// Request lookups for unresolved originals. Returns the lookups the actor
    /// must start now.
    pub(super) fn request<'a>(
        &mut self,
        original_event_ids: impl IntoIterator<Item = &'a str>,
    ) -> Vec<HydrationStep> {
        for event_id in original_event_ids {
            if self.entries.contains_key(event_id) || !self.make_room() {
                continue;
            }
            self.order.push_back(event_id.to_owned());
            self.entries
                .insert(event_id.to_owned(), EntryState::Queued { attempt: 1 });
        }
        self.start_queued()
    }

    /// Apply a lookup result. Results whose attempt no longer matches the entry
    /// are ignored.
    pub(super) fn complete(
        &mut self,
        event_id: &str,
        attempt: u32,
        outcome: OriginalLookupOutcome,
    ) -> Vec<HydrationStep> {
        if self.entries.get(event_id) != Some(&EntryState::InFlight { attempt }) {
            return Vec::new();
        }
        let mut steps = Vec::new();
        let terminal = match outcome {
            // `None` means undecryptable or hidden: worth another attempt.
            OriginalLookupOutcome::Loaded(item) => reply_quote_from_timeline_item(event_id, &item),
            OriginalLookupOutcome::Failed(
                OperationFailureKind::NotFound | OperationFailureKind::Forbidden,
            ) => Some(placeholder_quote(event_id, ReplyQuoteState::Missing)),
            OriginalLookupOutcome::Failed(
                OperationFailureKind::Invalid | OperationFailureKind::Sdk,
            ) => Some(placeholder_quote(event_id, ReplyQuoteState::Unsupported)),
            OriginalLookupOutcome::Failed(
                OperationFailureKind::Network | OperationFailureKind::Timeout,
            )
            | OriginalLookupOutcome::TimedOut => None,
        };
        match terminal {
            Some(quote) => {
                self.entries
                    .insert(event_id.to_owned(), EntryState::Settled(quote));
            }
            None if attempt >= REPLY_QUOTE_MAX_ATTEMPTS => {
                self.entries.insert(
                    event_id.to_owned(),
                    EntryState::Settled(placeholder_quote(event_id, ReplyQuoteState::Failed)),
                );
            }
            None => {
                let next = attempt + 1;
                self.entries
                    .insert(event_id.to_owned(), EntryState::RetryWait { attempt: next });
                steps.push(HydrationStep::ScheduleRetry {
                    event_id: event_id.to_owned(),
                    attempt: next,
                    delay: REPLY_QUOTE_RETRY_DELAYS
                        [usize::try_from(attempt - 1).unwrap_or(0).min(1)],
                });
            }
        }
        steps.extend(self.start_queued());
        steps
    }

    /// A retry delay elapsed.
    pub(super) fn retry_due(&mut self, event_id: &str, attempt: u32) -> Vec<HydrationStep> {
        if self.entries.get(event_id) != Some(&EntryState::RetryWait { attempt }) {
            return Vec::new();
        }
        self.entries
            .insert(event_id.to_owned(), EntryState::Queued { attempt });
        self.start_queued()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    fn in_flight(&self) -> usize {
        self.entries
            .values()
            .filter(|state| matches!(state, EntryState::InFlight { .. }))
            .count()
    }

    fn start_queued(&mut self) -> Vec<HydrationStep> {
        let mut available = REPLY_QUOTE_MAX_IN_FLIGHT.saturating_sub(self.in_flight());
        let mut steps = Vec::new();
        for event_id in &self.order {
            if available == 0 {
                break;
            }
            let Some(state) = self.entries.get_mut(event_id) else {
                continue;
            };
            if let EntryState::Queued { attempt } = *state {
                *state = EntryState::InFlight { attempt };
                steps.push(HydrationStep::Start {
                    event_id: event_id.clone(),
                    attempt,
                });
                available -= 1;
            }
        }
        steps
    }

    /// Ensure there is space for one more entry, evicting the oldest settled
    /// entry when full. Returns false when every entry is still unsettled.
    fn make_room(&mut self) -> bool {
        self.order
            .retain(|event_id| self.entries.contains_key(event_id));
        if self.entries.len() < REPLY_QUOTE_LEDGER_MAX_ENTRIES {
            return true;
        }
        let Some(position) = self.order.iter().position(|event_id| {
            matches!(self.entries.get(event_id), Some(EntryState::Settled(_)))
        }) else {
            return false;
        };
        if let Some(event_id) = self.order.remove(position) {
            self.entries.remove(&event_id);
        }
        true
    }
}

pub(super) fn placeholder_quote(event_id: &str, state: ReplyQuoteState) -> ReplyQuote {
    ReplyQuote {
        event_id: event_id.to_owned(),
        sender: None,
        sender_label: None,
        body_preview: None,
        formatted: None,
        state,
    }
}

pub(super) fn reply_quote_is_unresolved(quote: &ReplyQuote) -> bool {
    matches!(
        quote.state,
        ReplyQuoteState::Loading | ReplyQuoteState::Failed
    )
}

/// Resolve unresolved reply quotes on `items` from `originals` (the batch and
/// the actor's canonical items) or from the ledger. Returns true when any
/// quote changed.
pub(super) fn overlay_reply_quotes<'a>(
    items: impl IntoIterator<Item = &'a mut TimelineItem>,
    original_for: impl Fn(&str) -> Option<ReplyQuote>,
    hydration: &ReplyQuoteHydration,
) -> bool {
    let mut changed = false;
    for item in items {
        let Some(quote) = item.reply_quote.as_ref() else {
            continue;
        };
        if !reply_quote_is_unresolved(quote) {
            continue;
        }
        let resolved = original_for(&quote.event_id)
            .or_else(|| hydration.settled_quote(&quote.event_id).cloned());
        if let Some(resolved) = resolved
            && item.reply_quote.as_ref() != Some(&resolved)
        {
            item.reply_quote = Some(resolved);
            changed = true;
        }
    }
    changed
}

/// Original event ids that unresolved (`Loading`) quotes in `items` point at.
pub(super) fn loading_reply_quote_targets<'a>(
    items: impl IntoIterator<Item = &'a TimelineItem>,
) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter_map(|item| item.reply_quote.as_ref())
        .filter(|quote| quote.state == ReplyQuoteState::Loading)
        .map(|quote| quote.event_id.as_str())
        .filter(|event_id| seen.insert(*event_id))
        .collect()
}

/// Ready quotes in `items` that teach the ledger an original.
pub(super) fn learn_ready_reply_quotes<'a>(
    hydration: &mut ReplyQuoteHydration,
    items: impl IntoIterator<Item = &'a TimelineItem>,
) {
    for item in items {
        if let Some(quote) = item.reply_quote.as_ref()
            && quote.state == ReplyQuoteState::Ready
        {
            hydration.learn(quote.clone());
        }
    }
}

fn timeline_diff_items_mut(diffs: &mut [TimelineDiff]) -> impl Iterator<Item = &mut TimelineItem> {
    diffs.iter_mut().flat_map(|diff| match diff {
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => std::slice::from_mut(item).iter_mut(),
        TimelineDiff::Reset { items } => items.iter_mut(),
        _ => [].iter_mut(),
    })
}

fn unresolved_targets<'a>(items: impl IntoIterator<Item = &'a TimelineItem>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter_map(|item| item.reply_quote.as_ref())
        .filter(|quote| reply_quote_is_unresolved(quote))
        .filter(|quote| seen.insert(quote.event_id.as_str()))
        .map(|quote| quote.event_id.clone())
        .collect()
}

/// Project the originals for `targets` found in `batch` (preferred) or in
/// `canonical`.
fn known_originals(
    targets: &[String],
    batch: &[&TimelineItem],
    canonical: &[TimelineItem],
) -> HashMap<String, ReplyQuote> {
    targets
        .iter()
        .filter_map(|target| {
            let original = batch
                .iter()
                .copied()
                .find(|item| super::item_projection::timeline_item_event_id(item) == Some(target))
                .or_else(|| {
                    item_index_for_event_id(canonical, target).map(|index| &canonical[index])
                })?;
            reply_quote_from_timeline_item(target, original).map(|quote| (target.clone(), quote))
        })
        .collect()
}

impl TimelineActor {
    /// Resolve unresolved quotes on an SDK batch before it is committed, so a
    /// reply to a known original is published with a resolved quote.
    pub(super) fn overlay_reply_quotes_on_batch(&mut self, diffs: &mut [TimelineDiff]) {
        let targets = {
            let mut items = Vec::new();
            for item in timeline_diff_items_mut(diffs) {
                items.push(&*item);
            }
            learn_ready_reply_quotes(&mut self.reply_quote_hydration, items.iter().copied());
            unresolved_targets(items.iter().copied())
        };
        if targets.is_empty() {
            return;
        }
        let originals = {
            let mut items = Vec::new();
            for item in timeline_diff_items_mut(diffs) {
                items.push(&*item);
            }
            known_originals(&targets, &items, &self.navigation_items)
        };
        overlay_reply_quotes(
            timeline_diff_items_mut(diffs),
            |event_id| originals.get(event_id).cloned(),
            &self.reply_quote_hydration,
        );
    }

    /// Resolve unresolved quotes on manager-owned pending sends before they are
    /// handed to the display projection.
    pub(super) fn overlay_reply_quotes_on_pending(&self, items: &mut [TimelineItem]) {
        let targets = unresolved_targets(items.iter());
        if targets.is_empty() {
            return;
        }
        let originals = known_originals(&targets, &[], &self.navigation_items);
        overlay_reply_quotes(
            items.iter_mut(),
            |event_id| originals.get(event_id).cloned(),
            &self.reply_quote_hydration,
        );
    }

    /// Republish canonical items and pending sends whose unresolved quotes can
    /// now be resolved from a committed original or a settled ledger entry.
    pub(super) fn republish_reply_quote_dependents(&mut self) {
        let targets = unresolved_targets(self.navigation_items.iter());
        if !targets.is_empty() {
            let originals = known_originals(&targets, &[], &self.navigation_items);
            let mut core_diffs = Vec::new();
            for (index, item) in self.navigation_items.iter_mut().enumerate() {
                if overlay_reply_quotes(
                    std::iter::once(&mut *item),
                    |event_id| originals.get(event_id).cloned(),
                    &self.reply_quote_hydration,
                ) {
                    core_diffs.push(TimelineDiff::Set {
                        index,
                        item: item.clone(),
                    });
                }
            }
            if !core_diffs.is_empty() {
                let _ = self.emit_non_sdk_item_sets(core_diffs);
            }
        }

        let originals = {
            let pending_targets = unresolved_targets(self.display_projection.pending_items());
            known_originals(&pending_targets, &[], &self.navigation_items)
        };
        let hydration = &self.reply_quote_hydration;
        let context = self.display_projection_context();
        let diffs = self.display_projection.overlay_pending_items(
            |items| {
                overlay_reply_quotes(
                    items.iter_mut(),
                    |event_id| originals.get(event_id).cloned(),
                    hydration,
                )
            },
            &context,
        );
        if diffs.is_empty() {
            return;
        }
        let batch_id = self.next_batch_id;
        if super::navigation::emit_items_updated_for_generation(
            &self.event_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            self.generation,
            batch_id,
            diffs,
        ) {
            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
        }
    }

    /// Start bounded lookups for originals that displayed quotes still wait
    /// for. A Thread actor also hydrates its root so replies to it resolve
    /// before the thread is paginated to its start.
    pub(super) fn maybe_hydrate_reply_quotes(&mut self) {
        let mut targets = loading_reply_quote_targets(self.display_projection.display_items())
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if let TimelineKind::Thread { root_event_id, .. } = &self.key.kind
            && item_index_for_event_id(&self.navigation_items, root_event_id).is_none()
        {
            targets.insert(0, root_event_id.clone());
        }
        let steps = self
            .reply_quote_hydration
            .request(targets.iter().map(String::as_str));
        self.execute_reply_quote_steps(steps);
    }

    pub(super) fn handle_reply_quote_original_loaded(
        &mut self,
        event_id: String,
        attempt: u32,
        outcome: OriginalLookupOutcome,
    ) {
        self.reply_quote_tasks.remove(&event_id);
        let steps = self
            .reply_quote_hydration
            .complete(&event_id, attempt, outcome);
        self.execute_reply_quote_steps(steps);
        if self
            .reply_quote_hydration
            .settled_quote(&event_id)
            .is_some()
        {
            self.republish_reply_quote_dependents();
            self.maybe_hydrate_reply_quotes();
        }
    }

    pub(super) fn handle_reply_quote_retry_due(&mut self, event_id: String, attempt: u32) {
        self.reply_quote_tasks.remove(&event_id);
        let steps = self.reply_quote_hydration.retry_due(&event_id, attempt);
        self.execute_reply_quote_steps(steps);
    }

    fn execute_reply_quote_steps(&mut self, steps: Vec<HydrationStep>) {
        for step in steps {
            let msg_tx = self.msg_tx.clone();
            let (event_id, task) = match step {
                HydrationStep::Start { event_id, attempt } => {
                    let session = Arc::clone(&self.session);
                    let key = self.key.clone();
                    let lookup_event_id = event_id.clone();
                    let task = executor::spawn(async move {
                        let outcome = match executor::timeout(
                            REPLY_QUOTE_ATTEMPT_TIMEOUT,
                            super::thread_projection::load_exact_timeline_event_projection(
                                &session,
                                &key,
                                &lookup_event_id,
                            ),
                        )
                        .await
                        {
                            Ok(Ok(item)) => OriginalLookupOutcome::Loaded(Box::new(item)),
                            Ok(Err(kind)) => OriginalLookupOutcome::Failed(kind),
                            Err(executor::TimeoutElapsed) => OriginalLookupOutcome::TimedOut,
                        };
                        let _ = msg_tx
                            .send(TimelineActorMessage::ReplyQuoteOriginalLoaded {
                                event_id: lookup_event_id,
                                attempt,
                                outcome,
                            })
                            .await;
                    });
                    (event_id, task)
                }
                HydrationStep::ScheduleRetry {
                    event_id,
                    attempt,
                    delay,
                } => {
                    let retry_event_id = event_id.clone();
                    let task = executor::spawn(async move {
                        executor::sleep(delay).await;
                        let _ = msg_tx
                            .send(TimelineActorMessage::ReplyQuoteRetryDue {
                                event_id: retry_event_id,
                                attempt,
                            })
                            .await;
                    });
                    (event_id, task)
                }
            };
            if let Some(previous) = self.reply_quote_tasks.insert(event_id, task) {
                previous.abort();
            }
        }
    }
}

#[cfg(test)]
#[path = "reply_quote_hydration/tests.rs"]
mod tests;
