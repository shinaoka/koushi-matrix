use super::actor::{
    MissingSpaceChildLink, ROOM_OBSERVATION_SHUTDOWN_JOIN_TIMEOUT, RoomActor, RoomListReconcileAck,
    RoomMessage,
};
use super::normalization::{
    normalize_invites, normalize_rooms_with_previous, normalize_spaces, normalize_user_profiles,
    replace_known_room_ids,
};
use crate::direct_message_classification::{DirectAccountDataSource, DirectClassificationState};
use crate::executor;
use crate::timeline::{
    RoomMembershipTransition, RoomMembershipTransitionKind, TimelineSubscriptionResidencyHandle,
    VisibleRoomObservation,
};
use crate::unread_trace;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::event::{CoreEvent, RoomEvent};
use koushi_sdk::{
    MatrixClientSession, MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace,
};
use koushi_state::{AppAction, RoomListSource, RoomSummary};
use matrix_sdk::ruma::events::direct::DirectEvent;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Handle on the spawned room-list observation loop: oneshot stop signal plus
/// the task handle so teardown can await completion. Operation-triggered
/// refreshes are always sent to the observation loop so command handling never
/// blocks on room-list normalization.
pub(super) struct RoomListObservation {
    pub(super) stop_tx: oneshot::Sender<()>,
    pub(super) task: executor::JoinHandle<()>,
    pub(super) command_tx: mpsc::Sender<RoomListObservationCommand>,
    pub(super) generation: u64,
    pub(super) source: RoomListSource,
}

pub(super) enum RoomListObservationCommand {
    Refresh,
    RefreshRoom {
        room_id: String,
    },
    HydrateSpaceMembers {
        space_id: String,
    },
    Reconcile {
        backend_generation: u64,
        response_sequence: u64,
        ack: oneshot::Sender<RoomListReconcileAck>,
    },
}

/// Page size for the local dynamic entries adapter. The observer expands this
/// adapter to the SDK-reported all-rooms count; this is not a product cap.
const ROOM_LIST_ENTRIES_PAGE_SIZE: usize = 100;

fn additional_room_list_pages(page_size: usize, maximum_number_of_rooms: Option<u32>) -> usize {
    debug_assert!(page_size > 0);
    let Some(maximum) = maximum_number_of_rooms.and_then(|count| usize::try_from(count).ok())
    else {
        return 0;
    };
    maximum.saturating_sub(1) / page_size
}

fn room_list_range_is_complete(
    maximum_number_of_rooms: Option<u32>,
    current_entries: usize,
) -> bool {
    match maximum_number_of_rooms.and_then(|count| usize::try_from(count).ok()) {
        Some(maximum_number_of_rooms) => maximum_number_of_rooms == current_entries,
        // Some Simplified Sliding Sync responses omit `count`. The committed
        // response sequence is still the SDK's authority for the observed
        // projection; later responses replace it as the server reveals more
        // rooms.
        None => true,
    }
}

/// Number of distinct room ids in the ordered accumulator.
fn unique_room_id_count(
    current: &eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
) -> usize {
    current
        .iter()
        .map(|item| item.room_id().to_owned())
        .collect::<BTreeSet<_>>()
        .len()
}

/// Authoritative admission fence for a live room-list projection.
///
/// The accumulator is maintained by applying index-based `VectorDiff`s, so a
/// diff applied against a differently ordered vector overwrites the wrong entry
/// and produces one duplicate room id plus one silently missing room while the
/// length stays put (#446). Length equality alone therefore must never
/// establish authority: every entry must also carry a distinct identity.
fn room_list_projection_admits_authority(
    reconciliation_is_complete: bool,
    entries_count: usize,
    distinct_identity_count: usize,
) -> bool {
    reconciliation_is_complete && entries_count == distinct_identity_count
}

async fn forward_visible_rooms_if_authoritative(
    timeline_residency: Option<&TimelineSubscriptionResidencyHandle>,
    core_generation: u64,
    reconciliation_is_complete: bool,
    entries_count: usize,
    distinct_identity_count: usize,
    room_ids: Vec<VisibleRoomObservation>,
) -> bool {
    if !room_list_projection_admits_authority(
        reconciliation_is_complete,
        entries_count,
        distinct_identity_count,
    ) {
        return false;
    }
    let Some(timeline_residency) = timeline_residency else {
        return false;
    };
    timeline_residency
        .visible_rooms_observed(core_generation, room_ids)
        .await
}

async fn forward_membership_batches(
    timeline_residency: Option<&TimelineSubscriptionResidencyHandle>,
    core_generation: u64,
    membership_batches: impl IntoIterator<Item = Vec<RoomMembershipTransition>>,
) -> bool {
    let Some(timeline_residency) = timeline_residency else {
        return false;
    };
    let mut forwarded = false;
    for transitions in membership_batches {
        if !transitions.is_empty() {
            forwarded |= timeline_residency
                .membership_observed(core_generation, transitions)
                .await;
        }
    }
    forwarded
}

pub(super) fn room_stop_matches_generation(
    active_generation: Option<u64>,
    stopped_generation: u64,
) -> bool {
    active_generation == Some(stopped_generation)
}

#[derive(Default)]
struct LiveRoomListReconciliation {
    maximum_number_of_rooms: Option<u32>,
    range_fully_loaded: bool,
    authoritative: bool,
    pending: Option<(u64, u64, Option<oneshot::Sender<RoomListReconcileAck>>)>,
}

impl LiveRoomListReconciliation {
    fn report_maximum(&mut self, maximum_number_of_rooms: Option<u32>) {
        if self.maximum_number_of_rooms != maximum_number_of_rooms {
            self.authoritative = false;
        }
        self.maximum_number_of_rooms = maximum_number_of_rooms;
    }

    fn report_range_fully_loaded(&mut self, fully_loaded: bool) {
        if !fully_loaded {
            self.authoritative = false;
        }
        self.range_fully_loaded = fully_loaded;
    }

    fn begin(
        &mut self,
        backend_generation: u64,
        response_sequence: u64,
        ready_tx: oneshot::Sender<RoomListReconcileAck>,
    ) {
        self.authoritative = false;
        self.pending = Some((backend_generation, response_sequence, Some(ready_tx)));
    }

    fn is_complete(&self, current_entries: usize) -> bool {
        (self.range_fully_loaded || self.maximum_number_of_rooms.is_none())
            && room_list_range_is_complete(self.maximum_number_of_rooms, current_entries)
    }

