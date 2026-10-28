//! Shared fixture for the bridge tests: a disposable runtime whose session
//! state is driven through the real core reducer (#1580). It has no SDK
//! session, so nothing reaches a homeserver; it carries exactly the session
//! state `BridgeAccounts` reads.
#![allow(dead_code)] // each test binary uses its own subset

use std::time::Duration;

use koushi_core::runtime::CoreRuntime;
use koushi_state::{
    AppAction, CurrentDeviceTrustState, SessionAuthenticationMethod, SessionInfo, SessionState,
};

pub fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@member1:example.invalid".into(),
        device_id: "SYNTHETIC".into(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    }
}

/// Signs the disposable runtime in.
pub async fn sign_in(runtime: &CoreRuntime) {
    apply_actions(
        runtime,
        vec![
            AppAction::RestoreSessionRequested,
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
        ],
    )
    .await;
    wait_for_session(runtime, |session| matches!(session, SessionState::Ready(_))).await;
}

/// Injects reducer actions, as an actor side effect would.
pub async fn apply_actions(runtime: &CoreRuntime, actions: Vec<AppAction>) {
    runtime.inject_actions(actions).await;
}

/// Waits until the runtime's session state satisfies `done`.
pub async fn wait_for_session(runtime: &CoreRuntime, done: impl Fn(&SessionState) -> bool) {
    let connection = runtime.attach();
    for _ in 0..500 {
        if done(&connection.snapshot().session) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the session state did not settle in time");
}

/// A joined room summary for the room list.
pub fn room(room_id: &str) -> koushi_state::RoomSummary {
    koushi_state::RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: koushi_state::RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: true,
        joined_members: 2,
    }
}
