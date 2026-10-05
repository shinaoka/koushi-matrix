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
pub(super) const REPLY_QUOTE_MAX_UNDECRYPTABLE_ATTEMPTS: u32 = 8;
pub(super) const REPLY_QUOTE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const REPLY_QUOTE_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];
/// Room keys often arrive late, so undecryptable originals back off slowly and
/// are not counted against the transient-failure budget.
const REPLY_QUOTE_UNDECRYPTABLE_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(180),
    Duration::from_secs(300),
];

/// Classified result of one exact-event lookup.
#[derive(Clone, Debug)]
pub(super) enum OriginalLookupOutcome {
    Loaded(Box<TimelineItem>),
    Failed(OperationFailureKind),
    TimedOut,
}

/// Fences one lookup or retry wait. Tokens are unique for the lifetime of a
/// ledger, so a result from an evicted or superseded entry can never match a
/// later entry for the same original.
pub(super) type HydrationToken = u64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HydrationStep {
    Start {
        event_id: String,
        token: HydrationToken,
    },
    ScheduleRetry {
        event_id: String,
        token: HydrationToken,
        delay: Duration,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Attempts {
    /// Completed attempts that failed transiently (network, timeout).
    transient: u32,
    /// Completed attempts that loaded an undecryptable original.
    undecryptable: u32,
}

/// Which budget the attempt that just finished belongs to. The retry delay
/// family follows the current outcome, so an earlier undecryptable result can
/// never slow a later transient retry (or the other way round).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryKind {
    Transient,
    Undecryptable,
}

/// Result of learning one authoritative original.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LearnOutcome {
    /// An existing settled quote was replaced (an edit or redaction).
    pub(super) updated: bool,
    /// An in-flight lookup or retry wait was superseded and must be aborted.
    pub(super) superseded_task: bool,
}

/// Originals whose authoritative content changed while overlaying one batch.
#[derive(Debug, Default)]
pub(super) struct ReplyQuoteRefreshes {
    /// Settled entries that changed: every dependent quote must be re-derived,
    /// resolved or not.
    pub(super) changed: HashSet<String>,
    /// Lookups a change superseded: their actor tasks must be aborted.
    pub(super) superseded: HashSet<String>,
}

pub(super) fn record_learn(
    refreshes: &mut ReplyQuoteRefreshes,
    event_id: String,
    outcome: LearnOutcome,
) {
    if outcome.updated {
        refreshes.changed.insert(event_id.clone());
    }
    if outcome.superseded_task {
        refreshes.superseded.insert(event_id);
    }
}