    fn has_pending_reconciliation(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn is_authoritative(&self, current_entries: usize) -> bool {
        self.authoritative && self.is_complete(current_entries)
    }

    fn take_projection_ack(&mut self) -> Option<(u64, u64, oneshot::Sender<RoomListReconcileAck>)> {
        let (backend_generation, response_sequence, ready_tx) = self.pending.as_mut()?;
        Some((*backend_generation, *response_sequence, ready_tx.take()?))
    }

    fn finish_if_complete(
        &mut self,
        current_entries: usize,
    ) -> Option<(u64, u64, Option<oneshot::Sender<RoomListReconcileAck>>)> {
        if !self.is_complete(current_entries) {
            return None;
        }
        let pending = self.pending.take()?;
        self.authoritative = true;
        Some(pending)
    }
}

#[allow(clippy::too_many_arguments)]
async fn project_live_entries_and_ack_if_reconciled(
    reconciliation: &mut LiveRoomListReconciliation,
    session: &MatrixClientSession,
    current: &eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
    direct_state: &DirectClassificationState,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    known_dm_rooms: &Arc<RwLock<Vec<RoomSummary>>>,
    room_tx: &mpsc::Sender<RoomMessage>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
    generation: u64,
    source: RoomListSource,
    authoritative: &Arc<AtomicBool>,
    sliding_sync_diagnostics: &crate::SlidingSyncDiagnostics,
    timeline_residency: Option<&TimelineSubscriptionResidencyHandle>,
) {
    let entries_count = current.len();
    let unique_ids = unique_room_id_count(current);
    let reconciliation_is_complete = reconciliation.is_complete(entries_count);
    let projection_is_authoritative = room_list_projection_admits_authority(
        reconciliation_is_complete,
        entries_count,
        unique_ids,
    );
    if reconciliation_is_complete && !projection_is_authoritative {
        record(
            DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.room",
                "room_list_integrity_rejected",
            )
            .field(DiagnosticField::token("reason", "duplicate_identity"))
            .field(DiagnosticField::count(
                "entries_count",
                entries_count as u64,
            ))
            .field(DiagnosticField::count(
                "unique_room_id_count",
                unique_ids as u64,
            )),
        );
    }
    authoritative.store(projection_is_authoritative, Ordering::Release);
    let delivered = normalize_and_project_entries(
        session,
        current,
        direct_state.authoritative_targets(),
        known_room_ids,
        known_dm_rooms,
        room_tx,
        action_tx,
        event_tx,
        generation,
        source,
        authoritative,
        Some(direct_state),
        Some(sliding_sync_diagnostics),
    )
    .await;
    if !delivered {
        authoritative.store(false, Ordering::Release);
        return;
    }
    let visible_rooms = current
        .iter()
        .map(|item| item.clone().into_inner())
        .map(|room| VisibleRoomObservation {
            room_id: room.room_id().to_string(),
            non_left: room.state() != matrix_sdk::RoomState::Left,
        })
        .collect();
    let _ = forward_visible_rooms_if_authoritative(
        timeline_residency,
        generation,
        reconciliation_is_complete,
        entries_count,
        unique_ids,
        visible_rooms,
    )
    .await;
    if projection_is_authoritative {
        if let Some((backend_generation, response_sequence, ack)) =
            reconciliation.finish_if_complete(current.len())
            && let Some(ack) = ack
        {
            let _ = ack.send(RoomListReconcileAck::Reconciled {
                backend_generation,
                room_generation: generation,
                response_sequence,
            });
        }
    } else if let Some((backend_generation, response_sequence, ack)) =
        reconciliation.take_projection_ack()
    {
        let _ = ack.send(RoomListReconcileAck::Projected {
            backend_generation,
            room_generation: generation,
            response_sequence,
        });
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum LiveObserverTestEvent {
    RlsProjected {
        wake_count: u64,
        entries_len: usize,
    },
    DirectEventStreamClosed,
    DirectClassificationUpdated {
        event_wake_count: u64,
        applied_update_count: u64,
    },
    DirectClassificationProjected {
        event_wake_count: u64,
        applied_update_count: u64,
        projected_dm_count: usize,
    },
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveDirectEventTestSource {
    SdkAndInjected,
    InjectedOnly,
}

#[cfg(test)]
fn emit_live_observer_test_event(
    tx: &Option<mpsc::UnboundedSender<LiveObserverTestEvent>>,
    event: LiveObserverTestEvent,
) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}

#[cfg(test)]
fn projected_direct_room_count(
    current: &eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
    direct_state: &DirectClassificationState,
) -> usize {
    let Some(targets_by_room) = direct_state.authoritative_targets() else {
        return 0;
    };
    current
        .iter()
        .map(|item| item.clone().into_inner())
        .filter(|room| {
            room.state() == matrix_sdk::RoomState::Joined
                && targets_by_room.contains_key(room.room_id().as_str())
        })
        .map(|room| room.room_id().to_owned())
        .collect::<BTreeSet<_>>()
        .len()
}

/// Normalize a snapshot and project it as a generation-fenced room-list action +
/// `RoomEvent::RoomListUpdated`.
async fn project_room_list_snapshot(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    known_dm_rooms: &Arc<RwLock<Vec<RoomSummary>>>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
    generation: u64,
    source: RoomListSource,
    authoritative: bool,
) -> bool {
    let spaces = normalize_spaces(snapshot);
    let previous_dm_rooms = known_dm_rooms
        .read()
        .map(|rooms| rooms.clone())
        .unwrap_or_default();
    let rooms = normalize_rooms_with_previous(snapshot, &previous_dm_rooms);
    let invites = normalize_invites(snapshot);
    let user_profiles = normalize_user_profiles(snapshot);
    unread_trace::trace_room_list_snapshot(&rooms);
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room", "room_list_projection")
            .field(DiagnosticField::token(
                "source",
                room_list_source_label(source),
            ))
            .field(DiagnosticField::count("generation", generation))
            .field(DiagnosticField::boolean("authoritative", authoritative))
            .field(DiagnosticField::count("rooms_count", rooms.len() as u64))
            .field(DiagnosticField::count("spaces_count", spaces.len() as u64))
            .field(DiagnosticField::count(
                "complete_space_membership_count",
                snapshot.complete_space_member_ids.len() as u64,
            ))
            .field(DiagnosticField::count(
                "partial_space_membership_count",
                snapshot
                    .spaces
                    .len()
                    .saturating_sub(snapshot.complete_space_member_ids.len())
                    as u64,
            ))
            .field(DiagnosticField::count(
                "invites_count",
                invites.len() as u64,
            )),
    );
    let projected_rooms = rooms.clone();
    if authoritative {
        replace_known_room_ids(known_room_ids, &projected_rooms);
    }
    let snapshot_action = if authoritative {
        AppAction::RoomListSnapshotAuthoritative {
            generation,
            source,
            spaces,
            rooms,
            invites,
        }
    } else {
        AppAction::RoomListSnapshotProvisional {
            generation,
            source,
            spaces,
            rooms,
            invites,
        }
    };
    let delivered = action_tx
        .send(vec![
            snapshot_action,
            AppAction::UserProfilesUpdated {
                profiles: user_profiles,
            },
        ])
        .await
        .is_ok();
    let has_payload =
        !projected_rooms.is_empty() || !snapshot.spaces.is_empty() || !snapshot.invites.is_empty();
    if delivered {
        if let Ok(mut known) = known_dm_rooms.write() {
            *known = projected_rooms
                .iter()
                .filter(|room| room.is_dm)
                .cloned()
                .collect();
        }
        if authoritative || has_payload {
            let _ = event_tx.send(CoreEvent::Room(RoomEvent::RoomListUpdated));
        }
    }
    delivered
}

fn room_list_source_label(source: RoomListSource) -> &'static str {
    match source {
        RoomListSource::Cache => "cache",
        RoomListSource::Live => "live",
    }
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn direct_account_data_source_label(source: DirectAccountDataSource) -> &'static str {
    match source {
        DirectAccountDataSource::Unavailable => "unavailable",
        DirectAccountDataSource::LocalStore => "local_store",
        DirectAccountDataSource::SlidingSyncEvent => "sliding_sync_event",
    }
}

