//! #1007: add an existing joined room to a Space, including a room version 12
//! room whose ID has no server name.
//!
//! A second SDK session of user A (a disposable auditor device) creates a
//! version 12 room and gives it only a child-side `m.space.parent`, the case
//! that other clients do not list under the Space. Core must still offer it,
//! write the parent-side `m.space.child` with SDK-derived routing, and project
//! it as added. The auditor then reads the Space's state from the homeserver:
//! local projection alone is not evidence. Creating a room inside the Space
//! must link it the same way.

use super::super::fixtures::set_space_child_for_qa;
use super::super::scenario_identity::cleanup_qa_auditor_device;
use super::*;
use koushi_core::{CreateRoomOptions, CreateRoomParentSpace, CreateRoomVisibility};
use koushi_state::{SpaceAddRoomStatus, SpaceChildLinkOutcome, space_add_rooms_for_state};
use matrix_sdk::ruma::{
    OwnedRoomId, OwnedServerName, RoomVersionId,
    api::client::{room::create_room, state::get_state_event_for_key},
    events::{StateEventType, space::parent::SpaceParentEventContent},
};

pub(super) async fn verify(config: &QaConfig, conn_a: &mut CoreConnection) -> Result<(), String> {
    let auditor = koushi_sdk::login_with_password(&koushi_state::LoginRequest {
        homeserver: config.homeserver.clone(),
        username: config.user_a.clone(),
        password: super::super::AuthSecret::new(config.password_a.clone()),
        device_display_name: Some("Koushi Space Add Auditor".to_owned()),
    })
    .await
    .map_err(|_| "space_add_existing: auditor login failed".to_owned())?;
    let result = verify_with_auditor(config, conn_a, &auditor).await;
    let cleanup = cleanup_qa_auditor_device(&auditor, &config.password_a).await;
    let _ = koushi_sdk::close_session_stores(&auditor).await;
    drop(auditor);
    result?;
    cleanup.map_err(|error| format!("space_add_existing: {error}"))
}

async fn verify_with_auditor(
    config: &QaConfig,
    conn_a: &mut CoreConnection,
    auditor: &koushi_sdk::MatrixClientSession,
) -> Result<(), String> {
    let space_id =
        create_space_for_qa(conn_a, "QA Space Add Existing", "space_add_existing space").await?;
    wait_for_space_in_space_list(conn_a, &space_id, "space_add_existing space list").await?;

    // A room version 12 room: its ID is an event hash with no server name.
    let mut request = create_room::v3::Request::new();
    request.name = Some("QA Space Add Existing v12".to_owned());
    request.room_version = Some(RoomVersionId::V12);
    let room = auditor
        .client()
        .create_room(request)
        .await
        .map_err(|_| "space_add_existing: version 12 room creation failed".to_owned())?;
    let room_id = room.room_id().to_owned();
    if room_id.server_name().is_some() {
        return Err("space_add_existing: the homeserver created a room ID with a server".into());
    }
    let server: OwnedServerName = config
        .server_name
        .as_str()
        .try_into()
        .map_err(|_| "space_add_existing: invalid QA server name".to_owned())?;
    let space_room_id: OwnedRoomId = space_id
        .as_str()
        .try_into()
        .map_err(|_| "space_add_existing: invalid Space ID".to_owned())?;
    // Child-side relationship only: the Space does not list the room.
    room.send_state_event_for_key(&space_room_id, SpaceParentEventContent::new(vec![server]))
        .await
        .map_err(|_| "space_add_existing: parent-only relationship setup failed".to_owned())?;
    wait_for_room_in_room_list(conn_a, room_id.as_str(), "space_add_existing room list").await?;

    // Whether the room list already shows the parent-only room inside the
    // Space depends on sync timing; the add projection is what is asserted.
    let select_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id: select_id,
            space_id: Some(space_id.clone()),
        }))
        .await
        .map_err(|_| "space_add_existing: select Space submission failed".to_owned())?;
    wait_for_add_status(
        conn_a,
        room_id.as_str(),
        |status| status == SpaceAddRoomStatus::Available,
        "space_add_existing parent-only room offered",
    )
    .await?;

    set_space_child_for_qa(
        conn_a,
        &space_id,
        room_id.as_str(),
        "space_add_existing add",
    )
    .await?;
    wait_for_add_status(
        conn_a,
        room_id.as_str(),
        |status| status == SpaceAddRoomStatus::Added,
        "space_add_existing added",
    )
    .await?;
    assert_server_space_child(auditor, &space_room_id, room_id.as_str(), "added room").await?;

    // Creating a room in the Space links it through the same routing.
    let create_id = conn_a.next_request_id();
    conn_a
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id: create_id,
            options: CreateRoomOptions {
                name: format!("QA Space Created {}", std::process::id()),
                topic: None,
                alias_localpart: None,
                encrypted: false,
                invited_only: false,
                visibility: CreateRoomVisibility::Private,
                parent_space: Some(CreateRoomParentSpace {
                    space_id: space_id.clone(),
                }),
            },
        }))
        .await
        .map_err(|_| "space_add_existing: create submission failed".to_owned())?;
    let created_id =
        wait_for_room_created(conn_a, create_id, "space_add_existing create in Space").await?;
    // `RoomCreated` can reach this connection before the snapshot that
    // records the linking settlement; wait for that settlement.
    let linked = wait_for_link_settlement(conn_a, &space_id, &created_id).await?;
    if linked != SpaceChildLinkOutcome::Linked {
        return Err(format!(
            "space_add_existing: created room was not linked to its Space ({linked:?})"
        ));
    }
    assert_server_space_child(auditor, &space_room_id, &created_id, "created room").await?;

    println!("space_add_existing=ok");
    Ok(())
}

