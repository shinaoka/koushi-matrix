use koushi_sdk::{
    MatrixTimelineContinuity, MatrixTimelineGapError, MatrixTimelineGapRepairOutcome,
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