fn direct_account_data_initial_reason(
    cached: &koushi_sdk::MatrixCachedDirectAccountData,
) -> Option<&'static str> {
    match cached {
        koushi_sdk::MatrixCachedDirectAccountData::Present(_) => None,
        koushi_sdk::MatrixCachedDirectAccountData::Missing => Some("missing"),
        koushi_sdk::MatrixCachedDirectAccountData::StoreError => Some("store_error"),
        koushi_sdk::MatrixCachedDirectAccountData::Invalid => Some("invalid"),
    }
}

fn record_direct_account_data_initialization(cached: &koushi_sdk::MatrixCachedDirectAccountData) {
    let source = match cached {
        koushi_sdk::MatrixCachedDirectAccountData::Present(_) => {
            DirectAccountDataSource::LocalStore
        }
        koushi_sdk::MatrixCachedDirectAccountData::Missing
        | koushi_sdk::MatrixCachedDirectAccountData::StoreError
        | koushi_sdk::MatrixCachedDirectAccountData::Invalid => {
            DirectAccountDataSource::Unavailable
        }
    };
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Debug,
        "core.room",
        "direct_account_data_initialized",
    )
    .field(DiagnosticField::token(
        "source",
        direct_account_data_source_label(source),
    ));
    if let Some(reason) = direct_account_data_initial_reason(cached) {
        event = event.field(DiagnosticField::token("reason", reason));
    }
    record(event);
}

fn record_direct_event_stream_closed(state: &DirectClassificationState) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Warn,
            "core.room",
            "direct_event_stream_closed",
        )
        .field(DiagnosticField::token(
            "source",
            direct_account_data_source_label(state.source()),
        ))
        .field(DiagnosticField::count(
            "event_wake_count",
            state.event_wake_count(),
        ))
        .field(DiagnosticField::count(
            "event_applied_count",
            state.applied_update_count(),
        )),
    );
}

