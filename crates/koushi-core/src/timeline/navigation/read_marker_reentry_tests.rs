use super::derive_timeline_navigation_snapshot_with_read_state;
use crate::timeline::test_support::{thread_key, timeline_item};
use koushi_protocol::event::{TimelineReadStateSync, TimelineViewportObservation};

#[test]
fn hidden_confirmed_thread_receipt_restores_divider_without_local_observation() {
    let mut reply = timeline_item("$reply:test", Some("reply"), "@other:test", false);
    let mut hidden = timeline_item("$edit:test", Some("edit"), "@other:test", false);
    reply.thread_root = Some("$root:test".into());
    hidden.thread_root = Some("$root:test".into());
    hidden.is_hidden = true;
    for local in [None, Some("$missing:test")] {
        let snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &thread_key().kind,
            &[reply.clone(), hidden.clone()],
            None,
            Some("$edit:test"),
            local,
            TimelineReadStateSync::Synced,
            &TimelineViewportObservation::default(),
            Some("@me:test"),
        );
        assert_eq!(snapshot.read_marker_event_id.as_deref(), Some("$edit:test"));
        assert_eq!(
            snapshot.read_marker_display_event_id.as_deref(),
            Some("$reply:test")
        );
        assert_eq!(snapshot.unread_event_count, 0);
        assert_eq!(snapshot.local_viewed_event_id.as_deref(), local);
    }
}

#[test]
fn hidden_receipt_does_not_move_the_boundary_past_unread_replies() {
    let hidden = timeline_item("$hidden:test", None, "@other:test", true);
    let later = timeline_item("$later:test", Some("unread"), "@other:test", false);
    let snapshot = derive_timeline_navigation_snapshot_with_read_state(
        &thread_key().kind,
        &[hidden, later],
        None,
        Some("$hidden:test"),
        None,
        TimelineReadStateSync::Synced,
        &TimelineViewportObservation::default(),
        Some("@me:test"),
    );
    assert_eq!(
        snapshot.first_unread_event_id.as_deref(),
        Some("$later:test")
    );
    assert_eq!(snapshot.unread_event_count, 1);
    assert_eq!(snapshot.read_marker_display_event_id, None);
    assert_eq!(
        snapshot.server_confirmed_read_event_id.as_deref(),
        Some("$hidden:test")
    );
}

#[test]
fn reentry_without_a_loaded_receipt_cannot_invent_a_divider() {
    let reply = timeline_item("$reply:test", Some("reply"), "@other:test", false);
    for confirmed in [None, Some("$missing:test")] {
        let snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &thread_key().kind,
            std::slice::from_ref(&reply),
            None,
            confirmed,
            None,
            TimelineReadStateSync::Synced,
            &TimelineViewportObservation::default(),
            Some("@me:test"),
        );
        assert_eq!(snapshot.read_marker_display_event_id, None);
        assert_eq!(
            snapshot.server_confirmed_read_event_id.as_deref(),
            confirmed
        );
    }
}

#[test]
fn every_loaded_boundary_projects_before_unread_processing() {
    let visible = timeline_item("$reply:test", Some("reply"), "@other:test", false);
    let mut hidden = timeline_item("$edit:test", Some("edit"), "@other:test", false);
    hidden.is_hidden = true;
    let unread = timeline_item("$unread:test", Some("unread"), "@other:test", false);
    for (confirmed, local) in [
        (Some("$edit:test"), None),
        (Some("$edit:test"), Some("$edit:test")),
        (Some("$reply:test"), Some("$edit:test")),
        (Some("$missing:test"), Some("$edit:test")),
    ] {
        let snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &thread_key().kind,
            &[visible.clone(), hidden.clone(), unread.clone()],
            None,
            confirmed,
            local,
            TimelineReadStateSync::Synced,
            &TimelineViewportObservation::default(),
            Some("@me:test"),
        );
        assert_eq!(
            snapshot.read_marker_display_event_id.as_deref(),
            Some("$reply:test")
        );
        assert_eq!(
            snapshot.server_confirmed_read_event_id.as_deref(),
            confirmed
        );
        if confirmed != Some("$missing:test") {
            assert_eq!(snapshot.unread_event_count, 1);
            assert_eq!(
                snapshot.first_unread_event_id.as_deref(),
                Some("$unread:test")
            );
        }
    }
}

#[test]
fn room_boundary_on_thread_reply_projects_to_previous_room_row() {
    use crate::timeline::test_support::room_key;
    let room_row = timeline_item("$room:test", Some("room message"), "@other:test", false);
    let mut reply = timeline_item("$reply:test", Some("thread reply"), "@other:test", false);
    reply.thread_root = Some("$root:test".into());
    let snapshot = derive_timeline_navigation_snapshot_with_read_state(
        &room_key().kind,
        &[room_row, reply],
        None,
        Some("$reply:test"),
        Some("$reply:test"),
        TimelineReadStateSync::Synced,
        &TimelineViewportObservation::default(),
        Some("@me:test"),
    );
    assert_eq!(
        snapshot.read_marker_display_event_id.as_deref(),
        Some("$room:test")
    );
    assert_eq!(
        snapshot.server_confirmed_read_event_id.as_deref(),
        Some("$reply:test")
    );
    assert_eq!(snapshot.unread_event_count, 0);
}
