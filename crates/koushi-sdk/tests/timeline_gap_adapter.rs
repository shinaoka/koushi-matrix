use koushi_sdk::{
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
