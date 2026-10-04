//! #1110: the rendered viewport identity and Core's read boundary must agree.
//!
//! A resurrected bodyless state event was rendered as a blank row, so the DOM
//! reported it as the last visible event while Core's readable boundary skipped
//! it. The observation was then rejected and no read target was produced. With
//! the authoritative visibility policy the row is never rendered, so the DOM
//! reports the last row the reader actually saw.

use koushi_protocol::event::TimelineBottomArrival;

use crate::event_projection::project_timeline_item_display_labels;
use koushi_state::AppState;

use super::super::item_projection::timeline_item_event_id;
use super::super::test_support::{room_key, timeline_item};
use super::viewed_boundary_target;

/// Probe 3: an ordinary message followed by a suppressed technical update must
/// still acquire a read target at the bottom.
#[test]
fn hidden_technical_tail_does_not_block_the_viewed_boundary() {
    let canonical = vec![
        timeline_item(
            "$message:example.invalid",
            Some("Synthetic message"),
            "@member:example.invalid",
            false,
        ),
        timeline_item(
            "$acl:example.invalid",
            None,
            "@moderator:example.invalid",
            true,
        ),
    ];
    // Consumer export, exactly as the runtime connection performs it.
    let mut displayed = canonical.clone();
    for item in &mut displayed {
        project_timeline_item_display_labels(item, &AppState::default());
    }
    assert!(
        displayed[1].is_hidden,
        "the suppressed technical event must not render as a row"
    );

    let key = room_key();
    let (_, target) = viewed_boundary_target(
        &key.kind,
        &canonical,
        &displayed,
        "$message:example.invalid",
        TimelineBottomArrival::User,
    )
    .expect("the visible message must acquire a read target");

    assert_eq!(
        timeline_item_event_id(target),
        Some("$message:example.invalid")
    );
}

/// The boundary still refuses a viewport identity it cannot confirm: an id that
/// names no rendered eligible row must not acknowledge anything.
#[test]
fn unknown_viewport_identity_still_produces_no_read_target() {
    let canonical = vec![timeline_item(
        "$message:example.invalid",
        Some("Synthetic message"),
        "@member:example.invalid",
        false,
    )];
    let mut displayed = canonical.clone();
    for item in &mut displayed {
        project_timeline_item_display_labels(item, &AppState::default());
    }

    let key = room_key();
    assert!(
        viewed_boundary_target(
            &key.kind,
            &canonical,
            &displayed,
            "$never-rendered:example.invalid",
            TimelineBottomArrival::User,
        )
        .is_none(),
        "an unconfirmed viewport identity must not acknowledge read state"
    );
}