/// SyncService-path observation loop (Async rule 1: relay the SDK's
/// observable streams). Subscribes to the live `RoomListService`'s
/// `all_rooms()` entries stream (`entries_with_dynamic_adapters` with the
/// non-left filter — the same shape the live service drives with its
/// `required_state`, including `m.room.create` for space classification) and
/// KEEPS CONSUMING it: the current entry vector is maintained by applying
/// each `VectorDiff` batch, and every visible joined/invited batch triggers a
/// re-normalization. The base client's committed room-update broadcast is an
/// auxiliary wake source for membership-sensitive re-normalization,
/// mention-membership and pinned-state invalidation; it never supplies rooms
/// or invites and never owns or drives another network sync.
/// The first batch (a Reset with the current entries) doubles as the initial
/// snapshot. A refresh request (operation-triggered) re-reads the current
/// entries snapshot from that same service without creating another service or
/// sync loop. Exits on the oneshot stop
/// signal or when the stream ends.
async fn run_live_room_list_observation(
    session: Arc<MatrixClientSession>,
    service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    known_dm_rooms: Arc<RwLock<Vec<RoomSummary>>>,
    room_tx: mpsc::Sender<RoomMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    command_rx: mpsc::Receiver<RoomListObservationCommand>,
    stop_rx: oneshot::Receiver<()>,
    generation: u64,
    source: RoomListSource,
    authoritative: Arc<AtomicBool>,
    sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    timeline_residency: Option<TimelineSubscriptionResidencyHandle>,
) {
    let direct_observer = session.client().observe_events::<DirectEvent, ()>();
    let direct_events = direct_observer.subscribe();
    let cached_direct = koushi_sdk::cached_direct_account_data_targets_by_room(&session).await;
    record_direct_account_data_initialization(&cached_direct);
    let direct_state = DirectClassificationState::from_cached(cached_direct);
    sliding_sync_diagnostics.direct_classification_initialized(
        direct_state.source(),
        usize_to_u64(direct_state.targets_by_room().len()),
        usize_to_u64(
            direct_state
                .targets_by_room()
                .values()
                .map(Vec::len)
                .sum::<usize>(),
        ),
    );
    let room_updates_rx = session.client().subscribe_to_all_room_updates();
    #[cfg(test)]
    let (_direct_event_tx, direct_events_rx) = mpsc::unbounded_channel();
    #[cfg(test)]
    run_live_room_list_observation_with_sources(
        session,
        service,
        known_room_ids,
        known_dm_rooms,
        room_tx,
        action_tx,
        event_tx,
        command_rx,
        stop_rx,
        generation,
        source,
        authoritative,
        sliding_sync_diagnostics,
        timeline_residency,
        direct_observer,
        direct_events,
        direct_state,
        ROOM_LIST_ENTRIES_PAGE_SIZE,
        room_updates_rx,
        None,
        direct_events_rx,
        LiveDirectEventTestSource::SdkAndInjected,
        None,
    )
    .await;
    #[cfg(not(test))]
    run_live_room_list_observation_with_sources(
        session,
        service,
        known_room_ids,
        known_dm_rooms,
        room_tx,
        action_tx,
        event_tx,
        command_rx,
        stop_rx,
        generation,
        source,
        authoritative,
        sliding_sync_diagnostics,
        timeline_residency,
        direct_observer,
        direct_events,
        direct_state,
        ROOM_LIST_ENTRIES_PAGE_SIZE,
        room_updates_rx,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_live_room_list_observation_with_sources(
    session: Arc<MatrixClientSession>,
    service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    known_dm_rooms: Arc<RwLock<Vec<RoomSummary>>>,
    room_tx: mpsc::Sender<RoomMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut command_rx: mpsc::Receiver<RoomListObservationCommand>,
    mut stop_rx: oneshot::Receiver<()>,
    generation: u64,
    source: RoomListSource,
    authoritative: Arc<AtomicBool>,
    sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    timeline_residency: Option<TimelineSubscriptionResidencyHandle>,
    _direct_observer: matrix_sdk::event_handler::ObservableEventHandler<(DirectEvent, ())>,
    direct_events: matrix_sdk::event_handler::EventHandlerSubscriber<(DirectEvent, ())>,
    mut direct_state: DirectClassificationState,
    entries_limit: usize,
    mut room_updates_rx: broadcast::Receiver<matrix_sdk_base::sync::RoomUpdates>,
    #[cfg(test)] test_event_tx: Option<mpsc::UnboundedSender<LiveObserverTestEvent>>,
    #[cfg(test)] direct_events_rx: mpsc::UnboundedReceiver<
        matrix_sdk::ruma::events::direct::DirectEventContent,
    >,
    #[cfg(test)] direct_event_source: LiveDirectEventTestSource,
    #[cfg(test)] mut entries_start_rx: Option<mpsc::Receiver<()>>,
) {
    use futures_util::StreamExt as _;

    let sdk_direct_events = direct_events.map(|(event, ())| event.content);
    #[cfg(test)]
    let injected_direct_events =
        futures_util::stream::unfold(direct_events_rx, |mut receiver| async move {
            receiver.recv().await.map(|content| (content, receiver))
        });
    #[cfg(test)]
    let mut direct_events = match direct_event_source {
        LiveDirectEventTestSource::SdkAndInjected => {
            futures_util::stream::select(sdk_direct_events, injected_direct_events).boxed()
        }
        LiveDirectEventTestSource::InjectedOnly => injected_direct_events.boxed(),
    };
    #[cfg(not(test))]
    let mut direct_events = Box::pin(sdk_direct_events);
    let mut direct_events_closed = false;
    #[cfg(test)]
    let mut entries_enabled = entries_start_rx.is_none();
    #[cfg(not(test))]
    let mut entries_enabled = true;
    #[cfg(test)]
    let mut entries_start = Box::pin(async {
        if let Some(entries_start_rx) = entries_start_rx.as_mut() {
            let _ = entries_start_rx.recv().await;
        }
    });
    #[cfg(not(test))]
    let mut entries_start = Box::pin(futures_util::future::pending::<()>());

    let all_rooms = match service.all_rooms().await {
        Ok(all_rooms) => all_rooms,
        Err(_) => {
            record(
                DiagnosticEvent::new(DiagnosticLevel::Error, "core.room", "live_observer_exit")
                    .field(DiagnosticField::token("reason", "all_rooms_error")),
            );
            return;
        }
    };
    let (entries, entries_controller) = all_rooms.entries_with_dynamic_adapters(entries_limit);
    entries_controller.set_filter(Box::new(
        matrix_sdk_ui::room_list_service::filters::new_filter_non_left(),
    ));
    let mut entries = Box::pin(entries);
    let mut loading_state = all_rooms.loading_state();
    let mut room_updates_closed = false;
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room", "live_observer_started").field(
            DiagnosticField::count("entries_limit", entries_limit as u64),
        ),
    );

    // Current filtered entry vector, maintained by applying each diff batch.
    let mut current: eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem> =
        eyeball_im::Vector::new();
    let mut reconciliation = LiveRoomListReconciliation::default();
    if let matrix_sdk_ui::room_list_service::RoomListLoadingState::Loaded {
        maximum_number_of_rooms,
    } = loading_state.get()
    {
        reconciliation.report_maximum(maximum_number_of_rooms);
        for _ in 0..additional_room_list_pages(entries_limit, maximum_number_of_rooms) {
            entries_controller.add_one_page();
        }
    }
    // The authoritative, response-correlated snapshot is the single source of
    // truth for room-range readiness: the standalone `range_loading_state`
    // observable that the SDK used to publish is gone (koushi-matrix#888,
    // Stage 2).
    reconciliation.report_range_fully_loaded(
        all_rooms.current_entries_snapshot().range_fully_loaded() == Some(true),
    );
    let mut rls_wake_count = 0_u64;
    let mut base_wake_count = 0_u64;
    let mut entries_observed = false;

    loop {
        tokio::select! {
            _ = &mut entries_start, if !entries_enabled => {
                entries_enabled = true;
            },
            _ = &mut stop_rx => {
                record_live_observer_exit(
                    DiagnosticLevel::Debug,
                    "stopped",
                    rls_wake_count,
                    base_wake_count,
                );
                break;
            },
            maybe_command = command_rx.recv() => {
                let Some(command) = maybe_command else {
                    record_live_observer_exit(
                        DiagnosticLevel::Error,
                        "command_channel_closed",
                        rls_wake_count,
                        base_wake_count,
                    );
                    break;
                };
                // #446: a room-store snapshot may drive ONE projection and
                // reconciliation decision, but it must never become the ordered
                // accumulator that later index-based diffs address. Keep it in a
                // separate one-shot vector so `current` stays owned by the
                // dynamic-adapter diff stream.
                let observed_snapshot: Option<
                    eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
                >;
                match command {
                    RoomListObservationCommand::Refresh => {
                        // Read through the same live service that owns the
                        // entries stream. A committed response can publish
                        // its observed IDs before the matching SDK room-store
                        // update reaches the dynamic-entry stream; refreshing
                        // only `current` would therefore re-project the same
                        // stale vector and lose an invite during that window.
                        // This remains a bounded wake, not a second room-list
                        // source or a new sync service.
                        let snapshot = all_rooms.current_entries_snapshot();
                        let invited_entries = snapshot
                            .entries()
                            .iter()
                            .filter(|entry| {
                                (*entry).clone().into_inner().state()
                                    == matrix_sdk::RoomState::Invited
                            })
                            .count();
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Debug,
                                "core.room",
                                "live_observer_refresh_snapshot",
                            )
                            .field(DiagnosticField::count(
                                "entries_count",
                                snapshot.entries().len() as u64,
                            ))
                            .field(DiagnosticField::count(
                                "invited_entries_count",
                                invited_entries as u64,
                            ))
                            .field(DiagnosticField::boolean(
                                "authoritative",
                                snapshot.is_authoritative(),
                            )),
                        );
                        reconciliation.report_maximum(snapshot.maximum_number_of_rooms());
                        if let Some(range_fully_loaded) = snapshot.range_fully_loaded() {
                            reconciliation.report_range_fully_loaded(range_fully_loaded);
                        }
                        // #446: project these entries once, but do NOT assign
                        // them to `current`. They come from
                        // `client.rooms_stream()`, which is not the
                        // filtered/sorted/paged order the diff indices refer to,
                        // so making them the accumulator let the next
                        // `Set`/`Move` overwrite the wrong entry: one duplicate
                        // room id, one silently lost room, and a joined Space
                        // vanishing from the sidebar.
                        observed_snapshot = Some(snapshot.into_entries());
                    }
                    RoomListObservationCommand::HydrateSpaceMembers { space_id } => {
                        let mut attempted = false;
                        let mut succeeded = false;
                        let mut complete = false;
                        if let Ok(space_id) = matrix_sdk::ruma::OwnedRoomId::try_from(space_id)
                            && let Some(room) = session.client().get_room(&space_id)
                            && room.is_space()
                        {
                            complete = room.are_members_synced();
                            if !complete {
                                attempted = true;
                                succeeded = executor::timeout(
                                    Duration::from_secs(10),
                                    room.members(matrix_sdk::RoomMemberships::JOIN),
                                )
                                .await
                                .is_ok_and(|result| result.is_ok());
                                complete = room.are_members_synced();
                            }
                        }
                        record(
                            DiagnosticEvent::new(
                                if attempted && !succeeded {
                                    DiagnosticLevel::Warn
                                } else {
                                    DiagnosticLevel::Debug
                                },
                                "core.room",
                                "dm_space_membership_hydration",
                            )
                            .field(DiagnosticField::boolean("attempted", attempted))
                            .field(DiagnosticField::boolean("succeeded", succeeded))
                            .field(DiagnosticField::boolean("complete", complete)),
                        );
                        let snapshot = all_rooms.current_entries_snapshot();
                        reconciliation.report_maximum(snapshot.maximum_number_of_rooms());
                        if let Some(range_fully_loaded) = snapshot.range_fully_loaded() {
                            reconciliation.report_range_fully_loaded(range_fully_loaded);
                        }
                        observed_snapshot = Some(snapshot.into_entries());
                    }
                    RoomListObservationCommand::RefreshRoom { room_id } => {
                        let requested_room_id =
                            matrix_sdk::ruma::OwnedRoomId::try_from(room_id.as_str()).ok();
                        if let Some(room_id) = requested_room_id.as_ref() {
                            all_rooms.remember_room_id(room_id.clone());
                        }
                        let snapshot = all_rooms.current_entries_snapshot();
                        let requested_room_present = requested_room_id.as_ref().is_some_and(|room_id| {
                            snapshot
                                .entries()
                                .iter()
                                .any(|entry| entry.room_id() == room_id)
                        });
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Debug,
                                "core.room",
                                "live_observer_refresh_room",
                            )
                            .field(DiagnosticField::count(
                                "entries_count",
                                snapshot.entries().len() as u64,
                            ))
                            .field(DiagnosticField::boolean(
                                "requested_room_present",
                                requested_room_present,
                            ))
                            .field(DiagnosticField::boolean(
                                "authoritative",
                                snapshot.is_authoritative(),
                            )),
                        );
                        reconciliation.report_maximum(snapshot.maximum_number_of_rooms());
                        if let Some(range_fully_loaded) = snapshot.range_fully_loaded() {
                            reconciliation.report_range_fully_loaded(range_fully_loaded);
                        }
                        // #446: one-shot observation only; the ordered accumulator
                        // stays owned by the adapter's diff stream.
                        observed_snapshot = Some(snapshot.into_entries());
                    }
                    RoomListObservationCommand::Reconcile {
                        backend_generation,
                        response_sequence,
                        ack,
                    } => {
                        let snapshot = all_rooms.current_entries_snapshot();
                        let maximum_number_of_rooms = snapshot.maximum_number_of_rooms();
                        let snapshot_sequence = snapshot.response_sequence();
                        if snapshot_sequence.is_none() {
                            // The SDK can publish the committed-response signal before
                            // the RoomListService snapshot carries its response sequence.
                            // Keep the acknowledgement pending and let the next service
                            // update retry projection instead of terminating observation.
                            reconciliation.begin(backend_generation, response_sequence, ack);
                            continue;
                        }
                        if snapshot_sequence.is_some_and(|sequence| sequence < response_sequence) {
                            // The committed response and RoomListService snapshot are
                            // delivered on separate SDK observers. Wait for the service
                            // snapshot to catch up rather than treating this normal race as
                            // an infrastructure failure.
                            reconciliation.begin(backend_generation, response_sequence, ack);
                            continue;
                        }
                        let snapshot_is_complete = room_list_range_is_complete(
                            maximum_number_of_rooms,
                            snapshot.entries().len(),
                        );
                        if snapshot_sequence.is_some_and(|sequence| sequence > response_sequence)
                            && !snapshot_is_complete
                        {
                            let _ = ack.send(RoomListReconcileAck::Superseded {
                                backend_generation,
                                room_generation: generation,
                                response_sequence: snapshot_sequence
                                    .expect("checked snapshot sequence"),
                            });
                            continue;
                        }
                        reconciliation.report_maximum(maximum_number_of_rooms);
                        reconciliation.report_range_fully_loaded(true);
                        // #446: reconcile from this authoritative observation
                        // without poisoning the ordered accumulator.
                        observed_snapshot = Some(snapshot.into_entries());
                        reconciliation.begin(
                            backend_generation,
                            snapshot_sequence
                                .unwrap_or(response_sequence)
                                .max(response_sequence),
                            ack,
                        );
                    }
                }
                project_live_entries_and_ack_if_reconciled(
                    &mut reconciliation,
                    &session,
                    observed_snapshot.as_ref().unwrap_or(&current),
                    &direct_state,
                    &known_room_ids,
                    &known_dm_rooms,
                    &room_tx,
                    &action_tx,
                    &event_tx,
                    generation,
                    source,
                    &authoritative,
                    &sliding_sync_diagnostics,
                    timeline_residency.as_ref(),
                ).await;
            }
            next_loading_state = loading_state.next() => {
                let Some(next_loading_state) = next_loading_state else {
                    record_live_observer_exit(
                        DiagnosticLevel::Error,
                        "loading_state_stream_ended",
                        rls_wake_count,
                        base_wake_count,
                    );
                    break;
                };
                if let matrix_sdk_ui::room_list_service::RoomListLoadingState::Loaded {
                    maximum_number_of_rooms,
                } = next_loading_state
                {
                    reconciliation.report_maximum(maximum_number_of_rooms);
                    for _ in 0..additional_room_list_pages(entries_limit, maximum_number_of_rooms) {
                        entries_controller.add_one_page();
                    }
                    if reconciliation.has_pending_reconciliation() {
                        project_live_entries_and_ack_if_reconciled(
                            &mut reconciliation,
                            &session,
                            &current,
                            &direct_state,
                            &known_room_ids,
                            &known_dm_rooms,
                            &room_tx,
                            &action_tx,
                            &event_tx,
                            generation,
                            source,
                            &authoritative,
                            &sliding_sync_diagnostics,
                            timeline_residency.as_ref(),
                        ).await;
                    }
                }
            }
            maybe_diffs = entries.next(), if entries_enabled => match maybe_diffs {
                None => {
                    record_live_observer_exit(
                        DiagnosticLevel::Error,
                        "entries_stream_ended",
                        rls_wake_count,
                        base_wake_count,
                    );
                    break;
                },
                Some(diffs) => {
                    rls_wake_count = rls_wake_count.saturating_add(1);
                    for diff in diffs {
                        diff.apply(&mut current);
                    }
                    entries_observed = true;
                    if rls_wake_count.is_power_of_two() {
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Debug,
                                "core.room",
                                "live_observer_wake_milestone",
                            )
                            .field(DiagnosticField::token("source", "rls_diff"))
                            .field(DiagnosticField::count("wake_count", rls_wake_count))
                            .field(DiagnosticField::count("entries_count", current.len() as u64)),
                        );
                    }
                    project_live_entries_and_ack_if_reconciled(
                        &mut reconciliation,
                        &session,
                        &current,
                        &direct_state,
                        &known_room_ids,
                        &known_dm_rooms,
                        &room_tx,
                        &action_tx,
                        &event_tx,
                        generation,
                        source,
                        &authoritative,
                        &sliding_sync_diagnostics,
                        timeline_residency.as_ref(),
                    ).await;
                    #[cfg(test)]
                    emit_live_observer_test_event(
                        &test_event_tx,
                        LiveObserverTestEvent::RlsProjected {
                            wake_count: rls_wake_count,
                            entries_len: current.len(),
                        },
                    );
                }
            },
            room_update = room_updates_rx.recv(), if !room_updates_closed => {
                let mut update_count = 0_u64;
                let mut lagged = false;
                let mut updated_joined_room_ids = BTreeSet::new();
                let mut projection_required = false;
                let mut invite_update_observed = false;
                let mut invite_membership_changed = false;
                let mut pinned_event_room_ids = BTreeSet::new();
                let mut membership_batches = Vec::new();
                match room_update {
                    Ok(updates) => {
                        update_count = 1;
                        membership_batches.push(room_membership_transitions(&updates));
                        projection_required = room_updates_require_room_list_projection(&updates);
                        invite_update_observed =
                            !updates.invited.is_empty() || !updates.left.is_empty();
                        invite_membership_changed = room_updates_include_joined_state(&updates);
                        updated_joined_room_ids.extend(
                            updates.joined.keys().map(ToString::to_string),
                        );
                        pinned_event_room_ids.extend(crate::room::pins::pinned_event_room_ids(&updates));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => lagged = true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        room_updates_closed = true;
                    }
                }
                loop {
                    match room_updates_rx.try_recv() {
                        Ok(updates) => {
                            update_count = update_count.saturating_add(1);
                            membership_batches.push(room_membership_transitions(&updates));
                            projection_required |= room_updates_require_room_list_projection(&updates);
                            invite_update_observed |=
                                !updates.invited.is_empty() || !updates.left.is_empty();
                            invite_membership_changed |= room_updates_include_joined_state(&updates);
                            updated_joined_room_ids.extend(
                                updates.joined.keys().map(ToString::to_string),
                            );
                            pinned_event_room_ids.extend(crate::room::pins::pinned_event_room_ids(&updates));
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                            lagged = true;
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                            room_updates_closed = true;
                            break;
                        }
                    }
                }

                projection_required |= lagged;
                if room_updates_closed {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Warn,
                            "core.room",
                            "live_observer_auxiliary_closed",
                        )
                        .field(DiagnosticField::token("source", "base_room_updates"))
                        .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
                        .field(DiagnosticField::count("base_wake_count", base_wake_count)),
                    );
                }

                base_wake_count = base_wake_count.saturating_add(1);
                if base_wake_count.is_power_of_two() {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Debug,
                            "core.room",
                            "live_observer_wake_milestone",
                        )
                        .field(DiagnosticField::token("source", "base_room_updates"))
                        .field(DiagnosticField::count("wake_count", base_wake_count))
                        .field(DiagnosticField::count("drained_update_count", update_count))
                        .field(DiagnosticField::boolean("lagged", lagged))
                        .field(DiagnosticField::boolean(
                            "invite_update_observed",
                            invite_update_observed,
                        ))
                        .field(DiagnosticField::boolean(
                            "invite_membership_changed",
                            invite_membership_changed,
                        ))
                        .field(DiagnosticField::boolean(
                            "projection_required",
                            projection_required,
                        )),
                    );
                }
                if lagged {
                    record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Warn,
                            "core.room",
                            "live_observer_base_lagged",
                        )
                        .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
                        .field(DiagnosticField::count("base_wake_count", base_wake_count))
                        .field(DiagnosticField::count("drained_update_count", update_count))
                        .field(DiagnosticField::boolean(
                            "projection_required",
                            projection_required,
                        )),
                    );
                }
                if projection_required {
                    project_live_entries_and_ack_if_reconciled(
                        &mut reconciliation,
                        &session,
                        &current,
                        &direct_state,
                        &known_room_ids,
                        &known_dm_rooms,
                        &room_tx,
                        &action_tx,
                        &event_tx,
                        generation,
                        source,
                        &authoritative,
                        &sliding_sync_diagnostics,
                        timeline_residency.as_ref(),
                    )
                    .await;
                }
                let _ = forward_membership_batches(
                    timeline_residency.as_ref(),
                    generation,
                    membership_batches,
                )
                .await;
                if invite_update_observed {
                    let _ = room_tx
                        .send(RoomMessage::RefreshMembershipProjection {
                            source,
                            room_generation: generation,
                        })
                        .await;
                }
                if lagged || !updated_joined_room_ids.is_empty() {
                    let _ = room_tx
                        .send(RoomMessage::MentionMembershipChanged {
                            room_ids: (!lagged).then_some(updated_joined_room_ids),
                        })
                        .await;
                }
                if !pinned_event_room_ids.is_empty() {
                    let _ = room_tx
                        .send(RoomMessage::PinnedEventsChanged {
                            room_ids: pinned_event_room_ids,
                        })
                        .await;
                }
            }
            next_direct = direct_events.next(), if !direct_events_closed => {
                match next_direct {
                    Some(content) => {
                        let changed = direct_state.replace_targets(
                            koushi_sdk::direct_account_data_targets_by_room(&content),
                        );
                        sliding_sync_diagnostics.direct_event_recorded(
                            direct_state.source(),
                            usize_to_u64(direct_state.targets_by_room().len()),
                            usize_to_u64(
                                direct_state
                                    .targets_by_room()
                                    .values()
                                    .map(Vec::len)
                                    .sum::<usize>(),
                            ),
                            direct_state.event_wake_count(),
                            direct_state.applied_update_count(),
                            true,
                        );
                        #[cfg(test)]
                        emit_live_observer_test_event(
                            &test_event_tx,
                            LiveObserverTestEvent::DirectClassificationUpdated {
                                event_wake_count: direct_state.event_wake_count(),
                                applied_update_count: direct_state.applied_update_count(),
                            },
                        );
                        if changed && entries_observed {
                            project_live_entries_and_ack_if_reconciled(
                                &mut reconciliation,
                                &session,
                                &current,
                                &direct_state,
                                &known_room_ids,
                                &known_dm_rooms,
                                &room_tx,
                                &action_tx,
                                &event_tx,
                                generation,
                                source,
                                &authoritative,
                                &sliding_sync_diagnostics,
                                timeline_residency.as_ref(),
                            )
                            .await;
                            #[cfg(test)]
                            emit_live_observer_test_event(
                                &test_event_tx,
                                LiveObserverTestEvent::DirectClassificationProjected {
                                    event_wake_count: direct_state.event_wake_count(),
                                    applied_update_count: direct_state.applied_update_count(),
                                    projected_dm_count: projected_direct_room_count(
                                        &current,
                                        &direct_state,
                                    ),
                                },
                            );
                        }
                    }
                    None => {
                        direct_events_closed = true;
                        sliding_sync_diagnostics.direct_event_recorded(
                            direct_state.source(),
                            usize_to_u64(direct_state.targets_by_room().len()),
                            usize_to_u64(
                                direct_state
                                    .targets_by_room()
                                    .values()
                                    .map(Vec::len)
                                    .sum::<usize>(),
                            ),
                            direct_state.event_wake_count(),
                            direct_state.applied_update_count(),
                            false,
                        );
                        #[cfg(test)]
                        emit_live_observer_test_event(
                            &test_event_tx,
                            LiveObserverTestEvent::DirectEventStreamClosed,
                        );
                        record_direct_event_stream_closed(&direct_state);
                    }
                }
            }
        }
    }
}

