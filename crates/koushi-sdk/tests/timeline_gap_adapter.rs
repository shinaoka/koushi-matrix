use koushi_sdk::{
    MatrixCommittedRoomTimelineBackend, MatrixCommittedRoomTimelineCheckpoint,
    MatrixCommittedRoomTimelineOrigin, MatrixCommittedRoomUpdatesResponse,
    MatrixRoomSubscriptionCheckpoint, MatrixTimelineContinuity, MatrixTimelineGapError,
    MatrixTimelineGapHandle, MatrixTimelineGapRepairOutcome,
};

#[test]
fn public_gap_contract_is_token_free_and_coarse() {
    assert_eq!(
        MatrixTimelineContinuity::Unknown,
        MatrixTimelineContinuity::Unknown
    );
    assert_eq!(
        MatrixTimelineContinuity::Gapped,
        MatrixTimelineContinuity::Gapped
    );
    assert_eq!(
        MatrixTimelineContinuity::Complete,
        MatrixTimelineContinuity::Complete
    );

    assert_eq!(
        MatrixTimelineGapRepairOutcome::Progress { events: 4 },
        MatrixTimelineGapRepairOutcome::Progress { events: 4 }
    );
    assert_eq!(
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 2
        },
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded: 2
        }
    );
}

#[test]
fn committed_room_checkpoint_contract_is_backend_neutral_and_closed() {
    let from_subscription: fn(
        &matrix_sdk_ui::room_list_service::RoomSubscriptionCheckpoint,
    ) -> MatrixCommittedRoomTimelineCheckpoint =
        MatrixCommittedRoomTimelineCheckpoint::from_room_subscription;
    let from_legacy: fn(
        &matrix_sdk::event_cache::CommittedRoomTimelineObservation,
    ) -> MatrixCommittedRoomTimelineCheckpoint =
        MatrixCommittedRoomTimelineCheckpoint::from_committed_observation;
    let response_from_sdk: fn(
        &matrix_sdk::event_cache::CommittedRoomUpdatesResponse,
    ) -> MatrixCommittedRoomUpdatesResponse = MatrixCommittedRoomUpdatesResponse::from_sdk;
    let from_absent: fn(
        &MatrixCommittedRoomUpdatesResponse,
        &matrix_sdk::ruma::RoomId,
    ) -> MatrixCommittedRoomTimelineCheckpoint =
        MatrixCommittedRoomTimelineCheckpoint::from_legacy_room_absent;
    let backend: fn(&MatrixCommittedRoomTimelineCheckpoint) -> MatrixCommittedRoomTimelineBackend =
        MatrixCommittedRoomTimelineCheckpoint::backend;
    let generation: fn(&MatrixCommittedRoomTimelineCheckpoint) -> u64 =
        MatrixCommittedRoomTimelineCheckpoint::generation;
    let room_id: fn(&MatrixCommittedRoomTimelineCheckpoint) -> &str =
        MatrixCommittedRoomTimelineCheckpoint::room_id;
    let has_timeline: fn(&MatrixCommittedRoomTimelineCheckpoint) -> bool =
        MatrixCommittedRoomTimelineCheckpoint::has_timeline_update;
    let has_gap: fn(&MatrixCommittedRoomTimelineCheckpoint) -> bool =
        MatrixCommittedRoomTimelineCheckpoint::has_inserted_gap;
    let gap_handle: fn(&MatrixCommittedRoomTimelineCheckpoint) -> Option<MatrixTimelineGapHandle> =
        MatrixCommittedRoomTimelineCheckpoint::inserted_gap_handle;
    let matches_gap: fn(&MatrixCommittedRoomTimelineCheckpoint, &MatrixTimelineGapHandle) -> bool =
        MatrixCommittedRoomTimelineCheckpoint::matches_gap;
    let origin: fn(&MatrixCommittedRoomTimelineCheckpoint) -> MatrixCommittedRoomTimelineOrigin =
        MatrixCommittedRoomTimelineCheckpoint::origin;
    let is_room_absent: fn(&MatrixCommittedRoomTimelineCheckpoint) -> bool =
        MatrixCommittedRoomTimelineCheckpoint::is_room_absent;
    let response_generation: fn(&MatrixCommittedRoomUpdatesResponse) -> u64 =
        MatrixCommittedRoomUpdatesResponse::generation;
    let joined_room_count: fn(&MatrixCommittedRoomUpdatesResponse) -> usize =
        MatrixCommittedRoomUpdatesResponse::joined_room_count;
    let left_room_count: fn(&MatrixCommittedRoomUpdatesResponse) -> usize =
        MatrixCommittedRoomUpdatesResponse::left_room_count;
    let invited_room_count: fn(&MatrixCommittedRoomUpdatesResponse) -> usize =
        MatrixCommittedRoomUpdatesResponse::invited_room_count;
    let _ = (
        from_subscription,
        from_legacy,
        response_from_sdk,
        from_absent,
        backend,
        generation,
        room_id,
        has_timeline,
        has_gap,
        gap_handle,
        matches_gap,
        origin,
        is_room_absent,
        response_generation,
        joined_room_count,
        left_room_count,
        invited_room_count,
    );
}

#[test]
fn room_subscription_checkpoint_contract_is_closed_and_token_free() {
    let generation: fn(&MatrixRoomSubscriptionCheckpoint) -> u64 =
        MatrixRoomSubscriptionCheckpoint::subscription_generation;
    let room_id: fn(&MatrixRoomSubscriptionCheckpoint) -> &str =
        MatrixRoomSubscriptionCheckpoint::room_id;
    let has_timeline: fn(&MatrixRoomSubscriptionCheckpoint) -> bool =
        MatrixRoomSubscriptionCheckpoint::has_timeline_update;
    let has_gap: fn(&MatrixRoomSubscriptionCheckpoint) -> bool =
        MatrixRoomSubscriptionCheckpoint::has_inserted_gap;
    let matches_gap: fn(&MatrixRoomSubscriptionCheckpoint, &MatrixTimelineGapHandle) -> bool =
        MatrixRoomSubscriptionCheckpoint::matches_gap;
    let _ = (generation, room_id, has_timeline, has_gap, matches_gap);
}

#[test]
fn gap_handle_exposes_only_a_coarse_topology_revision() {
    let accessor: fn(&MatrixTimelineGapHandle) -> u64 = MatrixTimelineGapHandle::topology_revision;
    let _ = accessor;
}

#[test]
fn gap_errors_never_carry_raw_sdk_details() {
    for error in [
        MatrixTimelineGapError::InvalidRoom,
        MatrixTimelineGapError::RoomUnavailable,
        MatrixTimelineGapError::Sdk,
    ] {
        let debug = format!("{error:?}");
        assert!(!debug.contains("secret-token"));
        assert!(!debug.contains("!private-room"));
    }
}
