//! #1985: the state snapshot and its delta are the app's general diagnostic
//! dumps. They carry account, room and event identities plus message text, so
//! their `Debug` has to show shape and counts, never the values themselves.

use super::{StateDelta, StateDeltaChangedSlices, VersionedAppStateSnapshot};
use koushi_state::{
    AppError, AppState, SessionAuthenticationMethod, SessionInfo, SessionState,
    ThreadRootAttention, ThreadsListItem, ThreadsListState,
};
use std::collections::BTreeMap;

const PRIVATE_HOMESERVER: &str = "https://private-homeserver.invalid";
const PRIVATE_USER: &str = "@private-user:private-homeserver.invalid";
const PRIVATE_DEVICE: &str = "PRIVATE-DEVICE";
const PRIVATE_ROOM: &str = "!private-room:private-homeserver.invalid";
const PRIVATE_ROOT: &str = "$private-root:private-homeserver.invalid";
const PRIVATE_BODY: &str = "private body text";

fn private_values() -> [&'static str; 6] {
    [
        PRIVATE_HOMESERVER,
        PRIVATE_USER,
        PRIVATE_DEVICE,
        PRIVATE_ROOM,
        PRIVATE_ROOT,
        PRIVATE_BODY,
    ]
}

fn thread_item() -> ThreadsListItem {
    ThreadsListItem {
        room_id: PRIVATE_ROOM.to_owned(),
        root_event_id: PRIVATE_ROOT.to_owned(),
        root_sender: PRIVATE_USER.to_owned(),
        root_sender_label: Some("Private Sender".to_owned()),
        root_body_preview: Some(PRIVATE_BODY.to_owned()),
        root_timestamp_ms: Some(1_800_000_000_000),
        latest_event_id: Some("$private-latest:private-homeserver.invalid".to_owned()),
        latest_sender: Some(PRIVATE_USER.to_owned()),
        latest_sender_label: Some("Private Sender".to_owned()),
        latest_body_preview: Some(PRIVATE_BODY.to_owned()),
        latest_timestamp_ms: Some(1_800_000_000_000),
        reply_count: 2,
        unread_count: 1,
    }
}

fn ready_session() -> SessionState {
    SessionState::Ready(SessionInfo {
        homeserver: PRIVATE_HOMESERVER.to_owned(),
        user_id: PRIVATE_USER.to_owned(),
        device_id: PRIVATE_DEVICE.to_owned(),
        authentication_method: SessionAuthenticationMethod::Password,
    })
}

/// One populated state, so the assertions below cover the paths the issue names:
/// the session info, id-keyed maps, the threads list rows, and error text.
fn populated_state() -> AppState {
    let mut state = AppState {
        session: ready_session(),
        ..AppState::default()
    };
    state.navigation.active_room_id = Some(PRIVATE_ROOM.to_owned());
    state.navigation.space_order = vec![PRIVATE_ROOM.to_owned()];
    state.thread_unread.insert(
        (PRIVATE_ROOM.to_owned(), PRIVATE_ROOT.to_owned()),
        ThreadRootAttention {
            unread: 1,
            highlight: 0,
        },
    );
    state.threads_list = ThreadsListState::Open {
        room_id: PRIVATE_ROOM.to_owned(),
        request_id: 1,
        items: vec![thread_item()],
        is_paginating: false,
        end_reached: true,
    };
    state.errors = vec![AppError::new("private_code", PRIVATE_BODY, true)];
    state
}

/// Everything the delta can carry that is not a plain scalar, keyed by the ids
/// and text the dump must not print.
fn populated_delta() -> StateDeltaChangedSlices {
    StateDeltaChangedSlices {
        session: Some(ready_session()),
        rooms_by_id: Some(BTreeMap::from([(PRIVATE_ROOM.to_owned(), None)])),
        profile_users_by_id: Some(BTreeMap::from([(PRIVATE_USER.to_owned(), None)])),
        profile_local_aliases_by_id: Some(BTreeMap::from([(
            PRIVATE_USER.to_owned(),
            Some(PRIVATE_BODY.to_owned()),
        )])),
        live_signals_presence_by_user: Some(BTreeMap::from([(PRIVATE_USER.to_owned(), None)])),
        ..StateDeltaChangedSlices::default()
    }
}

fn assert_redacted(debug: &str) {
    for raw in private_values() {
        assert!(!debug.contains(raw), "raw {raw} leaked: {debug}");
    }
    for forbidden in [":private-homeserver.invalid", "Private Sender", "mxc://"] {
        assert!(!debug.contains(forbidden), "leaked {forbidden}: {debug}");
    }
}

#[test]
fn versioned_snapshot_debug_summarises_the_state_instead_of_printing_it() {
    let snapshot = VersionedAppStateSnapshot {
        generation: 7,
        state: populated_state(),
    };
    let debug = format!("{snapshot:?}");
    assert_redacted(&debug);
    assert!(debug.contains("generation: 7"), "{debug}");
}

#[test]
fn delta_debug_names_the_changed_slices_without_their_contents() {
    let delta = StateDelta {
        generation: 3,
        changed: Box::new(populated_delta()),
    };
    let debug = format!("{delta:?}");
    assert_redacted(&debug);
    // The names are the whole diagnostic value of this impl: a dump that
    // printed nothing (or everything) would still satisfy the redaction check.
    for changed in [
        "\"session\"",
        "\"rooms_by_id\"",
        "\"profile_users_by_id\"",
        "\"live_signals_presence_by_user\"",
    ] {
        assert!(debug.contains(changed), "missing {changed}: {debug}");
    }
    for unchanged in ["\"spaces\"", "\"timeline\"", "\"errors\""] {
        assert!(!debug.contains(unchanged), "named {unchanged}: {debug}");
    }
}
