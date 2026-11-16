//! #1061: the Activity states the renderer regression feeds to the WebView.
//!
//! Each state is taken from a real AppActor run (Activity projection, reducer,
//! and resolution lifecycle), serialized as the snapshot/delta `activity`
//! slice, and compared with the checked-in artifact the Playwright spec
//! `activity-resolution-states.spec.ts` publishes. The renderer is therefore
//! tested against Rust-owned shapes, not hand-written ones. All identifiers
//! are synthetic.
//!
//! Regenerate with
//! `UPDATE_ACTIVITY_RENDERER_GOLDEN=1 cargo test -p koushi-core --lib activity_renderer_states`.

use super::*;
use koushi_state::{ActivityRow, ActivityRowKind};

const DM: &str = "!activity-renderer-dm:example.invalid";
const RESOLVED_EVENT: &str = "$activity-renderer-resolved:example.invalid";
const OLD_EVENT: &str = "$activity-renderer-old:example.invalid";
const WAIT: Duration = Duration::from_secs(1);

fn open_activity_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        activity: ActivityState::Open {
            active_tab: koushi_state::ActivityTab::Unread,
            recent: koushi_state::ActivityStream::default(),
            unread: koushi_state::ActivityStream::default(),
            mark_read: Default::default(),
        },
        ..AppState::default()
    };
    state.live_signals.rooms.insert(
        DM.to_owned(),
        koushi_state::RoomLiveSignals {
            fully_read_event_id: Some("$before:example.invalid".to_owned()),
            ..Default::default()
        },
    );
    state
}

/// A notified unread DM whose newest activity is at `activity_timestamp_ms`.
fn unread_dm(activity_timestamp_ms: u64) -> RoomSummary {
    let mut room = live_unresolved_activity_room(DM, activity_timestamp_ms);
    room.display_name = "Synthetic DM".to_owned();
    room.display_label = "Synthetic DM".to_owned();
    room.original_display_label = "Synthetic DM".to_owned();
    room
}

fn dm_row(event_id: &str, preview: &str, timestamp_ms: u64) -> ActivityRow {
    ActivityRow::event(
        DM.to_owned(),
        event_id.to_owned(),
        Some("@peer:example.invalid".to_owned()),
        "Synthetic DM".to_owned(),
        Some("Synthetic peer".to_owned()),
        Some(preview.to_owned()),
        timestamp_ms,
        true,
        false,
    )
}

fn room_list(rooms: Vec<RoomSummary>) -> AppAction {
    AppAction::RoomListUpdated {
        spaces: Vec::new(),
        rooms,
    }
}

/// The published Activity once its unread resolution matches `predicate`.
async fn published_activity(
    snapshot_rx: &mut watch::Receiver<VersionedAppStateSnapshot>,
    predicate: impl Fn(&koushi_state::ActivityResolutionState) -> bool,
) -> ActivityState {
    tokio::time::timeout(WAIT, async {
        loop {
            let activity = snapshot_rx.borrow_and_update().state.activity.clone();
            if let ActivityState::Open { unread, .. } = &activity
                && predicate(&unread.resolution)
            {
                return activity;
            }
            snapshot_rx.changed().await.expect("activity snapshot");
        }
    })
    .await
    .expect("Activity reaches the expected resolution state")
}

fn resolving_generation(activity: &ActivityState) -> u64 {
    match activity {
        ActivityState::Open { unread, .. } => match unread.resolution {
            koushi_state::ActivityResolutionState::Resolving { generation, .. } => generation,
            other => panic!("unexpected resolution {other:?}"),
        },
        other => panic!("unexpected Activity {other:?}"),
    }
}