fn record_live_observer_exit(
    level: DiagnosticLevel,
    reason: &'static str,
    rls_wake_count: u64,
    base_wake_count: u64,
) {
    record(
        DiagnosticEvent::new(level, "core.room", "live_observer_exit")
            .field(DiagnosticField::token("reason", reason))
            .field(DiagnosticField::count("rls_wake_count", rls_wake_count))
            .field(DiagnosticField::count("base_wake_count", base_wake_count)),
    );
}

pub(super) fn record_residency_admission_failure(reason: &'static str) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Warn, "core.room", "residency_admission")
            .field(DiagnosticField::token("reason", reason)),
    );
}

pub(super) fn record_residency_ack_failure() {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Warn, "core.room", "residency_ack")
            .field(DiagnosticField::token("reason", "manager_unavailable")),
    );
}

fn room_membership_transitions(
    updates: &matrix_sdk_base::sync::RoomUpdates,
) -> Vec<RoomMembershipTransition> {
    updates
        .left
        .keys()
        .map(|room_id| RoomMembershipTransition {
            room_id: room_id.clone(),
            kind: RoomMembershipTransitionKind::Left,
        })
        .chain(
            updates
                .joined
                .keys()
                .map(|room_id| RoomMembershipTransition {
                    room_id: room_id.clone(),
                    kind: RoomMembershipTransitionKind::Joined,
                }),
        )
        .chain(
            updates
                .invited
                .keys()
                .map(|room_id| RoomMembershipTransition {
                    room_id: room_id.clone(),
                    kind: RoomMembershipTransitionKind::Invited,
                }),
        )
        .collect()
}

