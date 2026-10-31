//! Issue #1518: starting a direct message is a Rust-owned pending operation, so
//! the profile's Send message button can show it in flight instead of relying
//! on a renderer-wide busy flag that silently drops presses.

use koushi_state::{
    AppAction, AppState, BasicOperationRequest, BasicOperationState, SessionInfo, SessionState,
    reduce,
};

const TARGET: &str = "@member-1:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:example.invalid".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn request(state: &mut AppState, request_id: u64) {
    reduce(
        state,
        AppAction::BasicOperationRequested {
            request_id,
            request: BasicOperationRequest::StartDirectMessage {
                user_id: TARGET.to_owned(),
            },
        },
    );
}

fn starting(request_id: u64) -> BasicOperationState {
    BasicOperationState::StartingDirectMessage {
        request_id,
        user_id: TARGET.to_owned(),
    }
}

#[test]
fn a_direct_message_start_is_pending_for_its_target_until_it_settles() {
    let mut state = ready_state();
    request(&mut state, 4);
    assert_eq!(state.basic_operation, starting(4));
    assert_eq!(state.basic_operation.request_id(), Some(4));

    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 4 },
    );
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
}

#[test]
fn a_failed_direct_message_start_returns_to_idle() {
    let mut state = ready_state();
    request(&mut state, 5);

    reduce(
        &mut state,
        AppAction::BasicOperationFailed {
            request_id: 5,
            message: "Direct message could not be started".to_owned(),
        },
    );
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "basic_operation_failed")
    );
}

#[test]
fn stale_and_duplicate_completions_do_not_settle_a_direct_message_start() {
    let mut state = ready_state();
    request(&mut state, 6);

    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 99 },
    );
    assert_eq!(state.basic_operation, starting(6));

    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 6 },
    );
    request(&mut state, 7);
    // A duplicate completion of the settled request leaves the new one alone.
    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 6 },
    );
    assert_eq!(state.basic_operation, starting(7));
}

#[test]
fn a_second_start_while_one_is_in_flight_keeps_the_first() {
    let mut state = ready_state();
    request(&mut state, 8);
    request(&mut state, 9);
    assert_eq!(state.basic_operation, starting(8));
}

#[test]
fn a_direct_message_start_needs_a_ready_session() {
    let mut state = AppState::default();
    request(&mut state, 10);
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
}

#[test]
fn the_pending_target_is_redacted_from_debug_output() {
    let rendered = format!("{:?}", starting(11));
    assert!(rendered.contains("StartingDirectMessage"));
    assert!(!rendered.contains("member-1"));
}
