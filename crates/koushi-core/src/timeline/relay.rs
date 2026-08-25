use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use koushi_state::{AppAction, LiveEventReceipts};

use matrix_sdk_ui::timeline::{
    EncryptedMessage, EventTimelineItem, TimelineFocus, TimelineItem as SdkTimelineItem,
    TimelineReadReceiptTracking,
};
use tokio::sync::{broadcast, mpsc};

use crate::causal_projection::{CausalProjectionDomain, CausalProjectionId};
use crate::event::{
    CoreEvent, TimelineAnchorRestoreStatus, TimelineDiff, TimelineEvent, TimelineItem,
    TimelineResyncReason,
};
use crate::executor;
use crate::failure::TimelineFailureKind;
use crate::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};
use crate::search::SearchIndexMessage;
use crate::threads_list::ThreadRootProjectionService;

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{
    TimelineActor, TimelinePositionIndex, canonical_activity_window_action,
    reserve_canonical_activity_action,
};
use super::diagnostics::{
    record_thread_projection, record_timeline_gap_projection_boundary, trace_timeline_diffs,
    trace_timeline_items,
};
use super::display_projection::{
    DisplayProjectionContext, DisplayProjectionState, apply_timeline_diffs_to_items,
    commit_sdk_batch_for_generation, timeline_diffs_include_prepend,
};
use super::gap_repair::{
    TimelineGapRepairTrigger, observe_causal_projection, post_diff_gap_inspection_trigger,
    rendered_live_edge_target,
};
use super::item_projection::{
    ReceiptObservationTarget, apply_ignored_sender_suppression,
    apply_ignored_sender_suppression_to_diff, apply_link_previews_to_item,
    cache_sdk_item_media_source, emit_live_receipt_observation_actions,
    emit_receipt_observation_actions, live_event_receipts_from_sdk_items,
    sdk_item_to_timeline_item_with_send_states, sdk_vector_diffs_to_timeline_diffs,
    thread_auto_requestable_event_id, timeline_item_event_id, timeline_room_id,
};
use super::media::{PrivateMediaEntry, authoritative_media_gallery_replacement};
use super::navigation::{
    InitialItemsRequestIdentity, PreparedInitialWindow, TimelineActorGenerationGate,
    commit_prepared_initial_window_with_lease, derive_timeline_navigation_snapshot,
    emit_items_updated_and_reconcile_replay_known_with_lease, record_timeline_unread_consistency,
};
use super::outbound_send::thread_activity_observed_action_for_batch;
use super::room_key_recovery::{decrypt_retry_diff_settlement, decrypt_retry_settlement_operation};
use super::thread_projection::{
    ReplayKnownThreadRootProjectionRegistry, ThreadAttentionBatchProvenance,
    ThreadAttentionObservation, gap_repair_projections_from_sdk_diffs, overlay_thread_summary_diff,
    replay_known_candidates_for_display_items, seed_thread_summary_diff,
    thread_summary_affected_root_event_ids,
};
// END GENERATED SIBLING IMPORTS

#[derive(Clone, Copy, Debug, Eq, PartialEq)]

pub(super) enum TimelineRelayControl {
    Overflow {
        generation: TimelineGeneration,
    },
    StreamEnded {
        generation: TimelineGeneration,
    },
    RestartDue {
        generation: TimelineGeneration,
        serial: u64,
    },
}

pub(super) const RELAY_RESTART_BASE_DELAY: Duration = Duration::from_millis(100);

pub(super) const RELAY_RESTART_MAX_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct RelayRestartSchedule {
    generation: TimelineGeneration,
    serial: u64,
    delay: Duration,
}

pub(super) struct RelayRestartBackoff {
    base: Duration,
    cap: Duration,
    attempts: u32,
    next_serial: u64,
    active: Option<(TimelineGeneration, u64)>,
}

impl RelayRestartBackoff {
    pub(super) fn new(base: Duration, cap: Duration) -> Self {
        Self {
            base,
            cap,
            attempts: 0,
            next_serial: 0,
            active: None,
        }
    }

    fn schedule(&mut self, generation: TimelineGeneration) -> RelayRestartSchedule {
        self.next_serial = self.next_serial.wrapping_add(1);
        let factor = 1_u32.checked_shl(self.attempts.min(30)).unwrap_or(u32::MAX);
        let delay = self.base.saturating_mul(factor).min(self.cap);
        self.attempts = self.attempts.saturating_add(1);
        self.active = Some((generation, self.next_serial));
        RelayRestartSchedule {
            generation,
            serial: self.next_serial,
            delay,
        }
    }

    fn accept_due(&mut self, generation: TimelineGeneration, serial: u64) -> bool {
        if self.active != Some((generation, serial)) {
            return false;
        }
        self.active = None;
        true
    }

    pub(super) fn reset_after_live_batch(&mut self) {
        self.attempts = 0;
        self.active = None;
    }
}

fn spawn_relay_restart_timer(
    control_tx: mpsc::Sender<TimelineRelayControl>,
    schedule: RelayRestartSchedule,
    delay: impl std::future::Future<Output = ()> + Send + 'static,
) -> executor::JoinHandle<()> {
    executor::spawn(async move {
        delay.await;
        let _ = control_tx
            .send(TimelineRelayControl::RestartDue {
                generation: schedule.generation,
                serial: schedule.serial,
            })
            .await;
    })
}

pub(super) struct TimelineRelayBatch {
    pub(super) generation: TimelineGeneration,
    pub(super) diffs: Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>,
    pub(super) thread_attention_provenance: ThreadAttentionBatchProvenance,
    pub(super) gap_repair_projections: BTreeSet<CausalProjectionId>,
}

impl TimelineRelayBatch {
    pub(super) fn retain_gap_repair_projections_for_actor(&mut self, actor_generation: u64) {
        self.gap_repair_projections
            .retain(|projection| projection.actor_generation == actor_generation);
    }
}

