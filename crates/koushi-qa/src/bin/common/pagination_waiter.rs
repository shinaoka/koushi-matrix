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

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Wait,
    Request,
    Done,
}

struct PaginationWaiter {
    request_id: RequestId,
    phase: Phase,
}

impl PaginationWaiter {
    fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            phase: Phase::AwaitingAcceptance,
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
            }) if event_key == key && *request_id == self.request_id => {
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
            .map_err(|_| format!("{label}: timed out waiting for EndReached pagination state"))?
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
