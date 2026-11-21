//! #1846: a resolved send state is derived at projection time and never stored
//! in a row, so no publish path can roll it back.

use std::collections::{HashMap, HashSet};

use koushi_protocol::event::{TimelineItemId, TimelineSendFailureReason};

use super::super::item_projection::{
    SendStateAuthority, carry_resolved_send_failures, retry_admission,
};
use super::super::test_support::{room_key, send_row};
use super::*;

fn recoverable() -> TimelineSendState {
    TimelineSendState::NotSent {
        reason: TimelineSendFailureReason::Recoverable,
    }
}

/// The actor's table after `RoomSendQueueUpdate::SendError { is_recoverable: true }`
/// for the head of the queue, with the successor still unattempted.
fn stopped_room() -> HashMap<String, TimelineSendState> {
    HashMap::from([
        ("t1".to_owned(), recoverable()),
        ("t2".to_owned(), TimelineSendState::Sending),
    ])
}

fn canonical_queue() -> Vec<TimelineItem> {
    vec![
        // The SDK reports the failure on the head, `NotSentYet` on the successor.
        send_row("t1", Some(recoverable())),
        send_row("t2", Some(TimelineSendState::Sending)),
    ]
}

fn room_context(statuses: &HashMap<String, TimelineSendState>) -> DisplayProjectionContext {
    DisplayProjectionContext::for_timeline(
        &room_key().kind,
        &TimelineViewportObservation {
            at_bottom: true,
            ..Default::default()
        },
    )
    .with_send_states(statuses)
}

fn rendered_send_state(items: &[TimelineItem], transaction_id: &str) -> Option<TimelineSendState> {
    items
        .iter()
        .find(|item| {
            matches!(
                &item.id,
                TimelineItemId::Transaction { transaction_id: id } if id == transaction_id
            )
        })
        .and_then(|item| item.send_state.clone())
}

/// Path 1: return to live. Overflow recovery rebuilds the window from a snapshot
/// whose head the SDK reports as failed; the successor stays `not_sent`.
#[test]
fn return_to_live_keeps_a_resolved_not_sent_and_blocks_its_successor() {
    let statuses = stopped_room();
    let canonical = canonical_queue();
    let mut projection =
        DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
    let context = room_context(&statuses);

    projection.return_to_live_window(&canonical, &context);

    assert_eq!(
        rendered_send_state(projection.display_items(), "t1"),
        Some(recoverable())
    );
    assert_eq!(
        rendered_send_state(projection.display_items(), "t2"),
        Some(recoverable()),
        "a request queued behind a recoverable failure is not sending"
    );
}

/// Path 2: eviction. The window holds only the successor, so nothing it owns
/// remembers the failure; the room's table still does.
#[test]
fn an_evicted_failure_row_still_blocks_the_retained_successor() {
    let statuses = stopped_room();
    let canonical = canonical_queue();
    let context = room_context(&statuses);
    let mut projection = DisplayProjectionState::from_canonical_window(&canonical, 1..2);
    projection.reproject(&context);

    assert_eq!(
        rendered_send_state(projection.display_items(), "t1"),
        None,
        "the failed row is not in the window"
    );
    assert_eq!(
        rendered_send_state(projection.display_items(), "t2"),
        Some(recoverable()),
        "the table, not a retained slot, decides that the successor is blocked"
    );

    projection.return_to_live_window(&canonical, &context);

    assert_eq!(
        rendered_send_state(projection.display_items(), "t1"),
        Some(recoverable())
    );
    assert_eq!(
        rendered_send_state(projection.display_items(), "t2"),
        Some(recoverable())
    );
}