fn room_updates_include_joined_state(updates: &matrix_sdk_base::sync::RoomUpdates) -> bool {
    updates.joined.values().any(|update| match &update.state {
        matrix_sdk_base::sync::State::Before(events)
        | matrix_sdk_base::sync::State::After(events) => !events.is_empty(),
    })
}

fn room_updates_require_room_list_projection(updates: &matrix_sdk_base::sync::RoomUpdates) -> bool {
    !updates.left.is_empty()
        || !updates.invited.is_empty()
        || room_updates_include_joined_state(updates)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RoomListIdentityCounts {
    input_entry_count: usize,
    unique_room_id_count: usize,
    duplicate_entry_count: usize,
    display_name_collision_group_count: usize,
    display_name_collision_entry_count: usize,
}

fn room_list_identity_counts<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> RoomListIdentityCounts {
    let mut names_by_room_id = BTreeMap::new();
    let mut input_entry_count = 0;
    for (room_id, display_name) in entries {
        input_entry_count += 1;
        names_by_room_id
            .entry(room_id)
            .or_insert(display_name.trim());
    }
    let mut name_counts = BTreeMap::<&str, usize>::new();
    for display_name in names_by_room_id
        .values()
        .copied()
        .filter(|name| !name.is_empty())
    {
        *name_counts.entry(display_name).or_default() += 1;
    }
    let collision_counts = name_counts.values().copied().filter(|count| *count > 1);
    let collision_counts = collision_counts.collect::<Vec<_>>();
    RoomListIdentityCounts {
        input_entry_count,
        unique_room_id_count: names_by_room_id.len(),
        duplicate_entry_count: input_entry_count.saturating_sub(names_by_room_id.len()),
        display_name_collision_group_count: collision_counts.len(),
        display_name_collision_entry_count: collision_counts.into_iter().sum(),
    }
}

/// Normalize the live service's current entries and project the result.
async fn normalize_and_project_entries(
    _session: &MatrixClientSession,
    current: &eyeball_im::Vector<matrix_sdk_ui::room_list_service::RoomListItem>,
    direct_targets_by_room: Option<&koushi_sdk::MatrixDirectTargetsByRoom>,
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    known_dm_rooms: &Arc<RwLock<Vec<RoomSummary>>>,
    room_tx: &mpsc::Sender<RoomMessage>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
    generation: u64,
    source: RoomListSource,
    authoritative: &Arc<AtomicBool>,
    direct_state: Option<&DirectClassificationState>,
    sliding_sync_diagnostics: Option<&crate::SlidingSyncDiagnostics>,
) -> bool {
    // Collect before the await: mapping lazily across the await trips a
    // higher-ranked lifetime check on the iterator closure.
    let mut joined_rooms = Vec::with_capacity(current.len());
    let mut invited_rooms = Vec::new();
    let mut seen_room_ids = BTreeSet::new();
    let mut excluded_membership_count = 0_usize;
    for item in current.iter() {
        let room = item.clone().into_inner();
        if !seen_room_ids.insert(room.room_id().to_owned()) {
            continue;
        }
        match room.state() {
            matrix_sdk::RoomState::Joined => joined_rooms.push(room),
            matrix_sdk::RoomState::Invited => invited_rooms.push(room),
            matrix_sdk::RoomState::Knocked
            | matrix_sdk::RoomState::Left
            | matrix_sdk::RoomState::Banned => excluded_membership_count += 1,
        }
    }
    let joined_count = joined_rooms.len();
    let invited_count = invited_rooms.len();
    let mut snapshot = koushi_sdk::room_list_snapshot_from_sdk_rooms_with_direct_targets(
        joined_rooms,
        direct_targets_by_room,
    )
    .await;
    snapshot.invites = invite_previews_from_service_entries(invited_rooms).await;
    if let (Some(direct_state), Some(diagnostics)) = (direct_state, sliding_sync_diagnostics) {
        let projected_dms = snapshot.rooms.iter().filter(|room| room.is_dm).count();
        let explicit_dms = direct_state.authoritative_targets().map_or(0, |targets| {
            snapshot
                .rooms
                .iter()
                .filter(|room| room.is_dm && targets.contains_key(room.room_id.as_str()))
                .count()
        });
        diagnostics.direct_projection_recorded(
            usize_to_u64(projected_dms),
            usize_to_u64(explicit_dms),
            usize_to_u64(projected_dms.saturating_sub(explicit_dms)),
            usize_to_u64(snapshot.rooms.len().saturating_sub(projected_dms)),
            direct_state.invalid_entry_count(),
        );
    }
    let identity = room_list_identity_counts(
        snapshot
            .rooms
            .iter()
            .map(|room| (room.room_id.as_str(), room.display_name.as_str()))
            .chain(
                snapshot
                    .spaces
                    .iter()
                    .map(|space| (space.space_id.as_str(), space.display_name.as_str())),
            ),
    );
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.room", "room_list_input")
            .field(DiagnosticField::count(
                "sdk_store_entry_count",
                current.len() as u64,
            ))
            .field(DiagnosticField::count(
                "unique_input_room_id_count",
                seen_room_ids.len() as u64,
            ))
            .field(DiagnosticField::count(
                "duplicate_input_entry_count",
                current.len().saturating_sub(seen_room_ids.len()) as u64,
            ))
            .field(DiagnosticField::count("joined_count", joined_count as u64))
            .field(DiagnosticField::count(
                "invited_count",
                invited_count as u64,
            ))
            .field(DiagnosticField::count(
                "excluded_membership_count",
                excluded_membership_count as u64,
            ))
            .field(DiagnosticField::count(
                "normalized_input_entry_count",
                identity.input_entry_count as u64,
            ))
            .field(DiagnosticField::count(
                "normalized_unique_room_id_count",
                identity.unique_room_id_count as u64,
            ))
            .field(DiagnosticField::count(
                "normalized_duplicate_entry_count",
                identity.duplicate_entry_count as u64,
            ))
            .field(DiagnosticField::count(
                "display_name_collision_group_count",
                identity.display_name_collision_group_count as u64,
            ))
            .field(DiagnosticField::count(
                "display_name_collision_entry_count",
                identity.display_name_collision_entry_count as u64,
            )),
    );
    relay_missing_space_child_links(&snapshot, room_tx).await;
    project_room_list_snapshot(
        &snapshot,
        known_room_ids,
        known_dm_rooms,
        action_tx,
        event_tx,
        generation,
        source,
        authoritative.load(Ordering::Acquire),
    )
    .await
}

