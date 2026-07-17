#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveCatchupGate {
    AwaitingCheckpoint,
    Stale,
    NoTimelineUpdate,
    NoGap,
    RepairCheckpointGap,
}

impl LiveCatchupGate {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::AwaitingCheckpoint => "awaiting_subscription_response",
            Self::Stale => "stale",
            Self::NoTimelineUpdate => "no_timeline_update",
            Self::NoGap => "checkpoint_anchored",
            Self::RepairCheckpointGap => "checkpoint_gap_matches_selection",
        }
    }
}

pub(crate) fn classify_live_catchup_gate(
    expected_generation: Option<u64>,
    checkpoint: Option<(u64, bool, bool)>,
) -> LiveCatchupGate {
    let Some((checkpoint_generation, has_timeline, has_gap)) = checkpoint else {
        return LiveCatchupGate::AwaitingCheckpoint;
    };
    if expected_generation.is_some_and(|expected| checkpoint_generation != expected) {
        return LiveCatchupGate::Stale;
    }
    if !has_timeline {
        return LiveCatchupGate::NoTimelineUpdate;
    }
    if !has_gap {
        return LiveCatchupGate::NoGap;
    }
    LiveCatchupGate::RepairCheckpointGap
}

#[cfg(test)]
mod tests {
    use super::{LiveCatchupGate, classify_live_catchup_gate};

    #[test]
    fn live_edge_waits_for_the_current_subscription_checkpoint() {
        assert_eq!(
            classify_live_catchup_gate(Some(7), None),
            LiveCatchupGate::AwaitingCheckpoint,
        );
        assert_eq!(
            classify_live_catchup_gate(Some(7), Some((6, true, true))),
            LiveCatchupGate::Stale,
        );
        assert_eq!(
            classify_live_catchup_gate(Some(7), Some((7, false, false))),
            LiveCatchupGate::NoTimelineUpdate,
        );
        assert_eq!(
            classify_live_catchup_gate(Some(7), Some((7, true, false))),
            LiveCatchupGate::NoGap,
        );
        assert_eq!(
            classify_live_catchup_gate(Some(7), Some((7, true, true))),
            LiveCatchupGate::RepairCheckpointGap,
        );
        assert_eq!(
            classify_live_catchup_gate(None, None),
            LiveCatchupGate::AwaitingCheckpoint,
        );
    }

    #[test]
    fn legacy_live_edge_waits_for_a_committed_response() {
        assert_eq!(
            classify_live_catchup_gate(None, None),
            LiveCatchupGate::AwaitingCheckpoint,
        );
        assert_eq!(
            classify_live_catchup_gate(None, Some((19, true, true))),
            LiveCatchupGate::RepairCheckpointGap,
        );
    }
}