/// Path 4: initial materialization and mid-stream batch insertion. A row that
/// joins after the failure is derived from the same authority as the head.
#[test]
fn batch_inserted_successor_joins_as_blocked() {
    let statuses = stopped_room();
    let failed = send_row("t1", Some(recoverable()));
    let mut canonical = vec![failed];
    let mut projection =
        DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
    let context = room_context(&statuses);

    let successor = send_row("t2", Some(TimelineSendState::Sending));
    let projected = project_sdk_batch(
        &mut canonical,
        &mut projection,
        &[TimelineDiff::PushBack { item: successor }],
        &context,
    );

    assert!(
        !projected.display_diffs.is_empty(),
        "the successor is published by this batch"
    );
    assert_eq!(
        rendered_send_state(projection.display_items(), "t2"),
        Some(recoverable())
    );

    // A first materialization of the same rows reaches the same state: the table
    // is read at projection time, not baked into the rows it materializes.
    let authority = SendStateAuthority::new(&statuses);
    let mut fresh = DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
    fresh.reproject(&context);
    assert_eq!(
        rendered_send_state(fresh.display_items(), "t1"),
        Some(recoverable())
    );
    assert_eq!(
        rendered_send_state(fresh.display_items(), "t2"),
        Some(recoverable())
    );
    assert_eq!(
        rendered_send_state(fresh.display_items(), "t2"),
        authority.derive(Some(&TimelineSendState::Sending), Some("t2")),
        "the projection renders exactly the derivation"
    );
}

/// Path 3 at the commit boundary: the recovery-window replacement and the pending
/// refresh publish through the same authority as the live path, so a committed row
/// — or a manager-owned row joining the committed window — never renders what its
/// own copy carries.
#[test]
fn a_committed_window_and_its_pending_rows_keep_the_resolved_state() {
    let statuses = stopped_room();
    let canonical = canonical_queue();
    let authority = SendStateAuthority::new(&statuses);
    let scrolled_up = DisplayProjectionContext::for_timeline(
        &room_key().kind,
        &TimelineViewportObservation {
            at_bottom: false,
            ..Default::default()
        },
    )
    .with_send_states(&statuses);
    let mut projection =
        DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
    // The SDK/Core reset commits a replacement snapshot over the same positions.
    projection.reset_bounded(&canonical, &scrolled_up);
    assert_eq!(
        rendered_send_state(projection.display_items(), "t2"),
        Some(recoverable()),
        "the committed replacement window renders the authority, not its rows"
    );
    // A request the SDK has not bound yet joins the committed window.
    let pending = send_row("t3", Some(TimelineSendState::Sending));
    projection.replace_pending(vec![pending.clone()], HashSet::new(), &scrolled_up);
    assert_eq!(
        rendered_send_state(projection.display_items(), "t3"),
        Some(recoverable()),
        "a pending row joining the window is blocked by the same authority"
    );
    // Returning to live republishes the same rows.
    projection.return_to_live_window(&canonical, &room_context(&statuses));

    for transaction_id in ["t1", "t2", "t3"] {
        assert_eq!(
            rendered_send_state(projection.display_items(), transaction_id),
            Some(recoverable()),
            "{transaction_id} is not sending in a stopped room"
        );
    }
    // The row the command names is the un-derived one, and admission accepts it
    // exactly because it renders `not_sent`.
    for un_derived in [canonical[1].clone(), pending] {
        assert!(
            retry_admission(&authority, &un_derived).is_ok(),
            "a rendered Resend must be accepted: {:?}",
            un_derived.id
        );
    }
}

/// #1846 B1: the blocking authority is the room table, never the rows a display
/// window happens to keep. Counterexample shape: the SDK reported the failure on
/// the head's row, but the room monitor has not written it into the table yet, so
/// the only recoverable failure is the head's own row.
#[test]
fn window_composition_never_changes_a_rendered_send_state() {
    let canonical = canonical_queue();
    let row_evidence_only = HashMap::from([("t2".to_owned(), TimelineSendState::Sending)]);
    let table_knows = stopped_room();

    for statuses in [row_evidence_only, table_knows] {
        let context = room_context(&statuses);
        let with_head_in_window = {
            let mut projection = DisplayProjectionState::from_canonical_window(&canonical, 0..2);
            projection.reproject(&context);
            rendered_send_state(projection.display_items(), "t2")
        };
        // `return_to_live_window` keeps only the live tail, so a longer queue leaves
        // the failed head outside the materialized window.
        let without_head_in_window = {
            let mut projection = DisplayProjectionState::from_canonical_window(&canonical, 1..2);
            projection.reproject(&context);
            rendered_send_state(projection.display_items(), "t2")
        };

        assert_eq!(
            with_head_in_window, without_head_in_window,
            "the materialized window must not decide a rendered send state (table={statuses:?})"
        );
        let authority = SendStateAuthority::new(&statuses);
        let row = send_row("t2", Some(TimelineSendState::Sending));
        assert_eq!(
            matches!(
                without_head_in_window,
                Some(TimelineSendState::NotSent { .. })
            ),
            retry_admission(&authority, &row).is_ok(),
            "admission must agree with the rendered row (table={statuses:?})"
        );
    }
}

