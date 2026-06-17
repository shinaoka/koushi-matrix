//! Shared test support for `matrix-desktop-core` integration tests.
//!
//! This module contains synthetic fixtures and helpers used by multiple
//! per-feature test files. It must not contain any `#[test]` functions.

#![allow(dead_code)]

use std::time::Duration;

use matrix_desktop_core::executor;
use matrix_desktop_core::ids::{RequestId, RuntimeConnectionId};
use matrix_desktop_core::runtime::{CoreConnection, CoreRuntime};
use matrix_desktop_state::{
    ActivityRow, AppearanceSettings, RoomSummary, RoomTags, SessionInfo, SettingsPatch,
    ThemePreference,
};

pub const PASSWORD: &str = "p4ssw0rd-very-secret";
pub const RECOVERY: &str = "EsT1 RcVy KeyM ater";
pub const BODY: &str = "private message body 機密本文";
pub const QUERY: &str = "secret search terms";

pub fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "DEVICE1".to_owned(),
    }
}

pub fn fake_request_id() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(999),
        sequence: 1,
    }
}

pub fn dark_theme_settings_patch() -> SettingsPatch {
    SettingsPatch {
        appearance: Some(AppearanceSettings {
            theme: ThemePreference::Dark,
        }),
        ..SettingsPatch::default()
    }
}

pub fn future_epoch_ms(offset: Duration) -> u64 {
    std::time::SystemTime::now()
        .checked_add(offset)
        .expect("future timestamp")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_millis() as u64
}

pub fn room_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: "QA Room".to_owned(),
        display_label: "QA Room".to_owned(),
        original_display_label: "QA Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        parent_space_ids: vec![],
    }
}

pub fn unread_room_summary(room_id: &str, unread_count: u64) -> RoomSummary {
    RoomSummary {
        unread_count,
        ..room_summary(room_id)
    }
}

pub fn activity_row(room_id: &str, event_id: &str, timestamp_ms: u64) -> ActivityRow {
    ActivityRow {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        room_label: String::new(),
        sender_label: Some("Private sender".to_owned()),
        preview: Some("Private preview".to_owned()),
        timestamp_ms,
        unread: false,
        highlight: false,
    }
}

pub async fn wait_for_state<F>(
    connection: &mut CoreConnection,
    predicate: F,
) -> matrix_desktop_state::AppState
where
    F: Fn(&matrix_desktop_state::AppState) -> bool,
{
    for _ in 0..200 {
        let snapshot = connection.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }
    panic!("state predicate was not satisfied");
}

/// Convenience: start a runtime and inject a ready session plus a single
/// subscribed room. Returns the runtime, connection, and the snapshot that
/// satisfied the ready/subscribed predicate.
pub async fn ready_room_conn(
    room_id: &str,
) -> (CoreRuntime, CoreConnection, matrix_desktop_state::AppState) {
    use matrix_desktop_state::{AppAction, SessionState};

    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary(room_id)],
            },
            AppAction::SelectRoom {
                room_id: room_id.to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: room_id.to_owned(),
            },
        ])
        .await;
    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some(room_id)
    })
    .await;
    (runtime, conn, snapshot)
}