async fn wait_for_link_settlement(
    conn: &mut CoreConnection,
    space_id: &str,
    room_id: &str,
) -> Result<SpaceChildLinkOutcome, String> {
    let settled = |snapshot: &AppState| {
        snapshot
            .space_child_links
            .latest(space_id, room_id)
            .map(|result| result.outcome)
    };
    if let Some(outcome) = settled(&conn.snapshot()) {
        return Ok(outcome);
    }
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| "space_add_existing: creation link settlement timeout".to_owned())?
            .map_err(|lag| {
                format!(
                    "space_add_existing: event stream lagged (skipped={})",
                    lag.skipped
                )
            })?;
        if let Some(outcome) = settled(&conn.snapshot()) {
            return Ok(outcome);
        }
    }
}

async fn wait_for_add_status(
    conn: &mut CoreConnection,
    room_id: &str,
    expected: impl Fn(SpaceAddRoomStatus) -> bool,
    label: &str,
) -> Result<(), String> {
    let matches = |snapshot: &AppState| {
        space_add_rooms_for_state(snapshot).is_some_and(|model| {
            model
                .candidates
                .iter()
                .any(|candidate| candidate.room_id == room_id && expected(candidate.status))
        })
    };
    if matches(&conn.snapshot()) {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + ROOM_LIST_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                let status = space_add_rooms_for_state(&conn.snapshot()).and_then(|model| {
                    model
                        .candidates
                        .into_iter()
                        .find(|candidate| candidate.room_id == room_id)
                        .map(|candidate| candidate.status)
                });
                format!("{label}: timed out (status={status:?})")
            })?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;
        if matches!(
            event,
            CoreEvent::Room(RoomEvent::RoomListUpdated) | CoreEvent::StateDelta(_)
        ) && matches(&conn.snapshot())
        {
            return Ok(());
        }
    }
}

/// Read the Space's `m.space.child` for one room from the homeserver and
/// require nonempty routing.
async fn assert_server_space_child(
    auditor: &koushi_sdk::MatrixClientSession,
    space_id: &OwnedRoomId,
    child_room_id: &str,
    label: &str,
) -> Result<(), String> {
    let request = get_state_event_for_key::v3::Request::new(
        space_id.clone(),
        StateEventType::SpaceChild,
        child_room_id.to_owned(),
    );
    let response = auditor
        .client()
        .send(request)
        .await
        .map_err(|_| format!("space_add_existing: {label} has no server-side m.space.child"))?;
    let content: serde_json::Value = serde_json::from_str(response.event_or_content.get())
        .map_err(|_| format!("space_add_existing: {label} m.space.child is unreadable"))?;
    let has_route = content
        .get("via")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|via| {
            via.iter()
                .any(|server| server.as_str().is_some_and(|s| !s.is_empty()))
        });
    if has_route {
        Ok(())
    } else {
        Err(format!(
            "space_add_existing: {label} m.space.child has no routing"
        ))
    }
}
