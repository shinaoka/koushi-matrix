use koushi_sdk::{
    MatrixCommittedRoomTimelineBackend, MatrixCommittedRoomTimelineCheckpoint,
    MatrixCommittedRoomTimelineOrigin, MatrixCommittedRoomUpdatesResponse,
    MatrixLiveTailRefreshCancellation, MatrixLiveTailRefreshOutcome, MatrixLiveTailRefreshResult,
    MatrixRoomSubscriptionCheckpoint, MatrixTimelineContinuity, MatrixTimelineGapError,
    MatrixTimelineGapHandle, MatrixTimelineGapRepairOutcome, PersistableMatrixSession,
};

#[test]
fn live_tail_refresh_debug_is_privacy_safe() {
    let cancellation = MatrixLiveTailRefreshCancellation::new();
    let debug = format!("{cancellation:?}");
    assert_eq!(debug, "MatrixLiveTailRefreshCancellation(..)");
    assert!(!debug.contains("!room"));
    assert!(!debug.contains("token"));
}

#[test]
fn live_tail_refresh_outcomes_are_token_free_and_coarse() {
    let outcome = MatrixLiveTailRefreshOutcome::Detached {
        events: 128,
        historical_gap_remaining: true,
    };
    assert_eq!(
        format!("{outcome:?}"),
        "Detached { events: 128, historical_gap_remaining: true }"
    );
}

#[test]
fn live_tail_refresh_invalid_and_unavailable_rooms_fail_coarsely() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = koushi_sdk::restore_session(&persistable)
            .await
            .expect("synthetic session should restore");
        let failed = MatrixLiveTailRefreshResult {
            outcome: MatrixLiveTailRefreshOutcome::Failed,
            returned_events: 0,
            last_projection_batch: None,
        };

        for room_id in ["not-a-room-id", "!missing-room:example.invalid"] {
            let result = session
                .refresh_room_live_tail(
                    room_id,
                    64,
                    7,
                    11,
                    MatrixLiveTailRefreshCancellation::new(),
                )
                .await;

            assert_eq!(result, failed);
            let debug = format!("{result:?}");
            assert!(!debug.contains(room_id));
            assert!(!debug.contains("synthetic-access"));
            assert!(!debug.contains("error"));
        }
    });
}

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
    ) -> Option<MatrixCommittedRoomTimelineCheckpoint> =
        MatrixCommittedRoomTimelineCheckpoint::from_legacy_room_absent;
    let response_room_checkpoint: fn(
        &MatrixCommittedRoomUpdatesResponse,
        &matrix_sdk::ruma::RoomId,
    ) -> Option<MatrixCommittedRoomTimelineCheckpoint> =
        MatrixCommittedRoomUpdatesResponse::room_checkpoint;
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
        response_room_checkpoint,
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
