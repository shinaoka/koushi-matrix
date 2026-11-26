//! #2110: an agent account raises no owner-facing interruption.
//!
//! Before the role, an agent account behaved like a person's account: its
//! unread messages raised a notification candidate and counted into the badge.
//! The role is one fact on the account's settings; the attention projection
//! consults it, and the unread state itself stays in the data.

use super::*;
use crate::state::{AgentAccountPatch, AgentAccountRole, RoomSummary, SettingsPatch};

const ACTIVE_ROOM: &str = "!active:example.invalid";
const ROOM: &str = "!agent-room:example.invalid";

fn unread_room(unread: u64, timestamp_ms: u64) -> RoomSummary {
    let mut room = test_room(ROOM, None);
    room.unread_count = unread;
    room.notification_count = unread;
    room.recency_stamp = Some(timestamp_ms);
    room.latest_event = Some(latest_event("$agent-event:example.invalid", timestamp_ms));
    room
}

fn own_user_id(state: &AppState) -> String {
    match &state.session {
        crate::state::SessionState::Ready(info) => info.user_id.clone(),
        _ => unreachable!("the fixture is a ready account"),
    }
}

fn update(state: &mut AppState, rooms: Vec<RoomSummary>) {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    );
}

/// A ready account that sees one read room, then one unread message arrive live.
fn state_after_a_live_message(role: Option<AgentAccountRole>) -> AppState {
    let mut state = ready_state();
    if let Some(role) = role {
        let user_id = own_user_id(&state);
        state
            .settings
            .values
            .agent_accounts
            .apply(&AgentAccountPatch {
                user_id,
                role: Some(role),
            });
    }
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    let active = test_room(ACTIVE_ROOM, None);
    update(&mut state, vec![active.clone(), unread_room(0, 40)]);
    update(&mut state, vec![active, unread_room(2, 43)]);
    state
}

#[test]
fn an_ordinary_account_notifies_and_counts_into_the_badge() {
    let state = state_after_a_live_message(None);
    assert!(state.native_attention.summary.candidate.is_some());
    assert!(state.native_attention.notification.is_some());
    assert_eq!(state.native_attention.summary.badge_count, 2);
}

#[test]
fn an_agent_account_notifies_nothing_and_counts_nothing_but_keeps_its_unread_state() {
    let state = state_after_a_live_message(Some(AgentAccountRole::default()));
    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(state.native_attention.notification, None);
    assert_eq!(state.native_attention.summary.alert_id, None);
    assert_eq!(state.native_attention.summary.badge_count, 0);
    // The unread state is still there to read.
    assert_eq!(state.native_attention.summary.unread_count, 2);
    let room = state
        .rooms
        .iter()
        .find(|room| room.room_id == ROOM)
        .unwrap();
    assert_eq!(room.unread_count, 2);
}

#[test]
fn turning_the_role_on_silences_the_account_and_turning_it_off_restores_the_badge() {
    let mut state = state_after_a_live_message(None);
    assert_eq!(state.native_attention.summary.badge_count, 2);
    let user_id = own_user_id(&state);
    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                scope: Some(crate::state::SettingsPatchScope::App),
                agent_account: Some(AgentAccountPatch {
                    user_id: user_id.clone(),
                    role: Some(AgentAccountRole::default()),
                }),
                ..SettingsPatch::default()
            },
        },
    );
    assert!(state.is_agent_account());
    assert_eq!(state.native_attention.summary.badge_count, 0);
    assert_eq!(state.native_attention.summary.candidate, None);
    assert_eq!(state.native_attention.summary.unread_count, 2);

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                scope: Some(crate::state::SettingsPatchScope::App),
                agent_account: Some(AgentAccountPatch {
                    user_id: user_id.clone(),
                    role: None,
                }),
                ..SettingsPatch::default()
            },
        },
    );
    assert_eq!(state.native_attention.summary.badge_count, 2);
}