/// #1851: the queue-update stream can lag, and the actor rebuilds its send table
/// from the SDK's local echoes. An echo cannot express a resolved recoverable
/// failure, so the rebuild alone republishes a stopped queue as `Sending`: the
/// failed row loses `Resend` and `RetrySend` then answers `InvalidSendState`.
/// The carried record is what keeps the row (and the queue behind it) resendable.
#[test]
fn a_lagged_send_table_rebuild_keeps_the_stopped_room_resendable() {
    // What `remember_local_echo` produces on a lag rebuild: both requests look
    // like they are still on their way, because the transient failure lives in
    // the SDK's queue, not on the echo.
    let echoes_only = HashMap::from([
        ("t1".to_owned(), TimelineSendState::Sending),
        ("t2".to_owned(), TimelineSendState::Sending),
    ]);
    let canonical = canonical_queue();

    let rendered_after = |statuses: &HashMap<String, TimelineSendState>| {
        let mut projection =
            DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
        projection.reproject(&room_context(statuses));
        (
            rendered_send_state(projection.display_items(), "t1"),
            rendered_send_state(projection.display_items(), "t2"),
        )
    };

    assert_eq!(
        rendered_after(&echoes_only),
        (Some(recoverable()), Some(TimelineSendState::Sending)),
        "the head keeps its own SDK report, but the queue behind it silently goes back to `Sending`"
    );
    assert!(
        retry_admission(&SendStateAuthority::new(&echoes_only), &canonical[1]).is_err(),
        "so the queued row loses the Resend the stopped room offered"
    );

    let mut rebuilt = echoes_only;
    carry_resolved_send_failures(&mut rebuilt, stopped_room());

    assert_eq!(
        rendered_after(&rebuilt),
        (Some(recoverable()), Some(recoverable())),
        "the carried failure keeps the failed row and its successor not sent"
    );
    // The row renders `Resend`, so the command that names it must be accepted.
    let authority = SendStateAuthority::new(&rebuilt);
    for row in &canonical {
        assert!(
            retry_admission(&authority, row).is_ok(),
            "a rendered Resend must survive the rebuild: {:?}",
            row.id
        );
    }
}

/// #1851: the other echo-only seed is the actor's own startup. A replacement
/// actor starts with an empty table and only replays the SDK's echoes, which
/// cannot express a recoverable failure, so switching rooms and back would render
/// a stopped queue as sending again. The manager's still-owned failed request is
/// the evidence the seed carries.
#[test]
fn a_replacement_actor_keeps_the_stopped_room_resendable() {
    let canonical = canonical_queue();
    let echoes_only = HashMap::from([
        ("t1".to_owned(), TimelineSendState::Sending),
        ("t2".to_owned(), TimelineSendState::Sending),
    ]);
    let rendered = |statuses: &HashMap<String, TimelineSendState>| {
        let mut projection =
            DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
        projection.reproject(&room_context(statuses));
        (
            rendered_send_state(projection.display_items(), "t1"),
            rendered_send_state(projection.display_items(), "t2"),
        )
    };

    assert_eq!(
        rendered(&echoes_only),
        (Some(recoverable()), Some(TimelineSendState::Sending)),
        "the echoes alone cannot tell a stopped queue from a working one"
    );

    let mut seeded = echoes_only;
    carry_resolved_send_failures(&mut seeded, stopped_room());

    assert_eq!(
        rendered(&seeded),
        (Some(recoverable()), Some(recoverable())),
        "the manager's own record keeps the queue not sent across the switch"
    );
    assert!(
        retry_admission(&SendStateAuthority::new(&seeded), &canonical[1]).is_ok(),
        "and the queued row still offers the Resend that resumes the room"
    );
}