/// Teach the ledger the originals a projection derived, so a later edit or
/// redaction of one is detected against the value the actor last projected.
fn learn_derived_originals(
    hydration: &mut ReplyQuoteHydration,
    refreshes: &mut ReplyQuoteRefreshes,
    originals: &HashMap<String, ReplyQuote>,
) {
    for (target, quote) in originals {
        let outcome = hydration.learn(quote.clone());
        record_learn(refreshes, target.clone(), outcome);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EntryState {
    InFlight {
        attempts: Attempts,
        token: HydrationToken,
    },
    RetryWait {
        attempts: Attempts,
        token: HydrationToken,
    },
    /// Waiting for an in-flight slot.
    Queued {
        attempts: Attempts,
    },
    Settled(ReplyQuote),
}

#[derive(Debug, Default)]
pub(super) struct ReplyQuoteHydration {
    entries: HashMap<String, EntryState>,
    /// Insertion order of entries, used to evict settled entries oldest first.
    order: VecDeque<String>,
    next_token: HydrationToken,
    /// An original that is never evicted (a Thread actor's root).
    retained: Option<String>,
}

impl ReplyQuoteHydration {
    /// The settled quote for `original_event_id`, if the ledger has one.
    pub(super) fn settled_quote(&self, original_event_id: &str) -> Option<&ReplyQuote> {
        match self.entries.get(original_event_id)? {
            EntryState::Settled(quote) => Some(quote),
            _ => None,
        }
    }

    pub(super) fn tracks(&self, original_event_id: &str) -> bool {
        self.entries.contains_key(original_event_id)
    }

    pub(super) fn retain(&mut self, original_event_id: &str) {
        if self.retained.as_deref() != Some(original_event_id) {
            self.retained = Some(original_event_id.to_owned());
        }
    }

    /// Record an original observed from an authoritative source (a `Ready` or
    /// `Redacted` SDK detail, or a projected item of this actor's timeline).
    /// The observation supersedes any lookup or retry wait for the original;
    /// a changed settled value is reported as `updated`, so newer edits and
    /// redactions of an observed original reach every dependent quote.
    pub(super) fn learn(&mut self, quote: ReplyQuote) -> LearnOutcome {
        if !matches!(
            quote.state,
            ReplyQuoteState::Ready | ReplyQuoteState::Redacted | ReplyQuoteState::Unsupported
        ) {
            return LearnOutcome::default();
        }
        let event_id = quote.event_id.clone();
        match self.entries.get_mut(&event_id) {
            Some(EntryState::Settled(existing)) => {
                if *existing == quote {
                    return LearnOutcome::default();
                }
                *existing = quote;
                LearnOutcome {
                    updated: true,
                    superseded_task: false,
                }
            }
            Some(state) => {
                let superseded_task = matches!(
                    state,
                    EntryState::InFlight { .. } | EntryState::RetryWait { .. }
                );
                *state = EntryState::Settled(quote);
                LearnOutcome {
                    updated: false,
                    superseded_task,
                }
            }
            None => {
                if self.make_room() {
                    self.order.push_back(event_id.clone());
                    self.entries.insert(event_id, EntryState::Settled(quote));
                }
                LearnOutcome::default()
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
            self.entries.insert(
                event_id.to_owned(),
                EntryState::Queued {
                    attempts: Attempts::default(),
                },
            );
        }
        self.start_queued()
    }

    /// Apply a lookup result. Results whose token no longer matches the
    /// entry's in-flight lookup are ignored.
    pub(super) fn complete(
        &mut self,
        event_id: &str,
        token: HydrationToken,
        outcome: OriginalLookupOutcome,
    ) -> Vec<HydrationStep> {
        let Some(EntryState::InFlight {
            attempts,
            token: current,
        }) = self.entries.get(event_id).cloned()
        else {
            return Vec::new();
        };
        if current != token {
            return Vec::new();
        }
        let mut attempts = attempts;
        let mut retry_kind = RetryKind::Transient;
        let settled = match outcome {
            OriginalLookupOutcome::Loaded(item) => {
                match reply_quote_from_timeline_item(event_id, &item) {
                    Some(quote) => Some(quote),
                    None => {
                        attempts.undecryptable += 1;
                        retry_kind = RetryKind::Undecryptable;
                        (attempts.undecryptable >= REPLY_QUOTE_MAX_UNDECRYPTABLE_ATTEMPTS)
                            .then(|| placeholder_quote(event_id, ReplyQuoteState::Failed))
                    }
                }
            }
            OriginalLookupOutcome::Failed(
                OperationFailureKind::NotFound | OperationFailureKind::Forbidden,
            ) => Some(placeholder_quote(event_id, ReplyQuoteState::Missing)),
            OriginalLookupOutcome::Failed(
                OperationFailureKind::Invalid | OperationFailureKind::Sdk,
            ) => Some(placeholder_quote(event_id, ReplyQuoteState::Unsupported)),
            OriginalLookupOutcome::Failed(
                OperationFailureKind::Network | OperationFailureKind::Timeout,
            )
            | OriginalLookupOutcome::TimedOut => {
                attempts.transient += 1;
                (attempts.transient >= REPLY_QUOTE_MAX_ATTEMPTS)
                    .then(|| placeholder_quote(event_id, ReplyQuoteState::Failed))
            }
        };
        let mut steps = Vec::new();
        match settled {
            Some(quote) => {
                self.entries
                    .insert(event_id.to_owned(), EntryState::Settled(quote));
            }
            None => {
                let delay = retry_delay(retry_kind, attempts);
                let token = self.issue_token();
                self.entries.insert(
                    event_id.to_owned(),
                    EntryState::RetryWait { attempts, token },
                );
                steps.push(HydrationStep::ScheduleRetry {
                    event_id: event_id.to_owned(),
                    token,
                    delay,
                });
            }
        }
        steps.extend(self.start_queued());
        steps
    }

    /// A retry delay elapsed.
    pub(super) fn retry_due(
        &mut self,
        event_id: &str,
        token: HydrationToken,
    ) -> Vec<HydrationStep> {
        let Some(EntryState::RetryWait {
            attempts,
            token: current,
        }) = self.entries.get(event_id).cloned()
        else {
            return Vec::new();
        };
        if current != token {
            return Vec::new();
        }
        self.entries
            .insert(event_id.to_owned(), EntryState::Queued { attempts });
        self.start_queued()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    fn issue_token(&mut self) -> HydrationToken {
        self.next_token += 1;
        self.next_token
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
        let queued = self
            .order
            .iter()
            .filter(|event_id| {
                matches!(self.entries.get(*event_id), Some(EntryState::Queued { .. }))
            })
            .cloned()
            .collect::<Vec<_>>();
        for event_id in queued {
            if available == 0 {
                break;
            }
            let token = self.issue_token();
            if let Some(state) = self.entries.get_mut(&event_id)
                && let EntryState::Queued { attempts } = *state
            {
                *state = EntryState::InFlight { attempts, token };
                steps.push(HydrationStep::Start { event_id, token });
                available -= 1;
            }
        }
        steps
    }

    /// Ensure there is space for one more entry, evicting the oldest settled
    /// entry other than the retained original when full. Returns false when
    /// no entry can be evicted.
    fn make_room(&mut self) -> bool {
        self.order
            .retain(|event_id| self.entries.contains_key(event_id));
        if self.entries.len() < REPLY_QUOTE_LEDGER_MAX_ENTRIES {
            return true;
        }
        let Some(position) = self.order.iter().position(|event_id| {
            self.retained.as_deref() != Some(event_id.as_str())
                && matches!(self.entries.get(event_id), Some(EntryState::Settled(_)))
        }) else {
            return false;
        };
        if let Some(event_id) = self.order.remove(position) {
            self.entries.remove(&event_id);
        }
        true
    }
}

fn retry_delay(kind: RetryKind, attempts: Attempts) -> Duration {
    let (index, delays) = match kind {
        RetryKind::Transient => (
            attempts.transient.saturating_sub(1),
            &REPLY_QUOTE_RETRY_DELAYS[..],
        ),
        RetryKind::Undecryptable => (
            attempts.undecryptable.saturating_sub(1),
            &REPLY_QUOTE_UNDECRYPTABLE_RETRY_DELAYS[..],
        ),
    };
    let index = usize::try_from(index)
        .unwrap_or(usize::MAX)
        .min(delays.len() - 1);
    delays[index]
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
    refresh: &HashSet<String>,
) -> bool {
    let mut changed = false;
    for item in items {
        let Some(quote) = item.reply_quote.as_ref() else {
            continue;
        };
        // An unresolved quote always takes the newest value. A resolved quote
        // is re-derived only when its original changed (an edit or redaction),
        // so stale content cannot survive in the rejoinder.
        if !reply_quote_is_unresolved(quote) && !refresh.contains(&quote.event_id) {
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

/// Every original event id that a quote in `items` points at, resolved or not.
pub(super) fn quote_targets<'a>(items: impl IntoIterator<Item = &'a TimelineItem>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter_map(|item| item.reply_quote.as_ref())
        .map(|quote| quote.event_id.as_str())
        .filter(|event_id| seen.insert(*event_id))
        .collect()
}

/// Learn every authoritative original the items project: a `Ready`/`Redacted`
/// quote a rejoinder carries, and the original itself when a quote already
/// tracks it, so an edit or redaction refreshes the stored content. A target
/// outside this actor's timeline only updates when it appears here or the
/// actor is replaced.
pub(super) fn learn_projected_originals<'a>(
    hydration: &mut ReplyQuoteHydration,
    items: impl IntoIterator<Item = &'a TimelineItem>,
    refreshes: &mut ReplyQuoteRefreshes,
) {
    for item in items {
        if let Some(quote) = item.reply_quote.as_ref()
            && matches!(
                quote.state,
                ReplyQuoteState::Ready | ReplyQuoteState::Redacted
            )
        {
            let outcome = hydration.learn(quote.clone());
            record_learn(refreshes, quote.event_id.clone(), outcome);
        }
        let Some(event_id) = super::item_projection::timeline_item_event_id(item) else {
            continue;
        };
        if !hydration.tracks(event_id) {
            continue;
        }
        if let Some(quote) = reply_quote_from_timeline_item(event_id, item) {
            let outcome = hydration.learn(quote);
            record_learn(refreshes, event_id.to_owned(), outcome);
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

/// Project the originals for `targets` found in `batch` (preferred) or in
/// `canonical`, in a single pass per source.
fn known_originals(
    targets: &[String],
    batch: &[&TimelineItem],
    canonical: &[TimelineItem],
) -> HashMap<String, ReplyQuote> {
    let wanted: HashSet<&str> = targets.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return HashMap::new();
    }
    let mut originals = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for item in batch.iter().copied().chain(canonical.iter()) {
        let Some(event_id) = super::item_projection::timeline_item_event_id(item) else {
            continue;
        };
        if !wanted.contains(event_id) || !seen.insert(event_id.to_owned()) {
            continue;
        }
        if let Some(quote) = reply_quote_from_timeline_item(event_id, item) {
            originals.insert(event_id.to_owned(), quote);
        }
    }
    originals
}

/// Originals a batch projects plus the changes they caused.
#[derive(Debug, Default)]
pub(super) struct BatchReplyQuotes {
    /// Learned changes and superseded lookups.
    pub(super) refreshes: ReplyQuoteRefreshes,
    /// Originals this batch projects (batch items preferred over the actor's
    /// canonical items). While a batch is being committed the canonical items
    /// still hold the previous value, so a pre-commit consumer must prefer
    /// these over the canonical fallback.
    pub(super) originals: HashMap<String, ReplyQuote>,
}

/// Merge a batch's originals over the canonical fallback for `targets`. The
/// batch value wins for a target it projects, so a pre-commit consumer cannot
/// overwrite a newer observation with the previous committed one.
fn merge_originals_for(
    targets: &[String],
    batch: &HashMap<String, ReplyQuote>,
    canonical: HashMap<String, ReplyQuote>,
) -> HashMap<String, ReplyQuote> {
    let mut merged: HashMap<String, ReplyQuote> = targets
        .iter()
        .filter_map(|target| {
            batch
                .get(target)
                .map(|quote| (target.clone(), quote.clone()))
        })
        .collect();
    for target in targets {
        if merged.contains_key(target) {
            continue;
        }
        if let Some(quote) = canonical.get(target) {
            merged.insert(target.clone(), quote.clone());
        }
    }
    merged
}

impl TimelineActor {
    /// Resolve unresolved quotes on an SDK batch before it is committed, so a
    /// reply to a known original is published with a resolved quote. Returns
    /// the originals whose settled content changed (so callers refresh every
    /// dependent quote, resolved or not) and the originals this batch projects.
    pub(super) fn overlay_reply_quotes_on_batch(
        &mut self,
        diffs: &mut [TimelineDiff],
    ) -> BatchReplyQuotes {
        let mut refreshes = ReplyQuoteRefreshes::default();
        let targets = {
            let mut items = Vec::new();
            for item in timeline_diff_items_mut(diffs) {
                items.push(&*item);
            }
            learn_projected_originals(
                &mut self.reply_quote_hydration,
                items.iter().copied(),
                &mut refreshes,
            );
            quote_targets(items.iter().copied())
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        if targets.is_empty() {
            return BatchReplyQuotes {
                refreshes,
                originals: HashMap::new(),
            };
        }
        let originals = {
            let mut items = Vec::new();
            for item in timeline_diff_items_mut(diffs) {
                items.push(&*item);
            }
            known_originals(&targets, &items, &self.navigation_items)
        };
        for (target, quote) in &originals {
            let outcome = self.reply_quote_hydration.learn(quote.clone());
            record_learn(&mut refreshes, target.clone(), outcome);
        }
        overlay_reply_quotes(
            timeline_diff_items_mut(diffs),
            |event_id| originals.get(event_id).cloned(),
            &self.reply_quote_hydration,
            &refreshes.changed,
        );
        BatchReplyQuotes {
            refreshes,
            originals,
        }
    }

    /// Resolve unresolved quotes on manager-owned pending sends before they are
    /// handed to the display projection. The originals this overlay derives are
    /// also taught to the ledger: a pending reply can resolve from a canonical
    /// original without any lookup, and that original's later edit or redaction
    /// needs a previous value in the ledger to be detected against.
    ///
    /// `batch_originals` are the originals a not-yet-committed batch projects;
    /// they win over the canonical items, which still hold the previous value.
    pub(super) fn overlay_reply_quotes_on_pending(
        &mut self,
        items: &mut [TimelineItem],
        batch_originals: &HashMap<String, ReplyQuote>,
    ) {
        let targets = quote_targets(items.iter())
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return;
        }
        let canonical = known_originals(&targets, &[], &self.navigation_items);
        let originals = merge_originals_for(&targets, batch_originals, canonical);
        let mut refreshes = ReplyQuoteRefreshes::default();
        learn_derived_originals(&mut self.reply_quote_hydration, &mut refreshes, &originals);
        for event_id in refreshes.superseded {
            self.abort_reply_quote_task(&event_id);
        }
        self.reply_quote_refresh.extend(refreshes.changed);
        overlay_reply_quotes(
            items.iter_mut(),
            |event_id| originals.get(event_id).cloned(),
            &self.reply_quote_hydration,
            &self.reply_quote_refresh,
        );
    }

    /// Republish canonical items and pending sends whose quotes can now be
    /// resolved from a committed original or a settled ledger entry, including
    /// already-resolved quotes whose original changed in the retained refresh
    /// set.
    ///
    /// While an anchor restore is buffering its coalesced emission, the
    /// republish is deferred: emitting item sets now would overtake
    /// `restore_emit_buffer` and reorder the UI's settled update.
    pub(super) fn republish_reply_quote_dependents(&mut self) {
        if self.restore_anchor.is_some() {
            self.reply_quote_republish_pending = true;
            return;
        }
        self.reply_quote_republish_pending = false;
        let refresh = std::mem::take(&mut self.reply_quote_refresh);
        let refresh = &refresh;
        let targets = quote_targets(self.navigation_items.iter())
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            let originals = known_originals(&targets, &[], &self.navigation_items);
            let mut core_diffs = Vec::new();
            for (index, item) in self.navigation_items.iter_mut().enumerate() {
                if overlay_reply_quotes(
                    std::iter::once(&mut *item),
                    |event_id| originals.get(event_id).cloned(),
                    &self.reply_quote_hydration,
                    refresh,
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
            let pending_targets = quote_targets(self.display_projection.pending_items())
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
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
                    refresh,
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
        token: HydrationToken,
        outcome: OriginalLookupOutcome,
    ) {
        if !self.clear_reply_quote_task(&event_id, token) {
            return;
        }
        let steps = self
            .reply_quote_hydration
            .complete(&event_id, token, outcome);
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

    pub(super) fn handle_reply_quote_retry_due(&mut self, event_id: String, token: HydrationToken) {
        if !self.clear_reply_quote_task(&event_id, token) {
            return;
        }
        let steps = self.reply_quote_hydration.retry_due(&event_id, token);
        self.execute_reply_quote_steps(steps);
    }

    /// Drop the tracked task for `event_id` only while `token` is still current,
    /// so a result or wake from a superseded or evicted lookup is ignored.
    fn clear_reply_quote_task(&mut self, event_id: &str, token: HydrationToken) -> bool {
        if self
            .reply_quote_tasks
            .get(event_id)
            .map(|(tracked, _)| *tracked)
            != Some(token)
        {
            return false;
        }
        self.reply_quote_tasks.remove(event_id);
        true
    }

    /// Abort and drop the tracked task for an original whose observation
    /// superseded it.
    fn abort_reply_quote_task(&mut self, event_id: &str) {
        if let Some((_token, task)) = self.reply_quote_tasks.remove(event_id) {
            task.abort();
        }
    }

    /// Retain the originals a batch changed and abort the lookups it
    /// superseded, so a settled edit or redaction refreshes dependents and a
    /// stale lookup result can never overwrite it.
    pub(super) fn apply_reply_quote_refreshes(&mut self, refreshes: ReplyQuoteRefreshes) {
        for event_id in refreshes.superseded {
            self.abort_reply_quote_task(&event_id);
        }
        self.reply_quote_refresh.extend(refreshes.changed);
    }

    fn execute_reply_quote_steps(&mut self, steps: Vec<HydrationStep>) {
        for step in steps {
            let msg_tx = self.msg_tx.clone();
            let (event_id, token, task) = match step {
                HydrationStep::Start { event_id, token } => {
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
                                token,
                                outcome,
                            })
                            .await;
                    });
                    (event_id, token, task)
                }
                HydrationStep::ScheduleRetry {
                    event_id,
                    token,
                    delay,
                } => {
                    let retry_event_id = event_id.clone();
                    let task = executor::spawn(async move {
                        executor::sleep(delay).await;
                        let _ = msg_tx
                            .send(TimelineActorMessage::ReplyQuoteRetryDue {
                                event_id: retry_event_id,
                                token,
                            })
                            .await;
                    });
                    (event_id, token, task)
                }
            };
            if let Some((_token, previous)) = self.reply_quote_tasks.insert(event_id, (token, task))
            {
                previous.abort();
            }
        }
    }
}

#[cfg(test)]
#[path = "reply_quote_hydration/tests.rs"]
mod tests;
