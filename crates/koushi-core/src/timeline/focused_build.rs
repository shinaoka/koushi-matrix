//! #1146: cancellable, bounded preparation of `TimelineKind::Focused` actors.
//!
//! The SDK `TimelineFocus::Event` build may wait on a remote `/context`
//! request or the per-room focused-cache lock. It therefore runs as a
//! manager-owned task whose completion returns through the manager's select
//! loop, so the manager keeps admitting commands, navigation demand, cleanup
//! and other completions while a focused build is in flight.
//!
//! Each pending build is fenced by its actor-generation activation, which is
//! process-unique. Unsubscribe, navigation-demand retirement (supersession,
//! Home, room change, navigation deadline), the build timeout, and manager
//! shutdown (logout, account replacement) stop the manager's wait and roll
//! back its lease/activation; a late completion whose activation is no longer
//! pending is discarded and can never install an actor or publish a
//! projection.
//!
//! The SDK future itself is detached rather than aborted. Event-focused cache
//! creation (`EventCache` `event_focused` -> `EventFocusedCache::new` ->
//! `StateLock::try_insert_once_with`) registers its cache state before the
//! initial `/context` load and never unregisters it, so dropping the build
//! mid-load would make every later build of the same target fail with
//! `CacheStateAlreadyExists` for the rest of the session. A detached build
//! instead completes into the SDK focused cache, where a retry of the same
//! target queues behind it on the SDK's per-room focused-cache lock and then
//! reuses it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk_ui::timeline::Timeline;

use crate::executor;
use koushi_protocol::failure::TimelineFailureKind;
use koushi_protocol::ids::{RequestId, TimelineKey};

// BEGIN GENERATED SIBLING IMPORTS
use super::navigation::TimelineActorGenerationActivation;
use super::residency::SubscriptionReconcileTrigger;
// END GENERATED SIBLING IMPORTS

/// Upper bound for one focused SDK build. It stays below the AppActor
/// event-navigation deadline (`EVENT_NAVIGATION_TIMEOUT`, 15 s from the click,
/// which also covers the up-to-5 s target-event cache lookup), so the
/// manager's bounded failure normally settles the navigation and the AppActor
/// deadline remains the backstop.
pub(super) const FOCUSED_TIMELINE_BUILD_TIMEOUT: Duration = Duration::from_secs(10);

/// The prepared SDK timeline, before its actor is spawned on the manager.
pub(super) enum PreparedFocusedTimeline {
    Sdk(Arc<Timeline>),
    /// Sessionless manager fixtures install the residency test actor.
    #[cfg(any(test, feature = "test-hooks"))]
    TestActor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FocusedBuildFailure {
    Sdk,
    TimedOut,
    /// The task panicked before reporting.
    Interrupted,
}

impl FocusedBuildFailure {
    pub(super) fn timeline_failure_kind(self) -> TimelineFailureKind {
        match self {
            Self::TimedOut => TimelineFailureKind::Timeout,
            Self::Sdk | Self::Interrupted => TimelineFailureKind::Sdk,
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::Sdk => "failed",
            Self::TimedOut => "timeout",
            Self::Interrupted => "interrupted",
        }
    }
}

pub(super) struct FocusedBuildCompletion {
    pub(super) key: TimelineKey,
    pub(super) actor_generation: u64,
    pub(super) result: Result<PreparedFocusedTimeline, FocusedBuildFailure>,
}

/// Everything `handle_subscribe` admitted before the build, so the manager can
/// finish or roll back exactly as the former inline path did.
pub(super) struct PendingFocusedBuild {
    pub(super) activation: TimelineActorGenerationActivation,
    /// Latest-wins: a repeated subscribe for the same key while the build is
    /// pending adopts the newest request as the projection correlation.
    pub(super) request_id: RequestId,
    pub(super) emit_failure_terminal: bool,
    pub(super) lease_room_id: Option<OwnedRoomId>,
    pub(super) lease_added: bool,
    pub(super) reconcile_trigger: SubscriptionReconcileTrigger,
    pub(super) started: executor::Instant,
}

impl PendingFocusedBuild {
    pub(super) fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }
}

#[cfg(test)]
pub(super) type FocusedBuildTestGate =
    Arc<dyn Fn() -> BoxFuture<'static, Result<(), TimelineFailureKind>> + Send + Sync>;

#[derive(Default)]
pub(super) struct FocusedBuildSupervisor {
    pub(super) pending: HashMap<TimelineKey, PendingFocusedBuild>,
    pub(super) tasks: FuturesUnordered<BoxFuture<'static, FocusedBuildCompletion>>,
    /// Holds a sessionless build open at the SDK `build()` boundary.
    #[cfg(test)]
    pub(super) test_gate: Option<FocusedBuildTestGate>,
}