async fn invite_previews_from_service_entries(
    rooms: impl IntoIterator<Item = matrix_sdk::Room>,
) -> Vec<koushi_sdk::MatrixInvitePreview> {
    let mut invites = Vec::new();
    for room in rooms {
        if room.state() != matrix_sdk::RoomState::Invited {
            // The SDK room object is live and can transition to Joined after
            // the entries vector was collected. Do not panic or re-project a
            // stale invite during that normal acceptance race.
            continue;
        }
        let display_name = room
            .display_name()
            .await
            .ok()
            .map(|name| name.to_string())
            .or_else(|| room.name())
            .unwrap_or_else(|| "Invite".to_owned());
        let inviter = room
            .invite_details()
            .await
            .ok()
            .and_then(|details| details.inviter);
        let inviter_display_name = inviter
            .as_ref()
            .and_then(|inviter| inviter.display_name().map(ToOwned::to_owned));
        let inviter_user_id = inviter.map(|inviter| inviter.user_id().to_string());
        let is_dm = room.is_direct().await.unwrap_or(false);

        invites.push(koushi_sdk::MatrixInvitePreview {
            room_id: room.room_id().to_string(),
            display_name,
            avatar_mxc_uri: room.avatar_url().map(|uri| uri.to_string()),
            topic: room.topic(),
            inviter_display_name,
            inviter_user_id,
            is_dm,
        });
    }
    invites
}

