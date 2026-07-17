use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

pub(crate) const FOREGROUND_LIVE_TAIL_LIMIT: u16 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveTailFreshnessState {
    Unproven {
        epoch: u64,
    },
    Refreshing {
        epoch: u64,
        operation_generation: u64,
    },
    Fresh {
        epoch: u64,
    },
    Deferred {
        epoch: u64,
    },
    Retryable {
        epoch: u64,
    },
}

impl LiveTailFreshnessState {
    fn epoch(self) -> u64 {
        match self {
            Self::Unproven { epoch }
            | Self::Refreshing { epoch, .. }
            | Self::Fresh { epoch }
            | Self::Deferred { epoch }
            | Self::Retryable { epoch } => epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveTailRefreshOutcome {
    Cancelled,
    Unchanged,
    Advanced {
        events: usize,
    },
    Detached {
        events: usize,
        historical_gap_remaining: bool,
    },
    Stale,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LiveTailSchedulerAction<K> {
    Start {
        key: K,
        epoch: u64,
        operation_generation: u64,
        limit: u16,
    },
    CancelNetwork {
        key: K,
        operation_generation: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RunningLiveTailRefresh<K> {
    key: K,
    epoch: u64,
    operation_generation: u64,
}

pub(crate) struct LiveTailRefreshCoordinator<K> {
    active: Option<K>,
    states: HashMap<K, LiveTailFreshnessState>,
    running: Option<RunningLiveTailRefresh<K>>,
    delayed: VecDeque<K>,
    delayed_members: HashSet<K>,
    cancelled_active: Option<K>,
    operation_generation: u64,
}

impl<K> LiveTailRefreshCoordinator<K>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new() -> Self {
        Self {
            active: None,
            states: HashMap::new(),
            running: None,
            delayed: VecDeque::new(),
            delayed_members: HashSet::new(),
            cancelled_active: None,
            operation_generation: 0,
        }
    }

    pub(crate) fn freshness(&self, key: &K) -> Option<LiveTailFreshnessState> {
        self.states.get(key).copied()
    }

    pub(crate) fn activate(&mut self, key: K, epoch: u64) -> Vec<LiveTailSchedulerAction<K>> {
        if self.known_epoch_is_newer(&key, epoch) {
            return Vec::new();
        }

        let old_active = self.active.replace(key.clone());
        self.remove_delayed(&key);
        if !matches!(
            self.states.get(&key),
            Some(LiveTailFreshnessState::Unproven { epoch: known_epoch }
                | LiveTailFreshnessState::Refreshing { epoch: known_epoch, .. }
                | LiveTailFreshnessState::Fresh { epoch: known_epoch })
                if *known_epoch == epoch
        ) {
            self.states
                .insert(key.clone(), LiveTailFreshnessState::Unproven { epoch });
        }
        self.clear_cancelled_active(&key);

        let mut actions = Vec::new();
        if self.unproven_epoch(&key).is_some()
            && self
                .running
                .as_ref()
                .is_some_and(|running| running.key != key || running.epoch != epoch)
        {
            if let Some(cancel) = self.preempt_running() {
                actions.push(cancel);
            }
        }

        if let Some(old_active) = old_active {
            if old_active != key && !self.is_running(&old_active) {
                self.defer_if_pending(old_active);
            }
        }

        actions.extend(self.schedule_next(None));
        actions
    }

    pub(crate) fn mark_unproven(&mut self, key: K, epoch: u64) -> Vec<LiveTailSchedulerAction<K>> {
        if self.known_epoch_is_newer(&key, epoch) {
            return Vec::new();
        }

        match self.states.get(&key).copied() {
            Some(LiveTailFreshnessState::Fresh { epoch: known_epoch })
            | Some(LiveTailFreshnessState::Refreshing {
                epoch: known_epoch, ..
            }) if known_epoch == epoch => return Vec::new(),
            _ => {}
        }

        let mut actions = Vec::new();
        let fenced_replacement = if let Some(cancel) = self.fence_replaced_running(&key, epoch) {
            actions.push(cancel);
            true
        } else {
            false
        };
        let is_active = self.active.as_ref() == Some(&key);
        self.clear_cancelled_active(&key);
        self.states.insert(
            key.clone(),
            if is_active {
                LiveTailFreshnessState::Unproven { epoch }
            } else {
                LiveTailFreshnessState::Deferred { epoch }
            },
        );

        if !is_active {
            self.queue_delayed(key.clone());
            actions.extend(self.schedule_next(fenced_replacement.then_some(&key)));
            return actions;
        }

        if self
            .running
            .as_ref()
            .is_some_and(|running| running.key != key || running.epoch != epoch)
        {
            if let Some(cancel) = self.preempt_running() {
                actions.push(cancel);
            }
        }
        actions.extend(self.schedule_next(None));
        actions
    }

    pub(crate) fn mark_fresh(&mut self, key: K, epoch: u64) -> Vec<LiveTailSchedulerAction<K>> {
        if self.known_epoch_is_newer(&key, epoch) {
            return Vec::new();
        }

        self.states
            .insert(key.clone(), LiveTailFreshnessState::Fresh { epoch });
        self.remove_delayed(&key);
        self.clear_cancelled_active(&key);

        let mut actions = Vec::new();
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.key == key && running.epoch <= epoch)
        {
            let running = self
                .running
                .take()
                .expect("matching live-tail refresh must still be running");
            actions.push(LiveTailSchedulerAction::CancelNetwork {
                key: running.key,
                operation_generation: running.operation_generation,
            });
        }
        actions.extend(self.schedule_next(None));
        actions
    }

    pub(crate) fn finish(
        &mut self,
        key: K,
        epoch: u64,
        operation_generation: u64,
        outcome: LiveTailRefreshOutcome,
    ) -> Vec<LiveTailSchedulerAction<K>> {
        let matches_running = self.running.as_ref().is_some_and(|running| {
            running.key == key
                && running.epoch == epoch
                && running.operation_generation == operation_generation
        });
        if !matches_running {
            return Vec::new();
        }
        self.running = None;

        let is_active = self.active.as_ref() == Some(&key);
        let next_state = match outcome {
            LiveTailRefreshOutcome::Unchanged
            | LiveTailRefreshOutcome::Advanced { .. }
            | LiveTailRefreshOutcome::Detached { .. } => LiveTailFreshnessState::Fresh { epoch },
            LiveTailRefreshOutcome::Cancelled if is_active => {
                LiveTailFreshnessState::Unproven { epoch }
            }
            LiveTailRefreshOutcome::Cancelled => LiveTailFreshnessState::Deferred { epoch },
            LiveTailRefreshOutcome::Stale | LiveTailRefreshOutcome::Failed if is_active => {
                LiveTailFreshnessState::Retryable { epoch }
            }
            LiveTailRefreshOutcome::Stale | LiveTailRefreshOutcome::Failed => {
                LiveTailFreshnessState::Deferred { epoch }
            }
        };
        self.states.insert(key.clone(), next_state);
        self.remove_delayed(&key);
        self.clear_cancelled_active(&key);
        if outcome == LiveTailRefreshOutcome::Cancelled && is_active {
            self.cancelled_active = Some(key);
        }
        self.schedule_next(None)
    }

    pub(crate) fn invalidate_epoch(
        &mut self,
        key: K,
        epoch: u64,
    ) -> Vec<LiveTailSchedulerAction<K>> {
        if self.known_epoch_is_newer(&key, epoch)
            || self
                .states
                .get(&key)
                .is_some_and(|state| state.epoch() == epoch)
        {
            return Vec::new();
        }

        let is_active = self.active.as_ref() == Some(&key);
        let mut actions = Vec::new();
        self.clear_cancelled_active(&key);

        if let Some(cancel) = self.fence_replaced_running(&key, epoch) {
            actions.push(cancel);
        }

        self.states.insert(
            key.clone(),
            if is_active {
                LiveTailFreshnessState::Unproven { epoch }
            } else {
                LiveTailFreshnessState::Deferred { epoch }
            },
        );

        if is_active {
            if self.running.is_some() {
                if let Some(cancel) = self.preempt_running() {
                    actions.push(cancel);
                }
            }
            self.remove_delayed(&key);
            actions.extend(self.schedule_next(None));
        } else {
            self.queue_delayed(key.clone());
            actions.extend(self.schedule_next(Some(&key)));
        }

        actions
    }

    fn known_epoch_is_newer(&self, key: &K, epoch: u64) -> bool {
        self.states
            .get(key)
            .is_some_and(|state| state.epoch() > epoch)
    }

    fn recoverable_epoch(&self, key: &K) -> Option<u64> {
        match self.states.get(key) {
            Some(
                LiveTailFreshnessState::Unproven { epoch }
                | LiveTailFreshnessState::Deferred { epoch }
                | LiveTailFreshnessState::Retryable { epoch },
            ) => Some(*epoch),
            _ => None,
        }
    }

    fn unproven_epoch(&self, key: &K) -> Option<u64> {
        match self.states.get(key) {
            Some(LiveTailFreshnessState::Unproven { epoch }) => Some(*epoch),
            _ => None,
        }
    }

    fn deferred_epoch(&self, key: &K) -> Option<u64> {
        match self.states.get(key) {
            Some(LiveTailFreshnessState::Deferred { epoch }) => Some(*epoch),
            _ => None,
        }
    }

    fn is_running(&self, key: &K) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| &running.key == key)
    }

    fn fence_replaced_running(
        &mut self,
        key: &K,
        epoch: u64,
    ) -> Option<LiveTailSchedulerAction<K>> {
        let is_replaced = self
            .running
            .as_ref()
            .is_some_and(|running| &running.key == key && running.epoch != epoch);
        if !is_replaced {
            return None;
        }
        let running = self
            .running
            .take()
            .expect("replaced live-tail refresh must still be running");
        Some(LiveTailSchedulerAction::CancelNetwork {
            key: running.key,
            operation_generation: running.operation_generation,
        })
    }

    fn defer_if_pending(&mut self, key: K) {
        let Some(epoch) = self.recoverable_epoch(&key) else {
            return;
        };
        self.states
            .insert(key.clone(), LiveTailFreshnessState::Deferred { epoch });
        self.clear_cancelled_active(&key);
        self.queue_delayed(key);
    }

    fn preempt_running(&mut self) -> Option<LiveTailSchedulerAction<K>> {
        let running = self.running.take()?;
        let epoch_is_current = self
            .states
            .get(&running.key)
            .is_some_and(|state| state.epoch() == running.epoch);
        if epoch_is_current {
            self.states.insert(
                running.key.clone(),
                LiveTailFreshnessState::Deferred {
                    epoch: running.epoch,
                },
            );
            self.clear_cancelled_active(&running.key);
            self.queue_delayed(running.key.clone());
        }
        Some(LiveTailSchedulerAction::CancelNetwork {
            key: running.key,
            operation_generation: running.operation_generation,
        })
    }

    fn schedule_next(&mut self, skip: Option<&K>) -> Vec<LiveTailSchedulerAction<K>> {
        if self.running.is_some() {
            return Vec::new();
        }

        if let Some(active) = self.active.clone() {
            if skip != Some(&active) && self.cancelled_active.as_ref() != Some(&active) {
                if let Some(epoch) = self.unproven_epoch(&active) {
                    return vec![self.start(active, epoch)];
                }
            }
        }

        let delayed_count = self.delayed.len();
        for _ in 0..delayed_count {
            let key = self
                .delayed
                .pop_front()
                .expect("delayed count must match queue contents");
            self.delayed_members.remove(&key);

            if skip == Some(&key) {
                self.queue_delayed(key);
                continue;
            }
            if self.active.as_ref() == Some(&key) {
                continue;
            }
            let Some(epoch) = self.deferred_epoch(&key) else {
                continue;
            };
            return vec![self.start(key, epoch)];
        }

        Vec::new()
    }

    fn start(&mut self, key: K, epoch: u64) -> LiveTailSchedulerAction<K> {
        debug_assert!(self.running.is_none());
        self.remove_delayed(&key);
        self.clear_cancelled_active(&key);
        self.operation_generation = self
            .operation_generation
            .checked_add(1)
            .expect("live-tail operation generation exhausted");
        let operation_generation = self.operation_generation;
        self.states.insert(
            key.clone(),
            LiveTailFreshnessState::Refreshing {
                epoch,
                operation_generation,
            },
        );
        self.running = Some(RunningLiveTailRefresh {
            key: key.clone(),
            epoch,
            operation_generation,
        });
        LiveTailSchedulerAction::Start {
            key,
            epoch,
            operation_generation,
            limit: FOREGROUND_LIVE_TAIL_LIMIT,
        }
    }

    fn queue_delayed(&mut self, key: K) {
        if self.delayed_members.insert(key.clone()) {
            self.delayed.push_back(key);
        }
    }

    fn clear_cancelled_active(&mut self, key: &K) {
        if self.cancelled_active.as_ref() == Some(key) {
            self.cancelled_active = None;
        }
    }

    fn remove_delayed(&mut self, key: &K) {
        if self.delayed_members.remove(key) {
            self.delayed.retain(|queued| queued != key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FOREGROUND_LIVE_TAIL_LIMIT, LiveTailFreshnessState, LiveTailRefreshCoordinator,
        LiveTailRefreshOutcome, LiveTailSchedulerAction,
    };

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestKey {
        A,
        B,
        C,
    }

    use TestKey::{A, B, C};

    #[test]
    fn active_unproven_room_starts_once_and_same_epoch_does_not_retry_after_fresh() {
        let mut coordinator = LiveTailRefreshCoordinator::new();

        assert_eq!(
            coordinator.activate(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 1,
                limit: FOREGROUND_LIVE_TAIL_LIMIT,
            }]
        );
        assert_eq!(coordinator.mark_unproven(A, 7), Vec::new());
        assert_eq!(
            coordinator.finish(A, 7, 1, LiveTailRefreshOutcome::Unchanged),
            Vec::new()
        );
        assert_eq!(coordinator.mark_unproven(A, 7), Vec::new());
        assert_eq!(coordinator.activate(A, 7), Vec::new());
        assert_eq!(
            coordinator.freshness(&A),
            Some(LiveTailFreshnessState::Fresh { epoch: 7 })
        );
    }

    #[test]
    fn activating_b_preempts_a_and_delays_a_before_starting_b() {
        let mut coordinator = LiveTailRefreshCoordinator::new();

        assert_eq!(
            coordinator.activate(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 1,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.activate(B, 9),
            vec![
                LiveTailSchedulerAction::CancelNetwork {
                    key: A,
                    operation_generation: 1,
                },
                LiveTailSchedulerAction::Start {
                    key: B,
                    epoch: 9,
                    operation_generation: 2,
                    limit: 128,
                },
            ]
        );
        assert_eq!(
            coordinator.freshness(&A),
            Some(LiveTailFreshnessState::Deferred { epoch: 7 })
        );
        assert_eq!(
            coordinator.finish(A, 7, 1, LiveTailRefreshOutcome::Cancelled),
            Vec::new()
        );
        assert_eq!(
            coordinator.finish(B, 9, 2, LiveTailRefreshOutcome::Unchanged),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 3,
                limit: 128,
            }]
        );
    }

    #[test]
    fn late_old_epoch_finish_cannot_prove_replacement_epoch() {
        let mut coordinator = LiveTailRefreshCoordinator::new();

        assert_eq!(
            coordinator.activate(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 1,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.invalidate_epoch(A, 8),
            vec![
                LiveTailSchedulerAction::CancelNetwork {
                    key: A,
                    operation_generation: 1,
                },
                LiveTailSchedulerAction::Start {
                    key: A,
                    epoch: 8,
                    operation_generation: 2,
                    limit: 128,
                },
            ]
        );
        assert_eq!(
            coordinator.finish(A, 7, 1, LiveTailRefreshOutcome::Unchanged),
            Vec::new()
        );
        assert_eq!(coordinator.mark_unproven(A, 8), Vec::new());
        assert_eq!(
            coordinator.finish(A, 8, 2, LiveTailRefreshOutcome::Unchanged),
            Vec::new()
        );
        assert_eq!(coordinator.mark_unproven(A, 8), Vec::new());
        assert_eq!(
            coordinator.freshness(&A),
            Some(LiveTailFreshnessState::Fresh { epoch: 8 })
        );
    }

    #[test]
    fn inactive_epoch_replacement_fences_old_finish_and_preserves_one_deferred_entry() {
        let mut coordinator = LiveTailRefreshCoordinator::new();

        assert_eq!(
            coordinator.activate(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 1,
                limit: 128,
            }]
        );
        assert_eq!(coordinator.mark_unproven(B, 8), Vec::new());
        assert_eq!(
            coordinator.finish(A, 7, 1, LiveTailRefreshOutcome::Unchanged),
            vec![LiveTailSchedulerAction::Start {
                key: B,
                epoch: 8,
                operation_generation: 2,
                limit: 128,
            }]
        );

        assert_eq!(
            coordinator.mark_unproven(B, 9),
            vec![LiveTailSchedulerAction::CancelNetwork {
                key: B,
                operation_generation: 2,
            }]
        );
        assert_eq!(
            coordinator.freshness(&B),
            Some(LiveTailFreshnessState::Deferred { epoch: 9 })
        );
        assert_eq!(
            coordinator.delayed.iter().copied().collect::<Vec<_>>(),
            vec![B]
        );
        assert_eq!(coordinator.delayed_members.len(), 1);
        assert!(coordinator.delayed_members.contains(&B));

        assert_eq!(
            coordinator.finish(B, 8, 2, LiveTailRefreshOutcome::Unchanged),
            Vec::new()
        );
        assert_eq!(
            coordinator.freshness(&B),
            Some(LiveTailFreshnessState::Deferred { epoch: 9 })
        );
        assert_eq!(
            coordinator.delayed.iter().copied().collect::<Vec<_>>(),
            vec![B]
        );

        assert_eq!(
            coordinator.activate(C, 10),
            vec![LiveTailSchedulerAction::Start {
                key: C,
                epoch: 10,
                operation_generation: 3,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.finish(C, 10, 3, LiveTailRefreshOutcome::Unchanged),
            vec![LiveTailSchedulerAction::Start {
                key: B,
                epoch: 9,
                operation_generation: 4,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.finish(B, 9, 4, LiveTailRefreshOutcome::Unchanged),
            Vec::new()
        );
    }

    #[test]
    fn failed_active_refresh_is_retryable_without_busy_loop() {
        let mut coordinator = LiveTailRefreshCoordinator::new();

        assert_eq!(
            coordinator.activate(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 1,
                limit: 128,
            }]
        );
        assert_eq!(coordinator.mark_unproven(B, 8), Vec::new());
        assert_eq!(
            coordinator.finish(A, 7, 1, LiveTailRefreshOutcome::Failed),
            vec![LiveTailSchedulerAction::Start {
                key: B,
                epoch: 8,
                operation_generation: 2,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.freshness(&A),
            Some(LiveTailFreshnessState::Retryable { epoch: 7 })
        );
        assert_eq!(
            coordinator.finish(B, 8, 2, LiveTailRefreshOutcome::Failed),
            Vec::new()
        );
        assert_eq!(
            coordinator.mark_unproven(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 3,
                limit: 128,
            }]
        );
    }

    #[test]
    fn delayed_rooms_run_one_at_a_time_in_fifo_order() {
        let mut coordinator = LiveTailRefreshCoordinator::new();

        assert_eq!(
            coordinator.activate(A, 7),
            vec![LiveTailSchedulerAction::Start {
                key: A,
                epoch: 7,
                operation_generation: 1,
                limit: 128,
            }]
        );
        assert_eq!(coordinator.mark_unproven(B, 8), Vec::new());
        assert_eq!(coordinator.mark_unproven(C, 9), Vec::new());
        assert_eq!(coordinator.mark_unproven(B, 8), Vec::new());

        assert_eq!(
            coordinator.finish(A, 7, 1, LiveTailRefreshOutcome::Advanced { events: 3 }),
            vec![LiveTailSchedulerAction::Start {
                key: B,
                epoch: 8,
                operation_generation: 2,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.finish(
                B,
                8,
                2,
                LiveTailRefreshOutcome::Detached {
                    events: 5,
                    historical_gap_remaining: true,
                },
            ),
            vec![LiveTailSchedulerAction::Start {
                key: C,
                epoch: 9,
                operation_generation: 3,
                limit: 128,
            }]
        );
        assert_eq!(
            coordinator.finish(C, 9, 3, LiveTailRefreshOutcome::Unchanged),
            Vec::new()
        );
    }
}
