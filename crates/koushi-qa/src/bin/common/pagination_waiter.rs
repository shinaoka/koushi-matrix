use koushi_core::runtime::CoreConnection;
use koushi_protocol::{
    command::{CoreCommand, TimelineCommand},
    event::{CoreEvent, PaginationDirection, PaginationState, TimelineEvent},
    ids::{RequestId, TimelineKey},
};
use tokio::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitingAcceptance,
    Paginating,
    AwaitingGapRelease,
    NeedsRequest,
    Finished,
}

impl Phase {
    /// A bounded, identifier-free token for the timeout diagnostic.
    fn token(self) -> &'static str {
        match self {
            Phase::AwaitingAcceptance => "awaiting_acceptance",
            Phase::Paginating => "paginating",
            Phase::AwaitingGapRelease => "awaiting_gap_release",
            Phase::NeedsRequest => "needs_request",
            Phase::Finished => "finished",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Wait,
    Request,
    Done,
}

struct PaginationWaiter {
    request_id: RequestId,
    phase: Phase,
    /// #1200/#1233: the last backward pagination state seen for the awaited key,
    /// as a bounded token, so a shallow-phase timeout can distinguish a missing
    /// terminal state from a state that belonged to another request.
    last_transition: &'static str,
}

impl PaginationWaiter {
    fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            phase: Phase::AwaitingAcceptance,
            last_transition: "none",
        }
    }

    fn start_request(&mut self, request_id: RequestId) {
        self.request_id = request_id;
        self.phase = Phase::AwaitingAcceptance;
    }

    fn observe(&mut self, key: &TimelineKey, event: &CoreEvent) -> Result<Step, String> {
        if self.phase == Phase::Finished {
            return Ok(Step::Wait);
        }
        match event {
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if *request_id == self.request_id => {
                self.phase = Phase::Finished;
                Err(format!("pagination operation failed: {failure:?}"))
            }
            CoreEvent::Timeline(TimelineEvent::GapRepairReleased { key: event_key, .. })
                if event_key == key && self.phase == Phase::AwaitingGapRelease =>
            {
                self.phase = Phase::NeedsRequest;
                Ok(Step::Request)
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                request_id: Some(request_id),
                key: event_key,
                direction: PaginationDirection::Backward,
                state,
                ..
            }) if event_key == key => {
                // #1200/#1233: record the state even when it belongs to another
                // request, then keep ignoring it for settlement.
                self.last_transition = if *request_id == self.request_id {
                    pagination_state_token(state)
                } else {
                    "other_request"
                };
                if *request_id != self.request_id {
                    return Ok(Step::Wait);
                }
                match state {
                    PaginationState::Failed { kind } => {
                        self.phase = Phase::Finished;
                        Err(format!("pagination failed: {kind:?}"))
                    }
                    PaginationState::Paginating if self.phase == Phase::AwaitingAcceptance => {
                        self.phase = Phase::Paginating;
                        Ok(Step::Wait)
                    }
                    PaginationState::Idle if self.phase == Phase::Paginating => {
                        self.phase = Phase::NeedsRequest;
                        Ok(Step::Request)
                    }
                    PaginationState::Idle if self.phase == Phase::AwaitingAcceptance => {
                        // Admission was blocked by gap repair. Only its release can retry.
                        self.phase = Phase::AwaitingGapRelease;
                        Ok(Step::Wait)
                    }
                    PaginationState::EndReached => {
                        let accepted = self.phase == Phase::Paginating;
                        self.phase = Phase::Finished;
                        // This gate intentionally proves Core's correlated acceptance signal.
                        if accepted {
                            Ok(Step::Done)
                        } else {
                            Err("EndReached without prior Paginating".to_owned())
                        }
                    }
                    _ => Ok(Step::Wait),
                }
            }
            _ => Ok(Step::Wait),
        }
    }
}

fn pagination_state_token(state: &PaginationState) -> &'static str {
    match state {
        PaginationState::Idle => "idle",
        PaginationState::Paginating => "paginating",
        PaginationState::EndReached => "end_reached",
        PaginationState::Failed { .. } => "failed",
    }
}

pub(super) async fn wait_for_end_reached(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    label: &str,
    event_count: u16,
    deadline: Instant,
) -> Result<String, String> {
    let mut waiter = PaginationWaiter::new(request_id);
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{label}: timed out waiting for EndReached pagination state phase={} last_transition={}",
                    waiter.phase.token(),
                    waiter.last_transition
                )
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        match waiter
            .observe(key, &event)
            .map_err(|reason| format!("{label}: {reason}"))?
        {
            Step::Wait => {}
            Step::Done => return Ok("end_reached".to_owned()),
            Step::Request => {
                let request_id = conn.next_request_id();
                waiter.start_request(request_id);
                tokio::time::timeout_at(
                    deadline,
                    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
                        request_id,
                        key: key.clone(),
                        direction: PaginationDirection::Backward,
                        event_count,
                    })),
                )
                .await
                .map_err(|_| format!("{label}: pagination submission timed out"))?
                .map_err(|_| format!("{label}: pagination submission failed"))?;
            }
        }
    }
}

#[cfg(test)]
#[path = "pagination_waiter_tests.rs"]
mod tests;
