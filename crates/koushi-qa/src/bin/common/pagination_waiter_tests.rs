use super::*;
use koushi_protocol::{
    failure::{CoreFailure, TimelineFailureKind},
    ids::{AccountKey, RuntimeConnectionId},
};

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence,
    }
}
fn key() -> TimelineKey {
    TimelineKey::room(AccountKey("fixture".into()), "!room:example.invalid")
}
fn state(id: Option<RequestId>, state: PaginationState) -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
        request_id: id,
        key: key(),
        direction: PaginationDirection::Backward,
        state,
        prepend_expected: None,
    })
}
fn release() -> CoreEvent {
    CoreEvent::Timeline(TimelineEvent::GapRepairReleased {
        key: key(),
        actor_generation: 1,
        generation: 1,
    })
}

#[test]
fn unrelated_and_stale_states_cannot_settle_or_accept_current_request() {
    let mut waiter = PaginationWaiter::new(request(2));
    for id in [
        None,
        Some(request(1)),
        Some(RequestId {
            connection_id: RuntimeConnectionId(2),
            sequence: 2,
        }),
    ] {
        assert_eq!(
            waiter.observe(&key(), &state(id, PaginationState::Paginating)),
            Ok(Step::Wait)
        );
        assert_eq!(
            waiter.observe(&key(), &state(id, PaginationState::EndReached)),
            Ok(Step::Wait)
        );
    }
    let other_key = TimelineKey::room(AccountKey("fixture".into()), "!other:example.invalid");
    assert_eq!(
        waiter.observe(
            &other_key,
            &state(Some(request(2)), PaginationState::Paginating)
        ),
        Ok(Step::Wait)
    );
    assert!(
        waiter
            .observe(
                &key(),
                &state(Some(request(2)), PaginationState::EndReached)
            )
            .is_err()
    );
}

#[test]
fn blocked_admission_retries_once_only_after_matching_gap_release() {
    let mut waiter = PaginationWaiter::new(request(1));
    assert_eq!(waiter.observe(&key(), &release()), Ok(Step::Wait));
    let idle = state(Some(request(1)), PaginationState::Idle);
    assert_eq!(waiter.observe(&key(), &idle), Ok(Step::Wait));
    assert_eq!(waiter.observe(&key(), &idle), Ok(Step::Wait));
    let other_key = TimelineKey::room(AccountKey("fixture".into()), "!other:example.invalid");
    assert_eq!(waiter.observe(&other_key, &release()), Ok(Step::Wait));
    assert_eq!(waiter.observe(&key(), &release()), Ok(Step::Request));
    assert_eq!(waiter.observe(&key(), &release()), Ok(Step::Wait));
    waiter.start_request(request(2));
    assert_eq!(waiter.observe(&key(), &idle), Ok(Step::Wait));
    assert_eq!(waiter.observe(&key(), &release()), Ok(Step::Wait));
    assert_eq!(
        waiter.observe(
            &key(),
            &state(Some(request(2)), PaginationState::Paginating)
        ),
        Ok(Step::Wait)
    );
    assert_eq!(
        waiter.observe(
            &key(),
            &state(Some(request(2)), PaginationState::EndReached)
        ),
        Ok(Step::Done)
    );
}

#[test]
fn accepted_pages_require_new_request_and_have_one_terminal() {
    let mut waiter = PaginationWaiter::new(request(1));
    assert_eq!(
        waiter.observe(
            &key(),
            &state(Some(request(1)), PaginationState::Paginating)
        ),
        Ok(Step::Wait)
    );
    let idle = state(Some(request(1)), PaginationState::Idle);
    assert_eq!(waiter.observe(&key(), &idle), Ok(Step::Request));
    assert_eq!(waiter.observe(&key(), &idle), Ok(Step::Wait));
    waiter.start_request(request(2));
    assert_eq!(
        waiter.observe(
            &key(),
            &state(Some(request(1)), PaginationState::EndReached)
        ),
        Ok(Step::Wait)
    );
    assert_eq!(
        waiter.observe(
            &key(),
            &state(Some(request(2)), PaginationState::Paginating)
        ),
        Ok(Step::Wait)
    );
    let end = state(Some(request(2)), PaginationState::EndReached);
    assert_eq!(waiter.observe(&key(), &end), Ok(Step::Done));
    assert_eq!(waiter.observe(&key(), &end), Ok(Step::Wait));
}

#[test]
fn matching_failure_is_terminal_even_while_waiting_for_gap_release() {
    for blocked in [false, true] {
        let mut waiter = PaginationWaiter::new(request(1));
        if blocked {
            waiter
                .observe(&key(), &state(Some(request(1)), PaginationState::Idle))
                .unwrap();
        }
        let failure = CoreEvent::OperationFailed {
            request_id: request(1),
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Network,
            },
        };
        assert!(
            waiter
                .observe(&key(), &failure)
                .unwrap_err()
                .contains("Network")
        );
        assert_eq!(waiter.observe(&key(), &release()), Ok(Step::Wait));
        assert_eq!(waiter.observe(&key(), &failure), Ok(Step::Wait));
    }
    let mut waiter = PaginationWaiter::new(request(1));
    assert!(
        waiter
            .observe(
                &key(),
                &state(
                    Some(request(1)),
                    PaginationState::Failed {
                        kind: TimelineFailureKind::Forbidden
                    }
                )
            )
            .unwrap_err()
            .contains("Forbidden")
    );
}
