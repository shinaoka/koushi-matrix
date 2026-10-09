//! Leave a Space together with a chosen subset of its joined child rooms.
//!
//! Three rooms are created inside a fresh Space and a fourth outside it. The
//! `LeaveSpace` command names two children plus the outside room; Core must
//! admit only the Space's leave candidates, leave the two children and then
//! the Space, and keep both the unselected child and the outside room joined.

use super::*;
use koushi_core::{CreateRoomOptions, CreateRoomParentSpace, CreateRoomVisibility};
use koushi_state::space_leave_candidates_for_state;

const LABEL: &str = "space_leave_children";

pub(super) async fn verify(conn_a: &mut CoreConnection) -> Result<(), String> {
    let space_id = create_space_for_qa(conn_a, "QA Space Leave", LABEL).await?;
    wait_for_space_in_space_list(conn_a, &space_id, "space_leave_children space list").await?;

    let mut children = Vec::new();
    for index in 0..3 {
        children.push(create_room_in_space(conn_a, &space_id, index).await?);
    }
    let outside_id = create_room_for_qa(
        conn_a,
        "QA Space Leave Outside",
        false,
        "space_leave_children outside",
    )
    .await?;
    wait_for_room_in_room_list(conn_a, &outside_id, "space_leave_children outside list").await?;
    wait_for_snapshot(conn_a, "space_leave_children candidates", |state| {
        let candidates = space_leave_candidates_for_state(state, &space_id);
        children.iter().all(|child| {
            candidates
                .iter()
                .any(|candidate| &candidate.room_id == child)
        }) && !candidates
            .iter()
            .any(|candidate| candidate.room_id == outside_id)
    })
    .await?;

    let request_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::LeaveSpace {
            request_id,
            space_id: space_id.clone(),
            child_room_ids: vec![children[0].clone(), outside_id.clone(), children[1].clone()],
        }))
        .await
        .map_err(|e| format!("{LABEL}: submit leave space failed: {e}"))?;
    wait_for_room_left(conn_a, request_id, &space_id, "space_leave_children leave").await?;
    wait_for_snapshot(conn_a, "space_leave_children memberships", |state| {
        let joined = |room_id: &str| state.rooms.iter().any(|room| room.room_id == room_id);
        !state.spaces.iter().any(|space| space.space_id == space_id)
            && !joined(&children[0])
            && !joined(&children[1])
            && joined(&children[2])
            && joined(&outside_id)
    })
    .await?;
    println!("space_leave_children=ok");

    for room_id in [&children[2], &outside_id] {
        let leave_id = conn_a.next_request_id();
        conn_a
            .command(CoreCommand::Room(RoomCommand::LeaveRoom {
                request_id: leave_id,
                room_id: room_id.clone(),
            }))
            .await
            .map_err(|e| format!("{LABEL}: submit cleanup leave failed: {e}"))?;
        wait_for_room_left(conn_a, leave_id, room_id, "space_leave_children cleanup").await?;
    }
    Ok(())
}

async fn create_room_in_space(
    conn: &mut CoreConnection,
    space_id: &str,
    index: usize,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        options: CreateRoomOptions {
            name: format!("QA Space Leave Child {index}"),
            topic: None,
            alias_localpart: None,
            encrypted: false,
            invited_only: false,
            visibility: CreateRoomVisibility::Private,
            parent_space: Some(CreateRoomParentSpace {
                space_id: space_id.to_owned(),
            }),
            access_policy: None,
            history: None,
        },
    }))
    .await
    .map_err(|e| format!("{LABEL}: submit child create failed: {e}"))?;
    wait_for_room_created(conn, request_id, "space_leave_children child create").await
}

async fn wait_for_snapshot(
    conn: &mut CoreConnection,
    label: &str,
    matches: impl Fn(&AppState) -> bool,
) -> Result<(), String> {
    if matches(&conn.snapshot()) {
        return Ok(());
    }
    let deadline = QaEventDeadline::after(ROOM_LIST_EVENT_TIMEOUT);
    loop {
        deadline
            .recv(conn)
            .await
            .map_err(|_| format!("{label}: timed out"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if matches(&conn.snapshot()) {
            return Ok(());
        }
    }
}