/// A new unread DM arrives while Activity is open: resolving, then resolved.
async fn new_unread_dm_states() -> (ActivityState, ActivityState) {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (actor, _command_tx, action_tx, _account_rx, _event_rx, mut snapshot_rx, ..) =
        app_actor_event_navigation_fixture(data_dir.path(), open_activity_state());
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![room_list(vec![unread_dm(200)])])
        .await
        .expect("live room-list update");
    let resolving = published_activity(&mut snapshot_rx, |resolution| {
        matches!(
            resolution,
            koushi_state::ActivityResolutionState::Resolving { .. }
        )
    })
    .await;
    let generation = resolving_generation(&resolving);

    action_tx
        .send(vec![
            AppAction::ActivityResolutionRowsObserved {
                generation,
                rows: vec![dm_row(RESOLVED_EVENT, "Resolved message", 200)],
            },
            AppAction::ActivityResolutionSucceeded { generation },
        ])
        .await
        .expect("resolution settles");
    let resolved = published_activity(&mut snapshot_rx, |resolution| {
        *resolution == koushi_state::ActivityResolutionState::Idle
    })
    .await;

    actor_task.abort();
    let _ = actor_task.await;
    (resolving, resolved)
}

/// An old cached row lacks read-boundary proof while newer activity in the
/// same room resolves, then fails: the stream never looks complete.
async fn stale_row_states() -> (ActivityState, ActivityState) {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (actor, _command_tx, action_tx, _account_rx, _event_rx, mut snapshot_rx, ..) =
        app_actor_event_navigation_fixture(data_dir.path(), open_activity_state());
    let actor_task = tokio::spawn(actor.run());

    action_tx
        .send(vec![
            AppAction::ActivityRowsObserved {
                rows: vec![dm_row(OLD_EVENT, "Older message", 100)],
            },
            room_list(vec![unread_dm(200)]),
        ])
        .await
        .expect("old row and newer activity");
    let resolving = published_activity(&mut snapshot_rx, |resolution| {
        matches!(
            resolution,
            koushi_state::ActivityResolutionState::Resolving { .. }
        )
    })
    .await;
    let generation = resolving_generation(&resolving);

    action_tx
        .send(vec![AppAction::ActivityResolutionFailed {
            generation,
            unresolved_room_count: 1,
            kind: OperationFailureKind::Network,
        }])
        .await
        .expect("resolution fails");
    let failed = published_activity(&mut snapshot_rx, |resolution| {
        matches!(
            resolution,
            koushi_state::ActivityResolutionState::Failed { .. }
        )
    })
    .await;

    actor_task.abort();
    let _ = actor_task.await;
    (resolving, failed)
}

fn unread_rows(activity: &ActivityState) -> &[ActivityRow] {
    match activity {
        ActivityState::Open { unread, .. } => &unread.rows,
        other => panic!("unexpected Activity {other:?}"),
    }
}

#[tokio::test]
async fn activity_renderer_states_match_the_checked_in_artifact() {
    let (new_dm_resolving, new_dm_resolved) = new_unread_dm_states().await;
    let (stale_resolving, stale_failed) = stale_row_states().await;

    // The shapes the renderer regression depends on.
    assert!(
        unread_rows(&new_dm_resolving)
            .iter()
            .all(|row| row.kind == ActivityRowKind::RoomUnread)
    );
    assert!(
        unread_rows(&new_dm_resolved)
            .iter()
            .all(|row| row.kind == ActivityRowKind::RoomUnread)
    );
    for activity in [&stale_resolving, &stale_failed] {
        let rows = unread_rows(activity);
        assert!(
            rows.iter()
                .all(|row| row.event_id.as_deref() != Some(OLD_EVENT))
        );
        assert!(
            rows.iter()
                .any(|row| row.kind == ActivityRowKind::RoomUnread)
        );
    }

    let actual = serde_json::json!({
        "newUnreadDmResolving": new_dm_resolving,
        "newUnreadDmResolved": new_dm_resolved,
        "staleRowWithNewerActivityResolving": stale_resolving,
        "staleRowWithNewerActivityFailed": stale_failed,
    });
    // Runtime lookup: `env!` would key the test build on the checkout path.
    let artifact_path = std::path::Path::new(
        &std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo test sets CARGO_MANIFEST_DIR"),
    )
    .join("../../apps/desktop/e2e/fixtures/activity-resolution-states.generated.json");
    if std::env::var("UPDATE_ACTIVITY_RENDERER_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(
            &artifact_path,
            serde_json::to_string_pretty(&actual).expect("format Activity states") + "\n",
        )
        .expect("write Activity renderer artifact");
        return;
    }
    let checked_in: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&artifact_path).expect("read Activity renderer artifact"),
    )
    .expect("checked-in Activity renderer artifact must be valid JSON");
    assert_eq!(actual, checked_in);
}