async fn relay_missing_space_child_links(
    snapshot: &MatrixRoomListSnapshot,
    room_tx: &mpsc::Sender<RoomMessage>,
) {
    let links = missing_space_child_links(snapshot);
    if !links.is_empty() {
        let _ = room_tx
            .send(RoomMessage::MissingSpaceChildLinks { links })
            .await;
    }
}

fn missing_space_child_links(snapshot: &MatrixRoomListSnapshot) -> Vec<MissingSpaceChildLink> {
    let mut links = Vec::new();
    for room in &snapshot.rooms {
        for space in &snapshot.spaces {
            if room_has_parent_without_space_child(room, space)
                && let Ok(via_server) = koushi_sdk::room_id_server_name(&room.room_id)
            {
                links.push(MissingSpaceChildLink {
                    space_id: space.space_id.clone(),
                    child_room_id: room.room_id.clone(),
                    via_server,
                });
            }
        }
    }
    links.sort_by(|left, right| {
        left.space_id
            .cmp(&right.space_id)
            .then_with(|| left.child_room_id.cmp(&right.child_room_id))
    });
    links.dedup_by(|left, right| {
        left.space_id == right.space_id && left.child_room_id == right.child_room_id
    });
    links
}

fn room_has_parent_without_space_child(
    room: &MatrixRoomListRoom,
    space: &MatrixRoomListSpace,
) -> bool {
    room.parent_space_ids
        .iter()
        .any(|space_id| space_id == &space.space_id)
        && !space
            .child_room_ids
            .iter()
            .any(|child_room_id| child_room_id == &room.room_id)
}

pub(super) fn state_contains_pinned_events(state: &matrix_sdk_base::sync::State) -> bool {
    let events = match state {
        matrix_sdk_base::sync::State::Before(events)
        | matrix_sdk_base::sync::State::After(events) => events,
    };
    events.iter().any(|event| {
        serde_json::from_str::<serde_json::Value>(event.json().get())
            .ok()
            .and_then(|json| {
                json.get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("m.room.pinned_events")
    })
}

impl RoomActor {
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) async fn handle_test_visible_rooms_observed(
        &mut self,
        core_generation: u64,
        reconciliation_is_complete: bool,
        room_ids: Vec<VisibleRoomObservation>,
        forwarded: oneshot::Sender<bool>,
    ) {
        let entries_count = room_ids.len();
        let distinct_identity_count = room_ids
            .iter()
            .map(|observation| observation.room_id.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let current_session = self.session.as_ref();
        let timeline_residency = self
            .timeline_residency
            .borrow()
            .as_ref()
            .filter(|binding| {
                current_session.is_some_and(|session| Arc::ptr_eq(&binding.session, session))
            })
            .map(|binding| binding.handle.clone());
        let forwarded_result = forward_visible_rooms_if_authoritative(
            timeline_residency.as_ref(),
            core_generation,
            reconciliation_is_complete,
            entries_count,
            distinct_identity_count,
            room_ids,
        )
        .await;
        let _ = forwarded.send(forwarded_result);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) async fn handle_test_membership_observed(
        &mut self,
        core_generation: u64,
        transitions: Vec<RoomMembershipTransition>,
        forwarded: oneshot::Sender<bool>,
    ) {
        let current_session = self.session.as_ref();
        let timeline_residency = self
            .timeline_residency
            .borrow()
            .as_ref()
            .filter(|binding| {
                current_session.is_some_and(|session| Arc::ptr_eq(&binding.session, session))
            })
            .map(|binding| binding.handle.clone());
        let forwarded_result =
            forward_membership_batches(timeline_residency.as_ref(), core_generation, [transitions])
                .await;
        let _ = forwarded.send(forwarded_result);
    }

    /// Spawn the live-service observation loop (SyncService backend): relay
    /// the ONE live `RoomListService`'s entries stream and re-normalize on
    /// each diff batch.
    pub(super) fn start_live_observation(
        &mut self,
        session: Arc<MatrixClientSession>,
        service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
        generation: u64,
        source: RoomListSource,
        timeline_residency: Option<TimelineSubscriptionResidencyHandle>,
    ) {
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (command_tx, command_rx) = mpsc::channel::<RoomListObservationCommand>(8);
        let authoritative = Arc::new(AtomicBool::new(false));
        let task = executor::spawn(run_live_room_list_observation(
            session,
            service,
            self.known_room_ids.clone(),
            self.known_dm_rooms.clone(),
            self.self_tx.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            command_rx,
            stop_rx,
            generation,
            source,
            authoritative.clone(),
            self.sliding_sync_diagnostics.clone(),
            timeline_residency,
        ));
        self.observation = Some(RoomListObservation {
            stop_tx,
            task,
            command_tx,
            generation,
            source,
        });
    }

    /// Stop the observation loop (if running) and wait for it to exit.
    pub(super) async fn stop_observation(&mut self) {
        if let Some(mut observation) = self.observation.take() {
            let _ = observation.stop_tx.send(());
            if executor::timeout(
                ROOM_OBSERVATION_SHUTDOWN_JOIN_TIMEOUT,
                &mut observation.task,
            )
            .await
            .is_err()
            {
                observation.task.abort();
                let _ = observation.task.await;
            }
        }
    }

    /// Request a room-list refresh and projection into AppState via the action
    /// channel. Also emits `RoomEvent::RoomListUpdated` as a discrete event.
    ///
    /// This requests a re-normalization from the live service's current
    /// entries (inside the observation loop) — NEVER a new `RoomListService`.
    /// Before sync starts this is intentionally a no-op: reducer-owned cached
    /// rows remain visible until the live service begins projecting.
    pub(super) fn refresh_room_list(&self) {
        self.refresh_room_list_with_command(RoomListObservationCommand::Refresh);
    }

    /// Seed the same live service with a room returned by a successful local
    /// operation, then use its normal snapshot projection. This is needed for
    /// newly-created DMs whose list position is not present until a later
    /// server response; it does not create a second service or sync loop.
    pub(super) fn refresh_room_list_for_room(&self, room_id: &str) {
        self.refresh_room_list_with_command(RoomListObservationCommand::RefreshRoom {
            room_id: room_id.to_owned(),
        });
    }

    fn refresh_room_list_with_command(&self, command: RoomListObservationCommand) {
        if let Some(observation) = &self.observation {
            let retry_room_id = match &command {
                RoomListObservationCommand::Refresh => None,
                RoomListObservationCommand::RefreshRoom { room_id } => Some(room_id.clone()),
                RoomListObservationCommand::HydrateSpaceMembers { .. }
                | RoomListObservationCommand::Reconcile { .. } => return,
            };
            let command_tx = observation.command_tx.clone();
            let _ = command_tx.try_send(command);
            // A successful local mutation can update the SDK room store just
            // after the immediate refresh observes the live list. Keep the
            // same live-service projection authoritative by retrying a few
            // bounded wakes; this never creates another service or network
            // sync loop.
            let _ = executor::spawn(async move {
                for delay in [
                    Duration::from_millis(100),
                    Duration::from_millis(300),
                    Duration::from_millis(1_000),
                ] {
                    executor::sleep(delay).await;
                    let retry_command = match retry_room_id.as_deref() {
                        Some(room_id) => RoomListObservationCommand::RefreshRoom {
                            room_id: room_id.to_owned(),
                        },
                        None => RoomListObservationCommand::Refresh,
                    };
                    if command_tx.send(retry_command).await.is_err() {
                        break;
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests;