impl FocusedBuildSupervisor {
    /// Spawn one preparation task and register it as the pending build for
    /// `key`. The manager waits at most `FOCUSED_TIMELINE_BUILD_TIMEOUT`; the
    /// SDK task is detached, never aborted (see the module docs). The caller
    /// must not already have a pending build for this key.
    #[expect(
        clippy::too_many_arguments,
        reason = "the admitted subscribe state is moved into one pending record"
    )]
    pub(super) fn start(
        &mut self,
        key: TimelineKey,
        activation: TimelineActorGenerationActivation,
        request_id: RequestId,
        emit_failure_terminal: bool,
        lease_room_id: Option<OwnedRoomId>,
        lease_added: bool,
        reconcile_trigger: SubscriptionReconcileTrigger,
        build: BoxFuture<'static, Result<PreparedFocusedTimeline, FocusedBuildFailure>>,
    ) {
        debug_assert!(!self.pending.contains_key(&key));
        let actor_generation = activation.generation;
        let task = executor::spawn(build);
        let completion_key = key.clone();
        self.tasks.push(Box::pin(async move {
            // On timeout the JoinHandle is dropped, which detaches the task.
            let result = match executor::timeout(FOCUSED_TIMELINE_BUILD_TIMEOUT, task).await {
                Ok(joined) => joined.unwrap_or(Err(FocusedBuildFailure::Interrupted)),
                Err(_) => Err(FocusedBuildFailure::TimedOut),
            };
            FocusedBuildCompletion {
                key: completion_key,
                actor_generation,
                result,
            }
        }));
        record_focused_build("start", actor_generation, None);
        self.pending.insert(
            key,
            PendingFocusedBuild {
                activation,
                request_id,
                emit_failure_terminal,
                lease_room_id,
                lease_added,
                reconcile_trigger,
                started: executor::Instant::now(),
            },
        );
    }

    /// Adopt a repeated subscribe for a key whose build is still pending.
    pub(super) fn coalesce(
        &mut self,
        key: &TimelineKey,
        request_id: RequestId,
        emit_failure_terminal: bool,
    ) -> bool {
        let Some(pending) = self.pending.get_mut(key) else {
            return false;
        };
        pending.request_id = request_id;
        pending.emit_failure_terminal |= emit_failure_terminal;
        record_focused_build(
            "coalesced",
            pending.activation.generation,
            Some(pending.elapsed_ms()),
        );
        true
    }

    /// Remove the pending build that produced `completion`, or `None` when the
    /// completion is stale (cancelled, superseded, or already settled).
    pub(super) fn take_current(
        &mut self,
        completion: &FocusedBuildCompletion,
    ) -> Option<PendingFocusedBuild> {
        let current = self
            .pending
            .get(&completion.key)
            .is_some_and(|pending| pending.activation.generation == completion.actor_generation);
        if !current {
            record_focused_build("stale_discarded", completion.actor_generation, None);
            return None;
        }
        self.pending.remove(&completion.key)
    }

    /// Stop waiting for the pending build for `key`; its eventual completion
    /// is stale. The caller rolls back the returned lease/activation.
    pub(super) fn cancel(&mut self, key: &TimelineKey) -> Option<PendingFocusedBuild> {
        let pending = self.pending.remove(key)?;
        record_focused_build(
            "cancelled",
            pending.activation.generation,
            Some(pending.elapsed_ms()),
        );
        Some(pending)
    }

    pub(super) fn pending_keys(&self) -> impl Iterator<Item = &TimelineKey> {
        self.pending.keys()
    }

    /// Manager shutdown: forget every pending build and its completion.
    pub(super) fn cancel_all(&mut self) {
        for (_, pending) in self.pending.drain() {
            record_focused_build(
                "cancelled",
                pending.activation.generation,
                Some(pending.elapsed_ms()),
            );
        }
        self.tasks = FuturesUnordered::new();
    }
}

/// Identifier-free focused-build stage record. `build` is the process-local
/// actor-generation ordinal that correlates admission, completion and
/// cancellation of one build; it is not a Matrix identifier.
pub(super) fn record_focused_build(
    stage: &'static str,
    actor_generation: u64,
    elapsed_ms: Option<u128>,
) {
    let mut event = DiagnosticEvent::new(DiagnosticLevel::Debug, "core.timeline", "focused_build")
        .field(DiagnosticField::token("stage", stage))
        .field(DiagnosticField::count("build", actor_generation));
    if let Some(elapsed_ms) = elapsed_ms {
        event = event.field(DiagnosticField::milliseconds("duration_ms", elapsed_ms));
    }
    koushi_diagnostics::record(event);
}

pub(super) fn record_focused_build_settled(
    pending: &PendingFocusedBuild,
    result: Result<(), FocusedBuildFailure>,
) {
    let stage = match result {
        Ok(()) => "installed",
        Err(failure) => failure.token(),
    };
    record_focused_build(
        stage,
        pending.activation.generation,
        Some(pending.elapsed_ms()),
    );
}

/// Test-only view of one key's focused-build ownership inside a running
/// manager.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct FocusedBuildProbe {
    pub(super) pending_request_id: Option<RequestId>,
    pub(super) installed: bool,
    pub(super) room_leases: usize,
    pub(super) actor_generation: Option<u64>,
    /// Completion futures still polled by the manager (pending or stale).
    pub(super) in_flight: usize,
}

#[cfg(test)]
impl super::manager::TimelineManagerActor {
    pub(super) fn focused_build_probe(&self, key: &TimelineKey) -> FocusedBuildProbe {
        FocusedBuildProbe {
            pending_request_id: self
                .focused_builds
                .pending
                .get(key)
                .map(|pending| pending.request_id),
            installed: self.timelines.contains_key(key),
            room_leases: key
                .room_id()
                .parse::<OwnedRoomId>()
                .ok()
                .and_then(|room_id| self.subscribed_room_leases.get(&room_id).copied())
                .unwrap_or(0),
            actor_generation: self.timeline_actor_generations.current_generation(key),
            in_flight: self.focused_builds.tasks.len(),
        }
    }
}

#[cfg(test)]
mod tests;