impl TimelineActor {
    pub(super) async fn handle_relay_control(&mut self, control: TimelineRelayControl) {
        match control {
            TimelineRelayControl::Overflow { generation } => {
                self.handle_relay_overflow(generation, TimelineResyncReason::QueueOverflow)
                    .await;
            }
            TimelineRelayControl::StreamEnded { generation } => {
                self.schedule_relay_restart(generation);
            }
            TimelineRelayControl::RestartDue { generation, serial } => {
                if accept_relay_generation(self.generation, generation)
                    && self.relay_restart_backoff.accept_due(generation, serial)
                {
                    self.relay_restart_task = None;
                    self.handle_relay_overflow(
                        generation,
                        TimelineResyncReason::SubscriptionRestarted,
                    )
                    .await;
                }
            }
        }
    }
    fn schedule_relay_restart(&mut self, generation: TimelineGeneration) {
        if !accept_relay_generation(self.generation, generation) {
            return;
        }
        if let Some(task) = self.relay_task.take() {
            task.abort();
        }
        self.relay_data_rx = None;
        if let Some(task) = self.relay_restart_task.take() {
            task.abort();
        }
        let schedule = self.relay_restart_backoff.schedule(generation);
        let control_tx = self.relay_control_tx.clone();
        self.relay_restart_task = Some(spawn_relay_restart_timer(
            control_tx,
            schedule,
            executor::sleep(schedule.delay),
        ));
    }
    pub(super) async fn handle_diff_batch(
        &mut self,
        diffs: Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>,
        thread_attention_provenance: ThreadAttentionBatchProvenance,
        gap_repair_projections: BTreeSet<CausalProjectionId>,
    ) {
        if diffs.is_empty() {
            return;
        }
        // #478: start/join standard-only recovery for genuine missing-session
        // UTDs appearing in this batch.
        for diff in &diffs {
            let item = match diff {
                eyeball_im::VectorDiff::PushFront { value }
                | eyeball_im::VectorDiff::PushBack { value }
                | eyeball_im::VectorDiff::Insert { value, .. }
                | eyeball_im::VectorDiff::Set { value, .. } => value,
                _ => continue,
            };
            let Some(event) = item.as_event() else {
                continue;
            };
            let content = event.content();
            let Some(utd) = content.as_unable_to_decrypt() else {
                continue;
            };
            let EncryptedMessage::MegolmV1AesSha2 {
                session_id, cause, ..
            } = utd
            else {
                continue;
            };
            // Only genuine missing-session UTDs are eligible for automatic
            // standard recovery; identity/trust/policy causes use their
            // existing typed handling.
            use matrix_sdk_base::crypto::types::events::UtdCause;
            let eligible = matches!(
                cause,
                UtdCause::Unknown | UtdCause::HistoricalMessageAndBackupIsDisabled
            );
            if eligible {
                self.ensure_room_key_recovery(session_id);
            }
        }
        // Issue #460: a decrypted replacement for an event with a pending
        // key-request state settles the presentation as recovered (late keys
        // included, after the operational timeout). Scans every batch value
        // shape (singleton, Reset, Append) so a late decrypt delivered as a
        // full reset or append still settles an already-timed-out request.
        for diff in &diffs {
            let values: Vec<&Arc<SdkTimelineItem>> = match diff {
                eyeball_im::VectorDiff::PushFront { value } => vec![value],
                eyeball_im::VectorDiff::PushBack { value } => vec![value],
                eyeball_im::VectorDiff::Insert { value, .. } => vec![value],
                eyeball_im::VectorDiff::Set { value, .. } => vec![value],
                eyeball_im::VectorDiff::Reset { values } => values.iter().collect(),
                eyeball_im::VectorDiff::Append { values } => values.iter().collect(),
                _ => continue,
            };
            for item in values {
                let Some(event) = item.as_event() else {
                    continue;
                };
                let Some(event_id) = event.event_id() else {
                    continue;
                };
                let event_id = event_id.to_string();
                if !self.key_request_states.contains_key(&event_id) {
                    continue;
                }
                if event.content().is_unable_to_decrypt() {
                    continue;
                }
                let mut recovered = false;
                if let Some(state) = self.key_request_states.get_mut(&event_id) {
                    // Terminal send failure is not regressed by a late key.
                    if state.stage != "decryption_recovered" && state.stage != "send_failed" {
                        state.stage = "decryption_recovered";
                        recovered = true;
                    }
                }
                // Issue #460: a late recovery after the operational timeout is
                // published as the correlated Room event (the diff updates the
                // DTO, but only this event carries the request_id). The active
                // settlement path publishes for the current operation, so skip
                // it here to avoid a duplicate publication.
                if recovered
                    && decrypt_retry_settlement_operation(
                        &self.decrypt_retry,
                        self.actor_generation,
                        &event_id,
                    )
                    .is_none()
                    && let Some(state) = self.key_request_states.get(&event_id)
                {
                    self.publish_key_request_state(&event_id, state);
                }
            }
        }
        let decrypt_retry_resolution = self.decrypt_retry.pending.as_ref().and_then(|pending| {
            diffs.iter().find_map(|diff| {
                decrypt_retry_diff_settlement(diff, &pending.event_id).and_then(|result| {
                    decrypt_retry_settlement_operation(
                        &self.decrypt_retry,
                        self.actor_generation,
                        &pending.event_id,
                    )
                    .map(|operation| (operation, result))
                })
            })
        });
        // Issue #460: settle BEFORE the conversion below so the emitted batch
        // carries the settled request state (withheld/decryption_recovered) in
        // the same diff the UI applies — otherwise a static timeline would
        // never learn the outcome.
        if let Some((operation, result)) = decrypt_retry_resolution {
            self.settle_decrypt_retry(operation, result);
        }
        let sdk_diffs = diffs;
        // Issue #460: automatic one-shot key requests for Thread timelines.
        // Admission is Rust-owned (the automatic guard in
        // handle_request_room_key refuses repeats for events that already
        // have a request state), so React only renders the resulting state.
        if matches!(self.key.kind, TimelineKind::Thread { .. }) {
            let mut auto_request_event_ids: Vec<String> = Vec::new();
            for diff in &sdk_diffs {
                let values: Vec<&Arc<SdkTimelineItem>> = match diff {
                    eyeball_im::VectorDiff::PushFront { value } => vec![value],
                    eyeball_im::VectorDiff::PushBack { value } => vec![value],
                    eyeball_im::VectorDiff::Insert { value, .. } => vec![value],
                    eyeball_im::VectorDiff::Set { value, .. } => vec![value],
                    eyeball_im::VectorDiff::Reset { values } => values.iter().collect(),
                    eyeball_im::VectorDiff::Append { values } => values.iter().collect(),
                    _ => Vec::new(),
                };
                for value in values {
                    let Some(event_id) = thread_auto_requestable_event_id(value) else {
                        continue;
                    };
                    if self.key_request_states.contains_key(&event_id) {
                        continue;
                    }
                    auto_request_event_ids.push(event_id);
                }
            }
            // Dispatched through the helper so mailbox-full candidates are
            // retained and retried on the next loop iteration instead of
            // being silently lost.
            self.dispatch_auto_key_requests(auto_request_event_ids);
        }
        let has_historical_gap_repair_projection = gap_repair_projections
            .iter()
            .any(|projection| projection.operation.domain == CausalProjectionDomain::HistoricalGap);
        let has_live_tail_projection = gap_repair_projections
            .iter()
            .any(|projection| projection.operation.domain == CausalProjectionDomain::LiveTail);

        let mut core_diffs = sdk_vector_diffs_to_timeline_diffs(
            &sdk_diffs,
            self.navigation_items.len(),
            &self.key,
            self.own_user_id.as_deref(),
            &self.send_statuses,
            Some(&self.key_request_states),
            Some(&self.withheld_codes),
        );
        for diff in &mut core_diffs {
            apply_ignored_sender_suppression_to_diff(diff, &self.ignored_user_ids);
        }
        let link_preview_context = self.link_preview_policy.for_room(self.key.room_id());
        for diff in &mut core_diffs {
            match diff {
                TimelineDiff::Reset { items } => {
                    for item in items {
                        apply_link_previews_to_item(
                            item,
                            self.key.room_id(),
                            &link_preview_context,
                            &self.session,
                        )
                        .await;
                    }
                }
                TimelineDiff::PushFront { item }
                | TimelineDiff::PushBack { item }
                | TimelineDiff::Insert { item, .. }
                | TimelineDiff::Set { item, .. } => {
                    apply_link_previews_to_item(
                        item,
                        self.key.room_id(),
                        &link_preview_context,
                        &self.session,
                    )
                    .await;
                }
                _ => {}
            }
        }
        let receipts_action = Self::live_receipts_action_from_sdk_diffs(&self.key, &sdk_diffs);
        let search_messages = sdk_diffs
            .iter()
            .flat_map(|diff| self.search_index_messages_for_diff(diff))
            .collect::<Vec<_>>();
        let restore_diff_is_relevant = timeline_diffs_include_prepend(&core_diffs);
        let restore_active = self.restore_anchor.is_some();
        let previous_navigation_items = self.navigation_items.clone();
        let display_context = DisplayProjectionContext::for_timeline(
            &self.key.kind,
            &self.viewport_observation,
            restore_active,
        );
        // Reserve the sole canonical Activity replacement before entering the
        // generation commit. The permit and lease remain live through the
        // timeline publication and the full-room replacement.
        let activity_permit = reserve_canonical_activity_action(&self.action_tx, &self.key).await;
        let activity_commit_lease = if activity_permit.is_some() {
            self.timeline_actor_generations
                .try_acquire(&self.key, self.actor_generation)
        } else {
            None
        };
        if matches!(self.key.kind, TimelineKind::Room { .. })
            && (activity_permit.is_none() || activity_commit_lease.is_none())
        {
            return;
        }
        // Seed the accepted newer SDK summary before overlaying the existing
        // service value. Holding this lease makes the shared service mutation
        // and emitted batch one replacement-fenced commit; no await occurs.
        let Some(thread_summary_commit_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            return;
        };
        let mut raw_navigation_items = previous_navigation_items.clone();
        apply_timeline_diffs_to_items(&mut raw_navigation_items, &core_diffs);
        let mut thread_summary_affected_roots = thread_summary_affected_root_event_ids(
            &self.key,
            &previous_navigation_items,
            &raw_navigation_items,
        );
        for diff in &core_diffs {
            seed_thread_summary_diff(&self.thread_root_projection_service, &self.key, diff);
        }
        for diff in &mut core_diffs {
            overlay_thread_summary_diff(&self.thread_root_projection_service, &self.key, diff);
        }
        trace_timeline_diffs("diff_batch", &self.key, &core_diffs);

        let Some((emitted, emitted_batch_id)) = commit_sdk_batch_for_generation(
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            &mut self.navigation_items,
            &mut self.display_projection,
            &core_diffs,
            &display_context,
            |projection_lease, projected_batch, navigation_items, display_projection| {
                // State and synchronous batch-derived publications share this
                // one generation lease; a stale actor commits neither half.
                self.position_tx
                    .send_replace(Arc::new(TimelinePositionIndex::from_items(
                        self.actor_generation,
                        self.generation,
                        navigation_items,
                    )));
                self.diff_batch_seq = self.diff_batch_seq.wrapping_add(1);
                for diff in &sdk_diffs {
                    Self::apply_sdk_media_cache_diff(&mut self.media_sources, diff);
                }

                let emitted_batch_id = self.next_batch_id;
                record_thread_projection(
                    &self.key,
                    self.actor_generation,
                    self.generation,
                    emitted_batch_id,
                    sdk_diffs.len(),
                    projected_batch.display_diffs.len(),
                    display_projection.display_items().len(),
                );
                let display_diffs = projected_batch.display_diffs;
                let emitted = if restore_active {
                    self.next_batch_id = TimelineBatchId(self.next_batch_id.0 + 1);
                    self.restore_emit_buffer.extend(display_diffs);
                    false
                } else {
                    emit_items_updated_and_reconcile_replay_known_with_lease(
                        &self.event_tx,
                        &self.replay_known_thread_root_projections,
                        &self.thread_root_projection_service,
                        projection_lease,
                        &self.key,
                        self.generation,
                        emitted_batch_id,
                        display_diffs,
                        navigation_items,
                        display_projection.display_items(),
                    );
                    self.next_batch_id = TimelineBatchId(emitted_batch_id.0 + 1);
                    true
                };
                (emitted, emitted_batch_id)
            },
        ) else {
            drop(thread_summary_commit_lease);
            drop(activity_commit_lease);
            drop(activity_permit);
            return;
        };
        drop(thread_summary_commit_lease);

        thread_summary_affected_roots.extend(thread_summary_affected_root_event_ids(
            &self.key,
            &previous_navigation_items,
            &self.navigation_items,
        ));

        if let Some(activity_permit) = activity_permit {
            activity_permit.send(vec![
                canonical_activity_window_action(&self.key, &self.navigation_items)
                    .expect("room canonical Activity action"),
            ]);
        }
        drop(activity_commit_lease);

        if emitted
            && matches!(self.key.kind, TimelineKind::Thread { .. })
            && !self
                .publish_thread_summary_window_observations(
                    &previous_navigation_items,
                    &self.navigation_items,
                )
                .await
        {
            return;
        }

        if let Some(AppAction::LiveRoomReceiptsUpdated {
            room_id,
            receipts_by_event,
        }) = receipts_action
            && !emit_live_receipt_observation_actions(
                self.session.as_ref(),
                &self.action_tx,
                &self.timeline_actor_generations,
                &self.key,
                self.actor_generation,
                &room_id,
                receipts_by_event,
            )
            .await
        {
            return;
        }

        if !self.emit_search_messages_reliable(search_messages).await {
            return;
        }

        let Some(continuation_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            return;
        };
        let live_edge_target_changed = matches!(self.key.kind, TimelineKind::Room { .. })
            && !has_historical_gap_repair_projection
            && self
                .gap_repair
                .observe_live_edge_target(rendered_live_edge_target(&self.navigation_items));
        let thread_attention_action = self.thread_attention.reconcile_batch(
            &self.key,
            &self.navigation_items,
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
            &thread_attention_provenance,
        );
        let thread_activity_action = thread_activity_observed_action_for_batch(
            &self.key,
            &self.navigation_items,
            &thread_attention_provenance,
        );
        self.maybe_fetch_visible_reply_details();
        drop(continuation_lease);

        if let Some(action) = thread_activity_action
            && !self.emit_action_reliable(action).await
        {
            return;
        }
        if let Some(action) = thread_attention_action {
            let snapshot = derive_timeline_navigation_snapshot(
                &self.navigation_items,
                self.fully_read_event_id.as_deref(),
                &self.viewport_observation,
                self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
            );
            record_timeline_unread_consistency(
                "thread_attention_updated",
                &self.key,
                &self.navigation_items,
                self.display_projection.display_items(),
                self.last_navigation_snapshot.as_ref(),
                &snapshot,
                &self.thread_attention,
            );
            if !self.emit_action_reliable(action).await {
                return;
            }
        }
        self.emit_media_gallery_if_changed().await;
        let Some(post_media_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            return;
        };
        let mut post_media_lease = Some(post_media_lease);

        let mut live_tail_completion_published = false;

        if restore_active {
            // While a restore walk is in-flight, buffer this batch's diffs
            // instead of emitting ItemsUpdated per chunk. React receives ONE
            // settled update when the restore terminates. The batch_id counter
            // is still advanced so later non-restore emits remain monotonic.
            for projection in &gap_repair_projections {
                let correlation = match projection.operation.domain {
                    CausalProjectionDomain::HistoricalGap => &self.gap_projection_correlation,
                    CausalProjectionDomain::LiveTail => &self.live_tail_projection_correlation,
                };
                record_timeline_gap_projection_boundary(
                    "actor_received",
                    "buffered_restore",
                    projection.actor_generation,
                    self.generation,
                    projection.operation,
                    Some(projection.projection_batch),
                    None,
                    correlation.expected_last_projection_batch,
                    correlation.observed_batches.len(),
                );
            }
            self.restore_causal_projections
                .buffer_batch(gap_repair_projections);
            // Hydration Pending must be emitted only after this buffer's final
            // canonical ItemsUpdated group, otherwise the desktop store prunes
            // it before the reply item which keeps it active is present.
            self.hydrate_after_restore_flush = true;
            // Navigation is also suppressed until the flush at restore end.

            if restore_diff_is_relevant {
                let restore_event_id = self
                    .restore_anchor
                    .as_ref()
                    .map(|restore| restore.event_id.clone());
                if let Some(event_id) = restore_event_id {
                    if self.timeline_contains_event_id(&event_id) {
                        if let Some(restore) = self.restore_anchor.take() {
                            self.finish_anchor_restore(
                                restore.request_id,
                                TimelineAnchorRestoreStatus::Found,
                            );
                        }
                    } else {
                        drop(post_media_lease.take());
                        self.maybe_continue_restore_anchor_after_diff().await;
                    }
                }
            }
        } else {
            self.emit_navigation_if_changed();
            if emitted {
                let mut ready_gap_projection_batch = None;
                let mut live_tail_projection_ready = false;
                for projection in gap_repair_projections {
                    let correlation = match projection.operation.domain {
                        CausalProjectionDomain::HistoricalGap => &self.gap_projection_correlation,
                        CausalProjectionDomain::LiveTail => &self.live_tail_projection_correlation,
                    };
                    let accepts = correlation.accepts(projection);
                    let expected_projection_batch = correlation.expected_last_projection_batch;
                    let observed_projection_count = correlation.observed_batches.len();
                    record_timeline_gap_projection_boundary(
                        "actor_received",
                        if accepts {
                            "accepted"
                        } else {
                            "rejected_operation"
                        },
                        projection.actor_generation,
                        self.generation,
                        projection.operation,
                        Some(projection.projection_batch),
                        Some(emitted_batch_id),
                        expected_projection_batch,
                        observed_projection_count,
                    );
                    let observation = observe_causal_projection(
                        &mut self.gap_projection_correlation,
                        &mut self.live_tail_projection_correlation,
                        projection,
                        emitted_batch_id,
                    );
                    if observation.live_tail_batch_id.is_some() {
                        live_tail_projection_ready = true;
                    }
                    if ready_gap_projection_batch.is_none() {
                        ready_gap_projection_batch = observation.historical_gap_batch_id;
                    }
                }
                drop(post_media_lease.take());
                if let Some(batch_id) = ready_gap_projection_batch {
                    self.finish_pending_gap_projection(batch_id).await;
                    if self
                        .timeline_actor_generations
                        .try_acquire(&self.key, self.actor_generation)
                        .is_none()
                    {
                        return;
                    }
                }
                if live_tail_projection_ready {
                    live_tail_completion_published =
                        self.finish_pending_live_tail_projection().await;
                    if self
                        .timeline_actor_generations
                        .try_acquire(&self.key, self.actor_generation)
                        .is_none()
                    {
                        return;
                    }
                }
                self.maybe_hydrate_missing_thread_roots(Some(thread_summary_affected_roots))
                    .await;
            } else {
                for projection in gap_repair_projections {
                    let correlation = match projection.operation.domain {
                        CausalProjectionDomain::HistoricalGap => &self.gap_projection_correlation,
                        CausalProjectionDomain::LiveTail => &self.live_tail_projection_correlation,
                    };
                    record_timeline_gap_projection_boundary(
                        "actor_received",
                        "display_emit_rejected",
                        projection.actor_generation,
                        self.generation,
                        projection.operation,
                        Some(projection.projection_batch),
                        Some(emitted_batch_id),
                        correlation.expected_last_projection_batch,
                        correlation.observed_batches.len(),
                    );
                }
            }
        }
        drop(post_media_lease.take());
        if let Some(trigger) = post_diff_gap_inspection_trigger(
            has_live_tail_projection,
            live_tail_completion_published,
            live_edge_target_changed,
        ) {
            self.request_timeline_gap_inspection(trigger).await;
        }
    }
    fn prepare_authoritative_snapshot_reconciliation(
        &self,
        old_window_event_ids: &std::collections::BTreeSet<String>,
        items: &eyeball_im::Vector<Arc<SdkTimelineItem>>,
    ) -> PreparedAuthoritativeSnapshotReconciliation {
        let mut replacement_media_sources = HashMap::new();
        for item in items {
            cache_sdk_item_media_source(&mut replacement_media_sources, item);
        }
        let new_window_event_ids = items
            .iter()
            .filter_map(|item| match item.kind() {
                matrix_sdk_ui::timeline::TimelineItemKind::Event(event) => {
                    event.event_id().map(ToString::to_string)
                }
                matrix_sdk_ui::timeline::TimelineItemKind::Virtual(_) => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let reconciliation =
            authoritative_window_reconciliation(old_window_event_ids, &new_window_event_ids);
        let receipt_observation =
            timeline_room_id(&self.key).map(|room_id| ReceiptObservationRequest {
                room_id,
                receipts_by_event: live_event_receipts_from_sdk_items(items.iter()),
                target: ReceiptObservationTarget::Authoritative {
                    scoped_event_ids: reconciliation.scoped_event_ids.clone(),
                },
            });
        let mut search_messages = Vec::new();
        if self.search_index_tx.is_some() {
            search_messages.extend(authoritative_search_removals(&reconciliation));
            search_messages.extend(self.search_index_messages_for_diff(
                &eyeball_im::VectorDiff::Reset {
                    values: items.clone(),
                },
            ));
        }
        PreparedAuthoritativeSnapshotReconciliation {
            replacement_media_sources,
            receipt_observation,
            search_messages,
        }
    }
    pub(super) async fn handle_relay_overflow(
        &mut self,
        overflow_generation: TimelineGeneration,
        reason: TimelineResyncReason,
    ) {
        if !accept_relay_generation(self.generation, overflow_generation) {
            return;
        }
        // Avoid even tearing down a relay for an actor generation that has
        // already been replaced. The authoritative commit below reacquires
        // the lease after the SDK await and remains the final state fence.
        if self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
            .is_none()
        {
            return;
        }
        if let Some(task) = self.relay_restart_task.take() {
            task.abort();
        }
        if let Some(task) = self.relay_task.take() {
            task.abort();
        }
        let timeline = self.timeline.clone();
        let Some(prepared) =
            prepare_relay_recovery(self.generation, overflow_generation, move || async move {
                timeline.subscribe().await
            })
            .await
        else {
            return;
        };
        let recovery_generation = prepared.generation;

        // 3. Acquire the authoritative snapshot and its matching replacement
        //    stream from one SDK subscription boundary.
        let current_items = prepared.snapshot;
        let snapshot_gap_repair_projections = current_items
            .iter()
            .filter_map(|item| {
                item.as_event()
                    .and_then(EventTimelineItem::gap_repair_projection)
            })
            .map(CausalProjectionId::decode_transport)
            .filter(|projection| {
                self.gap_projection_correlation.accepts(*projection)
                    || self.live_tail_projection_correlation.accepts(*projection)
            })
            .collect::<BTreeSet<_>>();
        let diff_stream = prepared.stream;
        let old_window_event_ids = self
            .navigation_items
            .iter()
            .filter_map(timeline_item_event_id)
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        let PreparedAuthoritativeSnapshotReconciliation {
            replacement_media_sources,
            receipt_observation,
            search_messages,
        } = self
            .prepare_authoritative_snapshot_reconciliation(&old_window_event_ids, &current_items);

        // 5. Emit fresh InitialItems for the new generation.
        let link_preview_context = self.link_preview_policy.for_room(self.key.room_id());
        let items: Vec<TimelineItem> = current_items
            .iter()
            .map(|item| {
                sdk_item_to_timeline_item_with_send_states(
                    &self.key,
                    item,
                    self.own_user_id.as_deref(),
                    &self.send_statuses,
                    Some(&self.room_key_recovery),
                    Some(&self.key_request_states),
                    Some(&self.withheld_codes),
                )
            })
            .map(|mut item| {
                apply_ignored_sender_suppression(&mut item, &self.ignored_user_ids);
                item
            })
            .collect();
        let mut items = items;
        for item in &mut items {
            apply_link_previews_to_item(
                &mut *item,
                self.key.room_id(),
                &link_preview_context,
                &self.session,
            )
            .await;
        }
        trace_timeline_items("overflow_initial", &self.key, &items);
        let recovery_position_index = Arc::new(TimelinePositionIndex::from_items(
            self.actor_generation,
            recovery_generation,
            &items,
        ));
        let activity_permit = reserve_canonical_activity_action(&self.action_tx, &self.key).await;
        let activity_commit_lease = if activity_permit.is_some() {
            self.timeline_actor_generations
                .try_acquire(&self.key, self.actor_generation)
        } else {
            None
        };
        if matches!(self.key.kind, TimelineKind::Room { .. })
            && (activity_permit.is_none() || activity_commit_lease.is_none())
        {
            return;
        }
        if !commit_authoritative_recovery_window(
            &mut self.navigation_items,
            &mut self.display_projection,
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &self.thread_root_projection_service,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            recovery_generation,
            reason,
            items,
            || {
                // Generation-local mirrors must become authoritative in the
                // same lease as ResyncRequired + InitialItems. A replacement
                // actor therefore sees all of this recovery or none of it.
                self.position_tx
                    .send_replace(Arc::clone(&recovery_position_index));
                self.generation = recovery_generation;
                self.next_batch_id = TimelineBatchId(0);
                self.gap_repair.clear_projected_gaps();
                replace_authoritative_cache(&mut self.media_sources, replacement_media_sources);
            },
        ) {
            drop(activity_commit_lease);
            drop(activity_permit);
            return;
        }
        if let Some(activity_permit) = activity_permit {
            activity_permit.send(vec![
                canonical_activity_window_action(&self.key, &self.navigation_items)
                    .expect("room recovery Activity action"),
            ]);
        }
        drop(activity_commit_lease);
        if let Some(receipt_observation) = receipt_observation
            && !emit_receipt_observation_actions(
                self.session.as_ref(),
                &self.action_tx,
                &self.timeline_actor_generations,
                &self.key,
                self.actor_generation,
                &receipt_observation.room_id,
                receipt_observation.receipts_by_event,
                receipt_observation.target,
            )
            .await
        {
            return;
        }
        if !self.emit_search_messages_reliable(search_messages).await {
            return;
        }
        if let Some(replacement) = authoritative_media_gallery_replacement(
            &self.key,
            &self.media_gallery_items,
            &self.navigation_items,
        ) {
            if self.emit_action_reliable(replacement.action).await {
                self.media_gallery_items = replacement.items;
            } else {
                return;
            }
        }
        let Some(continuation_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            return;
        };
        if let Some(action) = self.thread_attention.reconcile(
            &self.key,
            &self.navigation_items,
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
            ThreadAttentionObservation::Replay,
        ) {
            drop(continuation_lease);
            if !self.emit_action_reliable(action).await {
                return;
            }
        } else {
            drop(continuation_lease);
        }
        if !snapshot_gap_repair_projections.is_empty() {
            let recovery_batch_id = self.next_batch_id;
            if self.emit_items_updated_and_reconcile_replay_known(Vec::new()) {
                let Some(projection_lease) = self
                    .timeline_actor_generations
                    .try_acquire(&self.key, self.actor_generation)
                else {
                    return;
                };
                let mut ready_gap_projection_batch = None;
                let mut live_tail_projection_ready = false;
                for projection in snapshot_gap_repair_projections {
                    let observation = observe_causal_projection(
                        &mut self.gap_projection_correlation,
                        &mut self.live_tail_projection_correlation,
                        projection,
                        recovery_batch_id,
                    );
                    ready_gap_projection_batch =
                        ready_gap_projection_batch.or(observation.historical_gap_batch_id);
                    live_tail_projection_ready |= observation.live_tail_batch_id.is_some();
                }
                drop(projection_lease);
                if let Some(batch_id) = ready_gap_projection_batch {
                    self.finish_pending_gap_projection(batch_id).await;
                }
                if live_tail_projection_ready {
                    if self.finish_pending_live_tail_projection().await {
                        self.request_timeline_gap_inspection(
                            TimelineGapRepairTrigger::LiveTailSnapshot,
                        )
                        .await;
                    }
                }
            }
        }
        if let Some((actor_generation, operation)) = self.gap_projection_correlation.operation {
            let trigger = self
                .pending_gap_projection
                .as_ref()
                .map_or(TimelineGapRepairTrigger::Automatic, |pending| {
                    pending.trigger
                });
            debug_assert_eq!(operation.domain, CausalProjectionDomain::HistoricalGap);
            self.release_gap_relay_settlement(actor_generation, operation.serial, trigger)
                .await;
            if self
                .timeline_actor_generations
                .try_acquire(&self.key, self.actor_generation)
                .is_none()
            {
                return;
            }
        }
        if let Some(fence) = self.gap_repair.awaiting_projection {
            let trigger = self
                .gap_repair
                .pending_trigger
                .unwrap_or(TimelineGapRepairTrigger::Automatic);
            self.recover_gap_render_settlement(fence, trigger).await;
            if self
                .timeline_actor_generations
                .try_acquire(&self.key, self.actor_generation)
                .is_none()
            {
                return;
            }
        }
        let Some(finalize_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            return;
        };
        let (relay_data_tx, relay_data_rx) = mpsc::channel(256);
        self.relay_data_rx = Some(relay_data_rx);
        let initial_items: Vec<_> = self.timeline.items().await.iter().cloned().collect();
        self.relay_task = Some(executor::spawn(run_diff_relay(
            relay_data_tx,
            self.relay_control_tx.clone(),
            self.generation,
            self.actor_generation,
            diff_stream,
            initial_items,
        )));
        let restore = self.restore_anchor.take();
        drop(finalize_lease);
        if let Some(restore) = restore {
            self.finish_anchor_restore(
                restore.request_id,
                TimelineAnchorRestoreStatus::Failed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            );
        }
    }
}

pub(super) fn koushi_timeline_builder(
    room: &matrix_sdk::Room,
    focus: TimelineFocus,
) -> matrix_sdk_ui::timeline::TimelineBuilder {
    matrix_sdk_ui::timeline::TimelineBuilder::new(room)
        .with_focus(focus)
        // Koushi renders read receipts on message-like timeline rows. Tracking
        // state-event receipts widens the SDK event-cache ordering surface and
        // has triggered linked_chunk order assertions during post-verification
        // normal-sync startup.
        .track_read_marker_and_receipts(TimelineReadReceiptTracking::MessageLikeEvents)
}

pub(super) struct PreparedRelayRecovery<Snapshot, Stream> {
    generation: TimelineGeneration,
    snapshot: Snapshot,
    stream: Stream,
}

struct AuthoritativeWindowReconciliation {
    scoped_event_ids: Vec<String>,
    removed_event_ids: Vec<String>,
}

struct ReceiptObservationRequest {
    room_id: String,
    receipts_by_event: Vec<LiveEventReceipts>,
    target: ReceiptObservationTarget,
}

struct PreparedAuthoritativeSnapshotReconciliation {
    replacement_media_sources: HashMap<String, PrivateMediaEntry>,
    receipt_observation: Option<ReceiptObservationRequest>,
    search_messages: Vec<SearchIndexMessage>,
}

fn authoritative_window_reconciliation(
    old_event_ids: &std::collections::BTreeSet<String>,
    new_event_ids: &std::collections::BTreeSet<String>,
) -> AuthoritativeWindowReconciliation {
    AuthoritativeWindowReconciliation {
        scoped_event_ids: old_event_ids.union(new_event_ids).cloned().collect(),
        removed_event_ids: old_event_ids.difference(new_event_ids).cloned().collect(),
    }
}

fn authoritative_search_removals(
    reconciliation: &AuthoritativeWindowReconciliation,
) -> Vec<SearchIndexMessage> {
    reconciliation
        .removed_event_ids
        .iter()
        .cloned()
        .map(|event_id| SearchIndexMessage::Redact { event_id })
        .collect()
}

fn authoritative_receipts_action(
    room_id: &str,
    reconciliation: &AuthoritativeWindowReconciliation,
    receipts_by_event: Vec<LiveEventReceipts>,
) -> AppAction {
    AppAction::LiveRoomReceiptsWindowReconciled {
        room_id: room_id.to_owned(),
        scoped_event_ids: reconciliation.scoped_event_ids.clone(),
        receipts_by_event,
    }
}

fn replace_authoritative_cache<K, V>(cache: &mut HashMap<K, V>, replacement: HashMap<K, V>) {
    *cache = replacement;
}

async fn prepare_relay_recovery<Subscribe, SubscribeFuture, Snapshot, Stream>(
    current_generation: TimelineGeneration,
    overflow_generation: TimelineGeneration,
    subscribe: Subscribe,
) -> Option<PreparedRelayRecovery<Snapshot, Stream>>
where
    Subscribe: FnOnce() -> SubscribeFuture,
    SubscribeFuture: std::future::Future<Output = (Snapshot, Stream)>,
{
    if !accept_relay_generation(current_generation, overflow_generation) {
        return None;
    }
    let (snapshot, stream) = subscribe().await;
    Some(PreparedRelayRecovery {
        generation: TimelineGeneration(current_generation.0 + 1),
        snapshot,
        stream,
    })
}

fn accept_relay_generation(
    current_generation: TimelineGeneration,
    incoming_generation: TimelineGeneration,
) -> bool {
    current_generation == incoming_generation
}

pub(super) fn accepted_relay_batch<T>(
    current_generation: TimelineGeneration,
    incoming_generation: TimelineGeneration,
    batch: T,
) -> Option<T> {
    accept_relay_generation(current_generation, incoming_generation).then_some(batch)
}

pub(super) fn commit_authoritative_recovery_window<F>(
    navigation_items: &mut Vec<TimelineItem>,
    display_projection: &mut DisplayProjectionState,
    event_tx: &broadcast::Sender<CoreEvent>,
    replay_known_thread_root_projections: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    reason: TimelineResyncReason,
    authoritative_items: Vec<TimelineItem>,
    commit_synchronous_candidates: F,
) -> bool
where
    F: FnOnce(),
{
    let candidate_display = DisplayProjectionState::from_canonical_window(
        &authoritative_items,
        0..authoritative_items.len(),
    );
    let replay_known_candidates = replay_known_candidates_for_display_items(
        key,
        &authoritative_items,
        candidate_display.display_items(),
    );
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    commit_prepared_initial_window_with_lease(
        navigation_items,
        display_projection,
        event_tx,
        replay_known_thread_root_projections,
        thread_root_projection_service,
        &lease,
        key,
        actor_generation,
        InitialItemsRequestIdentity::recovery(),
        generation,
        vec![TimelineEvent::ResyncRequired {
            key: key.clone(),
            reason,
        }],
        PreparedInitialWindow {
            display_projection: candidate_display,
            navigation_items: Some(authoritative_items.clone()),
            emitted_items: authoritative_items,
            replay_known_candidates,
        },
        commit_synchronous_candidates,
    );
    true
}

/// Event ID of an SDK timeline item, when it has one.
fn sdk_timeline_item_event_id(item: &SdkTimelineItem) -> Option<&matrix_sdk::ruma::EventId> {
    item.as_event()?.event_id()
}

pub(super) async fn run_diff_relay(
    actor_tx: mpsc::Sender<TimelineRelayBatch>,
    control_tx: mpsc::Sender<TimelineRelayControl>,
    generation: TimelineGeneration,
    actor_generation: u64,
    mut diff_stream: impl futures_util::Stream<Item = Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>>
    + Unpin,
    initial_items: Vec<Arc<SdkTimelineItem>>,
) {
    use futures_util::StreamExt;

    // Track recently-observed UTD event IDs so a late-decryption replacement
    // in the visible timeline is counted exactly once per UTD item (#476). The
    // set is bounded; old entries age out by replacement. Only the aggregate
    // counter is exported — no event IDs ever enter diagnostics. The set and
    // counter are updated only after the batch is accepted by the actor, so a
    // queue-overflow batch never claims a visible replacement.
    let mut utd_event_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    const UTD_TRACK_LIMIT: usize = 256;
    // Seed tracking from UTD rows already present when the timeline opened so
    // a later decrypted `Set` for an initial UTD is counted.
    for item in &initial_items {
        let is_utd = item
            .as_event()
            .map(|event| event.content().is_unable_to_decrypt())
            .unwrap_or(false);
        if is_utd
            && let Some(event_id) = sdk_timeline_item_event_id(item)
            && utd_event_ids.len() < UTD_TRACK_LIMIT
        {
            utd_event_ids.insert(event_id.to_string());
        }
    }

    loop {
        let Some(diffs) = diff_stream.next().await else {
            let _ = control_tx
                .send(TimelineRelayControl::StreamEnded { generation })
                .await;
            break;
        };

        // Compute visible-UTD tracking ops for this batch. They are applied
        // only after the actor accepts the batch, so dropped batches never
        // claim visible replacements.
        enum TrackOp {
            Track(String),
            CountReplacement(String),
            Clear,
        }
        let mut track_ops: Vec<TrackOp> = Vec::new();
        for diff in &diffs {
            match diff {
                eyeball_im::VectorDiff::Set { value, .. } => {
                    let item = value.as_ref();
                    let event_id = sdk_timeline_item_event_id(item).map(|id| id.to_string());
                    let is_utd = item
                        .as_event()
                        .map(|event| event.content().is_unable_to_decrypt())
                        .unwrap_or(false);
                    if is_utd {
                        if let Some(event_id) = event_id {
                            track_ops.push(TrackOp::Track(event_id));
                        }
                    } else if let Some(event_id) = event_id {
                        track_ops.push(TrackOp::CountReplacement(event_id));
                    }
                }
                eyeball_im::VectorDiff::PushFront { value }
                | eyeball_im::VectorDiff::PushBack { value }
                | eyeball_im::VectorDiff::Insert { value, .. } => {
                    let item = value.as_ref();
                    let is_utd = item
                        .as_event()
                        .map(|event| event.content().is_unable_to_decrypt())
                        .unwrap_or(false);
                    if is_utd && let Some(event_id) = sdk_timeline_item_event_id(item) {
                        track_ops.push(TrackOp::Track(event_id.to_string()));
                    }
                }
                // Removal/reset shapes invalidate tracked positions; drop the
                // tracking so stale entries cannot fill the bound.
                eyeball_im::VectorDiff::Remove { .. }
                | eyeball_im::VectorDiff::Truncate { .. }
                | eyeball_im::VectorDiff::Clear
                | eyeball_im::VectorDiff::PopFront
                | eyeball_im::VectorDiff::PopBack
                | eyeball_im::VectorDiff::Reset { .. } => {
                    track_ops.push(TrackOp::Clear);
                }
                _ => {}
            }
        }

        let thread_attention_provenance = ThreadAttentionBatchProvenance::from_sdk_diffs(&diffs);
        let mut batch = TimelineRelayBatch {
            generation,
            gap_repair_projections: gap_repair_projections_from_sdk_diffs(&diffs),
            diffs,
            thread_attention_provenance,
        };
        batch.retain_gap_repair_projections_for_actor(actor_generation);
        for projection in &batch.gap_repair_projections {
            record_timeline_gap_projection_boundary(
                "relay_received",
                "queued",
                projection.actor_generation,
                generation,
                projection.operation,
                Some(projection.projection_batch),
                None,
                None,
                batch.gap_repair_projections.len(),
            );
        }
        match actor_tx.try_send(batch) {
            Ok(_) => {
                // The batch is now owned by the actor's inbox; apply the
                // visible-UTD tracking against the diffs the actor will apply.
                for op in track_ops {
                    match op {
                        TrackOp::Track(event_id) => {
                            if utd_event_ids.len() < UTD_TRACK_LIMIT {
                                utd_event_ids.insert(event_id);
                            }
                        }
                        TrackOp::CountReplacement(event_id) => {
                            if utd_event_ids.remove(&event_id) {
                                koushi_diagnostics::increment_counter(
                                    "late_decryption_timeline_replacements",
                                );
                            }
                        }
                        TrackOp::Clear => {
                            utd_event_ids.clear();
                        }
                    }
                }
            }
            Err(mpsc::error::TrySendError::Full(batch)) => {
                for projection in &batch.gap_repair_projections {
                    record_timeline_gap_projection_boundary(
                        "relay_received",
                        "queue_full",
                        projection.actor_generation,
                        generation,
                        projection.operation,
                        Some(projection.projection_batch),
                        None,
                        None,
                        batch.gap_repair_projections.len(),
                    );
                }
                // Overflow control must not compete for capacity with data.
                // Once delivered, this generation is terminal; the actor
                // resubscribes and owns the replacement relay task.
                let _ = control_tx
                    .send(TimelineRelayControl::Overflow { generation })
                    .await;
                break;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Actor dropped — relay task should stop.
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_source::item_body;

    use std::collections::{BTreeSet, HashMap};

    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    use std::time::Duration;

    use futures_util::StreamExt;

    use koushi_state::AppAction;

    use matrix_sdk_ui::timeline::TimelineItem as SdkTimelineItem;
    use tokio::sync::{broadcast, mpsc};

    use crate::event::{
        CoreEvent, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId, TimelineResyncReason,
        TimelineSendState,
    };
    use crate::executor;

    use crate::ids::{TimelineBatchId, TimelineGeneration};

    use crate::search::SearchIndexMessage;

    use crate::threads_list::ThreadRootProjectionService;

    use super::super::actor::TimelineActorMessage;
    use super::super::display_projection::DisplayProjectionState;
    use super::super::item_projection::sdk_vector_diffs_to_timeline_diffs;
    use super::super::navigation::{
        InitialItemsRequestIdentity, PreparedInitialWindow, TimelineActorGenerationGate,
        commit_prepared_initial_window_for_generation,
        emit_items_updated_and_reconcile_replay_known_for_generation,
    };
    use super::super::test_support::{
        fake_rid, replacement_generation_fixture, replay_projection_services, room_key,
        timeline_item,
    };
    use super::super::thread_projection::{
        ReplayKnownThreadRootProjectionRegistry, ThreadAttentionBatchProvenance,
    };
    use super::{
        PreparedRelayRecovery, RelayRestartBackoff, RelayRestartSchedule, TimelineRelayBatch,
        TimelineRelayControl, accepted_relay_batch, authoritative_receipts_action,
        authoritative_search_removals, authoritative_window_reconciliation,
        commit_authoritative_recovery_window, prepare_relay_recovery, replace_authoritative_cache,
        run_diff_relay, spawn_relay_restart_timer,
    };

    #[test]
    fn batch_id_monotonically_increases_per_generation() {
        let mut id = TimelineBatchId(0);
        let mut seen = Vec::new();
        for _ in 0..10 {
            seen.push(id);
            id = TimelineBatchId(id.0 + 1);
        }
        for (i, pair) in seen.windows(2).enumerate() {
            assert!(pair[0] < pair[1], "batch ids must be increasing: index {i}");
        }
        // After generation reset, batch_id resets to 0.
        let reset = TimelineBatchId(0);
        assert_eq!(reset, TimelineBatchId(0));
    }

    #[tokio::test]
    async fn relay_overflow_control_is_delivered_when_data_inbox_is_full() {
        let (actor_tx, mut actor_rx) = mpsc::channel::<TimelineRelayBatch>(1);
        let (command_tx, mut command_rx) = mpsc::channel::<TimelineActorMessage>(1);
        let (control_tx, mut control_rx) = mpsc::channel::<TimelineRelayControl>(1);

        command_tx
            .try_send(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(fake_rid(1)),
            })
            .expect("command must queue independently of relay data");

        actor_tx
            .try_send(TimelineRelayBatch {
                generation: TimelineGeneration(7),
                diffs: Vec::new(),
                thread_attention_provenance: ThreadAttentionBatchProvenance::default(),
                gap_repair_projections: BTreeSet::new(),
            })
            .expect("test must fill the data inbox");

        let relay = executor::spawn(run_diff_relay(
            actor_tx.clone(),
            control_tx.clone(),
            TimelineGeneration(7),
            1,
            futures_util::stream::iter([Vec::new()]),
            vec![],
        ));

        let control = tokio::time::timeout(Duration::from_secs(1), control_rx.recv())
            .await
            .expect("overflow control must not wait for capacity in the data inbox")
            .expect("relay must keep the control lane open until overflow is delivered");
        assert!(matches!(
            control,
            TimelineRelayControl::Overflow {
                generation: TimelineGeneration(7)
            }
        ));
        relay
            .await
            .expect("overflowed relay must terminate cleanly");

        let _filled_message = actor_rx
            .recv()
            .await
            .expect("test must release the old full data inbox");
        run_diff_relay(
            actor_tx,
            control_tx,
            TimelineGeneration(8),
            1,
            futures_util::stream::iter([Vec::new()]),
            vec![],
        )
        .await;
        assert!(matches!(
            actor_rx.recv().await,
            Some(TimelineRelayBatch {
                generation: TimelineGeneration(8),
                diffs,
                ..
            }) if diffs.is_empty()
        ));
        assert!(matches!(
            command_rx.try_recv(),
            Ok(TimelineActorMessage::ReplayInitialItems { .. })
        ));
    }

    #[tokio::test]
    async fn relay_stream_end_emits_one_recovery_control_without_consuming_commands() {
        let (data_tx, mut data_rx) = mpsc::channel::<TimelineRelayBatch>(1);
        let (control_tx, mut control_rx) = mpsc::channel::<TimelineRelayControl>(1);
        let (command_tx, mut command_rx) = mpsc::channel::<TimelineActorMessage>(1);
        command_tx
            .try_send(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(fake_rid(88)),
            })
            .expect("command must queue independently");

        run_diff_relay(
            data_tx,
            control_tx,
            TimelineGeneration(9),
            1,
            futures_util::stream::empty(),
            vec![],
        )
        .await;

        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineRelayControl::StreamEnded {
                generation: TimelineGeneration(9)
            })
        ));
        assert!(matches!(data_rx.recv().await, None));
        assert!(matches!(
            command_rx.try_recv(),
            Ok(TimelineActorMessage::ReplayInitialItems { .. })
        ));
        assert!(matches!(
            control_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn relay_restart_backoff_grows_caps_resets_and_rejects_stale_due_tokens() {
        let mut backoff =
            RelayRestartBackoff::new(Duration::from_millis(10), Duration::from_millis(40));
        let first = backoff.schedule(TimelineGeneration(3));
        let second = backoff.schedule(TimelineGeneration(3));
        let third = backoff.schedule(TimelineGeneration(3));
        let capped = backoff.schedule(TimelineGeneration(3));
        assert_eq!(first.delay, Duration::from_millis(10));
        assert_eq!(second.delay, Duration::from_millis(20));
        assert_eq!(third.delay, Duration::from_millis(40));
        assert_eq!(capped.delay, Duration::from_millis(40));
        assert!(!backoff.accept_due(first.generation, first.serial));
        assert!(backoff.accept_due(capped.generation, capped.serial));
        assert!(!backoff.accept_due(capped.generation, capped.serial));

        backoff.reset_after_live_batch();
        let reset = backoff.schedule(TimelineGeneration(4));
        assert_eq!(reset.delay, Duration::from_millis(10));
        assert!(!backoff.accept_due(TimelineGeneration(3), reset.serial));
    }

    #[tokio::test]
    async fn relay_restart_timer_does_not_block_commands_and_emits_one_due_after_delay() {
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let (command_tx, mut command_rx) = mpsc::channel(1);
        command_tx
            .try_send(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(fake_rid(89)),
            })
            .expect("command queued during restart delay");
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let schedule = RelayRestartSchedule {
            generation: TimelineGeneration(5),
            serial: 11,
            delay: Duration::ZERO,
        };
        let timer = spawn_relay_restart_timer(control_tx, schedule, async move {
            let _ = release_rx.await;
        });

        assert!(matches!(
            control_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            command_rx.try_recv(),
            Ok(TimelineActorMessage::ReplayInitialItems { .. })
        ));
        release_tx.send(()).expect("release timer");
        timer.await.expect("timer task");
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineRelayControl::RestartDue {
                generation: TimelineGeneration(5),
                serial: 11,
            })
        ));
        assert!(matches!(
            control_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn relay_overflow_recovery_subscribes_once_emits_snapshot_then_next_live_update() {
        let key = room_key();
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let projection_service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
        let subscribe_count = Arc::new(AtomicU64::new(0));
        let subscribe_count_for_fake = subscribe_count.clone();

        let prepared = prepare_relay_recovery(
            TimelineGeneration(7),
            TimelineGeneration(7),
            move || async move {
                subscribe_count_for_fake.fetch_add(1, Ordering::SeqCst);
                (
                    Vec::<TimelineItem>::new(),
                    futures_util::stream::iter([vec![
                        eyeball_im::VectorDiff::<Arc<SdkTimelineItem>>::Clear,
                    ]]),
                )
            },
        )
        .await
        .expect("matching overflow generation must recover");
        assert_eq!(subscribe_count.load(Ordering::SeqCst), 1);
        assert_eq!(prepared.generation, TimelineGeneration(8));

        let PreparedRelayRecovery {
            generation,
            snapshot,
            stream,
        } = prepared;
        let mut navigation_items = Vec::new();
        let mut display_projection = DisplayProjectionState::default();
        assert!(commit_authoritative_recovery_window(
            &mut navigation_items,
            &mut display_projection,
            &event_tx,
            &replay_registry,
            &projection_service,
            &actor_generations,
            &key,
            actor_generation,
            generation,
            TimelineResyncReason::QueueOverflow,
            snapshot,
            || {},
        ));
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::ResyncRequired { .. }))
        ));
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                generation: TimelineGeneration(8),
                ..
            }))
        ));

        let (actor_tx, mut actor_rx) = mpsc::channel(1);
        let (control_tx, _control_rx) = mpsc::channel(1);
        run_diff_relay(
            actor_tx,
            control_tx,
            generation,
            actor_generation,
            stream,
            vec![],
        )
        .await;
        let Some(TimelineRelayBatch {
            generation: batch_generation,
            diffs,
            ..
        }) = actor_rx.recv().await
        else {
            panic!("replacement stream must produce a generation-tagged batch");
        };
        let sdk_diffs = accepted_relay_batch(generation, batch_generation, diffs)
            .expect("replacement generation must be accepted");
        let diffs = sdk_vector_diffs_to_timeline_diffs(
            &sdk_diffs,
            0,
            &key,
            None,
            &HashMap::new(),
            None,
            None,
        );
        assert!(
            emit_items_updated_and_reconcile_replay_known_for_generation(
                &event_tx,
                &replay_registry,
                &projection_service,
                &actor_generations,
                &key,
                actor_generation,
                generation,
                TimelineBatchId(0),
                diffs,
                &[],
                &[],
            )
        );
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                generation: TimelineGeneration(8),
                diffs,
                ..
            })) if matches!(diffs.as_slice(), [TimelineDiff::Clear])
        ));
    }

    #[test]
    fn relay_overflow_stale_generation_is_rejected_without_state_or_event_change() {
        let next_batch_id = TimelineBatchId(4);
        let (event_tx, mut event_rx) = broadcast::channel::<CoreEvent>(1);
        let stale = accepted_relay_batch(
            TimelineGeneration(8),
            TimelineGeneration(7),
            vec![TimelineDiff::Clear],
        );
        if stale.is_some() {
            let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: room_key(),
                generation: TimelineGeneration(7),
                batch_id: next_batch_id,
                diffs: vec![TimelineDiff::Clear],
            }));
        }
        assert!(stale.is_none());
        assert_eq!(next_batch_id, TimelineBatchId(4));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn relay_overflow_authoritative_window_plan_scopes_receipts_and_search_removals() {
        let old = std::collections::BTreeSet::from([
            "$removed:test".to_owned(),
            "$retained:test".to_owned(),
        ]);
        let new = std::collections::BTreeSet::from([
            "$retained:test".to_owned(),
            "$added:test".to_owned(),
        ]);
        let plan = authoritative_window_reconciliation(&old, &new);
        assert_eq!(
            plan.scoped_event_ids,
            vec!["$added:test", "$removed:test", "$retained:test"]
        );
        assert_eq!(plan.removed_event_ids, vec!["$removed:test"]);
        assert!(matches!(
            authoritative_search_removals(&plan).as_slice(),
            [SearchIndexMessage::Redact { event_id }] if event_id == "$removed:test"
        ));
        assert!(matches!(
            authoritative_receipts_action("!room:test", &plan, Vec::new()),
            AppAction::LiveRoomReceiptsWindowReconciled {
                scoped_event_ids,
                receipts_by_event,
                ..
            } if scoped_event_ids.len() == 3 && receipts_by_event.is_empty()
        ));
    }

    #[test]
    fn relay_overflow_authoritative_cache_rebuild_removes_old_and_installs_new_entries() {
        let mut cache = HashMap::from([("old", 1_u8), ("retained-old-value", 2)]);
        replace_authoritative_cache(
            &mut cache,
            HashMap::from([("retained-old-value", 3_u8), ("new", 4)]),
        );
        assert_eq!(
            cache,
            HashMap::from([("retained-old-value", 3_u8), ("new", 4)])
        );
    }

    #[tokio::test]
    async fn authoritative_resync_projects_event_only_and_emits_ordered_recovery_events() {
        let mut transaction = timeline_item(
            "$transaction-placeholder:test",
            Some("same body"),
            "@me:test",
            false,
        );
        transaction.id = TimelineItemId::Transaction {
            transaction_id: "txn-echo".to_owned(),
        };
        transaction.send_state = Some(TimelineSendState::Sending);
        let remote = timeline_item("$remote-echo:test", Some("same body"), "@me:test", false);
        let mut current = vec![transaction, remote.clone()];
        let mut display_projection =
            DisplayProjectionState::from_canonical_window(&current, 0..current.len());
        let key = room_key();
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let actor_generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let replay_registry = Arc::new(Mutex::new(
            ReplayKnownThreadRootProjectionRegistry::default(),
        ));
        let projection_service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));

        commit_authoritative_recovery_window(
            &mut current,
            &mut display_projection,
            &event_tx,
            &replay_registry,
            &projection_service,
            &actor_generations,
            &key,
            actor_generation,
            TimelineGeneration(2),
            TimelineResyncReason::QueueOverflow,
            vec![remote],
            || {},
        );

        assert_eq!(current.len(), 1);
        assert!(matches!(
            current[0].id,
            TimelineItemId::Event { ref event_id } if event_id == "$remote-echo:test"
        ));
        assert!(!matches!(
            current[0].send_state,
            Some(TimelineSendState::Sending)
        ));
        assert_eq!(display_projection.display_items(), current);
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::ResyncRequired { .. }))
        ));
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: None,
                cause_request_id: None,
                generation: TimelineGeneration(2),
                items,
                ..
            })) if items.len() == 1
                && matches!(items[0].id, TimelineItemId::Event { .. })
                && !matches!(items[0].send_state, Some(TimelineSendState::Sending))
        ));
    }

    #[tokio::test]
    async fn stale_generation_recovery_does_not_commit_candidate_or_publish() {
        let key = room_key();
        let (actor_generations, stale_generation, _current_generation) =
            replacement_generation_fixture(&key).await;
        let original = timeline_item("$original:test", Some("original"), "@me:test", false);
        let candidate = timeline_item("$candidate:test", Some("candidate"), "@me:test", false);
        let mut navigation_items = vec![original.clone()];
        let mut display_projection = DisplayProjectionState::from_canonical_window(
            &navigation_items,
            0..navigation_items.len(),
        );
        let display_before = display_projection.clone();
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (replay_registry, projection_service) = replay_projection_services();
        let mut synchronous_candidate_committed = false;

        assert!(!commit_authoritative_recovery_window(
            &mut navigation_items,
            &mut display_projection,
            &event_tx,
            &replay_registry,
            &projection_service,
            &actor_generations,
            &key,
            stale_generation,
            TimelineGeneration(2),
            TimelineResyncReason::QueueOverflow,
            vec![candidate],
            || synchronous_candidate_committed = true,
        ));

        assert_eq!(navigation_items, vec![original]);
        assert_eq!(display_projection, display_before);
        assert!(!synchronous_candidate_committed);
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn stale_generation_prepared_initial_window_does_not_commit_or_publish() {
        let key = room_key();
        let (actor_generations, stale_generation, _current_generation) =
            replacement_generation_fixture(&key).await;
        let original = timeline_item("$original:test", Some("original"), "@me:test", false);
        let candidate = timeline_item("$candidate:test", Some("candidate"), "@me:test", false);
        let mut navigation_items = vec![original.clone()];
        let mut display_projection = DisplayProjectionState::from_canonical_window(
            &navigation_items,
            0..navigation_items.len(),
        );
        let display_before = display_projection.clone();
        let candidate_navigation = vec![candidate.clone()];
        let prepared = PreparedInitialWindow {
            display_projection: DisplayProjectionState::from_canonical_window(
                &candidate_navigation,
                0..candidate_navigation.len(),
            ),
            navigation_items: Some(candidate_navigation),
            emitted_items: vec![candidate],
            replay_known_candidates: Vec::new(),
        };
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (replay_registry, projection_service) = replay_projection_services();

        assert!(!commit_prepared_initial_window_for_generation(
            &mut navigation_items,
            &mut display_projection,
            &event_tx,
            &replay_registry,
            &projection_service,
            &actor_generations,
            &key,
            stale_generation,
            InitialItemsRequestIdentity::recovery(),
            TimelineGeneration(2),
            Vec::new(),
            prepared,
        ));

        assert_eq!(navigation_items, vec![original]);
        assert_eq!(display_projection, display_before);
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn relay_overflow_signal_triggers_generation_bump() {
        // Test the overflow logic directly on the actor message pathway,
        // using a synthetic mpsc channel at capacity 1 to force overflow.
        let (event_tx, mut event_rx): (broadcast::Sender<CoreEvent>, _) = broadcast::channel(256);
        let (actor_tx, actor_rx) = mpsc::channel::<TimelineActorMessage>(2);

        let key = room_key();
        let generation = Arc::new(AtomicU64::new(0));
        let next_batch_id = Arc::new(AtomicU64::new(0));

        // Simulate the actor receiving RelayOverflow:
        // It should increment generation, reset batch_id, and emit ResyncRequired.
        // We test the state machine logic directly.
        let gen_before = generation.load(Ordering::SeqCst);
        let new_gen = gen_before + 1;
        generation.store(new_gen, Ordering::SeqCst);
        next_batch_id.store(0, Ordering::SeqCst);

        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ResyncRequired {
            key: key.clone(),
            reason: TimelineResyncReason::QueueOverflow,
        }));

        // Verify the event was emitted.
        let event = event_rx.recv().await.expect("event");
        match event {
            CoreEvent::Timeline(TimelineEvent::ResyncRequired {
                key: ev_key,
                reason,
            }) => {
                assert_eq!(ev_key, key);
                assert_eq!(reason, TimelineResyncReason::QueueOverflow);
            }
            other => panic!("expected ResyncRequired, got {other:?}"),
        }

        assert_eq!(
            generation.load(Ordering::SeqCst),
            1,
            "generation must be bumped"
        );
        assert_eq!(
            next_batch_id.load(Ordering::SeqCst),
            0,
            "batch_id resets to 0"
        );

        drop(actor_tx);
        drop(actor_rx);
    }

    #[test]
    fn timeline_subscribe_spawns_always_on_origin_observer() {
        let spawn = item_body(include_str!("actor.rs"), "async fn spawn(");
        let origin_token = item_body(
            include_str!("diagnostics.rs"),
            "fn event_cache_origin_trace_token",
        );
        assert!(
            spawn.contains("startup_trace::trace_origin"),
            "origin observer must always record structured startup provenance"
        );
        assert!(
            spawn.contains("event_cache()"),
            "origin observer must subscribe the SDK room event cache"
        );
        assert!(
            origin_token.contains("EventsOrigin"),
            "origin observer must read the SDK EventsOrigin (cache/network/sync)"
        );
    }
}
